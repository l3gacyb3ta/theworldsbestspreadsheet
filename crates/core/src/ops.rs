//! Editing operations built on the engine: fill, copy/paste, sort, scrub
//! helpers, formula extension. Each returns an `Edit` to apply (and undo).

use crate::engine::{Edit, Engine, Shown};
use crate::ids::*;
use crate::lex::{self, Tok};
use crate::model::{classify, number_literal, Cell, Kind, Piece, StoredRef, Workbook, MAX_INDEX};
use crate::units::Quant;
use crate::value::{fmt_date, fmt_quantity, Value};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::ops::Range;

/// Inclusive cell rectangle on one sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub sheet: SheetId,
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

impl Rect {
    pub fn cell(sheet: SheetId, r: usize, c: usize) -> Rect {
        Rect { sheet, r0: r, c0: c, r1: r, c1: c }
    }
    pub fn span(sheet: SheetId, a: (usize, usize), b: (usize, usize)) -> Rect {
        Rect { sheet, r0: a.0.min(b.0), c0: a.1.min(b.1), r1: a.0.max(b.0), c1: a.1.max(b.1) }
    }
    pub fn rows(&self) -> usize {
        self.r1 - self.r0 + 1
    }
    pub fn cols(&self) -> usize {
        self.c1 - self.c0 + 1
    }
    pub fn contains(&self, r: usize, c: usize) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }
}

/// A number literal inside a cell's text, for scrubbing and series.
#[derive(Clone, Debug, PartialEq)]
pub struct Lit {
    pub value: f64,
    pub span: Range<usize>,
    pub decimals: usize,
    pub is_date: bool,
}

fn decimals_of(s: &str) -> usize {
    if s.contains(['e', 'E']) {
        return 0;
    }
    s.split_once('.').map(|(_, d)| d.len()).unwrap_or(0)
}

/// The literal of a number cell (`5`, `0.25 [m]`, `2026-01-31`).
pub fn cell_literal(text: &str) -> Option<Lit> {
    if classify(text) != Kind::Number {
        return None;
    }
    let lead = text.len() - text.trim_start().len();
    let t = text.trim();
    let (span, _) = number_literal(t)?;
    let span = span.start + lead..span.end + lead;
    let s = &text[span.clone()];
    if let Some(d) = lex::parse_date(s) {
        return Some(Lit { value: d as f64, span, decimals: 0, is_date: true });
    }
    let value = lex::parse_number(s)?;
    Some(Lit { value, span, decimals: decimals_of(s), is_date: false })
}

/// Every number literal in a program's text, for scrubbing in the formula bar.
pub fn program_literals(text: &str) -> Vec<Lit> {
    lex::lex(text)
        .into_iter()
        .filter_map(|t| match t.tok {
            Tok::Num(v) => Some(Lit { value: v, decimals: decimals_of(&text[t.span.clone()]), span: t.span, is_date: false }),
            _ => None,
        })
        .collect()
}

pub fn format_lit(v: f64, decimals: usize, is_date: bool) -> String {
    if is_date {
        return fmt_date(v.round());
    }
    let s = format!("{:.*}", decimals, v);
    if s.starts_with("-") && s.trim_start_matches(['-', '0', '.']).is_empty() {
        return s[1..].to_string();
    }
    s
}

pub fn replace_span(text: &str, span: &Range<usize>, with: &str) -> String {
    format!("{}{}{}", &text[..span.start], with, &text[span.end..])
}

/// One step of scrubbing: `steps` ticks of the literal's last digit.
pub fn scrub(lit: &Lit, steps: f64) -> String {
    let unit = if lit.is_date { 1.0 } else { 10f64.powi(-(lit.decimals as i32)) };
    let v = lit.value + steps * unit;
    format_lit(v, lit.decimals, lit.is_date)
}

fn cell_of(e: &Engine, k: CellKey) -> Option<Cell> {
    e.wb.cell(k).cloned()
}

/// Fill `dst` (which contains `src` and extends it in one direction).
/// One rule: one source cell is copied; two or more number cells in an
/// arithmetic progression continue the series; otherwise the pattern repeats.
/// Formulas are copied with relative references adjusted.
pub fn fill(e: &mut Engine, src: Rect, dst: Rect) -> Edit {
    let vertical = dst.r0 != src.r0 || dst.r1 != src.r1;
    let mut out = Vec::new();
    let lanes: Vec<usize> = if vertical { (src.c0..=src.c1).collect() } else { (src.r0..=src.r1).collect() };
    for lane in lanes {
        let src_pos: Vec<(usize, usize)> = if vertical {
            (src.r0..=src.r1).map(|r| (r, lane)).collect()
        } else {
            (src.c0..=src.c1).map(|c| (lane, c)).collect()
        };
        let Some(keys) = src_pos.iter().map(|(r, c)| e.wb.sheet(src.sheet).unwrap().key(*r, *c)).collect::<Option<Vec<CellKey>>>() else { continue };
        let texts: Vec<String> = keys.iter().map(|k| e.wb.cell_text(*k)).collect();
        let series = series_of(&texts);
        let n = keys.len() as i64;
        let targets: Vec<(usize, usize)> = if vertical {
            (dst.r0..=dst.r1).filter(|r| !(src.r0..=src.r1).contains(r)).map(|r| (r, lane)).collect()
        } else {
            (dst.c0..=dst.c1).filter(|c| !(src.c0..=src.c1).contains(c)).map(|c| (lane, c)).collect()
        };
        for (tr, tc) in targets {
            let i = if vertical { tr as i64 - src.r0 as i64 } else { tc as i64 - src.c0 as i64 };
            let Some(tk) = e.wb.sheet(dst.sheet).unwrap().key(tr, tc) else { continue };
            if let Some((first, step, ref tmpl, ref lit)) = series {
                let v = first + step * i as f64;
                let text = replace_span(tmpl, &lit.span, &format_lit(v, lit.decimals, lit.is_date));
                out.push((tk, Some(Cell::new(vec![Piece::Text(text)]))));
                continue;
            }
            let j = i.rem_euclid(n) as usize;
            let sk = keys[j];
            let cell = cell_of(e, sk).map(|c| {
                if classify(&texts[j]).has_refs() {
                    Cell::new(e.wb.shift_pieces(&c.pieces, sk, tk))
                } else {
                    c
                }
            });
            out.push((tk, cell));
        }
    }
    Edit::Cells(out)
}

