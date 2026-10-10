mod commands;
mod files;
mod grid;
mod panels;
mod preferences;

pub use commands::Command;
#[cfg(test)]
mod tests;

use crate::chart_view::{PointHit, YAxis};
use crate::demo;
use crate::help_view::{self, Help, HelpAction, Page};
use crate::syntax;
use eframe::egui::RichText;
use wbs_core::help;
use eframe::egui::{
    self, text::CCursor, text::CCursorRange, Align2, Color32, CursorIcon, Event, FontId, Id, Key, Modifiers, Painter, PointerButton, Pos2,
    Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2,
};
use std::ops::Range;
use std::path::PathBuf;
use wbs_core::a1;
use wbs_core::engine::{Edit, Engine, ErrKind, Shown};
use wbs_core::ids::{CellKey, SheetId};
use wbs_core::model::{classify, Cell, Kind, Sheet, MAX_INDEX};
use wbs_core::ops::{self, Clip, Lit, Rect as CRect};
use wbs_core::parse::BUILTINS;
use wbs_core::value::{Prov, Value};

const HDR_W: f32 = 52.0;
const HDR_H: f32 = 22.0;
const DEF_W: f32 = 100.0;
const DEF_H: f32 = 24.0;
const FONT: f32 = 13.5;

struct Pal {
    bg: Color32,
    grid: Color32,
    hdr: Color32,
    hdr_sel: Color32,
    text: Color32,
    muted: Color32,
    sel_fill: Color32,
    sel: Color32,
    err: Color32,
    spill: Color32,
    decl: Color32,
    input_fill: Color32,
}

impl Pal {
    fn new(dark: bool) -> Pal {
        if dark {
            Pal {
                bg: Color32::from_rgb(0x17, 0x19, 0x1d),
                grid: Color32::from_rgb(0x2b, 0x2f, 0x36),
                hdr: Color32::from_rgb(0x20, 0x23, 0x28),
                hdr_sel: Color32::from_rgb(0x1e, 0x3a, 0x5f),
                text: Color32::from_gray(225),
                muted: Color32::from_gray(140),
                sel_fill: Color32::from_rgba_unmultiplied(0x3b, 0x82, 0xf6, 40),
                sel: Color32::from_rgb(0x3b, 0x82, 0xf6),
                err: Color32::from_rgb(0xf8, 0x71, 0x71),
                spill: Color32::from_rgb(0x60, 0xa5, 0xfa),
                decl: Color32::from_rgb(0xc0, 0x84, 0xfc),
                input_fill: Color32::from_rgba_unmultiplied(0xfa, 0xcc, 0x15, 28),
            }
        } else {
            Pal {
                bg: Color32::WHITE,
                grid: Color32::from_rgb(0xe3, 0xe5, 0xe8),
                hdr: Color32::from_rgb(0xf5, 0xf6, 0xf8),
                hdr_sel: Color32::from_rgb(0xdb, 0xe8, 0xfe),
                text: Color32::from_gray(25),
                muted: Color32::from_gray(120),
                sel_fill: Color32::from_rgba_unmultiplied(0x25, 0x63, 0xeb, 26),
                sel: Color32::from_rgb(0x25, 0x63, 0xeb),
                err: Color32::from_rgb(0xdc, 0x26, 0x26),
                spill: Color32::from_rgb(0x3b, 0x82, 0xf6),
                decl: Color32::from_rgb(0x7e, 0x22, 0xce),
                input_fill: Color32::from_rgba_unmultiplied(0xfa, 0xcc, 0x15, 45),
            }
        }
    }
}

struct Editing {
    key: CellKey,
    orig: String,
    text: String,
    /// Char index of the caret (tracked from whichever editor has focus).
    cursor: usize,
    /// Byte span of the reference being built by click/drag-to-reference.
    ref_span: Option<Range<usize>>,
    /// Which editor had focus last (formula bar or in-cell).
    in_bar: bool,
    /// The completion highlighted with ↓/↑ (none until the first ↓).
    pick: Option<usize>,
    /// The completion list was closed with Escape (until the text or caret changes).
    comp_closed: bool,
    /// An IME composition is in progress (its preedit text is part of `text`).
    composing: bool,
    /// Byte span of a composition started on a selected cell, held here until it's committed and the editor takes focus.
    preedit: Option<Range<usize>>,
}

