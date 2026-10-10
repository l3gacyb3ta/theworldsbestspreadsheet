//! The app side of settings: applying preferences, the settings window's
//! changes, and autosave.

use super::*;
use crate::prefs::Prefs;
use crate::settings_view::Sources;
use std::time::Duration;
use wbs_core::settings::{self as decl, Scope, Setting, Val};

/// Autosave: a workbook with a file is saved `autosave.interval_seconds` after its first unsaved change.
#[derive(Default)]
pub(super) struct Autosave {
    /// When (egui time, seconds) the workbook was first seen with unsaved changes.
    pub(super) since: Option<f64>,
    /// The workbook is as autosave last wrote it.
    saved: bool,
    /// Why the last autosave failed (it tries again after another interval).
    failed: Option<String>,
    /// The save under way: a snapshot being written by another thread.
    pub(super) pending: Option<files::BackgroundSave>,
}

impl App {
    /// Use these preferences (main: the user's settings file; tests: a temporary one or none).
    pub fn with_prefs(mut self, prefs: Prefs) -> App {
        self.prefs = prefs;
        for s in decl::SETTINGS.iter().filter(|s| s.scope == Scope::App) {
            self.apply_pref(s);
        }
        self.note_setting_problems();
        self
    }

    /// Makes an app preference take effect (those read where they're used need nothing here).
    fn apply_pref(&mut self, s: &Setting) {
        let v = self.prefs.val(s);
        if s.key == decl::TRACE_AT_START.key {
            self.trace = v.as_bool();
        } else if s.key == decl::GOAL_SEEK_LIVE_MS.key {
            self.goal_live_ms = v.as_int() as f64;
        }
    }

    /// Says in the status bar when a stored setting is unusable (the settings window says why).
    pub(super) fn note_setting_problems(&mut self) {
        let mut bad: Vec<&str> = self.prefs.problems().into_iter().map(|(k, _)| k).collect();
        bad.extend(decl::workbook_problems(&self.eng.wb).into_iter().map(|(k, _)| k));
        let file = self.prefs.file_error();
        if bad.is_empty() && file.is_none() {
            return;
        }
        let what = match (bad.len(), file) {
            (0, Some(f)) => f,
            (1, _) => format!("the setting {} has a value that can't be used, so its default is used", bad[0]),
            (_, _) => format!("the settings {} have values that can't be used, so their defaults are used", bad.join(", ")),
        };
        let prev = self.status.take().map(|s| format!("{s} · ")).unwrap_or_default();
        self.status = Some(format!("{prev}{what} — see Settings (⌘,)"));
    }

    pub(super) fn settings_ui(&mut self, ctx: &egui::Context) {
        let keys = if self.confirm.is_none() { self.owned_keys() } else { &[] };
        let name = self.display_name();
        let (changes, cmds) = self.settings.ui(ctx, &Sources { prefs: &self.prefs, wb: &self.eng.wb, doc_name: &name }, keys);
        for (s, v) in changes {
            self.change_setting(s, v.as_ref());
        }
        for c in cmds {
            self.run(ctx, c);
        }
    }

    /// Sets a setting (`None`: back to its default) and applies it now.
    pub(super) fn change_setting(&mut self, s: &'static Setting, v: Option<&Val>) {
        let res = match s.scope {
            Scope::App => {
                let r = self.prefs.set(s, v);
                self.apply_pref(s);
                r
            }
            Scope::Workbook => {
                self.dirty_stale = true;
                decl::set_workbook_value(&mut self.eng.wb, s, v)
            }
        };
        self.status = Some(match (res, v) {
            (Ok(()), Some(v)) => format!("{}: {}", s.label, s.show(v)),
            (Ok(()), None) => format!("{}: back to the default, {}", s.label, s.show(&s.default_val())),
            (Err(e), _) => format!("{}: {e}", s.label),
        });
    }

    /// Seconds from the first unsaved change to autosaving, if autosave applies to this workbook.
    fn autosave_interval(&self) -> Option<f64> {
        let on = match decl::workbook_value(&self.eng.wb, decl::AUTOSAVE_WORKBOOK).0.as_text() {
            "always" => true,
            "never" => false,
            _ => self.prefs.val(decl::AUTOSAVE_ENABLED).as_bool(),
        };
        on.then(|| self.prefs.val(decl::AUTOSAVE_INTERVAL).as_int() as f64)
    }

