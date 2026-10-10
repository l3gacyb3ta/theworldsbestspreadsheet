//! The help window: guides, reference, live units/words pages, search and
//! the playground. Content comes from `wbs_core::help`.
//!
//! Help is its own OS window (an immediate egui viewport, so it can borrow the
//! engine). Where viewports are embedded (headless tests, backends without
//! multi-window support) it is an `egui::Window` inside the main window.

use crate::app::Command;
use crate::syntax;
use eframe::egui::{self, Color32, Event, FontId, Rect, RichText, TextEdit, Ui, ViewportCommand, ViewportId};
use wbs_core::engine::{Engine, Scratch};
use wbs_core::help::{self, Block, Category, Hit, Inline, WordDoc, ERRORS, SAMPLE, SAMPLE_NAMES, WORDS};
use wbs_core::ids::{CellKey, SheetId};
use wbs_core::value::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Topic(&'static str),
    Word(&'static str),
    Reference,
    Units,
    YourWords,
    Playground,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PlayCtx {
    Sample,
    Workbook,
}

/// What the help window asks the app to do.
pub enum HelpAction {
    Goto(CellKey),
    /// A document shortcut (⌘S, ⌘O…) pressed while the help window had focus.
    Command(Command),
}

pub struct Help {
    pub open: bool,
    page: Page,
    back: Vec<Page>,
    fwd: Vec<Page>,
    query: String,
    sample: (Engine, SheetId),
    play: String,
    play_ctx: PlayCtx,
    focus_search: bool,
    scroll_top: bool,
    /// Drawn inside the main window rather than as its own OS window.
    embedded: bool,
    /// The OS window has keyboard focus.
    focused: bool,
    /// Bring the OS window to the front on the next frame.
    raise: bool,
    /// Edit commands from the menu bar to replay in the help window.
    forward: Vec<Command>,
    /// The OS window's outer position and inner size, and where it reopens.
    geom: Option<Rect>,
    restore: Option<Rect>,
}

const MONO: f32 = 13.0;
pub const TITLE: &str = "Help — the world's best spreadsheet";

fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("help")
}

impl Help {
    pub fn new() -> Help {
        Help {
            open: false,
            page: Page::Topic("welcome"),
            back: Vec::new(),
            fwd: Vec::new(),
            query: String::new(),
            sample: help::sample_engine(),
            play: "=A1:A5 2 * sum".into(),
            play_ctx: PlayCtx::Sample,
            focus_search: false,
            scroll_top: false,
            embedded: true,
            focused: false,
            raise: false,
            forward: Vec::new(),
            geom: None,
            restore: None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Opens help at `page`, recording history.
    pub fn show_page(&mut self, page: Page) {
        if self.page != page {
            self.back.push(std::mem::replace(&mut self.page, page));
            self.fwd.clear();
        }
        self.query.clear();
        self.scroll_top = true;
        self.show();
    }

    /// Opens help where it was, in front.
    pub fn show(&mut self) {
        self.open = true;
        self.raise = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.focused = false;
        self.forward.clear();
        if self.geom.is_some() {
            self.restore = self.geom;
        }
    }

    /// Help is drawn inside the main window (as of the last frame).
    pub fn embedded(&self) -> bool {
        self.embedded
    }

    /// The help window is its own OS window and has keyboard focus.
    pub fn has_focus(&self) -> bool {
        self.open && !self.embedded && self.focused
    }

    /// Hands an edit command (undo, copy…) to the focused help window.
    pub fn forward(&mut self, c: Command) {
        self.forward.push(c);
    }

    /// "x y w h": the OS window's position and size, for eframe storage.
    pub fn geometry(&self) -> Option<String> {
        self.geom.or(self.restore).map(|r| format!("{} {} {} {}", r.min.x, r.min.y, r.width(), r.height()))
    }

    pub fn set_geometry(&mut self, s: &str) {
        let v: Vec<f32> = s.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        if let [x, y, w, h] = v[..] {
            if w >= 200.0 && h >= 150.0 {
                self.restore = Some(Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)));
            }
        }
    }

    pub fn try_in_playground(&mut self, program: &str, ctx: PlayCtx) {
        self.play = if program.starts_with('=') { program.to_string() } else { format!("={program}") };
        self.play_ctx = ctx;
        self.show_page(Page::Playground);
    }

    pub fn focus_search(&mut self) {
        self.show();
        self.focus_search = true;
    }

    fn navigate(&mut self, target: &str) {
        match target.strip_prefix('#') {
            Some(id) => {
                if let Some(t) = help::topics().into_iter().find(|t| t.id == id) {
                    self.show_page(Page::Topic(t.id));
                }
            }
            None => {
                if let Some(w) = help::word_doc(target) {
                    self.show_page(Page::Word(w.name));
                }
            }
        }
    }

    /// Shows help if it's open. `keys` are the app shortcuts to act on when
    /// they're pressed in the help window: F1 closes it, ⌘/ searches, the rest go to the app.
    pub fn ui(&mut self, ctx: &egui::Context, eng: &Engine, home: SheetId, keys: &[Command]) -> Vec<HelpAction> {
        let mut actions = Vec::new();
        self.embedded = ctx.embed_viewports();
        if !self.open {
            return actions;
        }
        if self.embedded {
            self.raise = false;
            let mut open = self.open;
            let screen = ctx.content_rect();
            egui::Window::new("Help")
                .open(&mut open)
                .default_size([820.0, 660.0])
                .default_pos([(screen.right() - 840.0).max(screen.left() + 10.0), screen.top() + 60.0])
                .min_width(520.0)
                .max_width(1000.0)
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| self.contents(ui, eng, home, &mut actions));
            if !open {
                self.close();
            }
            return actions;
        }
        let mut vb = egui::ViewportBuilder::default().with_title(TITLE).with_min_inner_size([520.0, 360.0]);
        vb = match self.restore {
            Some(r) => vb.with_position(r.min).with_inner_size(r.size()),
            None => vb.with_inner_size([880.0, 720.0]),
        };
        ctx.show_viewport_immediate(viewport_id(), vb, |ui, class| {
            let ctx = ui.ctx().clone();
            // in the fallback (a backend without multi-window support) input is the main window's and handled there
            if class == egui::ViewportClass::Immediate {
                let (close, focused, outer, inner) = ctx.input(|i| {
                    let v = i.viewport();
                    (v.close_requested(), v.focused.unwrap_or(false), v.outer_rect, v.inner_rect)
                });
                self.focused = focused;
                if let (Some(o), Some(i)) = (outer, inner) {
                    self.geom = Some(Rect::from_min_size(o.min, i.size()));
                }
                if std::mem::take(&mut self.raise) {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                for c in std::mem::take(&mut self.forward) {
                    match c {
                        Command::Cut => ctx.send_viewport_cmd(ViewportCommand::RequestCut),
                        Command::Copy => ctx.send_viewport_cmd(ViewportCommand::RequestCopy),
                        Command::Paste => ctx.send_viewport_cmd(ViewportCommand::RequestPaste),
                        // the menu's key equivalent swallowed the keystroke: replay it for the focused field
                        c => {
                            if let Some(s) = c.shortcut() {
                                let ev = Event::Key { key: s.logical_key, physical_key: None, pressed: true, repeat: false, modifiers: s.modifiers };
                                ctx.input_mut(|i| i.events.push(ev));
                            }
                        }
                    }
                }
                for &c in keys {
                    let Some(s) = c.shortcut() else { continue };
                    if ctx.input_mut(|i| i.consume_shortcut(&s)) {
                        match c {
                            Command::Help => self.close(),
                            Command::SearchHelp => self.focus_search(),
                            c => actions.push(HelpAction::Command(c)),
                        }
                    }
                }
                if close {
                    self.close();
                }
            }
            egui::CentralPanel::default().show(ui, |ui| self.contents(ui, eng, home, &mut actions));
        });
        actions
    }

    fn contents(&mut self, ui: &mut Ui, eng: &Engine, home: SheetId, actions: &mut Vec<HelpAction>) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.back.is_empty(), egui::Button::new("◀")).on_hover_text("back").clicked() {
                let p = self.back.pop().unwrap();
                self.fwd.push(std::mem::replace(&mut self.page, p));
                self.scroll_top = true;
            }
            if ui.add_enabled(!self.fwd.is_empty(), egui::Button::new("▶")).on_hover_text("forward").clicked() {
                let p = self.fwd.pop().unwrap();
                self.back.push(std::mem::replace(&mut self.page, p));
                self.scroll_top = true;
            }
            let r = ui.add(TextEdit::singleline(&mut self.query).hint_text("Search help — words, units, topics…").desired_width(f32::INFINITY));
            if self.focus_search {
                r.request_focus();
                self.focus_search = false;
            }
        });
        ui.separator();
        egui::Panel::left("help_nav").resizable(false).exact_size(186.0).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("help_nav_scroll").show(ui, |ui| self.nav(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let mut sa = egui::ScrollArea::vertical().id_salt("help_page_scroll").auto_shrink([false, false]);
            if self.scroll_top {
                sa = sa.vertical_scroll_offset(0.0);
                self.scroll_top = false;
            }
            sa.show(ui, |ui| {
                ui.set_max_width(640.0);
                if !self.query.trim().is_empty() {
                    self.search_page(ui, eng, actions);
                    return;
                }
                match self.page.clone() {
                    Page::Topic(id) => self.topic_page(ui, id),
                    Page::Word(name) => self.word_page(ui, name),
                    Page::Reference => self.reference_page(ui),
                    Page::Units => self.units_page(ui, eng, actions),
                    Page::YourWords => self.your_words_page(ui, eng, actions),
                    Page::Playground => self.playground_page(ui, eng, home),
                }
            });
        });
    }

    fn nav(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("GUIDES").small().weak());
        for t in help::topics() {
            let on = self.page == Page::Topic(t.id) && self.query.is_empty();
            if ui.selectable_label(on, t.title).clicked() {
                self.show_page(Page::Topic(t.id));
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("REFERENCE").small().weak());
        for (page, label) in [
            (Page::Reference, "All words & syntax"),
            (Page::Units, "Units in this workbook"),
            (Page::YourWords, "Your words"),
            (Page::Playground, "Playground"),
        ] {
            let on = self.page == page && self.query.is_empty();
            if ui.selectable_label(on, label).clicked() {
                self.show_page(page);
            }
        }
    }

    // ---- pages ----------------------------------------------------------------

    fn topic_page(&mut self, ui: &mut Ui, id: &'static str) {
        let Some(t) = help::topic(id) else { return };
        ui.heading(t.title);
        ui.add_space(6.0);
        for b in help::parse_markdown(t.body) {
            self.block(ui, &b);
        }
        // previous / next guide
        let all = help::topics();
        let i = all.iter().position(|x| x.id == id).unwrap_or(0);
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            if i > 0 && ui.link(format!("← {}", all[i - 1].title)).clicked() {
                self.show_page(Page::Topic(all[i - 1].id));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if i + 1 < all.len() && ui.link(format!("{} →", all[i + 1].title)).clicked() {
                    self.show_page(Page::Topic(all[i + 1].id));
                }
            });
        });
    }

    fn word_page(&mut self, ui: &mut Ui, name: &'static str) {
        let Some(w) = help::word_doc(name) else { return };
        ui.horizontal(|ui| {
            ui.label(RichText::new(w.name).monospace().size(22.0).strong());
            ui.add_space(8.0);
            ui.label(RichText::new(w.category.title()).weak());
        });
        ui.label(RichText::new(w.effect).monospace().size(MONO + 1.0).color(syntax::C_WORD));
        ui.add_space(6.0);
        self.inline_para(ui, w.summary);
        if !w.units.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Units").strong());
                ui.label(w.units);
            });
        }
        if !w.details.is_empty() {
            ui.add_space(4.0);
            self.inline_para(ui, w.details);
        }
        if !w.examples.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Examples").strong());
            let lines: Vec<(String, String)> = w.examples.iter().map(|(p, r)| (p.to_string(), r.to_string())).collect();
            self.examples(ui, &lines);
        }
        if !w.see.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("See also").strong());
                for s in w.see {
                    let label = match s.strip_prefix('#') {
                        Some(id) => help::topic(id).map(|t| t.title.to_string()).unwrap_or_default(),
                        None => s.to_string(),
                    };
                    if ui.link(label).clicked() {
                        self.navigate(s);
                    }
                }
            });
        }
    }

    fn reference_page(&mut self, ui: &mut Ui) {
        ui.heading("All words & syntax");
        ui.label(RichText::new("Stack effects read inputs — outputs, with the top of the stack on the right. Click a word for details and live examples.").weak());
        for cat in Category::ALL {
            ui.add_space(10.0);
            ui.label(RichText::new(cat.title()).strong().size(15.0));
            egui::Grid::new(("ref", cat.title())).num_columns(3).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                for w in WORDS.iter().filter(|w| w.category == cat) {
                    if ui.link(RichText::new(w.name).monospace().strong()).clicked() {
                        self.show_page(Page::Word(w.name));
                    }
                    ui.label(RichText::new(w.effect).monospace().size(MONO - 1.0).color(syntax::C_WORD));
                    ui.label(w.summary);
                    ui.end_row();
                }
            });
        }
    }

    fn units_page(&mut self, ui: &mut Ui, eng: &Engine, actions: &mut Vec<HelpAction>) {
        ui.heading("Units in this workbook");
        self.inline_para(ui, "Every unit is declared in a cell; this list is read live from your workbook. Click where a unit is defined to jump there. See [[#units]] and [[#defining-units]].");
        let units = eng.units_list();
        // base unit for each base dimension, to state factors in base units
        let mut base_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (name, _, info) in &units {
            if let Some(u) = info {
                if let [(d, e)] = u.dim.0.as_slice() {
                    if *e == wbs_core::rational::Rational::ONE && u.factor == 1.0 && u.affine.is_none() {
                        base_of.entry(d.to_string()).or_insert_with(|| name.clone());
                    }
                }
            }
        }
        let in_base = |u: &wbs_core::units::UnitInfo| -> String {
            let terms: Vec<(std::sync::Arc<str>, wbs_core::rational::Rational)> = u
                .dim
                .0
                .iter()
                .map(|(d, e)| (std::sync::Arc::from(base_of.get(&**d).map(|s| s.as_str()).unwrap_or(&**d)), *e))
                .collect();
            wbs_core::units::format_terms(&terms)
        };
        // group by dimension
        let mut groups: Vec<(String, Vec<_>)> = Vec::new();
        for (name, k, info) in units {
            let dim = info.as_ref().map(|u| u.dim.to_string()).unwrap_or_else(|| "has an error".into());
            match groups.iter_mut().find(|g| g.0 == dim) {
                Some(g) => g.1.push((name, k, info)),
                None => groups.push((dim, vec![(name, k, info)])),
            }
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        for (dim, list) in groups {
            ui.add_space(8.0);
            ui.label(RichText::new(dim).strong());
            egui::Grid::new(("units", ui.next_auto_id())).num_columns(3).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
                for (name, k, info) in list {
                    ui.label(RichText::new(format!("[{name}]")).monospace().color(syntax::C_UNIT));
                    let desc = match &info {
                        Some(u) => {
                            let base = in_base(u);
                            let is_base = base_of.values().any(|b| b == &name);
                            match u.affine {
                                Some(off) => format!(
                                    "absolute: x {name} = x × {} + {} {base}",
                                    wbs_core::value::fmt_num(u.factor),
                                    wbs_core::value::fmt_num(off)
                                ),
                                None if is_base => format!("base unit of {}", u.dim),
                                None => format!("= {} {base}", wbs_core::value::group_thousands(&wbs_core::value::fmt_num(u.factor))),
                            }
                        }
                        None => "error in definition".into(),
                    };
                    ui.label(desc);
                    if ui.link(eng.wb.cell_label(k, None)).clicked() {
                        actions.push(HelpAction::Goto(k));
                    }
                    ui.end_row();
                }
            });
        }
    }

    fn your_words_page(&mut self, ui: &mut Ui, eng: &Engine, actions: &mut Vec<HelpAction>) {
        ui.heading("Your words");
        self.inline_para(ui, "Words defined in this workbook. A comment right after the name documents a word: `: sq ( x -- x² ) dup * ;`. See [[#words]].");
        let words = eng.words_list();
        if words.is_empty() {
            ui.label(RichText::new("No words defined yet.").weak());
        }
        for (name, k, doc, locals) in words {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&name).monospace().strong().size(15.0));
                if let Some(d) = &doc {
                    ui.label(RichText::new(format!("( {d} )")).monospace().color(syntax::C_WORD));
                }
                if ui.link(eng.wb.cell_label(k, None)).clicked() {
                    actions.push(HelpAction::Goto(k));
                }
            });
            if !locals.is_empty() {
                ui.label(RichText::new(format!("locals: {}", locals.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" "))).small().weak());
            }
            let src = eng.wb.cell_text(k);
            self.code_line(ui, &src);
        }
    }

    fn playground_page(&mut self, ui: &mut Ui, eng: &Engine, home: SheetId) {
        ui.heading("Playground");
        self.inline_para(ui, "Type a program and see its value and the stack after every token. Nothing in your workbook changes.");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.play_ctx, PlayCtx::Sample, "sample sheet");
            ui.radio_value(&mut self.play_ctx, PlayCtx::Workbook, "my workbook (current sheet)");
        });
        let (e, sid) = match self.play_ctx {
            PlayCtx::Sample => (&self.sample.0, self.sample.1),
            PlayCtx::Workbook => (eng, home),
        };
        let font = FontId::monospace(MONO + 1.0);
        let base = ui.visuals().text_color();
        let refs = syntax::analyze(&self.play, &e.wb, sid);
        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _w: f32| {
            let job = syntax::layout(buf.as_str(), &refs, None, font.clone(), base);
            ui.fonts_mut(|f| f.layout_job(job))
        };
        ui.add(TextEdit::singleline(&mut self.play).desired_width(f32::INFINITY).layouter(&mut layouter));
        let text = if self.play.trim_start().starts_with('=') { self.play.clone() } else { format!("={}", self.play) };
        let s = e.eval_scratch(&text, sid);
        ui.add_space(6.0);
        result_line(ui, &s);
        ui.add_space(6.0);
        trace_table(ui, &text, &s);
        if self.play_ctx == PlayCtx::Sample {
            ui.add_space(10.0);
            ui.label(RichText::new("Sample sheet").strong());
            sample_grid(ui);
        }
    }

    fn search_page(&mut self, ui: &mut Ui, eng: &Engine, actions: &mut Vec<HelpAction>) {
        let q = self.query.trim().to_lowercase();
        let hits = help::search(&q);
        let units: Vec<_> = eng.units_list().into_iter().filter(|(n, ..)| n.to_lowercase().contains(&q)).collect();
        let words: Vec<_> = eng.words_list().into_iter().filter(|(n, ..)| n.to_lowercase().contains(&q)).collect();
        let names: Vec<_> = eng.wb.names.iter().filter(|(n, _)| n.to_lowercase().contains(&q)).map(|(n, d)| (n.clone(), d.cell)).collect();
        if hits.is_empty() && units.is_empty() && words.is_empty() && names.is_empty() {
            ui.label(RichText::new(format!("Nothing matches “{}”.", self.query.trim())).weak());
            return;
        }
        let mut go: Option<Hit> = None;
        for (hit, text) in hits.iter().take(40) {
            ui.horizontal(|ui| {
                let tag = match hit {
                    Hit::Word(_) => "word",
                    Hit::Topic(_) => "guide",
                };
                ui.label(RichText::new(tag).small().weak());
                if ui.link(text).clicked() {
                    go = Some(hit.clone());
                }
            });
        }
        if let Some(h) = go {
            match h {
                Hit::Word(w) => self.show_page(Page::Word(w)),
                Hit::Topic(t) => self.show_page(Page::Topic(t)),
            }
        }
        for (n, k, _) in units.iter().take(20) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("unit").small().weak());
                if ui.link(format!("[{n}] — defined at {}", eng.wb.cell_label(*k, None))).clicked() {
                    actions.push(HelpAction::Goto(*k));
                }
            });
        }
        for (n, k, doc, _) in words.iter().take(20) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("your word").small().weak());
                let d = doc.as_ref().map(|d| format!(" ( {d} )")).unwrap_or_default();
                if ui.link(format!("{n}{d} — {}", eng.wb.cell_label(*k, None))).clicked() {
                    actions.push(HelpAction::Goto(*k));
                }
            });
        }
        for (n, k) in names.iter().take(20) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("name").small().weak());
                if ui.link(format!("{n} — {}", eng.wb.cell_label(*k, None))).clicked() {
                    actions.push(HelpAction::Goto(*k));
                }
            });
        }
    }

    // ---- blocks & inlines ---------------------------------------------------

    fn block(&mut self, ui: &mut Ui, b: &Block) {
        match b {
            Block::Heading(h) => {
                ui.add_space(10.0);
                ui.label(RichText::new(h).strong().size(16.0));
                ui.add_space(2.0);
            }
            Block::Para(p) => {
                self.inline_para(ui, p);
                ui.add_space(6.0);
            }
            Block::Bullet(p) => {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("  •  ");
                    self.inlines(ui, p);
                });
                ui.add_space(3.0);
            }
            Block::Code(lines) => {
                code_frame(ui, |ui| {
                    for l in lines {
                        self.code_line(ui, l);
                    }
                });
                ui.add_space(6.0);
            }
            Block::Example(lines) => {
                self.examples(ui, lines);
                ui.add_space(6.0);
            }
            Block::Table(rows) => {
                egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                    for (i, (a, b)) in rows.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            if i == 0 {
                                ui.label(RichText::new(a).strong());
                            } else {
                                self.inlines(ui, a);
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            if i == 0 {
                                ui.label(RichText::new(b).strong());
                            } else {
                                self.inlines(ui, b);
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
            }
            Block::Generated(g) if g == "errors" => {
                for e in ERRORS {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e.title).strong());
                    self.inline_para(ui, e.why);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("Fix: ").strong());
                        self.inlines(ui, e.fix);
                        ui.label("  ");
                        let link = format!("#{}", e.topic);
                        if e.topic != "errors" && ui.link("more").clicked() {
                            self.navigate(&link);
                        }
                    });
                }
            }
            Block::Generated(g) if g == "sample" => sample_grid(ui),
            Block::Generated(_) => {}
        }
    }

    fn inline_para(&mut self, ui: &mut Ui, text: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            self.inlines(ui, text);
        });
    }

    fn inlines(&mut self, ui: &mut Ui, text: &str) {
        for i in help::inlines(text) {
            match i {
                Inline::Text(t) => {
                    ui.label(t);
                }
                Inline::Bold(t) => {
                    ui.label(RichText::new(t).strong());
                }
                Inline::Code(c) => {
                    let r = ui.label(RichText::new(&c).monospace().size(MONO).background_color(code_bg(ui)));
                    // a code span that names a word links to it
                    if let Some(w) = help::word_doc(c.trim()) {
                        if r.on_hover_text(format!("{}  {}", w.effect, w.summary)).clicked() {
                            self.show_page(Page::Word(w.name));
                        }
                    }
                }
                Inline::Link { target, label } => {
                    if ui.link(label).clicked() {
                        self.navigate(&target);
                    }
                }
            }
        }
    }

    fn code_line(&self, ui: &mut Ui, line: &str) {
        let job = syntax::layout(line, &[], None, FontId::monospace(MONO), ui.visuals().text_color());
        ui.label(job);
    }

    fn examples(&mut self, ui: &mut Ui, lines: &[(String, String)]) {
        let (e, sid) = (&self.sample.0, self.sample.1);
        let mut try_it = None;
        code_frame(ui, |ui| {
            egui::Grid::new(ui.next_auto_id()).num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                for (prog, _) in lines {
                    let job = syntax::layout(&format!("={prog}"), &[], None, FontId::monospace(MONO), ui.visuals().text_color());
                    ui.label(job);
                    match help::run_example(e, sid, prog) {
                        Ok(v) => ui.label(RichText::new(format!("⇒ {v}")).monospace().size(MONO).strong()),
                        Err(m) => ui.label(RichText::new(format!("⇒ error: {m}")).monospace().size(MONO).color(syntax::C_ERR)),
                    };
                    if ui.small_button("Try").on_hover_text("open in the playground").clicked() {
                        try_it = Some(prog.clone());
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(p) = try_it {
            self.try_in_playground(&p, PlayCtx::Sample);
        }
    }
}

fn code_bg(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0x2a, 0x2d, 0x34)
    } else {
        Color32::from_rgb(0xf1, 0xf3, 0xf5)
    }
}

fn code_frame(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(code_bg(ui)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    });
}