enum Drag {
    None,
    Select,
    Ref { start: (usize, usize) },
    Fill { src: CRect, dst: CRect },
    /// Dragging the selection by its border: `grab` is the cell under the press; Alt copies instead.
    Move { src: CRect, grab: (usize, usize), dst: CRect, copy: bool },
    /// Resizing a column (row) by its header border: `w0` (`h0`) is the size at the press, `None` the default.
    Col { col: usize, x0: f32, w0: Option<f32> },
    Row { row: usize, y0: f32, h0: Option<f32> },
    Scrub { key: CellKey, orig: Option<Cell>, text: String, lit: Lit, x0: f32 },
    Point { key: CellKey, orig: Option<Cell>, text: String, lit: Lit, axis: YAxis, cell_disp: wbs_core::units::DispUnit },
    Goal(Box<GoalDrag>),
}

/// Dragging a computed chart point: goal-seeks `input` so element `index` of `target` lands at the pointer.
struct GoalDrag {
    target: CellKey,
    index: usize,
    input: CellKey,
    orig: Option<Cell>,
    axis: YAxis,
    /// Where the point was, where the press was, and the pointer now.
    at: Pos2,
    press: Pos2,
    pointer: Pos2,
    /// The input literal's decimals at the press; Shift (`fine`) writes two more.
    decimals: usize,
    fine: bool,
    /// False until the pointer leaves the press spot: a click switches the input instead.
    moved: bool,
    /// Solving every frame; turned off for the rest of the drag when one solve takes too long.
    live: bool,
    /// The target value under the pointer, in the axis's display unit.
    want: Option<f64>,
    /// The last solve: the input's new text, or why there's no answer.
    outcome: Option<Result<String, String>>,
    solve_ms: f64,
}

/// Per-frame grid geometry.
#[derive(Clone)]
struct Geo {
    cells: Rect,
    col_x: Vec<f32>,
    row_y: Vec<f32>,
    scroll: Vec2,
}

impl Geo {
    fn x(&self, c: usize) -> f32 {
        self.cells.left() + self.col_x[c.min(self.col_x.len() - 1)] - self.scroll.x
    }
    fn y(&self, r: usize) -> f32 {
        self.cells.top() + self.row_y[r.min(self.row_y.len() - 1)] - self.scroll.y
    }
    fn rect(&self, r0: usize, c0: usize, r1: usize, c1: usize) -> Rect {
        Rect::from_min_max(Pos2::new(self.x(c0), self.y(r0)), Pos2::new(self.x(c1 + 1), self.y(r1 + 1)))
    }
    fn cell(&self, r: usize, c: usize) -> Rect {
        self.rect(r, c, r, c)
    }
    fn col_at(&self, x: f32) -> usize {
        let v = x - self.cells.left() + self.scroll.x;
        match self.col_x.binary_search_by(|p| p.partial_cmp(&v).unwrap()) {
            Ok(i) => i.min(self.col_x.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.col_x.len() - 2),
        }
    }
    fn row_at(&self, y: f32) -> usize {
        let v = y - self.cells.top() + self.scroll.y;
        match self.row_y.binary_search_by(|p| p.partial_cmp(&v).unwrap()) {
            Ok(i) => i.min(self.row_y.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.row_y.len() - 2),
        }
    }
    fn visible(&self) -> (Range<usize>, Range<usize>) {
        let r0 = self.row_at(self.cells.top());
        let r1 = (self.row_at(self.cells.bottom()) + 1).min(self.row_y.len() - 1);
        let c0 = self.col_at(self.cells.left());
        let c1 = (self.col_at(self.cells.right()) + 1).min(self.col_x.len() - 1);
        (r0..r1, c0..c1)
    }
}

