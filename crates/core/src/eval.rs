//! The stack interpreter. A program runs on an empty stack; the cell's value
//! is the single value left on it.

use crate::chart::{Chart, Mark, Xs};
use crate::ids::*;
use crate::model::{StoredRef, MAX_INDEX};
use crate::parse::{Builtin, Callee, Compiled, Op, OpKind};
use crate::rational::Rational;
use crate::units::{self, Dim, DispUnit, Quant, UnitExpr, UnitInfo};
use crate::value::{broadcast2, Num, Provs, Text, Value};
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct EvalErr {
    pub msg: String,
    pub span: Option<Range<usize>>,
}

pub trait Env {
    /// Value of a referenced cell: the whole array for a spill source, the
    /// element for a spilled-into cell.
    fn cell_value(&self, k: CellKey) -> Result<Value, String>;
    /// A range as one array. With `gaps` (`A1:B5?`) its empty cells are
    /// skipped and the rest form a list, row by row.
    fn range_value(&self, sheet: SheetId, a: &StoredRef, b: &StoredRef, gaps: bool) -> Result<Value, String>;
    fn unit_info(&self, name: &str) -> Result<Arc<UnitInfo>, String>;
    fn word(&self, k: CellKey) -> Option<Arc<Compiled>>;
    /// The `sensitivity` chart for element `i` of cell `k` (`Engine::tornado`).
    fn tornado(&self, _k: CellKey, _i: usize) -> Result<Value, String> {
        Err("sensitivity isn't available here".into())
    }
}

type R<T> = Result<T, String>;

pub struct Interp<'e> {
    env: &'e dyn Env,
    depth: usize,
}

pub(crate) const MAX_DEPTH: usize = 64;