/// (first value, step, template text, template literal) if `texts` is an
/// arithmetic progression of ≥2 number literals sharing a unit.
fn series_of(texts: &[String]) -> Option<(f64, f64, String, Lit)> {
    if texts.len() < 2 {
        return None;
    }
    let lits: Vec<Lit> = texts.iter().map(|t| cell_literal(t)).collect::<Option<_>>()?;
    let suffix = |t: &str, l: &Lit| t[l.span.end..].trim().to_string();
    let s0 = suffix(&texts[0], &lits[0]);
    if lits.iter().zip(texts).any(|(l, t)| suffix(t, l) != s0 || l.is_date != lits[0].is_date) {
        return None;
    }
    let step = lits[1].value - lits[0].value;
    let tol = 1e-9 * step.abs().max(1.0);
    for w in lits.windows(2) {
        if ((w[1].value - w[0].value) - step).abs() > tol {
            return None;
        }
    }
    let decimals = lits.iter().map(|l| l.decimals).max().unwrap_or(0);
    let mut tmpl = lits[0].clone();
    tmpl.decimals = decimals;
    Some((lits[0].value, step, texts[0].clone(), tmpl))
}

/// A copied block: cells with the key they were copied from.
#[derive(Clone, Debug)]
pub struct Clip {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Option<(CellKey, Cell)>>,
    pub text: String,
}

pub fn copy(e: &Engine, r: Rect) -> Clip {
    let s = e.wb.sheet(r.sheet).unwrap();
    let mut cells = Vec::new();
    let mut lines = Vec::new();
    for row in r.r0..=r.r1 {
        let mut line = Vec::new();
        for col in r.c0..=r.c1 {
            let k = s.key(row, col);
            let c = k.and_then(|k| s.cell(k.row, k.col).map(|c| (k, c.clone())));
            line.push(k.map(|k| e.wb.cell_text(k)).unwrap_or_default());
            cells.push(c);
        }
        lines.push(line.join("\t"));
    }
    Clip { rows: r.rows(), cols: r.cols(), cells, text: lines.join("\n") }
}

/// ⇧⌘C: the values the cells show, tab/newline separated, each written so that typing (or pasting) it
/// back gives the same value: `5300 [m]`, `50 [%]`, `20 [°C]`, `2026-01-31`, text. See `value_literal`.
pub fn copy_values(e: &Engine, r: Rect) -> String {
    let s = e.wb.sheet(r.sheet).unwrap();
    let lines: Vec<String> = (r.r0..=r.r1)
        .map(|row| (r.c0..=r.c1).map(|col| s.key(row, col).map(|k| value_literal(e, k)).unwrap_or_default()).collect::<Vec<_>>().join("\t"))
        .collect();
    lines.join("\n")
}

/// The value a cell shows as a re-typeable literal. A spilled cell gives its element; a number its full
/// precision in its display unit; text is quoted with `'` when it would otherwise read as a number,
/// program or declaration (tabs and newlines become spaces). An error gives its short code (`#err`),
/// which pastes back as text; charts, unit/dim/word declarations and empty cells give nothing.
pub fn value_literal(e: &Engine, k: CellKey) -> String {
    let (value, dr, dc) = match e.shown(k) {
        Shown::Empty => return String::new(),
        Shown::Error(err) => return err.short().to_string(),
        Shown::Value { value, dr, dc, .. } => (value, dr, dc),
    };
    let ix = |shape: &[usize]| match shape {
        [] => Some(0),
        [_] => Some(dr),
        [_, c] => Some(dr * c + dc),
        _ => None,
    };
    match value {
        Value::Num(n) => ix(&n.shape).and_then(|i| n.data.get(i)).map(|x| quantity_literal(*x, &n.q)).unwrap_or_default(),
        Value::Text(t) => match ix(&t.shape).and_then(|i| t.data.get(i)) {
            Some(s) => {
                let s = s.replace(['\t', '\n', '\r'], " ");
                if classify(&s) == Kind::Text && !s.starts_with(['\'', ' ']) {
                    s
                } else {
                    format!("'{s}")
                }
            }
            None => String::new(),
        },
        Value::Chart(_) | Value::Unit(_) | Value::Dim(_) | Value::Word(_) => String::new(),
    }
}