fn sample_grid(ui: &mut Ui) {
    egui::Grid::new("sample_grid").num_columns(4).spacing([10.0, 2.0]).striped(true).show(ui, |ui| {
        for chunk in SAMPLE.chunks(2) {
            for (at, text) in chunk {
                ui.label(RichText::new(*at).monospace().weak());
                ui.label(RichText::new(*text).monospace());
            }
            ui.end_row();
        }
    });
    let names: Vec<String> = SAMPLE_NAMES.iter().map(|(n, at)| format!("{n} = {at}")).collect();
    ui.label(RichText::new(format!("names: {}", names.join(", "))).small().weak());
}

/// The value (or error) of a scratch evaluation.
pub fn result_line(ui: &mut Ui, s: &Scratch) {
    match &s.result {
        Ok(v) => {
            ui.label(RichText::new(format!("⇒ {}", v.summary(12))).monospace().size(MONO + 2.0).strong());
        }
        Err(e) => {
            ui.label(RichText::new(format!("error: {}", e.msg)).color(syntax::C_ERR));
            if let Some(h) = help::explain_error(&e.msg) {
                ui.label(RichText::new(format!("{} — {}", h.why, h.fix)).small());
            }
        }
    }
}

/// Token-by-token stack table.
pub fn trace_table(ui: &mut Ui, text: &str, s: &Scratch) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([16.0, 3.0]).striped(true).show(ui, |ui| {
        ui.label(RichText::new("token").small().weak());
        ui.label(RichText::new("stack after (top on the right)").small().weak());
        ui.end_row();
        for st in &s.steps {
            let tok = text.get(st.span.clone()).unwrap_or("?");
            ui.label(syntax::layout(tok, &[], None, FontId::monospace(MONO), ui.visuals().text_color()));
            ui.horizontal_wrapped(|ui| stack_chips(ui, &st.stack));
            ui.end_row();
        }
        if let Err(e) = &s.result {
            if let Some(span) = &e.span {
                let tok = text.get(span.clone()).unwrap_or("?");
                ui.label(RichText::new(tok).monospace().color(syntax::C_ERR));
                ui.label(RichText::new(&e.msg).color(syntax::C_ERR));
                ui.end_row();
            }
        }
    });
}

pub fn stack_chips(ui: &mut Ui, stack: &[Value]) {
    if stack.is_empty() {
        ui.label(RichText::new("(empty)").weak().small());
        return;
    }
    let bg = code_bg(ui);
    for v in stack {
        egui::Frame::new().fill(bg).corner_radius(4.0).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
            ui.label(RichText::new(v.summary(5)).monospace().size(MONO - 1.0));
        });
    }
}

/// One-line documentation for a word (builtin or user-defined).
pub fn word_hint(eng: &Engine, word: &str) -> Option<String> {
    if let Some(d) = help::word_doc(word) {
        return Some(doc_line(d));
    }
    eng.words_list().into_iter().find(|w| w.0 == word).map(|(n, k, doc, _)| match doc {
        Some(d) => format!("{n}  ( {d} )  — your word, defined at {}", eng.wb.cell_label(k, None)),
        None => format!("{n} — your word, defined at {}", eng.wb.cell_label(k, None)),
    })
}

pub fn doc_line(d: &WordDoc) -> String {
    format!("{}   {}  — {}", d.name, d.effect, d.summary)
}
