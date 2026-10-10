//! The static dimension pass (SPEC §4): an abstract interpretation of a
//! program that tracks what each stack slot is — a number of some dimension,
//! text, a chart — without computing magnitudes. Every rule mirrors eval.rs:
//! where the interpreter would certainly fail on any inputs, this pass fails
//! with the same message at the same token; where the outcome depends on
//! values (shapes, indices, a missing input) it assumes success and goes on.
//! So a static error is always the error evaluation reports once the inputs
//! are there, unless a value-dependent check fails first.

use crate::eval::MAX_DEPTH;
use crate::ids::*;
use crate::model::StoredRef;
use crate::parse::{Builtin, Callee, Compiled, Op, OpKind};
use crate::rational::Rational;
use crate::units::{self, Dim, UnitExpr, UnitInfo};
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

pub use crate::eval::EvalErr;

/// What a number is known to be. `None` fields are unknown.
#[derive(Clone, Debug, PartialEq)]
pub struct SNum {
    pub dim: Option<Dim>,
    /// Absolute (a temperature or date) rather than an amount.
    pub abs: Option<bool>,
    /// The value, for a dimensionless scalar written in the program itself
    /// (`2`, `1 3 /`): exponents of `^` must be known statically.
    pub konst: Option<f64>,
}

/// One stack slot.
#[derive(Clone, Debug, PartialEq)]
pub enum SVal {
    Num(SNum),
    Text,
    Chart,
    /// A unit, dimension or word cell's value.
    Other(&'static str),
    /// Could be anything (an empty input, a cycle, an unknown branch).
    Any,
}

impl SVal {
    pub fn num(dim: Option<Dim>, abs: Option<bool>) -> SVal {
        SVal::Num(SNum { dim, abs, konst: None })
    }
    fn plain() -> SVal {
        SVal::num(Some(Dim::none()), Some(false))
    }
    fn konst(x: f64) -> SVal {
        SVal::Num(SNum { dim: Some(Dim::none()), abs: Some(false), konst: Some(x) })
    }
    /// The same slot without its constant (what a reference or row sees).
    pub fn forget(&self) -> SVal {
        match self {
            SVal::Num(n) => SVal::num(n.dim.clone(), n.abs),
            v => v.clone(),
        }
    }
    /// The dimension, if this is a number whose dimension is known.
    pub fn dim(&self) -> Option<&Dim> {
        match self {
            SVal::Num(n) => n.dim.as_ref(),
            _ => None,
        }
    }
    fn same(&self, o: &SVal) -> bool {
        self.forget() == o.forget()
    }
}

impl fmt::Display for SVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SVal::Num(n) => {
                match &n.dim {
                    Some(d) => write!(f, "{d}")?,
                    None => write!(f, "?")?,
                }
                if n.abs == Some(true) {
                    write!(f, " (absolute)")?;
                }
                Ok(())
            }
            SVal::Text => write!(f, "text"),
            SVal::Chart => write!(f, "chart"),
            SVal::Other(t) => write!(f, "{t}"),
            SVal::Any => write!(f, "?"),
        }
    }
}

/// Why the pass stopped early: a certain error, or something it doesn't
/// model that makes evaluation fail before (a broken word, a deleted range).
pub enum Halt {
    Err(String),
    Stop,
}

pub trait StaticEnv {
    /// What a reference to `k` pushes; `None` if reading it certainly fails
    /// in a way this pass doesn't model.
    fn cell(&self, k: CellKey) -> Option<SVal>;
    fn range(&self, sheet: SheetId, a: &StoredRef, b: &StoredRef, gaps: bool) -> Result<SVal, Halt>;
    /// A unit's dimension (if known statically) and whether it is affine.
    fn unit(&self, name: &str) -> Option<(Option<Dim>, bool)>;
    fn word(&self, k: CellKey) -> Option<Arc<Compiled>>;
}

