//! Document model. Row/column order are maps from id to position key, cells are
//! a map keyed by ids, formulas store references as ids. Nothing here assumes
//! positions are stable, so the model maps onto Automerge maps directly.

use crate::a1::{self, A1Ref};
use crate::ids::*;
use crate::lex::{self, Tok};
use crate::poskey;
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

/// Rows and columns are addressable up to here (`get` and `index` stop): an A1 reference can name
/// any row, and one past this is a typo, not a row.
pub const MAX_INDEX: usize = 1 << 24;

/// The rows (or columns) of a sheet: a map from id to position key (`poskey`), in order of
/// `(key, id)`, with tombstones. Deleted ids keep their key, so a range whose end row was deleted
/// can shrink instead of breaking, and undo (or someone else's edit) can revive them.
///
/// Past the stored ("materialised") ids come virtual ones, without end: virtual row k has the id
/// `mix(base + k)` (`base` comes from the sheet's seed) and the key `t` + k, so everyone agrees on
/// it without storing anything. The visible order is the alive materialised ids by key, then the
/// virtual rows by k.
///
/// A row is materialised when its place has to be written down: it was inserted, deleted (a
/// tombstone), sorted (a fresh key), or rows were inserted in front of it. Materialising virtual
/// row k stores rows 0..=k with their canonical keys, so stored keys always sort before virtual
/// ones, and two people doing it at once write the same thing. Holding a cell, being referenced
/// or resized doesn't materialise a row: a virtual id gives back its k (`ids::unmix`), so those
/// find their row without a stored key, and scrolling, spills and goal-seek write nothing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "AxisFile<I>", into = "AxisFile<I>", bound(serialize = "I: Serialize", deserialize = "I: Deserialize<'de>"))]
pub struct Axis<I: AxisId> {
    keys: HashMap<I, String>,
    dead: HashSet<I>,
    /// Virtual row k is `mix(base + k)`.
    base: u64,
    /// Virtual rows `0..filled` are materialised.
    filled: usize,
    // derived by `reindex`
    /// Every materialised id by (key, id), tombstones included.
    order: Vec<I>,
    alive: Vec<I>,
    pos: HashMap<I, usize>,
    /// For dead ids: number of alive ids before it.
    tomb: HashMap<I, usize>,
}

/// An axis as saved: ids with their keys, in order. Files saved before position keys have `order`
/// instead (random ids, tombstones included), keyed `m` + index when read, so they keep their
/// places and come before every virtual row.
#[derive(Serialize, Deserialize)]
#[serde(bound(serialize = "I: Serialize", deserialize = "I: Deserialize<'de>"))]
struct AxisFile<I> {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    order: Vec<I>,
    #[serde(default)]
    keys: Vec<(I, String)>,
    #[serde(default)]
    dead: Vec<I>,
}

impl<I: AxisId> From<AxisFile<I>> for Axis<I> {
    fn from(f: AxisFile<I>) -> Self {
        let mut a = Axis::new(0);
        a.keys = f.order.iter().enumerate().map(|(i, id)| (*id, poskey::fixed('m', i))).collect();
        a.keys.extend(f.keys);
        a.dead = f.dead.into_iter().filter(|id| a.keys.contains_key(id)).collect();
        a.reindex();
        a
    }
}

impl<I: AxisId> From<Axis<I>> for AxisFile<I> {
    fn from(a: Axis<I>) -> Self {
        let dead = a.order.iter().filter(|id| a.dead.contains(id)).copied().collect();
        AxisFile { order: vec![], keys: a.order.iter().map(|id| (*id, a.keys[id].clone())).collect(), dead }
    }
}