/// `5300 [m]` for 5300 m: the number in its display unit, rounded to 15 significant digits (so `0.1 0.2 +`
/// copies as `0.3`, and anything typed with 15 digits or fewer copies exactly); dates as ISO dates.
fn quantity_literal(canonical: f64, q: &Quant) -> String {
    let shown = q.disp.to_display(canonical);
    if !shown.is_finite() {
        return fmt_quantity(canonical, q);
    }
    if q.disp.is_date() && q.absolute.is_some() {
        return fmt_date(shown);
    }
    let best: f64 = format!("{shown:.14e}").parse().unwrap_or(shown);
    let ax = best.abs();
    let n = if ax == 0.0 || (1e-6..1e15).contains(&ax) { format!("{best}") } else { format!("{best:e}") };
    if q.disp.is_none() {
        n
    } else {
        format!("{n} [{}]", q.disp)
    }
}

/// Paste a clip with its top-left at `at`; formulas move relative refs.
pub fn paste(e: &mut Engine, clip: &Clip, sheet: SheetId, at: (usize, usize)) -> Edit {
    let mut out = Vec::new();
    for i in 0..clip.rows {
        for j in 0..clip.cols {
            let Some(tk) = e.wb.sheet(sheet).unwrap().key(at.0 + i, at.1 + j) else { continue };
            let cell = clip.cells[i * clip.cols + j].as_ref().map(|(sk, c)| {
                let text = e.wb.render(&c.pieces, sk.sheet);
                if classify(&text).has_refs() {
                    Cell::new(e.wb.shift_pieces(&c.pieces, *sk, tk))
                } else {
                    c.clone()
                }
            });
            out.push((tk, cell));
        }
    }
    Edit::Cells(out)
}

/// Paste plain text (tab/newline separated) as typed input.
pub fn paste_text(e: &mut Engine, text: &str, sheet: SheetId, at: (usize, usize)) -> Edit {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (j, field) in line.split('\t').enumerate() {
            let Some(tk) = e.wb.sheet(sheet).unwrap().key(at.0 + i, at.1 + j) else { continue };
            let cell = if field.trim().is_empty() { None } else { Some(Cell::new(e.wb.parse_text(field, sheet))) };
            out.push((tk, cell));
        }
    }
    Edit::Cells(out)
}

pub fn clear(e: &Engine, r: Rect) -> Edit {
    let s = e.wb.sheet(r.sheet).unwrap();
    let mut out = Vec::new();
    for row in r.r0..=r.r1 {
        for col in r.c0..=r.c1 {
            if let Some(k) = s.key(row, col) {
                if s.cell(k.row, k.col).is_some() {
                    out.push((k, None));
                }
            }
        }
    }
    Edit::Cells(out)
}

/// Moves the cells of `src` so its top-left lands at `at` on `sheet`, as one undoable edit.
/// One rule: every reference to a moved cell follows it, every reference to a cell it lands on
/// becomes `#ref!` (that cell is gone, as if deleted), and nothing else is rewritten. A range
/// counts as a reference to those cells only when all of its cells are; any other range is left
/// alone. Names follow their cells the same way. A spill moves with its source; a spilled cell
/// can't be moved on its own.
pub fn move_cells(e: &mut Engine, src: Rect, sheet: SheetId, at: (usize, usize)) -> Result<Edit, String> {
    let (dr, dc) = (at.0 as i64 - src.r0 as i64, at.1 as i64 - src.c0 as i64);
    e.wb.sheet(sheet).ok_or("no such sheet")?;
    let s = e.wb.sheet(src.sheet).ok_or("no such sheet")?;
    if src.r1.max(at.0 + src.rows()) >= MAX_INDEX || src.c1.max(at.1 + src.cols()) >= MAX_INDEX {
        return Err("that is past the last row".into());
    }
    // source position -> destination position: the block, plus the spills of sources in it
    let mut spans: Vec<(usize, usize, usize, usize)> = vec![(src.r0, src.c0, src.rows(), src.cols())];
    for r in src.r0..=src.r1 {
        for c in src.c0..=src.c1 {
            let k = s.key(r, c).unwrap();
            if let Some(a) = e.spill_anchor(k).filter(|a| *a != k) {
                if !s.pos(a).is_some_and(|(ar, ac)| src.contains(ar, ac)) {
                    let at = e.wb.cell_label(k, Some(src.sheet));
                    return Err(format!("{at} is spilled — move its source {} instead", e.wb.cell_label(a, Some(src.sheet))));
                }
            }
            if let Some((nr, nc)) = e.spill_size(k) {
                spans.push((r, c, nr, nc));
            }
        }
    }
    if sheet == src.sheet && dr == 0 && dc == 0 {
        return Ok(Edit::Cells(vec![]));
    }
    let mut moved: HashMap<CellKey, CellKey> = HashMap::default();
    let mut block = Vec::new();
    for (r0, c0, nr, nc) in spans {
        for r in r0..r0 + nr {
            for c in c0..c0 + nc {
                let sk = e.wb.sheet(src.sheet).unwrap().key(r, c).unwrap();
                let dk = e.wb.sheet(sheet).unwrap().key((r as i64 + dr) as usize, (c as i64 + dc) as usize).unwrap();
                if moved.insert(sk, dk).is_none() && src.contains(r, c) {
                    block.push((sk, dk));
                }
            }
        }
    }
    let replaced: HashSet<CellKey> = block.iter().map(|(_, d)| *d).filter(|d| !moved.contains_key(d)).collect();
    let wb = &e.wb;
    let m = Mover { wb, moved: &moved, replaced: &replaced };
    let mut out: HashMap<CellKey, Option<Cell>> = HashMap::default();
    for (sk, _) in &block {
        out.insert(*sk, None);
    }
    for (sk, dk) in &block {
        out.insert(*dk, wb.cell(*sk).map(|c| Cell::new(m.pieces(&c.pieces, src.sheet, sheet))));
    }
    let touched: HashSet<CellKey> = block.iter().flat_map(|(s, d)| [*s, *d]).collect();
    for s in &wb.sheets {
        for ((r, c), cell) in &s.cells {
            let k = CellKey { sheet: s.id, row: *r, col: *c };
            if touched.contains(&k) || !cell.pieces.iter().any(|p| !matches!(p, Piece::Text(_))) {
                continue;
            }
            let pieces = m.pieces(&cell.pieces, s.id, s.id);
            if pieces[..] != cell.pieces[..] {
                out.insert(k, Some(Cell::new(pieces)));
            }
        }
    }
    let mut names = wb.names.clone();
    names.retain(|_, d| !replaced.contains(&d.cell));
    for d in names.values_mut() {
        if let Some(k) = moved.get(&d.cell) {
            d.cell = *k;
        }
    }
    // a stable order keeps the edit (and its inverse) deterministic
    let mut cells: Vec<(CellKey, Option<Cell>)> = out.into_iter().collect();
    cells.sort_by_key(|(k, _)| (k.sheet.0, k.row.0, k.col.0));
    let mut edits = vec![Edit::Cells(cells)];
    edits.extend(Edit::names(&wb.names, &names));
    Ok(Edit::Batch(edits))
}

