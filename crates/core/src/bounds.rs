//! Input ranges: an optional `min` and `max` on a named input, each a number in the input's unit
//! (`0 [1/s]`, `5 [%]`). One rule: an input's range limits scrubbing, chart dragging and goal-seek;
//! typing a value outside it is an error on that cell. The value is kept, never clamped, and cells
//! reading it show `#upstream` as usual.
//!
//! The ends are stored as written (`NameDef::min`/`max`) and compiled on every rebuild (a name only
//! changes through an edit that rebuilds). An end's units are dependencies of the input's cell.

use crate::engine::{local, CellResult, Edit, Engine, View};
use crate::eval::run_program;
use crate::ids::CellKey;
use crate::model::{classify, Kind, NameDef};
use crate::ops::format_lit;
use crate::parse::{Compiled, Compiler, Dep, Op};
use crate::units::Quant;
use crate::value::Value;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

const LABELS: [&str; 2] = ["min", "max"];

/// One end of a range, compiled.
pub(crate) struct End {
    text: String,
    ops: Result<Vec<Op>, String>,
}

/// The range of the input in a cell.
pub(crate) struct Ranged {
    name: String,
    /// min, max
    ends: [Option<End>; 2],
    pub(crate) deps: Vec<Dep>,
}

/// An input's range in the numbers its literal is written in (`5` in `5 [%]`): what scrubbing,
/// chart dragging and goal-seek move. Each end comes with its text as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputRange {
    pub min: Option<(f64, String)>,
    pub max: Option<(f64, String)>,
}

/// Float noise from unit conversions isn't outside the range.
fn below(x: f64, lo: f64) -> bool {
    x < lo - 1e-9 * lo.abs()
}

impl InputRange {
    /// `v` stopped at the range, and the end that stopped it (`min 0 [1/s]`).
    pub fn pin(&self, v: f64) -> (f64, Option<String>) {
        match (&self.min, &self.max) {
            (Some((lo, t)), _) if below(v, *lo) => (*lo, Some(format!("min {t}"))),
            (_, Some((hi, t))) if below(-v, -*hi) => (*hi, Some(format!("max {t}"))),
            _ => (v, None),
        }
    }

    pub fn contains(&self, v: f64) -> bool {
        self.pin(v).1.is_none()
    }

    /// `v` written with `decimals` places, or as many more as it takes to stay inside the range
    /// (a bound of 0.05 on a literal written `0.3`).
    pub fn format(&self, v: f64, decimals: usize, is_date: bool) -> String {
        let mut d = decimals;
        loop {
            let s = format_lit(v, d, is_date);
            if is_date || d >= decimals + 12 || s.parse::<f64>().is_ok_and(|x| self.contains(x)) {
                return s;
            }
            d += 1;
        }
    }
}

/// Whether two quantities can be compared: same dimension, both absolute or neither.
fn fits(a: &Quant, b: &Quant) -> bool {
    a.dim == b.dim && a.absolute.is_some() == b.absolute.is_some()
}

impl Engine {
    /// Every input's range, compiled.
    pub(crate) fn compile_ranges(&self) -> HashMap<CellKey, Arc<Ranged>> {
        let mut out = HashMap::default();
        for (name, d) in &self.wb.names {
            if !d.input || (d.min.is_none() && d.max.is_none()) {
                continue;
            }
            let mut deps = Vec::new();
            let mut end = |t: &Option<String>| {
                t.as_ref().map(|t| {
                    let (ops, d) = self.compile_end(t, d.cell);
                    deps.extend(d);
                    End { text: t.clone(), ops }
                })
            };
            let ends = [end(&d.min), end(&d.max)];
            out.insert(d.cell, Arc::new(Ranged { name: name.clone(), ends, deps }));
        }
        out
    }

    fn compile_end(&self, text: &str, k: CellKey) -> (Result<Vec<Op>, String>, Vec<Dep>) {
        if classify(text) != Kind::Number {
            return (Err(format!("{} isn't a number with a unit, like 0 [1/s]", text.trim())), vec![]);
        }
        let mut c = Compiler::new(&self.wb, self.symbols(), k.sheet);
        match c.compile(text) {
            Ok(Compiled::Program(ops)) => (Ok(ops), c.deps),
            Ok(_) => (Err(format!("{} isn't a number", text.trim())), vec![]),
            Err(e) => (Err(e.msg), vec![]),
        }
    }