    /// Once per frame, after `document_ui` has refreshed `dirty`. Never asks where to save: an
    /// untitled workbook waits for ⌘S. Waits while a cell is being edited, a drag is under way,
    /// or the save-changes prompt is up.
    pub(super) fn autosave(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if let Some(done) = self.autosave.pending.as_mut().and_then(|p| p.result()) {
            let p = self.autosave.pending.take().unwrap();
            match done {
                Ok(fp) => {
                    self.saved_fp = fp;
                    // no edit since the snapshot: clean; otherwise compare with what was written
                    if self.eng.revision() == p.revision {
                        self.dirty = false;
                        self.dirty_rev = p.revision;
                    } else {
                        self.dirty_stale = true;
                    }
                    self.autosave = Autosave { since: None, saved: true, failed: None, pending: None };
                    self.status = Some(format!("autosaved {}", self.display_name()));
                }
                Err(e) => {
                    self.status = Some(format!("save failed: {e}"));
                    self.autosave.failed = self.status.clone();
                    self.autosave.since = Some(now);
                }
            }
            ctx.request_repaint();
        }
        if self.autosave.pending.is_some() {
            return;
        }
        let Some(interval) = self.autosave_interval().filter(|_| self.path.is_some()) else {
            self.autosave.since = None;
            return;
        };
        if !self.dirty {
            self.autosave.since = None;
            self.autosave.failed = None;
            return;
        }
        self.autosave.saved = false;
        let since = *self.autosave.since.get_or_insert(now);
        if now < since + interval {
            // wake up at the deadline, and each second for the countdown in the status bar
            ctx.request_repaint_after(Duration::from_secs_f64((since + interval - now).min(1.0)));
            return;
        }
        if self.autosave_guard().is_some() {
            // ending an edit, a drag or the prompt is input, which runs a frame; this only refreshes the status bar
            ctx.request_repaint_after(Duration::from_secs(1));
            return;
        }
        // serializing and writing a big workbook takes a while: a snapshot (cells are shared, so
        // it's cheap) is written by another thread, which wakes the UI when it's done
        let path = self.path.clone().unwrap();
        self.autosave.pending = Some(files::BackgroundSave::start(ctx, self.eng.wb.clone(), path, self.eng.revision()));
        // the status bar was drawn earlier in this frame, still counting down: show "autosaving" now
        ctx.request_repaint();
    }

    /// What an overdue autosave is waiting for, if anything.
    fn autosave_guard(&self) -> Option<&'static str> {
        if self.confirm.is_some() {
            Some("autosave waits for the save prompt")
        } else if self.edit.is_some() {
            Some("autosave waits while you edit")
        } else if !matches!(self.drag, Drag::None) || self.bar_scrub.is_some() || self.input_scrub.is_some() {
            Some("autosave waits while you drag")
        } else {
            None
        }
    }

    /// The status bar's autosave note.
    pub(super) fn autosave_note(&self, now: f64) -> Option<String> {
        let Some(interval) = self.autosave_interval() else {
            let never = decl::workbook_value(&self.eng.wb, decl::AUTOSAVE_WORKBOOK).0.as_text() == "never";
            return never.then(|| "autosave off for this workbook".to_string());
        };
        Some(if self.path.is_none() {
            "autosave starts once the workbook has a file".to_string()
        } else if let Some(f) = &self.autosave.failed {
            format!("autosave: {f}")
        } else if let (true, Some(t)) = (self.dirty, self.autosave.since) {
            match (self.autosave_guard(), (t + interval - now).ceil()) {
                _ if self.autosave.pending.is_some() => "autosaving".to_string(),
                (Some(g), left) if left <= 0.0 => g.to_string(),
                (None, left) if left <= 0.0 => "autosaving".to_string(),
                (_, left) => format!("autosave in {left} s"),
            }
        } else if self.autosave.saved {
            "autosaved".to_string()
        } else {
            "autosave on".to_string()
        })
    }
}