pub struct App {
    eng: Engine,
    sheet_ix: usize,
    anchor: (usize, usize),
    cursor: (usize, usize),
    edit: Option<Editing>,
    /// (pass number at which to apply, caret char index)
    focus_req: Option<(u64, usize)>,
    scroll: Vec2,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    clip: Option<Clip>,
    /// Cells marked by ⌘X, with the document fingerprint then: the next paste moves them if nothing changed since.
    cut: Option<(CRect, u64)>,
    drag: Drag,
    offer: Option<(CellKey, CRect)>,
    status: Option<String>,
    /// `None`: untitled (Save asks where).
    path: Option<PathBuf>,
    dialogs: Box<dyn files::Dialogs>,
    /// `files::fingerprint` of the workbook as last opened/saved.
    saved_fp: u64,
    /// Cached `is_dirty()` for the title; refreshed on input.
    dirty: bool,
    dirty_stale: bool,
    title: String,
    /// The save-changes prompt is up, for this action.
    confirm: Option<files::Pending>,
    /// Unsaved changes dealt with: let the window close.
    close_ok: bool,
    queued: Vec<Command>,
    /// Synthetic input for the next frame (menu commands replayed as keys).
    inject: Vec<Event>,
    /// System fonts for CJK, Arabic etc., loaded once such text appears.
    fonts: crate::fonts::Fallbacks,
    native_menu: Option<crate::menus::NativeMenu>,
    name_buf: String,
    name_for: Option<CellKey>,
    chart_hits: Vec<(PointHit, YAxis)>,
    /// The input a computed cell's chart points goal-seek, when switched from the default (click the point).
    goal_inputs: std::collections::HashMap<CellKey, CellKey>,
    /// Why the last goal-seek found no answer, shown at its point (target, element) until the next press.
    goal_note: Option<(CellKey, usize, String)>,
    /// A goal-seek slower than this stops following the pointer and solves on release.
    goal_live_ms: f64,
    editor_rect: Option<Rect>,
    trace: bool,
    tabs: panels::Tabs,
    last_recalc_ms: f64,
    scroll_into_view: bool,
    /// Formula-bar galley from last frame, for alt-scrub hit testing.
    bar_galley: Option<(std::sync::Arc<egui::Galley>, Pos2)>,
    bar_scrub: Option<(CellKey, Option<Cell>, String, Lit, f32)>,
    input_scrub: Option<(CellKey, Option<Cell>)>,
    /// Last frame's grid geometry (used by UI tests to aim at cells).
    #[cfg_attr(not(test), allow(dead_code))]
    geo: Option<Geo>,
    help: Help,
    prefs: crate::prefs::Prefs,
    settings: crate::settings_view::SettingsWindow,
    autosave: preferences::Autosave,
}

impl App {
    /// On first run (no saved file) open help at the welcome guide.
    pub fn with_welcome(mut self) -> App {
        if !self.path.as_ref().is_some_and(|p| p.exists()) {
            self.help.show_page(Page::Topic("welcome"));
        }
        self
    }

