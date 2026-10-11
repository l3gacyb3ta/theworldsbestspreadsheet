//! Input ranges: an optional `min` and `max` on a named input. One rule: an input's range limits
//! scrubbing, chart dragging and goal-seek; typing a value outside it is an error on that cell. The
//! value is kept, never clamped, and cells reading it show `#upstream` as usual.
//!
//! An end is a number in the input's unit (`0 [1/s]`, `5 [%]`) or a formula: a reference (`B7`),
//! a name (`max_damping`) or any program that leaves one number of the input's dimension. It is
//! stored like a cell (`NameDef::min`/`max`), so its references are ids that survive inserts,
//! sorts and moves, and compiled on every rebuild (a name only changes through an edit that
//! rebuilds). What an end reads (cells, names, units) is a dependency of the input's cell: the
//! input is checked again whenever it changes. An end that is empty or an error makes the range
//! unusable, and the input says so.

use crate::engine::{local, CellResult, Edit, Engine, View};
use crate::eval::run_program;
use crate::ids::CellKey;
use crate::model::{classify, Cell, Kind, NameDef};
use crate::ops::format_lit;
use crate::parse::{Compiled, Compiler, Dep, Op};
use crate::units::Quant;
use crate::value::{fmt_quantity, Value};
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

const LABELS: [&str; 2] = ["min", "max"];

/// One end of a range, compiled.
pub(crate) struct End {
    /// As the inspector shows it (`0 [1/s]`, `B7`).
    text: String,
    /// A number literal (shown as written) rather than a formula (shown with its value).
    literal: bool,
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
/// chart dragging and goal-seek move. Each end comes with how to name it (`0 [1/s]`, `B7 (0.5 1/s)`).
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
            let mut end = |c: &Option<Cell>| {
                c.as_ref().map(|c| {
                    let (end, d) = self.compile_end(c, d.cell);
                    deps.extend(d);
                    end
                })
            };
            let ends = [end(&d.min), end(&d.max)];
            out.insert(d.cell, Arc::new(Ranged { name: name.clone(), ends, deps }));
        }
        out
    }

    /// An end as typed in the inspector, stored like a cell: a number stays one, anything else is a formula.
    fn end_cell(&self, text: &str, k: CellKey) -> Cell {
        let t = text.trim();
        let t = if classify(t) == Kind::Number || t.starts_with('=') { t.to_string() } else { format!("={t}") };
        Cell::new(self.wb.parse_text(&t, k.sheet))
    }

    /// An end as the inspector shows it, with its references in A1.
    fn end_text(&self, c: &Cell, k: CellKey) -> String {
        let t = self.wb.render(&c.pieces, k.sheet);
        t.strip_prefix('=').unwrap_or(&t).trim().to_string()
    }

    /// The range of the input `name` as the inspector shows it (empty: no bound on that side).
    pub fn range_text(&self, name: &str) -> (String, String) {
        let Some(d) = self.wb.names.get(name) else { return Default::default() };
        let show = |c: &Option<Cell>| c.as_ref().map(|c| self.end_text(c, d.cell)).unwrap_or_default();
        (show(&d.min), show(&d.max))
    }

    fn compile_end(&self, c: &Cell, k: CellKey) -> (End, Vec<Dep>) {
        let source = self.wb.render(&c.pieces, k.sheet);
        let text = self.end_text(c, k);
        let literal = classify(&source) == Kind::Number;
        let mut comp = Compiler::new(&self.wb, self.symbols(), k.sheet);
        let ops = match comp.compile(&source) {
            Ok(Compiled::Program(ops)) => Ok(ops),
            Ok(_) => Err(format!("{text} isn't a number or a formula")),
            Err(e) => Err(e.msg),
        };
        (End { text, literal, ops }, comp.deps)
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

    /// How a message names an end: a literal as written, a formula with its value (`B7 (0.5 1/s)`).
    fn end_label(end: &End, b: f64, q: &Quant) -> String {
        if end.literal {
            end.text.clone()
        } else {
            format!("{} ({})", end.text, fmt_quantity(b, q))
        }
    }

    /// The cell's result, or the error that its value is outside its input's range (or that the
    /// range doesn't fit it, or can't be read).
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
                return Err(local(format!("{} must be {} {}", r.name, if i == 0 { "≥" } else { "≤" }, Self::end_label(end, b, &q)), None));
            }
        }
        res
    }

    /// The range of the input at `k` in the numbers its literal is written in, with the ends'
    /// current values; `None` when it has none or isn't a number literal. An end that doesn't
    /// evaluate or fit is left out (the cell shows why).
    pub fn input_range(&self, k: CellKey) -> Option<InputRange> {
        let r = self.ranges.get(&k)?;
        let Ok(Value::Num(own)) = self.trace_cell(k).result else { return None };
        crate::ops::cell_literal(&self.wb.cell_text(k))?;
        let mut out = InputRange::default();
        for (i, end) in r.ends.iter().enumerate() {
            let Some(end) = end else { continue };
            let Ok((b, q)) = self.end_value(&end.ops) else { continue };
            if fits(&q, &own.q) {
                let v = Some((own.q.disp.to_display(b), Self::end_label(end, b, &q)));
                if i == 0 {
                    out.min = v;
                } else {
                    out.max = v;
                }
            }
        }
        Some(out)
    }

    /// Sets the range of the input `name` (empty text: no bound on that side). Each end is a number
    /// in the input's dimension, or a formula (`B7`, `max_damping`) that gives one. A formula whose
    /// cells are empty or have errors right now is accepted: the input says so until they're fixed.
    pub fn set_range(&mut self, name: &str, min: &str, max: &str) -> Result<Edit, String> {
        let def = self.wb.names.get(name).filter(|d| d.input).ok_or_else(|| format!("{name} isn't an input"))?.clone();
        let own = match self.trace_cell(def.cell).result {
            Ok(Value::Num(n)) => n.q,
            _ => return Err(format!("{name} needs a number before it can have a range")),
        };
        let mut vals = [None, None];
        let mut cells = [None, None];
        for (i, text) in [min, max].into_iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let cell = self.end_cell(text, def.cell);
            let (end, _) = self.compile_end(&cell, def.cell);
            if let Err(m) = &end.ops {
                return Err(format!("{}: {m}", LABELS[i]));
            }
            match self.end_value(&end.ops) {
                Ok((b, q)) => {
                    if !fits(&q, &own) {
                        return Err(format!("{} {} is {}, but {name} is {}", LABELS[i], end.text, q.dim, own.dim));
                    }
                    vals[i] = Some(b);
                }
                // a formula that can't run now (an empty cell, an error) is the input's to report
                Err(_) if !end.literal && end.ops.as_ref().is_ok_and(|ops| run_program(&View(self), ops).is_err()) => {}
                Err(m) => return Err(format!("{}: {m}", LABELS[i])),
            }
            cells[i] = Some(cell);
        }
        if let [Some(lo), Some(hi)] = vals {
            if lo > hi {
                return Err(format!("min {} is more than max {}", min.trim(), max.trim()));
            }
        }
        let [min, max] = cells;
        Ok(self.apply(Edit::Name { name: name.to_string(), def: Some(NameDef { min, max, ..def }) }))
    }
}