thread_local! {
    /// Stacks to reuse: a recalc runs a program per cell, and each would allocate (and grow) its own.
    static STACKS: std::cell::RefCell<Vec<Vec<Value>>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn run_program(env: &dyn Env, ops: &[Op]) -> Result<Value, EvalErr> {
    let mut it = Interp { env, depth: 0 };
    let mut stack = STACKS.with(|s| s.borrow_mut().pop()).unwrap_or_default();
    let res = it.run(ops, &mut stack, &[]).and_then(|()| match stack.len() {
        1 => Ok(stack.pop().unwrap()),
        0 => Err(EvalErr { msg: "nothing left on the stack".into(), span: None }),
        n => Err(EvalErr { msg: format!("{} values left on stack", n), span: None }),
    });
    stack.clear();
    STACKS.with(|s| s.borrow_mut().push(stack));
    res
}

/// The stack after one top-level token (word calls are not stepped into).
#[derive(Clone, Debug)]
pub struct TraceStep {
    pub span: Range<usize>,
    pub stack: Vec<Value>,
}

/// Like `run_program`, recording the stack after every token. Stops at the
/// first error, which is returned alongside the steps that succeeded.
pub fn run_traced(env: &dyn Env, ops: &[Op]) -> (Vec<TraceStep>, Result<Value, EvalErr>) {
    let mut it = Interp { env, depth: 0 };
    let mut stack = Vec::new();
    let mut steps = Vec::with_capacity(ops.len());
    for op in ops {
        if let Err(e) = it.run(std::slice::from_ref(op), &mut stack, &[]) {
            return (steps, Err(e));
        }
        steps.push(TraceStep { span: op.span.clone(), stack: stack.clone() });
    }
    let res = match stack.len() {
        1 => Ok(stack.pop().unwrap()),
        0 => Err(EvalErr { msg: "nothing left on the stack".into(), span: None }),
        n => Err(EvalErr { msg: format!("{} values left on stack", n), span: None }),
    };
    (steps, res)
}

fn pop(stack: &mut Vec<Value>, what: &str) -> R<Value> {
    stack.pop().ok_or_else(|| format!("{what} needs more values on the stack"))
}

fn pop_num(stack: &mut Vec<Value>, what: &str) -> R<Num> {
    match pop(stack, what)? {
        Value::Num(n) => Ok(n),
        v => Err(format!("{what} expects a number, got {}", v.type_name())),
    }
}

fn pop_str(stack: &mut Vec<Value>, what: &str) -> R<String> {
    match pop(stack, what)? {
        Value::Text(t) if t.shape.is_empty() => Ok(t.data[0].to_string()),
        v => Err(format!("{what} expects text, got {}", v.type_name())),
    }
}

fn pop_chart(stack: &mut Vec<Value>, what: &str) -> R<Chart> {
    match pop(stack, what)? {
        Value::Chart(c) => Ok((*c).clone()),
        v => Err(format!("{what} expects a chart, got {}", v.type_name())),
    }
}

fn scalar_dimless(n: &Num, what: &str) -> R<f64> {
    if !n.q.is_dimensionless() {
        return Err(format!("{what} expects a dimensionless number, got {}", n.q.dim));
    }
    n.as_scalar().ok_or_else(|| format!("{what} expects a single number, got shape {:?}", n.shape))
}

fn same_dims(a: &Quant, b: &Quant, what: &str) -> R<()> {
    if a.dim != b.dim {
        return Err(format!("{what} needs matching units: {} vs {}", a.dim, b.dim));
    }
    Ok(())
}

fn map(n: &Num, q: Quant, f: impl Fn(f64) -> f64) -> Num {
    if n.shape.is_empty() {
        return Num::scalar(f(n.data[0]), q);
    }
    Num::with_shape(n.shape.clone(), n.data.iter().map(|x| f(*x)).collect(), q)
}

fn zip(a: &Num, b: &Num, q: Quant, f: impl Fn(f64, f64) -> f64) -> R<Num> {
    // two scalars: the common case, without building a shape or a vector
    if a.shape.is_empty() && b.shape.is_empty() {
        return Ok(Num::scalar(f(a.data[0], b.data[0]), q));
    }
    let (shape, data) = broadcast2(a, b, f)?;
    Ok(Num::with_shape(shape, data, q))
}

// ---- tangents ------------------------------------------------------------------------------
// A sensitivity pass (`Engine::sensitivities`) carries each value's derivative with respect to one
// input alongside it (`Num::tan`). Every word that computes says how a tangent goes through it, next
// to how the value does; words that only move elements around move their tangents with them. A
// plain recalc has no tangents, and each rule below starts by checking for none.

/// `p·t`, where a zero tangent stays zero even when `p` isn't finite (`sqrt` at 0, `/` by 0): an
/// input that doesn't reach this element can't make its derivative NaN.
fn lin(p: f64, t: f64) -> f64 {
    if t == 0.0 {
        0.0
    } else {
        p * t
    }
}

/// `map` with the chain rule: each tangent times `df` at its element.
fn map_d(n: &Num, q: Quant, f: impl Fn(f64) -> f64, df: impl Fn(f64) -> f64) -> Num {
    let r = map(n, q, f);
    match &n.tan {
        None => r,
        Some(t) => r.with_tan(Some(n.data.iter().zip(t.iter()).map(|(x, t)| lin(df(*x), *t)).collect())),
    }
}

/// `zip` with tangents: `d(x, y, tx, ty)` for each pair of elements, paired as `broadcast2` pairs them.
fn zip_d(a: &Num, b: &Num, q: Quant, f: impl Fn(f64, f64) -> f64, d: impl Fn(f64, f64, f64, f64) -> f64) -> R<Num> {
    let r = zip(a, b, q, f)?;
    if a.tan.is_none() && b.tan.is_none() {
        return Ok(r);
    }
    let t = pairs(a, b, |i, j| d(a.data[i], b.data[j], a.tan_at(i), b.tan_at(j)));
    Ok(r.with_tan(Some(t)))
}

/// `f(i, j)` for element `i` of `a` and `j` of `b`, for each pair `broadcast2` makes (it has already
/// checked that the shapes fit).
fn pairs(a: &Num, b: &Num, f: impl Fn(usize, usize) -> f64) -> Vec<f64> {
    let (la, lb) = (a.len(), b.len());
    if a.shape == b.shape {
        (0..la).map(|i| f(i, i)).collect()
    } else if a.shape.len() <= b.shape.len() && b.shape.starts_with(&a.shape) {
        let inner = (lb / la.max(1)).max(1);
        (0..lb).map(|i| f(i / inner, i)).collect()
    } else {
        let inner = (la / lb.max(1)).max(1);
        (0..la).map(|i| f(i, i / inner)).collect()
    }
}

/// A value's tangents as an array of their own (zeros where it has none), for words that pick elements.
fn tangents(n: &Num) -> Num {
    Num::with_shape(n.shape.clone(), (0..n.len()).map(|i| n.tan_at(i)).collect(), Quant::none())
}

/// The sum of a value's tangents, in the same order as its values are summed.
fn tan_sum(n: &Num) -> Option<f64> {
    n.tan.as_ref().map(|t| t.iter().sum())
}

/// `+ - * /` on the two scalars on top of the stack, written into the lower one, when
/// the result's units are simply the lower one's: same dimension for `+ -` (neither
/// absolute), plain numbers for `* /`. The general path gives the same result; this
/// one skips building a new value (most of a numeric model's work).
fn arith_in_place(b: Builtin, st: &mut [Value]) -> bool {
    let [.., Value::Num(x), Value::Num(y)] = st else { return false };
    if !x.shape.is_empty() || !y.shape.is_empty() || x.q.absolute.is_some() || y.q.absolute.is_some() || x.tan.is_some() || y.tan.is_some() {
        return false;
    }
    let plain = |q: &Quant| q.dim.is_none() && q.disp.is_none() && q.disp.factor == 1.0 && q.disp.offset == 0.0;
    let (a, c) = (x.data[0], y.data[0]);
    let v = match b {
        Builtin::Add if x.q.dim == y.q.dim => a + c,
        Builtin::Sub if x.q.dim == y.q.dim => a - c,
        Builtin::Mul if plain(&x.q) && plain(&y.q) => a * c,
        Builtin::Div if plain(&x.q) && plain(&y.q) => a / c,
        _ => return false,
    };
    x.data = crate::value::Data::One(v);
    x.prov = Provs::None;
    true
}

fn bool_q() -> Quant {
    Quant::none()
}

/// Elements `from..from + n` of `p`'s provenance.
fn slice_prov(p: &Provs, from: usize, n: usize) -> Provs {
    match (p, n) {
        (Provs::None, _) => Provs::None,
        (_, 1) => Provs::one(p.get(from)),
        _ => Provs::List((from..from + n).map(|i| p.get(i)).collect()),
    }
}

/// `v` without provenance: reduce and scan compute (even a one-row reduce, or a scan's first row).
fn untraced(mut v: Value) -> Value {
    if let Value::Num(n) = &mut v {
        n.prov = Provs::None;
    }
    v
}

/// How many rows `rows_of` gives: the leading axis's length (a scalar is one row).
fn row_count(v: &Value) -> R<usize> {
    match v {
        Value::Num(_) | Value::Text(_) => Ok(v.shape().first().copied().unwrap_or(1)),
        v => Err(format!("can't split a {} into rows", v.type_name())),
    }
}

/// Row `i` along the leading axis (`i` < `row_count`), with its elements' provenance. Builds only that
/// row: `pick` in a step word runs once per step, on arrays as long as the steps.
fn row_of(v: &Value, i: usize) -> R<Value> {
    match v {
        Value::Num(n) => {
            if n.shape.is_empty() {
                return Ok(v.clone());
            }
            let inner: usize = n.shape[1..].iter().product();
            let mut r = Num::with_shape(n.shape[1..].to_vec(), n.data[i * inner..(i + 1) * inner].to_vec(), n.q.clone());
            r.prov = slice_prov(&n.prov, i * inner, inner);
            if let Some(t) = &n.tan {
                r.set_tan(Some(t[i * inner..(i + 1) * inner].to_vec()));
            }
            Ok(Value::Num(r))
        }
        Value::Text(t) => {
            if t.shape.is_empty() {
                return Ok(v.clone());
            }
            let inner: usize = t.shape[1..].iter().product();
            Ok(Value::Text(Text { shape: t.shape[1..].to_vec(), data: Arc::new(t.data[i * inner..(i + 1) * inner].to_vec()) }))
        }
        v => Err(format!("can't split a {} into rows", v.type_name())),
    }
}

/// Split along the leading axis. Each row keeps its elements' provenance.
fn rows_of(v: &Value) -> R<Vec<Value>> {
    (0..row_count(v)?).map(|i| row_of(v, i)).collect()
}

/// Stack values (all the same shape and units) along a new leading axis, with their elements' provenance.
fn from_rows(rows: Vec<Value>, what: &str) -> R<Value> {
    let Some(first) = rows.first() else { return Ok(Value::Num(Num::vector(vec![], Quant::none()))) };
    match first {
        Value::Num(f) => {
            let mut data = Vec::with_capacity(rows.len() * f.len());
            let traced = rows.iter().any(|r| matches!(r, Value::Num(n) if !n.prov.is_none()));
            let mut prov = Vec::with_capacity(if traced { rows.len() * f.len() } else { 0 });
            let tanned = rows.iter().any(|r| matches!(r, Value::Num(n) if n.tan.is_some()));
            let mut tan = Vec::with_capacity(if tanned { rows.len() * f.len() } else { 0 });
            for r in &rows {
                let Value::Num(n) = r else { return Err(format!("{what}: mixed numbers and {}", r.type_name())) };
                if n.shape != f.shape {
                    return Err(format!("{what}: rows have different shapes {:?} and {:?}", f.shape, n.shape));
                }
                if n.q.dim != f.q.dim || n.q.absolute.is_some() != f.q.absolute.is_some() {
                    return Err(format!("{what}: rows have different units {} and {}", f.q.dim, n.q.dim));
                }
                data.extend_from_slice(&n.data);
                if traced {
                    prov.extend((0..n.len()).map(|i| n.prov.get(i)));
                }
                if tanned {
                    tan.extend((0..n.len()).map(|i| n.tan_at(i)));
                }
            }
            let mut shape = vec![rows.len()];
            shape.extend_from_slice(&f.shape);
            let q = match &rows.last().unwrap() {
                Value::Num(n) => n.q.clone(),
                _ => unreachable!(),
            };
            let mut n = Num::with_shape(shape, data, q).with_tan(tanned.then_some(tan));
            if traced {
                n.prov = Provs::List(prov.into());
            }
            Ok(Value::Num(n))
        }
        Value::Text(f) => {
            let mut data = Vec::new();
            for r in &rows {
                let Value::Text(t) = r else { return Err(format!("{what}: mixed text and {}", r.type_name())) };
                if t.shape != f.shape {
                    return Err(format!("{what}: rows have different shapes"));
                }
                data.extend(t.data.iter().cloned());
            }
            let mut shape = vec![rows.len()];
            shape.extend_from_slice(&f.shape);
            Ok(Value::Text(Text { shape, data: Arc::new(data) }))
        }
        v => Err(format!("{what}: can't make an array of {}", v.type_name())),
    }
}

impl<'e> Interp<'e> {
    fn run(&mut self, ops: &[Op], stack: &mut Vec<Value>, locals: &[Value]) -> Result<(), EvalErr> {
        for op in ops {
            self.step(op, stack, locals).map_err(|e| match e {
                Step::Here(msg) => EvalErr { msg, span: Some(op.span.clone()) },
                Step::Inner(e) => e,
            })?;
        }
        Ok(())
    }

    fn step(&mut self, op: &Op, stack: &mut Vec<Value>, locals: &[Value]) -> Result<(), Step> {
        match &op.kind {
            OpKind::Num(x) => stack.push(Value::Num(Num::plain(*x))),
            OpKind::Str(s) => stack.push(Value::Text(Text { shape: vec![], data: Arc::new(vec![s.clone()]) })),
            OpKind::Ref(k) => stack.push(self.env.cell_value(*k)?),
            OpKind::Range(r) => stack.push(self.env.range_value(r.sheet, &r.a, &r.b, r.gaps)?),
            OpKind::Unit(u) => {
                let n = pop_num(stack, "a unit")?;
                stack.push(Value::Num(self.apply_unit(n, u)?));
            }
            OpKind::To(u) => {
                let n = pop_num(stack, "to")?;
                stack.push(Value::Num(self.convert(n, u)?));
            }
            OpKind::Local(i) => stack.push(locals[*i].clone()),
            OpKind::Builtin(b) => self.builtin(*b, stack, &op.span)?,
            OpKind::Call(c) => self.call(&c.0, c.1, stack, &op.span)?,
            OpKind::Reduce(c) => {
                let v = pop(stack, "reduce")?;
                stack.push(self.reduce(c, v, &op.span)?);
            }
            OpKind::Scan(c) => {
                let v = pop(stack, "scan")?;
                stack.push(self.scan(c, v, &op.span)?);
            }
            OpKind::Quote(c) => stack.push(Value::Fn((**c).clone())),
        }
        Ok(())
    }

    fn call(&mut self, name: &str, k: CellKey, stack: &mut Vec<Value>, span: &Range<usize>) -> Result<(), Step> {
        let Some(def) = self.env.word(k) else { return Err(Step::Here(format!("word {name} is not defined"))) };
        let Compiled::WordDef { locals, body, .. } = &*def else { return Err(Step::Here(format!("{name} is not a word"))) };
        if self.depth >= MAX_DEPTH {
            return Err(Step::Here(format!("{name}: words nested more than {MAX_DEPTH} deep")));
        }
        if stack.len() < locals.len() {
            return Err(Step::Here(format!("{name} takes {} values from the stack", locals.len())));
        }
        let args: Vec<Value> = stack.split_off(stack.len() - locals.len());
        self.depth += 1;
        let r = self.run(body, stack, &args);
        self.depth -= 1;
        r.map_err(|e| Step::Inner(EvalErr { msg: format!("in {name}: {}", e.msg), span: Some(span.clone()) }))
    }

    fn apply_callee(&mut self, c: &Callee, stack: &mut Vec<Value>, span: &Range<usize>) -> Result<(), Step> {
        match c {
            Callee::Builtin(b) => self.builtin(*b, stack, span),
            Callee::User(name, k) => self.call(name, *k, stack, span),
        }
    }

    fn reduce(&mut self, c: &Callee, v: Value, span: &Range<usize>) -> Result<Value, Step> {
        let v = untraced(v);
        let rows = rows_of(&v)?;
        if rows.is_empty() {
            if let (Callee::Builtin(Builtin::Add), Value::Num(n)) = (c, &v) {
                return Ok(Value::Num(Num::scalar(0.0, n.q.linear())));
            }
            return Err(Step::Here("can't reduce an empty array".into()));
        }
        // fast path for sums of numbers
        if let (Callee::Builtin(Builtin::Add), Value::Num(n)) = (c, &v) {
            if n.shape.len() == 1 {
                if n.q.absolute.is_some() && n.len() > 1 {
                    return Err(Step::Here("can't add absolute values (temperatures, dates)".into()));
                }
                return Ok(Value::Num(Num::scalar(n.data.iter().sum(), n.q.clone()).with_tan(tan_sum(n).map(|t| vec![t]))));
            }
        }
        let mut it = rows.into_iter();
        let mut acc = it.next().unwrap();
        let mut st = Vec::new();
        for r in it {
            st.clear();
            st.push(acc);
            st.push(r);
            self.apply_callee(c, &mut st, span)?;
            if st.len() != 1 {
                return Err(Step::Here("reduce needs a word that turns two values into one".into()));
            }
            acc = st.pop().unwrap();
        }
        Ok(acc)
    }

    fn scan(&mut self, c: &Callee, v: Value, span: &Range<usize>) -> Result<Value, Step> {
        let v = untraced(v);
        let rows = rows_of(&v)?;
        let mut out = Vec::with_capacity(rows.len());
        let mut it = rows.into_iter();
        let Some(mut acc) = it.next() else { return Ok(v) };
        out.push(acc.clone());
        let mut st = Vec::new();
        for r in it {
            st.clear();
            st.push(acc);
            st.push(r);
            self.apply_callee(c, &mut st, span)?;
            if st.len() != 1 {
                return Err(Step::Here("scan needs a word that turns two values into one".into()));
            }
            acc = st.pop().unwrap();
            out.push(acc.clone());
        }
        Ok(from_rows(out, "scan")?)
    }

    fn lookup(&self, name: &str) -> R<Arc<UnitInfo>> {
        self.env.unit_info(name)
    }

    fn resolve(&self, u: &UnitExpr) -> R<units::Resolved> {
        units::resolve(u, &mut |n| self.lookup(n))
    }

    fn apply_unit(&self, n: Num, u: &UnitExpr) -> R<Num> {
        let r = self.resolve(u)?;
        if let Some(delta) = r.absolute {
            if !n.q.is_dimensionless() || !n.q.disp.is_none() {
                return Err(format!("{} is an absolute unit; it applies to a plain number, not {}", r.disp, n.q.disp));
            }
            let disp = r.disp;
            let data = n.data.iter().map(|x| disp.to_canonical(*x)).collect();
            let tan = n.tan.as_ref().map(|t| t.iter().map(|t| t * disp.factor).collect());
            return Ok(Num::with_shape(n.shape, data, Quant { dim: r.dim, disp, absolute: Some(delta) }).with_tan(tan));
        }
        let q = n.q.linear();
        let f = r.disp.factor;
        let tan = n.tan.as_ref().map(|t| t.iter().map(|t| t * f).collect());
        Ok(Num::with_shape(
            n.shape,
            n.data.iter().map(|x| x * f).collect(),
            Quant { dim: q.dim.mul(&r.dim), disp: q.disp.mul(&r.disp), absolute: None },
        )
        .with_tan(tan))
    }

    fn convert(&self, mut n: Num, u: &UnitExpr) -> R<Num> {
        let r = self.resolve(u)?;
        if n.q.dim != r.dim {
            return Err(format!("can't show {} in {}: dimensions differ ({} vs {})", n.q.dim, r.disp, n.q.dim, r.dim));
        }
        match (&r.absolute, &n.q.absolute) {
            (Some(_), None) => {
                return Err(format!("{} is for absolute values; this is a difference — use its Δ unit", r.disp));
            }
            (Some(delta), Some(_)) => n.q.absolute = Some(delta.clone()),
            _ => {}
        }
        // only the display unit changes, so each element still comes from where it did
        n.q.disp = r.disp;
        Ok(n)
    }

    fn builtin(&mut self, b: Builtin, st: &mut Vec<Value>, span: &Range<usize>) -> Result<(), Step> {
        use Builtin::*;
        if matches!(b, Add | Sub | Mul | Div) && arith_in_place(b, st) {
            st.pop();
            return Ok(());
        }
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
                    return Err(Step::Here("over needs two values".into()));
                }
                st.push(st[st.len() - 2].clone());
            }
            Rot => {
                if st.len() < 3 {
                    return Err(Step::Here("rot needs three values".into()));
                }
                let a = st.remove(st.len() - 3);
                st.push(a);
            }
            Add | Sub => {
                let y = pop_num(st, name)?;
                let x = pop_num(st, name)?;
                same_dims(&x.q, &y.q, name)?;
                let q = match (b, &x.q.absolute, &y.q.absolute) {
                    (Add, Some(_), Some(_)) => {
                        return Err(Step::Here("can't add two absolute values (temperatures, dates); subtract them or add a Δ".into()))
                    }
                    (Add, None, Some(_)) => y.q.clone(),
                    (Sub, Some(d), Some(_)) => Quant { dim: x.q.dim.clone(), disp: (**d).clone(), absolute: None },
                    (Sub, None, Some(_)) => return Err(Step::Here("can't subtract an absolute value from a difference".into())),
                    _ => x.q.clone(),
                };
                let r = if b == Add {
                    zip_d(&x, &y, q, |a, b| a + b, |_, _, ta, tb| ta + tb)?
                } else {
                    zip_d(&x, &y, q, |a, b| a - b, |_, _, ta, tb| ta - tb)?
                };
                st.push(Value::Num(r));
            }
            Mul | Div => {
                let y = pop_num(st, name)?;
                let x = pop_num(st, name)?;
                let (mut qx, mut qy) = (x.q.linear(), y.q.linear());
                // A dimensionless display unit (%, ‰) is absorbed into a
                // quantity with a dimension: 4 % of 100 USD shows as 4 USD.
                if qx.dim.is_none() && !qy.dim.is_none() {
                    qx.disp = DispUnit::none();
                }
                if qy.dim.is_none() && !qx.dim.is_none() {
                    qy.disp = DispUnit::none();
                }
                let q = if b == Mul {
                    Quant { dim: qx.dim.mul(&qy.dim), disp: qx.disp.mul(&qy.disp), absolute: None }
                } else {
                    Quant { dim: qx.dim.mul(&qy.dim.inv()), disp: qx.disp.mul(&qy.disp.pow(Rational::int(-1))), absolute: None }
                };
                let r = if b == Mul {
                    zip_d(&x, &y, q, |a, b| a * b, |a, b, ta, tb| lin(b, ta) + lin(a, tb))?
                } else {
                    zip_d(&x, &y, q, |a, b| a / b, |a, b, ta, tb| lin(1.0 / b, ta) - lin(a / (b * b), tb))?
                };
                st.push(Value::Num(r));
            }
            Pow => {
                let e = pop_num(st, name)?;
                let x = pop_num(st, name)?;
                if !e.q.is_dimensionless() {
                    return Err(Step::Here(format!("^ needs a dimensionless exponent, got {}", e.q.dim)));
                }
                let qx = x.q.linear();
                if qx.dim.is_none() {
                    let q = if qx.disp.is_none() { Quant::none() } else {
                        match e.as_scalar().and_then(Rational::from_f64) {
                            Some(r) => Quant { dim: Dim::none(), disp: qx.disp.pow(r), absolute: None },
                            None => Quant::none(),
                        }
                    };
                    // d(aⁿ) = n·aⁿ⁻¹·da + aⁿ·ln a·dn
                    st.push(Value::Num(zip_d(&x, &e, q, f64::powf, |a, n, ta, tn| lin(n * a.powf(n - 1.0), ta) + lin(a.powf(n) * a.ln(), tn))?));
                } else {
                    let s = e.as_scalar().ok_or("^ on a quantity with units needs a single exponent")?;
                    let r = Rational::from_f64(s).ok_or("^ on a quantity with units needs a simple fractional exponent")?;
                    let q = Quant { dim: qx.dim.pow(r), disp: qx.disp.pow(r), absolute: None };
                    // (the exponent is a fixed rational here: the result's dimension depends on it)
                    st.push(Value::Num(map_d(&x, q, |v| v.powf(s), |v| s * v.powf(s - 1.0))));
                }
            }
            Sqrt => {
                let x = pop_num(st, name)?;
                let qx = x.q.linear();
                let h = Rational::new(1, 2);
                let q = Quant { dim: qx.dim.pow(h), disp: qx.disp.pow(h), absolute: None };
                st.push(Value::Num(map_d(&x, q, f64::sqrt, |v| 0.5 / v.sqrt())));
            }
            Neg | Abs => {
                let x = pop_num(st, name)?;
                if x.q.absolute.is_some() {
                    return Err(Step::Here(format!("{name} of an absolute value (temperature, date) is meaningless")));
                }
                // |a| at 0 has derivative 0
                let r = if b == Neg {
                    map_d(&x, x.q.clone(), |v| -v, |_| -1.0)
                } else {
                    map_d(&x, x.q.clone(), f64::abs, |v| if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 })
                };
                st.push(Value::Num(r));
            }
            Exp | Log | Log10 | Log2 | Sin | Cos | Tan => {
                let x = pop_num(st, name)?;
                if !x.q.is_dimensionless() {
                    return Err(Step::Here(format!("{name} needs a dimensionless argument, got {}", x.q.dim)));
                }
                type F = fn(f64) -> f64;
                let (f, df): (F, F) = match b {
                    Exp => (f64::exp, f64::exp),
                    Log => (f64::ln, |v| 1.0 / v),
                    Log10 => (f64::log10, |v| 1.0 / (v * std::f64::consts::LN_10)),
                    Log2 => (f64::log2, |v| 1.0 / (v * std::f64::consts::LN_2)),
                    Sin => (f64::sin, f64::cos),
                    Cos => (f64::cos, |v| -v.sin()),
                    _ => (f64::tan, |v| 1.0 / (v.cos() * v.cos())),
                };
                st.push(Value::Num(map_d(&x, Quant::none(), f, df)));
            }
            Floor | Ceil | Round => {
                let x = pop_num(st, name)?;
                let f: fn(f64) -> f64 = match b {
                    Floor => f64::floor,
                    Ceil => f64::ceil,
                    _ => f64::round,
                };
                let d = x.q.disp.clone();
                st.push(Value::Num(map(&x, x.q.clone(), |v| d.to_canonical(f(d.to_display(v))))));
            }
            Min | Max | Lt | Gt | Le | Ge | Eq | Ne => {
                let y = pop_num(st, name)?;
                let x = pop_num(st, name)?;
                same_dims(&x.q, &y.q, name)?;
                if x.q.absolute.is_some() != y.q.absolute.is_some() {
                    return Err(Step::Here(format!("{name} can't compare an absolute value with a difference")));
                }
                let bq = bool_q();
                let r = match b {
                    // the derivative of the side chosen; a tie takes the left one's
                    Min => zip_d(&x, &y, x.q.clone(), f64::min, |a, b, ta, tb| if a <= b { ta } else { tb })?,
                    Max => zip_d(&x, &y, x.q.clone(), f64::max, |a, b, ta, tb| if a >= b { ta } else { tb })?,
                    // comparisons are flat: no tangent
                    Lt => zip(&x, &y, bq, |a, b| (a < b) as u8 as f64)?,
                    Gt => zip(&x, &y, bq, |a, b| (a > b) as u8 as f64)?,
                    Le => zip(&x, &y, bq, |a, b| (a <= b) as u8 as f64)?,
                    Ge => zip(&x, &y, bq, |a, b| (a >= b) as u8 as f64)?,
                    Eq => zip(&x, &y, bq, |a, b| (a == b) as u8 as f64)?,
                    _ => zip(&x, &y, bq, |a, b| (a != b) as u8 as f64)?,
                };
                st.push(Value::Num(r));
            }
            Not => {
                let x = pop_num(st, name)?;
                if !x.q.is_dimensionless() {
                    return Err(Step::Here("not needs a dimensionless argument".into()));
                }
                st.push(Value::Num(map(&x, Quant::none(), |v| (v == 0.0) as u8 as f64)));
            }
            If => {
                let e = pop(st, name)?;
                let t = pop(st, name)?;
                let c = pop_num(st, name)?;
                if !c.q.is_dimensionless() {
                    return Err(Step::Here("if needs a dimensionless condition".into()));
                }
                // both branches must measure the same thing, so the result's dimension is static
                if let (Value::Num(t), Value::Num(e)) = (&t, &e) {
                    same_dims(&t.q, &e.q, name)?;
                }
                if let Some(cv) = c.as_scalar() {
                    st.push(if cv != 0.0 { t } else { e });
                } else {
                    let (Value::Num(t), Value::Num(e)) = (t, e) else {
                        return Err(Step::Here("if with an array condition needs number branches".into()));
                    };
                    let picked = broadcast3(&c, &t, &e)?;
                    let tan = if t.tan.is_some() || e.tan.is_some() { Some(broadcast3(&c, &tangents(&t), &tangents(&e))?.1) } else { None };
                    st.push(Value::Num(Num::with_shape(picked.0, picked.1, t.q.clone()).with_tan(tan)));
                }
            }
            Len => {
                let v = pop(st, name)?;
                let n = match &v {
                    Value::Num(_) | Value::Text(_) => v.shape().first().copied().unwrap_or(1),
                    _ => return Err(Step::Here(format!("len of a {}", v.type_name()))),
                };
                st.push(Value::Num(Num::plain(n as f64)));
            }
            Range => {
                let n = pop_num(st, name)?;
                let k = scalar_dimless(&n, name)?;
                if k < 0.0 || k.fract() != 0.0 || k > 10_000_000.0 {
                    return Err(Step::Here("range needs a whole number ≥ 0".into()));
                }
                st.push(Value::Num(Num::vector((0..k as usize).map(|i| i as f64).collect(), Quant::none())));
            }
            Sum | Mean => {
                let x = pop_num(st, name)?;
                if x.is_empty() {
                    if b == Sum {
                        st.push(Value::Num(Num::scalar(0.0, x.q.linear())));
                        return Ok(());
                    }
                    return Err(Step::Here("mean of an empty array".into()));
                }
                if b == Sum && x.q.absolute.is_some() && x.len() > 1 {
                    return Err(Step::Here("can't sum absolute values (temperatures, dates)".into()));
                }
                let s: f64 = x.data.iter().sum();
                let v = if b == Sum { s } else { s / x.len() as f64 };
                let t = tan_sum(&x).map(|t| vec![if b == Sum { t } else { t / x.len() as f64 }]);
                st.push(Value::Num(Num::scalar(v, x.q.clone()).with_tan(t)));
            }
            Rev => {
                let v = pop(st, name)?;
                let mut rows = rows_of(&v)?;
                if v.shape().is_empty() {
                    st.push(v);
                } else {
                    rows.reverse();
                    st.push(from_rows_or_empty(rows, &v)?);
                }
            }
            Join => {
                let y = pop(st, name)?;
                let x = pop(st, name)?;
                st.push(join(x, y)?);
            }
            First | Last => {
                let v = pop(st, name)?;
                let n = row_count(&v)?;
                if n == 0 {
                    return Err(Step::Here(format!("{name} of an empty array")));
                }
                st.push(row_of(&v, if b == First { 0 } else { n - 1 })?);
            }
            Pick => {
                let i = pop_num(st, name)?;
                let v = pop(st, name)?;
                let k = scalar_dimless(&i, name)?;
                let n = row_count(&v)?;
                if k < 0.0 || k.fract() != 0.0 || k as usize >= n {
                    return Err(Step::Here(format!("pick index {k} is out of range 0..{n}")));
                }
                st.push(row_of(&v, k as usize)?);
            }
            Transpose => {
                let v = pop(st, name)?;
                st.push(transpose(v)?);
            }
            Reshape => {
                let s = pop_num(st, name)?;
                let v = pop(st, name)?;
                st.push(reshape(v, &s)?);
            }
            Couple => {
                let y = pop(st, name)?;
                let x = pop(st, name)?;
                st.push(from_rows(vec![x, y], name)?);
            }
            Pi => st.push(Value::Num(Num::plain(std::f64::consts::PI))),
            Line | Scatter | Path | Bar => {
                let ys = pop_num(st, name)?;
                let xs = pop(st, name)?;
                if ys.rank() != 1 {
                    return Err(Step::Here(format!("{name} needs a list of y values, got shape {:?}", ys.shape)));
                }
                // charts carry values only
                let mut ys = ys;
                ys.tan = None;
                let xs = match xs {
                    Value::Num(n) if n.rank() == 1 => Xs::Num(Num { tan: None, ..n }),
                    Value::Text(t) if t.shape.len() == 1 && b == Bar => Xs::Text(t),
                    Value::Text(_) => return Err(Step::Here(format!("{name} needs numeric x values (bar takes text categories)"))),
                    v => return Err(Step::Here(format!("{name} needs a list of x values, got {} {:?}", v.type_name(), v.shape()))),
                };
                if xs.len() != ys.len() {
                    return Err(Step::Here(format!("{name}: {} x values but {} y values", xs.len(), ys.len())));
                }
                let mark = match b {
                    Line => Mark::Line,
                    Scatter => Mark::Scatter,
                    Path => Mark::Path,
                    _ => Mark::Bar,
                };
                st.push(Value::Chart(Arc::new(Chart::single(crate::chart::Layer { mark, xs, ys }))));
            }
            Layer => {
                let c2 = pop_chart(st, name)?;
                let mut c1 = pop_chart(st, name)?;
                let (a, b2) = (&c1.layers[0], &c2.layers[0]);
                if a.ys.q.dim != b2.ys.q.dim {
                    return Err(Step::Here(format!("layer: y units differ ({} vs {})", a.ys.q.dim, b2.ys.q.dim)));
                }
                match (&a.xs, &b2.xs) {
                    (Xs::Num(x1), Xs::Num(x2)) if x1.q.dim != x2.q.dim => {
                        return Err(Step::Here(format!("layer: x units differ ({} vs {})", x1.q.dim, x2.q.dim)));
                    }
                    (Xs::Num(_), Xs::Text(_)) | (Xs::Text(_), Xs::Num(_)) => {
                        return Err(Step::Here("layer: can't mix category and numeric x axes".into()));
                    }
                    _ => {}
                }
                c1.layers.extend(c2.layers);
                st.push(Value::Chart(Arc::new(c1)));
            }
            Title | XLabel | YLabel => {
                let s = pop_str(st, name)?;
                let mut c = pop_chart(st, name)?;
                match b {
                    Title => c.title = Some(s),
                    XLabel => c.xlabel = Some(s),
                    _ => c.ylabel = Some(s),
                }
                st.push(Value::Chart(Arc::new(c)));
            }
            Size => {
                let rows = pop_num(st, name)?;
                let cols = pop_num(st, name)?;
                let (r, c) = (scalar_dimless(&rows, name)?, scalar_dimless(&cols, name)?);
                if !(1.0..=200.0).contains(&r) || !(1.0..=50.0).contains(&c) {
                    return Err(Step::Here("size: cols 1–50, rows 1–200".into()));
                }
                let mut ch = pop_chart(st, name)?;
                ch.rows = r as usize;
                ch.cols = c as usize;
                st.push(Value::Chart(Arc::new(ch)));
            }
            Iterate => self.iterate(st, span)?,
            Rk4 => self.rk4(st, span)?,
            Sensitivity => {
                // which cell's value this is: a reference carries it
                let x = pop_num(st, name)?;
                if x.len() != 1 {
                    return Err(Step::Here(format!("sensitivity needs a single number, got shape {:?}; pick one element (`last`, `pick`)", x.shape)));
                }
                let (k, i) = match x.prov.get(0) {
                    crate::value::Prov::Derived(k, i) => (k, i),
                    crate::value::Prov::Literal(_) => return Err(Step::Here("sensitivity: that's an input itself; give it a cell computed from inputs".into())),
                    crate::value::Prov::None => return Err(Step::Here("sensitivity needs a cell's value (a reference like B10), not one computed here".into())),
                };
                st.push(self.env.tornado(k, i)?);
            }
        }
        Ok(())
    }

    /// Runs `f` on a stack holding just `args`, and checks that it leaves `k` numbers.
    fn run_word(&mut self, f: &Callee, args: impl IntoIterator<Item = Value>, k: usize, what: &str, span: &Range<usize>) -> Result<Vec<Num>, Step> {
        // room for the word's working values, so its stack doesn't grow call after call
        let mut st = Vec::with_capacity(k + 16);
        st.extend(args);
        self.apply_callee(f, &mut st, span)?;
        if st.len() != k {
            return Err(Step::Here(left_count(what, f, k, st.len())));
        }
        let mut out = Vec::with_capacity(k);
        for (i, v) in st.into_iter().enumerate() {
            match v {
                Value::Num(n) => out.push(n),
                v => return Err(Step::Here(left_type(what, f, i, v.type_name()))),
            }
        }
        Ok(out)
    }

    /// `n s1 … sk 'step k iterate`: the state after 0, 1, … n applications of `step`, one array per state value.
    fn iterate(&mut self, st: &mut Vec<Value>, span: &Range<usize>) -> Result<(), Step> {
        let what = "iterate";
        let k = state_count(st, what)?;
        let f = pop_fn(st, what)?;
        let s0 = pop_states(st, k, what)?;
        let n = pop_num(st, what)?;
        dimless_only(&n, what)?;
        // the step always runs once, so its unit errors show even for 0 steps
        let s1 = self.run_word(&f, s0.iter().cloned().map(Value::Num), k, what, span)?;
        keeps_units(&f, &s0, &s1)?;
        // the static pass knows these checks' outcome so far; the rest depend on values
        let n = step_count(&n, what)?;
        check_shapes(&f, &s0, &s1, what)?;
        check_size(&s0, n, what)?;
        let mut hist = History::new(&s0, n);
        let mut cur = s0.clone();
        for step in 0..n {
            hist.push(&cur);
            let next = if step == 0 { s1.clone() } else { self.run_word(&f, cur.into_iter().map(Value::Num), k, what, span)? };
            // a step's units can depend on values (`2 [m] x ^`), so every step is checked
            keeps_units(&f, &s0, &next)?;
            check_shapes(&f, &s0, &next, what)?;
            cur = next;
        }
        hist.push(&cur);
        hist.push_onto(st, &s0, n);
        Ok(())
    }

    /// `t0 h n s1 … sk 'deriv k rk4`: fixed-step 4th-order Runge–Kutta for `ds/dt = deriv(t, s)`.
    fn rk4(&mut self, st: &mut Vec<Value>, span: &Range<usize>) -> Result<(), Step> {
        let what = "rk4";
        let k = state_count(st, what)?;
        let f = pop_fn(st, what)?;
        let s0 = pop_states(st, k, what)?;
        let n = pop_num(st, what)?;
        dimless_only(&n, what)?;
        let h = pop_num(st, what)?;
        let t0 = pop_num(st, what)?;
        same_dims(&t0.q, &h.q, what)?;
        if h.q.absolute.is_some() {
            return Err(Step::Here(RK4_ABS_STEP.into()));
        }
        let args = |t: f64, s: &[Num]| std::iter::once(Value::Num(Num::scalar(t, t0.q.clone()))).chain(s.iter().cloned().map(Value::Num)).collect::<Vec<_>>();
        // the derivative at the start is both the unit check and the first step's first stage
        // (whether t0 is a single number is checked after the unit checks; until then, its first element)
        let d0 = self.run_word(&f, args(t0.data.first().copied().unwrap_or(f64::NAN), &s0), k, what, span)?;
        for (i, (s, d)) in s0.iter().zip(&d0).enumerate() {
            if d.q.absolute.is_some() {
                return Err(Step::Here(rk4_abs_deriv(i)));
            }
            let dh = d.q.dim.mul(&h.q.dim);
            if dh != s.q.dim {
                return Err(Step::Here(rk4_units(i, &s.q.dim, &dh)));
            }
        }
        // the static pass knows these checks' outcome so far; the rest depend on values
        let n = step_count(&n, what)?;
        check_shapes(&f, &s0, &d0, what)?;
        if h.as_scalar().is_none() || t0.as_scalar().is_none() {
            return Err(Step::Here("rk4: the start time and the step are single numbers".into()));
        }
        check_size(&s0, n, what)?;
        // the start time and the step can be inputs too: their tangents go into every stage
        let (dt0, dh) = (t0.tan_at(0), h.tan_at(0));
        let (t0, h) = (t0.data[0], h.data[0]);
        let time = |t: f64, dt: f64| (t, dt);
        // s + c·d, elementwise, keeping s's units (dc: c's tangent)
        let axpy = |s: &[Num], c: f64, dc: f64, d: &[Num]| -> Vec<Num> {
            s.iter()
                .zip(d)
                .map(|(s, d)| {
                    let r = Num::with_shape(s.shape.clone(), s.data.iter().zip(d.data.iter()).map(|(x, y)| x + c * y).collect(), s.q.clone());
                    if s.tan.is_none() && d.tan.is_none() && dc == 0.0 {
                        return r;
                    }
                    r.with_tan(Some((0..s.len()).map(|e| s.tan_at(e) + lin(c, d.tan_at(e)) + lin(d.data[e], dc)).collect()))
                })
                .collect()
        };
        let targs = |(t, dt): (f64, f64), s: &[Num]| {
            let mut a = args(t, s);
            if let Some(Value::Num(tn)) = a.first_mut() {
                tn.set_tan((dt != 0.0).then(|| vec![dt]));
            }
            a
        };
        let mut hist = History::new(&s0, n);
        let mut cur = s0.clone();
        for i in 0..n {
            hist.push(&cur);
            let t = time(t0 + i as f64 * h, dt0 + i as f64 * dh);
            let mid = time(t.0 + h / 2.0, t.1 + dh / 2.0);
            let end = time(t.0 + h, t.1 + dh);
            let k1 = if i == 0 && dt0 == 0.0 { d0.clone() } else { self.run_word(&f, targs(t, &cur), k, what, span)? };
            let k2 = self.run_word(&f, targs(mid, &axpy(&cur, h / 2.0, dh / 2.0, &k1)), k, what, span)?;
            let k3 = self.run_word(&f, targs(mid, &axpy(&cur, h / 2.0, dh / 2.0, &k2)), k, what, span)?;
            let k4 = self.run_word(&f, targs(end, &axpy(&cur, h, dh, &k3)), k, what, span)?;
            for ks in [&k1, &k2, &k3, &k4] {
                for (j, (d, d0)) in ks.iter().zip(&d0).enumerate() {
                    if d.q.dim != d0.q.dim || d.q.absolute.is_some() {
                        return Err(Step::Here(format!("rk4: derivative {} changed units, from {} to {}", j + 1, describe(&d0.q), describe(&d.q))));
                    }
                }
                check_shapes(&f, &s0, ks, what)?;
            }
            cur = cur
                .iter()
                .enumerate()
                .map(|(j, s)| {
                    let v = (0..s.len()).map(|e| s.data[e] + h / 6.0 * (k1[j].data[e] + 2.0 * k2[j].data[e] + 2.0 * k3[j].data[e] + k4[j].data[e])).collect();
                    let r = Num::with_shape(s.shape.clone(), v, s.q.clone());
                    if s.tan.is_none() && [&k1, &k2, &k3, &k4].iter().all(|ks| ks[j].tan.is_none()) && dh == 0.0 {
                        return r;
                    }
                    let t = (0..s.len())
                        .map(|e| {
                            let sum = k1[j].data[e] + 2.0 * k2[j].data[e] + 2.0 * k3[j].data[e] + k4[j].data[e];
                            let dsum = k1[j].tan_at(e) + 2.0 * k2[j].tan_at(e) + 2.0 * k3[j].tan_at(e) + k4[j].tan_at(e);
                            s.tan_at(e) + lin(h / 6.0, dsum) + lin(sum, dh / 6.0)
                        })
                        .collect();
                    r.with_tan(Some(t))
                })
                .collect();
        }
        hist.push(&cur);
        hist.push_onto(st, &s0, n);
        Ok(())
    }
}