/// The static result of a program: the slot it leaves, or the first certain
/// error. Where the pass can't follow (a broken word) the result is `Any`.
pub fn analyze(env: &dyn StaticEnv, ops: &[Op]) -> Result<SVal, EvalErr> {
    let mut a = Abs::new(env);
    let mut st = Vec::new();
    match a.run(ops, &mut st, &[]) {
        Ok(()) => finish(st),
        Err(Fail::Err(e)) => Err(e),
        Err(Fail::Stop) => Ok(SVal::Any),
    }
}

/// The inferred stack after each top-level token, and the first certain error.
#[derive(Clone, Debug, Default)]
pub struct StaticTrace {
    pub steps: Vec<(Range<usize>, Vec<SVal>)>,
    pub error: Option<EvalErr>,
}

pub fn analyze_traced(env: &dyn StaticEnv, ops: &[Op]) -> StaticTrace {
    let mut a = Abs::new(env);
    let mut st = Vec::new();
    let mut out = StaticTrace::default();
    for op in ops {
        match a.run(std::slice::from_ref(op), &mut st, &[]) {
            Ok(()) => out.steps.push((op.span.clone(), st.clone())),
            Err(Fail::Err(e)) => {
                out.error = Some(e);
                return out;
            }
            Err(Fail::Stop) => return out,
        }
    }
    out.error = finish(st).err();
    out
}

fn finish(mut st: Vec<SVal>) -> Result<SVal, EvalErr> {
    match st.len() {
        1 => Ok(st.pop().unwrap()),
        0 => Err(EvalErr { msg: "nothing left on the stack".into(), span: None }),
        n => Err(EvalErr { msg: format!("{} values left on stack", n), span: None }),
    }
}

/// The type of a range from its cells' slots, in reading order (cells a `?`
/// range skips are left out; `None` is a cell that certainly fails to read).
/// Mirrors `Engine::range_value`: an error is reported only when every cell
/// before it is certain.
pub fn range_type(cells: &[(CellKey, Option<SVal>)], label: &dyn Fn(CellKey) -> String) -> Result<SVal, Halt> {
    let mut first: Option<(SNum, CellKey)> = None;
    let (mut nums, mut texts, mut certain) = (false, false, true);
    let (mut dim, mut abs) = (None, None);
    for (k, v) in cells {
        match v {
            None => return Err(Halt::Stop),
            Some(SVal::Num(n)) => {
                if texts && certain {
                    return Err(Halt::Err(format!("range mixes text and numbers at {}", label(*k))));
                }
                nums = true;
                if certain {
                    match &first {
                        _ if n.dim.is_none() || n.abs.is_none() => certain = false,
                        None => first = Some((n.clone(), *k)),
                        Some((q0, k0)) => {
                            if q0.dim != n.dim || q0.abs != n.abs {
                                let (d0, d) = (q0.dim.clone().unwrap(), n.dim.clone().unwrap());
                                return Err(Halt::Err(format!("range mixes units: {} is {}, {} is {}", label(*k0), d0, label(*k), d)));
                            }
                        }
                    }
                }
                dim = dim.or(n.dim.clone());
                abs = abs.or(n.abs);
            }
            Some(SVal::Text) => {
                if nums && certain {
                    return Err(Halt::Err(format!("range mixes text and numbers at {}", label(*k))));
                }
                texts = true;
            }
            Some(SVal::Any) => certain = false,
            Some(SVal::Chart | SVal::Other(_)) => return Err(Halt::Stop),
        }
    }
    Ok(match (nums, texts) {
        (true, false) => SVal::num(dim, abs),
        (false, true) => SVal::Text,
        (false, false) if cells.is_empty() => SVal::plain(),
        _ => SVal::Any,
    })
}

enum Fail {
    Err(EvalErr),
    Stop,
}

/// An error either at the current op, already located (from a word body), or a stop.
enum Step {
    Here(String),
    Inner(EvalErr),
    Stop,
}