/// Rewrites references for `move_cells`.
struct Mover<'a> {
    wb: &'a Workbook,
    moved: &'a HashMap<CellKey, CellKey>,
    replaced: &'a HashSet<CellKey>,
}

enum Fate {
    Moved,
    Replaced,
    Stays,
}

impl Mover<'_> {
    fn fate(&self, keys: &[CellKey]) -> Fate {
        if keys.is_empty() {
            Fate::Stays
        } else if keys.iter().all(|k| self.moved.contains_key(k)) {
            Fate::Moved
        } else if keys.iter().all(|k| self.replaced.contains(k)) {
            Fate::Replaced
        } else {
            Fate::Stays
        }
    }

    /// The pieces of a cell that lived on `from` and now lives on `to`.
    fn pieces(&self, pieces: &[Piece], from: SheetId, to: SheetId) -> Vec<Piece> {
        // a reference without a sheet means the cell's own sheet, so it needs one if the two differ
        let sheet_of = |orig: Option<SheetId>, target: SheetId| if orig.is_some() || target != to { Some(target) } else { None };
        pieces
            .iter()
            .map(|p| match p {
                Piece::Text(_) => p.clone(),
                Piece::Ref(r) => {
                    let k = CellKey { sheet: r.sheet.unwrap_or(from), row: r.row, col: r.col };
                    match self.fate(&[k]) {
                        Fate::Moved => {
                            let n = self.moved[&k];
                            Piece::Ref(StoredRef { sheet: sheet_of(r.sheet, n.sheet), row: n.row, col: n.col, ..*r })
                        }
                        Fate::Replaced => Piece::Text("#ref!".into()),
                        Fate::Stays => Piece::Ref(StoredRef { sheet: sheet_of(r.sheet, k.sheet), ..*r }),
                    }
                }
                Piece::Range(a, b) => {
                    let sid = a.sheet.unwrap_or(from);
                    let stays = Piece::Range(StoredRef { sheet: sheet_of(a.sheet, sid), ..*a }, *b);
                    let Some(s) = self.wb.sheet(sid) else { return stays };
                    let Some((r0, c0, r1, c1)) = s.range_bounds(a, b) else { return stays };
                    // only a range no bigger than the moved block can be wholly inside it
                    if (r1 - r0 + 1) * (c1 - c0 + 1) > self.moved.len().max(self.replaced.len()) {
                        return stays;
                    }
                    let keys: Vec<CellKey> = (r0..=r1).flat_map(|r| (c0..=c1).map(move |c| (r, c))).filter_map(|(r, c)| s.key(r, c)).collect();
                    match self.fate(&keys) {
                        Fate::Moved => {
                            let (ka, kb) = (self.moved[&s.key(r0, c0).unwrap()], self.moved[&s.key(r1, c1).unwrap()]);
                            Piece::Range(
                                StoredRef { sheet: sheet_of(a.sheet, ka.sheet), row: ka.row, col: ka.col, ..*a },
                                StoredRef { row: kb.row, col: kb.col, ..*b },
                            )
                        }
                        Fate::Replaced => Piece::Text("#ref!".into()),
                        Fate::Stays => stays,
                    }
                }
            })
            .collect()
    }
}

#[derive(PartialEq, PartialOrd)]
enum SortKey {
    Num(f64),
    Text(String),
    Empty,
}