/// `iterate`'s step keeps each state's dimension, and whether it's absolute.
fn keeps_units(f: &Callee, before: &[Num], after: &[Num]) -> R<()> {
    for (i, (a, b)) in before.iter().zip(after).enumerate() {
        if a.q.dim != b.q.dim || a.q.absolute.is_some() != b.q.absolute.is_some() {
            return Err(units_changed(f, i, &describe(&a.q), &describe(&b.q)));
        }
    }
    Ok(())
}

pub(crate) fn units_changed(f: &Callee, i: usize, from: &str, to: &str) -> String {
    format!("iterate needs matching units: '{} turns state {} from {from} into {to}", f.name(), i + 1)
}

pub(crate) fn rk4_units(i: usize, state: &Dim, dh: &Dim) -> String {
    format!("rk4 needs matching units: state {} is {state}, but its derivative × the step is {dh}", i + 1)
}

pub(crate) const RK4_ABS_STEP: &str = "rk4: the step size is a difference, not an absolute value (a date or temperature)";

pub(crate) fn rk4_abs_deriv(i: usize) -> String {
    format!("rk4: derivative {} is an absolute value; a rate of change is a difference", i + 1)
}

/// The most steps `iterate` and `rk4` take, and the most values they produce.
pub(crate) const MAX_STEPS: f64 = 1_000_000.0;
pub(crate) const MAX_VALUES: usize = 10_000_000;

