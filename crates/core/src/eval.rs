//! The stack interpreter. A program runs on an empty stack; the cell's value
//! is the single value left on it.

use crate::chart::{Chart, Mark, Xs};
use crate::ids::*;
use crate::model::StoredRef;
use crate::parse::{Builtin, Callee, Compiled, Op, OpKind};
use crate::rational::Rational;
use crate::units::{self, Dim, DispUnit, Quant, UnitExpr, UnitInfo};
use crate::value::{broadcast2, Num, Text, Value};
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
    fn range_value(&self, sheet: SheetId, a: &StoredRef, b: &StoredRef) -> Result<Value, String>;
    fn unit_info(&self, name: &str) -> Result<UnitInfo, String>;
    fn word(&self, k: CellKey) -> Option<Arc<Compiled>>;
}

type R<T> = Result<T, String>;

pub struct Interp<'e> {
    env: &'e dyn Env,
    depth: usize,
}

const MAX_DEPTH: usize = 64;

pub fn run_program(env: &dyn Env, ops: &[Op]) -> Result<Value, EvalErr> {
    let mut it = Interp { env, depth: 0 };
    let mut stack = Vec::new();
    it.run(ops, &mut stack, &[])?;
    match stack.len() {
        1 => Ok(stack.pop().unwrap()),
        0 => Err(EvalErr { msg: "nothing left on the stack".into(), span: None }),
        n => Err(EvalErr { msg: format!("{} values left on stack", n), span: None }),
    }
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
    Num::with_shape(n.shape.clone(), n.data.iter().map(|x| f(*x)).collect(), q)
}

fn zip(a: &Num, b: &Num, q: Quant, f: impl Fn(f64, f64) -> f64) -> R<Num> {
    let (shape, data) = broadcast2(a, b, f)?;
    Ok(Num::with_shape(shape, data, q))
}

fn bool_q() -> Quant {
    Quant::none()
}

/// Split along the leading axis.
fn rows_of(v: &Value) -> R<Vec<Value>> {
    match v {
        Value::Num(n) => {
            if n.shape.is_empty() {
                return Ok(vec![v.clone()]);
            }
            let k = n.shape[0];
            let inner: usize = n.shape[1..].iter().product();
            Ok((0..k)
                .map(|i| {
                    Value::Num(Num::with_shape(n.shape[1..].to_vec(), n.data[i * inner..(i + 1) * inner].to_vec(), n.q.clone()))
                })
                .collect())
        }
        Value::Text(t) => {
            if t.shape.is_empty() {
                return Ok(vec![v.clone()]);
            }
            let k = t.shape[0];
            let inner: usize = t.shape[1..].iter().product();
            Ok((0..k)
                .map(|i| Value::Text(Text { shape: t.shape[1..].to_vec(), data: Arc::new(t.data[i * inner..(i + 1) * inner].to_vec()) }))
                .collect())
        }
        v => Err(format!("can't split a {} into rows", v.type_name())),
    }
}

