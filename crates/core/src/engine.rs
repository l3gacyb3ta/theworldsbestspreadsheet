//! The dependency graph and recalculation. Every cell with content is a node;
//! its dependencies come from the references, names, units and words its
//! program resolved to. Edits mark downstream cells dirty and only those are
//! recomputed, in dependency order. Spill regions are part of the graph:
//! a reference into a spilled cell depends on the spill's source.

use crate::dims::{self, Halt, SVal, StaticEnv, StaticTrace};
use crate::eval::{run_program, run_traced, Env, EvalErr, TraceStep};
use crate::ids::*;
use crate::model::{classify, Cell, Kind, NameDef, Piece, Sheet, StoredRef, Workbook};
use crate::parse::{declares, CompileError, Compiled, Compiler, Declares, Dep, OpKind, Symbols};
use crate::units::{Dim, Quant, UnitInfo};
use crate::value::{Num, Prov, Text, Value};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum ErrKind {
    /// The problem is in this cell; `span` points at the token if known.
    Local,
    /// A referenced cell has an error.
    Upstream(CellKey),
    /// The cell is part of this cycle.
    Cycle(Vec<CellKey>),
    /// The spill would overwrite this cell.
    SpillBlocked(CellKey),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellError {
    pub msg: String,
    pub span: Option<Range<usize>>,
    pub kind: ErrKind,
}

impl CellError {
    pub fn short(&self) -> &'static str {
        match self.kind {
            ErrKind::Local => "#err",
            ErrKind::Upstream(_) => "#upstream",
            ErrKind::Cycle(_) => "#cycle",
            ErrKind::SpillBlocked(_) => "#spill blocked",
        }
    }
}

pub type CellResult = Result<Value, CellError>;

/// A read-only evaluation for the playground and step-through views.
pub struct Scratch {
    pub steps: Vec<TraceStep>,
    pub result: CellResult,
}

struct Node {
    kind: Kind,
    compiled: Result<Arc<Compiled>, CompileError>,
    deps: Vec<Dep>,
}

/// A node's result from the static dimension pass (`dims`).
struct Static {
    /// What the cell holds (for a unit declaration: its defining quantity).
    val: SVal,
    err: Option<EvalErr>,
}