    /// An end's value (canonical) and quantity.
    fn end_value(&self, ops: &Result<Vec<Op>, String>) -> Result<(f64, Quant), String> {
        match run_program(&View(self), ops.as_ref().map_err(Clone::clone)?) {
            Ok(Value::Num(n)) => match n.as_scalar() {
                Some(x) if x.is_finite() => Ok((x, n.q)),
                _ => Err("an end of a range is a single number".into()),
            },
            Ok(v) => Err(format!("an end of a range is a number, not a {}", v.type_name())),
            Err(e) => Err(e.msg),
        }
    }

    /// The cell's result, or the error that its value is outside its input's range (or that the
    /// range doesn't fit it).
    pub(crate) fn check_range(&self, k: CellKey, res: CellResult) -> CellResult {
        let Some(r) = self.ranges.get(&k) else { return res };
        let n = match &res {
            Ok(Value::Num(n)) => n,
            Ok(v) => return Err(local(format!("{}'s range: {} is a {}, not a number", r.name, r.name, v.type_name()), None)),
            Err(_) => return res,
        };
        for (i, end) in r.ends.iter().enumerate() {
            let Some(end) = end else { continue };
            let (b, q) = match self.end_value(&end.ops) {
                Ok(b) => b,
                Err(m) => return Err(local(format!("{}'s range: {} {}: {m}", r.name, LABELS[i], end.text), None)),
            };
            if !fits(&q, &n.q) {
                return Err(local(format!("{}'s range: {} {} is {}, but {} is {}", r.name, LABELS[i], end.text, q.dim, r.name, n.q.dim), None));
            }
            let out = |x: &f64| if i == 0 { below(*x, b) } else { below(-*x, -b) };
            if n.data.iter().any(out) {
                return Err(local(format!("{} must be {} {}", r.name, if i == 0 { "≥" } else { "≤" }, end.text), None));
            }
        }
        res
    }

    /// The range of the input at `k` in the numbers its literal is written in; `None` when it has
    /// none or isn't a number literal. An end that doesn't evaluate or fit is left out (the cell
    /// shows why).
    pub fn input_range(&self, k: CellKey) -> Option<InputRange> {
        let r = self.ranges.get(&k)?;
        let Ok(Value::Num(own)) = self.trace_cell(k).result else { return None };
        crate::ops::cell_literal(&self.wb.cell_text(k))?;
        let mut out = InputRange::default();
        for (i, end) in r.ends.iter().enumerate() {
            let Some(end) = end else { continue };
            let Ok((b, q)) = self.end_value(&end.ops) else { continue };
            if fits(&q, &own.q) {
                let v = Some((own.q.disp.to_display(b), end.text.clone()));
                if i == 0 {
                    out.min = v;
                } else {
                    out.max = v;
                }
            }
        }
        Some(out)
    }

    /// Sets the range of the input `name` (empty text: no bound on that side). Each end must be a
    /// number in the input's dimension, and min no more than max.
    pub fn set_range(&mut self, name: &str, min: &str, max: &str) -> Result<Edit, String> {
        let def = self.wb.names.get(name).filter(|d| d.input).ok_or_else(|| format!("{name} isn't an input"))?.clone();
        let own = match self.trace_cell(def.cell).result {
            Ok(Value::Num(n)) => n.q,
            _ => return Err(format!("{name} needs a number before it can have a range")),
        };
        let mut vals = [None, None];
        let mut texts = [None, None];
        for (i, text) in [min, max].into_iter().enumerate() {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let (ops, _) = self.compile_end(text, def.cell);
            let (b, q) = self.end_value(&ops).map_err(|m| format!("{}: {m}", LABELS[i]))?;
            if !fits(&q, &own) {
                return Err(format!("{} {text} is {}, but {name} is {}", LABELS[i], q.dim, own.dim));
            }
            vals[i] = Some(b);
            texts[i] = Some(text.to_string());
        }
        if let [Some(lo), Some(hi)] = vals {
            if lo > hi {
                return Err(format!("min {} is more than max {}", min.trim(), max.trim()));
            }
        }
        let [min, max] = texts;
        Ok(self.apply(Edit::Name { name: name.to_string(), def: Some(NameDef { min, max, ..def }) }))
    }
}
