//! Edit ▸ Paste CSV: the clipboard read as CSV, previewed before it's written. Plain ⌘V never guesses
//! that text is CSV; this is the one way to say it is.

use super::*;
use wbs_core::csv::{self, Sep};

/// Pastes bigger than this get a warning in the preview.
const BIG: usize = 100_000;
const PREVIEW_ROWS: usize = 8;
/// Columns past this aren't shown (nor can they get a unit).
const PREVIEW_COLS: usize = 40;

/// The preview: the clipboard text and how it's read.
pub(super) struct CsvPaste {
    text: String,
    sep: Sep,
    header: bool,
    rows: Vec<Vec<String>>,
    at: (usize, usize),
    /// The unit typed for each column (blank: none).
    units: Vec<String>,
    /// Per column: how many fields its unit goes on and how many it doesn't (`csv::unit_coverage`),
    /// worked out when the unit, separator or header row changes.
    coverage: Vec<Option<(usize, usize)>>,
}

impl CsvPaste {
    fn set_sep(&mut self, sep: Sep) {
        if sep != self.sep {
            self.sep = sep;
            self.rows = csv::parse(&self.text, sep);
            self.coverage.iter_mut().for_each(|c| *c = None);
        }
    }
}

impl App {
    /// Paste CSV: ask the integration for the clipboard; the Paste event it sends back opens the preview.
    pub(super) fn request_csv_paste(&mut self, ctx: &egui::Context) {
        self.csv_wanted = Some(ctx.cumulative_pass_nr());
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }

    /// Whether a Paste event this frame answers a Paste CSV (it arrives within a frame or two). Called
    /// for every frame with a Paste event, so a request the clipboard never answered doesn't linger.
    pub(super) fn take_csv_request(&mut self, ctx: &egui::Context) -> bool {
        self.csv_wanted.take().is_some_and(|p| ctx.cumulative_pass_nr() <= p + 3)
    }

    /// Opens the preview for `text`, to land at the selection's top-left.
    pub(super) fn open_csv_paste(&mut self, text: &str) {
        let sep = Sep::default();
        let rows = csv::parse(text, sep);
        if rows.is_empty() {
            self.status = Some("nothing to paste".into());
            return;
        }
        let at = (self.anchor.0.min(self.cursor.0), self.anchor.1.min(self.cursor.1));
        self.csv = Some(CsvPaste { text: text.to_string(), sep, header: false, rows, at, units: Vec::new(), coverage: Vec::new() });
    }

    /// The preview window, while it's up: separator, header row, the first rows as they'll land.
    pub(super) fn csv_paste_ui(&mut self, ctx: &egui::Context, dark: bool) {
        let Some(p) = self.csv.as_mut() else { return };
        let pal = Pal::new(dark);
        let (rows, cols) = csv::size(&p.rows);
        let shown_cols = cols.min(PREVIEW_COLS);
        p.units.resize(shown_cols.max(p.units.len()), String::new());
        p.coverage.resize(p.units.len(), None);
        // a unit that isn't defined here, per column
        let errs: Vec<Option<String>> = p.units.iter().map(|u| csv::column_unit(u).and_then(|u| self.eng.check_unit(u).err())).collect();
        for j in 0..shown_cols {
            if p.coverage[j].is_none() && csv::column_unit(&p.units[j]).is_some() {
                p.coverage[j] = Some(csv::unit_coverage(&p.rows, p.header, j));
            }
        }
        let bad_unit = errs.iter().take(shown_cols).any(Option::is_some);
        let mut choice = None;
        let modal = egui::Modal::new(Id::new("paste_csv")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Paste CSV");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Separator");
                for s in Sep::ALL {
                    if ui.selectable_label(p.sep == s, s.label()).clicked() {
                        p.set_sep(s);
                    }
                }
            });
            if ui.checkbox(&mut p.header, "First row is a header (pasted as text)").changed() {
                p.coverage.iter_mut().for_each(|c| *c = None);
            }
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    egui::Grid::new("paste_csv_rows").striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                        // a unit per column, for its plain numbers
                        for j in 0..shown_cols {
                            let field = TextEdit::singleline(&mut p.units[j]).hint_text("unit").desired_width(80.0);
                            let r = ui.add(field).on_hover_text("A unit for this column's numbers, like USD or m/s");
                            if r.changed() {
                                p.coverage[j] = None;
                            }
                        }
                        ui.end_row();
                        for j in 0..shown_cols {
                            match (&errs[j], p.coverage[j]) {
                                (Some(e), _) => ui.colored_label(pal.err, "unknown unit").on_hover_text(e),
                                (None, Some((gets, not))) => ui
                                    .colored_label(pal.muted, format!("{gets} get it, {not} don't"))
                                    .on_hover_text("Plain numbers get the unit; text, dates and numbers with a unit of their own stay as they are"),
                                _ => ui.label(""),
                            };
                        }
                        ui.end_row();
                        for (i, row) in p.rows.iter().take(PREVIEW_ROWS).enumerate() {
                            for j in 0..shown_cols {
                                let field = row.get(j).map(String::as_str).unwrap_or("");
                                preview_cell(ui, &pal, field, p.header && i == 0, csv::column_unit(&p.units[j]));
                            }
                            if cols > PREVIEW_COLS {
                                ui.colored_label(pal.muted, "…");
                            }
                            ui.end_row();
                        }
                    });
                });
                if rows > PREVIEW_ROWS {
                    ui.colored_label(pal.muted, format!("… {} more rows", rows - PREVIEW_ROWS));
                }
                if cols > PREVIEW_COLS {
                    ui.colored_label(pal.muted, format!("… {} more columns (pasted without units)", cols - PREVIEW_COLS));
                }
            });
            ui.add_space(4.0);
            ui.label(format!("{rows} × {cols} at {}. Numbers and ISO dates become numbers; everything else is text.", a1::cell_name(p.at.0, p.at.1)));
            if rows * cols > BIG {
                ui.colored_label(pal.err, format!("That's {} cells: pasting may take a while.", rows * cols));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let paste = ui.add_enabled(!bad_unit, egui::Button::new("Paste"));
                if paste.clicked() || (!bad_unit && ui.input(|i| i.key_pressed(Key::Enter))) {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                let p = self.csv.take().unwrap();
                let edit = csv::paste(&self.eng, &p.rows, p.header, &p.units, self.sid(), p.at);
                self.exec(edit);
                self.anchor = p.at;
                self.cursor = (p.at.0 + rows - 1, p.at.1 + cols.max(1) - 1);
                self.clip = None;
                self.status = Some(format!("pasted {rows}×{cols} as CSV"));
            }
            Some(false) => self.csv = None,
            None => {}
        }
    }
}

/// One field as it will land: numbers and dates in monospace, header and text cells plain, and what was
/// quoted so it doesn't run (`=A1`, `dim x`) muted.
fn preview_cell(ui: &mut Ui, pal: &Pal, field: &str, header: bool, unit: Option<&str>) {
    let cut = |s: &str| {
        let mut shown: String = s.split('\n').next().unwrap_or("").chars().take(18).collect();
        if shown.len() < s.len() {
            shown.push('…');
        }
        shown
    };
    let shown = cut(field);
    let text = match csv::cell_text(field, header, unit) {
        None => RichText::new(""),
        Some(_) if header => RichText::new(shown).strong(),
        // what lands, unit included
        Some(t) if classify(&t) == Kind::Number => RichText::new(cut(&t)).monospace(),
        Some(t) if t.len() > field.len() && classify(field).has_refs() => RichText::new(shown).color(pal.muted).italics(),
        Some(_) => RichText::new(shown),
    };
    ui.label(text);
}
