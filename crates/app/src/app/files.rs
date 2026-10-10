//! Documents: New / Open / Save / Save As, the dirty marker in the window
//! title, and the "save changes?" prompt before closing, New and Open.

use super::*;
use std::collections::hash_map::DefaultHasher;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use wbs_core::model::{Piece, Workbook};

const APP_NAME: &str = "the world's best spreadsheet";

/// Where file dialogs come from; swapped out in tests so nothing real opens.
pub trait Dialogs {
    fn open(&mut self, dir: Option<&Path>) -> Option<PathBuf>;
    fn save_as(&mut self, dir: Option<&Path>, file_name: &str) -> Option<PathBuf>;
}

#[cfg_attr(test, allow(dead_code))]
pub struct NativeDialogs;

impl Dialogs for NativeDialogs {
    fn open(&mut self, dir: Option<&Path>) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title("Open workbook").add_filter("Workbook", &["json"]).add_filter("All files", &["*"]);
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        d.pick_file()
    }
    fn save_as(&mut self, dir: Option<&Path>, file_name: &str) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title("Save workbook").add_filter("Workbook", &["json"]).set_file_name(file_name);
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        d.save_file()
    }
}

/// Tests must never open a real dialog: they install a scripted one.
#[cfg(test)]
pub struct NoDialogs;

#[cfg(test)]
impl Dialogs for NoDialogs {
    fn open(&mut self, _: Option<&Path>) -> Option<PathBuf> {
        panic!("test opened a file dialog without scripting one")
    }
    fn save_as(&mut self, _: Option<&Path>, _: &str) -> Option<PathBuf> {
        panic!("test opened a file dialog without scripting one")
    }
}

pub(super) fn default_dialogs() -> Box<dyn Dialogs> {
    #[cfg(test)]
    return Box::new(NoDialogs);
    #[cfg(not(test))]
    Box::new(NativeDialogs)
}

/// What to do once unsaved changes have been dealt with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pending {
    New,
    Open,
    Quit,
}

