//! The settings window, drawn entirely from the declarations in
//! `wbs_core::settings`: a section per group, a widget per kind, a one-line
//! summary (the full help is on hover and in Help ▸ Settings), any problem with
//! the stored value, and Reset. It doesn't change anything
//! itself; it returns the changes for the app to apply.
//!
//! Like help, it's its own OS window (an immediate viewport), drawn as an
//! `egui::Window` where viewports are embedded (headless tests).

use crate::app::Command;
use crate::help_view::replay_edit_commands;
use crate::prefs::Prefs;
use eframe::egui::{self, Color32, RichText, TextEdit, Ui, ViewportCommand, ViewportId};
use std::collections::HashMap;
use wbs_core::help::{self, Inline};
use wbs_core::model::Workbook;
use wbs_core::settings::{self, Kind, Scope, Setting, Val};

pub const TITLE: &str = "Settings — the world's best spreadsheet";

/// A change to apply: a new value, or `None` for back to the default.
pub type Change = (&'static Setting, Option<Val>);

fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    embedded: bool,
    focused: bool,
    raise: bool,
    forward: Vec<Command>,
    /// Text typed into a text setting that isn't valid yet (so isn't applied).
    drafts: HashMap<&'static str, String>,
}

/// What the window shows: both scopes' stored values.
pub struct Sources<'a> {
    pub prefs: &'a Prefs,
    pub wb: &'a Workbook,
    pub doc_name: &'a str,
}

impl SettingsWindow {
    pub fn show(&mut self) {
        self.open = true;
        self.raise = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.focused = false;
        self.forward.clear();
        self.drafts.clear();
    }

    /// The settings window is its own OS window and has keyboard focus.
    pub fn has_focus(&self) -> bool {
        self.open && !self.embedded && self.focused
    }

    /// Hands an edit command (undo, copy…) to the focused settings window.
    pub fn forward(&mut self, c: Command) {
        self.forward.push(c);
    }