/// A quantity's dimension, and whether it's absolute, the way the static pass writes it.
fn describe(q: &Quant) -> String {
    if q.absolute.is_some() {
        format!("{} (absolute)", q.dim)
    } else {
        q.dim.to_string()
    }
}

pub(crate) fn left_count(what: &str, f: &Callee, k: usize, got: usize) -> String {
    format!("{what}: '{} should leave {k} value{}, left {got}", f.name(), if k == 1 { "" } else { "s" })
}

pub(crate) fn left_type(what: &str, f: &Callee, i: usize, ty: &str) -> String {
    format!("{what}: '{}'s value {} is a {ty}, not a number", f.name(), i + 1)
}

pub(crate) fn bad_count(what: &str, v: f64) -> String {
    format!("{what}: the state count must be a whole number ≥ 1, got {}", crate::value::fmt_num(v))
}

pub(crate) fn bad_steps(what: &str, v: f64) -> String {
    format!("{what}: the step count must be a whole number from 0 to 1,000,000, got {}", crate::value::fmt_num(v))
}

/// The state count `k` on top of the stack.
fn state_count(st: &mut Vec<Value>, what: &str) -> R<usize> {
    let k = pop_num(st, what)?;
    let v = scalar_dimless(&k, what)?;
    if v < 1.0 || v.fract() != 0.0 {
        return Err(bad_count(what, v));
    }
    Ok(v.min(u32::MAX as f64) as usize)
}

