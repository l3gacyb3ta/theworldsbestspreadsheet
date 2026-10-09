//! Keyboard handling, toolbar, formula bar, inspector and status bar.

use super::*;

pub(super) struct EditorOut {
    pub lost_focus: bool,
}

impl App {
    /// Grid keyboard: only when no text field has focus.
    pub(super) fn keys(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let focused = ctx.memory(|m| m.focused());
        for ev in &events {
            if let Event::Key { key: Key::S, pressed: true, modifiers, .. } = ev {
                if modifiers.command {
                    self.commit();
                    self.save();
                }
            }
        }
        if self.edit.is_some() {
            // An edit whose editor lost focus (e.g. after clicking a header) can still be cancelled.
            if focused.is_none() && events.iter().any(|e| matches!(e, Event::Key { key: Key::Escape, pressed: true, .. })) {
                self.cancel_edit();
            }
            return;
        }
        if focused.is_some() {
            return;
        }
        let (r, c) = self.cursor;
        for ev in events {
            match ev {
                Event::Text(t) if !mods.command && !mods.ctrl => {
                    self.start_edit(ctx, r, c, Some(t), false);
                    return;
                }
                Event::Copy => self.copy(ctx),
                Event::Cut => {
                    self.copy(ctx);
                    let e = ops::clear(&self.eng, self.sel());
                    self.exec(e);
                }
                Event::Paste(s) => self.paste(&s),
                Event::Key { key, pressed: true, modifiers: m, .. } => {
                    let ext = m.shift;
                    match key {
                        Key::ArrowUp => self.move_sel(-1, 0, ext),
                        Key::ArrowDown => self.move_sel(1, 0, ext),
                        Key::ArrowLeft => self.move_sel(0, -1, ext),
                        Key::ArrowRight => self.move_sel(0, 1, ext),
                        Key::Enter if m.command => self.accept_offer(),
                        Key::Enter => self.move_sel(if m.shift { -1 } else { 1 }, 0, false),
                        Key::Tab => self.move_sel(0, if m.shift { -1 } else { 1 }, false),
                        Key::F2 => self.start_edit(ctx, r, c, None, false),
                        Key::Delete | Key::Backspace => {
                            let e = ops::clear(&self.eng, self.sel());
                            self.exec(e);
                        }
                        Key::Z if m.command && m.shift => self.redo(),
                        Key::Z if m.command => self.undo(),
                        Key::Y if m.command => self.redo(),
                        Key::D if m.command => self.fill_down(),
                        Key::E if m.command => self.accept_offer(),
                        Key::Escape => {
                            self.offer = None;
                            self.status = None;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn copy(&mut self, ctx: &egui::Context) {
        let clip = ops::copy(&self.eng, self.sel());
        ctx.copy_text(clip.text.clone());
        self.status = Some(format!("copied {}×{}", clip.rows, clip.cols));
        self.clip = Some(clip);
    }

    fn paste(&mut self, s: &str) {
        let sid = self.sid();
        let (r0, c0) = (self.anchor.0.min(self.cursor.0), self.anchor.1.min(self.cursor.1));
        let edit = match &self.clip {
            Some(clip) if clip.text.trim_end() == s.trim_end().replace("\r\n", "\n") => {
                let clip = clip.clone();
                ops::paste(&mut self.eng, &clip, sid, (r0, c0))
            }
            _ => ops::paste_text(&mut self.eng, s, sid, (r0, c0)),
        };
        self.exec(edit);
    }

    pub(super) fn fill_down(&mut self) {
        let sel = self.sel();
        if sel.rows() < 2 {
            return;
        }
        let src = CRect { r1: sel.r0, ..sel };
        let e = ops::fill(&mut self.eng, src, sel);
        self.exec(e);
    }

    pub(super) fn accept_offer(&mut self) {
        if let Some((k, dst)) = self.offer.take() {
            if let Some((r, c)) = self.eng.wb.pos(k) {
                let src = CRect::cell(k.sheet, r, c);
                let e = ops::fill(&mut self.eng, src, dst);
                self.exec(e);
                self.status = Some(format!("extended {} down to row {}", self.label(k), dst.r1 + 1));
            }
        }
    }

    pub(super) fn goto(&mut self, k: CellKey) {
        if let Some(ix) = self.eng.wb.sheets.iter().position(|s| s.id == k.sheet) {
            self.sheet_ix = ix;
        }
        if let Some((r, c)) = self.eng.wb.pos(k) {
            self.select(r, c, false);
            self.scroll_into_view = true;
        }
    }

    /// The shared formula editor (used in the cell and in the formula bar).
    pub(super) fn editor(&mut self, ui: &mut Ui, id: Id, width: f32, in_bar: bool) -> EditorOut {
        let ctx = ui.ctx().clone();
        let ed = self.edit.as_ref().unwrap();
        let refs = syntax::analyze(&ed.text, &self.eng.wb, ed.key.sheet);
        let font = FontId::proportional(FONT);
        let base = ui.visuals().text_color();
        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _w: f32| {
            let job = syntax::layout(buf.as_str(), &refs, None, font.clone(), base);
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let ed = self.edit.as_mut().unwrap();
        let mut out = TextEdit::singleline(&mut ed.text)
            .id(id)
            .frame(egui::Frame::NONE)
            .desired_width(width)
            .layouter(&mut layouter)
            .show(ui);
        let resp = out.response.response.clone();
        if let Some((pass, caret)) = self.focus_req {
            if ed.in_bar == in_bar && ctx.cumulative_pass_nr() >= pass {
                resp.request_focus();
                out.state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
                out.state.store(&ctx, id);
                self.focus_req = None;
            }
        }
        if resp.changed() {
            ed.ref_span = None;
        }
        if resp.has_focus() {
            ed.in_bar = in_bar;
            if let Some(cr) = out.cursor_range {
                if cr.primary.index.0 != ed.cursor {
                    ed.cursor = cr.primary.index.0;
                }
            }
        }
        EditorOut { lost_focus: resp.lost_focus() }
    }

    /// Enter/Tab/Escape after an editor lost focus.
    pub(super) fn editor_keys(&mut self, ui: &Ui, out: &EditorOut) {
        if !out.lost_focus {
            return;
        }
        let (enter, tab, esc, shift) =
            ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Tab), i.key_pressed(Key::Escape), i.modifiers.shift));
        if esc {
            self.cancel_edit();
        } else if enter {
            let k = self.edit.as_ref().unwrap().key;
            self.commit();
            self.goto(k);
            self.move_sel(if shift { -1 } else { 1 }, 0, false);
        } else if tab {
            let k = self.edit.as_ref().unwrap().key;
            self.commit();
            self.goto(k);
            self.move_sel(0, if shift { -1 } else { 1 }, false);
            ui.ctx().memory_mut(|m| m.stop_text_input());
        }
    }

    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("the world's best spreadsheet").strong());
            ui.separator();
            if ui.button("Save").on_hover_text("⌘S").clicked() {
                self.commit();
                self.save();
            }
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).on_hover_text("⌘Z").clicked() {
                self.undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).on_hover_text("⇧⌘Z").clicked() {
                self.redo();
            }
            ui.separator();
            ui.toggle_value(&mut self.trace, "Trace").on_hover_text("Highlight precedents (blue) and dependents (orange) of the selected cell");
            ui.separator();
            ui.label(egui::RichText::new(format!("recalc {:.2} ms · {}", self.last_recalc_ms, self.path.display())).weak().small());
        });
    }

    pub(super) fn formula_bar(&mut self, ui: &mut Ui, _dark: bool) {
        let ctx = ui.ctx().clone();
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            let k = {
                let (r, c) = self.cursor;
                self.key(r, c)
            };
            let shown_key = self.edit.as_ref().map(|e| e.key).unwrap_or(k);
            let mut addr = self.label(shown_key);
            if let Some(n) = self.eng.name_of(shown_key) {
                addr = format!("{addr} · {n}");
            }
            ui.add_sized([110.0, 22.0], egui::Label::new(egui::RichText::new(addr).monospace().strong()).truncate());
            let text_now = self.edit.as_ref().map(|e| e.text.clone()).unwrap_or_else(|| self.eng.wb.cell_text(k));
            let kind = classify(&text_now);
            let badge = match kind {
                Kind::Empty => "",
                Kind::Text => "text",
                Kind::Number => "number",
                Kind::Program => "program",
                Kind::WordDef => "word",
                Kind::UnitDecl => "unit",
            };
            ui.add_sized([64.0, 22.0], egui::Label::new(egui::RichText::new(badge).small().weak()));
            ui.separator();
            let width = ui.available_width();
            if self.edit.is_some() {
                let out = self.editor(ui, Id::new("formula_bar_editor"), width, true);
                self.editor_keys(ui, &out);
                self.bar_galley = None;
                return;
            }
            // display mode: colored program, alt-drag numbers to scrub, click to edit
            let err_span = match self.eng.shown(k) {
                Shown::Error(e) if e.kind == ErrKind::Local => e.span.clone(),
                _ => self.eng.compile_error(k).and_then(|e| e.span.clone()),
            };
            let font = FontId::proportional(FONT);
            let job = syntax::layout(&text_now, &[], err_span.as_ref(), font, ui.visuals().text_color());
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click_and_drag());
            let pos = Pos2::new(rect.left() + 2.0, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley.clone(), ui.visuals().text_color());
            let lits: Vec<Lit> = match kind {
                Kind::Number => ops::cell_literal(&text_now).into_iter().collect(),
                Kind::Program | Kind::UnitDecl | Kind::WordDef => ops::program_literals(&text_now),
                _ => vec![],
            };
            let alt = ui.input(|i| i.modifiers.alt);
            let lit_at = |p: Pos2| -> Option<Lit> {
                let ci = galley.cursor_from_pos(p - pos).index.0;
                let byte = text_now.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(text_now.len());
                lits.iter().find(|l| l.span.start <= byte && byte <= l.span.end).cloned()
            };
            if let Some(hp) = resp.hover_pos() {
                if alt && lit_at(hp).is_some() {
                    ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                } else {
                    ctx.set_cursor_icon(CursorIcon::Text);
                }
            }
            if resp.drag_started() && alt {
                if let Some(lit) = resp.interact_pointer_pos().and_then(lit_at) {
                    let x0 = resp.interact_pointer_pos().unwrap().x;
                    self.bar_scrub = Some((k, self.eng.wb.cell(k).cloned(), text_now.clone(), lit, x0));
                }
            }
            if let Some((sk, _, ref text, ref lit, x0)) = self.bar_scrub {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let fast = if ui.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
                    let steps = ((p.x - x0) / 4.0).round() as f64 * fast;
                    let new = ops::replace_span(text, &lit.span, &ops::scrub(lit, steps));
                    if new != self.eng.wb.cell_text(sk) {
                        let t = std::time::Instant::now();
                        self.eng.set_text(sk, &new);
                        self.last_recalc_ms = t.elapsed().as_secs_f64() * 1000.0;
                    }
                }
                if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                    let (sk, orig, ..) = self.bar_scrub.take().unwrap();
                    if self.eng.wb.cell(sk) != orig.as_ref() {
                        self.undo.push(Edit::Cells(vec![(sk, orig)]));
                        self.redo.clear();
                    }
                }
            } else if resp.clicked() && !alt {
                let caret = resp.interact_pointer_pos().map(|p| galley.cursor_from_pos(p - pos).index.0).unwrap_or(0);
                let (r, c) = self.cursor;
                self.start_edit(&ctx, r, c, None, true);
                if let Some(ed) = &mut self.edit {
                    ed.cursor = caret;
                }
                if let Some(f) = &mut self.focus_req {
                    f.1 = caret;
                }
            }
        });
        ui.add_space(2.0);
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let mut switch = None;
            for (i, s) in self.eng.wb.sheets.iter().enumerate() {
                if ui.selectable_label(i == self.sheet_ix, &s.name).clicked() {
                    switch = Some(i);
                }
            }
            if ui.small_button("+").on_hover_text("add a sheet").clicked() {
                let n = self.eng.wb.sheets.len() + 1;
                let mut name = format!("Sheet{n}");
                while self.eng.wb.sheet_by_name(&name).is_some() {
                    name.push('_');
                }
                self.eng.wb.sheets.push(Sheet::new(&name, 200, 26));
                switch = Some(self.eng.wb.sheets.len() - 1);
            }
            if let Some(i) = switch {
                if i != self.sheet_ix {
                    self.sheet_ix = i;
                    self.anchor = (0, 0);
                    self.cursor = (0, 0);
                    self.scroll = Vec2::ZERO;
                    self.sheet_name_buf = self.sheet().name.clone();
                }
            }
            ui.separator();
            if self.sheet_name_buf.is_empty() {
                self.sheet_name_buf = self.sheet().name.clone();
            }
            let r = ui.add(TextEdit::singleline(&mut self.sheet_name_buf).desired_width(90.0).hint_text("sheet name"));
            if r.lost_focus() {
                let new = self.sheet_name_buf.trim().to_string();
                if !new.is_empty() && new != self.sheet().name && self.eng.wb.sheet_by_name(&new).is_none() {
                    let ix = self.sheet_ix;
                    self.eng.wb.sheets[ix].name = new;
                    self.eng.rebuild();
                } else {
                    self.sheet_name_buf = self.sheet().name.clone();
                }
            }
            ui.separator();
            let k = self.sheet().key(self.cursor.0, self.cursor.1);
            let msg = match k.map(|k| self.eng.shown(k)) {
                Some(Shown::Error(e)) => Some((e.msg.clone(), true)),
                _ => self.status.clone().map(|s| (s, false)),
            };
            if let Some((m, is_err)) = msg {
                let t = egui::RichText::new(m);
                ui.label(if is_err { t.color(Color32::from_rgb(0xdc, 0x26, 0x26)) } else { t.weak() });
            }
        });
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let (r, c) = self.cursor;
        let k = self.key(r, c);
        ui.add_space(4.0);
        ui.heading(self.label(k));
        let kind = self.eng.kind(k);
        let anchor = self.eng.spill_anchor(k);
        ui.label(egui::RichText::new(match (kind, anchor) {
            (Kind::Empty, Some(_)) => "spilled (read-only)".to_string(),
            (k, _) => k.label().to_string(),
        }).weak());
        if let Some(a) = anchor {
            if kind == Kind::Empty && ui.link(format!("source: {}", self.label(a))).clicked() {
                self.goto(a);
            }
        }
        ui.add_space(4.0);
        let mut goto = None;
        match self.eng.shown(k) {
            Shown::Value { value, dr, dc, .. } => {
                let shape = value.shape();
                let desc = match value {
                    Value::Chart(c) => format!("chart · {} layer(s) · {}×{} cells", c.layers.len(), c.cols, c.rows),
                    _ if !shape.is_empty() && (dr, dc) == (0, 0) => format!("array {:?} · first: {}", shape, value.display_at(0, 0)),
                    _ => value.display_at(dr, dc),
                };
                ui.label(egui::RichText::new(desc).size(16.0));
                if let Value::Num(n) = value {
                    if !n.q.dim.is_none() {
                        ui.label(egui::RichText::new(format!("dimension: {}", n.q.dim)).small().weak());
                    }
                }
            }
            Shown::Error(e) => {
                ui.colored_label(Color32::from_rgb(0xdc, 0x26, 0x26), &e.msg);
                match &e.kind {
                    ErrKind::Upstream(u) => {
                        if ui.link(format!("→ go to {}", self.label(*u))).clicked() {
                            goto = Some(*u);
                        }
                    }
                    ErrKind::SpillBlocked(b) => {
                        if ui.link(format!("→ go to blocking cell {}", self.label(*b))).clicked() {
                            goto = Some(*b);
                        }
                    }
                    _ => {}
                }
            }
            Shown::Empty => {
                ui.label(egui::RichText::new("empty").weak());
            }
        }
        if let Some(g) = goto {
            self.goto(g);
        }

        ui.separator();
        // naming
        if self.name_for != Some(k) {
            self.name_for = Some(k);
            self.name_buf = self.eng.name_of(k).unwrap_or_default().to_string();
        }
        let current = self.eng.name_of(k).map(|s| s.to_string());
        let is_input = current.as_ref().and_then(|n| self.eng.wb.names.get(n)).map(|d| d.input).unwrap_or(false);
        ui.horizontal(|ui| {
            ui.label("Name");
            let r = ui.add(TextEdit::singleline(&mut self.name_buf).desired_width(120.0).hint_text("e.g. growth"));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button("Set").clicked() || enter) && !self.name_buf.trim().is_empty() {
                let name = self.name_buf.trim().to_string();
                if let Some(old) = &current {
                    if *old != name {
                        let e = self.eng.set_name(old, None, false);
                        if let Ok(e) = e {
                            self.undo.push(e);
                        }
                    }
                }
                match self.eng.set_name(&name, Some(k), is_input || current.is_none()) {
                    Ok(e) => {
                        self.undo.push(e);
                        self.redo.clear();
                        self.status = Some(format!("named {} {name}", self.label(k)));
                    }
                    Err(m) => self.status = Some(m),
                }
            }
        });
        if let Some(n) = &current {
            ui.horizontal(|ui| {
                let mut inp = is_input;
                if ui.checkbox(&mut inp, "input").on_hover_text("inputs are listed below and tinted in the grid").changed() {
                    if let Ok(e) = self.eng.set_name(n, Some(k), inp) {
                        self.undo.push(e);
                    }
                }
                if ui.small_button("remove name").clicked() {
                    if let Ok(e) = self.eng.set_name(n, None, false) {
                        self.undo.push(e);
                        self.name_buf.clear();
                    }
                }
            });
        }

        ui.separator();
        ui.strong("Inputs");
        let inputs: Vec<(String, CellKey)> =
            self.eng.wb.names.iter().filter(|(_, d)| d.input).map(|(n, d)| (n.clone(), d.cell)).collect();
        if inputs.is_empty() {
            ui.label(egui::RichText::new("Name a cell and tick “input” to list it here.").weak().small());
        }
        egui::Grid::new("inputs").num_columns(2).striped(true).show(ui, |ui| {
            for (name, ik) in inputs {
                if ui.link(&name).clicked() {
                    self.goto(ik);
                }
                let text = self.eng.wb.cell_text(ik);
                match ops::cell_literal(&text) {
                    Some(lit) if !lit.is_date => {
                        let mut v = lit.value;
                        let speed = 10f64.powi(-(lit.decimals as i32));
                        let suffix = text[lit.span.end..].trim().to_string();
                        let r = ui.add(
                            egui::DragValue::new(&mut v)
                                .speed(speed)
                                .max_decimals(lit.decimals.max(0))
                                .min_decimals(lit.decimals)
                                .suffix(if suffix.is_empty() { String::new() } else { format!(" {suffix}") }),
                        );
                        if r.changed() {
                            let new = ops::replace_span(&text, &lit.span, &ops::format_lit(v, lit.decimals, false));
                            if self.input_scrub.is_none() {
                                self.input_scrub = Some((ik, self.eng.wb.cell(ik).cloned()));
                            }
                            let t = std::time::Instant::now();
                            self.eng.set_text(ik, &new);
                            self.last_recalc_ms = t.elapsed().as_secs_f64() * 1000.0;
                        }
                        if !r.dragged() && !r.has_focus() {
                            if let Some((sk, orig)) = self.input_scrub.take() {
                                if sk == ik {
                                    self.undo.push(Edit::Cells(vec![(sk, orig)]));
                                    self.redo.clear();
                                } else {
                                    self.input_scrub = Some((sk, orig));
                                }
                            }
                        }
                    }
                    _ => {
                        let shown = match self.eng.shown(ik) {
                            Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
                            Shown::Error(e) => e.short().to_string(),
                            Shown::Empty => String::new(),
                        };
                        ui.label(shown);
                    }
                }
                ui.end_row();
            }
        });

        if self.trace {
            ui.separator();
            let pre = self.eng.precedents(k);
            let dep = self.eng.dependents(k);
            let mut goto = None;
            ui.strong("Precedents");
            ui.horizontal_wrapped(|ui| {
                if pre.is_empty() {
                    ui.label(egui::RichText::new("none").weak());
                }
                for p in pre.iter().take(40) {
                    if ui.small_button(self.label(*p)).clicked() {
                        goto = Some(*p);
                    }
                }
            });
            ui.strong("Dependents");
            ui.horizontal_wrapped(|ui| {
                if dep.is_empty() {
                    ui.label(egui::RichText::new("none").weak());
                }
                for p in dep.iter().take(40) {
                    if ui.small_button(self.label(*p)).clicked() {
                        goto = Some(*p);
                    }
                }
            });
            if let Some(g) = goto {
                self.goto(g);
            }
        }

        ui.separator();
        egui::CollapsingHeader::new("Language").default_open(false).show(ui, |ui| {
            ui.label(egui::RichText::new(HELP).small());
            ui.add_space(4.0);
            egui::Grid::new("builtins").num_columns(2).striped(true).show(ui, |ui| {
                for (name, _, doc) in BUILTINS {
                    ui.label(egui::RichText::new(*name).monospace().strong());
                    ui.label(egui::RichText::new(*doc).small());
                    ui.end_row();
                }
            });
        });
    }
}

const HELP: &str = "\
=  program: postfix, the cell's value is the top of the stack
   =A1:A10 /+      sum a column (a range is one array)
   =B1:B9 C1:C9 *  elementwise; scalars broadcast
   =5 [m/s] 3 [s] *    units multiply; to[km/h] changes display
:  word: `: sq dup * ;`   or with locals `: f { a b } a b + ;`
dim widgets · base [widget] widgets · [mi] = 1609.344 [m]
'  force text.   /op reduce · \\op scan
Click cells while editing to insert references.
Alt-drag a number (in a cell or here) to scrub it.
Drag the selection's corner to fill.";