impl Static {
    fn any() -> Static {
        Static { val: SVal::Any, err: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Region {
    sheet: SheetId,
    r0: usize,
    c0: usize,
    rows: usize,
    cols: usize,
}

impl Region {
    fn contains(&self, sheet: SheetId, r: usize, c: usize) -> bool {
        sheet == self.sheet && r >= self.r0 && r < self.r0 + self.rows && c >= self.c0 && c < self.c0 + self.cols
    }
    fn overlaps(&self, o: &Region) -> bool {
        self.sheet == o.sheet && self.r0 < o.r0 + o.rows && o.r0 < self.r0 + self.rows && self.c0 < o.c0 + o.cols && o.c0 < self.c0 + self.cols
    }
}

/// What a grid position shows.
pub enum Shown<'a> {
    Empty,
    /// A value; (dr, dc) is the offset inside its spill (0,0 for the source).
    Value { value: &'a Value, dr: usize, dc: usize, anchor: CellKey },
    Error(&'a CellError),
}

/// A reversible change to the document.
#[derive(Clone, Debug)]
pub enum Edit {
    Cells(Vec<(CellKey, Option<Cell>)>),
    InsertRows { sheet: SheetId, at: usize, ids: Vec<RowId> },
    DeleteRows { sheet: SheetId, at: usize, n: usize },
    InsertCols { sheet: SheetId, at: usize, ids: Vec<ColId> },
    DeleteCols { sheet: SheetId, at: usize, n: usize },
    PermuteRows { sheet: SheetId, at: usize, ids: Vec<RowId> },
    Names(BTreeMap<String, NameDef>),
    /// Puts a whole sheet, with its own ids, at `at` in the tab order.
    InsertSheet { at: usize, sheet: Box<Sheet> },
    /// Removes a sheet; the inverse carries it whole, so undo restores the same ids.
    DeleteSheet { sheet: SheetId },
    MoveSheet { sheet: SheetId, to: usize },
    RenameSheet { sheet: SheetId, name: String },
    Batch(Vec<Edit>),
}

pub struct Engine {
    pub wb: Workbook,
    nodes: HashMap<CellKey, Node>,
    results: HashMap<CellKey, CellResult>,
    statics: HashMap<CellKey, Static>,
    /// Anchor → region it actually spills into.
    spills: HashMap<CellKey, Region>,
    /// Anchor → region it wants (even when blocked).
    desired: HashMap<CellKey, Region>,
    /// Spilled-into position → anchor.
    cover: HashMap<CellKey, CellKey>,
    rdeps: HashMap<CellKey, Vec<CellKey>>,
    range_deps: Vec<(SheetId, StoredRef, StoredRef, CellKey)>,
    syms: Symbols,
    pub cycles: Vec<Vec<CellKey>>,
    /// Number of cells evaluated by the last recalc (for diagnostics/tests).
    pub last_eval_count: usize,
    /// Bumped whenever the dependency graph or spill coverage changes.
    graph_gen: u64,
    /// The last recalc's dirty set and evaluation order, reused while the same
    /// cells change and the graph doesn't (scrubbing, goal-seek).
    plan: Option<Plan>,
}

/// A recalc plan: valid for `seeds` while `graph_gen` hasn't moved.
struct Plan {
    gen: u64,
    seeds: Vec<CellKey>,
    dirty: HashSet<CellKey>,
    order: Vec<CellKey>,
}

struct View<'a>(&'a Engine);

impl<'a> Env for View<'a> {
    fn cell_value(&self, k: CellKey) -> Result<Value, String> {
        self.0.ref_value(k)
    }
    fn range_value(&self, sheet: SheetId, a: &StoredRef, b: &StoredRef, gaps: bool) -> Result<Value, String> {
        self.0.range_value(sheet, a, b, gaps)
    }
    fn unit_info(&self, name: &str) -> Result<Arc<UnitInfo>, String> {
        let Some(k) = self.0.syms.units.get(name) else { return Err(format!("unknown unit {name}")) };
        match self.0.results.get(k) {
            Some(Ok(Value::Unit(u))) => Ok(u.clone()),
            Some(Err(_)) => Err(format!("unit {name} has an error (at {})", self.0.wb.cell_label(*k, None))),
            _ => Err(format!("unit {name} isn't ready")),
        }
    }
    fn word(&self, k: CellKey) -> Option<Arc<Compiled>> {
        self.0.nodes.get(&k)?.compiled.as_ref().ok().cloned()
    }
}

impl<'a> StaticEnv for View<'a> {
    fn cell(&self, k: CellKey) -> Option<SVal> {
        let e = self.0;
        e.wb.sheet(k.sheet)?;
        if e.nodes.contains_key(&k) {
            return Some(e.static_ref(k));
        }
        match e.cover.get(&k) {
            Some(a) => e.static_element(*a),
            None => Some(SVal::Any),
        }
    }
    fn range(&self, sheet: SheetId, a: &StoredRef, b: &StoredRef, gaps: bool) -> Result<SVal, Halt> {
        let e = self.0;
        let s = e.wb.sheet(sheet).ok_or(Halt::Stop)?;
        let (r0, c0, r1, c1) = s.range_bounds(a, b).ok_or(Halt::Stop)?;
        let mut cells = Vec::with_capacity((r1 - r0 + 1) * (c1 - c0 + 1));
        for r in r0..=r1 {
            for c in c0..=c1 {
                let k = s.key(r, c).unwrap();
                if gaps && !e.nodes.contains_key(&k) && matches!(e.shown(k), Shown::Empty) {
                    continue;
                }
                let src = if e.nodes.contains_key(&k) { Some(k) } else { e.cover.get(&k).copied() };
                cells.push((k, src.map_or(Some(SVal::Any), |a| e.static_element(a))));
            }
        }
        dims::range_type(&cells, &|k| e.wb.cell_label(k, None))
    }
    fn unit(&self, name: &str) -> Option<(Option<Dim>, bool)> {
        let e = self.0;
        let k = e.syms.units.get(name)?;
        match &**e.nodes.get(k)?.compiled.as_ref().ok()? {
            Compiled::Base { dim, .. } => Some((Some(Dim::base(dim)), false)),
            Compiled::UnitDef { offset, .. } => Some((e.statics.get(k).and_then(|s| s.val.dim().cloned()), offset.is_some())),
            _ => None,
        }
    }
    fn word(&self, k: CellKey) -> Option<Arc<Compiled>> {
        Env::word(self, k)
    }
}

impl Engine {
    pub fn new(wb: Workbook) -> Engine {
        let mut e = Engine {
            wb,
            nodes: HashMap::default(),
            results: HashMap::default(),
            statics: HashMap::default(),
            spills: HashMap::default(),
            desired: HashMap::default(),
            cover: HashMap::default(),
            rdeps: HashMap::default(),
            range_deps: Vec::new(),
            syms: Symbols::default(),
            cycles: Vec::new(),
            last_eval_count: 0,
            graph_gen: 0,
            plan: None,
        };
        e.wb.after_load();
        e.rebuild();
        e
    }

    // ---- queries ---------------------------------------------------------

    pub fn result(&self, k: CellKey) -> Option<&CellResult> {
        self.results.get(&k)
    }

    pub fn kind(&self, k: CellKey) -> Kind {
        self.nodes.get(&k).map(|n| n.kind).unwrap_or(Kind::Empty)
    }

    pub fn compile_error(&self, k: CellKey) -> Option<&CompileError> {
        self.nodes.get(&k)?.compiled.as_ref().err()
    }

    pub fn spill_anchor(&self, k: CellKey) -> Option<CellKey> {
        self.cover.get(&k).copied()
    }

    /// (rows, cols) of a cell's spill region if it spills.
    pub fn spill_size(&self, anchor: CellKey) -> Option<(usize, usize)> {
        self.spills.get(&anchor).map(|r| (r.rows, r.cols))
    }

    pub fn shown(&self, k: CellKey) -> Shown<'_> {
        if let Some(r) = self.results.get(&k) {
            return match r {
                Ok(v) => Shown::Value { value: v, dr: 0, dc: 0, anchor: k },
                Err(e) => Shown::Error(e),
            };
        }
        if let Some(a) = self.cover.get(&k) {
            if let (Some(Ok(v)), Some(reg), Some((r, c))) = (self.results.get(a), self.spills.get(a), self.wb.pos(k)) {
                return Shown::Value { value: v, dr: r - reg.r0, dc: c - reg.c0, anchor: *a };
            }
        }
        Shown::Empty
    }

    pub fn symbols(&self) -> &Symbols {
        &self.syms
    }

    /// Direct precedents (cells this cell reads), with spill coverage resolved.
    pub fn precedents(&self, k: CellKey) -> Vec<CellKey> {
        let mut out = Vec::new();
        if let Some(n) = self.nodes.get(&k) {
            for d in &n.deps {
                match d {
                    Dep::Cell(p) => out.push(*p),
                    Dep::Range { sheet, a, b } => {
                        if let Some(s) = self.wb.sheet(*sheet) {
                            if let Some((r0, c0, r1, c1)) = s.range_bounds(a, b) {
                                for r in r0..=r1 {
                                    for c in c0..=c1 {
                                        if let Some(p) = s.key(r, c) {
                                            out.push(p);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Direct dependents (cells that read this position or its spill).
    pub fn dependents(&self, k: CellKey) -> Vec<CellKey> {
        let mut positions = vec![k];
        if let Some(reg) = self.spills.get(&k) {
            positions.extend(self.region_keys(reg));
        }
        let mut out = Vec::new();
        for p in positions {
            out.extend(self.dependents_of_position(p));
        }
        out.sort();
        out.dedup();
        out.retain(|x| *x != k);
        out
    }

    /// Literal number cells upstream of `k` (transitively), named inputs first.
    pub fn upstream_inputs(&self, k: CellKey) -> Vec<CellKey> {
        let mut seen = HashSet::default();
        let mut stack = vec![k];
        let mut found = Vec::new();
        while let Some(c) = stack.pop() {
            if !seen.insert(c) {
                continue;
            }
            let c = self.cover.get(&c).copied().unwrap_or(c);
            if c != k && self.kind(c) == Kind::Number {
                found.push(c);
                continue;
            }
            for p in self.precedents(c) {
                if !matches!(self.kind(p), Kind::UnitDecl | Kind::WordDef) {
                    stack.push(p);
                }
            }
        }
        let named: HashSet<CellKey> = self.wb.names.values().map(|n| n.cell).collect();
        found.sort_by_key(|c| (!named.contains(c), self.wb.pos(*c)));
        found.dedup();
        found
    }

    pub fn name_of(&self, k: CellKey) -> Option<&str> {
        self.wb.names.iter().find(|(_, d)| d.cell == k).map(|(n, _)| n.as_str())
    }

    // ---- help / playground support ------------------------------------------

    /// Runs `text` as if it were in a cell on `home`, without changing the
    /// document, recording the stack after each token.
    pub fn eval_scratch(&self, text: &str, home: SheetId) -> Scratch {
        let mut c = Compiler::new(&self.wb, &self.syms, home);
        let compiled = match c.compile(text) {
            Ok(x) => x,
            Err(e) => return Scratch { steps: vec![], result: Err(local(e.msg, e.span)) },
        };
        let env = View(self);
        match compiled {
            Compiled::Program(ops) => {
                let (steps, res) = run_traced(&env, &ops);
                Scratch { steps, result: res.map_err(|e| local(e.msg, e.span)) }
            }
            Compiled::Empty => Scratch { steps: vec![], result: Err(local("empty".into(), None)) },
            Compiled::Text(t) => Scratch { steps: vec![], result: Ok(Value::text(&t)) },
            _ => Scratch {
                steps: vec![],
                result: Err(local("definitions take effect only in a cell; step through a program that uses them".into(), None)),
            },
        }
    }

    /// Step-by-step evaluation of a cell's program.
    pub fn trace_cell(&self, k: CellKey) -> Scratch {
        self.eval_scratch(&self.wb.cell_text(k), k.sheet)
    }

    /// The static pass over `text` as if it were in a cell on `home`: the
    /// inferred stack after each token, without evaluating anything.
    pub fn static_scratch(&self, text: &str, home: SheetId) -> StaticTrace {
        let mut c = Compiler::new(&self.wb, &self.syms, home);
        match c.compile(text) {
            Ok(Compiled::Program(ops)) => dims::analyze_traced(&View(self), &ops),
            _ => StaticTrace::default(),
        }
    }

    // ---- static dimensions -------------------------------------------------

    /// What a cell's value statically is — its dimension and whether it is
    /// absolute — known even when the value is an error or its inputs are
    /// empty. A spilled-into cell reports its source; a unit declaration, the
    /// quantity that defines it.
    pub fn static_value(&self, k: CellKey) -> Option<&SVal> {
        let k = if self.nodes.contains_key(&k) { k } else { *self.cover.get(&k)? };
        self.statics.get(&k).map(|s| &s.val)
    }

    /// The dimension a cell's value statically has, if known.
    pub fn static_dim(&self, k: CellKey) -> Option<&Dim> {
        self.static_value(k)?.dim()
    }

    /// The unit error the static pass found in a cell, if any.
    pub fn static_error(&self, k: CellKey) -> Option<&EvalErr> {
        self.statics.get(&k)?.err.as_ref()
    }

    /// What a reference to node `k` pushes.
    fn static_ref(&self, k: CellKey) -> SVal {
        match self.nodes.get(&k).map(|n| n.compiled.as_ref().map(|c| &**c)) {
            Some(Ok(Compiled::UnitDef { .. } | Compiled::Base { .. })) => SVal::Other("unit"),
            Some(Ok(Compiled::Dim(_))) => SVal::Other("dimension"),
            Some(Ok(Compiled::WordDef { .. })) => SVal::Other("word"),
            Some(Ok(_)) => self.statics.get(&k).map_or(SVal::Any, |s| s.val.forget()),
            _ => SVal::Any,
        }
    }

    /// One element of node `a`'s value, as `scalar_at` reads it; `None` where that certainly fails.
    fn static_element(&self, a: CellKey) -> Option<SVal> {
        match self.static_ref(a) {
            SVal::Chart | SVal::Other(_) => None,
            v => Some(v),
        }
    }

    fn compute_static(&self, k: CellKey) -> Static {
        let Some(Ok(c)) = self.nodes.get(&k).map(|n| &n.compiled) else { return Static::any() };
        let env = View(self);
        let from = |r: Result<SVal, EvalErr>| match r {
            Ok(val) => Static { val, err: None },
            Err(e) => Static { val: SVal::Any, err: Some(e) },
        };
        match &**c {
            Compiled::Program(ops) => from(dims::analyze(&env, ops)),
            Compiled::Text(_) => Static { val: SVal::Text, err: None },
            // a duplicate definition fails before its body runs
            Compiled::UnitDef { name, body, .. } if self.syms.units.get(&**name) == Some(&k) => from(match dims::analyze(&env, body) {
                Ok(v @ (SVal::Num(_) | SVal::Any)) => Ok(v),
                Ok(_) => Err(EvalErr { msg: "a unit is defined by a number with units".into(), span: None }),
                Err(e) => Err(e),
            }),
            Compiled::Base { dim, .. } => Static { val: SVal::num(Some(Dim::base(dim)), Some(false)), err: None },
            _ => Static::any(),
        }
    }

    /// Whether an edit leaves a cell's static result as it was: a number
    /// literal whose value changed but not its unit (scrubbing).
    fn same_statics(&self, k: CellKey, old: &Node, new: &Node) -> bool {
        if old.kind != Kind::Number || new.kind != Kind::Number || old.deps != new.deps {
            return false;
        }
        if self.statics.get(&k).is_none_or(|s| s.err.is_some()) {
            return false;
        }
        match (old.compiled.as_deref(), new.compiled.as_deref()) {
            (Ok(Compiled::Program(a)), Ok(Compiled::Program(b))) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(x, y)| matches!((&x.kind, &y.kind), (OpKind::Num(_), OpKind::Num(_))) || x.kind == y.kind)
            }
            _ => false,
        }
    }

    /// Every unit with its defining cell and current definition, sorted by name.
    pub fn units_list(&self) -> Vec<(String, CellKey, Option<Arc<UnitInfo>>)> {
        let mut v: Vec<_> = self
            .syms
            .units
            .iter()
            .map(|(n, k)| {
                let info = match self.results.get(k) {
                    Some(Ok(Value::Unit(u))) => Some(u.clone()),
                    _ => None,
                };
                (n.clone(), *k, info)
            })
            .collect();
        v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        v
    }

    /// Every user-defined word: (name, defining cell, `( doc )` comment, locals).
    pub fn words_list(&self) -> Vec<(String, CellKey, Option<Arc<str>>, Vec<Arc<str>>)> {
        let mut v: Vec<_> = self
            .syms
            .words
            .iter()
            .map(|(n, k)| match self.nodes.get(k).and_then(|n| n.compiled.as_ref().ok()).map(|c| &**c) {
                Some(Compiled::WordDef { doc, locals, .. }) => (n.clone(), *k, doc.clone(), locals.clone()),
                _ => (n.clone(), *k, None, vec![]),
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    // ---- value access used by the interpreter ----------------------------

    fn ref_value(&self, k: CellKey) -> Result<Value, String> {
        if self.wb.sheet(k.sheet).is_none() {
            return Err("reference to a deleted sheet".into());
        }
        if let Some(r) = self.results.get(&k) {
            return match r {
                // provenance was stamped when the result was stored (see recalc)
                Ok(v) => Ok(v.clone()),
                Err(_) => Err(format!("{} has an error", self.wb.cell_label(k, None))),
            };
        }
        if self.cover.contains_key(&k) {
            return self.scalar_at(k);
        }
        if self.nodes.contains_key(&k) {
            return Err(format!("{} isn't computed (cycle?)", self.wb.cell_label(k, None)));
        }
        Err(format!("{} is empty", self.wb.cell_label(k, None)))
    }

    /// The single element shown at a position (for ranges and spilled cells).
    fn scalar_at(&self, k: CellKey) -> Result<Value, String> {
        let label = || self.wb.cell_label(k, None);
        let (v, dr, dc, anchor) = match self.shown(k) {
            Shown::Empty => {
                if self.nodes.contains_key(&k) {
                    return Err(format!("{} isn't computed (cycle?)", label()));
                }
                return Err(format!("{} is empty", label()));
            }
            Shown::Error(_) => return Err(format!("{} has an error", label())),
            Shown::Value { value, dr, dc, anchor } => (value, dr, dc, anchor),
        };
        match v {
            Value::Num(n) => {
                let i = match n.shape.as_slice() {
                    [] => 0,
                    [_] => dr,
                    [_, c] => dr * c + dc,
                    _ => return Err(format!("{} holds a rank {} array", label(), n.rank())),
                };
                let x = *n.data.get(i).ok_or_else(|| format!("{} is an empty array", label()))?;
                let prov = if anchor == k && self.kind(k) == Kind::Number { Prov::Literal(k) } else { Prov::Derived(anchor, i) };
                let mut s = Num::scalar(x, n.q.clone());
                s.prov = Some(Arc::new(vec![prov]));
                Ok(Value::Num(s))
            }
            Value::Text(t) => {
                let i = match t.shape.as_slice() {
                    [] => 0,
                    [_] => dr,
                    [_, c] => dr * c + dc,
                    _ => return Err(format!("{} holds a rank {} array", label(), t.shape.len())),
                };
                Ok(Value::text(t.data.get(i).ok_or_else(|| format!("{} is an empty array", label()))?))
            }
            Value::Chart(_) => Err(format!("{} is part of a chart", label())),
            other => Err(format!("{} is a {}", label(), other.type_name())),
        }
    }

    /// The cells of a range as one array. With `gaps` (`A1:B5?`), empty cells
    /// are skipped and the rest form a list, row by row; otherwise an empty
    /// cell is an error.
    fn range_value(&self, sheet: SheetId, a: &StoredRef, b: &StoredRef, gaps: bool) -> Result<Value, String> {
        let s = self.wb.sheet(sheet).ok_or("sheet was deleted")?;
        let (r0, c0, r1, c1) = s.range_bounds(a, b).ok_or("range refers to deleted cells")?;
        let (nr, nc) = (r1 - r0 + 1, c1 - c0 + 1);
        let mut nums: Vec<f64> = Vec::with_capacity(nr * nc);
        let mut texts: Vec<Arc<str>> = Vec::new();
        let mut prov = Vec::with_capacity(nr * nc);
        let mut q: Option<(Quant, CellKey)> = None;
        for r in r0..=r1 {
            for c in c0..=c1 {
                let k = s.key(r, c).unwrap();
                if gaps && !self.nodes.contains_key(&k) && matches!(self.shown(k), Shown::Empty) {
                    continue;
                }
                match self.scalar_at(k)? {
                    Value::Num(n) => {
                        if !texts.is_empty() {
                            return Err(format!("range mixes text and numbers at {}", self.wb.cell_label(k, None)));
                        }
                        match &q {
                            None => q = Some((n.q.clone(), k)),
                            Some((q0, k0)) => {
                                if q0.dim != n.q.dim || q0.absolute.is_some() != n.q.absolute.is_some() {
                                    return Err(format!(
                                        "range mixes units: {} is {}, {} is {}",
                                        self.wb.cell_label(*k0, None),
                                        q0.dim,
                                        self.wb.cell_label(k, None),
                                        n.q.dim
                                    ));
                                }
                            }
                        }
                        nums.push(n.data[0]);
                        prov.push(n.prov.as_ref().map(|p| p[0]).unwrap_or(Prov::None));
                    }
                    Value::Text(t) => {
                        if !nums.is_empty() {
                            return Err(format!("range mixes text and numbers at {}", self.wb.cell_label(k, None)));
                        }
                        texts.push(t.data[0].clone());
                    }
                    other => return Err(format!("range contains a {}", other.type_name())),
                }
            }
        }
        let shape = if gaps {
            vec![nums.len() + texts.len()]
        } else if nr == 1 || nc == 1 {
            vec![nr * nc]
        } else {
            vec![nr, nc]
        };
        if !texts.is_empty() {
            return Ok(Value::Text(Text { shape, data: Arc::new(texts) }));
        }
        let mut n = Num::with_shape(shape, nums, q.map(|x| x.0).unwrap_or_else(Quant::none));
        n.prov = Some(Arc::new(prov));
        Ok(Value::Num(n))
    }

    // ---- editing -----------------------------------------------------------

    /// Set a cell from typed text (A1 references are resolved to ids).
    pub fn set_text(&mut self, k: CellKey, text: &str) -> Edit {
        let cell = if text.trim().is_empty() {
            None
        } else {
            Some(Cell { pieces: self.wb.parse_text(text, k.sheet) })
        };
        self.apply(Edit::Cells(vec![(k, cell)]))
    }

    /// Applies an edit, recalculates, and returns its inverse.
    pub fn apply(&mut self, e: Edit) -> Edit {
        let mut touched = Vec::new();
        let mut structural = false;
        let inv = self.apply_raw(e, &mut touched, &mut structural);
        if structural {
            self.rebuild();
        } else {
            self.cells_changed(&touched);
        }
        inv
    }

    fn apply_raw(&mut self, e: Edit, touched: &mut Vec<CellKey>, structural: &mut bool) -> Edit {
        match e {
            Edit::Cells(cells) => {
                let mut inv = Vec::with_capacity(cells.len());
                for (k, cell) in cells {
                    let Some(s) = self.wb.sheet_mut(k.sheet) else { continue };
                    let old = match cell {
                        Some(c) => s.cells.insert((k.row, k.col), c),
                        None => s.cells.remove(&(k.row, k.col)),
                    };
                    inv.push((k, old));
                    touched.push(k);
                }
                inv.reverse();
                Edit::Cells(inv)
            }
            Edit::InsertRows { sheet, at, ids } => {
                *structural = true;
                let n = ids.len();
                if let Some(s) = self.wb.sheet_mut(sheet) {
                    s.rows.insert(at, &ids);
                }
                Edit::DeleteRows { sheet, at, n }
            }
            Edit::InsertCols { sheet, at, ids } => {
                *structural = true;
                let n = ids.len();
                if let Some(s) = self.wb.sheet_mut(sheet) {
                    s.cols.insert(at, &ids);
                }
                Edit::DeleteCols { sheet, at, n }
            }
            Edit::DeleteRows { sheet, at, n } => {
                *structural = true;
                let Some(s) = self.wb.sheet_mut(sheet) else { return Edit::Batch(vec![]) };
                let (ids, removed) = s.delete_rows(at, n);
                let cells = removed.into_iter().map(|((r, c), cell)| (CellKey { sheet, row: r, col: c }, Some(cell))).collect();
                Edit::Batch(vec![Edit::InsertRows { sheet, at, ids }, Edit::Cells(cells)])
            }
            Edit::DeleteCols { sheet, at, n } => {
                *structural = true;
                let Some(s) = self.wb.sheet_mut(sheet) else { return Edit::Batch(vec![]) };
                let (ids, removed) = s.delete_cols(at, n);
                let cells = removed.into_iter().map(|((r, c), cell)| (CellKey { sheet, row: r, col: c }, Some(cell))).collect();
                Edit::Batch(vec![Edit::InsertCols { sheet, at, ids }, Edit::Cells(cells)])
            }
            Edit::PermuteRows { sheet, at, ids } => {
                *structural = true;
                // Ranges keep covering the same block of rows: a sort moves
                // cells within a range, it doesn't move the range.
                let before = self.range_bounds_on(sheet);
                let Some(s) = self.wb.sheet_mut(sheet) else { return Edit::Batch(vec![]) };
                let old = s.rows.ids()[at..at + ids.len()].to_vec();
                s.rows.permute(at, &ids);
                let mut restore = Vec::new();
                for (k, cell, bounds) in before {
                    let s = self.wb.sheet(sheet).unwrap();
                    let mut pieces = cell.pieces.clone();
                    for (i, b) in bounds {
                        if let (Piece::Range(a, z), Some((r0, c0, r1, c1))) = (&mut pieces[i], b) {
                            let (ka, kz) = (s.key(r0, c0).unwrap(), s.key(r1, c1).unwrap());
                            a.row = ka.row;
                            a.col = ka.col;
                            z.row = kz.row;
                            z.col = kz.col;
                        }
                    }
                    if pieces != cell.pieces {
                        let ks = self.wb.sheet_mut(k.sheet).unwrap();
                        ks.cells.insert((k.row, k.col), Cell { pieces });
                        restore.push((k, Some(cell)));
                    }
                }
                Edit::Batch(vec![Edit::PermuteRows { sheet, at, ids: old }, Edit::Cells(restore)])
            }
            Edit::Names(n) => {
                *structural = true;
                Edit::Names(std::mem::replace(&mut self.wb.names, n))
            }
            Edit::InsertSheet { at, sheet } => {
                if self.wb.sheet(sheet.id).is_some() {
                    return Edit::Batch(vec![]);
                }
                *structural = true;
                let id = sheet.id;
                let mut sheet = *sheet;
                sheet.reindex();
                self.wb.sheets.insert(at.min(self.wb.sheets.len()), sheet);
                Edit::DeleteSheet { sheet: id }
            }
            Edit::DeleteSheet { sheet } => {
                // the last sheet stays: the app always shows one
                let Some(at) = self.wb.sheet_index(sheet).filter(|_| self.wb.sheets.len() > 1) else { return Edit::Batch(vec![]) };
                *structural = true;
                Edit::InsertSheet { at, sheet: Box::new(self.wb.sheets.remove(at)) }
            }
            Edit::MoveSheet { sheet, to } => {
                let Some(from) = self.wb.sheet_index(sheet) else { return Edit::Batch(vec![]) };
                *structural = true;
                let s = self.wb.sheets.remove(from);
                self.wb.sheets.insert(to.min(self.wb.sheets.len()), s);
                Edit::MoveSheet { sheet, to: from }
            }
            Edit::RenameSheet { sheet, name } => {
                let Some(s) = self.wb.sheet_mut(sheet) else { return Edit::Batch(vec![]) };
                *structural = true;
                Edit::RenameSheet { sheet, name: std::mem::replace(&mut s.name, name) }
            }
            Edit::Batch(es) => {
                let mut inv: Vec<Edit> = es.into_iter().map(|e| self.apply_raw(e, touched, structural)).collect();
                inv.reverse();
                Edit::Batch(inv)
            }
        }
    }

    /// Every cell holding ranges on `sheet`, with each range piece's bounds.
    #[allow(clippy::type_complexity)]
    fn range_bounds_on(&self, sheet: SheetId) -> Vec<(CellKey, Cell, Vec<(usize, Option<(usize, usize, usize, usize)>)>)> {
        let mut out = Vec::new();
        let Some(target) = self.wb.sheet(sheet) else { return out };
        for s in &self.wb.sheets {
            for ((r, c), cell) in &s.cells {
                let bounds: Vec<_> = cell
                    .pieces
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| match p {
                        Piece::Range(a, b) if a.sheet.unwrap_or(s.id) == sheet => Some((i, target.range_bounds(a, b))),
                        _ => None,
                    })
                    .collect();
                if !bounds.is_empty() {
                    out.push((CellKey { sheet: s.id, row: *r, col: *c }, cell.clone(), bounds));
                }
            }
        }
        out
    }

    /// Name a cell. Names must not shadow words or look like references.
    pub fn set_name(&mut self, name: &str, cell: Option<CellKey>, input: bool) -> Result<Edit, String> {
        let mut names = self.wb.names.clone();
        match cell {
            None => {
                names.remove(name);
            }
            Some(cell) => {
                valid_name(name)?;
                if self.syms.words.contains_key(name) {
                    return Err(format!("{name} is a word"));
                }
                if let Some(d) = names.get(name) {
                    if d.cell != cell {
                        return Err(format!("{name} already names {}", self.wb.cell_label(d.cell, None)));
                    }
                }
                names.retain(|_, d| d.cell != cell);
                names.insert(name.to_string(), NameDef { cell, input });
            }
        }
        Ok(self.apply(Edit::Names(names)))
    }

    // ---- sheets: these build an edit for `apply` (and the undo stack) ----------

    /// A new empty sheet at `at`, named `SheetN`.
    pub fn add_sheet_edit(&self, at: usize) -> Edit {
        let mut n = self.wb.sheets.len() + 1;
        while self.wb.sheet_by_name(&format!("Sheet{n}")).is_some() {
            n += 1;
        }
        Edit::InsertSheet { at, sheet: Box::new(Sheet::new(&format!("Sheet{n}"), 200, 26)) }
    }

    /// A copy right after the original. It keeps the row and column ids (cells are keyed by
    /// sheet too, so nothing collides), which makes its own-sheet references point at the copy.
    pub fn duplicate_sheet_edit(&self, sheet: SheetId) -> Result<Edit, String> {
        let at = self.wb.sheet_index(sheet).ok_or("no such sheet")?;
        let mut copy = self.wb.sheets[at].clone();
        copy.id = SheetId(fresh_id());
        copy.name = self.wb.free_sheet_name(&format!("{} copy", copy.name));
        Ok(Edit::InsertSheet { at: at + 1, sheet: Box::new(copy) })
    }

    pub fn delete_sheet_edit(&self, sheet: SheetId) -> Result<Edit, String> {
        self.wb.sheet_index(sheet).ok_or("no such sheet")?;
        if self.wb.sheets.len() < 2 {
            return Err("can't delete the only sheet".into());
        }
        Ok(Edit::DeleteSheet { sheet })
    }

    pub fn rename_sheet_edit(&self, sheet: SheetId, name: &str) -> Result<Edit, String> {
        self.wb.sheet_index(sheet).ok_or("no such sheet")?;
        Ok(Edit::RenameSheet { sheet, name: self.wb.check_sheet_name(name, Some(sheet))? })
    }

    /// Unit and dimension declarations on a sheet: deleting it turns their users into errors.
    pub fn declarations_on(&self, sheet: SheetId) -> usize {
        self.syms.units.values().chain(self.syms.dims.values()).filter(|k| k.sheet == sheet).count()
    }

    // ---- graph maintenance ---------------------------------------------------

    fn all_content(&self) -> Vec<(CellKey, String)> {
        let mut out = Vec::new();
        for s in &self.wb.sheets {
            let mut keys: Vec<(usize, usize, CellKey)> = s
                .cells
                .keys()
                .filter_map(|(r, c)| {
                    let k = CellKey { sheet: s.id, row: *r, col: *c };
                    s.pos(k).map(|(ri, ci)| (ri, ci, k))
                })
                .collect();
            // position order makes "first definition wins" deterministic
            keys.sort_by_key(|(r, c, _)| (*r, *c));
            for (_, _, k) in keys {
                out.push((k, self.wb.cell_text(k)));
            }
        }
        out
    }

    fn build_symbols(&mut self, content: &[(CellKey, String)]) {
        let mut syms = Symbols::default();
        for (k, text) in content {
            match declares(text) {
                Some(Declares::Word(w)) => {
                    syms.words.entry(w).or_insert(*k);
                }
                Some(Declares::Unit(u)) => {
                    syms.units.entry(u).or_insert(*k);
                }
                Some(Declares::Dim(d)) => {
                    syms.dims.entry(d).or_insert(*k);
                }
                None => {}
            }
        }
        self.syms = syms;
    }

    fn compile_node(&self, k: CellKey, text: &str) -> Node {
        if let Some(e) = self.dead_sheet_ref(k) {
            return Node { kind: classify(text), compiled: Err(e), deps: vec![] };
        }
        let mut c = Compiler::new(&self.wb, &self.syms, k.sheet);
        let compiled = c.compile(text).map(Arc::new);
        Node { kind: classify(text), compiled, deps: c.deps }
    }

    /// A reference into a deleted sheet renders as `#ref!`; say which kind of deletion it was.
    fn dead_sheet_ref(&self, k: CellKey) -> Option<CompileError> {
        let mut at = 0;
        for p in &self.wb.cell(k)?.pieces {
            let len = self.wb.render(std::slice::from_ref(p), k.sheet).len();
            let sheet = match p {
                Piece::Ref(r) | Piece::Range(r, _) => r.sheet,
                Piece::Text(_) => None,
            };
            if sheet.is_some_and(|s| self.wb.sheet(s).is_none()) {
                return Some(CompileError { msg: "reference to a deleted sheet".into(), span: Some(at..at + len) });
            }
            at += len;
        }
        None
    }

    fn index_deps(&mut self, k: CellKey) {
        let Some(n) = self.nodes.get(&k) else { return };
        let deps = n.deps.clone();
        for d in deps {
            match d {
                Dep::Cell(p) => self.rdeps.entry(p).or_default().push(k),
                Dep::Range { sheet, a, b } => self.range_deps.push((sheet, a, b, k)),
            }
        }
    }

    fn unindex_deps(&mut self, k: CellKey) {
        let Some(n) = self.nodes.get(&k) else { return };
        for d in &n.deps {
            if let Dep::Cell(p) = d {
                if let Some(v) = self.rdeps.get_mut(p) {
                    v.retain(|x| *x != k);
                }
            }
        }
        self.range_deps.retain(|x| x.3 != k);
    }

    /// Recompile everything and recalculate everything.
    pub fn rebuild(&mut self) {
        let content = self.all_content();
        self.build_symbols(&content);
        self.nodes.clear();
        self.rdeps.clear();
        self.range_deps.clear();
        self.results.clear();
        self.statics.clear();
        self.spills.clear();
        self.desired.clear();
        self.cover.clear();
        for (k, text) in &content {
            let n = self.compile_node(*k, text);
            self.nodes.insert(*k, n);
            self.index_deps(*k);
        }
        self.graph_gen += 1;
        self.plan = None;
        let all: Vec<CellKey> = self.nodes.keys().copied().collect();
        self.recalc(all.into_iter().collect(), None, None);
    }

    fn cells_changed(&mut self, keys: &[CellKey]) {
        // Declarations change the symbol table: recompile everything.
        let decl_change = keys.iter().any(|k| {
            let old = self.nodes.get(k).map(|n| matches!(n.kind, Kind::WordDef | Kind::UnitDecl)).unwrap_or(false);
            old || matches!(classify(&self.wb.cell_text(*k)), Kind::WordDef | Kind::UnitDecl)
        });
        if decl_change {
            self.rebuild();
            return;
        }
        let mut seeds = Vec::new();
        let mut static_seeds = Vec::new();
        for k in keys {
            let k = *k;
            self.unindex_deps(k);
            let old = self.nodes.remove(&k);
            let text = self.wb.cell_text(k);
            if !text.trim().is_empty() {
                let n = self.compile_node(k, &text);
                self.nodes.insert(k, n);
                self.index_deps(k);
            }
            // same dependencies (e.g. a scrubbed number): the graph, and so the plan, still hold
            if !matches!((&old, self.nodes.get(&k)), (Some(o), Some(n)) if o.deps == n.deps) {
                self.graph_gen += 1;
            }
            // scrubbing a number never changes a dimension: nothing downstream needs the static pass again
            if !matches!((&old, self.nodes.get(&k)), (Some(o), Some(n)) if self.same_statics(k, o, n)) {
                static_seeds.push(k);
            }
            seeds.push(k);
            // A cell typed into (or cleared from) a spill region affects the spiller.
            for (a, reg) in &self.desired {
                if let Some((r, c)) = self.wb.pos(k) {
                    if reg.contains(k.sheet, r, c) && *a != k {
                        seeds.push(*a);
                    }
                }
            }
        }
        let static_dirty = if static_seeds.is_empty() { HashSet::default() } else { self.closure(static_seeds) };
        let (dirty, order) = match self.plan.take().filter(|p| p.gen == self.graph_gen && p.seeds == seeds) {
            Some(p) => (p.dirty, Some(p.order)),
            None => (self.closure(seeds.clone()), None),
        };
        if let Some((dirty, order)) = self.recalc(dirty, Some(static_dirty), order) {
            self.plan = Some(Plan { gen: self.graph_gen, seeds, dirty, order });
        }
    }

    fn dependents_of_position(&self, p: CellKey) -> Vec<CellKey> {
        let mut out: Vec<CellKey> = self.rdeps.get(&p).cloned().unwrap_or_default();
        if !self.range_deps.is_empty() {
            if let Some((r, c)) = self.wb.pos(p) {
                for (sheet, a, b, k) in &self.range_deps {
                    if *sheet != p.sheet {
                        continue;
                    }
                    if let Some((r0, c0, r1, c1)) = self.wb.sheet(*sheet).and_then(|s| s.range_bounds(a, b)) {
                        if r >= r0 && r <= r1 && c >= c0 && c <= c1 {
                            out.push(*k);
                        }
                    }
                }
            }
        }
        out
    }

    fn region_keys(&self, reg: &Region) -> Vec<CellKey> {
        let Some(s) = self.wb.sheet(reg.sheet) else { return vec![] };
        let mut out = Vec::with_capacity(reg.rows * reg.cols);
        for r in reg.r0..reg.r0 + reg.rows {
            for c in reg.c0..reg.c0 + reg.cols {
                if let Some(k) = s.key(r, c) {
                    out.push(k);
                }
            }
        }
        out
    }

    /// Seeds plus everything downstream of them.
    fn closure(&self, seeds: Vec<CellKey>) -> HashSet<CellKey> {
        let mut seen: HashSet<CellKey> = HashSet::default();
        let mut stack = seeds;
        while let Some(k) = stack.pop() {
            if !seen.insert(k) {
                continue;
            }
            let mut positions = vec![k];
            if let Some(reg) = self.spills.get(&k) {
                positions.extend(self.region_keys(reg));
            }
            for p in positions {
                for d in self.dependents_of_position(p) {
                    if !seen.contains(&d) {
                        stack.push(d);
                    }
                }
            }
        }
        seen
    }

    /// The nodes a node must be evaluated after.
    fn eval_deps(&self, k: CellKey) -> Vec<CellKey> {
        let mut out = Vec::new();
        let Some(n) = self.nodes.get(&k) else { return out };
        let add = |p: CellKey, out: &mut Vec<CellKey>| {
            if self.nodes.contains_key(&p) {
                out.push(p);
            } else if let Some(a) = self.cover.get(&p) {
                out.push(*a);
            }
        };
        for d in &n.deps {
            match d {
                Dep::Cell(p) => add(*p, &mut out),
                Dep::Range { sheet, a, b } => {
                    if let Some(s) = self.wb.sheet(*sheet) {
                        if let Some((r0, c0, r1, c1)) = s.range_bounds(a, b) {
                            for r in r0..=r1 {
                                for c in c0..=c1 {
                                    if let Some(p) = s.key(r, c) {
                                        add(p, &mut out);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Recomputes `dirty` in dependency order (`order` when the caller has a
    /// cached plan for it). The static pass reruns on `static_dirty` (all of
    /// `dirty` when `None`) and on nodes it hasn't seen. Returns the dirty set
    /// and order when they can be reused: one pass, no cycles, no spill changes.
    fn recalc(
        &mut self,
        mut dirty: HashSet<CellKey>,
        mut static_dirty: Option<HashSet<CellKey>>,
        mut order_hint: Option<Vec<CellKey>>,
    ) -> Option<(HashSet<CellKey>, Vec<CellKey>)> {
        self.last_eval_count = 0;
        self.cycles.retain(|cyc| !cyc.iter().any(|k| dirty.contains(k)));
        let mut reusable: Option<Vec<CellKey>> = None;
        for pass in 0..8 {
            // drop results/spills for cells that no longer have content
            let gone: Vec<CellKey> = dirty.iter().filter(|k| !self.nodes.contains_key(k)).copied().collect();
            let mut changed_positions: Vec<CellKey> = Vec::new();
            for k in &gone {
                self.results.remove(k);
                self.statics.remove(k);
                changed_positions.extend(self.set_desired(*k, None));
                if let Some(reg) = self.spills.remove(k) {
                    changed_positions.extend(self.uncover(&reg));
                }
            }
            let order = match order_hint.take() {
                Some(o) => o.into_iter().map(|k| (k, None)).collect(),
                None => self.topo(&dirty),
            };
            if pass == 0 && gone.is_empty() && order.iter().all(|(_, c)| c.is_none()) {
                reusable = Some(order.iter().map(|(k, _)| *k).collect());
            }
            for (k, cycle) in order {
                if let Some(path) = cycle {
                    self.results.insert(
                        k,
                        Err(CellError {
                            msg: format!(
                                "cycle: {}",
                                path.iter().chain(path.first()).map(|c| self.wb.cell_label(*c, Some(k.sheet))).collect::<Vec<_>>().join(" → ")
                            ),
                            span: None,
                            kind: ErrKind::Cycle(path),
                        }),
                    );
                    self.statics.insert(k, Static::any());
                    changed_positions.extend(self.set_desired(k, None));
                    if let Some(reg) = self.spills.remove(&k) {
                        changed_positions.extend(self.uncover(&reg));
                    }
                    continue;
                }
                if static_dirty.as_ref().is_none_or(|s| s.contains(&k)) || !self.statics.contains_key(&k) {
                    let s = self.compute_static(k);
                    self.statics.insert(k, s);
                }
                self.last_eval_count += 1;
                let res = self.evaluate(k);
                let (mut res, changed) = self.place_spill(k, res);
                changed_positions.extend(changed);
                // Stamp the provenance a reference to this cell carries once, here, so
                // every reference is an Arc clone instead of a fresh allocation.
                if let Ok(Value::Num(n)) = &mut res {
                    let p: Vec<Prov> = if self.kind(k) == Kind::Number {
                        vec![Prov::Literal(k); n.len()]
                    } else {
                        (0..n.len()).map(|i| Prov::Derived(k, i)).collect()
                    };
                    n.prov = Some(Arc::new(p));
                }
                self.results.insert(k, res);
            }
            if changed_positions.is_empty() {
                return reusable.map(|o| (dirty, o));
            }
            // spill coverage changed, so eval_deps and closure results may have too
            self.graph_gen += 1;
            reusable = None;
            // Positions whose spill coverage changed: their readers recompute.
            let mut next = Vec::new();
            for p in changed_positions {
                next.extend(self.dependents_of_position(p));
                if self.nodes.contains_key(&p) {
                    next.push(p);
                }
            }
            if next.is_empty() {
                break;
            }
            dirty = self.closure(next);
            // spill coverage changed: what references and ranges read has too
            static_dirty = None;
        }
        None
    }

    /// Dependency order over the dirty set; cycle members come with their cycle.
    fn topo(&mut self, dirty: &HashSet<CellKey>) -> Vec<(CellKey, Option<Vec<CellKey>>)> {
        let mut state: HashMap<CellKey, u8> = HashMap::default();
        let mut in_cycle: HashMap<CellKey, Vec<CellKey>> = HashMap::default();
        let mut order = Vec::with_capacity(dirty.len());
        let mut roots: Vec<CellKey> = dirty.iter().copied().filter(|k| self.nodes.contains_key(k)).collect();
        // position lookups are hash lookups: compute each key once, not per comparison
        roots.sort_by_cached_key(|k| (self.wb.pos(*k), k.sheet));
        for root in roots {
            if state.contains_key(&root) {
                continue;
            }
            let mut stack: Vec<(CellKey, Vec<CellKey>, usize)> = vec![(root, self.eval_deps(root), 0)];
            state.insert(root, 1);
            while let Some(top) = stack.last_mut() {
                if top.2 < top.1.len() {
                    let d = top.1[top.2];
                    top.2 += 1;
                    if !dirty.contains(&d) {
                        continue;
                    }
                    match state.get(&d) {
                        Some(2) => {}
                        Some(1) => {
                            let start = stack.iter().position(|f| f.0 == d).unwrap();
                            let path: Vec<CellKey> = stack[start..].iter().map(|f| f.0).collect();
                            for c in &path {
                                in_cycle.insert(*c, path.clone());
                            }
                            self.cycles.push(path);
                        }
                        _ => {
                            state.insert(d, 1);
                            let deps = self.eval_deps(d);
                            stack.push((d, deps, 0));
                        }
                    }
                } else {
                    let (k, _, _) = stack.pop().unwrap();
                    state.insert(k, 2);
                    order.push((k, in_cycle.get(&k).cloned()));
                }
            }
        }
        order
    }

    fn uncover(&mut self, reg: &Region) -> Vec<CellKey> {
        let keys = self.region_keys(reg);
        for k in &keys {
            self.cover.remove(k);
        }
        keys
    }

    /// Records the region an anchor wants. When it changes, every other anchor
    /// whose wanted region overlaps the old or new one is returned: whether
    /// those are blocked may have changed, so they must be placed again.
    fn set_desired(&mut self, k: CellKey, reg: Option<Region>) -> Vec<CellKey> {
        let old = match reg {
            Some(r) => self.desired.insert(k, r),
            None => self.desired.remove(&k),
        };
        if old == reg {
            return vec![];
        }
        self.desired.iter().filter(|(a, d)| **a != k && [old, reg].iter().flatten().any(|r| r.overlaps(d))).map(|(a, _)| *a).collect()
    }

    /// Updates spill bookkeeping for an anchor; returns positions whose
    /// coverage changed (and anchors to place again). Turns the result into
    /// an error if blocked: by content in its region, or by another anchor
    /// whose wanted region overlaps it — then both are blocked, no matter
    /// which was computed first.
    fn place_spill(&mut self, k: CellKey, res: CellResult) -> (CellResult, Vec<CellKey>) {
        let old = self.spills.remove(&k);
        let mut changed = Vec::new();
        if let Some(reg) = old {
            changed.extend(self.uncover(&reg));
        }
        let size = match &res {
            Ok(v) => v.spill_size(),
            Err(_) => (1, 1),
        };
        let pos = if size == (1, 1) { None } else { self.wb.pos(k) };
        let Some((r0, c0)) = pos else {
            changed.extend(self.set_desired(k, None));
            return (res, changed);
        };
        let reg = Region { sheet: k.sheet, r0, c0, rows: size.0, cols: size.1 };
        changed.extend(self.set_desired(k, Some(reg)));
        if let Some(s) = self.wb.sheet_mut(k.sheet) {
            s.ensure_size(r0 + size.0, c0 + size.1);
        }
        // blocked?
        let keys = self.region_keys(&reg);
        if let Some(b) = keys.iter().copied().find(|p| *p != k && self.nodes.contains_key(p)) {
            let label = self.wb.cell_label(b, Some(k.sheet));
            let err = CellError { msg: format!("#spill blocked: {label} is in the way"), span: None, kind: ErrKind::SpillBlocked(b) };
            return (Err(err), changed);
        }
        let other = self.desired.iter().filter(|(a, d)| **a != k && d.overlaps(&reg)).map(|(a, d)| (d.r0, d.c0, *a)).min();
        if let Some((_, _, b)) = other {
            let label = self.wb.cell_label(b, Some(k.sheet));
            let err = CellError { msg: format!("#spill blocked: overlaps the spill from {label}"), span: None, kind: ErrKind::SpillBlocked(b) };
            return (Err(err), changed);
        }
        for p in &keys {
            if *p != k {
                self.cover.insert(*p, k);
            }
        }
        self.spills.insert(k, reg);
        if old == Some(reg) {
            // same region: readers of the spilled cells are already downstream
            changed.clear();
        } else {
            changed.extend(keys);
        }
        (res, changed)
    }

    fn evaluate(&self, k: CellKey) -> CellResult {
        let Some(n) = self.nodes.get(&k) else { return Err(local("no content".into(), None)) };
        let compiled = match &n.compiled {
            Ok(c) => c.clone(),
            Err(e) => return Err(CellError { msg: e.msg.clone(), span: e.span.clone(), kind: ErrKind::Local }),
        };
        // A unit error found statically is this cell's, whatever its inputs hold.
        if let Some(e) = self.static_error(k) {
            return Err(local(e.msg.clone(), e.span.clone()));
        }
        // An error in a referenced cell is reported as upstream, not here.
        if let Some(up) = self.first_upstream_error(k) {
            let label = self.wb.cell_label(up, Some(k.sheet));
            return Err(CellError { msg: format!("{label} has an error"), span: None, kind: ErrKind::Upstream(up) });
        }
        let env = View(self);
        match &*compiled {
            Compiled::Empty => Err(local("empty".into(), None)),
            Compiled::Text(t) => Ok(Value::text(t)),
            Compiled::Program(ops) => run_program(&env, ops).map_err(|e| local(e.msg, e.span)),
            Compiled::WordDef { name, .. } => {
                self.dup_check(&self.syms.words, name, k, "word")?;
                Ok(Value::Word(name.clone()))
            }
            Compiled::Dim(name) => {
                self.dup_check(&self.syms.dims, name, k, "dimension")?;
                Ok(Value::Dim(name.clone()))
            }
            Compiled::Base { unit, dim, .. } => {
                self.dup_check(&self.syms.units, unit, k, "unit")?;
                Ok(Value::Unit(Arc::new(UnitInfo::new(unit.clone(), Dim::base(dim), 1.0, None, None))))
            }
            Compiled::UnitDef { name, body, offset } => {
                self.dup_check(&self.syms.units, name, k, "unit")?;
                let v = run_program(&env, body).map_err(|e| local(e.msg, e.span))?;
                let Value::Num(n) = v else { return Err(local("a unit is defined by a number with units".into(), None)) };
                let Some(f) = n.as_scalar() else { return Err(local("a unit is defined by a single number".into(), None)) };
                if n.q.absolute.is_some() {
                    return Err(local("a unit can't be defined from an absolute value".into(), None));
                }
                if f == 0.0 || !f.is_finite() {
                    return Err(local("a unit's size must be a finite non-zero number".into(), None));
                }
                Ok(Value::Unit(Arc::new(UnitInfo::new(name.clone(), n.q.dim.clone(), f, *offset, offset.map(|_| n.q.disp.clone())))))
            }
        }
    }

    fn dup_check(&self, table: &HashMap<String, CellKey>, name: &str, k: CellKey, what: &str) -> Result<(), CellError> {
        match table.get(name) {
            Some(owner) if *owner != k => Err(local(
                format!("{what} {name} is already defined at {}", self.wb.cell_label(*owner, Some(k.sheet))),
                None,
            )),
            _ => Ok(()),
        }
    }

    fn first_upstream_error(&self, k: CellKey) -> Option<CellKey> {
        // Only direct cell references; ranges report through range_value.
        let n = self.nodes.get(&k)?;
        for d in &n.deps {
            if let Dep::Cell(p) = d {
                let src = if self.nodes.contains_key(p) { Some(*p) } else { self.cover.get(p).copied() };
                if let Some(src) = src {
                    if src != k && matches!(self.results.get(&src), Some(Err(_))) {
                        return Some(src);
                    }
                }
            }
        }
        None
    }
}

fn local(msg: String, span: Option<Range<usize>>) -> CellError {
    CellError { msg, span, kind: ErrKind::Local }
}

pub fn valid_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok_start = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_');
    if !ok_start || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return Err("names start with a letter and use letters, digits, _ and .".into());
    }
    if crate::parse::builtin(name).is_some() {
        return Err(format!("{name} is a builtin word"));
    }
    if crate::a1::parse_ref(name).is_some() {
        return Err(format!("{name} looks like a cell reference"));
    }
    if ["dim", "base", "offset", "to"].contains(&name) {
        return Err(format!("{name} is reserved"));
    }
    Ok(())
}
