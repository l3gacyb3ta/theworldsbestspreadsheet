//! Document model. Row/column order are lists of ids, cells are a map keyed by
//! ids, formulas store references as ids. Nothing here assumes positions are
//! stable, so the model maps onto Automerge (lists + maps) directly.

use crate::a1::{self, A1Ref};
use crate::ids::*;
use crate::lex::{self, Tok};
use serde::{Deserialize, Serialize};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct StoredRef {
    /// `None`: the formula's own sheet.
    pub sheet: Option<SheetId>,
    pub row: RowId,
    pub col: ColId,
    pub row_abs: bool,
    pub col_abs: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Piece {
    Text(String),
    Ref(StoredRef),
    /// Both corners; the second corner's `sheet` is ignored.
    Range(StoredRef, StoredRef),
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Cell {
    /// Shared: copying a cell (undo, snapshots for saving) doesn't copy its pieces.
    pub pieces: Arc<[Piece]>,
}

impl Cell {
    pub fn new(pieces: Vec<Piece>) -> Cell {
        Cell { pieces: pieces.into() }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Empty,
    Text,
    Number,
    Program,
    WordDef,
    UnitDecl,
}

impl Kind {
    pub fn has_refs(self) -> bool {
        matches!(self, Kind::Program | Kind::WordDef | Kind::UnitDecl)
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Empty => "empty",
            Kind::Text => "text",
            Kind::Number => "number",
            Kind::Program => "program",
            Kind::WordDef => "word",
            Kind::UnitDecl => "unit declaration",
        }
    }
}

/// How a cell's text is read. One rule each:
/// `=` program, `:` word definition, `dim`/`base`/`[u] =` unit declaration,
/// a number (optionally followed by `[unit]`) or ISO date is a number,
/// `'` forces text, everything else is text.
pub fn classify(text: &str) -> Kind {
    let t = text.trim();
    if t.is_empty() {
        return Kind::Empty;
    }
    if t.starts_with('\'') {
        return Kind::Text;
    }
    if t.starts_with('=') {
        return Kind::Program;
    }
    if t.starts_with(':') {
        return Kind::WordDef;
    }
    if is_unit_decl(t) {
        return Kind::UnitDecl;
    }
    if number_literal(t).is_some() {
        return Kind::Number;
    }
    Kind::Text
}

fn is_unit_decl(t: &str) -> bool {
    let toks = lex::lex_code(t);
    match toks.first().map(|t| &t.tok) {
        Some(Tok::Word(w)) if w == "dim" => toks.len() == 2 && matches!(toks[1].tok, Tok::Word(_)),
        Some(Tok::Word(w)) if w == "base" => {
            toks.len() == 3 && matches!(toks[1].tok, Tok::Unit(_)) && matches!(toks[2].tok, Tok::Word(_))
        }
        Some(Tok::Unit(_)) => matches!(toks.get(1).map(|t| &t.tok), Some(Tok::Word(w)) if w == "="),
        _ => false,
    }
}

/// A literal number: `5`, `-2.5e3`, `5 [m/s]`, `2026-10-08`.
/// Returns (number token span, optional unit text).
pub fn number_literal(t: &str) -> Option<(std::ops::Range<usize>, Option<String>)> {
    let toks = lex::lex_code(t);
    match toks.as_slice() {
        [n] if matches!(n.tok, Tok::Num(_) | Tok::Date(_)) => Some((n.span.clone(), None)),
        [n, u] if matches!(n.tok, Tok::Num(_)) => match &u.tok {
            Tok::Unit(s) => Some((n.span.clone(), Some(s.clone()))),
            _ => None,
        },
        _ => None,
    }
}

/// An ordered list of ids with tombstones, like a list CRDT: deleted ids keep
/// their place so a range whose end row was deleted can shrink instead of
/// breaking, and undo can revive them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(serialize = "I: Serialize", deserialize = "I: Deserialize<'de>"))]
pub struct Axis<I: Copy + Eq + std::hash::Hash> {
    order: Vec<I>,
    dead: HashSet<I>,
    #[serde(skip)]
    alive: Vec<I>,
    #[serde(skip)]
    pos: HashMap<I, usize>,
    /// For dead ids: number of alive ids before it.
    #[serde(skip)]
    tomb: HashMap<I, usize>,
}

impl<I: Copy + Eq + std::hash::Hash> Axis<I> {
    pub fn new(ids: Vec<I>) -> Self {
        let mut a = Axis { order: ids, dead: HashSet::default(), alive: vec![], pos: HashMap::default(), tomb: HashMap::default() };
        a.reindex();
        a
    }
    pub fn reindex(&mut self) {
        self.alive.clear();
        self.pos.clear();
        self.tomb.clear();
        for id in &self.order {
            if self.dead.contains(id) {
                self.tomb.insert(*id, self.alive.len());
            } else {
                self.pos.insert(*id, self.alive.len());
                self.alive.push(*id);
            }
        }
    }
    pub fn len(&self) -> usize {
        self.alive.len()
    }
    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }
    pub fn ids(&self) -> &[I] {
        &self.alive
    }
    pub fn get(&self, i: usize) -> Option<I> {
        self.alive.get(i).copied()
    }
    pub fn index(&self, id: I) -> Option<usize> {
        self.pos.get(&id).copied()
    }
    /// Index for a range corner: a deleted start corner moves to the next
    /// surviving id, a deleted end corner to the previous one.
    pub fn corner_index(&self, id: I, is_end: bool) -> Option<usize> {
        if let Some(i) = self.index(id) {
            return Some(i);
        }
        let t = *self.tomb.get(&id)?;
        if is_end {
            t.checked_sub(1)
        } else if t < self.alive.len() {
            Some(t)
        } else {
            None
        }
    }
    pub fn push(&mut self, id: I) {
        self.order.push(id);
        self.pos.insert(id, self.alive.len());
        self.alive.push(id);
    }
    /// Whether `id` is in the list at all, alive or deleted.
    pub fn contains(&self, id: I) -> bool {
        self.pos.contains_key(&id) || self.tomb.contains_key(&id)
    }
    /// Inserts (or revives) ids so they appear starting at visible index `at`.
    pub fn insert(&mut self, at: usize, ids: &[I]) {
        self.insert_before(self.get(at), ids);
    }
    /// Inserts (or revives) ids just in front of `before` (alive or deleted; the end if `None` or unknown).
    pub fn insert_before(&mut self, before: Option<I>, ids: &[I]) {
        for id in ids {
            self.dead.remove(id);
            self.order.retain(|x| x != id);
        }
        let slot = before.and_then(|b| self.order.iter().position(|x| *x == b)).unwrap_or(self.order.len());
        self.order.splice(slot..slot, ids.iter().copied());
        self.reindex();
    }
    /// Deletes (`dead`) or restores ids in place. Returns the ids whose state changed, in the
    /// given order: exactly what the opposite call needs to undo it.
    pub fn set_dead(&mut self, ids: &[I], dead: bool) -> Vec<I> {
        let changed: Vec<I> = ids.iter().copied().filter(|id| self.contains(*id) && self.dead.contains(id) != dead).collect();
        for id in &changed {
            if dead {
                self.dead.insert(*id);
            } else {
                self.dead.remove(id);
            }
        }
        if !changed.is_empty() {
            self.reindex();
        }
        changed
    }
    /// Each `(id, place)` puts `id` where `place` was. The ids must be a permutation of the places;
    /// otherwise nothing changes and this returns false. Undo is the same pairs swapped.
    pub fn permute(&mut self, moves: &[(I, I)]) -> bool {
        let slots: HashMap<I, usize> = self.order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let places: HashSet<I> = moves.iter().map(|m| m.1).collect();
        let ids: HashSet<I> = moves.iter().map(|m| m.0).collect();
        if places.len() != moves.len() || ids != places || !places.iter().all(|p| slots.contains_key(p)) {
            return false;
        }
        for (id, place) in moves {
            self.order[slots[place]] = *id;
        }
        self.reindex();
        true
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sheet {
    pub id: SheetId,
    pub name: String,
    pub rows: Axis<RowId>,
    pub cols: Axis<ColId>,
    #[serde(with = "cell_map")]
    pub cells: HashMap<(RowId, ColId), Cell>,
    #[serde(default)]
    pub row_heights: HashMap<RowId, f32>,
    #[serde(default)]
    pub col_widths: HashMap<ColId, f32>,
}

mod cell_map {
    use super::*;
    use serde::{Deserializer, Serializer};
    pub fn serialize<S: Serializer>(m: &HashMap<(RowId, ColId), Cell>, s: S) -> Result<S::Ok, S::Error> {
        let mut v: Vec<(&RowId, &ColId, &Cell)> = m.iter().map(|((r, c), cell)| (r, c, cell)).collect();
        v.sort_by_key(|(r, c, _)| (r.0, c.0));
        v.serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<HashMap<(RowId, ColId), Cell>, D::Error> {
        let v: Vec<(RowId, ColId, Cell)> = Vec::deserialize(d)?;
        Ok(v.into_iter().map(|(r, c, cell)| ((r, c), cell)).collect())
    }
}

impl Sheet {
    pub fn new(name: &str, rows: usize, cols: usize) -> Sheet {
        Sheet {
            id: SheetId(fresh_id()),
            name: name.to_string(),
            rows: Axis::new((0..rows).map(|_| RowId(fresh_id())).collect()),
            cols: Axis::new((0..cols).map(|_| ColId(fresh_id())).collect()),
            cells: HashMap::default(),
            row_heights: HashMap::default(),
            col_widths: HashMap::default(),
        }
    }
    pub fn reindex(&mut self) {
        self.rows.reindex();
        self.cols.reindex();
    }
    pub fn row_index(&self, r: RowId) -> Option<usize> {
        self.rows.index(r)
    }
    pub fn col_index(&self, c: ColId) -> Option<usize> {
        self.cols.index(c)
    }
    pub fn pos(&self, k: CellKey) -> Option<(usize, usize)> {
        Some((self.row_index(k.row)?, self.col_index(k.col)?))
    }
    pub fn key(&self, row: usize, col: usize) -> Option<CellKey> {
        Some(CellKey { sheet: self.id, row: self.rows.get(row)?, col: self.cols.get(col)? })
    }
    /// Grows the sheet so that (row, col) exists.
    pub fn key_grow(&mut self, row: usize, col: usize) -> CellKey {
        self.ensure_size(row + 1, col + 1);
        self.key(row, col).unwrap()
    }
    pub fn ensure_size(&mut self, rows: usize, cols: usize) {
        while self.rows.len() < rows {
            self.rows.push(RowId(fresh_id()));
        }
        while self.cols.len() < cols {
            self.cols.push(ColId(fresh_id()));
        }
    }
    pub fn cell(&self, row: RowId, col: ColId) -> Option<&Cell> {
        self.cells.get(&(row, col))
    }
    pub fn cell_at(&self, row: usize, col: usize) -> Option<&Cell> {
        let k = self.key(row, col)?;
        self.cell(k.row, k.col)
    }
    /// Whether a cell's row and column are both alive. Cells in deleted rows and columns stay in
    /// `cells`, hidden, so restoring the row brings them (and any edit made to them since) back.
    pub fn visible(&self, row: RowId, col: ColId) -> bool {
        self.rows.index(row).is_some() && self.cols.index(col).is_some()
    }
    pub fn used_extent(&self) -> (usize, usize) {
        let mut r = 0;
        let mut c = 0;
        for (rid, cid) in self.cells.keys() {
            if let (Some(ri), Some(ci)) = (self.row_index(*rid), self.col_index(*cid)) {
                r = r.max(ri + 1);
                c = c.max(ci + 1);
            }
        }
        (r, c)
    }
    /// Visible bounds of a stored range, shrinking past deleted corners.
    pub fn range_bounds(&self, a: &StoredRef, b: &StoredRef) -> Option<(usize, usize, usize, usize)> {
        // corners may be given in any order; resolve each axis independently
        let r1 = self.rows.index(a.row);
        let r2 = self.rows.index(b.row);
        let c1 = self.cols.index(a.col);
        let c2 = self.cols.index(b.col);
        let (r0, r1) = match (r1, r2) {
            (Some(x), Some(y)) => (x.min(y), x.max(y)),
            _ => {
                // order unknown for dead corners: assume a is the start corner
                (self.rows.corner_index(a.row, false)?, self.rows.corner_index(b.row, true)?)
            }
        };
        let (c0, c1) = match (c1, c2) {
            (Some(x), Some(y)) => (x.min(y), x.max(y)),
            _ => (self.cols.corner_index(a.col, false)?, self.cols.corner_index(b.col, true)?),
        };
        if r0 > r1 || c0 > c1 {
            return None;
        }
        Some((r0, c0, r1, c1))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NameDef {
    pub cell: CellKey,
    pub input: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workbook {
    /// The visible sheets, in tab order.
    pub sheets: Vec<Sheet>,
    pub names: BTreeMap<String, NameDef>,
    /// Deleted sheets, kept whole (like deleted rows) so restoring one brings back its cells and
    /// any edit made to them since. Nothing outside the edit code looks here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_sheets: Vec<Sheet>,
    /// Per-workbook settings as stored (see `settings`): unknown keys and invalid values are kept as they are.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, serde_json::Value>,
}

impl Workbook {
    pub fn empty() -> Workbook {
        Workbook { sheets: Vec::new(), names: BTreeMap::new(), deleted_sheets: Vec::new(), settings: BTreeMap::new() }
    }
    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.id == id)
    }
    pub fn sheet_mut(&mut self, id: SheetId) -> Option<&mut Sheet> {
        self.sheets.iter_mut().find(|s| s.id == id)
    }
    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.name == name)
    }
    /// Position of a sheet in the tab order.
    pub fn sheet_index(&self, id: SheetId) -> Option<usize> {
        self.sheets.iter().position(|s| s.id == id)
    }
    /// A sheet name must be non-empty, on one line, and not used by another sheet. Returns it trimmed.
    pub fn check_sheet_name(&self, name: &str, renaming: Option<SheetId>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a sheet needs a name".into());
        }
        if name.chars().any(char::is_control) {
            return Err("sheet names are a single line".into());
        }
        match self.sheet_by_name(name) {
            Some(s) if Some(s.id) != renaming => Err(format!("there is already a sheet named {name}")),
            _ => Ok(name.to_string()),
        }
    }
    /// `base`, or `base 2`, `base 3`… whichever is free.
    pub fn free_sheet_name(&self, base: &str) -> String {
        let mut name = base.to_string();
        let mut n = 2;
        while self.sheet_by_name(&name).is_some() {
            name = format!("{base} {n}");
            n += 1;
        }
        name
    }
    pub fn cell(&self, k: CellKey) -> Option<&Cell> {
        self.sheet(k.sheet)?.cell(k.row, k.col)
    }
    pub fn pos(&self, k: CellKey) -> Option<(usize, usize)> {
        self.sheet(k.sheet)?.pos(k)
    }
    pub fn after_load(&mut self) {
        for s in self.sheets.iter_mut().chain(&mut self.deleted_sheets) {
            s.reindex();
        }
    }

    /// A1 name of a cell, with sheet prefix if it differs from `from`.
    pub fn cell_label(&self, k: CellKey, from: Option<SheetId>) -> String {
        let Some(sheet) = self.sheet(k.sheet) else { return "?".into() };
        let Some((r, c)) = sheet.pos(k) else { return "#ref!".into() };
        let base = a1::cell_name(r, c);
        if from == Some(k.sheet) {
            base
        } else {
            format!("{}{}", a1::sheet_prefix(&sheet.name), base)
        }
    }

    // ---- text <-> pieces -------------------------------------------------

    pub fn cell_text(&self, k: CellKey) -> String {
        match self.cell(k) {
            Some(c) => self.render(&c.pieces, k.sheet),
            None => String::new(),
        }
    }

    pub fn render(&self, pieces: &[Piece], home: SheetId) -> String {
        let mut s = String::new();
        for p in pieces {
            match p {
                Piece::Text(t) => s.push_str(t),
                Piece::Ref(r) => s.push_str(&self.render_ref(r, None, home)),
                Piece::Range(a, b) => s.push_str(&self.render_range(a, b, home)),
            }
        }
        s
    }

    pub fn render_range(&self, a: &StoredRef, b: &StoredRef, home: SheetId) -> String {
        let sid = a.sheet.unwrap_or(home);
        let Some(sheet) = self.sheet(sid) else { return "#ref!".into() };
        let Some((r0, c0, r1, c1)) = sheet.range_bounds(a, b) else { return "#ref!".into() };
        let first = a1::format_ref(&A1Ref {
            sheet: a.sheet.map(|_| sheet.name.clone()),
            col: c0,
            row: r0,
            col_abs: a.col_abs,
            row_abs: a.row_abs,
        });
        let second = a1::format_ref(&A1Ref { sheet: None, col: c1, row: r1, col_abs: b.col_abs, row_abs: b.row_abs });
        format!("{first}:{second}")
    }

    fn render_ref(&self, r: &StoredRef, force_sheet: Option<SheetId>, home: SheetId) -> String {
        let sid = force_sheet.or(r.sheet).unwrap_or(home);
        let Some(sheet) = self.sheet(sid) else { return "#ref!".into() };
        let (Some(row), Some(col)) = (sheet.row_index(r.row), sheet.col_index(r.col)) else {
            return "#ref!".into();
        };
        let show_sheet = force_sheet.is_none() && r.sheet.is_some();
        a1::format_ref(&A1Ref {
            sheet: if show_sheet { Some(sheet.name.clone()) } else { None },
            col,
            row,
            col_abs: r.col_abs,
            row_abs: r.row_abs,
        })
    }

    fn resolve_a1(&mut self, r: &A1Ref, home: SheetId, force_sheet: Option<SheetId>) -> Option<StoredRef> {
        let (explicit, sid) = match (&r.sheet, force_sheet) {
            (_, Some(f)) => (None, f),
            (Some(name), None) => {
                let s = self.sheet_by_name(name)?;
                (Some(s.id), s.id)
            }
            (None, None) => (None, home),
        };
        let sheet = self.sheet_mut(sid)?;
        let k = sheet.key_grow(r.row, r.col);
        Some(StoredRef { sheet: explicit, row: k.row, col: k.col, row_abs: r.row_abs, col_abs: r.col_abs })
    }

    /// Converts typed text into stored pieces, resolving A1 references to ids.
    /// References to unknown sheets stay as text and fail at compile time.
    pub fn parse_text(&mut self, text: &str, home: SheetId) -> Vec<Piece> {
        if !classify(text).has_refs() {
            return vec![Piece::Text(text.to_string())];
        }
        let mut pieces = Vec::new();
        let mut last = 0;
        for t in lex::lex(text) {
            let piece = match &t.tok {
                Tok::Ref(r) => self.resolve_a1(r, home, None).map(Piece::Ref),
                // a trailing `?` is outside the token's span, so it stays as text
                Tok::Range(a, b, _) => {
                    let ra = self.resolve_a1(a, home, None);
                    let sid = ra.and_then(|x| x.sheet).unwrap_or(home);
                    let rb = self.resolve_a1(b, home, Some(sid));
                    match (ra, rb) {
                        (Some(ra), Some(rb)) => Some(Piece::Range(ra, rb)),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(p) = piece {
                if t.span.start > last {
                    pieces.push(Piece::Text(text[last..t.span.start].to_string()));
                }
                pieces.push(p);
                last = t.span.end;
            }
        }
        if last < text.len() {
            pieces.push(Piece::Text(text[last..].to_string()));
        }
        pieces
    }

    /// Copy semantics: relative axes of every reference move by the offset
    /// between `src` and `dst`; absolute axes stay put.
    pub fn shift_pieces(&mut self, pieces: &[Piece], src: CellKey, dst: CellKey) -> Vec<Piece> {
        let (Some((sr, sc)), Some((dr, dc))) = (self.pos(src), self.pos(dst)) else {
            return pieces.to_vec();
        };
        let (drow, dcol) = (dr as i64 - sr as i64, dc as i64 - sc as i64);
        pieces
            .iter()
            .map(|p| match p {
                Piece::Text(t) => Piece::Text(t.clone()),
                Piece::Ref(r) => match self.shift_ref(r, src.sheet, dst.sheet, drow, dcol, None) {
                    Some(r) => Piece::Ref(r),
                    None => Piece::Text("#ref!".into()),
                },
                Piece::Range(a, b) => {
                    let corner_sheet = a.sheet.unwrap_or(src.sheet);
                    let bounds = self.sheet(corner_sheet).and_then(|s| {
                        let (r0, c0, r1, c1) = s.range_bounds(a, b)?;
                        let ka = s.key(r0, c0)?;
                        let kb = s.key(r1, c1)?;
                        Some((
                            StoredRef { row: ka.row, col: ka.col, ..*a },
                            StoredRef { row: kb.row, col: kb.col, ..*b },
                        ))
                    });
                    let shifted = bounds.and_then(|(a, b)| {
                        let na = self.shift_ref(&a, src.sheet, dst.sheet, drow, dcol, None)?;
                        let nb = self.shift_ref(&b, src.sheet, dst.sheet, drow, dcol, Some(corner_sheet))?;
                        Some(Piece::Range(na, nb))
                    });
                    shifted.unwrap_or_else(|| Piece::Text("#ref!".into()))
                }
            })
            .collect()
    }

    fn shift_ref(
        &mut self,
        r: &StoredRef,
        src_home: SheetId,
        dst_home: SheetId,
        drow: i64,
        dcol: i64,
        corner_sheet: Option<SheetId>,
    ) -> Option<StoredRef> {
        let from_sheet = corner_sheet.or(r.sheet).unwrap_or(src_home);
        let to_sheet = corner_sheet.or(r.sheet).unwrap_or(dst_home);
        let s = self.sheet(from_sheet)?;
        let (row, col) = (s.row_index(r.row)? as i64, s.col_index(r.col)? as i64);
        let nr = if r.row_abs { row } else { row + drow };
        let nc = if r.col_abs { col } else { col + dcol };
        if nr < 0 || nc < 0 {
            return None;
        }
        let sheet = self.sheet_mut(to_sheet)?;
        let k = sheet.key_grow(nr as usize, nc as usize);
        Some(StoredRef { row: k.row, col: k.col, ..*r })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb() -> (Workbook, SheetId) {
        let mut wb = Workbook::empty();
        let s = Sheet::new("Sheet1", 20, 10);
        let id = s.id;
        wb.sheets.push(s);
        (wb, id)
    }

    #[test]
    fn classify_rules() {
        assert_eq!(classify("5"), Kind::Number);
        assert_eq!(classify("5 [m/s]"), Kind::Number);
        assert_eq!(classify("2026-10-08"), Kind::Number);
        assert_eq!(classify("=A1 1 +"), Kind::Program);
        assert_eq!(classify(": sq dup * ;"), Kind::WordDef);
        assert_eq!(classify("dim widgets"), Kind::UnitDecl);
        assert_eq!(classify("base [USD] currency"), Kind::UnitDecl);
        assert_eq!(classify("[mi] = 1609.344 [m]"), Kind::UnitDecl);
        assert_eq!(classify("dim sum place"), Kind::Text);
        assert_eq!(classify("hello"), Kind::Text);
        assert_eq!(classify("'5"), Kind::Text);
    }

    #[test]
    fn refs_survive_inserts() {
        let (mut wb, sid) = wb();
        let pieces = wb.parse_text("=B3 $C$4 + A1:A3 sum", sid);
        assert_eq!(wb.render(&pieces, sid), "=B3 $C$4 + A1:A3 sum");
        wb.sheet_mut(sid).unwrap().rows.insert(1, &[RowId(1), RowId(2)]);
        assert_eq!(wb.render(&pieces, sid), "=B5 $C$6 + A1:A5 sum");
        let row = wb.sheet(sid).unwrap().rows.get(4).unwrap();
        wb.sheet_mut(sid).unwrap().rows.set_dead(&[row], true); // the row holding B3 and the range's end
        assert_eq!(wb.render(&pieces, sid), "=#ref! $C$5 + A1:A4 sum");

    }

    #[test]
    fn shift() {
        let (mut wb, sid) = wb();
        let src = wb.sheet(sid).unwrap().key(0, 2).unwrap();
        let dst = wb.sheet(sid).unwrap().key(3, 3).unwrap();
        let pieces = wb.parse_text("=A1 $B1 B$1 $B$1", sid);
        let shifted = wb.shift_pieces(&pieces, src, dst);
        assert_eq!(wb.render(&shifted, sid), "=B4 $B4 C$1 $B$1");
        let up = wb.sheet(sid).unwrap().key(0, 0).unwrap();
        let s2 = wb.shift_pieces(&pieces, src, up);
        assert_eq!(wb.render(&s2, sid), "=#ref! $B1 #ref! $B$1");
    }
}
