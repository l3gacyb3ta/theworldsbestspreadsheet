//! Sensitivities of a cell to its named inputs: the inspector's list and the `sensitivity` word's
//! tornado chart. Each input is one forward-mode pass (`Engine::sensitivities`).

use crate::chart::{Chart, Layer, Mark, Xs};
use crate::engine::Engine;
use crate::ids::CellKey;
use crate::model::Kind;
use crate::units::{DispUnit, Quant};
use crate::value::{fmt_num, fmt_quantity, group_thousands, Num, Text, Value};
use std::sync::Arc;

/// The most named inputs one cell's sensitivities look at (a pass each).
pub const MAX_INPUTS: usize = 50;

/// How one element of a cell moves with one named input.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSens {
    pub input: CellKey,
    pub name: String,
    /// Canonical units of the cell per canonical unit of the input.
    pub d: f64,
    /// The derivative's units (the cell's per the input's), for display.
    pub q: Quant,
    /// What the input is written in (`USD/kW`), to say "per +100 USD/kW".
    pub input_disp: DispUnit,
    /// The input's range, `max − min` in canonical units, when it has both ends.
    pub span: Option<f64>,
    /// The input's current value, canonical.
    pub value: f64,
}

impl InputSens {
    /// `-0.8 yr per +100 USD/kW`: the change for a round step of the input, in its own unit.
    pub fn per_step(&self, out: &Quant) -> String {
        let step = round_step(self.input_disp.to_display(self.value.abs()).max(self.span.map_or(0.0, |s| s / self.input_disp.factor / 10.0)));
        let out = out.linear();
        // a readout, not a value to copy: three significant figures
        let dy = out.disp.to_canonical(round_sig(out.disp.to_display(self.d * step * self.input_disp.factor), 3));
        let unit = if self.input_disp.is_none() { String::new() } else { format!(" {}", self.input_disp) };
        format!("{} per +{}{unit}", fmt_quantity(dy, &out), group_thousands(&fmt_num(step)))
    }

    /// What the tornado chart draws: the change across the input's range, or across a tenth of its
    /// value when it has none (`None` for an absolute input, a date or temperature, without one).
    pub fn swing(&self, absolute: bool) -> Option<(f64, &'static str)> {
        match self.span {
            Some(s) => Some((self.d * s, "range")),
            None if absolute => None,
            None => Some((self.d * self.value.abs() / 10.0, "±10%")),
        }
    }
}

fn round_sig(x: f64, digits: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let p = 10f64.powi(digits - 1 - x.abs().log10().floor() as i32);
    (x * p).round() / p
}

/// A round number near a tenth of `x` (1, 2 or 5 × a power of ten), for "per +100 USD/kW".
fn round_step(x: f64) -> f64 {
    if !x.is_finite() || x <= 0.0 {
        return 1.0;
    }
    let t = x / 10.0;
    let p = 10f64.powf(t.log10().floor());
    let m = t / p;
    p * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

impl Engine {
    /// Element `index` of cell `k`'s sensitivity to each named input (a name ticked "input")
    /// upstream of it, read inside words included, in the order `upstream_inputs` gives, up to
    /// `MAX_INPUTS`.
    pub fn input_sensitivities(&self, k: CellKey, index: usize) -> Result<Vec<InputSens>, String> {
        let k = self.spill_anchor(k).unwrap_or(k);
        let named = self.upstream_inputs(k).into_iter().filter_map(|c| self.wb.names.iter().find(|(_, d)| d.cell == c && d.input).map(|(n, _)| (c, n.clone())));
        let mut out = Vec::new();
        for (input, name) in named.take(MAX_INPUTS) {
            if self.kind(input) == Kind::Number {
                out.push(self.input_sens(input, name, k, index)?);
            }
        }
        Ok(out)
    }

    /// How element `index` of `k` moves with the number cell `input`.
    pub fn input_sens(&self, input: CellKey, name: String, k: CellKey, index: usize) -> Result<InputSens, String> {
        let Some(Ok(Value::Num(iv))) = self.result(input) else { return Err(format!("{} isn't a number", self.wb.cell_label(input, None))) };
        let d = self.sensitivities(input, &[k])?.remove(0);
        let Some(&dv) = d.data.get(index) else { return Err(format!("{} has no element {index}", self.wb.cell_label(k, None))) };
        let span = self.input_range(input).and_then(|r| Some((r.max?.0 - r.min?.0) * iv.q.disp.factor));
        Ok(InputSens { input, name, d: dv, q: d.q.clone(), input_disp: iv.q.linear().disp, span, value: iv.data[0] })
    }

    /// `breakeven: -0.8 yr per +100 USD/kW`: element `index` of `k`'s local slope by `input`, for the
    /// goal-seek drag's tooltip.
    pub fn slope_text(&self, input: CellKey, k: CellKey, index: usize) -> Result<String, String> {
        let a = self.spill_anchor(k).unwrap_or(k);
        let Some(Ok(Value::Num(out))) = self.result(a) else { return Err(format!("{} isn't a number", self.wb.cell_label(a, None))) };
        let s = self.input_sens(input, String::new(), a, index)?;
        let what = self.name_of(a).map(str::to_string).unwrap_or_else(|| self.wb.cell_label(self.element_cell(a, index), Some(a.sheet)));
        Ok(format!("{what}: {}", s.per_step(&out.q)))
    }

    /// The `sensitivity` word: a bar chart of how much element `index` of `k` swings with each named
    /// input (across its range, or ±10% of its value), largest first.
    pub fn tornado(&self, k: CellKey, index: usize) -> Result<Value, String> {
        let label = self.wb.cell_label(k, None);
        let q = match self.result(self.spill_anchor(k).unwrap_or(k)) {
            Some(Ok(Value::Num(n))) => n.q.linear(),
            _ => return Err(format!("sensitivity: {label} isn't a number")),
        };
        let mut bars: Vec<(String, f64)> = Vec::new();
        for s in self.input_sensitivities(k, index)? {
            let absolute = matches!(self.result(s.input), Some(Ok(Value::Num(n))) if n.q.absolute.is_some());
            if let Some((v, how)) = s.swing(absolute) {
                bars.push((format!("{} ({how})", s.name), v));
            }
        }
        if bars.is_empty() {
            return Err(format!("sensitivity: no named inputs upstream of {label}"));
        }
        bars.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        let cats = Text { shape: vec![bars.len()], data: Arc::new(bars.iter().map(|b| Arc::<str>::from(b.0.as_str())).collect()) };
        let ys = Num::vector(bars.iter().map(|b| b.1).collect(), q);
        let mut c = Chart::single(Layer { mark: Mark::Bar, xs: Xs::Text(cats), ys });
        c.title = Some(format!("sensitivity of {}", self.wb.cell_label(k, Some(k.sheet))));
        Ok(Value::Chart(Arc::new(c)))
    }
}

#[cfg(test)]
mod tests {
    use super::round_step;


    #[test]
    fn round_steps() {
        assert_eq!(round_step(1200.0), 100.0);
        assert_eq!(round_step(5.0), 0.5);
        assert_eq!(round_step(0.3), 0.02);
        assert_eq!(round_step(0.0), 1.0);
        assert_eq!(round_step(80.0), 10.0);
    }

    #[test]
    fn significant_figures() {
        use super::round_sig;
        assert_eq!(round_sig(13159.318, 3), 13200.0);
        assert_eq!(round_sig(-0.081234, 3), -0.0812);
        assert_eq!(round_sig(0.0, 3), 0.0);
    }
}