/// Stack values (all the same shape and units) along a new leading axis.
fn from_rows(rows: Vec<Value>, what: &str) -> R<Value> {
    let Some(first) = rows.first() else { return Ok(Value::Num(Num::vector(vec![], Quant::none()))) };
    match first {
        Value::Num(f) => {
            let mut data = Vec::with_capacity(rows.len() * f.len());
            for r in &rows {
                let Value::Num(n) = r else { return Err(format!("{what}: mixed numbers and {}", r.type_name())) };
                if n.shape != f.shape {
                    return Err(format!("{what}: rows have different shapes {:?} and {:?}", f.shape, n.shape));
                }
                if n.q.dim != f.q.dim || n.q.absolute.is_some() != f.q.absolute.is_some() {
                    return Err(format!("{what}: rows have different units {} and {}", f.q.dim, n.q.dim));
                }
                data.extend_from_slice(&n.data);
            }
            let mut shape = vec![rows.len()];
            shape.extend_from_slice(&f.shape);
            let q = match &rows.last().unwrap() {
                Value::Num(n) => n.q.clone(),
                _ => unreachable!(),
            };
            Ok(Value::Num(Num::with_shape(shape, data, q)))
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
            OpKind::Range { sheet, a, b } => stack.push(self.env.range_value(*sheet, a, b)?),
            OpKind::Unit(u) => {
                let n = pop_num(stack, "a unit")?;
                stack.push(Value::Num(self.apply_unit(n, u)?));
            }
            OpKind::To(u) => {
                let n = pop_num(stack, "to")?;
                stack.push(Value::Num(self.convert(n, u)?));
            }
            OpKind::Local(i) => stack.push(locals[*i].clone()),
            OpKind::Builtin(b) => self.builtin(*b, stack)?,
            OpKind::Call(name, k) => self.call(name, *k, stack, &op.span)?,
            OpKind::Reduce(c) => {
                let v = pop(stack, "reduce")?;
                stack.push(self.reduce(c, v, &op.span)?);
            }
            OpKind::Scan(c) => {
                let v = pop(stack, "scan")?;
                stack.push(self.scan(c, v, &op.span)?);
            }
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
            Callee::Builtin(b) => self.builtin(*b, stack),
            Callee::User(name, k) => self.call(name, *k, stack, span),
        }
    }

    fn reduce(&mut self, c: &Callee, v: Value, span: &Range<usize>) -> Result<Value, Step> {
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
                return Ok(Value::Num(Num::scalar(n.data.iter().sum(), n.q.clone())));
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

    fn lookup(&self, name: &str) -> R<UnitInfo> {
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
            return Ok(Num::with_shape(n.shape, data, Quant { dim: r.dim, disp, absolute: Some(delta) }));
        }
        let q = n.q.linear();
        let f = r.disp.factor;
        Ok(Num::with_shape(
            n.shape,
            n.data.iter().map(|x| x * f).collect(),
            Quant { dim: q.dim.mul(&r.dim), disp: q.disp.mul(&r.disp), absolute: None },
        ))
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
        n.q.disp = r.disp;
        n.prov = None;
        Ok(n)
    }

    fn builtin(&mut self, b: Builtin, st: &mut Vec<Value>) -> Result<(), Step> {
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
                let r = if b == Add { zip(&x, &y, q, |a, b| a + b)? } else { zip(&x, &y, q, |a, b| a - b)? };
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
                let r = if b == Mul { zip(&x, &y, q, |a, b| a * b)? } else { zip(&x, &y, q, |a, b| a / b)? };
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
                    st.push(Value::Num(zip(&x, &e, q, f64::powf)?));
                } else {
                    let s = e.as_scalar().ok_or("^ on a quantity with units needs a single exponent")?;
                    let r = Rational::from_f64(s).ok_or("^ on a quantity with units needs a simple fractional exponent")?;
                    let q = Quant { dim: qx.dim.pow(r), disp: qx.disp.pow(r), absolute: None };
                    st.push(Value::Num(map(&x, q, |v| v.powf(s))));
                }
            }
            Sqrt => {
                let x = pop_num(st, name)?;
                let qx = x.q.linear();
                let h = Rational::new(1, 2);
                let q = Quant { dim: qx.dim.pow(h), disp: qx.disp.pow(h), absolute: None };
                st.push(Value::Num(map(&x, q, f64::sqrt)));
            }
            Neg | Abs => {
                let x = pop_num(st, name)?;
                if x.q.absolute.is_some() {
                    return Err(Step::Here(format!("{name} of an absolute value (temperature, date) is meaningless")));
                }
                let r = if b == Neg { map(&x, x.q.clone(), |v| -v) } else { map(&x, x.q.clone(), f64::abs) };
                st.push(Value::Num(r));
            }
            Exp | Log | Log10 | Log2 | Sin | Cos | Tan => {
                let x = pop_num(st, name)?;
                if !x.q.is_dimensionless() {
                    return Err(Step::Here(format!("{name} needs a dimensionless argument, got {}", x.q.dim)));
                }
                let f: fn(f64) -> f64 = match b {
                    Exp => f64::exp,
                    Log => f64::ln,
                    Log10 => f64::log10,
                    Log2 => f64::log2,
                    Sin => f64::sin,
                    Cos => f64::cos,
                    _ => f64::tan,
                };
                st.push(Value::Num(map(&x, Quant::none(), f)));
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
                    Min => zip(&x, &y, x.q.clone(), f64::min)?,
                    Max => zip(&x, &y, x.q.clone(), f64::max)?,
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
                if let Some(cv) = c.as_scalar() {
                    st.push(if cv != 0.0 { t } else { e });
                } else {
                    let (Value::Num(t), Value::Num(e)) = (t, e) else {
                        return Err(Step::Here("if with an array condition needs number branches".into()));
                    };
                    same_dims(&t.q, &e.q, name)?;
                    let picked = broadcast3(&c, &t, &e)?;
                    st.push(Value::Num(Num::with_shape(picked.0, picked.1, t.q.clone())));
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
                st.push(Value::Num(Num::scalar(v, x.q.clone())));
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
                let rows = rows_of(&v)?;
                let r = if b == First { rows.into_iter().next() } else { rows.into_iter().last() };
                st.push(r.ok_or_else(|| format!("{name} of an empty array"))?);
            }
            Pick => {
                let i = pop_num(st, name)?;
                let v = pop(st, name)?;
                let k = scalar_dimless(&i, name)?;
                let rows = rows_of(&v)?;
                if k < 0.0 || k.fract() != 0.0 || k as usize >= rows.len() {
                    return Err(Step::Here(format!("pick index {k} is out of range 0..{}", rows.len())));
                }
                st.push(rows.into_iter().nth(k as usize).unwrap());
            }
            Transpose => {
                let v = pop(st, name)?;
                st.push(transpose(v)?);
            }
            Couple => {
                let y = pop(st, name)?;
                let x = pop(st, name)?;
                st.push(from_rows(vec![x, y], name)?);
            }
            Pi => st.push(Value::Num(Num::plain(std::f64::consts::PI))),
            Line | Scatter | Bar => {
                let ys = pop_num(st, name)?;
                let xs = pop(st, name)?;
                if ys.rank() != 1 {
                    return Err(Step::Here(format!("{name} needs a list of y values, got shape {:?}", ys.shape)));
                }
                let xs = match xs {
                    Value::Num(n) if n.rank() == 1 => Xs::Num(n),
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
        }
        Ok(())
    }
}

fn from_rows_or_empty(rows: Vec<Value>, like: &Value) -> R<Value> {
    if rows.is_empty() {
        return Ok(like.clone());
    }
    from_rows(rows, "array")
}

fn broadcast3(c: &Num, t: &Num, e: &Num) -> R<(Vec<usize>, Vec<f64>)> {
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
    let mut rows = promote(x, ry)?;
    rows.extend(promote(y, rx)?);
    from_rows(rows, "join")
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
            Ok(Value::Num(Num::with_shape(vec![c, r], d, n.q.clone())))
        }
        Value::Num(n) if n.rank() == 1 => {
            let k = n.len();
            Ok(Value::Num(Num::with_shape(vec![1, k], (*n.data).clone(), n.q.clone())))
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
