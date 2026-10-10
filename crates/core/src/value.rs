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
    /// Element `i` (row-major) of a computed cell's value; dragging it goal-seeks an input.
    Derived(CellKey, usize),
}

/// Per-element provenance of an array, dropped by any operation that computes.
/// The common cases (a cell's own value, one element of it) don't allocate.
#[derive(Clone, Debug, Default)]
pub enum Provs {
    #[default]
    None,
    /// A cell's whole value: element `i` is `Literal(k)` for a number literal, else `Derived(k, i)`.
    Cell(CellKey, bool),
    /// A single element: `Literal(cell)` or `Derived(cell, index)` (an index past
    /// u32 isn't a spreadsheet's; it's kept this small so values stay small).
    One { cell: CellKey, index: u32, literal: bool },
    List(Arc<[Prov]>),
}

impl Provs {
    pub fn is_none(&self) -> bool {
        matches!(self, Provs::None)
    }
    /// Provenance of element `i` (`Prov::None` when unknown).
    pub fn get(&self, i: usize) -> Prov {
        match self {
            Provs::None => Prov::None,
            Provs::Cell(k, true) => Prov::Literal(*k),
            Provs::Cell(k, false) => Prov::Derived(*k, i),
            Provs::One { cell, literal: true, .. } if i == 0 => Prov::Literal(*cell),
            Provs::One { cell, index, .. } if i == 0 => Prov::Derived(*cell, *index as usize),
            Provs::One { .. } => Prov::None,
            Provs::List(l) => l.get(i).copied().unwrap_or(Prov::None),
        }
    }
    /// One element's provenance.
    pub fn one(p: Prov) -> Provs {
        match p {
            Prov::None => Provs::None,
            Prov::Literal(cell) => Provs::One { cell, index: 0, literal: true },
            Prov::Derived(cell, i) => Provs::One { cell, index: i.min(u32::MAX as usize) as u32, literal: false },
        }
    }
    /// The first `n` elements' provenance (`None` when there's none).
    pub fn to_vec(&self, n: usize) -> Option<Vec<Prov>> {
        (!self.is_none()).then(|| (0..n).map(|i| self.get(i)).collect())
    }
}

impl PartialEq for Provs {
    fn eq(&self, o: &Provs) -> bool {
        match (self, o) {
            (Provs::None, Provs::None) => true,
            (Provs::None, _) | (_, Provs::None) => false,
            (Provs::Cell(a, x), Provs::Cell(b, y)) => a == b && x == y,
            (Provs::List(a), Provs::List(b)) => a == b,
            // mixed representations: element by element, as far as either says anything
            _ => {
                let n = |p: &Provs| match p {
                    Provs::List(l) => l.len(),
                    _ => 1,
                };
                (0..n(self).max(n(o))).all(|i| self.get(i) == o.get(i))
            }
        }
    }
}

/// An array's shape. Ranks 0-2 (nearly every value) are stored inline, so copying,
/// comparing and dropping one doesn't touch the heap.
#[derive(Clone)]
pub enum Shape {
    Small { rank: u8, dims: [usize; 2] },
    Big(Box<[usize]>),
}

impl Shape {
    pub const SCALAR: Shape = Shape::Small { rank: 0, dims: [0, 0] };
    pub fn as_slice(&self) -> &[usize] {
        self
    }
}

impl Default for Shape {
    fn default() -> Shape {
        Shape::SCALAR
    }
}

impl std::ops::Deref for Shape {
    type Target = [usize];
    fn deref(&self) -> &[usize] {
        match self {
            Shape::Small { rank, dims } => &dims[..*rank as usize],
            Shape::Big(v) => v,
        }
    }
}

impl From<&[usize]> for Shape {
    fn from(s: &[usize]) -> Shape {
        match *s {
            [] => Shape::SCALAR,
            [a] => Shape::Small { rank: 1, dims: [a, 0] },
            [a, b] => Shape::Small { rank: 2, dims: [a, b] },
            _ => Shape::Big(s.into()),
        }
    }
}

impl From<Vec<usize>> for Shape {
    fn from(v: Vec<usize>) -> Shape {
        Shape::from(v.as_slice())
    }
}

impl PartialEq for Shape {
    fn eq(&self, o: &Shape) -> bool {
        **self == **o
    }
}

impl std::fmt::Debug for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

/// An array's magnitudes: a scalar is stored inline, an array is one shared allocation.
#[derive(Clone, Debug)]
pub enum Data {
    One(f64),
    Many(Arc<[f64]>),
}

impl std::ops::Deref for Data {
    type Target = [f64];
    fn deref(&self) -> &[f64] {
        match self {
            Data::One(x) => std::slice::from_ref(x),
            Data::Many(a) => a,
        }
    }
}

impl PartialEq for Data {
    fn eq(&self, o: &Data) -> bool {
        **self == **o
    }
}

