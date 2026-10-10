//! Units. A quantity's magnitudes are always canonical (base units); the
//! dimension is a sparse map base-dimension → rational exponent; the display
//! unit is purely presentational and is composed by `*`, `/`, `^`, cancelling
//! identical factors only.

use crate::rational::Rational;
use std::fmt;
use std::sync::Arc;

/// Name → exponent terms, shared: every value carries a dimension and a display
/// unit, so cloning them must not allocate. None (dimensionless, no unit), the
/// common case, is no pointer at all, so copying it touches no reference count.
/// (A thin pointer: values carry two of these, and their size is copying cost.)
#[derive(Clone, Default)]
pub struct Terms(Option<Arc<Vec<(Arc<str>, Rational)>>>);

impl Terms {
    fn ptr_eq(a: &Terms, b: &Terms) -> bool {
        match (&a.0, &b.0) {
            (None, None) => true,
            (Some(x), Some(y)) => Arc::ptr_eq(x, y),
            _ => false,
        }
    }
}

impl std::ops::Deref for Terms {
    type Target = [(Arc<str>, Rational)];
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().map_or(&[], |v| v.as_slice())
    }
}

impl From<Vec<(Arc<str>, Rational)>> for Terms {
    fn from(v: Vec<(Arc<str>, Rational)>) -> Terms {
        Terms(if v.is_empty() { None } else { Some(Arc::new(v)) })
    }
}

impl PartialEq for Terms {
    fn eq(&self, o: &Terms) -> bool {
        Terms::ptr_eq(self, o) || **self == **o
    }
}

impl fmt::Debug for Terms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

fn no_terms() -> Terms {
    Terms(None)
}

/// Adds `o`'s exponents into `a`'s, keeping `a`'s order; zero exponents drop out.
fn merge_terms(a: &Terms, o: &Terms, sign: Rational) -> Terms {
    if o.is_empty() {
        return a.clone();
    }
    if a.is_empty() && sign == Rational::ONE {
        return o.clone();
    }
    let mut v = a.to_vec();
    for (n, e) in o.iter() {
        let e = e.mul(sign);
        match v.iter_mut().find(|(m, _)| m == n) {
            Some(slot) => slot.1 = slot.1.add(e),
            None => v.push((n.clone(), e)),
        }
    }
    v.retain(|(_, e)| !e.is_zero());
    if v.is_empty() { no_terms() } else { v.into() }
}

fn pow_terms(a: &Terms, r: Rational) -> Terms {
    if r == Rational::ONE || a.is_empty() {
        return a.clone();
    }
    let v: Vec<_> = a.iter().map(|(n, e)| (n.clone(), e.mul(r))).filter(|(_, e)| !e.is_zero()).collect();
    if v.is_empty() { no_terms() } else { v.into() }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Dim(pub Terms);

impl Default for Dim {
    fn default() -> Dim {
        Dim::none()
    }
}

impl Dim {
    pub fn none() -> Dim {
        Dim(no_terms())
    }
    pub fn base(name: &str) -> Dim {
        Dim(vec![(name.into(), Rational::ONE)].into())
    }
    pub fn is_none(&self) -> bool {
        self.0.is_empty()
    }
    pub fn mul(&self, o: &Dim) -> Dim {
        let m = merge_terms(&self.0, &o.0, Rational::ONE);
        if Terms::ptr_eq(&m, &self.0) || Terms::ptr_eq(&m, &o.0) || m.len() < 2 {
            return Dim(m);
        }
        // dimensions are kept sorted so equal dimensions compare equal
        let mut v = m.to_vec();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        Dim(v.into())
    }
    pub fn pow(&self, r: Rational) -> Dim {
        Dim(pow_terms(&self.0, r))
    }
    pub fn inv(&self) -> Dim {
        self.pow(Rational::int(-1))
    }
}

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return write!(f, "dimensionless");
        }
        write!(f, "{}", format_terms(&self.0))
    }
}

