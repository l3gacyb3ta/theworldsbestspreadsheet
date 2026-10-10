//! The grid: geometry, pointer interaction (select, click-to-reference,
//! fill-drag, resize, scrub, chart point drags) and painting.

use super::*;
use crate::chart_view::{self, Tip};
use std::collections::HashSet;
use wbs_core::solve::Goal;

impl App {
    fn geometry(&self, area: Rect) -> Geo {
        let cells = Rect::from_min_max(area.min + Vec2::new(HDR_W, HDR_H), area.max);
        let s = self.sheet();
        let (nr, nc) = self.extent();
        // the extent, and always room to scroll into: rows past the stored ones are virtual, so
        // this lays out as many as it likes without touching the document
        let mut col_x = Vec::with_capacity(nc + 1);
        let mut acc = 0.0;
        col_x.push(0.0);
        while col_x.len() <= nc || acc < self.scroll.x + cells.width() + 300.0 {
            let Some(c) = s.cols.get(col_x.len() - 1) else { break };
            acc += s.col_widths.get(&c).copied().unwrap_or(DEF_W);
            col_x.push(acc);
        }
        let mut row_y = Vec::with_capacity(nr + 1);
        acc = 0.0;
        row_y.push(0.0);
        while row_y.len() <= nr || acc < self.scroll.y + cells.height() + 400.0 {
            let Some(r) = s.rows.get(row_y.len() - 1) else { break };
            acc += s.row_heights.get(&r).copied().unwrap_or(DEF_H);
            row_y.push(acc);
        }
        Geo { cells, col_x, row_y, scroll: self.scroll }
    }

    /// What a cell shows, its colour, whether it's right-aligned, and (numbers only) shorter
    /// ways to write it for when it doesn't fit its column, longest first.
    fn cell_display(&self, k: CellKey, pal: &Pal) -> Option<(String, Color32, bool, Vec<String>)> {
        match self.eng.shown(k) {
            Shown::Empty => None,
            Shown::Error(e) => Some((e.short().to_string(), pal.err, false, vec![])),
            Shown::Value { value, dr, dc, anchor } => {
                let spilled = anchor != k;
                let (color, right) = match value {
                    Value::Chart(_) => return None,
                    Value::Num(_) => (if spilled { pal.spill } else { pal.text }, true),
                    Value::Text(_) => (if spilled { pal.spill } else { pal.text }, false),
                    _ => (pal.decl, false),
                };
                let text = match value {
                    Value::Unit(_) | Value::Dim(_) | Value::Word(_) => self.eng.wb.cell_text(k),
                    _ => value.display_at(dr, dc),
                };
                let shorter = match value {
                    Value::Num(n) if n.rank() <= 2 && !n.data.is_empty() => {
                        let i = if n.rank() == 2 { dr * n.shape[1] + dc } else { dr };
                        n.fmt_elem_shorter(i).into_iter().skip(1).collect()
                    }
                    _ => vec![],
                };
                self.fonts.note(&text);
                Some((text, color, right, shorter))
            }
        }
    }

    pub(super) fn grid(&mut self, ui: &mut Ui, dark: bool) {
        let ctx = ui.ctx().clone();
        let pal = Pal::new(dark);
        let area = ui.max_rect();
        let resp = ui.allocate_rect(area, Sense::click_and_drag());
        let sid = self.sid();

        // scrolling
        if resp.hovered() {
            let d = ui.input(|i| i.smooth_scroll_delta());
            self.scroll -= d;
        }
        self.scroll = self.scroll.max(Vec2::ZERO);
        let mut g = self.geometry(area);
        if self.scroll_into_view {
            self.scroll_into_view = false;
            let r = g.cell(self.cursor.0, self.cursor.1);
            let cells = g.cells;
            if r.top() < cells.top() {
                self.scroll.y -= cells.top() - r.top();
            } else if r.bottom() > cells.bottom() {
                self.scroll.y += r.bottom() - cells.bottom();
            }
            if r.left() < cells.left() {
                self.scroll.x -= cells.left() - r.left();
            } else if r.right() > cells.right() {
                self.scroll.x += r.right() - cells.right();
            }
            self.scroll = self.scroll.max(Vec2::ZERO);
            g = self.geometry(area);
        }

        self.pointer(ui, &ctx, &resp, &g, area);
        let g = self.geometry(area);
        self.geo = Some(g.clone());
        self.paint(ui, &ctx, &pal, &g, area, dark, sid);
        self.cell_editor(ui, &g, &pal, sid);
        self.ime_area(&ctx, g.cell(self.cursor.0, self.cursor.1));
        self.offer_banner(ui, &g, sid);
        self.context_menu(&resp);
    }