    pub fn new(path: PathBuf) -> App {
        let (eng, status) = match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(wb) => (Engine::new(wb), format!("opened {}", path.display())),
                Err(e) => (demo::workbook(), format!("couldn't read {}: {e} — showing the demo", path.display())),
            },
            Err(_) => (demo::workbook(), format!("demo workbook — ⌘S saves to {}", path.display())),
        };
        let mut app = App {
            eng,
            sheet_ix: 0,
            anchor: (3, 1),
            cursor: (3, 1),
            edit: None,
            focus_req: None,
            scroll: Vec2::ZERO,
            undo: Vec::new(),
            redo: Vec::new(),
            clip: None,
            cut: None,
            drag: Drag::None,
            offer: None,
            status: Some(status),
            path: Some(path),
            dialogs: files::default_dialogs(),
            saved_fp: 0,
            dirty: false,
            dirty_stale: false,
            title: String::new(),
            confirm: None,
            close_ok: false,
            queued: Vec::new(),
            inject: Vec::new(),
            fonts: Default::default(),
            native_menu: None,
            name_buf: String::new(),
            name_for: None,
            chart_hits: Vec::new(),
            goal_inputs: Default::default(),
            goal_note: None,
            goal_live_ms: 50.0,
            editor_rect: None,
            trace: true,
            tabs: Default::default(),
            last_recalc_ms: 0.0,
            scroll_into_view: false,
            bar_galley: None,
            bar_scrub: None,
            input_scrub: None,
            geo: None,
            help: Help::new(),
            prefs: crate::prefs::Prefs::in_memory(),
            settings: Default::default(),
            autosave: Default::default(),
        };
        // the demo (or the opened file) is the clean state
        app.mark_clean();
        app
    }

    /// Where the help window opens, from `Help::geometry` saved last run.
    pub fn with_help_geometry(mut self, g: Option<String>) -> App {
        if let Some(g) = g {
            self.help.set_geometry(&g);
        }
        self
    }

    /// Use the native menu bar where there is one (macOS).
    pub fn with_native_menu(mut self, ctx: &egui::Context) -> App {
        self.native_menu = crate::menus::NativeMenu::install(ctx);
        self
    }

    // ---- helpers ------------------------------------------------------------

    fn sheet(&self) -> &Sheet {
        &self.eng.wb.sheets[self.sheet_ix]
    }
    fn sid(&self) -> SheetId {
        self.sheet().id
    }
    /// Any cell, stored or not: rows past the stored ones are virtual, with ids of their own.
    fn key(&self, r: usize, c: usize) -> CellKey {
        self.sheet().key(r, c).expect("selection within MAX_INDEX")
    }
    /// Rows and columns the grid lays out at least: the default size, the stored rows and the
    /// content, and the selection with a margin (the grid adds room to scroll into). None of it is
    /// in the document, so moving around and scrolling never change it.
    fn extent(&self) -> (usize, usize) {
        let s = self.sheet();
        let (ur, uc) = s.used_extent();
        let r = [200, s.rows.len(), ur, self.cursor.0.max(self.anchor.0) + 30].into_iter().max().unwrap();
        let c = [26, s.cols.len(), uc, self.cursor.1.max(self.anchor.1) + 6].into_iter().max().unwrap();
        (r.min(MAX_INDEX), c.min(MAX_INDEX))
    }
    fn sel(&self) -> CRect {
        CRect::span(self.sid(), self.anchor, self.cursor)
    }
    fn label(&self, k: CellKey) -> String {
        self.eng.wb.cell_label(k, Some(self.sid()))
    }

    fn exec(&mut self, e: Edit) {
        self.cut = None;
        self.goal_note = None;
        let t = std::time::Instant::now();
        let inv = self.apply_edit(e);
        self.last_recalc_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.undo.push(inv);
        self.redo.clear();
    }

    fn undo(&mut self) {
        self.cut = None;
        if let Some(e) = self.undo.pop() {
            let inv = self.apply_edit(e);
            self.redo.push(inv);
        }
    }
    fn redo(&mut self) {
        self.cut = None;
        if let Some(e) = self.redo.pop() {
            let inv = self.apply_edit(e);
            self.undo.push(inv);
        }
    }

    /// Applies an edit and keeps `sheet_ix` valid: it follows the sheet that was showing, or the
    /// sheet a sheet edit added, moved or renamed; if the showing sheet was deleted, its neighbour.
    fn apply_edit(&mut self, e: Edit) -> Edit {
        let prev = self.sid();
        let follow = match &e {
            Edit::InsertSheet { sheet, .. } => sheet.id,
            Edit::RestoreSheet { sheet, .. } | Edit::MoveSheet { sheet, .. } | Edit::SheetPos { sheet, .. } | Edit::RenameSheet { sheet, .. } => *sheet,
            _ => prev,
        };
        let inv = self.eng.apply(e);
        let wb = &self.eng.wb;
        let ix = wb.sheet_index(follow).or(wb.sheet_index(prev)).unwrap_or(self.sheet_ix.min(wb.sheets.len() - 1));
        self.show_sheet(ix, prev);
        if self.edit.as_ref().is_some_and(|ed| self.eng.wb.sheet(ed.key.sheet).is_none()) {
            self.edit = None;
        }
        inv
    }

    /// Switches to the sheet at `ix`; the selection and scroll reset unless it is still `prev`.
    fn show_sheet(&mut self, ix: usize, prev: SheetId) {
        self.sheet_ix = ix;
        if self.sid() != prev {
            self.anchor = (0, 0);
            self.cursor = (0, 0);
            self.scroll = Vec2::ZERO;
        }
    }

    fn select(&mut self, r: usize, c: usize, extend: bool) {
        let (r, c) = (r.min(MAX_INDEX - 1), c.min(MAX_INDEX - 1));
        self.cursor = (r, c);
        if !extend {
            self.anchor = (r, c);
        }
    }

    fn move_sel(&mut self, dr: i64, dc: i64, extend: bool) {
        let r = (self.cursor.0 as i64 + dr).max(0) as usize;
        let c = (self.cursor.1 as i64 + dc).max(0) as usize;
        self.select(r, c, extend);
        self.scroll_into_view = true;
    }

    fn start_edit(&mut self, ctx: &egui::Context, r: usize, c: usize, text: Option<String>, in_bar: bool) {
        let k = self.key(r, c);
        if self.eng.kind(k) == Kind::Empty {
            if let Some(a) = self.eng.spill_anchor(k) {
                self.status = Some(format!("spilled cells are read-only — edit the source {}", self.label(a)));
                return;
            }
        }
        let orig = self.eng.wb.cell_text(k);
        let text = text.unwrap_or_else(|| orig.clone());
        let cursor = text.chars().count();
        self.edit = Some(Editing { key: k, orig, text, cursor, ref_span: None, in_bar, pick: None, comp_closed: false, composing: false, preedit: None });
        self.focus_req = Some((ctx.cumulative_pass_nr() + 1, cursor));
        self.offer = None;
    }

    fn commit(&mut self) {
        let Some(ed) = self.edit.take() else { return };
        if ed.text != ed.orig {
            self.cut = None;
            let t = std::time::Instant::now();
            let inv = self.eng.set_text(ed.key, &ed.text);
            self.last_recalc_ms = t.elapsed().as_secs_f64() * 1000.0;
            self.undo.push(inv);
            self.redo.clear();
            self.offer = ops::extension_offer(&self.eng, ed.key).map(|r| (ed.key, r));
        }
    }

    fn cancel_edit(&mut self) {
        self.edit = None;
    }

    fn editing_formula(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| classify(&e.text).has_refs())
    }

    /// Click-to-reference: insert (or update) a reference token at the caret.
    fn insert_ref(&mut self, ctx: &egui::Context, a: (usize, usize), b: (usize, usize)) {
        let home = self.edit.as_ref().unwrap().key.sheet;
        let here = self.sid();
        let mut s = a1::cell_name(a.0.min(b.0), a.1.min(b.1));
        if a != b {
            s = format!("{s}:{}", a1::cell_name(a.0.max(b.0), a.1.max(b.1)));
        }
        if here != home {
            s = format!("{}{s}", a1::sheet_prefix(&self.sheet().name));
        }
        let ed = self.edit.as_mut().unwrap();
        let span = match ed.ref_span.clone() {
            Some(sp) => sp,
            None => {
                let byte = ed.text.char_indices().nth(ed.cursor).map(|(i, _)| i).unwrap_or(ed.text.len());
                let before = ed.text[..byte].chars().last();
                let after = ed.text[byte..].chars().next();
                // tokens are space separated: pad before (except right after a
                // leading `=`), and always leave a space after so typing continues
                let mut ins = String::new();
                let after_marker = byte == 1 && matches!(before, Some('=') | Some(':'));
                if before.is_some_and(|c| !c.is_whitespace()) && !after_marker {
                    ins.push(' ');
                }
                let start = byte + ins.len();
                if !after.is_some_and(|c| c.is_whitespace()) {
                    ed.text.insert(byte, ' ');
                }
                ed.text.insert_str(byte, &ins);
                start..start
            }
        };
        ed.text.replace_range(span.clone(), &s);
        let end = span.start + s.len();
        ed.ref_span = Some(span.start..end);
        ed.cursor = ed.text[..end].chars().count() + 1;
        self.focus_req = Some((ctx.cumulative_pass_nr() + 1, ed.cursor));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.run_queued(&ctx);
        if self.confirm.is_none() {
            self.command_keys(&ctx);
            self.keys(&ctx);
        }
        let dark = ui.visuals().dark_mode;
        if self.native_menu.is_none() {
            egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        }
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("formula_bar").show(ui, |ui| self.formula_bar(ui, dark));
        egui::Panel::bottom("statusbar").show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("inspector").resizable(true).default_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
        });
        egui::CentralPanel::no_frame().show(ui, |ui| self.grid(ui, dark));
        let home = self.sid();
        let keys = if self.confirm.is_none() { self.owned_keys() } else { &[] };
        for a in self.help.ui(&ctx, &self.eng, home, keys) {
            match a {
                HelpAction::Goto(k) => {
                    self.goto(k);
                    if !self.help.embedded() {
                        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                    }
                }
                HelpAction::Command(c) => self.run(&ctx, c),
            }
        }
        self.settings_ui(&ctx);
        self.document_ui(&ctx);
        self.autosave(&ctx);
        self.fonts.load_if_wanted(&ctx);
        if let Some(m) = &self.native_menu {
            m.sync(self.trace, !self.undo.is_empty(), !self.redo.is_empty());
        }
        if !self.inject.is_empty() {
            ctx.request_repaint();
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.events.append(&mut self.inject);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(p) = self.path.as_ref().and_then(|p| std::path::absolute(p).ok()) {
            storage.set_string(LAST_FILE_KEY, p.display().to_string());
        }
        if let Some(g) = self.help.geometry() {
            storage.set_string(HELP_WINDOW_KEY, g);
        }
    }
}

/// eframe storage key for the most recently opened/saved workbook.
pub const LAST_FILE_KEY: &str = "last_file";
/// eframe storage key for the help window's position and size.
pub const HELP_WINDOW_KEY: &str = "help_window";