/// Reorders whole rows r0..=r1 by the value shown in `col`. Rows move as
/// ids, so every reference keeps pointing at the same cell.
pub fn sort_rows(e: &Engine, r: Rect, col: usize, ascending: bool) -> Edit {
    let s = e.wb.sheet(r.sheet).unwrap();
    let mut rows: Vec<(SortKey, RowId)> = (r.r0..=r.r1)
        .map(|row| {
            let k = s.key(row, col).unwrap();
            let key = match e.shown(k) {
                Shown::Value { value: Value::Num(n), dr, dc, .. } => {
                    let i = match n.shape.as_slice() {
                        [] => 0,
                        [_] => dr,
                        [_, c, ..] => dr * c + dc,
                    };
                    n.data.get(i).map(|x| SortKey::Num(*x)).unwrap_or(SortKey::Empty)
                }
                Shown::Value { value, dr, dc, .. } => SortKey::Text(value.display_at(dr, dc).to_lowercase()),
                _ => SortKey::Empty,
            };
            (key, k.row)
        })
        .collect();
    // empty rows at the end stay where they are anyway: leave them out, so sorting a whole column
    // doesn't materialise every virtual row in it
    let used: HashSet<RowId> = s.cells.keys().map(|(r, _)| *r).collect();
    while rows.len() > 1 && rows.last().is_some_and(|(k, id)| matches!(k, SortKey::Empty) && !used.contains(id)) {
        rows.pop();
    }
    let r = Rect { r1: r.r0 + rows.len() - 1, ..r };
    rows.sort_by(|a, b| {
        let o = match (&a.0, &b.0) {
            (SortKey::Empty, SortKey::Empty) => std::cmp::Ordering::Equal,
            (SortKey::Empty, _) => return std::cmp::Ordering::Greater,
            (_, SortKey::Empty) => return std::cmp::Ordering::Less,
            (x, y) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        };
        if ascending { o } else { o.reverse() }
    });
    // the i-th sorted row takes the place of the i-th row as it was
    let places = (r.r0..=r.r1).map(|row| s.rows.get(row).unwrap());
    Edit::PermuteRows { sheet: r.sheet, moves: rows.into_iter().map(|x| x.1).zip(places).collect() }
}