fn pop_fn(st: &mut Vec<Value>, what: &str) -> R<Callee> {
    match pop(st, what)? {
        Value::Fn(c) => Ok(c),
        v => Err(format!("{what} expects a word ('name), got {}", v.type_name())),
    }
}

/// `k` numbers, deepest first.
fn pop_states(st: &mut Vec<Value>, k: usize, what: &str) -> R<Vec<Num>> {
    let mut v = Vec::with_capacity(k.min(st.len()));
    for _ in 0..k {
        v.push(pop_num(st, what)?);
    }
    v.reverse();
    Ok(v)
}

fn dimless_only(n: &Num, what: &str) -> R<()> {
    if !n.q.is_dimensionless() {
        return Err(format!("{what} expects a dimensionless number, got {}", n.q.dim));
    }
    Ok(())
}

fn step_count(n: &Num, what: &str) -> R<usize> {
    let v = n.as_scalar().ok_or_else(|| format!("{what} expects a single number, got shape {:?}", n.shape))?;
    if !(0.0..=MAX_STEPS).contains(&v) || v.fract() != 0.0 {
        return Err(bad_steps(what, v));
    }
    Ok(v as usize)
}

fn check_size(s0: &[Num], n: usize, what: &str) -> R<()> {
    let per: usize = s0.iter().map(|s| s.len()).sum();
    if per.saturating_mul(n + 1) > MAX_VALUES {
        return Err(format!("{what}: {n} steps of {per} values each is more than 10,000,000 values"));
    }
    Ok(())
}