impl From<String> for Step {
    fn from(s: String) -> Step {
        Step::Here(s)
    }
}
impl From<&str> for Step {
    fn from(s: &str) -> Step {
        Step::Here(s.to_string())
    }
}

type S<T> = Result<T, Step>;

fn pop(st: &mut Vec<SVal>, what: &str) -> S<SVal> {
    st.pop().ok_or_else(|| Step::Here(format!("{what} needs more values on the stack")))
}

fn type_name(v: &SVal) -> &'static str {
    match v {
        SVal::Num(_) => "number",
        SVal::Text => "text",
        SVal::Chart => "chart",
        SVal::Other(t) => t,
        SVal::Any => "value",
    }
}

/// `Some(false)` when certainly not dimensionless (then `dim` is known).
fn dimless(n: &SNum) -> Option<bool> {
    match (&n.dim, n.abs) {
        (Some(d), _) if !d.is_none() => Some(false),
        (Some(_), Some(a)) => Some(!a),
        _ => None,
    }
}

fn scalar_dimless(n: &SNum, what: &str) -> S<()> {
    if dimless(n) == Some(false) {
        return Err(format!("{what} expects a dimensionless number, got {}", n.dim.as_ref().unwrap()).into());
    }
    Ok(())
}

fn same_dims(a: &SNum, b: &SNum, what: &str) -> S<()> {
    if let (Some(x), Some(y)) = (&a.dim, &b.dim) {
        if x != y {
            return Err(format!("{what} needs matching units: {x} vs {y}").into());
        }
    }
    Ok(())
}

/// `rows_of`: only numbers and text split into rows.
fn rows_ok(v: &SVal) -> S<()> {
    match v {
        SVal::Chart | SVal::Other(_) => Err(format!("can't split a {} into rows", type_name(v)).into()),
        _ => Ok(()),
    }
}

fn both<T>(a: Option<T>, b: Option<T>, f: impl Fn(T, T) -> T) -> Option<T> {
    Some(f(a?, b?))
}

/// The result of two slots that must agree for evaluation to succeed.
fn agree<T: PartialEq + Clone>(a: &Option<T>, b: &Option<T>) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) if x == y => Some(x.clone()),
        _ => None,
    }
}

struct Abs<'e> {
    env: &'e dyn StaticEnv,
    depth: usize,
    /// The current op took an unknown slot as a number, text or chart: that
    /// may fail first, so no later failure in the op is certain.
    loose: bool,
}