impl<I: AxisId> Axis<I> {
    /// Nothing stored: every row is virtual.
    pub fn new(seed: u64) -> Self {
        Axis {
            keys: HashMap::default(),
            dead: HashSet::default(),
            base: mix(seed ^ I::TAG),
            filled: 0,
            order: vec![],
            alive: vec![],
            pos: HashMap::default(),
            tomb: HashMap::default(),
        }
    }
    /// The seed the virtual ids come from (the sheet's).
    pub fn set_seed(&mut self, seed: u64) {
        self.base = mix(seed ^ I::TAG);
        self.reindex();
    }
    fn virtual_id(&self, k: usize) -> I {
        I::from_raw(mix(self.base.wrapping_add(k as u64)))
    }
    /// Which virtual row an id is, if it is one (stored or not).
    fn virtual_k(&self, id: I) -> Option<usize> {
        let k = unmix(id.raw()).wrapping_sub(self.base);
        (k < MAX_INDEX as u64).then_some(k as usize)
    }
    pub fn reindex(&mut self) {
        // materialising row k stores rows 0..=k: fill any gap, so virtual rows come after them all
        match self.keys.keys().filter_map(|id| self.virtual_k(*id)).max() {
            Some(top) => {
                for k in 0..=top {
                    let id = self.virtual_id(k);
                    self.keys.entry(id).or_insert_with(|| poskey::fixed('t', k));
                }
                self.filled = top + 1;
            }
            None => self.filled = 0,
        }
        let keys = &self.keys;
        self.order = keys.keys().copied().collect();
        self.order.sort_unstable_by(|a, b| keys[a].cmp(&keys[b]).then(a.cmp(b)));
        self.recount();
    }
    fn recount(&mut self) {
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
    /// The alive materialised ids; rows past these are virtual (`get` still has them).
    pub fn len(&self) -> usize {
        self.alive.len()
    }
    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }
    pub fn ids(&self) -> &[I] {
        &self.alive
    }
    /// How many virtual rows are materialised (they are rows `0..filled` of the virtual ones).
    pub fn filled(&self) -> usize {
        self.filled
    }
    /// Every materialised id with its key, in order, tombstones included.
    pub fn keys(&self) -> impl Iterator<Item = (I, &str)> + '_ {
        self.order.iter().map(|id| (*id, self.keys[id].as_str()))
    }
    /// An id's position key: stored, or a virtual row's canonical one.
    pub fn pos_key(&self, id: I) -> Option<String> {
        match self.keys.get(&id) {
            Some(k) => Some(k.clone()),
            None => self.virtual_k(id).map(|k| poskey::fixed('t', k)),
        }
    }
    pub fn is_dead(&self, id: I) -> bool {
        self.dead.contains(&id)
    }
    /// The id at visible index `i`: a materialised one, or virtual past those.
    pub fn get(&self, i: usize) -> Option<I> {
        match self.alive.get(i) {
            Some(id) => Some(*id),
            None => {
                let k = self.filled + (i - self.alive.len());
                (k < MAX_INDEX).then(|| self.virtual_id(k))
            }
        }
    }
    pub fn index(&self, id: I) -> Option<usize> {
        if let Some(i) = self.pos.get(&id) {
            return Some(*i);
        }
        let k = self.virtual_k(id).filter(|k| *k >= self.filled)?;
        Some(self.alive.len() + k - self.filled)
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
        } else {
            // past the materialised rows comes the first virtual one
            Some(t)
        }
    }
    /// Whether `id` is in the axis at all: materialised (alive or deleted) or virtual.
    pub fn contains(&self, id: I) -> bool {
        self.keys.contains_key(&id) || self.index(id).is_some()
    }
    /// Materialises the virtual ones among `ids`, and every virtual row before them.
    pub fn materialise(&mut self, ids: &[I]) {
        if let Some(k) = ids.iter().filter(|id| !self.keys.contains_key(id)).filter_map(|id| self.virtual_k(*id)).max() {
            self.fill(k + 1);
        }
    }
    /// Materialises virtual rows until `n` are, or drops them back to `n`; returns the old count.
    /// Dropping stops at a row that changed since (moved or deleted) or that another stored row
    /// comes after (an inserted row, deleted again by undo): those keep their keys.
    pub fn fill(&mut self, n: usize) -> usize {
        let old = self.filled;
        while self.filled < n.min(MAX_INDEX) {
            let id = self.virtual_id(self.filled);
            self.keys.insert(id, poskey::fixed('t', self.filled));
            self.order.push(id);
            self.filled += 1;
        }
        while self.filled > n {
            let (k, id) = (self.filled - 1, self.virtual_id(self.filled - 1));
            if self.order.last() != Some(&id) || self.dead.contains(&id) || self.keys[&id] != poskey::fixed('t', k) {
                break;
            }
            self.order.pop();
            self.keys.remove(&id);
            self.filled -= 1;
        }
        if self.filled != old {
            self.recount();
        }
        old
    }
    /// Inserts (or revives) ids so they appear starting at visible index `at`.
    pub fn insert(&mut self, at: usize, ids: &[I]) {
        self.insert_before(self.get(at), ids);
    }
    /// Inserts (or revives) ids just in front of `before` (alive, deleted or virtual; after the
    /// materialised rows if `None` or unknown), with keys between its predecessor's and its own.
    pub fn insert_before(&mut self, before: Option<I>, ids: &[I]) {
        if let Some(b) = before {
            self.materialise(&[b]);
        }
        let moving: HashSet<I> = ids.iter().copied().collect();
        for id in ids {
            self.dead.remove(id);
        }
        self.order.retain(|x| !moving.contains(x));
        let slot = before.and_then(|b| self.order.iter().position(|x| *x == b)).unwrap_or(self.order.len());
        let lo = slot.checked_sub(1).map_or(String::new(), |i| self.keys[&self.order[i]].clone());
        let hi = self.order.get(slot).map_or_else(|| poskey::fixed('t', self.filled), |b| self.keys[b].clone());
        for (id, k) in ids.iter().zip(poskey::run(&lo, Some(&hi), ids.len())) {
            self.keys.insert(*id, k);
        }
        self.order.splice(slot..slot, ids.iter().copied());
        self.recount();
    }
    /// Deletes (`dead`) or restores ids in place (deleting a virtual row materialises it). Returns
    /// the ids whose state changed, in the given order: exactly what the opposite call needs to undo it.
    pub fn set_dead(&mut self, ids: &[I], dead: bool) -> Vec<I> {
        if dead {
            self.materialise(ids);
        }
        let changed: Vec<I> = ids.iter().copied().filter(|id| self.keys.contains_key(id) && self.dead.contains(id) != dead).collect();
        for id in &changed {
            if dead {
                self.dead.insert(*id);
            } else {
                self.dead.remove(id);
            }
        }
        if !changed.is_empty() {
            self.recount();
        }
        changed
    }
    /// Each `(id, place)` puts `id` where `place` was. The ids must be a permutation of the places;
    /// otherwise nothing changes and this returns `None`. Every row from the first place to the
    /// last, tombstones included, gets a fresh key under one random prefix (§2.2), so concurrent
    /// sorts can only ever give a permutation. Returns their old keys, which undo writes back.
    pub fn permute(&mut self, moves: &[(I, I)]) -> Option<Vec<(I, String)>> {
        let places: Vec<I> = moves.iter().map(|m| m.1).collect();
        let place_set: HashSet<I> = places.iter().copied().collect();
        let ids: HashSet<I> = moves.iter().map(|m| m.0).collect();
        if place_set.len() != moves.len() || ids != place_set || !places.iter().all(|p| self.contains(*p)) {
            return None;
        }
        if moves.is_empty() {
            return Some(vec![]);
        }
        self.materialise(&places);
        let slot: HashMap<I, usize> = self.order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let s0 = places.iter().map(|p| slot[p]).min().unwrap();
        let s1 = places.iter().map(|p| slot[p]).max().unwrap();
        let to: HashMap<I, I> = moves.iter().map(|(id, place)| (*place, *id)).collect();
        let lo = s0.checked_sub(1).map_or(String::new(), |i| self.keys[&self.order[i]].clone());
        let hi = self.order.get(s1 + 1).map_or_else(|| poskey::fixed('t', self.filled), |x| self.keys[x].clone());
        let old: Vec<(I, String)> = self.order[s0..=s1].iter().map(|id| (*id, self.keys[id].clone())).collect();
        for (s, key) in (s0..=s1).zip(poskey::run(&lo, Some(&hi), s1 - s0 + 1)) {
            let x = self.order[s];
            self.order[s] = to.get(&x).copied().unwrap_or(x);
            self.keys.insert(self.order[s], key);
        }
        self.recount();
        Some(old)
    }
    /// Puts ids at the given keys (a sort's undo and redo). Returns the keys they had.
    pub fn set_keys(&mut self, keys: &[(I, String)]) -> Vec<(I, String)> {
        let ids: Vec<I> = keys.iter().map(|k| k.0).collect();
        self.materialise(&ids);
        let mut old = Vec::with_capacity(keys.len());
        for (id, k) in keys {
            if let Some(was) = self.keys.get_mut(id) {
                old.push((*id, std::mem::replace(was, k.clone())));
            }
        }
        self.reindex();
        old
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sheet {
    pub id: SheetId,
    pub name: String,
    /// Tab order: a position key, like a row's. Set by the edits that place a sheet (`Engine`).
    #[serde(default)]
    pub pos: String,
    /// Where the ids of the virtual rows and columns come from (`Axis`). A copy of the sheet keeps
    /// it, as it keeps the stored ids. `0` (files saved before seeds): the sheet id.
    #[serde(default)]
    pub seed: u64,
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
    /// A new sheet stores no rows or columns: they are all virtual.
    pub fn new(name: &str) -> Sheet {
        let seed = fresh_id();
        Sheet {
            id: SheetId(fresh_id()),
            name: name.to_string(),
            pos: String::new(),
            seed,
            rows: Axis::new(seed),
            cols: Axis::new(seed),
            cells: HashMap::default(),
            row_heights: HashMap::default(),
            col_widths: HashMap::default(),
        }
    }
    pub fn reindex(&mut self) {
        if self.seed == 0 {
            self.seed = self.id.0;
        }
        self.rows.set_seed(self.seed);
        self.cols.set_seed(self.seed);
    }
    /// How many virtual rows and columns are materialised (`Edit::Materialise`).
    pub fn filled(&self) -> (usize, usize) {
        (self.rows.filled(), self.cols.filled())
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
    /// An input's range: each end a number in the input's unit (`0 [1/s]`) or a formula (`=B7`,
    /// `=max_damping`), stored like a cell so its references are ids; see `bounds`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Cell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Cell>,
}

impl NameDef {
    pub fn new(cell: CellKey, input: bool) -> NameDef {
        NameDef { cell, input, min: None, max: None }
    }
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
        // files saved before tab keys (and sheets built in code) get `m` + their place, in order
        if self.sheets.iter().chain(&self.deleted_sheets).any(|s| s.pos.is_empty()) {
            for (i, s) in self.sheets.iter_mut().chain(&mut self.deleted_sheets).enumerate() {
                s.pos = poskey::fixed('m', i);
            }
        }
        self.sheets.sort_by(|a, b| (&a.pos, a.id).cmp(&(&b.pos, b.id)));
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

    fn resolve_a1(&self, r: &A1Ref, home: SheetId, force_sheet: Option<SheetId>) -> Option<StoredRef> {
        let (explicit, sid) = match (&r.sheet, force_sheet) {
            (_, Some(f)) => (None, f),
            (Some(name), None) => {
                let s = self.sheet_by_name(name)?;
                (Some(s.id), s.id)
            }
            (None, None) => (None, home),
        };
        // a row past the stored ones is virtual: its id is known without writing anything
        let k = self.sheet(sid)?.key(r.row, r.col)?;
        Some(StoredRef { sheet: explicit, row: k.row, col: k.col, row_abs: r.row_abs, col_abs: r.col_abs })
    }

    /// Converts typed text into stored pieces, resolving A1 references to ids.
    /// References to unknown sheets (or past `MAX_INDEX`) stay as text and fail at compile time.
    pub fn parse_text(&self, text: &str, home: SheetId) -> Vec<Piece> {
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
    pub fn shift_pieces(&self, pieces: &[Piece], src: CellKey, dst: CellKey) -> Vec<Piece> {
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
        &self,
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
        let k = self.sheet(to_sheet)?.key(nr as usize, nc as usize)?;
        Some(StoredRef { row: k.row, col: k.col, ..*r })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb() -> (Workbook, SheetId) {
        let mut wb = Workbook::empty();
        let s = Sheet::new("Sheet1");
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
        let (wb, sid) = wb();
        let src = wb.sheet(sid).unwrap().key(0, 2).unwrap();
        let dst = wb.sheet(sid).unwrap().key(3, 3).unwrap();
        let pieces = wb.parse_text("=A1 $B1 B$1 $B$1", sid);
        let shifted = wb.shift_pieces(&pieces, src, dst);
        assert_eq!(wb.render(&shifted, sid), "=B4 $B4 C$1 $B$1");
        let up = wb.sheet(sid).unwrap().key(0, 0).unwrap();
        let s2 = wb.shift_pieces(&pieces, src, up);
        assert_eq!(wb.render(&s2, sid), "=#ref! $B1 #ref! $B$1");
    }

    fn axis() -> Axis<RowId> {
        Axis::new(42)
    }

    /// Keys in order, with every id at the index it says it has.
    fn check(a: &Axis<RowId>) -> Vec<String> {
        let keys: Vec<(RowId, String)> = a.keys().map(|(id, k)| (id, k.to_string())).collect();
        for w in keys.windows(2) {
            assert!((&w[0].1, w[0].0) < (&w[1].1, w[1].0), "{keys:?}");
        }
        assert!(keys.iter().all(|(_, k)| *k < poskey::fixed('t', a.filled())), "stored keys come before the virtual ones");
        for i in 0..a.len() + 50 {
            assert_eq!(a.index(a.get(i).unwrap()), Some(i));
        }
        keys.into_iter().map(|k| k.1).collect()
    }

    #[test]
    fn virtual_rows_need_nothing_stored() {
        let a = axis();
        assert_eq!((a.len(), a.filled()), (0, 0));
        check(&a);
        let far = a.get(5000).unwrap();
        assert_eq!(a.index(far), Some(5000));
        assert_eq!(a.pos_key(far), Some(poskey::fixed('t', 5000)));
        assert_eq!(a.get(MAX_INDEX), None);
        // the same seed gives the same ids (a duplicated sheet, or another peer)
        let b = Axis::<RowId>::new(7);
        assert_eq!(Axis::<RowId>::new(7).get(123), b.get(123));
        assert_ne!(Axis::<ColId>::new(7).get(123).map(|c| c.0), b.get(123).map(|r| r.0), "rows and columns differ");
        // an id from elsewhere isn't a row here
        assert_eq!(a.index(RowId(fresh_id())), None);
    }

    #[test]
    fn inserting_materialises_up_to_the_anchor() {
        let mut a = axis();
        let (v3, v4) = (a.get(3).unwrap(), a.get(4).unwrap());
        a.insert(4, &[RowId(1), RowId(2)]);
        assert_eq!(a.filled(), 5, "virtual rows 0..=4 are stored with their canonical keys");
        assert_eq!(a.ids()[3..], [v3, RowId(1), RowId(2), v4]);
        let keys = check(&a);
        assert_eq!(keys[0], poskey::fixed('t', 0));
        assert!(poskey::fixed('t', 3) < keys[4] && keys[5] < poskey::fixed('t', 4));
        // past the stored rows: the virtual ones carry on where they were
        assert_eq!(a.get(10), Axis::<RowId>::new(42).get(8));
        assert_eq!(a.index(a.get(100).unwrap()), Some(100));
        // inserting further down, in front of a virtual row
        let v20 = a.get(20).unwrap();
        a.insert(20, &[RowId(3)]);
        assert_eq!((a.get(20), a.get(21)), (Some(RowId(3)), Some(v20)));
        check(&a);
    }

    #[test]
    fn delete_and_restore_keep_keys() {
        let mut a = axis();
        let ids: Vec<RowId> = (5..8).map(|i| a.get(i).unwrap()).collect();
        let after = a.get(8).unwrap();
        assert_eq!(a.set_dead(&ids, true), ids);
        assert_eq!(a.filled(), 8);
        assert_eq!(a.get(5), Some(after));
        let keys = check(&a);
        assert_eq!(keys.len(), 8, "tombstones keep their keys");
        assert_eq!(a.set_dead(&ids, false), ids);
        assert_eq!(a.get(5), Some(ids[0]));
        assert_eq!(check(&a), keys);
        // nothing changed since: dropping the stored rows gives the empty axis back
        assert_eq!(a.fill(0), 8);
        assert_eq!((a.len(), a.filled()), (0, 0));
        assert_eq!(a.get(5), Some(ids[0]));
    }

    #[test]
    fn dropping_stored_rows_stops_at_one_that_changed() {
        let mut a = axis();
        a.insert(3, &[RowId(9)]);
        a.set_dead(&[RowId(9)], true);
        // the tombstone sits between virtual rows 2 and 3, so rows 0..=2 stay stored
        assert_eq!(a.fill(0), 4);
        assert_eq!(a.filled(), 3);
        check(&a);
        let v1 = a.get(1).unwrap();
        a.set_dead(&[v1], true);
        a.fill(0);
        assert_eq!(a.filled(), 3);
    }

    #[test]
    fn sort_writes_fresh_keys_and_returns_the_old_ones() {
        let mut a = axis();
        a.insert(2, &[RowId(1)]);
        let rows: Vec<RowId> = (0..6).map(|i| a.get(i).unwrap()).collect();
        a.set_dead(&[rows[3]], true);
        let before = check(&a);
        let filled = a.filled();
        // reverse rows 1, 2, 4, 5 (row 3 is a tombstone between them)
        let places = [rows[1], rows[2], rows[4], rows[5]];
        let moves: Vec<(RowId, RowId)> = places.iter().rev().copied().zip(places).collect();
        let old = a.permute(&moves).unwrap();
        assert_eq!(a.ids()[..5], [rows[0], rows[5], rows[4], rows[2], rows[1]]);
        assert_eq!(a.corner_index(rows[3], false), Some(3), "the tombstone keeps its slot");
        assert_eq!(old.len(), 5, "every row from the first place to the last, tombstone included");
        let after = check(&a);
        assert_eq!(after[0], before[0], "rows outside the block keep their keys");
        assert!(after[1..6].iter().all(|k| !before.contains(k)), "fresh keys: {before:?} {after:?}");
        assert!(a.permute(&[(rows[0], rows[1])]).is_none(), "not a permutation");
        // undo: the old keys, then the rows the sort materialised go again (`Edit::Materialise`)
        a.set_keys(&old);
        a.fill(filled);
        assert_eq!(check(&a), before);
        assert_eq!((0..5).map(|i| a.get(i).unwrap()).collect::<Vec<_>>(), [rows[0], rows[1], rows[2], rows[4], rows[5]]);
    }

    #[test]
    fn files_before_keys_keep_their_order() {
        let json = r#"{"order":[5,3,9,4],"dead":[9]}"#;
        let mut a: Axis<RowId> = serde_json::from_str(json).unwrap();
        a.set_seed(1);
        assert_eq!(a.ids(), [RowId(5), RowId(3), RowId(4)]);
        assert_eq!(a.corner_index(RowId(9), true), Some(1));
        assert_eq!(check(&a), ["m000000", "m000001", "m000002", "m000003"]);
        // virtual rows come after them
        assert_eq!(a.index(Axis::<RowId>::new(1).get(0).unwrap()), Some(3));
        // and saving writes keys, which load back the same
        let saved = serde_json::to_string(&a).unwrap();
        assert!(saved.contains("m000002") && !saved.contains("order"), "{saved}");
        let mut b: Axis<RowId> = serde_json::from_str(&saved).unwrap();
        b.set_seed(1);
        assert_eq!(b.ids(), a.ids());
        assert_eq!(check(&b), check(&a));
    }
}