fn check_shapes(f: &Callee, s0: &[Num], got: &[Num], what: &str) -> R<()> {
    for (i, (a, b)) in s0.iter().zip(got).enumerate() {
        if a.shape != b.shape {
            let role = if what == "rk4" { "derivative" } else { "state" };
            return Err(format!("{what}: '{} gives {role} {} with shape {:?}, but state {} has shape {:?}", f.name(), i + 1, b.shape, i + 1, a.shape));
        }
    }
    Ok(())
}

/// Each state's values step after step, and their tangents once any state has one.
struct History {
    vals: Vec<Vec<f64>>,
    tans: Option<Vec<Vec<f64>>>,
}

impl History {
    fn new(s0: &[Num], n: usize) -> History {
        History { vals: s0.iter().map(|s| Vec::with_capacity((n + 1) * s.len())).collect(), tans: None }
    }

    fn push(&mut self, cur: &[Num]) {
        if self.tans.is_none() && cur.iter().any(|s| s.tan.is_some()) {
            // the steps so far had none
            self.tans = Some(self.vals.iter().map(|v| vec![0.0; v.len()]).collect());
        }
        for (j, s) in cur.iter().enumerate() {
            self.vals[j].extend_from_slice(&s.data);
            if let Some(t) = &mut self.tans {
                t[j].extend((0..s.len()).map(|e| s.tan_at(e)));
            }
        }
    }

