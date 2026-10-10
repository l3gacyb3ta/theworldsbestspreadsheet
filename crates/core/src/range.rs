//! Input ranges: optional `min` / `max` on a named input.
//!
//! The bounds are stored as written, in the input's display unit (`0 [1/s]`), and limit scrubbing, chart
//! dragging and goal-seek (callers ask `display_range`). A typed value outside the range is an error on
//! the input's cell (`check_range`): the value is kept, never clamped, and downstream cells show `#upstream`.

use crate::engine::{local, CellResult, Edit, Engine};
use crate::ids::CellKey;
use crate::model::NameDef;
use crate::units::{format_terms, Dim, Quant};
use crate::value::Value;

/// A range; canonical units from `range`, the input's written unit from `display_range`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Range {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bound {
    Min,
    Max,
}

impl Range {
    /// Which end `x` is beyond, allowing for rounding in unit conversions.
    pub fn violated(&self, x: f64) -> Option<Bound> {
        let slack = |b: f64| 1e-9 * b.abs().max(1e-12);
        match (self.min, self.max) {
            (Some(lo), _) if x < lo - slack(lo) => Some(Bound::Min),
            (_, Some(hi)) if x > hi + slack(hi) => Some(Bound::Max),
            _ => None,
        }
    }

    /// `x` moved to the nearest end of the range.
    pub fn clamp(&self, x: f64) -> f64 {
        x.max(self.min.unwrap_or(f64::NEG_INFINITY)).min(self.max.unwrap_or(f64::INFINITY))
    }
}

impl Engine {
    /// The name and definition of the input at `k`, if it has a range.
    fn bounded(&self, k: CellKey) -> Option<(&String, &NameDef)> {
        self.wb.names.iter().find(|(_, d)| d.cell == k && (d.min.is_some() || d.max.is_some()))
    }

    /// A bound's text as a number with its quantity.
    fn bound_quant(&self, text: &str, k: CellKey) -> Result<(f64, Quant), String> {
        match self.eval_scratch(text, k.sheet).result {
            Ok(Value::Num(n)) => match n.as_scalar() {
                Some(x) if x.is_finite() => Ok((x, n.q)),
                _ => Err(format!("a bound is a single number, not {text}")),
            },
            Ok(v) => Err(format!("a bound is a number, not a {}", v.type_name())),
            Err(e) => Err(e.msg),
        }
    }

    /// The quantity an input's own text writes (its dimension and display unit), even when the cell
    /// shows an error because it is out of range.
    fn own_quant(&self, k: CellKey) -> Option<Quant> {
        match self.eval_scratch(&self.wb.cell_text(k), k.sheet).result {
            Ok(Value::Num(n)) => Some(n.q),
            _ => None,
        }
    }

    /// The range of the input at `k` in canonical units. A bound that doesn't evaluate is skipped.
    pub fn range(&self, k: CellKey) -> Option<Range> {
        let (_, d) = self.bounded(k)?;
        let get = |t: &Option<String>| t.as_deref().and_then(|t| self.bound_quant(t, k).ok()).map(|b| b.0);
        Some(Range { min: get(&d.min), max: get(&d.max) })
    }

    /// The range of the input at `k` in the unit its text is written in, which is what scrubbing and
    /// goal-seek move; `None` when the input has no range.
    pub fn display_range(&self, k: CellKey) -> Option<Range> {
        let r = self.range(k)?;
        let disp = self.own_quant(k).map(|q| q.disp);
        let conv = |x: Option<f64>| x.map(|x| disp.as_ref().map_or(x, |d| d.to_display(x)));
        Some(Range { min: conv(r.min), max: conv(r.max) })
    }

    /// The bounds of the cell at `k` as written (for messages and the inspector).
    pub fn range_text(&self, k: CellKey) -> (Option<String>, Option<String>) {
        self.wb.names.values().find(|d| d.cell == k).map(|d| (d.min.clone(), d.max.clone())).unwrap_or_default()
    }

    /// Sets the bounds of the input `name`; empty text clears a bound. Each must be a number of the
    /// input's dimension.
    pub fn set_bounds(&mut self, name: &str, min: &str, max: &str) -> Result<Edit, String> {
        let def = self.wb.names.get(name).filter(|d| d.input).ok_or_else(|| format!("{name} isn't an input"))?.clone();
        let own = self.own_quant(def.cell);
        let mut parsed = [None, None];
        let mut stored = [None, None];
        for (i, (label, text)) in [("min", min), ("max", max)].into_iter().enumerate() {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let (x, q) = self.bound_quant(text, def.cell).map_err(|e| format!("{label}: {e}"))?;
            if let Some(own) = &own {
                if q.dim != own.dim || q.absolute.is_some() != own.absolute.is_some() {
                    let dim = |d: &Dim| if d.is_none() { "a plain number".to_string() } else { format_terms(&d.0) };
                    return Err(format!("{label}: {name} is {}, but {text} is {}", dim(&own.dim), dim(&q.dim)));
                }
            }
            parsed[i] = Some(x);
            stored[i] = Some(text.to_string());
        }
        if let [Some(lo), Some(hi)] = parsed {
            if lo > hi {
                return Err(format!("min {} is above max {}", min.trim(), max.trim()));
            }
        }
        let [min, max] = stored;
        Ok(self.apply(Edit::Name { name: name.to_string(), def: Some(NameDef { min, max, ..def }) }))
    }

    /// An input outside its range is an error on its own cell, and only there.
    pub(crate) fn check_range(&self, k: CellKey, res: CellResult) -> CellResult {
        let Ok(Value::Num(n)) = &res else { return res };
        let Some((name, d)) = self.bounded(k) else { return res };
        let Some(range) = self.range(k) else { return res };
        for x in n.data.iter() {
            let (sign, text) = match range.violated(*x) {
                Some(Bound::Min) => ("≥", &d.min),
                Some(Bound::Max) => ("≤", &d.max),
                None => continue,
            };
            return Err(local(format!("{name} must be {sign} {}", text.as_deref().unwrap_or("").trim()), None));
        }
        res
    }
}