impl From<Vec<f64>> for Data {
    fn from(v: Vec<f64>) -> Data {
        if v.len() == 1 { Data::One(v[0]) } else { Data::Many(v.into()) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Num {
    pub shape: Shape,
    pub data: Data,
    pub q: Quant,
    pub prov: Provs,
}

impl Num {
    pub fn scalar(x: f64, q: Quant) -> Num {
        Num { shape: Shape::SCALAR, data: Data::One(x), q, prov: Provs::None }
    }
    pub fn plain(x: f64) -> Num {
        Num::scalar(x, Quant::none())
    }
    pub fn vector(v: Vec<f64>, q: Quant) -> Num {
        Num { shape: Shape::Small { rank: 1, dims: [v.len(), 0] }, data: v.into(), q, prov: Provs::None }
    }
    pub fn with_shape(shape: impl Into<Shape>, v: Vec<f64>, q: Quant) -> Num {
        let shape = shape.into();
        debug_assert_eq!(shape.iter().product::<usize>(), v.len());
        Num { shape, data: v.into(), q, prov: Provs::None }
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
    /// Element `i` at decreasing precision, for fitting it in a column: see [`fmt_num_shorter`].
    pub fn fmt_elem_shorter(&self, i: usize) -> Vec<String> {
        let shown = self.q.disp.to_display(self.data[i]);
        if self.q.disp.is_date() && self.q.absolute.is_some() {
            return vec![fmt_date(shown)];
        }
        let unit = if self.q.disp.is_none() { String::new() } else { format!(" {}", self.q.disp) };
        fmt_num_shorter(shown).into_iter().map(|n| format!("{}{unit}", group_thousands(&n))).collect()
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
                            crate::chart::Mark::Path => "path",
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

/// `x` the way [`fmt_num`] writes it, then shorter and shorter, as a spreadsheet does when a
/// number doesn't fit its column: drop decimals, then scientific notation with fewer mantissa
/// digits — but never below 3 significant digits, so a shortened number stays close to the real
/// one (`1234.5678` can become `1235`, never `1e3`). The caller shows the first that fits, and
/// `###` when none does.
pub fn fmt_num_shorter(x: f64) -> Vec<String> {
    let full = fmt_num(x);
    let mut out = vec![full.clone()];
    if !x.is_finite() || x == 0.0 {
        return out;
    }
    // keep only forms shorter (as shown, with thousands separators) than the last one kept
    let shown_len = |s: &str| group_thousands(s).chars().count();
    let mut push = |s: String| {
        if shown_len(&s) < out.last().map_or(usize::MAX, |l| shown_len(l)) {
            out.push(s);
        }
    };
    let ax = x.abs();
    if !full.contains('e') {
        // the most significant digit sits at 10^lead; keep it and the two after it
        let lead = ax.log10().floor() as i32;
        let decimals = full.split_once('.').map_or(0, |(_, f)| f.len()) as i32;
        let min_dec = (2 - lead).max(0);
        for d in (min_dec..decimals).rev() {
            push(trim_zeros(format!("{:.*}", d as usize, x)));
        }
    }
    // a mantissa with 2 decimals is 3 significant digits
    for m in (2..9).rev() {
        let s = format!("{:.*e}", m, x);
        push(match s.split_once('e') {
            Some((mant, e)) => format!("{}e{e}", if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.') } else { mant }),
            None => s,
        });
    }
    out
}

fn trim_zeros(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
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
pub fn broadcast2(a: &Num, b: &Num, f: impl Fn(f64, f64) -> f64) -> Result<(Shape, Vec<f64>), String> {
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
    fn shorter() {
        assert_eq!(fmt_num_shorter(5.0), vec!["5"]);
        let v = fmt_num_shorter(1234.5678);
        assert_eq!(&v[..5], ["1234.5678", "1234.568", "1234.57", "1234.6", "1235"]);
        // never fewer than 3 significant digits: 1234.5678 stops at 1235, not 1e3
        assert_eq!(v.last().unwrap(), "1235");
        let v = fmt_num_shorter(0.000123456);
        assert_eq!(v, ["0.000123456", "0.00012346", "0.0001235", "0.000123", "1.23e-4"]);
        assert_eq!(fmt_num_shorter(1.2e-17), ["1.2e-17"]);
        // each step is shorter than the last
        for x in [1.0 / 3.0, -98765.4321, 6.02214076e23, 1.5e-7, 123456789.0] {
            let v = fmt_num_shorter(x);
            let len = |s: &str| group_thousands(s).chars().count();
            assert!(v.windows(2).all(|w| len(&w[0]) > len(&w[1])), "{v:?}");
        }
        assert_eq!(fmt_num_shorter(123456789.0), vec!["123456789", "1.234568e8", "1.23457e8", "1.2346e8", "1.235e8", "1.23e8"]);
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