    /// One array per state: its value after 0, 1, … n steps, along a new leading axis.
    fn push_onto(self, st: &mut Vec<Value>, s0: &[Num], n: usize) {
        let mut tans = self.tans.map(|t| t.into_iter());
        for (s, h) in s0.iter().zip(self.vals) {
            let mut shape = vec![n + 1];
            shape.extend_from_slice(&s.shape);
            let t = tans.as_mut().and_then(|t| t.next());
            st.push(Value::Num(Num::with_shape(shape, h, s.q.clone()).with_tan(t)));
        }
    }
}

fn from_rows_or_empty(rows: Vec<Value>, like: &Value) -> R<Value> {
    if rows.is_empty() {
        return Ok(like.clone());
    }
    from_rows(rows, "array")
}

fn broadcast3(c: &Num, t: &Num, e: &Num) -> R<(crate::value::Shape, Vec<f64>)> {
    // pick per element: broadcast cond against each branch
    let (s1, tv) = broadcast2(c, t, |c, t| if c != 0.0 { t } else { f64::NAN })?;
    let tmp = Num::with_shape(s1.clone(), tv, Quant::none());
    let (s2, ev) = broadcast2(c, e, |c, e| if c != 0.0 { f64::NAN } else { e })?;
    if s1 != s2 {
        return Err(format!("if: shapes {:?} and {:?} don't match", s1, s2));
    }
    let picked = tmp.data.iter().zip(ev.iter()).map(|(a, b)| if a.is_nan() { *b } else { *a }).collect();
    Ok((s1, picked))
}

