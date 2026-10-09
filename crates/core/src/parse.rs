//! Compiles a cell's text into ops, resolving references, names, units and
//! words against the current symbol table. Also extracts dependencies.

use crate::ids::*;
use crate::lex::{self, Tok, Token};
use crate::model::{classify, Kind, StoredRef, Workbook};
use crate::units::{parse_unit, UnitExpr};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Builtin {
    Add, Sub, Mul, Div, Pow, Neg, Abs, Sqrt, Exp, Log, Log10, Log2, Sin, Cos, Tan,
    Floor, Ceil, Round, Min, Max, Dup, Drop, Swap, Over, Rot, Len, Range, If,
    Lt, Gt, Le, Ge, Eq, Ne, Not, Sum, Mean, Rev, Join, First, Last, Pick, Transpose,
    Filled, Couple, Pi, Line, Scatter, Bar, Layer, Title, XLabel, YLabel, Size,
}

pub const BUILTINS: &[(&str, Builtin, &str)] = {
    use Builtin::*;
    &[
        ("+", Add, "a b → a+b (dims must match)"),
        ("-", Sub, "a b → a-b (dims must match)"),
        ("*", Mul, "a b → a*b (dims multiply)"),
        ("/", Div, "a b → a/b (dims divide)"),
        ("^", Pow, "a n → a^n (n dimensionless)"),
        ("neg", Neg, "a → -a"),
        ("abs", Abs, "a → |a|"),
        ("sqrt", Sqrt, "a → √a (dims halve)"),
        ("exp", Exp, "a → eᵃ (dimensionless)"),
        ("log", Log, "a → ln a (dimensionless)"),
        ("log10", Log10, "a → log₁₀ a"),
        ("log2", Log2, "a → log₂ a"),
        ("sin", Sin, "a → sin a"),
        ("cos", Cos, "a → cos a"),
        ("tan", Tan, "a → tan a"),
        ("floor", Floor, "a → ⌊a⌋ in its display unit"),
        ("ceil", Ceil, "a → ⌈a⌉ in its display unit"),
        ("round", Round, "a → a rounded in its display unit"),
        ("min", Min, "a b → elementwise min"),
        ("max", Max, "a b → elementwise max"),
        ("dup", Dup, "a → a a"),
        ("drop", Drop, "a → "),
        ("swap", Swap, "a b → b a"),
        ("over", Over, "a b → a b a"),
        ("rot", Rot, "a b c → b c a"),
        ("len", Len, "a → length of leading axis"),
        ("range", Range, "n → [0 1 … n-1]"),
        ("if", If, "cond a b → a where cond≠0 else b"),
        ("<", Lt, "a b → a<b"),
        (">", Gt, "a b → a>b"),
        ("<=", Le, "a b → a≤b"),
        (">=", Ge, "a b → a≥b"),
        ("=", Eq, "a b → a=b"),
        ("!=", Ne, "a b → a≠b"),
        ("not", Not, "a → 1 where a=0"),
        ("sum", Sum, "a → sum of every element"),
        ("mean", Mean, "a → mean of every element"),
        ("rev", Rev, "a → a reversed along leading axis"),
        ("join", Join, "a b → a then b along leading axis"),
        ("first", First, "a → first row"),
        ("last", Last, "a → last row"),
        ("pick", Pick, "a i → row i of a (0-based)"),
        ("transpose", Transpose, "a → a with axes swapped"),
        ("filled", Filled, "range → list of its non-empty cells"),
        ("couple", Couple, "a b → 2-row array [a, b]"),
        ("pi", Pi, " → π"),
        ("line", Line, "xs ys → line chart"),
        ("scatter", Scatter, "xs ys → scatter chart"),
        ("bar", Bar, "cats vals → bar chart"),
        ("layer", Layer, "chart chart → combined chart"),
        ("title", Title, "chart \"t\" → chart with title"),
        ("xlabel", XLabel, "chart \"t\" → chart with x label"),
        ("ylabel", YLabel, "chart \"t\" → chart with y label"),
        ("size", Size, "chart cols rows → chart spilling cols×rows cells"),
    ]
};