impl<'e> Abs<'e> {
    fn new(env: &'e dyn StaticEnv) -> Abs<'e> {
        Abs { env, depth: 0, loose: false }
    }

    fn pop_typed(&mut self, st: &mut Vec<SVal>, what: &str) -> S<SVal> {
        let v = pop(st, what)?;
        self.loose |= v == SVal::Any;
        Ok(v)
    }

    fn pop_num(&mut self, st: &mut Vec<SVal>, what: &str) -> S<SNum> {
        match self.pop_typed(st, what)? {
            SVal::Num(n) => Ok(n),
            SVal::Any => Ok(SNum { dim: None, abs: None, konst: None }),
            v => Err(format!("{what} expects a number, got {}", type_name(&v)).into()),
        }
    }

    fn pop_str(&mut self, st: &mut Vec<SVal>, what: &str) -> S<()> {
        match self.pop_typed(st, what)? {
            SVal::Text | SVal::Any => Ok(()),
            v => Err(format!("{what} expects text, got {}", type_name(&v)).into()),
        }
    }

    fn pop_chart(&mut self, st: &mut Vec<SVal>, what: &str) -> S<()> {
        match self.pop_typed(st, what)? {
            SVal::Chart | SVal::Any => Ok(()),
            v => Err(format!("{what} expects a chart, got {}", type_name(&v)).into()),
        }
    }

    fn run(&mut self, ops: &[Op], st: &mut Vec<SVal>, locals: &[SVal]) -> Result<(), Fail> {
        for op in ops {
            self.loose = false;
            self.step(op, st, locals).map_err(|e| match e {
                Step::Here(_) if self.loose => Fail::Stop,
                Step::Here(msg) => Fail::Err(EvalErr { msg, span: Some(op.span.clone()) }),
                Step::Inner(e) => Fail::Err(e),
                Step::Stop => Fail::Stop,
            })?;
        }
        Ok(())
    }

    fn step(&mut self, op: &Op, st: &mut Vec<SVal>, locals: &[SVal]) -> S<()> {
        match &op.kind {
            OpKind::Num(x) => st.push(SVal::konst(*x)),
            OpKind::Str(_) => st.push(SVal::Text),
            OpKind::Ref(k) => st.push(self.env.cell(*k).ok_or(Step::Stop)?),
            OpKind::Range { sheet, a, b, gaps } => match self.env.range(*sheet, a, b, *gaps) {
                Ok(v) => st.push(v),
                Err(Halt::Err(m)) => return Err(Step::Here(m)),
                Err(Halt::Stop) => return Err(Step::Stop),
            },
            OpKind::Unit(u) => {
                let n = self.pop_num(st, "a unit")?;
                let v = match self.resolve(u)? {
                    None => SVal::num(None, None),
                    // `5 [%] [°C]` fails at run time, but the message names display units this pass doesn't track
                    Some(r) if r.absolute.is_some() => SVal::num(Some(r.dim), Some(true)),
                    Some(r) => SVal::num(n.dim.map(|d| d.mul(&r.dim)), Some(false)),
                };
                st.push(v);
            }
            OpKind::To(u) => {
                let n = self.pop_num(st, "to")?;
                let v = match self.resolve(u)? {
                    None => SVal::num(n.dim, n.abs),
                    Some(r) => {
                        if let Some(d) = n.dim.as_ref().filter(|d| **d != r.dim) {
                            return Err(format!("can't show {} in {}: dimensions differ ({} vs {})", d, r.disp, d, r.dim).into());
                        }
                        let abs = match (r.absolute.is_some(), n.abs) {
                            // only once the dimension check above is certain to pass
                            (true, Some(false)) if n.dim.is_some() => {
                                return Err(format!("{} is for absolute values; this is a difference — use its Δ unit", r.disp).into())
                            }
                            (true, _) => Some(true),
                            (false, a) => a,
                        };
                        SVal::num(Some(r.dim), abs)
                    }
                };
                st.push(v);
            }
            OpKind::Local(i) => st.push(locals[*i].clone()),
            OpKind::Builtin(b) => self.builtin(*b, st)?,
            OpKind::Call(name, k) => self.call(name, *k, st, &op.span)?,
            OpKind::Reduce(c) => {
                let v = pop(st, "reduce")?;
                rows_ok(&v)?;
                // one row is the result itself, so only a word that keeps a row's type has a static result
                let row = v.forget();
                let r = match self.apply_quietly(c, &row, &op.span) {
                    Some(r) if r.same(&row) => match (&c, &row) {
                        // an empty array sums to zero of its linear unit
                        (Callee::Builtin(Builtin::Add), SVal::Num(n)) if n.abs != Some(false) => SVal::num(n.dim.clone(), None),
                        _ => row,
                    },
                    _ => SVal::Any,
                };
                st.push(r);
            }
            OpKind::Scan(c) => {
                let v = pop(st, "scan")?;
                rows_ok(&v)?;
                let row = v.forget();
                let r = match self.apply_quietly(c, &row, &op.span) {
                    Some(r) if r.same(&row) => row,
                    _ => SVal::Any,
                };
                st.push(r);
            }
        }
        Ok(())
    }

    /// The slot a callee leaves when applied to two `row`s, if it certainly
    /// leaves exactly one (errors here depend on how many rows there are).
    fn apply_quietly(&mut self, c: &Callee, row: &SVal, span: &Range<usize>) -> Option<SVal> {
        let mut st = vec![row.clone(), row.clone()];
        let loose = self.loose;
        let ok = match c {
            Callee::Builtin(b) => self.builtin(*b, &mut st).is_ok(),
            Callee::User(name, k) => self.call(name, *k, &mut st, span).is_ok(),
        };
        self.loose = loose;
        if ok && st.len() == 1 {
            st.pop()
        } else {
            None
        }
    }

    fn call(&mut self, name: &str, k: CellKey, st: &mut Vec<SVal>, span: &Range<usize>) -> S<()> {
        // a word that isn't defined is an error in its own cell, reported upstream
        let Some(def) = self.env.word(k) else { return Err(Step::Stop) };
        let Compiled::WordDef { locals, body, .. } = &*def else { return Err(Step::Stop) };
        if self.depth >= MAX_DEPTH {
            return Err(format!("{name}: words nested more than {MAX_DEPTH} deep").into());
        }
        if st.len() < locals.len() {
            return Err(format!("{name} takes {} values from the stack", locals.len()).into());
        }
        let args: Vec<SVal> = st.split_off(st.len() - locals.len());
        self.depth += 1;
        let r = self.run(body, st, &args);
        self.depth -= 1;
        r.map_err(|e| match e {
            Fail::Err(e) => Step::Inner(EvalErr { msg: format!("in {name}: {}", e.msg), span: Some(span.clone()) }),
            Fail::Stop => Step::Stop,
        })
    }

    /// Resolves a bracketed unit with the units' static dimensions; `None`
    /// when one of them isn't known.
    fn resolve(&self, u: &UnitExpr) -> S<Option<units::Resolved>> {
        const UNKNOWN: &str = "\0unknown";
        let r = units::resolve(u, &mut |n| match self.env.unit(n) {
            Some((Some(dim), affine)) => Ok(UnitInfo { name: n.into(), dim, factor: 1.0, affine: affine.then_some(0.0), delta: None }),
            _ => Err(UNKNOWN.into()),
        });
        match r {
            Ok(r) => Ok(Some(r)),
            Err(m) if m == UNKNOWN => Ok(None),
            Err(m) => Err(m.into()),
        }
    }

    fn builtin(&mut self, b: Builtin, st: &mut Vec<SVal>) -> S<()> {
        use Builtin::*;
        let name = crate::parse::builtin_name(b);
        match b {
            Dup => {
                let v = st.last().cloned().ok_or("dup needs a value")?;
                st.push(v);
            }
            Drop => {
                pop(st, name)?;
            }
            Swap => {
                let b = pop(st, name)?;
                let a = pop(st, name)?;
                st.push(b);
                st.push(a);
            }
            Over => {
                if st.len() < 2 {
                    return Err("over needs two values".into());
                }
                st.push(st[st.len() - 2].clone());
            }
            Rot => {
                if st.len() < 3 {
                    return Err("rot needs three values".into());
                }
                let a = st.remove(st.len() - 3);
                st.push(a);
            }
            Add | Sub => {
                let y = self.pop_num(st, name)?;
                let x = self.pop_num(st, name)?;
                same_dims(&x, &y, name)?;
                // the checks on absolute values come after the dimension check, so they're certain only when it is
                let sure = x.dim.is_some() && y.dim.is_some();
                let abs = match (b, x.abs, y.abs) {
                    (Add, Some(true), Some(true)) if sure => {
                        return Err("can't add two absolute values (temperatures, dates); subtract them or add a Δ".into())
                    }
                    (Sub, Some(false), Some(true)) if sure => return Err("can't subtract an absolute value from a difference".into()),
                    (Add, Some(true), Some(true)) | (Sub, Some(false), Some(true)) => None,
                    (Add, Some(a), Some(b)) => Some(a || b),
                    (Add, _, Some(true)) | (Add, Some(true), _) => Some(true),
                    (Sub, Some(true), Some(true)) | (Sub, None, Some(true)) => Some(false),
                    (Sub, Some(a), Some(false)) => Some(a),
                    (Sub, Some(false), None) => Some(false),
                    _ => None,
                };
                let konst = both(x.konst, y.konst, |p, q| if b == Add { p + q } else { p - q });
                st.push(SVal::Num(SNum { dim: x.dim.or(y.dim), abs, konst }));
            }
            Mul | Div => {
                let y = self.pop_num(st, name)?;
                let x = self.pop_num(st, name)?;
                let dim = both(x.dim, y.dim.map(|d| if b == Mul { d } else { d.inv() }), |a, c| a.mul(&c));
                let konst = both(x.konst, y.konst, |a, c| if b == Mul { a * c } else { a / c });
                st.push(SVal::Num(SNum { dim, abs: Some(false), konst }));
            }
            Pow => {
                let e = self.pop_num(st, name)?;
                let x = self.pop_num(st, name)?;
                if dimless(&e) == Some(false) {
                    return Err(format!("^ needs a dimensionless exponent, got {}", e.dim.as_ref().unwrap()).into());
                }
                let v = match &x.dim {
                    Some(d) if d.is_none() => {
                        SVal::Num(SNum { dim: Some(Dim::none()), abs: Some(false), konst: both(x.konst, e.konst, f64::powf) })
                    }
                    Some(d) => match e.konst {
                        Some(s) => {
                            let r = Rational::from_f64(s).ok_or("^ on a quantity with units needs a simple fractional exponent")?;
                            SVal::num(Some(d.pow(r)), Some(false))
                        }
                        None => SVal::num(None, Some(false)),
                    },
                    None => SVal::num(None, Some(false)),
                };
                st.push(v);
            }
            Sqrt => {
                let x = self.pop_num(st, name)?;
                st.push(SVal::num(x.dim.map(|d| d.pow(Rational::new(1, 2))), Some(false)));
            }
            Neg | Abs => {
                let x = self.pop_num(st, name)?;
                if x.abs == Some(true) {
                    return Err(format!("{name} of an absolute value (temperature, date) is meaningless").into());
                }
                let konst = x.konst.map(|v| if b == Neg { -v } else { v.abs() });
                st.push(SVal::Num(SNum { dim: x.dim, abs: Some(false), konst }));
            }
            Exp | Log | Log10 | Log2 | Sin | Cos | Tan => {
                let x = self.pop_num(st, name)?;
                if dimless(&x) == Some(false) {
                    return Err(format!("{name} needs a dimensionless argument, got {}", x.dim.as_ref().unwrap()).into());
                }
                st.push(SVal::plain());
            }
            Floor | Ceil | Round => {
                let x = self.pop_num(st, name)?;
                st.push(SVal::num(x.dim, x.abs));
            }
            Min | Max | Lt | Gt | Le | Ge | Eq | Ne => {
                let y = self.pop_num(st, name)?;
                let x = self.pop_num(st, name)?;
                same_dims(&x, &y, name)?;
                if let (Some(a), Some(c), true) = (x.abs, y.abs, x.dim.is_some() && y.dim.is_some()) {
                    if a != c {
                        return Err(format!("{name} can't compare an absolute value with a difference").into());
                    }
                }
                st.push(if matches!(b, Min | Max) { SVal::num(x.dim.or(y.dim), x.abs.or(y.abs)) } else { SVal::plain() });
            }
            Not => {
                let x = self.pop_num(st, name)?;
                if dimless(&x) == Some(false) {
                    return Err("not needs a dimensionless argument".into());
                }
                st.push(SVal::plain());
            }
            If => {
                let e = pop(st, name)?;
                let t = pop(st, name)?;
                let c = self.pop_num(st, name)?;
                if dimless(&c) == Some(false) {
                    return Err("if needs a dimensionless condition".into());
                }
                let v = match (t, e) {
                    (SVal::Num(t), SVal::Num(e)) => {
                        // after the condition's check
                        if dimless(&c) == Some(true) {
                            same_dims(&t, &e, name)?;
                        }
                        SVal::num(t.dim.or(e.dim), agree(&t.abs, &e.abs))
                    }
                    (SVal::Text, SVal::Text) => SVal::Text,
                    (SVal::Chart, SVal::Chart) => SVal::Chart,
                    _ => SVal::Any,
                };
                st.push(v);
            }
            Len => {
                let v = pop(st, name)?;
                if let SVal::Chart | SVal::Other(_) = v {
                    return Err(format!("len of a {}", type_name(&v)).into());
                }
                st.push(SVal::plain());
            }
            Range => {
                let n = self.pop_num(st, name)?;
                scalar_dimless(&n, name)?;
                st.push(SVal::plain());
            }
            Sum | Mean => {
                let x = self.pop_num(st, name)?;
                // the sum of an empty array is in the linear unit; of several absolute values, an error
                let abs = if b == Sum && x.abs != Some(false) { None } else { x.abs };
                st.push(SVal::num(x.dim, abs));
            }
            Rev | First | Last | Transpose => {
                let v = pop(st, name)?;
                if b != Transpose {
                    rows_ok(&v)?;
                }
                st.push(v.forget());
            }
            Join => {
                let y = pop(st, name)?;
                let x = pop(st, name)?;
                // an empty side adds no rows, so only agreeing sides give a static result
                let v = match (x, y) {
                    (SVal::Num(x), SVal::Num(y)) => SVal::num(agree(&x.dim, &y.dim), agree(&x.abs, &y.abs)),
                    (SVal::Text, SVal::Text) => SVal::Text,
                    _ => SVal::Any,
                };
                st.push(v);
            }
            Pick => {
                let i = self.pop_num(st, name)?;
                let v = pop(st, name)?;
                scalar_dimless(&i, name)?;
                // a non-scalar index fails first
                if i.konst.is_some() {
                    rows_ok(&v)?;
                }
                st.push(v.forget());
            }
            Couple => {
                let y = pop(st, name)?;
                let x = pop(st, name)?;
                let v = match (&x, &y) {
                    (SVal::Any, _) | (_, SVal::Any) => SVal::Any,
                    (SVal::Num(a), SVal::Num(c)) => SVal::num(agree(&a.dim, &c.dim), agree(&a.abs, &c.abs)),
                    (SVal::Num(_), o) => return Err(format!("couple: mixed numbers and {}", type_name(o)).into()),
                    (SVal::Text, SVal::Text) => SVal::Text,
                    (SVal::Text, o) => return Err(format!("couple: mixed text and {}", type_name(o)).into()),
                    (o, _) => return Err(format!("couple: can't make an array of {}", type_name(o)).into()),
                };
                st.push(v);
            }
            Pi => st.push(SVal::konst(std::f64::consts::PI)),
            Line | Scatter | Bar => {
                self.pop_num(st, name)?;
                pop(st, name)?;
                st.push(SVal::Chart);
            }
            Layer => {
                self.pop_chart(st, name)?;
                self.pop_chart(st, name)?;
                st.push(SVal::Chart);
            }
            Title | XLabel | YLabel => {
                self.pop_str(st, name)?;
                // text from a range may be a list, which fails first
                self.pop_chart(st, name).map_err(|_| Step::Stop)?;
                st.push(SVal::Chart);
            }
            Size => {
                let rows = self.pop_num(st, name)?;
                let cols = self.pop_num(st, name)?;
                scalar_dimless(&rows, name)?;
                if rows.konst.is_some() {
                    scalar_dimless(&cols, name)?;
                }
                // after the value checks on rows and cols
                self.pop_chart(st, name).map_err(|_| Step::Stop)?;
                st.push(SVal::Chart);
            }
        }
        Ok(())
    }
}