/// After entering a program at `k`: if the column beside it continues below
/// while this column is empty there, offer to extend the program down.
/// Returns the fill target (including `k`).
pub fn extension_offer(e: &Engine, k: CellKey) -> Option<Rect> {
    if e.kind(k) != Kind::Program {
        return None;
    }
    let s = e.wb.sheet(k.sheet)?;
    let (r, c) = s.pos(k)?;
    let filled = |row: usize, col: usize| s.cell_at(row, col).is_some();
    if filled(r + 1, c) {
        return None;
    }
    let mut best = None;
    for nc in [c.checked_sub(1), Some(c + 1)].into_iter().flatten() {
        if !filled(r, nc) || !filled(r + 1, nc) {
            continue;
        }
        let mut end = r + 1;
        while filled(end + 1, nc) && !filled(end + 1, c) {
            end += 1;
        }
        if best.map(|b: usize| end > b).unwrap_or(true) {
            best = Some(end);
        }
    }
    best.map(|end| Rect { sheet: k.sheet, r0: r, c0: c, r1: end, c1: c })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::default_workbook;

    fn setup() -> (Engine, SheetId) {
        let e = Engine::new(default_workbook());
        let sid = e.wb.sheets[0].id;
        (e, sid)
    }
    fn k(e: &Engine, s: SheetId, r: usize, c: usize) -> CellKey {
        e.wb.sheet(s).unwrap().key(r, c).unwrap()
    }

    #[test]
    fn fill_series_and_formulas() {
        let (mut e, s) = setup();
        e.set_text(k(&e, s, 0, 0), "1.0 [m]");
        e.set_text(k(&e, s, 1, 0), "1.5 [m]");
        e.set_text(k(&e, s, 0, 1), "=A1 2 *");
        e.set_text(k(&e, s, 0, 2), "2026-01-30");
        e.set_text(k(&e, s, 1, 2), "2026-01-31");
        e.set_text(k(&e, s, 0, 3), "x");
        let ed = fill(&mut e, Rect::span(s, (0, 0), (1, 3)), Rect::span(s, (0, 0), (4, 3)));
        e.apply(ed);
        assert_eq!(e.wb.cell_text(k(&e, s, 4, 0)), "3.0 [m]");
        assert_eq!(e.wb.cell_text(k(&e, s, 2, 1)), "=A3 2 *");
        assert_eq!(e.wb.cell_text(k(&e, s, 3, 1)), ""); // B2 was empty, pattern repeats
        assert_eq!(e.wb.cell_text(k(&e, s, 4, 2)), "2026-02-03");
        assert_eq!(e.wb.cell_text(k(&e, s, 2, 3)), "x");
        // single number copies
        e.set_text(k(&e, s, 0, 5), "7");
        let ed = fill(&mut e, Rect::cell(s, 0, 5), Rect::span(s, (0, 5), (2, 5)));
        e.apply(ed);
        assert_eq!(e.wb.cell_text(k(&e, s, 2, 5)), "7");
        // fill right with relative refs
        let ed = fill(&mut e, Rect::cell(s, 0, 1), Rect::span(s, (0, 1), (0, 2)));
        e.apply(ed);
        assert_eq!(e.wb.cell_text(k(&e, s, 0, 2)), "=B1 2 *");
    }

    #[test]
    fn copy_paste_and_sort() {
        let (mut e, s) = setup();
        e.set_text(k(&e, s, 0, 0), "3");
        e.set_text(k(&e, s, 1, 0), "1");
        e.set_text(k(&e, s, 2, 0), "2");
        e.set_text(k(&e, s, 0, 1), "=A1 10 *");
        e.set_text(k(&e, s, 1, 1), "=A2 10 *");
        e.set_text(k(&e, s, 2, 1), "=A3 10 *");
        e.set_text(k(&e, s, 0, 2), "=B1:B3 sum");
        let clip = copy(&e, Rect::cell(s, 0, 1));
        let ed = paste(&mut e, &clip, s, (5, 3));
        e.apply(ed);
        assert_eq!(e.wb.cell_text(k(&e, s, 5, 3)), "=C6 10 *");
        let ed = sort_rows(&e, Rect::span(s, (0, 0), (2, 1)), 0, true);
        e.apply(ed);
        assert_eq!(e.wb.cell_text(k(&e, s, 0, 1)), "=A1 10 *");
        let v = |e: &Engine, r, c| match e.shown(k(e, s, r, c)) {
            Shown::Value { value, .. } => value.display_at(0, 0),
            _ => "?".into(),
        };
        assert_eq!(v(&e, 0, 0), "1");
        assert_eq!(v(&e, 0, 1), "10");
        assert_eq!(v(&e, 2, 1), "30");
        // C1 was row 0 and moved with it; its range still covers B1:B3
        assert_eq!(e.wb.cell_text(k(&e, s, 2, 2)), "=B1:B3 sum");
    }

    #[test]
    fn scrubbing_and_extension() {
        assert_eq!(scrub(&cell_literal("0.05").unwrap(), 3.0), "0.08");
        assert_eq!(scrub(&cell_literal(" 10 [m]").unwrap(), -12.0), "-2");
        let lits = program_literals("=A1 1.5 * 2 +");
        assert_eq!(lits.len(), 2);
        assert_eq!(lits[0].span, 4..7);
        let (mut e, s) = setup();
        for r in 0..5 {
            e.set_text(k(&e, s, r, 0), &format!("{}", r + 1));
        }
        e.set_text(k(&e, s, 0, 1), "=A1 2 *");
        let offer = extension_offer(&e, k(&e, s, 0, 1)).unwrap();
        assert_eq!((offer.r0, offer.r1), (0, 4));
    }

    fn text(e: &Engine, s: SheetId, at: &str) -> String {
        let r = crate::a1::parse_ref(at).unwrap();
        e.wb.cell_text(k(e, s, r.row, r.col))
    }
    fn val(e: &Engine, s: SheetId, at: &str) -> String {
        let r = crate::a1::parse_ref(at).unwrap();
        match e.shown(k(e, s, r.row, r.col)) {
            Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
            Shown::Error(err) => format!("ERR {}", err.msg),
            Shown::Empty => String::new(),
        }
    }
    fn put(e: &mut Engine, s: SheetId, cells: &[(&str, &str)]) {
        for (at, t) in cells {
            let r = crate::a1::parse_ref(at).unwrap();
            e.set_text(k(e, s, r.row, r.col), t);
        }
    }
    /// Every cell's text by position, and the names, for exact round-trip checks.
    fn snapshot(e: &Engine) -> Vec<String> {
        let mut out: Vec<String> = e
            .wb
            .sheets
            .iter()
            .flat_map(|s| s.cells.keys().map(move |(r, c)| CellKey { sheet: s.id, row: *r, col: *c }))
            .map(|k| format!("{} {}", e.wb.cell_label(k, None), e.wb.cell_text(k)))
            .collect();
        out.extend(e.wb.names.iter().map(|(n, d)| format!("{n} -> {}", e.wb.cell_label(d.cell, None))));
        out.sort();
        out
    }
    /// Moves, checks that undo restores exactly and redo reapplies exactly.
    fn mv(e: &mut Engine, src: Rect, sheet: SheetId, at: (usize, usize)) {
        let before = snapshot(e);
        let ed = move_cells(e, src, sheet, at).unwrap();
        let inv = e.apply(ed);
        let after = snapshot(e);
        let redo = e.apply(inv);
        assert_eq!(snapshot(e), before);
        e.apply(redo);
        assert_eq!(snapshot(e), after);
    }

    #[test]
    fn move_references_follow() {
        let (mut e, s) = setup();
        put(&mut e, s, &[("A1", "1"), ("A2", "2"), ("B1", "=A1 10 *"), ("C1", "=A1:A2 sum"), ("C2", "=A1:A3? sum"), ("C3", "=$A$2")]);
        mv(&mut e, Rect::span(s, (0, 0), (1, 0)), s, (0, 4));
        assert_eq!(text(&e, s, "A1"), "");
        assert_eq!(text(&e, s, "E1"), "1");
        // single references and whole ranges follow, keeping their $ flags
        assert_eq!(text(&e, s, "B1"), "=E1 10 *");
        assert_eq!(val(&e, s, "B1"), "10");
        assert_eq!(text(&e, s, "C1"), "=E1:E2 sum");
        assert_eq!(val(&e, s, "C1"), "3");
        assert_eq!(text(&e, s, "C3"), "=$E$2");
        // a range only partly inside the moved block is left alone
        assert_eq!(text(&e, s, "C2"), "=A1:A3? sum");
    }

    #[test]
    fn moved_formulas_keep_their_targets() {
        let (mut e, s) = setup();
        put(&mut e, s, &[("A1", "1"), ("B1", "=A1 1 +"), ("B2", "=B1 2 *"), ("B3", "=B1:B2 sum")]);
        mv(&mut e, Rect::span(s, (0, 1), (2, 1)), s, (4, 3));
        // relative or not, a reference to a cell that didn't move still points at it
        assert_eq!(text(&e, s, "D5"), "=A1 1 +");
        // references to cells that moved along follow them
        assert_eq!(text(&e, s, "D6"), "=D5 2 *");
        assert_eq!(text(&e, s, "D7"), "=D5:D6 sum");
        assert_eq!(val(&e, s, "D7"), "6");
        // unlike paste, which moves relative references
        let clip = copy(&e, Rect::cell(s, 4, 3));
        let ed = paste(&mut e, &clip, s, (4, 5));
        e.apply(ed);
        assert_eq!(text(&e, s, "F5"), "=C1 1 +");
    }

    #[test]
    fn move_replaces_destination_cells() {
        let (mut e, s) = setup();
        put(&mut e, s, &[("A1", "5"), ("D1", "7"), ("D2", "8"), ("B1", "=D1"), ("B2", "=D1:D1 sum"), ("B3", "=D1:D2 sum"), ("B4", "=D2")]);
        e.set_name("seven", Some(k(&e, s, 0, 3)), false).unwrap();
        e.set_name("five", Some(k(&e, s, 0, 0)), false).unwrap();
        mv(&mut e, Rect::cell(s, 0, 0), s, (0, 3));
        assert_eq!(text(&e, s, "D1"), "5");
        // references to the cell the move landed on break, as if it was deleted
        assert_eq!(text(&e, s, "B1"), "=#ref!");
        assert_eq!(val(&e, s, "B1"), "ERR reference to a deleted cell");
        assert_eq!(text(&e, s, "B2"), "=#ref! sum");
        // a range that only touches it, and other cells, are left alone
        assert_eq!(text(&e, s, "B3"), "=D1:D2 sum");
        assert_eq!(val(&e, s, "B3"), "13");
        assert_eq!(text(&e, s, "B4"), "=D2");
        // names follow too: the replaced cell's name is gone
        assert!(!e.wb.names.contains_key("seven"));
        assert_eq!(e.wb.names["five"].cell, k(&e, s, 0, 3));
        // undo brings the cell and its references back
        let before = snapshot(&e);
        let ed = move_cells(&mut e, Rect::cell(s, 0, 3), s, (0, 0)).unwrap();
        let inv = e.apply(ed);
        e.apply(inv);
        assert_eq!(snapshot(&e), before);
    }

    #[test]
    fn move_onto_itself_overlapping() {
        let (mut e, s) = setup();
        put(&mut e, s, &[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "=A1:A3 sum"), ("B2", "=A3"), ("B3", "=A4")]);
        mv(&mut e, Rect::span(s, (0, 0), (2, 0)), s, (1, 0));
        assert_eq!(text(&e, s, "A1"), "");
        assert_eq!(text(&e, s, "A4"), "3");
        assert_eq!(text(&e, s, "B1"), "=A2:A4 sum");
        assert_eq!(text(&e, s, "B2"), "=A4");
        // A4 was replaced (it didn't move itself)
        assert_eq!(text(&e, s, "B3"), "=#ref!");
        // a move to the same place does nothing
        let ed = move_cells(&mut e, Rect::span(s, (1, 0), (3, 0)), s, (1, 0)).unwrap();
        assert!(matches!(ed, Edit::Cells(ref c) if c.is_empty()));
    }

    #[test]
    fn move_across_sheets() {
        let (mut e, s) = setup();
        let ed = e.add_sheet_edit(1);
        e.apply(ed);
        let t = e.wb.sheets[1].id;
        assert_eq!(e.wb.sheets[1].name, "Sheet3");
        put(&mut e, s, &[("A1", "4"), ("A2", "=A1 C1 +"), ("C1", "1"), ("B1", "=A1 2 *"), ("B2", "=A1:A2 sum")]);
        put(&mut e, t, &[("D1", "=Sheet1!A2")]);
        mv(&mut e, Rect::span(s, (0, 0), (1, 0)), t, (1, 1));
        assert_eq!(text(&e, s, "B1"), "=Sheet3!B2 2 *");
        assert_eq!(text(&e, s, "B2"), "=Sheet3!B2:B3 sum");
        assert_eq!(val(&e, s, "B2"), "9");
        // on the new sheet: the moved pair still refer to each other, and C1 stays on Sheet1
        assert_eq!(text(&e, t, "B3"), "=B2 Sheet1!C1 +");
        assert_eq!(text(&e, t, "D1"), "=Sheet3!B3");
        assert_eq!(val(&e, t, "D1"), "5");
    }

    #[test]
    fn move_takes_the_spill_along() {
        let (mut e, s) = setup();
        put(&mut e, s, &[("A1", "=3 range"), ("B1", "=A3"), ("B2", "=A1 sum")]);
        assert_eq!(val(&e, s, "A3"), "2");
        // a spilled cell can't move on its own
        let err = move_cells(&mut e, Rect::cell(s, 1, 0), s, (5, 5)).unwrap_err();
        assert!(err.contains("A2 is spilled"), "{err}");
        mv(&mut e, Rect::cell(s, 0, 0), s, (0, 3));
        assert_eq!(val(&e, s, "D3"), "2");
        assert_eq!(text(&e, s, "B1"), "=D3");
        assert_eq!(val(&e, s, "B1"), "2");
        assert_eq!(text(&e, s, "B2"), "=D1 sum");
    }

    /// A cell's value exactly: number bits and unit, or text.
    #[derive(Debug, PartialEq)]
    enum V {
        Num(f64, Quant),
        Text(String),
        Err(&'static str),
        Other,
        Empty,
    }

    fn exact(e: &Engine, k: CellKey) -> V {
        match e.shown(k) {
            Shown::Value { value: Value::Num(n), dr, dc, .. } => {
                let i = if n.shape.len() == 2 { dr * n.shape[1] + dc } else { dr };
                V::Num(n.data[i], n.q.clone())
            }
            Shown::Value { value: Value::Text(t), dr, .. } => V::Text(t.data[dr].to_string()),
            Shown::Value { .. } => V::Other,
            Shown::Error(err) => V::Err(err.short()),
            Shown::Empty => V::Empty,
        }
    }

    #[test]
    fn copied_values_paste_back_as_the_same_values() {
        let (mut e, s) = setup();
        let rows = [
            ["5300 [m]", "=A1 3 /", "50 [%]", "=C1 3 /", "=1 3e9 /", "=1e20 [m]", "=0.1 0.2 +", "=5.3 [km] 3 *"],
            ["20 [°C]", "=A2 5.5 [Δ°C] +", "2026-01-31", "=C2 30 [day] +", "=212 [°F] to[°C]", "=-0.25 [m/s^2]", "=1 [kg*m/s^2] 3 /", ""],
            ["hello", "'5", "'=A1", "'  padded", "'dim x", "=\"2026-01-31\"", "'it's", "=1 [m] 1 [s] +"],
            ["1", "2", "=A4:B5 10 *", "", "=A4:A5 1 [km] *", "", "", ""],
            ["3", "4", "", "", "", "", "", ""],
            ["4.0 [%]", "0.05", "120000 [USD]", "123456789012345", "0.1 [km]", "-37.5 [°F]", "1.5e-7 [m]", "98.6 [°F]"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, t) in row.iter().enumerate() {
                e.set_text(k(&e, s, r, c), t);
            }
        }
        let clip = copy_values(&e, Rect::span(s, (0, 0), (5, 7)));
        let lines: Vec<&str> = clip.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].split('\t').count(), 8);
        let at = |r: usize, c: usize| lines[r].split('\t').nth(c).unwrap().to_string();
        // re-typeable literals, not the display (`5,300 m`)
        assert_eq!(at(0, 0), "5300 [m]");
        assert_eq!(at(0, 2), "50 [%]");
        assert_eq!(at(0, 5), "1e20 [m]");
        assert_eq!(at(0, 7), "15.9 [km]");
        assert_eq!(at(1, 0), "20 [°C]");
        assert_eq!(at(1, 1), "25.5 [°C]");
        assert_eq!(at(1, 2), "2026-01-31");
        assert_eq!(at(1, 3), "2026-03-02");
        // computed values are rounded to 15 significant digits, dropping float noise
        assert_eq!(at(0, 6), "0.3");
        assert_eq!(at(1, 4), "100 [°C]");
        assert_eq!(at(0, 1), "1766.66666666667 [m]");
        assert_eq!(at(5, 0), "4 [%]");
        assert_eq!(at(5, 3), "123456789012345");
        assert_eq!(at(1, 5), "-0.25 [m/s^2]");
        // text is quoted only where it would read as something else
        assert_eq!(at(2, 0), "hello");
        assert_eq!(at(2, 1), "'5");
        assert_eq!(at(2, 2), "'=A1");
        assert_eq!(at(2, 3), "'  padded");
        assert_eq!(at(2, 4), "'dim x");
        assert_eq!(at(2, 5), "'2026-01-31");
        assert_eq!(at(2, 6), "it's");
        // an error copies its short code; spilled cells their element
        assert_eq!(at(2, 7), "#err");
        assert_eq!((at(3, 2), at(3, 3), at(4, 2), at(4, 3)), ("10".into(), "20".into(), "30".into(), "40".into()));
        assert_eq!((at(3, 4), at(4, 4)), ("1 [km]".into(), "3 [km]".into()));
        // pasting back reproduces every value with 15 significant digits or fewer exactly, units included;
        // computed values with more come back within the rounding
        let noisy = [(0, 1), (0, 3), (0, 4), (0, 6), (1, 4), (1, 6)];
        let ed = paste_text(&mut e, &clip, s, (10, 0));
        e.apply(ed);
        for r in 0..6 {
            for c in 0..8 {
                let (a, b) = (exact(&e, k(&e, s, r, c)), exact(&e, k(&e, s, r + 10, c)));
                match (a, b) {
                    (V::Err(code), b) => assert_eq!(b, V::Text(code.into())),
                    (V::Num(x, qa), V::Num(y, qb)) if noisy.contains(&(r, c)) => {
                        assert_eq!(qa, qb);
                        let tol = if (r, c) == (0, 6) { 1e-15 } else { 1e-14 };
                        assert!((x - y).abs() <= tol * x.abs(), "{} pasted back as {y}, was {x}", at(r, c));
                    }
                    (a, b) => assert_eq!(a, b, "{} pasted back as {:?}", at(r, c), b),
                }
            }
        }
    }

    #[test]
    fn charts_and_declarations_copy_as_nothing() {
        let (mut e, s) = setup();
        e.set_text(k(&e, s, 0, 0), "1");
        e.set_text(k(&e, s, 1, 0), "2");
        e.set_text(k(&e, s, 0, 1), "=A1:A2 A1:A2 line");
        e.set_text(k(&e, s, 0, 20), "[furlong] = 201.168 [m]");
        e.set_text(k(&e, s, 1, 20), "dim money");
        e.set_text(k(&e, s, 2, 20), ": twice ( x -- 2x ) 2 * ;");
        assert!(matches!(exact(&e, k(&e, s, 0, 1)), V::Other));
        assert!(matches!(e.shown(k(&e, s, 3, 3)), Shown::Value { value: Value::Chart(_), .. }), "C4 is under the chart");
        assert_eq!(copy_values(&e, Rect::span(s, (0, 0), (1, 2))), "1\t\t\n2\t\t");
        assert_eq!(copy_values(&e, Rect::span(s, (0, 20), (2, 20))), "\n\n");
    }
}