pub fn builtin(name: &str) -> Option<Builtin> {
    BUILTINS.iter().find(|(n, _, _)| *n == name).map(|(_, b, _)| *b)
}
pub fn builtin_name(b: Builtin) -> &'static str {
    BUILTINS.iter().find(|(_, x, _)| *x == b).map(|(n, _, _)| *n).unwrap()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    Builtin(Builtin),
    User(Arc<str>, CellKey),
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpKind {
    Num(f64),
    Str(Arc<str>),
    Ref(CellKey),
    Range { sheet: SheetId, a: StoredRef, b: StoredRef },
    /// Multiply TOS by a unit. `deps` are the declaring cells, by name.
    Unit(UnitExpr),
    To(UnitExpr),
    Builtin(Builtin),
    Call(Arc<str>, CellKey),
    Local(usize),
    Reduce(Callee),
    Scan(Callee),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub kind: OpKind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Compiled {
    Empty,
    Text(Arc<str>),
    /// Programs and number literals.
    Program(Vec<Op>),
    WordDef { name: Arc<str>, locals: Vec<Arc<str>>, body: Vec<Op>, doc: Option<Arc<str>> },
    Dim(Arc<str>),
    Base { unit: Arc<str>, dim: Arc<str>, dim_cell: CellKey },
    UnitDef { name: Arc<str>, body: Vec<Op>, offset: Option<f64> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Dep {
    Cell(CellKey),
    Range { sheet: SheetId, a: StoredRef, b: StoredRef },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompileError {
    pub msg: String,
    pub span: Option<Range<usize>>,
}

fn err<T>(msg: impl Into<String>, span: Range<usize>) -> Result<T, CompileError> {
    Err(CompileError { msg: msg.into(), span: Some(span) })
}

/// Who defines what. Built from all cells before compiling.
#[derive(Default, Clone, Debug)]
pub struct Symbols {
    pub words: HashMap<String, CellKey>,
    pub units: HashMap<String, CellKey>,
    pub dims: HashMap<String, CellKey>,
}

/// What a declaration cell declares, read without compiling the body
/// (used to build the symbol table).
pub enum Declares {
    Word(String),
    Unit(String),
    Dim(String),
}

pub fn declares(text: &str) -> Option<Declares> {
    match classify(text) {
        Kind::WordDef => {
            let toks = lex::lex_code(text.trim());
            match toks.get(1).map(|t| &t.tok) {
                Some(Tok::Word(w)) => Some(Declares::Word(w.clone())),
                _ => None,
            }
        }
        Kind::UnitDecl => {
            let toks = lex::lex_code(text.trim());
            match (&toks[0].tok, toks.get(1).map(|t| &t.tok)) {
                (Tok::Word(w), Some(Tok::Word(n))) if w == "dim" => Some(Declares::Dim(n.clone())),
                (Tok::Word(w), Some(Tok::Unit(u))) if w == "base" => Some(Declares::Unit(u.trim().to_string())),
                (Tok::Unit(u), _) => Some(Declares::Unit(u.trim().to_string())),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn valid_unit_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| !c.is_whitespace() && !"*/^()[]".contains(c)) && !n.starts_with(|c: char| c.is_ascii_digit())
}

pub struct Compiler<'a> {
    pub wb: &'a Workbook,
    pub syms: &'a Symbols,
    pub home: SheetId,
    pub deps: Vec<Dep>,
}

impl<'a> Compiler<'a> {
    pub fn new(wb: &'a Workbook, syms: &'a Symbols, home: SheetId) -> Self {
        Compiler { wb, syms, home, deps: Vec::new() }
    }

    pub fn compile(&mut self, text: &str) -> Result<Compiled, CompileError> {
        match classify(text) {
            Kind::Empty => Ok(Compiled::Empty),
            Kind::Text => {
                let t = text.trim();
                Ok(Compiled::Text(t.strip_prefix('\'').unwrap_or(t).into()))
            }
            Kind::Number | Kind::Program => {
                let toks = lex::lex_code(text);
                let body = if toks.first().map(|t| &t.tok) == Some(&Tok::Word("=".into())) { &toks[1..] } else { &toks[..] };
                if body.is_empty() {
                    return Err(CompileError { msg: "empty program".into(), span: None });
                }
                Ok(Compiled::Program(self.ops(body, &[])?))
            }
            Kind::WordDef => self.word_def(text),
            Kind::UnitDecl => self.unit_decl(text),
        }
    }

    fn word_def(&mut self, text: &str) -> Result<Compiled, CompileError> {
        let all = lex::lex(text);
        // `: name ( doc ) …` — a comment right after the name documents the word
        let doc = match all.get(2).map(|t| &t.tok) {
            Some(Tok::Comment(c)) => Some(Arc::<str>::from(c.as_str())),
            _ => None,
        };
        let toks: Vec<Token> = all.into_iter().filter(|t| !matches!(t.tok, Tok::Comment(_))).collect();
        let name = match toks.get(1) {
            Some(Token { tok: Tok::Word(w), span }) => {
                if builtin(w).is_some() {
                    return err(format!("{w} is a builtin word"), span.clone());
                }
                if !w.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
                    return err("word names start with a letter", span.clone());
                }
                Arc::<str>::from(w.as_str())
            }
            Some(t) => return err("expected a word name after :", t.span.clone()),
            None => return err("expected a word name after :", 0..1),
        };
        let last = toks.last().unwrap();
        if last.tok != Tok::Word(";".into()) || toks.len() < 3 {
            return err("word definitions end with ;", last.span.clone());
        }
        let mut i = 2;
        let mut locals = Vec::new();
        if toks.get(i).map(|t| &t.tok) == Some(&Tok::Word("{".into())) {
            i += 1;
            loop {
                match toks.get(i) {
                    Some(Token { tok: Tok::Word(w), .. }) if w == "}" => {
                        i += 1;
                        break;
                    }
                    Some(Token { tok: Tok::Word(w), span }) if w != ";" => {
                        if builtin(w).is_some() {
                            return err(format!("{w} is a builtin word"), span.clone());
                        }
                        locals.push(Arc::<str>::from(w.as_str()));
                        i += 1;
                    }
                    Some(t) => return err("expected a local name or }", t.span.clone()),
                    None => return err("missing }", last.span.clone()),
                }
            }
        }
        let body = self.ops(&toks[i..toks.len() - 1], &locals)?;
        Ok(Compiled::WordDef { name, locals, body, doc })
    }

    fn unit_decl(&mut self, text: &str) -> Result<Compiled, CompileError> {
        let toks = lex::lex_code(text);
        match &toks[0].tok {
            Tok::Word(w) if w == "dim" => {
                let Tok::Word(n) = &toks[1].tok else { unreachable!() };
                Ok(Compiled::Dim(n.as_str().into()))
            }
            Tok::Word(w) if w == "base" => {
                let (Tok::Unit(u), Tok::Word(d)) = (&toks[1].tok, &toks[2].tok) else { unreachable!() };
                let u = u.trim();
                if !valid_unit_name(u) {
                    return err("a base unit is a single name", toks[1].span.clone());
                }
                let Some(dc) = self.syms.dims.get(d.as_str()) else {
                    return err(format!("no dimension named {d}; declare it with `dim {d}`"), toks[2].span.clone());
                };
                self.deps.push(Dep::Cell(*dc));
                Ok(Compiled::Base { unit: u.into(), dim: d.as_str().into(), dim_cell: *dc })
            }
            Tok::Unit(u) => {
                let u = u.trim();
                if !valid_unit_name(u) {
                    return err("a unit definition names a single unit", toks[0].span.clone());
                }
                let mut body = &toks[2..];
                let mut offset = None;
                if body.len() >= 2 && body[body.len() - 2].tok == Tok::Word("offset".into()) {
                    match body[body.len() - 1].tok {
                        Tok::Num(o) => offset = Some(o),
                        _ => return err("offset takes a number (in base units)", body[body.len() - 1].span.clone()),
                    }
                    body = &body[..body.len() - 2];
                }
                if body.is_empty() {
                    return err("expected a value after =", toks[1].span.clone());
                }
                let ops = self.ops(body, &[])?;
                Ok(Compiled::UnitDef { name: u.into(), body: ops, offset })
            }
            _ => unreachable!(),
        }
    }

    fn resolve_ref(&self, r: &crate::a1::A1Ref, span: &Range<usize>) -> Result<(SheetId, Option<SheetId>), CompileError> {
        match &r.sheet {
            Some(name) => match self.wb.sheet_by_name(name) {
                Some(s) => Ok((s.id, Some(s.id))),
                None => err(format!("no sheet named {name}"), span.clone()),
            },
            None => Ok((self.home, None)),
        }
    }

    fn stored(&self, sid: SheetId, explicit: Option<SheetId>, r: &crate::a1::A1Ref, span: &Range<usize>) -> Result<StoredRef, CompileError> {
        let sheet = self.wb.sheet(sid).unwrap();
        match sheet.key(r.row, r.col) {
            Some(k) => Ok(StoredRef { sheet: explicit, row: k.row, col: k.col, row_abs: r.row_abs, col_abs: r.col_abs }),
            None => err("reference is outside the sheet", span.clone()),
        }
    }

    fn unit_expr(&mut self, s: &str, span: &Range<usize>) -> Result<UnitExpr, CompileError> {
        let e = parse_unit(s).map_err(|m| CompileError { msg: m, span: Some(span.clone()) })?;
        for (n, _) in &e.terms {
            match self.syms.units.get(n) {
                Some(k) => self.deps.push(Dep::Cell(*k)),
                None => return err(format!("unknown unit {n}"), span.clone()),
            }
        }
        Ok(e)
    }

    fn callee(&mut self, w: &str, span: &Range<usize>) -> Result<Callee, CompileError> {
        if let Some(b) = builtin(w) {
            return Ok(Callee::Builtin(b));
        }
        if let Some(k) = self.syms.words.get(w) {
            self.deps.push(Dep::Cell(*k));
            return Ok(Callee::User(w.into(), *k));
        }
        err(format!("unknown word {w}"), span.clone())
    }

    pub fn ops(&mut self, toks: &[Token], locals: &[Arc<str>]) -> Result<Vec<Op>, CompileError> {
        let mut ops = Vec::with_capacity(toks.len());
        for t in toks {
            let span = t.span.clone();
            let kind = match &t.tok {
                Tok::Num(x) => OpKind::Num(*x),
                Tok::Date(d) => {
                    ops.push(Op { kind: OpKind::Num(*d as f64), span: span.clone() });
                    match self.syms.units.get("date") {
                        Some(k) => self.deps.push(Dep::Cell(*k)),
                        None => return err("dates need a [date] unit to be defined", span),
                    }
                    OpKind::Unit(UnitExpr { terms: vec![("date".into(), crate::rational::Rational::ONE)] })
                }
                Tok::Str(s) => OpKind::Str(s.as_str().into()),
                Tok::Unit(u) => OpKind::Unit(self.unit_expr(u, &span)?),
                Tok::To(u) => OpKind::To(self.unit_expr(u, &span)?),
                Tok::Ref(r) => {
                    let (sid, explicit) = self.resolve_ref(r, &span)?;
                    let s = self.stored(sid, explicit, r, &span)?;
                    let k = CellKey { sheet: sid, row: s.row, col: s.col };
                    self.deps.push(Dep::Cell(k));
                    OpKind::Ref(k)
                }
                Tok::Range(a, b) => {
                    let (sid, explicit) = self.resolve_ref(a, &span)?;
                    let sa = self.stored(sid, explicit, a, &span)?;
                    let sb = self.stored(sid, None, b, &span)?;
                    self.deps.push(Dep::Range { sheet: sid, a: sa, b: sb });
                    OpKind::Range { sheet: sid, a: sa, b: sb }
                }
                Tok::DeadRef => return err("reference to a deleted cell", span),
                Tok::Reduce(w) => OpKind::Reduce(self.callee(w, &span)?),
                Tok::Scan(w) => OpKind::Scan(self.callee(w, &span)?),
                Tok::Bad(m) => return err(m.clone(), span),
                Tok::Comment(_) => continue,
                Tok::Word(w) => {
                    if let Some(i) = locals.iter().position(|l| **l == **w) {
                        OpKind::Local(i)
                    } else if let Some(b) = builtin(w) {
                        OpKind::Builtin(b)
                    } else if let Some(k) = self.syms.words.get(w.as_str()) {
                        self.deps.push(Dep::Cell(*k));
                        OpKind::Call(w.as_str().into(), *k)
                    } else if let Some(nd) = self.wb.names.get(w.as_str()) {
                        self.deps.push(Dep::Cell(nd.cell));
                        OpKind::Ref(nd.cell)
                    } else if w == ":" || w == ";" {
                        return err(format!("{w} only appears in word definitions"), span);
                    } else if w == "offset" {
                        return err("offset only appears at the end of a unit definition", span);
                    } else {
                        return err(format!("unknown word or name {w}"), span);
                    }
                }
            };
            ops.push(Op { kind, span });
        }
        Ok(ops)
    }
}