/// `kg*m/s^2`, `1/s`, `m^1/2`, `kg/(m*s^2)`.
pub fn format_terms(terms: &[(Arc<str>, Rational)]) -> String {
    let fmt_term = |n: &str, e: Rational| {
        if e == Rational::ONE {
            n.to_string()
        } else if e.den == 1 {
            format!("{n}^{}", e.num)
        } else {
            format!("{n}^{}", e)
        }
    };
    let pos: Vec<String> = terms.iter().filter(|(_, e)| e.num > 0).map(|(n, e)| fmt_term(n, *e)).collect();
    let neg: Vec<String> = terms.iter().filter(|(_, e)| e.num < 0).map(|(n, e)| fmt_term(n, e.neg())).collect();
    let mut s = if pos.is_empty() { if neg.is_empty() { String::new() } else { "1".into() } } else { pos.join("*") };
    if !neg.is_empty() {
        s.push('/');
        if neg.len() == 1 {
            s.push_str(&neg[0]);
        } else {
            s.push_str(&format!("({})", neg.join("*")));
        }
    }
    s
}

/// The unit a value is rendered in. `factor` converts display → canonical:
/// canonical = shown * factor + offset. `offset` is non-zero only for a lone
/// affine unit (°C, °F, date).
#[derive(Clone, PartialEq, Debug)]
pub struct DispUnit {
    pub terms: Terms,
    pub factor: f64,
    pub offset: f64,
}

impl DispUnit {
    pub fn none() -> DispUnit {
        DispUnit { terms: no_terms(), factor: 1.0, offset: 0.0 }
    }
    pub fn named(name: &str, factor: f64) -> DispUnit {
        DispUnit { terms: vec![(name.into(), Rational::ONE)].into(), factor, offset: 0.0 }
    }
    pub fn is_none(&self) -> bool {
        self.terms.is_empty()
    }
    pub fn mul(&self, o: &DispUnit) -> DispUnit {
        DispUnit { terms: merge_terms(&self.terms, &o.terms, Rational::ONE), factor: self.factor * o.factor, offset: 0.0 }
    }
    pub fn pow(&self, r: Rational) -> DispUnit {
        let factor = if r == Rational::ONE { self.factor } else { self.factor.powf(r.to_f64()) };
        DispUnit { terms: pow_terms(&self.terms, r), factor, offset: 0.0 }
    }
    pub fn to_display(&self, canonical: f64) -> f64 {
        (canonical - self.offset) / self.factor
    }
    pub fn to_canonical(&self, shown: f64) -> f64 {
        shown * self.factor + self.offset
    }
    pub fn is_date(&self) -> bool {
        self.terms.len() == 1 && &*self.terms[0].0 == "date"
    }
}

impl fmt::Display for DispUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format_terms(&self.terms))
    }
}

/// Everything about a value except its magnitudes.
#[derive(Clone, PartialEq, Debug)]
pub struct Quant {
    pub dim: Dim,
    pub disp: DispUnit,
    /// `Some(delta unit)` for absolute values of affine units (20 °C, a date).
    /// The delta unit is how a difference of two of them displays.
    pub absolute: Option<Arc<DispUnit>>,
}

impl Quant {
    pub fn none() -> Quant {
        Quant { dim: Dim::none(), disp: DispUnit::none(), absolute: None }
    }
    pub fn is_dimensionless(&self) -> bool {
        self.dim.is_none() && self.absolute.is_none()
    }
    /// For `*`, `/`, `^` etc.: absolute values go through their linear unit
    /// (°C → K-scaled Δ°C, date → days).
    pub fn linear(&self) -> Quant {
        match &self.absolute {
            Some(delta) => Quant { dim: self.dim.clone(), disp: (**delta).clone(), absolute: None },
            None => self.clone(),
        }
    }
}

