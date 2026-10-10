//! Keyboard handling, toolbar, formula bar, inspector and status bar.

use super::*;

pub(super) struct EditorOut {
    pub lost_focus: bool,
}

/// Completions offered while editing: the byte span they replace and the candidates.
pub(super) struct Comps {
    pub span: Range<usize>,
    pub cands: Vec<String>,
    pub unit: bool,
}

/// Sheet-tab state: an inline rename, a delete waiting for confirmation, a tab being dragged.
#[derive(Default)]
pub(super) struct Tabs {
    pub(super) rename: Option<(SheetId, String)>,
    pub(super) focus_rename: bool,
    pub(super) confirm_delete: Option<SheetId>,
    pub(super) drag: Option<SheetId>,
}

enum TabAct {
    Show(usize),
    StartRename(SheetId),
    Rename(SheetId, String),
    /// (sheet, confirmed)
    Delete(SheetId, bool),
    Add,
    Duplicate(SheetId),
    Move(SheetId, usize),
}

impl App {
    /// Grid keyboard: only when no text field has focus.
    pub(super) fn keys(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let focused = ctx.memory(|m| m.focused());
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
                Event::Cut => self.cut(ctx),
                Event::Paste(s) => self.paste(ctx, &s),
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
                            self.cut = None;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// F1: open help (in front) at the page that explains what's under the caret
    /// (when editing) or in the selected cell. Help drawn inside this window
    /// closes instead if it's open; a separate help window closes on F1 in itself.
    pub(super) fn context_help(&mut self) {
        if self.help.open && self.help.embedded() {
            self.help.close();
            return;
        }
        if let Some(ed) = &self.edit {
            let at = syntax::char_to_byte(&ed.text, ed.cursor);
            if let Some(t) = syntax::token_at(&ed.text, at) {
                if let Some(d) = help::doc_for_token(&t.tok) {
                    self.help.show_page(Page::Word(d.name));
                    return;
                }
                if let wbs_core::lex::Tok::Word(w) = &t.tok {
                    if self.eng.symbols().words.contains_key(w.as_str()) {
                        self.help.show_page(Page::YourWords);
                        return;
                    }
                }
            }
            self.help.show_page(Page::Topic(kind_topic(classify(&ed.text))));
            return;
        }
        let k = {
            let (r, c) = self.cursor;
            self.key(r, c)
        };
        if let Shown::Error(e) = self.eng.shown(k) {
            if let Some(h) = help::explain_error(&e.msg) {
                self.help.show_page(Page::Topic(h.topic));
                return;
            }
        }
        let kind = self.eng.kind(k);
        let topic = if kind == Kind::Empty && self.eng.spill_anchor(k).is_some() { "spill" } else { kind_topic(kind) };
        self.help.show_page(Page::Topic(topic));
    }

    /// The editing hint strip: docs for the token at the caret, the stack at
    /// that point, and completions.
    fn assist(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(ed) = &self.edit else { return };
        let text = ed.text.clone();
        let caret_ci = ed.cursor;
        let in_bar = ed.in_bar;
        let home = ed.key.sheet;
        let at = syntax::char_to_byte(&text, caret_ci);
        let kind = classify(&text);
        let tok = syntax::token_at(&text, at);
        let comps = self.completions();
        let pick = ed.pick.filter(|p| comps.as_ref().is_some_and(|c| *p < c.cands.len()));
        let mut insert: Option<(Range<usize>, String)> = None;
        ui.horizontal(|ui| {
            ui.add_space(176.0);
            // 1. what the highlighted completion, or else the token at the caret, is
            if let (Some(p), Some(c)) = (pick, &comps) {
                ui.label(RichText::new(self.completion_hint(c, &c.cands[p], home)).monospace().size(12.0));
            } else {
                let hint = match tok.as_ref().map(|t| &t.tok) {
                    Some(wbs_core::lex::Tok::Word(w)) => help_view::word_hint(&self.eng, w).or_else(|| {
                        self.eng.wb.names.get(w.as_str()).map(|n| format!("{w} — name for {}", self.eng.wb.cell_label(n.cell, Some(home))))
                    }),
                    Some(t) => help::doc_for_token(t).map(help_view::doc_line),
                    None => None,
                };
                match hint {
                    Some(h) => {
                        if ui.link(RichText::new(h).monospace().size(12.0)).on_hover_text("F1 for full help").clicked() {
                            self.context_help();
                        }
                    }
                    None => {
                        ui.label(RichText::new(kind_hint(kind)).small().weak());
                    }
                }
            }
            // 2. completions for a partial word or unit (↓/↑ highlight, Tab/Enter accept)
            if let Some(c) = &comps {
                ui.label(RichText::new("   complete:").small().weak());
                for (i, cand) in c.cands.iter().enumerate() {
                    let b = egui::Button::new(RichText::new(cand).monospace()).small().selected(pick == Some(i));
                    if ui.add(b).on_hover_text(self.completion_hint(c, cand, home)).clicked() {
                        insert = Some((c.span.clone(), cand.clone()));
                    }
                }
            }
        });
        // 3. the stack at the caret
        if kind == Kind::Program {
            let s = self.eng.eval_scratch(&text, home);
            ui.horizontal(|ui| {
                ui.add_space(176.0);
                ui.label(RichText::new("stack").small().weak());
                let before: Vec<&wbs_core::eval::TraceStep> = s.steps.iter().filter(|st| st.span.end <= at).collect();
                match before.last() {
                    Some(st) => help_view::stack_chips(ui, &st.stack),
                    None => help_view::stack_chips(ui, &[]),
                }
                if let Err(e) = &s.result {
                    let past_error = e.span.as_ref().is_none_or(|sp| sp.start < at);
                    if past_error && at >= text.trim_end().len() {
                        ui.label(RichText::new(format!("  ✗ {}", e.msg)).color(syntax::C_ERR).small());
                    }
                }
            });
        }
        if let Some((span, word)) = insert {
            self.complete(span, &word, ctx.cumulative_pass_nr() + 1);
            if let Some(ed) = &mut self.edit {
                ed.in_bar = in_bar;
            }
        }
    }

    /// Completions for the partial word (or unit, inside `[…`) just before the caret, shortest first.
    /// `None` when there are none or the list was closed with Escape.
    pub(super) fn completions(&self) -> Option<Comps> {
        let ed = self.edit.as_ref()?;
        if ed.comp_closed {
            return None;
        }
        let text = &ed.text;
        let at = syntax::char_to_byte(text, ed.cursor);
        let t = syntax::token_at(text, at)?;
        if t.span.end != at {
            return None;
        }
        let (mut cands, span, unit): (Vec<String>, Range<usize>, bool) = match &t.tok {
            wbs_core::lex::Tok::Word(w) if w.chars().next().is_some_and(|c| c.is_alphabetic()) => {
                let mut pool: Vec<String> =
                    BUILTINS.iter().map(|(n, _, _)| n.to_string()).filter(|n| n.chars().next().is_some_and(|c| c.is_alphabetic())).collect();
                pool.extend(self.eng.symbols().words.keys().cloned());
                pool.extend(self.eng.wb.names.keys().cloned());
                (pool.into_iter().filter(|n| n.starts_with(w.as_str()) && n != w).collect(), t.span.clone(), false)
            }
            wbs_core::lex::Tok::Bad(_) | wbs_core::lex::Tok::Unit(_) | wbs_core::lex::Tok::To(_) => {
                // inside `[…`: complete the last unit name
                let src = &text[t.span.start..at];
                let open = src.find('[')?;
                let inner = &src[open + 1..];
                let frag_start = inner.rfind(['*', '/', '(', ' ']).map(|i| i + 1).unwrap_or(0);
                let frag = &inner[frag_start..];
                if frag.is_empty() || inner.ends_with(']') {
                    return None;
                }
                let start = t.span.start + open + 1 + frag_start;
                (self.eng.symbols().units.keys().filter(|u| u.starts_with(frag) && *u != frag).cloned().collect(), start..at, true)
            }
            _ => return None,
        };
        cands.sort_by_key(|c| (c.len(), c.clone()));
        cands.dedup();
        cands.truncate(8);
        (!cands.is_empty()).then_some(Comps { span, cands, unit })
    }

    /// One line describing a completion candidate.
    fn completion_hint(&self, c: &Comps, cand: &str, home: SheetId) -> String {
        if !c.unit {
            return help_view::word_hint(&self.eng, cand)
                .or_else(|| self.eng.wb.names.get(cand).map(|n| format!("{cand} — name for {}", self.eng.wb.cell_label(n.cell, Some(home)))))
                .unwrap_or_else(|| cand.to_string());
        }
        match self.eng.units_list().into_iter().find(|(m, ..)| m == cand) {
            Some((_, k, Some(i))) => format!("[{cand}]: {} · defined at {}", i.dim, self.eng.wb.cell_label(k, Some(home))),
            _ => format!("[{cand}]"),
        }
    }

    /// Replace `span` of the edit text with a completion, add a separating space after a word, and put the
    /// caret after it (from pass `pass` on).
    fn complete(&mut self, span: Range<usize>, word: &str, pass: u64) {
        let Some(ed) = &mut self.edit else { return };
        ed.text.replace_range(span.clone(), word);
        let end = span.start + word.len();
        if !ed.text[end..].starts_with(' ') && !word.is_empty() && !ed.text[..span.start].ends_with('[') {
            ed.text.insert(end, ' ');
        }
        let caret = ed.text[..end].chars().count() + usize::from(ed.text[end..].starts_with(' '));
        ed.cursor = caret;
        ed.pick = None;
        ed.comp_closed = false;
        self.focus_req = Some((pass, caret));
    }

    /// Completion keys, before the focused editor sees them: ↓/↑ move the highlight, Tab/Enter accept the
    /// highlighted completion, Escape closes the list. With no list (or nothing highlighted for Tab/Enter)
    /// the keys are left to the editor.
    fn completion_keys(&mut self, ctx: &egui::Context) {
        let Some(c) = self.completions() else { return };
        let plain = |k: Key| ctx.input_mut(|i| i.modifiers.is_none() && i.consume_key(Modifiers::NONE, k));
        // Enter/Tab also accept with Shift held (consume_key ignores Shift)
        let accept = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let ed = self.edit.as_mut().unwrap();
        let before = (ed.pick, ed.comp_closed);
        if plain(Key::ArrowDown) {
            ed.pick = Some(ed.pick.map_or(0, |p| (p + 1).min(c.cands.len() - 1)));
        }
        if plain(Key::ArrowUp) {
            ed.pick = ed.pick.and_then(|p| p.checked_sub(1));
        }
        if plain(Key::Escape) {
            ed.pick = None;
            ed.comp_closed = true;
        }
        if let Some(p) = ed.pick.filter(|p| *p < c.cands.len()) {
            if accept(Key::Enter) || accept(Key::Tab) {
                self.complete(c.span.clone(), &c.cands[p], ctx.cumulative_pass_nr());
                ctx.request_repaint();
                return;
            }
        }
        if (ed.pick, ed.comp_closed) != before {
            // the strip may already be drawn this frame
            ctx.request_repaint();
        }
    }

    fn copy(&mut self, ctx: &egui::Context) {
        let clip = ops::copy(&self.eng, self.sel());
        ctx.copy_text(clip.text.clone());
        self.status = Some(format!("copied {}×{}", clip.rows, clip.cols));
        self.clip = Some(clip);
    }

    /// ⌘X marks the selection; the next ⌘V moves it, unless the document changed in between.
    fn cut(&mut self, ctx: &egui::Context) {
        self.copy(ctx);
        let sel = self.sel();
        self.cut = Some((sel, files::fingerprint(&self.eng.wb)));
        self.status = Some(format!("cut {}×{} — paste to move it", sel.rows(), sel.cols()));
    }

    fn paste(&mut self, ctx: &egui::Context, s: &str) {
        let sid = self.sid();
        let (r0, c0) = (self.anchor.0.min(self.cursor.0), self.anchor.1.min(self.cursor.1));
        let ours = self.clip.as_ref().is_some_and(|clip| clip.text.trim_end() == s.trim_end().replace("\r\n", "\n"));        if let Some((src, fp)) = self.cut.take().filter(|(_, fp)| ours && *fp == files::fingerprint(&self.eng.wb)) {
            if self.move_block(src, (r0, c0)) {
                // the cut is used up: pasting again copies the cells from where they are now
                self.copy(ctx);
                self.status = Some(format!("moved {}×{}", src.rows(), src.cols()));
            } else {
                self.cut = Some((src, fp));
            }
            return;
        }
        let edit = match &self.clip {
            Some(clip) if ours => {
                let clip = clip.clone();
                ops::paste(&mut self.eng, &clip, sid, (r0, c0))
            }
            _ => ops::paste_text(&mut self.eng, s, sid, (r0, c0)),
        };
        self.exec(edit);
    }

    /// Moves `src` so its top-left is at `at` on this sheet and selects it there; false (with the
    /// reason in the status bar) if it can't be moved.
    pub(super) fn move_block(&mut self, src: CRect, at: (usize, usize)) -> bool {
        let sid = self.sid();
        match ops::move_cells(&mut self.eng, src, sid, at) {
            Ok(e) => {
                if !matches!(&e, Edit::Cells(c) if c.is_empty()) {
                    self.exec(e);
                }
                self.anchor = at;
                self.cursor = (at.0 + src.rows() - 1, at.1 + src.cols() - 1);
                true
            }
            Err(msg) => {
                self.status = Some(msg);
                false
            }
        }
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

    /// Byte span of the token a cell's error points at, if any.
    pub(super) fn err_span(&self, k: CellKey) -> Option<Range<usize>> {
        match self.eng.shown(k) {
            Shown::Error(e) if e.kind == ErrKind::Local => e.span.clone(),
            _ => self.eng.compile_error(k).and_then(|e| e.span.clone()),
        }
    }

    /// The shared formula editor (used in the cell and in the formula bar).
    pub(super) fn editor(&mut self, ui: &mut Ui, id: Id, width: f32, in_bar: bool) -> EditorOut {
        let ctx = ui.ctx().clone();
        if ctx.memory(|m| m.has_focus(id)) {
            self.completion_keys(&ctx);
        }
        let ed = self.edit.as_ref().unwrap();
        let refs = syntax::analyze(&ed.text, &self.eng.wb, ed.key.sheet);
        // underline the error while the text is still what produced it
        let orig = ed.orig.clone();
        let err = self.err_span(ed.key);
        let font = FontId::proportional(FONT);
        let base = ui.visuals().text_color();
        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _w: f32| {
            let err = err.as_ref().filter(|_| buf.as_str() == orig);
            let job = syntax::layout(buf.as_str(), &refs, err, font.clone(), base);
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
        let mut placed = false;
        if let Some((pass, caret)) = self.focus_req {
            if ed.in_bar == in_bar && ctx.cumulative_pass_nr() >= pass {
                resp.request_focus();
                out.state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
                out.state.store(&ctx, id);
                self.focus_req = None;
                placed = true;
            }
        }
        if resp.changed() {
            ed.ref_span = None;
            ed.pick = None;
            ed.comp_closed = false;
        }
        if resp.has_focus() {
            ed.in_bar = in_bar;
            if let Some(cr) = out.cursor_range.filter(|_| !placed) {
                if cr.primary.index.0 != ed.cursor {
                    ed.cursor = cr.primary.index.0;
                    ed.pick = None;
                    ed.comp_closed = false;
                }
            }
            // while the completion list shows, Escape (and Tab, once one is highlighted) are ours, not egui's focus keys
            let list = self.completions().is_some();
            let picked = list && self.edit.as_ref().is_some_and(|e| e.pick.is_some());
            let filter = egui::EventFilter { tab: picked, horizontal_arrows: true, vertical_arrows: true, escape: list };
            ctx.memory_mut(|m| m.set_focus_lock_filter(id, filter));
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
                self.run(ui.ctx(), Command::Save);
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
            if ui.button("Help").on_hover_text("F1 — help for the selected cell or the word at the cursor").clicked() {
                if self.help.open && self.help.embedded() {
                    self.help.close();
                } else {
                    self.help.show();
                }
            }
            ui.separator();
            let file = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Untitled".into());
            let file = if self.dirty { format!("{file} (edited)") } else { file };
            ui.label(egui::RichText::new(format!("recalc {:.2} ms · {file}", self.last_recalc_ms)).weak().small());
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
            let err_span = self.err_span(k);
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
                    // hover a token for its documentation
                    let ci = galley.cursor_from_pos(hp - pos).index.0;
                    let at = syntax::char_to_byte(&text_now, ci);
                    if hp.x <= pos.x + galley.size().x + 2.0 && kind.has_refs() {
                        if let Some(tip) = self.token_tooltip(&text_now, at, k.sheet) {
                            resp.clone().on_hover_text_at_pointer(tip);
                        }
                    }
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
        // the hint strip always takes the same height, so starting an edit doesn't shift the grid
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ASSIST_H), Sense::hover());
        let mut strip = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        strip.set_clip_rect(rect.intersect(ui.clip_rect()));
        if self.edit.is_some() {
            self.assist(&mut strip);
        } else {
            self.idle_hint(&mut strip);
        }
        ui.add_space(2.0);
    }

    /// The hint strip when not editing: what kind of cell is selected.
    fn idle_hint(&mut self, ui: &mut Ui) {
        let k = {
            let (r, c) = self.cursor;
            self.key(r, c)
        };
        let kind = self.eng.kind(k);
        let hint = match self.eng.spill_anchor(k) {
            Some(a) if kind == Kind::Empty => format!("spilled from {} — read-only, edit the source", self.label(a)),
            _ => kind_hint(kind).to_string(),
        };
        ui.horizontal(|ui| {
            ui.add_space(176.0);
            ui.label(RichText::new(hint).small().weak());
        });
    }

    /// Tooltip text for the token at byte `at` of a cell's text.
    fn token_tooltip(&self, text: &str, at: usize, home: SheetId) -> Option<String> {
        use wbs_core::lex::Tok;
        let t = syntax::token_at(text, at)?;
        let src = &text[t.span.clone()];
        let value = |prog: &str| match self.eng.eval_scratch(&format!("={prog}"), home).result {
            Ok(v) => v.summary(6),
            Err(e) => format!("error: {}", e.msg),
        };
        Some(match &t.tok {
            Tok::Ref(_) | Tok::Range(..) => format!("{src} = {}", value(src)),
            Tok::Word(w) if self.eng.wb.names.contains_key(w.as_str()) => {
                let k = self.eng.wb.names[w.as_str()].cell;
                format!("{w} — name for {} = {}", self.eng.wb.cell_label(k, Some(home)), value(w))
            }
            Tok::Word(w) => help_view::word_hint(&self.eng, w)?,
            Tok::Unit(u) | Tok::To(u) => {
                let names: Vec<String> = wbs_core::units::parse_unit(u)
                    .map(|e| e.terms.into_iter().map(|(n, _)| n).collect())
                    .unwrap_or_default();
                let mut lines = vec![help::doc_for_token(&t.tok).map(help_view::doc_line).unwrap_or_default()];
                for n in names {
                    let info = self.eng.units_list().into_iter().find(|(m, ..)| *m == n);
                    lines.push(match info {
                        Some((_, k, Some(i))) => format!("[{n}]: {} · defined at {}", i.dim, self.eng.wb.cell_label(k, Some(home))),
                        Some((_, k, None)) => format!("[{n}]: error in its definition at {}", self.eng.wb.cell_label(k, Some(home))),
                        None => format!("[{n}]: unknown unit"),
                    });
                }
                lines.join("\n")
            }
            tok => help::doc_for_token(tok).map(help_view::doc_line)?,
        })
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            self.sheet_tabs(ui);
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

    /// Click a tab to switch, double-click to rename, right-click for more, drag to reorder.
    fn sheet_tabs(&mut self, ui: &mut Ui) {
        let tabs: Vec<(SheetId, String)> = self.eng.wb.sheets.iter().map(|s| (s.id, s.name.clone())).collect();
        let n = tabs.len();
        let mut act = None;
        let mut rects = Vec::with_capacity(n);
        let mut dropped = false;
        for (i, (id, name)) in tabs.iter().enumerate() {
            let id = *id;
            if let Some((_, buf)) = self.tabs.rename.as_mut().filter(|(r, _)| *r == id) {
                let r = ui.add(TextEdit::singleline(buf).id(Id::new(("rename sheet", id.0))).desired_width(90.0));
                if std::mem::take(&mut self.tabs.focus_rename) {
                    r.request_focus();
                }
                if r.lost_focus() {
                    let new = std::mem::take(buf);
                    self.tabs.rename = None;
                    if !ui.input(|i| i.key_pressed(Key::Escape)) {
                        act = Some(TabAct::Rename(id, new));
                    }
                }
                rects.push(r.rect);
                continue;
            }
            let r = ui.add(egui::Button::selectable(i == self.sheet_ix, name.as_str()).sense(Sense::click_and_drag()));
            if r.double_clicked() {
                act = Some(TabAct::StartRename(id));
            } else if r.clicked() {
                act = Some(TabAct::Show(i));
            }
            if r.drag_started() {
                self.tabs.drag = Some(id);
            }
            dropped |= r.drag_stopped();
            r.context_menu(|ui| {
                let mut item = |ui: &mut Ui, label: &str, enabled: bool, a: TabAct| {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        act = Some(a);
                        ui.close();
                    }
                };
                item(ui, "Rename", true, TabAct::StartRename(id));
                item(ui, "Duplicate", true, TabAct::Duplicate(id));
                item(ui, "Move left", i > 0, TabAct::Move(id, i.saturating_sub(1)));
                item(ui, "Move right", i + 1 < n, TabAct::Move(id, i + 1));
                ui.separator();
                item(ui, "Delete", n > 1, TabAct::Delete(id, false));
            });
            rects.push(r.rect);
        }
        if let Some(d) = self.tabs.drag {
            let from = tabs.iter().position(|t| t.0 == d);
            let at = ui.input(|i| i.pointer.latest_pos());
            if let (Some(from), Some(p)) = (from, at) {
                // the drop index counts the other tabs left of the pointer
                let others: Vec<Rect> = rects.iter().enumerate().filter(|(j, _)| *j != from).map(|(_, r)| *r).collect();
                let to = others.iter().filter(|r| r.center().x < p.x).count();
                if to != from {
                    let x = if to == 0 { others[0].left() - 2.0 } else { others[to - 1].right() + 2.0 };
                    let y = rects[from].y_range();
                    ui.painter().vline(x, y, Stroke::new(2.0, ui.visuals().selection.stroke.color));
                }
                if dropped && to != from {
                    act = Some(TabAct::Move(d, to));
                }
            }
            if dropped || from.is_none() {
                self.tabs.drag = None;
            }
        }
        if ui.small_button("+").on_hover_text("add a sheet").clicked() {
            act = Some(TabAct::Add);
        }
        if let Some(d) = self.tabs.confirm_delete {
            match self.eng.wb.sheet(d) {
                None => self.tabs.confirm_delete = None,
                Some(s) => {
                    ui.separator();
                    let msg = format!(
                        "Delete {}? It defines {} units and dimensions; cells using them will show errors.",
                        s.name,
                        self.eng.declarations_on(d)
                    );
                    ui.label(RichText::new(msg).color(Color32::from_rgb(0xdc, 0x26, 0x26)));
                    if ui.button("Delete sheet").clicked() {
                        act = Some(TabAct::Delete(d, true));
                    }
                    if ui.button("Cancel").clicked() {
                        self.tabs.confirm_delete = None;
                    }
                }
            }
        }
        let Some(act) = act else { return };
        let edit = match act {
            TabAct::Show(i) => {
                if i != self.sheet_ix {
                    let prev = self.sid();
                    self.show_sheet(i, prev);
                }
                return;
            }
            TabAct::StartRename(id) => {
                let name = self.eng.wb.sheet(id).map(|s| s.name.clone()).unwrap_or_default();
                self.tabs.rename = Some((id, name));
                self.tabs.focus_rename = true;
                return;
            }
            TabAct::Rename(id, name) => {
                if self.eng.wb.sheet(id).is_some_and(|s| s.name == name.trim()) {
                    return;
                }
                self.eng.rename_sheet_edit(id, &name)
            }
            TabAct::Delete(id, confirmed) => {
                if !confirmed && self.eng.declarations_on(id) > 0 {
                    self.tabs.confirm_delete = Some(id);
                    return;
                }
                self.tabs.confirm_delete = None;
                self.eng.delete_sheet_edit(id)
            }
            TabAct::Add => Ok(self.eng.add_sheet_edit(n)),
            TabAct::Duplicate(id) => self.eng.duplicate_sheet_edit(id),
            TabAct::Move(id, to) => Ok(Edit::MoveSheet { sheet: id, to }),
        };
        match edit {
            Ok(e) => {
                self.commit();
                let deleted = match &e {
                    Edit::DeleteSheet { sheet } => self.eng.wb.sheet(*sheet).map(|s| s.name.clone()),
                    _ => None,
                };
                self.exec(e);
                if let Some(name) = deleted {
                    self.status = Some(format!("deleted sheet {name} — ⌘Z brings it back"));
                }
            }
            Err(m) => self.status = Some(m),
        }
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let (r, c) = self.cursor;
        let k = self.key(r, c);
        ui.add_space(4.0);
        ui.heading(self.label(k));
        let kind = self.eng.kind(k);
        let anchor = self.eng.spill_anchor(k);
        let (kind_label, topic) = match (kind, anchor) {
            (Kind::Empty, Some(_)) => ("spilled (read-only)".to_string(), "spill"),
            (k, _) => (k.label().to_string(), kind_topic(k)),
        };
        if ui.link(RichText::new(format!("{kind_label}  ?")).weak()).on_hover_text("what is this kind of cell?").clicked() {
            self.help.show_page(Page::Topic(topic));
        }
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
                if let Some(h) = help::explain_error(&e.msg) {
                    egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.label(RichText::new(h.title).strong());
                            ui.label(RichText::new(h.why).small());
                            ui.label(RichText::new(format!("Fix: {}", h.fix)).small());
                            if ui.link("Learn more").clicked() {
                                self.help.show_page(Page::Topic(h.topic));
                            }
                        });
                }
            }
            Shown::Empty => {
                ui.label(egui::RichText::new("empty").weak());
            }
        }
        if let Some(g) = goto {
            self.goto(g);
        }
        if matches!(kind, Kind::Program) {
            egui::CollapsingHeader::new("Step through").id_salt("step_through").show(ui, |ui| {
                let text = self.eng.wb.cell_text(k);
                let s = self.eng.trace_cell(k);
                help_view::trace_table(ui, &text, &s);
            });
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

/// Height of the hint strip under the formula bar (two rows: hint, stack).
const ASSIST_H: f32 = 42.0;

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

/// The guide that explains a kind of cell.
fn kind_topic(kind: Kind) -> &'static str {
    match kind {
        Kind::Empty => "welcome",
        Kind::Text | Kind::Number => "cells",
        Kind::Program => "stack",
        Kind::WordDef => "words",
        Kind::UnitDecl => "defining-units",
    }
}

/// What the hint strip says when the caret isn't on a known token.
fn kind_hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Program => "program — values first, then the word: =2 3 +   ·   click cells to insert references   ·   F1 for help",
        Kind::WordDef => "word definition — : name ( doc ) body ;",
        Kind::UnitDecl => "unit declaration — dim name · base [unit] dim · [unit] = value",
        Kind::Number => "number — optionally with a unit: 5 [m/s]",
        Kind::Text => "text — start with = for a program, : for a word",
        Kind::Empty => "type a value, or = to start a program",
    }
}
