//! Editing operations built on the engine: fill, copy/paste, sort, scrub
//! helpers, formula extension. Each returns an `Edit` to apply (and undo).

use crate::engine::{Edit, Engine, Shown};
use crate::ids::*;
use crate::lex::{self, Tok};
use crate::model::{classify, number_literal, Cell, Kind, Piece};
use crate::value::{fmt_date, Value};
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
        let keys: Vec<CellKey> = src_pos.iter().map(|(r, c)| e.wb.sheet_mut(src.sheet).unwrap().key_grow(*r, *c)).collect();
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
            let tk = e.wb.sheet_mut(dst.sheet).unwrap().key_grow(tr, tc);
            if let Some((first, step, ref tmpl, ref lit)) = series {
                let v = first + step * i as f64;
                let text = replace_span(tmpl, &lit.span, &format_lit(v, lit.decimals, lit.is_date));
                out.push((tk, Some(Cell { pieces: vec![Piece::Text(text)] })));
                continue;
            }
            let j = i.rem_euclid(n) as usize;
            let sk = keys[j];
            let cell = cell_of(e, sk).map(|c| {
                if classify(&texts[j]).has_refs() {
                    Cell { pieces: e.wb.shift_pieces(&c.pieces, sk, tk) }
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

/// Paste a clip with its top-left at `at`; formulas move relative refs.
pub fn paste(e: &mut Engine, clip: &Clip, sheet: SheetId, at: (usize, usize)) -> Edit {
    let mut out = Vec::new();
    for i in 0..clip.rows {
        for j in 0..clip.cols {
            let tk = e.wb.sheet_mut(sheet).unwrap().key_grow(at.0 + i, at.1 + j);
            let cell = clip.cells[i * clip.cols + j].as_ref().map(|(sk, c)| {
                let text = e.wb.render(&c.pieces, sk.sheet);
                if classify(&text).has_refs() {
                    Cell { pieces: e.wb.shift_pieces(&c.pieces, *sk, tk) }
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
            let tk = e.wb.sheet_mut(sheet).unwrap().key_grow(at.0 + i, at.1 + j);
            let cell = if field.trim().is_empty() { None } else { Some(Cell { pieces: e.wb.parse_text(field, sheet) }) };
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
    rows.sort_by(|a, b| {
        let o = match (&a.0, &b.0) {
            (SortKey::Empty, SortKey::Empty) => std::cmp::Ordering::Equal,
            (SortKey::Empty, _) => return std::cmp::Ordering::Greater,
            (_, SortKey::Empty) => return std::cmp::Ordering::Less,
            (x, y) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        };
        if ascending { o } else { o.reverse() }
    });
    Edit::PermuteRows { sheet: r.sheet, at: r.r0, ids: rows.into_iter().map(|x| x.1).collect() }
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
        if nc >= s.cols.len() || !filled(r, nc) || !filled(r + 1, nc) {
            continue;
        }
        let mut end = r + 1;
        while end + 1 < s.rows.len() && filled(end + 1, nc) && !filled(end + 1, c) {
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
}