    /// Shows the window if it's open. `keys` are the app shortcuts to pass back to the app when pressed in it.
    pub fn ui(&mut self, ctx: &egui::Context, src: &Sources, keys: &[Command]) -> (Vec<Change>, Vec<Command>) {
        let (mut changes, mut cmds) = (Vec::new(), Vec::new());
        self.embedded = ctx.embed_viewports();
        if !self.open {
            return (changes, cmds);
        }
        if self.embedded {
            self.raise = false;
            let mut open = true;
            let screen = ctx.content_rect();
            egui::Window::new("Settings")
                .open(&mut open)
                .default_size([560.0, 640.0])
                .default_pos([screen.center().x - 280.0, screen.top() + 60.0])
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| self.contents(ui, src, &mut changes));
            if !open {
                self.close();
            }
            return (changes, cmds);
        }
        let vb = egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([600.0, 700.0]).with_min_inner_size([420.0, 300.0]);
        ctx.show_viewport_immediate(viewport_id(), vb, |ui, class| {
            let ctx = ui.ctx().clone();
            if class == egui::ViewportClass::Immediate {
                let (close, focused) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().focused.unwrap_or(false)));
                self.focused = focused;
                if std::mem::take(&mut self.raise) {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                replay_edit_commands(&ctx, std::mem::take(&mut self.forward));
                for &c in keys {
                    if c.shortcut().is_some_and(|s| ctx.input_mut(|i| i.consume_shortcut(&s))) {
                        cmds.push(c);
                    }
                }
                if close {
                    self.close();
                }
            }
            egui::CentralPanel::default().show(ui, |ui| self.contents(ui, src, &mut changes));
        });
        (changes, cmds)
    }

    fn contents(&mut self, ui: &mut Ui, src: &Sources, changes: &mut Vec<Change>) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(560.0);
            ui.label(RichText::new(Scope::App.title()).strong().size(18.0));
            match src.prefs.path() {
                Some(p) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("Located at ").weak());
                        ui.add(egui::Label::new(RichText::new(p.display().to_string()).monospace()).selectable(true));
                    });
                }
                None => {
                    ui.label(RichText::new("Not saved: there's no settings file.").weak());
                }
            }
            if let Some(e) = src.prefs.file_error() {
                ui.label(RichText::new(e).color(err_colour(ui)));
            }
            for section in settings::sections(Scope::App) {
                self.section(ui, section, Scope::App, src, changes);
            }
            ui.add_space(16.0);
            ui.label(RichText::new(format!("{} — {}", Scope::Workbook.title(), src.doc_name)).strong().size(18.0));
            ui.label(RichText::new("Saved in the workbook file.").weak());
            for section in settings::sections(Scope::Workbook) {
                self.section(ui, section, Scope::Workbook, src, changes);
            }
            let unknown: Vec<String> = src.prefs.unknown_keys().into_iter().map(|k| format!("`{k}` in the settings file"))
                .chain(settings::unknown_keys(Scope::Workbook, src.wb.settings.keys().map(String::as_str)).into_iter().map(|k| format!("`{k}` in this workbook")))
                .collect();
            if !unknown.is_empty() {
                ui.add_space(16.0);
                ui.label(RichText::new("Not used by this version").strong().size(15.0)).on_hover_text("Kept as they are, so a newer version (or a typo you fix) still finds them.");
                for u in unknown {
                    inline_para(ui, &format!("• {u}"));
                }
            }
        });
    }

    fn section(&mut self, ui: &mut Ui, section: &str, scope: Scope, src: &Sources, changes: &mut Vec<Change>) {
        ui.add_space(12.0);
        ui.label(RichText::new(section).strong().size(15.0));
        ui.separator();
        for s in settings::SETTINGS.iter().copied().filter(|s| s.scope == scope && s.section == section) {
            let ((val, problem), is_set) = match scope {
                Scope::App => (src.prefs.get(s), src.prefs.is_set(s)),
                Scope::Workbook => (settings::workbook_value(src.wb, s), src.wb.settings.contains_key(s.key)),
            };
            ui.add_space(6.0);
            ui.push_id(s.key, |ui| {
                ui.horizontal(|ui| {
                    // a checkbox carries its own label
                    if !matches!(s.kind, Kind::Bool) {
                        ui.label(RichText::new(s.label).strong()).on_hover_ui(|ui| inline_para(ui, s.help));
                    }
                    if let Some(v) = self.widget(ui, s, &val) {
                        changes.push((s, Some(v)));
                    }
                    let reset = ui.add_enabled(is_set || self.drafts.contains_key(s.key), egui::Button::new("Reset").small());
                    if reset.on_hover_text(format!("Back to the default: {}", s.show(&s.default_val()))).clicked() {
                        self.drafts.remove(s.key);
                        changes.push((s, None));
                    }
                });
                if let Some(d) = self.drafts.get(s.key) {
                    if let Err(why) = s.check(&Val::Text(d.clone())) {
                        ui.label(RichText::new(format!("Not applied: {why}.")).color(err_colour(ui)));
                    }
                }
                if let Some(why) = problem {
                    let msg = format!("Stored value ignored ({why}). Using the default, {}.", s.show(&s.default_val()));
                    ui.label(RichText::new(msg).color(err_colour(ui)));
                }
                ui.label(RichText::new(s.summary).weak());
                if let Some(p) = s.pending {
                    ui.label(RichText::new(p).italics().weak());
                }
            });
        }
    }

    /// The control for one setting; returns a new value when the user changes it.
    fn widget(&mut self, ui: &mut Ui, s: &'static Setting, val: &Val) -> Option<Val> {
        match s.kind {
            Kind::Bool => {
                let mut b = val.as_bool();
                ui.checkbox(&mut b, RichText::new(s.label).strong()).on_hover_ui(|ui| inline_para(ui, s.help)).changed().then_some(Val::Bool(b))
            }
            Kind::Int { min, max, unit } => {
                let mut n = val.as_int();
                ui.add(egui::DragValue::new(&mut n).range(min..=max).suffix(format!(" {unit}"))).changed().then_some(Val::Int(n))
            }
            Kind::Choice(opts) => {
                let mut cur = val.as_text().to_string();
                egui::ComboBox::from_id_salt(s.key).selected_text(s.show(val)).show_ui(ui, |ui| {
                    for (v, label) in opts {
                        ui.selectable_value(&mut cur, v.to_string(), *label);
                    }
                });
                (cur != val.as_text()).then_some(Val::Text(cur))
            }
            Kind::Colour => {
                let mut c = val.as_colour();
                let r = ui.color_edit_button_srgb(&mut c);
                ui.label(RichText::new(settings::colour_hex(c)).monospace());
                r.changed().then_some(Val::Colour(c))
            }
            Kind::Text { .. } => {
                let mut text = self.drafts.get(s.key).cloned().unwrap_or_else(|| val.as_text().to_string());
                let hint = s.show(&s.default_val());
                if !ui.add(TextEdit::singleline(&mut text).hint_text(hint).desired_width(260.0)).changed() {
                    return None;
                }
                let v = Val::Text(text.clone());
                if s.check(&v).is_ok() {
                    self.drafts.remove(s.key);
                    Some(v)
                } else {
                    self.drafts.insert(s.key, text);
                    None
                }
            }
        }
    }
}

fn err_colour(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0xf8, 0x71, 0x71)
    } else {
        Color32::from_rgb(0xdc, 0x26, 0x26)
    }
}

/// Help text: plain, **bold** and `code`.
fn inline_para(ui: &mut Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in help::inlines(text) {
            match i {
                Inline::Text(t) => ui.label(t),
                Inline::Bold(t) => ui.label(RichText::new(t).strong()),
                Inline::Code(c) => ui.label(RichText::new(c).monospace()),
                Inline::Link { label, .. } => ui.label(label),
            };
        }
    });
}