/// What a unit declaration cell evaluates to.
#[derive(Clone, PartialEq, Debug)]
pub struct UnitInfo {
    pub name: Arc<str>,
    pub dim: Dim,
    pub factor: f64,
    /// `Some(offset)` for affine units: canonical = x * factor + offset.
    pub affine: Option<f64>,
    /// For affine units: how differences display.
    pub delta: Option<DispUnit>,
    /// `[name]` as a display unit, built once so applying a unit doesn't allocate.
    pub disp: DispUnit,
}

impl UnitInfo {
    pub fn new(name: Arc<str>, dim: Dim, factor: f64, affine: Option<f64>, delta: Option<DispUnit>) -> UnitInfo {
        let disp = DispUnit { terms: vec![(name.clone(), Rational::ONE)].into(), factor, offset: 0.0 };
        UnitInfo { name, dim, factor, affine, delta, disp }
    }
}

// ---- unit expression grammar: `kg*m/s^2`, `1/s`, `m^1/2`, `(m/s)^2` ----

#[derive(Clone, Debug, PartialEq)]
pub struct UnitExpr {
    /// Unit names with exponents, identical names merged, in written order.
    pub terms: Vec<(String, Rational)>,
}

#[derive(Debug, Clone, PartialEq)]
enum UTok {
    Name(String),
    Int(i64),
    Op(char),
}

fn utokens(s: &str) -> Result<Vec<UTok>, String> {
    let mut out = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if "*/^()".contains(c) {
            out.push(UTok::Op(c));
            i += 1;
        } else if c == '-' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            let st = i;
            i += 1;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            let t: String = cs[st..i].iter().collect();
            out.push(UTok::Int(t.parse().map_err(|_| format!("bad number {t}"))?));
        } else if c.is_ascii_digit() {
            let st = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            let t: String = cs[st..i].iter().collect();
            out.push(UTok::Int(t.parse().map_err(|_| format!("bad number {t}"))?));
        } else {
            let st = i;
            while i < cs.len() && !cs[i].is_whitespace() && !"*/^()".contains(cs[i]) {
                i += 1;
            }
            out.push(UTok::Name(cs[st..i].iter().collect()));
        }
    }
    Ok(out)
}

struct UParser {
    t: Vec<UTok>,
    i: usize,
}

impl UParser {
    fn peek(&self) -> Option<&UTok> {
        self.t.get(self.i)
    }
    fn expr(&mut self) -> Result<Vec<(String, Rational)>, String> {
        let mut acc = self.term()?;
        while let Some(UTok::Op(c @ ('*' | '/'))) = self.peek().cloned() {
            self.i += 1;
            let rhs = self.term()?;
            let sign = if c == '/' { Rational::int(-1) } else { Rational::ONE };
            for (n, e) in rhs {
                acc.push((n, e.mul(sign)));
            }
        }
        Ok(acc)
    }
    fn term(&mut self) -> Result<Vec<(String, Rational)>, String> {
        let base = match self.peek().cloned() {
            Some(UTok::Name(n)) => {
                self.i += 1;
                vec![(n, Rational::ONE)]
            }
            Some(UTok::Int(1)) => {
                self.i += 1;
                vec![]
            }
            Some(UTok::Int(n)) => return Err(format!("only 1 may appear as a number inside a unit (found {n})")),
            Some(UTok::Op('(')) => {
                self.i += 1;
                let e = self.expr()?;
                if self.peek() != Some(&UTok::Op(')')) {
                    return Err("missing )".into());
                }
                self.i += 1;
                e
            }
            Some(t) => return Err(format!("unexpected {t:?} in unit")),
            None => return Err("unit ends unexpectedly".into()),
        };
        if self.peek() == Some(&UTok::Op('^')) {
            self.i += 1;
            let num = match self.peek().cloned() {
                Some(UTok::Int(n)) => n,
                _ => return Err("expected a number after ^".into()),
            };
            self.i += 1;
            let mut exp = Rational::int(num);
            // `^1/2` is a rational exponent; `^2/s` is a division
            if self.peek() == Some(&UTok::Op('/')) {
                if let Some(UTok::Int(d)) = self.t.get(self.i + 1).cloned() {
                    if d == 0 {
                        return Err("zero denominator in exponent".into());
                    }
                    self.i += 2;
                    exp = Rational::new(num, d);
                }
            }
            return Ok(base.into_iter().map(|(n, e)| (n, e.mul(exp))).collect());
        }
        Ok(base)
    }
}