/// A hash of everything the user would call the document: sheet names and
/// order, cell sources by position, sizes and names. Unlike comparing the
/// saved JSON, it ignores hash-map order, hidden content and which rows are
/// stored, so undoing back to the saved state counts as clean again.
///
/// Each cell is hashed with FxHash (fast; it runs after every edit) and then
/// scrambled with a full-avalanche finalizer, so the per-sheet sums stay as
/// collision-resistant as summing SipHashes.
pub(super) fn fingerprint(wb: &Workbook) -> u64 {
    fn mix(mut x: u64) -> u64 {
        // splitmix64's finalizer
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    fn one(x: impl Hash) -> u64 {
        let mut h = FxHasher::default();
        x.hash(&mut h);
        mix(h.finish())
    }
    let mut h = DefaultHasher::new();
    for (si, s) in wb.sheets.iter().enumerate() {
        (si, s.id, &s.name).hash(&mut h);
        // order-independent sums over the hash maps
        let mut sum = 0u64;
        for ((r, c), cell) in &s.cells {
            // cells in deleted rows and columns are kept, hidden: like deleted sheets, not part of the document's look
            let (Some(ri), Some(ci)) = (s.row_index(*r), s.col_index(*c)) else { continue };
            let mut ch = FxHasher::default();
            (si, ri, ci).hash(&mut ch);
            for p in cell.pieces.iter() {
                match p {
                    Piece::Text(t) => (0u8, t).hash(&mut ch),
                    Piece::Ref(a) => (1u8, a).hash(&mut ch),
                    Piece::Range(a, b) => (2u8, a, b).hash(&mut ch),
                }
            }
            sum = sum.wrapping_add(mix(ch.finish()));
        }
        for (r, v) in &s.row_heights {
            sum = sum.wrapping_add(one((1u8, si, s.row_index(*r), v.to_bits())));
        }
        for (c, v) in &s.col_widths {
            sum = sum.wrapping_add(one((2u8, si, s.col_index(*c), v.to_bits())));
        }
        sum.hash(&mut h);
    }
    for (n, d) in &wb.names {
        (n, d.cell, d.input).hash(&mut h);
    }
    for (k, v) in &wb.settings {
        (k, v.to_string()).hash(&mut h);
    }
    h.finish()
}

fn write_workbook(wb: &Workbook, path: &Path) -> Result<(), String> {
    let s = serde_json::to_string_pretty(wb).map_err(|e| e.to_string())?;
    std::fs::write(path, s).map_err(|e| e.to_string())
}

/// A workbook snapshot being written by another thread. Dropping it waits for the write,
/// so quitting, opening or saving again never races a half-written file.
pub(super) struct BackgroundSave {
    /// `Engine::revision()` when the snapshot was taken.
    pub(super) revision: u64,
    rx: std::sync::mpsc::Receiver<Result<u64, String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BackgroundSave {
    pub(super) fn start(ctx: &egui::Context, wb: Workbook, path: PathBuf, revision: u64) -> BackgroundSave {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let thread = std::thread::spawn(move || {
            // the snapshot's fingerprint, for the dirty check: computed here, off the UI thread
            let res = write_workbook(&wb, &path).map(|()| fingerprint(&wb));
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        BackgroundSave { revision, rx, thread: Some(thread) }
    }

    /// The fingerprint of what was written, once the write is done. (Tests wait for it, so
    /// a save lands in the frame after it starts, however the threads are scheduled.)
    pub(super) fn result(&mut self) -> Option<Result<u64, String>> {
        if cfg!(test) {
            return Some(self.wait());
        }
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("the save thread stopped".into())),
        }
    }
}

impl BackgroundSave {
    pub(super) fn wait(&mut self) -> Result<u64, String> {
        self.rx.recv().unwrap_or_else(|_| Err("the save thread stopped".into()))
    }
}

impl Drop for BackgroundSave {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn read_workbook(path: &Path) -> Result<Engine, String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let wb: Workbook = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    Ok(Engine::new(wb))
}

impl App {
    pub(super) fn display_name(&self) -> String {
        match &self.path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
            None => "Untitled".to_string(),
        }
    }

    /// Fresh check (hashes the workbook); per-frame code reads `self.dirty`.
    pub(super) fn is_dirty(&self) -> bool {
        fingerprint(&self.eng.wb) != self.saved_fp
    }

    pub(super) fn mark_clean(&mut self) {
        self.saved_fp = fingerprint(&self.eng.wb);
        self.dirty = false;
        self.dirty_rev = self.eng.revision();
    }

    /// Replace the document, dropping everything that pointed into the old one.
    pub(super) fn load(&mut self, eng: Engine, path: Option<PathBuf>) {
        self.eng = eng;
        self.path = path;
        self.sheet_ix = 0;
        self.anchor = (0, 0);
        self.cursor = (0, 0);
        self.edit = None;
        self.focus_req = None;
        self.scroll = Vec2::ZERO;
        self.undo.clear();
        self.redo.clear();
        self.clip = None;
        self.cut = None;
        self.drag = Drag::None;
        self.offer = None;
        self.name_for = None;
        self.chart_hits.clear();
        self.bar_scrub = None;
        self.input_scrub = None;
        self.tabs = Default::default();
        self.autosave = Default::default();
        self.mark_clean();
    }

    pub(super) fn new_doc(&mut self) {
        self.load(Engine::new(wbs_core::stdlib::default_workbook()), None);
        self.status = Some("new workbook".into());
    }

    pub(super) fn open_path(&mut self, path: PathBuf) -> bool {
        match read_workbook(&path) {
            Ok(eng) => {
                self.status = Some(format!("opened {}", path.display()));
                self.load(eng, Some(path));
                self.note_setting_problems();
                true
            }
            Err(e) => {
                self.status = Some(format!("couldn't open {}: {e}", path.display()));
                false
            }
        }
    }

    fn dialog_dir(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|p| p.parent()).filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf())
    }

    pub(super) fn open_dialog(&mut self) {
        let dir = self.dialog_dir();
        if let Some(p) = self.dialogs.open(dir.as_deref()) {
            self.open_path(p);
        }
    }

    /// Save to the current file, asking for one if the workbook is untitled.
    pub(super) fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.write_to(p),
            None => self.save_as(),
        }
    }

    pub(super) fn save_as(&mut self) -> bool {
        let dir = self.dialog_dir();
        let name = match &self.path {
            Some(_) => self.display_name(),
            None => "Untitled.wbs.json".to_string(),
        };
        match self.dialogs.save_as(dir.as_deref(), &name) {
            Some(p) => self.write_to(p),
            None => false,
        }
    }

    pub(super) fn write_to(&mut self, path: PathBuf) -> bool {
        // an autosave still writing finishes first, so it can't land after this
        self.autosave.pending = None;
        match write_workbook(&self.eng.wb, &path) {
            Ok(()) => {
                self.status = Some(format!("saved {}", path.display()));
                self.path = Some(path);
                self.mark_clean();
                true
            }
            Err(e) => {
                self.status = Some(format!("save failed: {e}"));
                false
            }
        }
    }

    /// New / Open / Quit: ask first if there are unsaved changes.
    pub(super) fn guarded(&mut self, ctx: &egui::Context, what: Pending) {
        self.commit();
        self.settle_autosave(ctx);
        if self.is_dirty() {
            self.confirm = Some(what);
        } else {
            self.proceed(ctx, what);
        }
    }

    fn proceed(&mut self, ctx: &egui::Context, what: Pending) {
        match what {
            Pending::New => self.new_doc(),
            Pending::Open => self.open_dialog(),
            Pending::Quit => {
                self.close_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Once per frame: intercept the window closing, keep the title current,
    /// and show the save-changes prompt.
    pub(super) fn document_ui(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_ok {
            self.commit();
            self.settle_autosave(ctx);
            if self.is_dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(Pending::Quit);
            }
        }
        if let Some(what) = self.confirm {
            let mut choice = None;
            let modal = egui::Modal::new(Id::new("unsaved_changes")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.heading(format!("Save changes to “{}”?", self.display_name()));
                ui.add_space(4.0);
                ui.label("Your changes will be lost if you don't save them.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        choice = Some(1);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(0);
                    }
                });
            });
            if modal.should_close() && choice.is_none() {
                choice = Some(0);
            }
            match choice {
                Some(1) => {
                    self.confirm = None;
                    if self.save() {
                        self.proceed(ctx, what);
                    }
                }
                Some(2) => {
                    self.confirm = None;
                    self.proceed(ctx, what);
                }
                Some(_) => self.confirm = None,
                None => {}
            }
        }
        // hashing the workbook is O(cells): only after an edit, or a change made without one,
        // and not on every step of a scrub or drag (its end is the next frame's edit check)
        let dragging = !matches!(self.drag, Drag::None) || self.bar_scrub.is_some() || self.input_scrub.is_some();
        if self.dirty_stale || (self.eng.revision() != self.dirty_rev && !dragging) {
            self.dirty = self.is_dirty();
            self.dirty_stale = false;
            self.dirty_rev = self.eng.revision();
        }
        let title = format!("{}{} — {APP_NAME}", if self.dirty { "• " } else { "" }, self.display_name());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}
