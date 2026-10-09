//! Runtime values: rank-polymorphic arrays carrying a unit.

use crate::chart::Chart;
use crate::ids::CellKey;
use crate::lex::civil_from_days;
use crate::units::{Quant, UnitInfo};
use std::sync::Arc;

/// Where an array element came from, for chart bidirectional editing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prov {
    None,
    /// Read directly from a literal number cell; dragging it writes that cell.
    Literal(CellKey),
    /// Read from a computed cell.
    Derived(CellKey),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Num {
    pub shape: Vec<usize>,
    pub data: Arc<Vec<f64>>,
    pub q: Quant,
    /// Per-element provenance; dropped by any operation that computes.
    pub prov: Option<Arc<Vec<Prov>>>,
}

impl Num {
    pub fn scalar(x: f64, q: Quant) -> Num {
        Num { shape: vec![], data: Arc::new(vec![x]), q, prov: None }
    }
    pub fn plain(x: f64) -> Num {
        Num::scalar(x, Quant::none())
    }
    pub fn vector(v: Vec<f64>, q: Quant) -> Num {
        Num { shape: vec![v.len()], data: Arc::new(v), q, prov: None }
    }
    pub fn with_shape(shape: Vec<usize>, v: Vec<f64>, q: Quant) -> Num {
        debug_assert_eq!(shape.iter().product::<usize>(), v.len());
        Num { shape, data: Arc::new(v), q, prov: None }
    }
    pub fn rank(&self) -> usize {
        self.shape.len()
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn as_scalar(&self) -> Option<f64> {
        if self.shape.is_empty() {
            Some(self.data[0])
        } else {
            None
        }
    }
    pub fn shown(&self, i: usize) -> f64 {
        self.q.disp.to_display(self.data[i])
    }
    pub fn fmt_elem(&self, i: usize) -> String {
        fmt_quantity(self.data[i], &self.q)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub shape: Vec<usize>,
    pub data: Arc<Vec<Arc<str>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(Num),
    Text(Text),
    Chart(Arc<Chart>),
    /// A unit declaration's value.
    Unit(Arc<UnitInfo>),
    /// A `dim` declaration.
    Dim(Arc<str>),
    /// A word definition.
    Word(Arc<str>),
}

impl Value {
    pub fn text(s: &str) -> Value {
        Value::Text(Text { shape: vec![], data: Arc::new(vec![s.into()]) })
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Text(_) => "text",
            Value::Chart(_) => "chart",
            Value::Unit(_) => "unit",
            Value::Dim(_) => "dimension",
            Value::Word(_) => "word",
        }
    }
    pub fn shape(&self) -> &[usize] {
        match self {
            Value::Num(n) => &n.shape,
            Value::Text(t) => &t.shape,
            _ => &[],
        }
    }
    /// A one-line description showing at most `max` elements per axis:
    /// `5.3 km`, `[1, 2, 3] m`, `[[1, 2], [3, 4]]`, `chart · line (5 points) · 6×14 cells`.
    pub fn summary(&self, max: usize) -> String {
        fn list<T>(items: &[T], max: usize, f: impl Fn(&T) -> String) -> String {
            let mut parts: Vec<String> = items.iter().take(max).map(f).collect();
            if items.len() > max {
                parts.push(format!("… +{}", items.len() - max));
            }
            format!("[{}]", parts.join(", "))
        }
        match self {
            Value::Num(n) => {
                if n.shape.is_empty() {
                    return n.fmt_elem(0);
                }
                let is_date = n.q.disp.is_date() && n.q.absolute.is_some();
                let elem = |x: &f64| {
                    let s = n.q.disp.to_display(*x);
                    if is_date { fmt_date(s) } else { group_thousands(&fmt_num(s)) }
                };
                let body = match n.shape.as_slice() {
                    [_] => list(&n.data, max, elem),
                    [r, c] => {
                        let rows: Vec<&[f64]> = (0..*r).map(|i| &n.data[i * c..(i + 1) * c]).collect();
                        list(&rows, max, |row| list(row, max, elem))
                    }
                    s => format!("rank {} array {:?}", s.len(), s),
                };
                if n.q.disp.is_none() || is_date {
                    body
                } else {
                    format!("{body} {}", n.q.disp)
                }
            }
            Value::Text(t) => match t.shape.as_slice() {
                [] => t.data[0].to_string(),
                [_] => list(&t.data, max, |s| s.to_string()),
                [r, c] => {
                    let rows: Vec<&[Arc<str>]> = (0..*r).map(|i| &t.data[i * c..(i + 1) * c]).collect();
                    list(&rows, max, |row| list(row, max, |s| s.to_string()))
                }
                s => format!("rank {} text array", s.len()),
            },
            Value::Chart(c) => {
                let layers: Vec<String> = c
                    .layers
                    .iter()
                    .map(|l| {
                        let m = match l.mark {
                            crate::chart::Mark::Line => "line",
                            crate::chart::Mark::Scatter => "scatter",
                            crate::chart::Mark::Bar => "bar",
                        };
                        format!("{m} ({} points)", l.ys.len())
                    })
                    .collect();
                let title = c.title.as_ref().map(|t| format!(" \"{t}\"")).unwrap_or_default();
                format!("chart{title} · {} · {}×{} cells", layers.join(" + "), c.cols, c.rows)
            }
            other => other.display_at(0, 0),
        }
    }

    /// Spill extent (rows, cols) for this value; (1, 1) means no spill.
    pub fn spill_size(&self) -> (usize, usize) {
        match self {
            Value::Chart(c) => (c.rows.max(1), c.cols.max(1)),
            _ => match self.shape() {
                [] => (1, 1),
                [n] => ((*n).max(1), 1),
                [r, c, ..] => ((*r).max(1), (*c).max(1)),
            },
        }
    }
    /// Text shown for the element at (dr, dc) of the spill region.
    pub fn display_at(&self, dr: usize, dc: usize) -> String {
        match self {
            Value::Num(n) => {
                let i = match n.shape.as_slice() {
                    [] => 0,
                    [_] => dr,
                    [_, c, ..] => dr * c + dc,
                };
                if n.data.is_empty() {
                    return "[]".into();
                }
                if n.rank() > 2 {
                    return format!("rank {} array", n.rank());
                }
                n.fmt_elem(i)
            }
            Value::Text(t) => {
                let i = match t.shape.as_slice() {
                    [] => 0,
                    [_] => dr,
                    [_, c, ..] => dr * c + dc,
                };
                t.data.get(i).map(|s| s.to_string()).unwrap_or_else(|| "[]".into())
            }
            Value::Chart(_) => String::new(),
            Value::Unit(u) => {
                let target = match &u.delta {
                    Some(d) => format!("{}", d),
                    None => String::new(),
                };
                match u.affine {
                    Some(off) => format!("[{}] affine {} {target} + {}", u.name, fmt_num(u.factor), fmt_num(off)),
                    None => format!("[{}] = {} base {}", u.name, fmt_num(u.factor), u.dim),
                }
            }
            Value::Dim(d) => format!("dim {d}"),
            Value::Word(w) => format!(": {w}"),
        }
    }
}

pub fn fmt_num(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let ax = x.abs();
    if ax != 0.0 && !(1e-6..1e15).contains(&ax) {
        let s = format!("{:.9e}", x);
        // trim mantissa zeros: 1.500000000e3 -> 1.5e3
        if let Some((m, e)) = s.split_once('e') {
            let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
            return format!("{m}e{e}");
        }
        return s;
    }
    if x == x.trunc() && ax < 1e15 {
        return format!("{}", x as i64);
    }
    // 8 significant digits
    let digits = (8 - (ax.log10().floor() as i32 + 1)).clamp(0, 15) as usize;
    let s = format!("{:.*}", digits, x);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

/// `1234567.5` → `1,234,567.5` (plain decimal notation only).
pub fn group_thousands(s: &str) -> String {
    if s.contains(['e', 'N', '∞']) {
        return s.to_string();
    }
    let (sign, rest) = s.strip_prefix('-').map(|r| ("-", r)).unwrap_or(("", s));
    let (int, frac) = rest.split_once('.').map(|(i, f)| (i, Some(f))).unwrap_or((rest, None));
    if int.len() <= 3 {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + int.len() / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

pub fn fmt_date(days: f64) -> String {
    let d = days.floor() as i64;
    let (y, m, dd) = civil_from_days(d);
    format!("{y:04}-{m:02}-{dd:02}")
}

pub fn fmt_quantity(canonical: f64, q: &Quant) -> String {
    let shown = q.disp.to_display(canonical);
    if q.disp.is_date() && q.absolute.is_some() {
        return fmt_date(shown);
    }
    let n = group_thousands(&fmt_num(shown));
    if q.disp.is_none() {
        n
    } else {
        format!("{n} {}", q.disp)
    }
}

/// Leading-axis broadcasting: shapes agree if one is a prefix of the other.
pub fn broadcast2(a: &Num, b: &Num, f: impl Fn(f64, f64) -> f64) -> Result<(Vec<usize>, Vec<f64>), String> {
    let (la, lb) = (a.data.len(), b.data.len());
    if a.shape == b.shape {
        return Ok((a.shape.clone(), a.data.iter().zip(b.data.iter()).map(|(x, y)| f(*x, *y)).collect()));
    }
    if a.shape.len() <= b.shape.len() && b.shape.starts_with(&a.shape) {
        let inner = if la == 0 { 0 } else { lb / la };
        let v = (0..lb).map(|i| f(a.data[i / inner.max(1)], b.data[i])).collect();
        return Ok((b.shape.clone(), v));
    }
    if b.shape.len() < a.shape.len() && a.shape.starts_with(&b.shape) {
        let inner = if lb == 0 { 0 } else { la / lb };
        let v = (0..la).map(|i| f(a.data[i], b.data[i / inner.max(1)])).collect();
        return Ok((a.shape.clone(), v));
    }
    Err(format!("shapes {:?} and {:?} don't match", a.shape, b.shape))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(fmt_num(5.0), "5");
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
        assert_eq!(fmt_num(-2.5), "-2.5");
        assert_eq!(fmt_num(1.0 / 3.0), "0.33333333");
        assert_eq!(group_thousands("1234567.25"), "1,234,567.25");
        assert_eq!(group_thousands("-1000"), "-1,000");
        assert_eq!(group_thousands("999"), "999");
        assert_eq!(fmt_num(1234.5678), "1234.5678");
        assert_eq!(fmt_num(1.5e20), "1.5e20");
        assert_eq!(fmt_num(2e-9), "2e-9");
    }
    #[test]
    fn bcast() {
        let a = Num::vector(vec![1.0, 2.0], Quant::none());
        let b = Num::with_shape(vec![2, 2], vec![10.0, 20.0, 30.0, 40.0], Quant::none());
        assert_eq!(broadcast2(&a, &b, |x, y| x + y).unwrap().1, vec![11.0, 21.0, 32.0, 42.0]);
        let s = Num::plain(1.0);
        assert_eq!(broadcast2(&b, &s, |x, y| x - y).unwrap().1, vec![9.0, 19.0, 29.0, 39.0]);
        let c = Num::vector(vec![1.0, 2.0, 3.0], Quant::none());
        assert!(broadcast2(&a, &c, |x, y| x + y).is_err());
    }
}