pub fn parse_unit(s: &str) -> Result<UnitExpr, String> {
    let toks = utokens(s)?;
    if toks.is_empty() {
        return Err("empty unit".into());
    }
    let mut p = UParser { t: toks, i: 0 };
    let raw = p.expr()?;
    if p.i != p.t.len() {
        return Err(format!("unexpected {:?} in unit", p.t[p.i]));
    }
    let mut terms: Vec<(String, Rational)> = Vec::new();
    for (n, e) in raw {
        match terms.iter_mut().find(|(m, _)| *m == n) {
            Some(slot) => slot.1 = slot.1.add(e),
            None => terms.push((n, e)),
        }
    }
    terms.retain(|(_, e)| !e.is_zero());
    Ok(UnitExpr { terms })
}

/// The result of applying a bracketed unit: either a linear unit or a lone
/// affine one.
pub struct Resolved {
    pub dim: Dim,
    pub disp: DispUnit,
    pub absolute: Option<Arc<DispUnit>>,
}

pub fn resolve(expr: &UnitExpr, lookup: &mut dyn FnMut(&str) -> Result<Arc<UnitInfo>, String>) -> Result<Resolved, String> {
    if let [(name, e)] = expr.terms.as_slice() {
        if *e == Rational::ONE {
            let u = lookup(name)?;
            if let Some(off) = u.affine {
                let delta = u.delta.clone().unwrap_or_else(DispUnit::none);
                return Ok(Resolved {
                    dim: u.dim.clone(),
                    disp: DispUnit { terms: vec![(u.name.clone(), Rational::ONE)].into(), factor: u.factor, offset: off },
                    absolute: Some(Arc::new(delta)),
                });
            }
        }
    }
    let mut dim = Dim::none();
    let mut disp = DispUnit::none();
    for (name, e) in &expr.terms {
        let u = lookup(name)?;
        if u.affine.is_some() {
            return Err(format!("{name} is an absolute (affine) unit and can't be combined; use Δ{name} for differences"));
        }
        dim = dim.mul(&u.dim.pow(*e));
        disp = disp.mul(&u.disp.pow(*e));
    }
    Ok(Resolved { dim, disp, absolute: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grammar() {
        let e = parse_unit("kg*m/s^2").unwrap();
        assert_eq!(
            e.terms,
            vec![("kg".into(), Rational::ONE), ("m".into(), Rational::ONE), ("s".into(), Rational::int(-2))]
        );
        let e = parse_unit("m^1/2").unwrap();
        assert_eq!(e.terms, vec![("m".into(), Rational::new(1, 2))]);
        let e = parse_unit("m^2/s").unwrap();
        assert_eq!(e.terms, vec![("m".into(), Rational::int(2)), ("s".into(), Rational::int(-1))]);
        let e = parse_unit("(m/s)^2").unwrap();
        assert_eq!(e.terms, vec![("m".into(), Rational::int(2)), ("s".into(), Rational::int(-2))]);
        let e = parse_unit("1/s").unwrap();
        assert_eq!(e.terms, vec![("s".into(), Rational::int(-1))]);
        assert!(parse_unit("m*").is_err());
    }
    #[test]
    fn display() {
        let t = |v: &[(&str, i64)]| v.iter().map(|(n, e)| (Arc::<str>::from(*n), Rational::int(*e))).collect::<Vec<_>>();
        assert_eq!(format_terms(&t(&[("kg", 1), ("m", 1), ("s", -2)])), "kg*m/s^2");
        assert_eq!(format_terms(&t(&[("s", -1)])), "1/s");
        assert_eq!(format_terms(&t(&[("kg", 1), ("m", -1), ("s", -2)])), "kg/(m*s^2)");
    }
}