fn join(x: Value, y: Value) -> R<Value> {
    let rank = |v: &Value| v.shape().len();
    let promote = |v: Value, other_rank: usize| -> R<Vec<Value>> {
        if rank(&v) == 0 || rank(&v) + 1 == other_rank {
            Ok(vec![v])
        } else {
            rows_of(&v)
        }
    };
    let (rx, ry) = (rank(&x), rank(&y));
    let mut rows = promote(x.clone(), ry)?;
    rows.extend(promote(y, rx)?);
    // two empty arrays join to an empty array that keeps the left one's units
    if rows.is_empty() {
        return Ok(x);
    }
    from_rows(rows, "join")
}

/// `a shape reshape`: a's elements, in order, in a new shape. The shape is a number or a list of
/// whole numbers whose product is a's element count; nothing is dropped or repeated to make it fit.
fn reshape(v: Value, s: &Num) -> R<Value> {
    dimless_only(s, "reshape")?;
    let n = match &v {
        Value::Num(n) => n.len(),
        Value::Text(t) => t.data.len(),
        v => return Err(format!("can't reshape a {}", v.type_name())),
    };
    if s.rank() > 1 {
        return Err(format!("reshape: the shape is a number or a list, got shape {:?}", s.shape));
    }
    let mut shape = Vec::with_capacity(s.len());
    for &d in s.data.iter() {
        if !(0.0..=MAX_INDEX as f64).contains(&d) || d.fract() != 0.0 {
            return Err(format!("reshape: {d} isn't a length (a whole number, 0 or more)"));
        }
        shape.push(d as usize);
    }
    let fits: usize = shape.iter().product();
    if fits != n {
        let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
        return Err(format!("reshape: {n} elements don't fit {} ({fits})", dims.join("×")));
    }
    Ok(match v {
        Value::Num(x) => Value::Num(Num { shape: shape.into(), ..x }),
        Value::Text(t) => Value::Text(Text { shape, data: t.data }),
        _ => unreachable!(),
    })
}

fn transpose(v: Value) -> R<Value> {
    match v {
        Value::Num(n) if n.rank() == 2 => {
            let (r, c) = (n.shape[0], n.shape[1]);
            let mut d = Vec::with_capacity(r * c);
            for j in 0..c {
                for i in 0..r {
                    d.push(n.data[i * c + j]);
                }
            }
            let tan = n.tan.as_ref().map(|t| (0..r * c).map(|e| t[(e % r) * c + e / r]).collect());
            let mut t = Num::with_shape(vec![c, r], d, n.q.clone()).with_tan(tan);
            if !n.prov.is_none() {
                t.prov = Provs::List((0..r * c).map(|e| n.prov.get((e % r) * c + e / r)).collect());
            }
            Ok(Value::Num(t))
        }
        Value::Num(n) if n.rank() == 1 => {
            let k = n.len();
            Ok(Value::Num(Num { shape: vec![1, k].into(), ..n }))
        }
        Value::Text(t) if t.shape.len() == 2 => {
            let (r, c) = (t.shape[0], t.shape[1]);
            let mut d = Vec::with_capacity(r * c);
            for j in 0..c {
                for i in 0..r {
                    d.push(t.data[i * c + j].clone());
                }
            }
            Ok(Value::Text(Text { shape: vec![c, r], data: Arc::new(d) }))
        }
        Value::Text(t) if t.shape.len() == 1 => {
            let k = t.data.len();
            Ok(Value::Text(Text { shape: vec![1, k], data: t.data.clone() }))
        }
        v => Ok(v),
    }
}

/// Internal: error either at the current op, or already located (from a word body).
enum Step {
    Here(String),
    Inner(EvalErr),
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