    fn pointer(&mut self, ui: &mut Ui, ctx: &egui::Context, resp: &egui::Response, g: &Geo, area: Rect) {
        let (pos, pressed, down, dbl, secondary, mods) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.button_double_clicked(PointerButton::Primary) || i.pointer.button_triple_clicked(PointerButton::Primary),
                i.pointer.secondary_pressed(),
                i.modifiers,
            )
        });
        let Some(pos) = pos else { return };
        let over_editor = self.editor_rect.is_some_and(|r| r.contains(pos));
        let in_cells = g.cells.contains(pos);
        let in_col_hdr = pos.y >= area.top() && pos.y < g.cells.top() && pos.x >= g.cells.left();
        let in_row_hdr = pos.x >= area.left() && pos.x < g.cells.left() && pos.y >= g.cells.top();
        let hovering = resp.hovered() && !over_editor;

        // hover feedback
        if hovering && matches!(self.drag, Drag::None) {
            if in_col_hdr && self.col_border(g, pos).is_some() {
                ctx.set_cursor_icon(CursorIcon::ResizeColumn);
            } else if in_row_hdr && self.row_border(g, pos).is_some() {
                ctx.set_cursor_icon(CursorIcon::ResizeRow);
            } else if in_cells {
                if let Some((hit, axis)) = self.chart_hit(pos) {
                    let label = self.hit_label(hit, axis);
                    self.fonts.note(&label);
                    // scatter and path points move along each axis whose value comes straight from a number cell
                    let xk = match hit.xprov {
                        Prov::Literal(k) if hit.prov != Prov::Literal(k) => Some(k),
                        _ => None,
                    };
                    let why = |p: Prov| match p {
                        Prov::Derived(k, i) => format!("computed in {}", self.label(self.eng.element_cell(k, i))),
                        _ => "computed by the chart's own program".to_string(),
                    };
                    match (hit.prov, xk) {
                        (Prov::Literal(k), Some(xk)) => {
                            ctx.set_cursor_icon(CursorIcon::Move);
                            chart_view::tooltip_lines(
                                ctx,
                                pos,
                                &[
                                    Tip::Value(label.clone()),
                                    Tip::Action(format!("drag to edit {} (x) and {} (y)", self.label(xk), self.label(k))),
                                    Tip::Note("hold Alt to lock to one axis, Shift for finer steps".to_string()),
                                ],
                            );
                        }
                        (p, Some(xk)) => {
                            ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                            chart_view::tooltip_lines(
                                ctx,
                                pos,
                                &[
                                    Tip::Value(label.clone()),
                                    Tip::Action(format!("drag sideways to edit {}", self.label(xk))),
                                    Tip::Note(format!("y is {}, so only x moves", why(p))),
                                ],
                            );
                        }
                        (Prov::Literal(k), None) if hit.two_d && hit.xprov != Prov::Literal(k) => {
                            ctx.set_cursor_icon(CursorIcon::ResizeVertical);
                            chart_view::tooltip_lines(
                                ctx,
                                pos,
                                &[
                                    Tip::Value(label.clone()),
                                    Tip::Action(format!("drag up and down to edit {}", self.label(k))),
                                    Tip::Note(format!("x is {}, so only y moves", why(hit.xprov))),
                                ],
                            );
                        }
                        (Prov::Literal(k), None) => {
                            ctx.set_cursor_icon(CursorIcon::ResizeVertical);
                            chart_view::tooltip_lines(ctx, pos, &[Tip::Value(label.clone()), Tip::Action(format!("drag to edit {}", self.label(k)))]);
                        }
                        (Prov::Derived(k, i), None) => {
                            let elem = self.eng.element_cell(k, i);
                            let from = if elem == k { format!("computed in {}", self.label(k)) } else { format!("{}, spilled from {}", self.label(elem), self.label(k)) };
                            let mut tip = vec![Tip::Value(label.clone())];
                            match self.goal_choice(k) {
                                None => tip.push(Tip::Note("not draggable: no number cells upstream to goal-seek".to_string())),
                                Some((pick, cands)) => {
                                    ctx.set_cursor_icon(CursorIcon::ResizeVertical);
                                    tip.push(Tip::Action(format!("drag to goal-seek {}", self.input_name(pick))));
                                    let at = cands.iter().position(|c| *c == pick).unwrap_or(0);
                                    let others: Vec<String> = (1..cands.len()).take(4).map(|j| self.input_name(cands[(at + j) % cands.len()])).collect();
                                    // a goal-seek solves for one input, so it never moves x too
                                    if hit.two_d {
                                        tip.push(Tip::Note(format!("x is {} too, so only y moves", why(hit.xprov))));
                                    }
                                    if !others.is_empty() {
                                        tip.push(Tip::Note(format!("click to switch to {}", others.join(", "))));
                                    }
                                }
                            }
                            tip.push(Tip::Note(from));
                            chart_view::tooltip_lines(ctx, pos, &tip);
                        }
                        (Prov::None, None) => chart_view::tooltip_lines(ctx, pos, &[Tip::Value(label), Tip::Note("computed by the chart's own program: not draggable".to_string())]),
                    }
                } else if self.fill_handle(g).contains(pos) && self.edit.is_none() {
                    ctx.set_cursor_icon(CursorIcon::Crosshair);
                } else if self.on_sel_border(g, pos, mods) {
                    ctx.set_cursor_icon(if mods.alt { CursorIcon::Copy } else { CursorIcon::Grab });
                } else if mods.alt {
                    let (r, c) = (g.row_at(pos.y), g.col_at(pos.x));
                    if let Some(k) = self.sheet().key(r, c) {
                        if ops::cell_literal(&self.eng.wb.cell_text(k)).is_some() {
                            ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                        }
                    }
                }
            }
        }

        if secondary && hovering && in_cells {
            let (r, c) = (g.row_at(pos.y), g.col_at(pos.x));
            if !self.sel().contains(r, c) {
                self.commit();
                self.select(r, c, false);
            }
        }

        if pressed && hovering {
            self.press(ctx, g, pos, area, dbl, mods, in_cells, in_col_hdr, in_row_hdr);
        } else if dbl && hovering && !self.editing_formula() {
            // egui reports a double click on the second release, a frame after its press
            self.press(ctx, g, pos, area, dbl, mods, in_cells, in_col_hdr, in_row_hdr);
        }

        if down {
            self.drag_update(ctx, g, pos, mods);
        } else if !matches!(self.drag, Drag::None) {
            self.drag_end();
        }
    }

    fn col_border(&self, g: &Geo, pos: Pos2) -> Option<usize> {
        let c = g.col_at(pos.x);
        if (pos.x - g.x(c + 1)).abs() < 5.0 {
            Some(c)
        } else if c > 0 && (pos.x - g.x(c)).abs() < 5.0 {
            Some(c - 1)
        } else {
            None
        }
    }
    fn row_border(&self, g: &Geo, pos: Pos2) -> Option<usize> {
        let r = g.row_at(pos.y);
        if (pos.y - g.y(r + 1)).abs() < 4.0 {
            Some(r)
        } else if r > 0 && (pos.y - g.y(r)).abs() < 4.0 {
            Some(r - 1)
        } else {
            None
        }
    }
    fn fill_handle(&self, g: &Geo) -> Rect {
        let s = self.sel();
        let br = g.rect(s.r0, s.c0, s.r1, s.c1).right_bottom();
        Rect::from_center_size(br, Vec2::splat(9.0))
    }
    /// Within a few points of the selection's outline (not its fill handle): dragging there moves the selection.
    fn on_sel_border(&self, g: &Geo, pos: Pos2, mods: Modifiers) -> bool {
        if self.edit.is_some() || mods.shift || mods.command || self.fill_handle(g).contains(pos) {
            return false;
        }
        let s = self.sel();
        let r = g.rect(s.r0, s.c0, s.r1, s.c1);
        r.expand(3.0).contains(pos) && !r.shrink(3.0).contains(pos)
    }
    /// A chart point's `x → y`.
    pub(super) fn hit_label(&self, hit: &PointHit, axis: &YAxis) -> String {
        match axis.anchor.and_then(|a| self.eng.result(a)) {
            Some(Ok(Value::Chart(c))) => chart_view::point_label(c, hit.layer, hit.index),
            _ => String::new(),
        }
    }

    fn chart_hit(&self, pos: Pos2) -> Option<(&PointHit, &YAxis)> {
        self.chart_hits
            .iter()
            .filter(|(h, _)| h.pos.distance(pos) < 8.0)
            .min_by(|a, b| a.0.pos.distance(pos).partial_cmp(&b.0.pos.distance(pos)).unwrap())
            .map(|(h, a)| (h, a))
    }
    /// The number cells a computed cell's points can goal-seek (named inputs first), and the one they do:
    /// the first, unless a click on one of its points switched it.
    fn goal_choice(&self, target: CellKey) -> Option<(CellKey, Vec<CellKey>)> {
        let cands = self.eng.upstream_inputs(target);
        let pick = self.goal_inputs.get(&target).copied().filter(|k| cands.contains(k)).or(cands.first().copied())?;
        Some((pick, cands))
    }
    /// The literal number cell a chart point's value comes straight from, which dragging it writes.
    fn point_cell(&self, p: Prov) -> Option<PointCell> {
        let Prov::Literal(key) = p else { return None };
        let text = self.eng.wb.cell_text(key);
        let lit = ops::cell_literal(&text)?;
        let Some(Ok(Value::Num(n))) = self.eng.result(key) else { return None };
        Some(PointCell { key, orig: self.eng.wb.cell(key).cloned(), disp: n.q.disp.clone(), text, lit })
    }
    /// `growth (B4)`, or `B4` for an unnamed cell.
    fn input_name(&self, k: CellKey) -> String {
        match self.eng.name_of(k) {
            Some(n) => format!("{n} ({})", self.label(k)),
            None => self.label(k),
        }
    }
    fn shown_text(&self, k: CellKey) -> String {
        match self.eng.shown(k) {
            Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
            Shown::Error(e) => e.short().to_string(),
            Shown::Empty => String::new(),
        }
    }

    fn goal_update(&mut self, ctx: &egui::Context, g: &mut GoalDrag, pos: Pos2, mods: Modifiers) {
        ctx.set_cursor_icon(CursorIcon::ResizeVertical);
        g.pointer = pos;
        if !g.moved {
            if pos.distance(g.press) < 3.0 {
                return;
            }
            g.moved = true;
        }
        let want = g.axis.from_screen(pos.y);
        if g.want == Some(want) && g.fine == mods.shift {
            return;
        }
        g.want = Some(want);
        g.fine = mods.shift;
        if g.live {
            self.goal_run(g, true);
        }
    }

    /// Solves for the pointer's value and writes the input, or puts it back as it was if there's no answer.
    /// `live`: a solve slower than `goal_live_ms` writes nothing and leaves the rest of the drag to the release.
    fn goal_run(&mut self, g: &mut GoalDrag, live: bool) {
        let Some(want) = g.want else { return };
        let a = &g.axis;
        let goal = Goal {
            target: g.target,
            index: g.index,
            want: a.disp.to_canonical(want),
            // a thousandth of the axis: finer than the pointer can aim
            tol: (a.y1 - a.y0).abs() / 1000.0 * a.disp.factor.abs(),
            input: g.input,
            decimals: Some(g.decimals + if g.fine { 2 } else { 0 }),
        };
        let t = std::time::Instant::now();
        let res = self.eng.goal_seek(&goal);
        g.solve_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.last_recalc_ms = g.solve_ms;
        if live && g.solve_ms > self.goal_live_ms {
            g.live = false;
            g.outcome = None;
            if self.eng.wb.cell(g.input) != g.orig.as_ref() {
                self.eng.apply(Edit::Cells(vec![(g.input, g.orig.clone())]));
            }
            return;
        }
        g.outcome = Some(match res {
            Ok(s) => {
                if s.text != self.eng.wb.cell_text(g.input) {
                    self.eng.set_text(g.input, &s.text);
                }
                Ok(s.text)
            }
            Err(e) => {
                if self.eng.wb.cell(g.input) != g.orig.as_ref() {
                    self.eng.apply(Edit::Cells(vec![(g.input, g.orig.clone())]));
                }
                Err(e)
            }
        });
    }

    fn goal_end(&mut self, mut g: GoalDrag) {
        if !g.moved {
            // a click: the next candidate input
            if let Some((pick, cands)) = self.goal_choice(g.target) {
                let at = cands.iter().position(|c| *c == pick).unwrap_or(0);
                let next = cands[(at + 1) % cands.len()];
                self.goal_inputs.insert(g.target, next);
                self.status = Some(format!("dragging {}'s points now goal-seeks {}", self.label(g.target), self.input_name(next)));
            }
            return;
        }
        if !g.live {
            self.goal_run(&mut g, false);
        }
        let elem = self.eng.element_cell(g.target, g.index);
        match &g.outcome {
            Some(Ok(text)) => {
                self.status = Some(format!("goal-seek: {} = {text} puts {} at {}", self.input_name(g.input), self.label(elem), self.shown_text(elem)))
            }
            Some(Err(e)) => {
                self.status = Some(format!("goal-seek: no answer — {e}"));
                self.goal_note = Some((g.target, g.index, e.clone()));
            }
            None => {}
        }
        if self.eng.wb.cell(g.input) != g.orig.as_ref() {
            self.undo.push(Edit::Cells(vec![(g.input, g.orig)]));
            self.redo.clear();
        }
    }

    /// The ghost line at the pointer's value and what the solve says, while dragging a computed point.
    fn paint_goal(&self, ctx: &egui::Context, cp: &Painter, pal: &Pal) {
        let Drag::Goal(g) = &self.drag else { return };
        if !g.moved {
            return;
        }
        let Some(want) = g.want else { return };
        let y = g.axis.to_screen(want);
        let plot = g.axis.plot;
        cp.extend(egui::Shape::dashed_line(&[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.5, pal.sel), 5.0, 4.0));
        cp.circle_stroke(Pos2::new(g.at.x, y), 5.0, Stroke::new(2.0, pal.sel));
        let res = ((g.axis.y1 - g.axis.y0).abs() / 1000.0).max(f64::MIN_POSITIVE);
        let step = 10f64.powf(res.log10().floor());
        let shown = wbs_core::value::group_thousands(&wbs_core::value::fmt_num((want / step).round() * step));
        let unit = if g.axis.disp.is_none() { String::new() } else { format!(" {}", g.axis.disp) };
        let input = self.input_name(g.input);
        let head = format!("goal: {} = {shown}{unit}", self.label(self.eng.element_cell(g.target, g.index)));
        let mut tip = vec![Tip::Value(head)];
        match &g.outcome {
            _ if !g.live => {
                tip.push(Tip::Action(format!("release to goal-seek {input}")));
                tip.push(Tip::Note(format!("a solve takes {:.0} ms, too slow to follow the pointer", g.solve_ms)));
            }
            Some(Ok(text)) => tip.push(Tip::Action(format!("{input} = {text}"))),
            Some(Err(e)) => {
                tip.push(Tip::Action(format!("no answer: {input} stays as it was")));
                tip.push(Tip::Note(e.to_string()));
            }
            None => tip.push(Tip::Note(format!("solving {input}…"))),
        }
        chart_view::tooltip_lines(ctx, g.pointer, &tip);
    }

    /// Why the last goal-seek failed, next to its point, until the next press.
    fn paint_goal_note(&self, cp: &Painter, pal: &Pal) {
        let Some((target, index, msg)) = &self.goal_note else { return };
        let Some((hit, _)) = self.chart_hits.iter().find(|(h, _)| h.prov == Prov::Derived(*target, *index)) else { return };
        let galley = cp.layout(format!("no answer: {msg}"), FontId::proportional(11.5), pal.err, 260.0);
        let r = Rect::from_min_size(hit.pos + Vec2::new(10.0, 8.0), galley.size() + Vec2::splat(10.0));
        cp.rect_filled(r, 4.0, pal.bg);
        cp.rect_stroke(r, 4.0, Stroke::new(1.0, pal.err), StrokeKind::Inside);
        cp.galley(r.min + Vec2::splat(5.0), galley, pal.err);
    }

    #[allow(clippy::too_many_arguments)]
    fn press(
        &mut self,
        ctx: &egui::Context,
        g: &Geo,
        pos: Pos2,
        area: Rect,
        dbl: bool,
        mods: Modifiers,
        in_cells: bool,
        in_col_hdr: bool,
        in_row_hdr: bool,
    ) {
        let ix = self.sheet_ix;
        self.goal_note = None;
        if in_col_hdr {
            if let Some(c) = self.col_border(g, pos) {
                if dbl {
                    self.autofit(ctx, c);
                } else {
                    let cid = self.sheet().cols.get(c).unwrap();
                    let w0 = self.sheet().col_widths.get(&cid).copied();
                    self.drag = Drag::Col { col: c, x0: pos.x, w0 };
                }
                return;
            }
            let c = g.col_at(pos.x);
            if self.editing_formula() {
                let last = self.sheet().used_extent().0.max(1) - 1;
                self.insert_ref(ctx, (0, c), (last, c));
                return;
            }
            self.commit();
            // a whole column: down to the last row laid out
            let last = g.row_y.len() - 2;
            if mods.shift {
                self.anchor.0 = 0;
                self.cursor = (last, c);
            } else {
                self.anchor = (0, c);
                self.cursor = (last, c);
            }
            return;
        }
        if in_row_hdr {
            if let Some(r) = self.row_border(g, pos) {
                let rid = self.sheet().rows.get(r).unwrap();
                let h0 = self.sheet().row_heights.get(&rid).copied();
                self.drag = Drag::Row { row: r, y0: pos.y, h0 };
                return;
            }
            self.commit();
            let r = g.row_at(pos.y);
            let last = g.col_x.len() - 2;
            if mods.shift {
                self.anchor.1 = 0;
                self.cursor = (r, last);
            } else {
                self.anchor = (r, 0);
                self.cursor = (r, last);
            }
            return;
        }
        if !in_cells {
            if pos.x < g.cells.left() && pos.y < g.cells.top() && area.contains(pos) {
                self.commit();
                let end = (g.row_y.len() - 2, g.col_x.len() - 2);
                self.anchor = (0, 0);
                self.cursor = end;
            }
            return;
        }
        let (r, c) = (g.row_at(pos.y), g.col_at(pos.x));
        // 1. click-to-reference while editing a formula
        if self.editing_formula() && !dbl {
            if let Some(ed) = &mut self.edit {
                ed.ref_span = None;
            }
            self.insert_ref(ctx, (r, c), (r, c));
            self.drag = Drag::Ref { start: (r, c) };
            return;
        }
        // 2. dragging a chart point: the literal cells behind it (its y; its x too on a scatter or path) are written,
        // or, with neither, a computed y goal-seeks an input
        if self.edit.is_none() {
            if let Some((hit, axis)) = self.chart_hit(pos) {
                let y = self.point_cell(hit.prov);
                let x = if hit.two_d { self.point_cell(hit.xprov).filter(|x| y.as_ref().is_none_or(|y| y.key != x.key)) } else { None };
                if x.is_some() || y.is_some() {
                    let axis = axis.clone();
                    // (a press with Alt held drags the point, locked to one axis, rather than scrubbing the cell under it)
                    self.drag = Drag::Point { y, x, axis, press: pos };
                    return;
                }
                if let Prov::Derived(target, index) = hit.prov {
                    if let Some((input, _)) = self.goal_choice(target) {
                        let decimals = ops::cell_literal(&self.eng.wb.cell_text(input)).map_or(0, |l| l.decimals);
                        let (axis, at, orig) = (axis.clone(), hit.pos, self.eng.wb.cell(input).cloned());
                        let g = GoalDrag { target, index, input, orig, axis, at, press: pos, pointer: pos, decimals, fine: false, moved: false, live: true, want: None, outcome: None, solve_ms: 0.0 };
                        self.drag = Drag::Goal(Box::new(g));
                        return;
                    }
                }
            }
        }
        // 3. dragging the selection's border moves it (Alt: copies); scrubbing is Alt-drag inside a cell
        if !dbl && self.on_sel_border(g, pos, mods) {
            let s = self.sel();
            self.drag = Drag::Move { src: s, grab: (r, c), dst: s, copy: mods.alt };
            return;
        }
        // 4. alt-drag scrubs a literal number
        if mods.alt && self.edit.is_none() {
            let k = self.eng.wb.sheets[ix].key(r, c).unwrap();
            let text = self.eng.wb.cell_text(k);
            if let Some(lit) = ops::cell_literal(&text) {
                self.select(r, c, false);
                self.drag = Drag::Scrub { key: k, orig: self.eng.wb.cell(k).cloned(), text, lit, x0: pos.x };
                return;
            }
        }
        // 5. fill handle: drag to fill; double-click fills down as far as the column beside goes
        if self.edit.is_none() && self.fill_handle(g).contains(pos) {
            let s = self.sel();
            if dbl {
                if let Some(dst) = ops::fill_down_extent(&self.eng, s) {
                    let e = ops::fill(&mut self.eng, s, dst);
                    self.exec(e);
                    self.anchor = (dst.r0, dst.c0);
                    self.cursor = (dst.r1, dst.c1);
                }
                return;
            }
            self.drag = Drag::Fill { src: s, dst: s };
            return;
        }
        // 6. plain selection
        self.commit();
        if dbl {
            self.select(r, c, false);
            self.start_edit(ctx, r, c, None, false);
            return;
        }
        self.select(r, c, mods.shift);
        self.offer = self.offer.filter(|(k, _)| self.eng.wb.pos(*k) == Some((r, c)));
        self.drag = Drag::Select;
    }

    fn drag_update(&mut self, ctx: &egui::Context, g: &Geo, pos: Pos2, mods: Modifiers) {
        if let Drag::Goal(_) = self.drag {
            let Drag::Goal(mut gd) = std::mem::replace(&mut self.drag, Drag::None) else { unreachable!() };
            self.goal_update(ctx, &mut gd, pos, mods);
            self.drag = Drag::Goal(gd);
            return;
        }
        let autoscroll = matches!(self.drag, Drag::Select | Drag::Ref { .. } | Drag::Fill { .. } | Drag::Move { .. });
        if autoscroll {
            let c = g.cells;
            let mut d = Vec2::ZERO;
            if pos.y > c.bottom() {
                d.y = (pos.y - c.bottom()).min(60.0) * 0.5;
            } else if pos.y < c.top() {
                d.y = (pos.y - c.top()).max(-60.0) * 0.5;
            }
            if pos.x > c.right() {
                d.x = (pos.x - c.right()).min(60.0) * 0.5;
            } else if pos.x < c.left() {
                d.x = (pos.x - c.left()).max(-60.0) * 0.5;
            }
            if d != Vec2::ZERO {
                self.scroll = (self.scroll + d).max(Vec2::ZERO);
                ctx.request_repaint();
            }
        }
        let cp = Pos2::new(pos.x.clamp(g.cells.left() + 1.0, g.cells.right() - 1.0), pos.y.clamp(g.cells.top() + 1.0, g.cells.bottom() - 1.0));
        let (r, c) = (g.row_at(cp.y), g.col_at(cp.x));
        let ix = self.sheet_ix;
        match &mut self.drag {
            Drag::None => {}
            Drag::Select => {
                self.cursor = (r, c);
            }
            Drag::Ref { start } => {
                let start = *start;
                self.insert_ref(ctx, start, (r, c));
            }
            Drag::Fill { src, dst } => {
                let s = *src;
                let down = r as i64 - s.r1 as i64;
                let up = s.r0 as i64 - r as i64;
                let right = c as i64 - s.c1 as i64;
                let left = s.c0 as i64 - c as i64;
                let v = down.max(up);
                let h = right.max(left);
                *dst = if v <= 0 && h <= 0 {
                    s
                } else if v >= h {
                    if down > 0 { CRect { r1: r, ..s } } else { CRect { r0: r, ..s } }
                } else if right > 0 {
                    CRect { c1: c, ..s }
                } else {
                    CRect { c0: c, ..s }
                };
                ctx.set_cursor_icon(CursorIcon::Crosshair);
            }
            Drag::Move { src, grab, dst, copy } => {
                let r0 = (src.r0 + r).saturating_sub(grab.0);
                let c0 = (src.c0 + c).saturating_sub(grab.1);
                *dst = CRect { r0, c0, r1: r0 + src.rows() - 1, c1: c0 + src.cols() - 1, ..*src };
                *copy = mods.alt;
                ctx.set_cursor_icon(if mods.alt { CursorIcon::Copy } else { CursorIcon::Grabbing });
            }
            Drag::Col { col, x0, w0 } => {
                let w = (w0.unwrap_or(DEF_W) + pos.x - *x0).max(24.0);
                let cid = self.eng.wb.sheets[ix].cols.get(*col).unwrap();
                self.eng.wb.sheets[ix].col_widths.insert(cid, w);
                ctx.set_cursor_icon(CursorIcon::ResizeColumn);
            }
            Drag::Row { row, y0, h0 } => {
                let h = (h0.unwrap_or(DEF_H) + pos.y - *y0).max(14.0);
                let rid = self.eng.wb.sheets[ix].rows.get(*row).unwrap();
                self.eng.wb.sheets[ix].row_heights.insert(rid, h);
                ctx.set_cursor_icon(CursorIcon::ResizeRow);
            }
            Drag::Scrub { key, text, lit, x0, .. } => {
                let fast = if mods.shift { 10.0 } else { 1.0 };
                let steps = ((pos.x - *x0) / 4.0).round() as f64 * fast;
                let new = ops::replace_span(text, &lit.span, &ops::scrub(lit, steps));
                let key = *key;
                ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
                if new != self.eng.wb.cell_text(key) {
                    let t = std::time::Instant::now();
                    self.eng.set_text(key, &new);
                    self.last_recalc_ms = t.elapsed().as_secs_f64() * 1000.0;
                }
            }
            Drag::Goal(_) => {}
            Drag::Point { y, x, axis, press } => {
                // Alt locks a 2D drag to the axis moved along most since the press (neither, until one leads);
                // the other cell stays exactly as it was
                let (mut move_x, mut move_y) = (x.is_some(), y.is_some());
                let locked = mods.alt && move_x && move_y;
                if locked {
                    let d = pos - *press;
                    (move_x, move_y) = (d.x.abs() > d.y.abs(), d.y.abs() > d.x.abs());
                    let to = if move_x { "x" } else if move_y { "y" } else { "one axis" };
                    chart_view::tooltip_lines(ctx, pos, &[Tip::Note(format!("locked to {to} — release Alt to move freely"))]);
                }
                ctx.set_cursor_icon(match (move_x, move_y) {
                    (true, true) => CursorIcon::Move,
                    (true, false) => CursorIcon::ResizeHorizontal,
                    (false, true) => CursorIcon::ResizeVertical,
                    (false, false) => CursorIcon::Move,
                });
                let mut writes = Vec::new();
                if let Some(c) = y {
                    let new = if move_y { point_text(c, &axis.disp, axis.from_screen(pos.y), axis.y1 - axis.y0, mods.shift) } else { c.text.clone() };
                    writes.push((c, new));
                }
                if let (Some(c), Some(xdisp)) = (x, &axis.xdisp) {
                    let new = if move_x { point_text(c, xdisp, axis.x_from_screen(pos.x), axis.x1 - axis.x0, mods.shift) } else { c.text.clone() };
                    writes.push((c, new));
                }
                writes.retain(|(c, new)| *new != self.eng.wb.cell_text(c.key));
                if !writes.is_empty() {
                    // both cells of a 2D drag in one edit: one recalc; a cell back at its press text gets its original cell back
                    let cells = writes
                        .into_iter()
                        .map(|(c, new)| (c.key, if new == c.text { c.orig.clone() } else { Some(Cell::new(self.eng.wb.parse_text(&new, c.key.sheet))) }))
                        .collect();
                    let t = std::time::Instant::now();
                    self.eng.apply(Edit::Cells(cells));
                    self.last_recalc_ms = t.elapsed().as_secs_f64() * 1000.0;
                }
            }
        }
    }

    fn drag_end(&mut self) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::Fill { src, dst } => {
                if src != dst {
                    let e = ops::fill(&mut self.eng, src, dst);
                    self.exec(e);
                    self.anchor = (dst.r0, dst.c0);
                    self.cursor = (dst.r1, dst.c1);
                }
            }
            // a press on the border that didn't go anywhere is a click on the cell under it
            Drag::Move { src, grab, dst, .. } if src == dst => self.select(grab.0, grab.1, false),
            Drag::Move { src, dst, copy: true, .. } => {
                let clip = ops::copy(&self.eng, src);
                let e = ops::paste(&mut self.eng, &clip, dst.sheet, (dst.r0, dst.c0));
                self.exec(e);
                self.anchor = (dst.r0, dst.c0);
                self.cursor = (dst.r1, dst.c1);
            }
            Drag::Move { src, dst, .. } => {
                self.move_block(src, (dst.r0, dst.c0));
            }
            Drag::Goal(g) => self.goal_end(*g),
            Drag::Scrub { key, orig, .. } => {
                if self.eng.wb.cell(key) != orig.as_ref() {
                    self.undo.push(Edit::Cells(vec![(key, orig)]));
                    self.redo.clear();
                }
            }
            // a diagonal drag that wrote two cells is one undo step
            Drag::Point { y, x, .. } => {
                let undo: Vec<_> = y.into_iter().chain(x).filter(|c| self.eng.wb.cell(c.key) != c.orig.as_ref()).map(|c| (c.key, c.orig)).collect();
                if !undo.is_empty() {
                    self.undo.push(Edit::Cells(undo));
                    self.redo.clear();
                }
            }
            // the drag resized live; the whole drag is one undo step
            Drag::Col { col, w0, .. } => {
                self.dirty_stale = true;
                let s = self.sheet();
                let (sheet, cid) = (s.id, s.cols.get(col).unwrap());
                if s.col_widths.get(&cid).copied() != w0 {
                    self.undo.push(Edit::ColWidth { sheet, col: cid, width: w0 });
                    self.redo.clear();
                }
            }
            Drag::Row { row, h0, .. } => {
                self.dirty_stale = true;
                let s = self.sheet();
                let (sheet, rid) = (s.id, s.rows.get(row).unwrap());
                if s.row_heights.get(&rid).copied() != h0 {
                    self.undo.push(Edit::RowHeight { sheet, row: rid, height: h0 });
                    self.redo.clear();
                }
            }
            _ => {}
        }
    }

    fn autofit(&mut self, ctx: &egui::Context, c: usize) {
        let s = self.sheet();
        let font = FontId::proportional(FONT);
        let pal = Pal::new(false);
        let mut texts = Vec::new();
        for r in 0..s.used_extent().0 {
            if let Some(k) = s.key(r, c) {
                if let Some((t, ..)) = self.cell_display(k, &pal) {
                    texts.push(t);
                }
            }
        }
        let w = texts
            .iter()
            .map(|t| ctx.fonts_mut(|f| f.layout_no_wrap(t.clone(), font.clone(), Color32::WHITE).size().x))
            .fold(0.0f32, f32::max);
        let (sheet, cid) = (self.sid(), self.sheet().cols.get(c).unwrap());
        let width = Some((w + 18.0).clamp(40.0, 600.0));
        if self.sheet().col_widths.get(&cid).copied() != width {
            let inv = self.eng.apply(Edit::ColWidth { sheet, col: cid, width });
            self.undo.push(inv);
            self.redo.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(&mut self, ui: &mut Ui, ctx: &egui::Context, pal: &Pal, g: &Geo, area: Rect, dark: bool, sid: SheetId) {
        let cur = self.sheet().key(self.cursor.0, self.cursor.1);
        let trace = cur.filter(|_| self.trace && self.edit.is_none()).map(|k| self.trace_sets(k));
        let painter = ui.painter_at(area);
        painter.rect_filled(g.cells, 0.0, pal.bg);
        let cp = painter.with_clip_rect(g.cells);
        let (rows, cols) = g.visible();
        let font = FontId::proportional(FONT);
        let s = self.sheet();
        let sel = self.sel();

        // input tint
        let inputs: HashSet<CellKey> = self.eng.wb.names.values().filter(|d| d.input).map(|d| d.cell).collect();
        // grid lines
        for c in cols.clone() {
            let x = g.x(c + 1) - 0.5;
            cp.line_segment([Pos2::new(x, g.cells.top()), Pos2::new(x, g.cells.bottom())], Stroke::new(1.0, pal.grid));
        }
        for r in rows.clone() {
            let y = g.y(r + 1) - 0.5;
            cp.line_segment([Pos2::new(g.cells.left(), y), Pos2::new(g.cells.right(), y)], Stroke::new(1.0, pal.grid));
        }
        let is_blank = |r: usize, c: usize| match s.key(r, c) {
            Some(k) => matches!(self.eng.shown(k), Shown::Empty),
            None => true,
        };
        // cells
        let mut anchors: HashSet<CellKey> = HashSet::new();
        let mut blockers: Vec<CellKey> = Vec::new();
        let hover = ctx.pointer_hover_pos().filter(|_| ui.rect_contains_pointer(g.cells) && matches!(self.drag, Drag::None));
        let mut too_wide: Option<String> = None;
        for r in rows.clone() {
            // right edge of the cells already claimed by text overflowing in this row
            let mut claimed = f32::NEG_INFINITY;
            for c in cols.clone() {
                let Some(k) = s.key(r, c) else { continue };
                let rect = g.cell(r, c);
                if inputs.contains(&k) {
                    cp.rect_filled(rect.shrink2(Vec2::new(0.0, 0.5)).translate(Vec2::new(-0.5, -0.5)), 0.0, pal.input_fill);
                }
                if let Some(a) = self.eng.spill_anchor(k) {
                    anchors.insert(a);
                }
                if self.eng.spill_size(k).is_some() {
                    anchors.insert(k);
                }
                if let Shown::Error(e) = self.eng.shown(k) {
                    if let ErrKind::SpillBlocked(b) = e.kind {
                        blockers.push(b);
                    }
                }
                if let Some((mut text, color, right, shorter)) = self.cell_display(k, pal) {
                    let mut clip = rect;
                    let width = |t: &str| ctx.fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), color).size().x) + 10.0;
                    let needed = width(&text);
                    if !right {
                        // text overflows into empty cells to its right
                        let mut c2 = c + 1;
                        while clip.width() < needed && c2 < g.col_x.len() - 1 && is_blank(r, c2) && c2 < cols.end + 8 {
                            clip.max.x = g.x(c2 + 1);
                            c2 += 1;
                        }
                        if clip.max.x > rect.max.x {
                            let cover = Rect::from_min_max(Pos2::new(rect.right(), rect.top()), Pos2::new(clip.right() - 1.0, rect.bottom() - 1.0));
                            cp.rect_filled(cover, 0.0, pal.bg);
                        }
                    } else if clip.width() < needed {
                        // a number overflows into empty cells to its left; if it still doesn't fit
                        // show ### rather than a clipped number that reads as a different one
                        let mut c2 = c;
                        while clip.width() < needed && c2 > 0 && c2 + 8 > c && is_blank(r, c2 - 1) && g.x(c2 - 1) >= claimed {
                            c2 -= 1;
                            clip.min.x = g.x(c2);
                        }
                        if clip.width() < needed {
                            // still too wide: fewer decimals, then scientific, before giving up
                            let full = text.clone();
                            if let Some(t) = shorter.into_iter().find(|t| width(t) <= rect.width()) {
                                text = t;
                            } else {
                                text = "###".to_string();
                            }
                            clip = rect;
                            if hover.is_some_and(|p| rect.contains(p)) {
                                too_wide = Some(full);
                            }
                        } else if clip.min.x < rect.min.x {
                            let cover = Rect::from_min_max(Pos2::new(clip.left(), rect.top()), Pos2::new(rect.left(), rect.bottom() - 1.0));
                            cp.rect_filled(cover, 0.0, pal.bg);
                        }
                    }
                    claimed = claimed.max(clip.right());
                    let tp = cp.with_clip_rect(clip.shrink2(Vec2::new(3.0, 0.0)).intersect(g.cells));
                    let (pos, align) = if right {
                        (Pos2::new(rect.right() - 5.0, rect.center().y), Align2::RIGHT_CENTER)
                    } else {
                        (Pos2::new(rect.left() + 5.0, rect.center().y), Align2::LEFT_CENTER)
                    };
                    tp.text(pos, align, text, font.clone(), color);
                }
            }
        }
        if let (Some(t), Some(p)) = (too_wide, hover) {
            chart_view::tooltip(ctx, p, &t);
        }
        // spills and charts
        let mut chart_hits = Vec::new();
        for a in anchors {
            let Some((r0, c0)) = self.eng.wb.pos(a) else { continue };
            let Some((nr, nc)) = self.eng.spill_size(a) else { continue };
            let rect = g.rect(r0, c0, r0 + nr - 1, c0 + nc - 1);
            if let Some(Ok(Value::Chart(ch))) = self.eng.result(a) {
                // the chart under a point drag keeps its ranges
                let held = match &self.drag {
                    Drag::Goal(g) => Some(&g.axis),
                    Drag::Point { axis, .. } => Some(axis),
                    _ => None,
                };
                let fixed = held.filter(|ax| ax.anchor == Some(a)).map(|ax| ((ax.y0, ax.y1), (ax.x0, ax.x1)));
                let (mut axis, hits) = chart_view::draw(&cp, rect, ch, dark, fixed);
                axis.anchor = Some(a);
                for h in hits {
                    chart_hits.push((h, axis.clone()));
                }
            } else {
                dashed_rect(&cp, rect.shrink(1.0), Stroke::new(1.0, pal.spill));
            }
        }
        for b in blockers {
            if let Some((r, c)) = self.eng.wb.pos(b) {
                if b.sheet == sid {
                    let rect = g.cell(r, c);
                    cp.rect_filled(rect, 0.0, pal.err.gamma_multiply(0.12));
                    dashed_rect(&cp, rect.shrink(1.0), Stroke::new(1.5, pal.err));
                }
            }
        }
        // cycles
        for cyc in &self.eng.cycles {
            for k in cyc {
                if k.sheet == sid {
                    if let Some((r, c)) = self.eng.wb.pos(*k) {
                        cp.rect_filled(g.cell(r, c), 0.0, pal.err.gamma_multiply(0.15));
                        cp.rect_stroke(g.cell(r, c).shrink(1.0), 0.0, Stroke::new(1.5, pal.err), StrokeKind::Inside);
                    }
                }
            }
        }
        // trace (only what's on screen: a cell can have thousands of dependents)
        if let Some(t) = &trace {
            let pre_c = Color32::from_rgb(0x3b, 0x82, 0xf6);
            let dep_c = Color32::from_rgb(0xf9, 0x73, 0x16);
            for (cells, color, edge) in [(&t.pre, pre_c, 0.7), (&t.dep, dep_c, 0.8)] {
                for (k, at) in cells {
                    let Some((r, c)) = *at else { continue };
                    if k.sheet != sid || !rows.contains(&r) || !cols.contains(&c) {
                        continue;
                    }
                    cp.rect_filled(g.cell(r, c), 0.0, color.gamma_multiply(0.10));
                    cp.rect_stroke(g.cell(r, c).shrink(1.0), 0.0, Stroke::new(1.0, color.gamma_multiply(edge)), StrokeKind::Inside);
                }
            }
        }
        // references in the formula being edited
        if let Some(ed) = &self.edit {
            for h in syntax::analyze(&ed.text, &self.eng.wb, ed.key.sheet) {
                if h.sheet != sid {
                    continue;
                }
                let rect = g.rect(h.r0, h.c0, h.r1, h.c1);
                cp.rect_filled(rect, 0.0, h.color.gamma_multiply(0.12));
                cp.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, h.color), StrokeKind::Inside);
            }
        }
        // selection
        let srect = g.rect(sel.r0, sel.c0, sel.r1, sel.c1);
        if sel.rows() > 1 || sel.cols() > 1 {
            cp.rect_filled(srect, 0.0, pal.sel_fill);
        }
        cp.rect_stroke(srect, 0.0, Stroke::new(1.5, pal.sel), StrokeKind::Inside);
        cp.rect_stroke(g.cell(self.cursor.0, self.cursor.1), 0.0, Stroke::new(2.0, pal.sel), StrokeKind::Inside);
        if self.edit.is_none() {
            cp.rect_filled(self.fill_handle(g).shrink(1.5), 1.0, pal.sel);
            cp.rect_stroke(self.fill_handle(g).shrink(1.5), 1.0, Stroke::new(1.0, pal.bg), StrokeKind::Outside);
        }
        if let Drag::Fill { dst, .. } = &self.drag {
            dashed_rect(&cp, g.rect(dst.r0, dst.c0, dst.r1, dst.c1), Stroke::new(1.5, pal.sel));
        }
        // where a border drag will drop, and the cells waiting for a paste to move them
        if let Drag::Move { src, dst, .. } = &self.drag {
            if src != dst {
                let r = g.rect(dst.r0, dst.c0, dst.r1, dst.c1);
                cp.rect_filled(r, 0.0, pal.sel_fill);
                dashed_rect(&cp, r.shrink(1.0), Stroke::new(2.0, pal.sel));
            }
        }
        if let Some((cut, _)) = self.cut.filter(|(c, _)| c.sheet == sid) {
            dashed_rect(&cp, g.rect(cut.r0, cut.c0, cut.r1, cut.c1).shrink(3.5), Stroke::new(1.5, pal.sel));
        }

        // headers
        let hp = painter.with_clip_rect(Rect::from_min_max(Pos2::new(g.cells.left(), area.top()), Pos2::new(area.right(), g.cells.top())));
        let small = FontId::proportional(11.5);
        for c in cols.clone() {
            let rect = Rect::from_min_max(Pos2::new(g.x(c), area.top()), Pos2::new(g.x(c + 1), g.cells.top()));
            let on = c >= sel.c0 && c <= sel.c1;
            hp.rect_filled(rect, 0.0, if on { pal.hdr_sel } else { pal.hdr });
            hp.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, pal.grid));
            hp.text(rect.center(), Align2::CENTER_CENTER, a1::col_name(c), small.clone(), if on { pal.sel } else { pal.muted });
        }
        let vp = painter.with_clip_rect(Rect::from_min_max(Pos2::new(area.left(), g.cells.top()), Pos2::new(g.cells.left(), area.bottom())));
        for r in rows.clone() {
            let rect = Rect::from_min_max(Pos2::new(area.left(), g.y(r)), Pos2::new(g.cells.left(), g.y(r + 1)));
            let on = r >= sel.r0 && r <= sel.r1;
            vp.rect_filled(rect, 0.0, if on { pal.hdr_sel } else { pal.hdr });
            vp.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, pal.grid));
            vp.text(rect.center(), Align2::CENTER_CENTER, (r + 1).to_string(), small.clone(), if on { pal.sel } else { pal.muted });
        }
        painter.rect_filled(Rect::from_min_max(area.min, g.cells.min), 0.0, pal.hdr);
        painter.line_segment([Pos2::new(area.left(), g.cells.top()), Pos2::new(area.right(), g.cells.top())], Stroke::new(1.0, pal.grid));
        painter.line_segment([Pos2::new(g.cells.left(), area.top()), Pos2::new(g.cells.left(), area.bottom())], Stroke::new(1.0, pal.grid));
        self.chart_hits = chart_hits;
        self.paint_goal_note(&cp, pal);
        self.paint_goal(ctx, &cp, pal);
    }

    fn cell_editor(&mut self, ui: &mut Ui, g: &Geo, pal: &Pal, sid: SheetId) {
        self.editor_rect = None;
        let Some(ed) = &self.edit else { return };
        if ed.key.sheet != sid {
            return;
        }
        let Some((r, c)) = self.eng.wb.pos(ed.key) else { return };
        let cell = g.cell(r, c);
        if !g.cells.intersects(cell) {
            return;
        }
        let text_w = ui.fonts_mut(|f| f.layout_no_wrap(ed.text.clone(), FontId::proportional(FONT), Color32::WHITE).size().x);
        let w = (text_w + 24.0).max(cell.width()).min((g.cells.right() - cell.left()).max(cell.width()));
        let rect = Rect::from_min_size(cell.min, Vec2::new(w, cell.height()));
        ui.painter().rect_filled(rect, 0.0, pal.bg);
        if matches!(self.eng.shown(ed.key), Shown::Error(_)) {
            // the cell shows an error: tint so it's clear what's being edited
            ui.painter().rect_filled(rect, 0.0, pal.err.gamma_multiply(0.10));
        }
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(2.0, pal.sel), StrokeKind::Inside);
        self.editor_rect = Some(rect);
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(Vec2::new(5.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let out = self.editor(&mut child, Id::new("cell_editor"), rect.width() - 10.0, false);
        self.editor_keys(ui, &out);
    }

    fn offer_banner(&mut self, ui: &mut Ui, g: &Geo, sid: SheetId) {
        let Some((k, dst)) = self.offer else { return };
        if k.sheet != sid || self.edit.is_some() {
            return;
        }
        let Some((r, c)) = self.eng.wb.pos(k) else { return };
        let cell = g.cell(r, c);
        if !g.cells.contains(cell.left_bottom()) {
            return;
        }
        let label = format!("↓ extend to {}:{}  ⌘E", a1::cell_name(dst.r0, dst.c0), a1::cell_name(dst.r1, dst.c1));
        egui::Area::new(Id::new("offer"))
            .fixed_pos(cell.right_top() + Vec2::new(10.0, -2.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button(label).clicked() {
                            self.accept_offer();
                        }
                        if ui.small_button("dismiss").clicked() {
                            self.offer = None;
                        }
                    });
                });
            });
    }

    fn context_menu(&mut self, resp: &egui::Response) {
        resp.context_menu(|ui| {
            let sel = self.sel();
            let sid = sel.sheet;
            let col = a1::col_name(self.cursor.1);
            if ui.button(format!("Insert {} row(s) above", sel.rows())).clicked() {
                self.exec(self.eng.insert_rows_edit(sid, sel.r0, sel.rows()));
                ui.close();
            }
            if ui.button(format!("Insert {} row(s) below", sel.rows())).clicked() {
                self.exec(self.eng.insert_rows_edit(sid, sel.r1 + 1, sel.rows()));
                ui.close();
            }
            if ui.button(format!("Delete row(s) {}–{}", sel.r0 + 1, sel.r1 + 1)).clicked() {
                self.exec(self.eng.delete_rows_edit(sid, sel.r0, sel.rows()));
                ui.close();
            }
            ui.separator();
            if ui.button(format!("Insert {} column(s) left", sel.cols())).clicked() {
                self.exec(self.eng.insert_cols_edit(sid, sel.c0, sel.cols()));
                ui.close();
            }
            if ui.button(format!("Insert {} column(s) right", sel.cols())).clicked() {
                self.exec(self.eng.insert_cols_edit(sid, sel.c1 + 1, sel.cols()));
                ui.close();
            }
            if ui.button(format!("Delete column(s) {}–{}", a1::col_name(sel.c0), a1::col_name(sel.c1))).clicked() {
                self.exec(self.eng.delete_cols_edit(sid, sel.c0, sel.cols()));
                ui.close();
            }
            ui.separator();
            let rows = if sel.rows() > 1 { sel } else { CRect { r0: 0, r1: self.sheet().used_extent().0.max(1) - 1, ..sel } };
            if ui.button(format!("Sort rows {}–{} by {col} ↑", rows.r0 + 1, rows.r1 + 1)).clicked() {
                let e = ops::sort_rows(&self.eng, rows, self.cursor.1, true);
                self.exec(e);
                ui.close();
            }
            if ui.button(format!("Sort rows {}–{} by {col} ↓", rows.r0 + 1, rows.r1 + 1)).clicked() {
                let e = ops::sort_rows(&self.eng, rows, self.cursor.1, false);
                self.exec(e);
                ui.close();
            }
            ui.separator();
            if ui.add_enabled(sel.rows() > 1, egui::Button::new("Fill down  ⌘D")).clicked() {
                self.fill_down();
                ui.close();
            }
        });
    }
}

fn dashed_rect(p: &Painter, r: Rect, stroke: Stroke) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(egui::Shape::dashed_line(&pts, stroke, 4.0, 3.0));
}

/// A dragged point's literal cell's new text for the value `shown` in the axis' unit `disp`, the axis spanning `span`:
/// in the cell's own unit and with its own decimals (a date in whole days); `fine` (Shift) about 1/200 of the axis.
fn point_text(c: &PointCell, disp: &wbs_core::units::DispUnit, shown: f64, span: f64, fine: bool) -> String {
    let v = c.disp.to_display(disp.to_canonical(shown));
    let decimals = if fine {
        let step = span.abs() / 200.0 / c.disp.factor.abs().max(1e-300) * disp.factor.abs();
        let dec = if step > 0.0 { (-step.log10()).ceil().max(0.0) as usize } else { 0 };
        dec.max(c.lit.decimals).min(10)
    } else {
        c.lit.decimals
    };
    ops::replace_span(&c.text, &c.lit.span, &ops::format_lit(v, decimals, c.lit.is_date))
}
