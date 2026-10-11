//! Edit ▸ Paste CSV: the clipboard read as CSV, previewed before it's written. Plain ⌘V never guesses
//! that text is CSV; this is the one way to say it is.

use super::*;
use wbs_core::csv::{self, Sep};

/// Pastes bigger than this get a warning in the preview.
const BIG: usize = 100_000;
const PREVIEW_ROWS: usize = 8;
const PREVIEW_COLS: usize = 8;

/// The preview: the clipboard text and how it's read.
pub(super) struct CsvPaste {
    text: String,
    sep: Sep,
    header: bool,
    rows: Vec<Vec<String>>,
    at: (usize, usize),
}

impl CsvPaste {
    fn set_sep(&mut self, sep: Sep) {
        if sep != self.sep {
            self.sep = sep;
            self.rows = csv::parse(&self.text, sep);
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
        self.csv = Some(CsvPaste { text: text.to_string(), sep, header: false, rows, at });
    }

    /// The preview window, while it's up: separator, header row, the first rows as they'll land.
    pub(super) fn csv_paste_ui(&mut self, ctx: &egui::Context, dark: bool) {
        let Some(p) = self.csv.as_mut() else { return };
        let pal = Pal::new(dark);
        let (rows, cols) = csv::size(&p.rows);
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
            ui.checkbox(&mut p.header, "First row is a header (pasted as text)");
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("paste_csv_rows").striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (i, row) in p.rows.iter().take(PREVIEW_ROWS).enumerate() {
                        for j in 0..cols.min(PREVIEW_COLS) {
                            let field = row.get(j).map(String::as_str).unwrap_or("");
                            preview_cell(ui, &pal, field, p.header && i == 0);
                        }
                        if cols > PREVIEW_COLS {
                            ui.colored_label(pal.muted, "…");
                        }
                        ui.end_row();
                    }
                });
                if rows > PREVIEW_ROWS {
                    ui.colored_label(pal.muted, format!("… {} more rows", rows - PREVIEW_ROWS));
                }
            });
            ui.add_space(4.0);
            ui.label(format!("{rows} × {cols} at {}. Numbers and ISO dates become numbers; everything else is text.", a1::cell_name(p.at.0, p.at.1)));
            if rows * cols > BIG {
                ui.colored_label(pal.err, format!("That's {} cells: pasting may take a while.", rows * cols));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Paste").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
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
                let edit = csv::paste(&self.eng, &p.rows, p.header, self.sid(), p.at);
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
fn preview_cell(ui: &mut Ui, pal: &Pal, field: &str, header: bool) {
    let mut shown: String = field.split('\n').next().unwrap_or("").chars().take(18).collect();
    if shown.len() < field.len() {
        shown.push('…');
    }
    let text = match csv::cell_text(field, header) {
        None => RichText::new(""),
        Some(_) if header => RichText::new(shown).strong(),
        Some(t) if classify(&t) == Kind::Number => RichText::new(shown).monospace(),
        Some(t) if t.len() > field.len() && classify(field).has_refs() => RichText::new(shown).color(pal.muted).italics(),
        Some(_) => RichText::new(shown),
    };
    ui.label(text);
}
