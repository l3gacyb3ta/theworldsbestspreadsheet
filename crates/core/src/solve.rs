//! Goal-seek: change one number cell so a computed value hits a target.
//!
//! The 1-D search first brackets outward from the input's current value
//! (steps that double, alternating up and down, nearest first), then narrows
//! the bracket: Brent's method for inputs with decimals, bisection over whole
//! numbers for dates and inputs written without decimals. It only ever returns
//! a value from a bracket that straddles the target; when there is none it
//! says why (out of reach, a jump, an error) instead of picking the closest.
//!
//! `Engine::goal_seek` evaluates by writing the input and recalculating, and
//! restores the document exactly before returning, success or not.

use crate::engine::{Edit, Engine};
use crate::ids::{CellKey, SheetId};
use crate::ops::{cell_literal, format_lit, replace_span};
use crate::units::Quant;
use crate::value::{fmt_date, fmt_num, fmt_quantity, group_thousands, Value};

/// How to search.
#[derive(Clone, Debug, PartialEq)]
pub struct Opts {
    /// The first step away from the start; each further step doubles.
    pub step: f64,
    /// How many doublings to try on each side.
    pub doublings: u32,
    /// `|f(x) − target| ≤ tol` counts as hitting the target.
    pub tol: f64,
    /// Only whole numbers are tried (dates; literals written without decimals).
    pub whole: bool,
    /// Cap on evaluations of `f`, bracketing and narrowing together.
    pub max_evals: usize,
    /// The input's range: nothing outside `lo..=hi` is tried (infinite when unbounded).
    pub lo: f64,
    pub hi: f64,
}

impl Opts {
    /// The same search kept within `lo..=hi`; whole-number searches use the whole numbers inside.
    pub fn within(mut self, lo: Option<f64>, hi: Option<f64>) -> Opts {
        self.lo = lo.unwrap_or(f64::NEG_INFINITY);
        self.hi = hi.unwrap_or(f64::INFINITY);
        if self.whole {
            (self.lo, self.hi) = (self.lo.ceil(), self.hi.floor());
        }
        self
    }

    /// The search for a literal now at `x0`, written with `decimals` places: up to 1024× its size either way
    /// (from zero: up to 10⁶ of its last decimal place; a date: ±65,536 days).
    pub fn for_literal(x0: f64, decimals: usize, is_date: bool, tol: f64) -> Opts {
        let whole = is_date || decimals == 0;
        let (step, doublings) = if is_date {
            (1.0, 16)
        } else if x0 != 0.0 {
            (x0.abs() / 16.0, 14)
        } else {
            (10f64.powi(-(decimals as i32)), 20)
        };
        Opts { step: if whole { step.max(1.0) } else { step }, doublings, tol, whole, max_evals: 120, lo: f64::NEG_INFINITY, hi: f64::INFINITY }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub x: f64,
    pub y: f64,
    pub evals: usize,
}

/// Why there is no answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    /// No two tried inputs straddle the target. Over inputs `lo..=hi` the value stayed in `ymin..=ymax`;
    /// `err` is the first input where the value was an error (the search stops there on that side).
    /// `at_lo` / `at_hi`: the search ran into the input's lower / upper bound.
    OutOfReach { lo: f64, hi: f64, ymin: f64, ymax: f64, err: Option<(f64, String)>, at_lo: bool, at_hi: bool },
    /// The value jumps over the target at `x`: from `left` just below it to `right` just above.
    Jump { x: f64, left: f64, right: f64 },
    /// The value is an error at `x`, inside a bracket (or at the start).
    Error { x: f64, msg: String },
    /// Gave up after this many evaluations.
    Evals(usize),
}

struct Counted<'a> {
    f: &'a mut dyn FnMut(f64) -> Result<f64, String>,
    n: usize,
    max: usize,
}

impl Counted<'_> {
    /// `Err` only when out of evaluations; the inner result is f's.
    fn at(&mut self, x: f64) -> Result<Result<f64, String>, Fail> {
        if self.n >= self.max {
            return Err(Fail::Evals(self.n));
        }
        self.n += 1;
        Ok(match (self.f)(x) {
            Ok(y) if y.is_finite() => Ok(y),
            Ok(_) => Err("not a finite number".into()),
            Err(e) => Err(e),
        })
    }
    /// Like `at`, with an error being the end of the search.
    fn need(&mut self, x: f64) -> Result<f64, Fail> {
        self.at(x)?.map_err(|msg| Fail::Error { x, msg })
    }
}

fn side(y: f64, target: f64) -> bool {
    y > target
}

/// Finds `x` with `f(x)` within `o.tol` of `target`, starting from `x0`.
pub fn solve(f: &mut dyn FnMut(f64) -> Result<f64, String>, x0: f64, target: f64, o: &Opts) -> Result<Root, Fail> {
    let mut fx = Counted { f, n: 0, max: o.max_evals };
    let x0 = if o.whole { x0.round() } else { x0 };
    let x0 = x0.max(o.lo).min(o.hi);
    let (mut at_lo, mut at_hi) = (false, false);
    let y0 = fx.need(x0)?;
    if (y0 - target).abs() <= o.tol {
        return Ok(Root { x: x0, y: y0, evals: fx.n });
    }
    let (mut lo, mut hi, mut ymin, mut ymax) = (x0, x0, y0, y0);
    let mut err = None;
    // the last point tried going up, and going down; None once a side hits an error
    let mut paths = [Some((x0, y0)), Some((x0, y0))];
    for k in 0..=o.doublings {
        for (d, dir) in [1.0, -1.0].into_iter().enumerate() {
            let Some((px, py)) = paths[d] else { continue };
            let mut x = x0 + dir * o.step * 2f64.powi(k as i32);
            if o.whole {
                x = x.round();
            }
            // the last step on a side stops at the bound
            let clamped = x < o.lo || x > o.hi;
            if clamped {
                x = x.max(o.lo).min(o.hi);
                if dir > 0.0 { at_hi = true } else { at_lo = true }
                paths[d] = None;
            }
            if x == px {
                continue;
            }
            match fx.at(x)? {
                Err(msg) => {
                    paths[d] = None;
                    err = err.or(Some((x, msg)));
                }
                Ok(y) => {
                    (lo, hi, ymin, ymax) = (lo.min(x), hi.max(x), ymin.min(y), ymax.max(y));
                    if (y - target).abs() <= o.tol {
                        return Ok(Root { x, y, evals: fx.n });
                    }
                    if side(y, target) != side(py, target) {
                        let (a, b) = if px < x { ((px, py), (x, y)) } else { ((x, y), (px, py)) };
                        return if o.whole { bisect_whole(&mut fx, a, b, target, o.tol) } else { brent(&mut fx, a, b, target, o.tol) };
                    }
                    paths[d] = if clamped { None } else { Some((x, y)) };
                }
            }
        }
        if paths.iter().all(Option::is_none) {
            break;
        }
    }
    Err(Fail::OutOfReach { lo, hi, ymin, ymax, err, at_lo, at_hi })
}

/// Bisection over whole numbers; ends at the whole number nearer the target.
fn bisect_whole(fx: &mut Counted, (mut a, mut ya): (f64, f64), (mut b, mut yb): (f64, f64), t: f64, tol: f64) -> Result<Root, Fail> {
    while b - a > 1.0 {
        let m = ((a + b) / 2.0).floor();
        let ym = fx.need(m)?;
        if (ym - t).abs() <= tol {
            return Ok(Root { x: m, y: ym, evals: fx.n });
        }
        if side(ym, t) == side(ya, t) {
            (a, ya) = (m, ym);
        } else {
            (b, yb) = (m, ym);
        }
    }
    let (x, y) = if (ya - t).abs() <= (yb - t).abs() { (a, ya) } else { (b, yb) };
    Ok(Root { x, y, evals: fx.n })
}

/// Brent's method on a bracket `a < b` whose values straddle `t` (after Numerical Recipes' `zbrent`).
fn brent(fx: &mut Counted, (xa, ya): (f64, f64), (xb, yb): (f64, f64), t: f64, tol: f64) -> Result<Root, Fail> {
    let (mut a, mut b, mut c) = (xa, xb, xb);
    let (mut fa, mut fb) = (ya - t, yb - t);
    let mut fc = fb;
    let (mut d, mut e) = (b - a, b - a);
    loop {
        if (fb > 0.0) == (fc > 0.0) {
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            (a, b, c) = (b, c, b);
            (fa, fb, fc) = (fb, fc, fb);
        }
        if fb.abs() <= tol {
            return Ok(Root { x: b, y: fb + t, evals: fx.n });
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 1e-300;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 {
            // the bracket can't shrink further but the value still misses: it jumps across the target
            let (left, right) = if b < c { (fb, fc) } else { (fc, fb) };
            return Err(Fail::Jump { x: b, left: left + t, right: right + t });
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            let s = fb / fa;
            let (mut p, mut q) = if a == c {
                (2.0 * xm * s, 1.0 - s)
            } else {
                let (q, r) = (fa / fc, fb / fc);
                (s * (2.0 * xm * q * (q - r) - (b - a) * (r - 1.0)), (q - 1.0) * (r - 1.0) * (s - 1.0))
            };
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            if 2.0 * p < (3.0 * xm * q - (tol1 * q).abs()).min((e * q).abs()) {
                e = d;
                d = p / q;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        a = b;
        fa = fb;
        b += if d.abs() > tol1 { d } else { tol1.copysign(xm) };
        fb = fx.need(b)? - t;
    }
}

/// A goal: make element `index` (row-major) of `target`'s value equal `want` (canonical units, within `tol`)
/// by changing the number literal in `input`.
#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub target: CellKey,
    pub index: usize,
    pub want: f64,
    pub tol: f64,
    pub input: CellKey,
    /// Decimal places to write; `None` keeps the literal's own.
    pub decimals: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Solved {
    /// The input cell's new text: the solved number rounded to the decimals, with its unit kept.
    pub text: String,
    /// The number written, as in the text.
    pub value: f64,
    pub evals: usize,
}

impl Engine {
    /// The cell showing element `i` of `anchor`'s value: inside its spill, or the cell itself.
    pub fn element_cell(&self, anchor: CellKey, i: usize) -> CellKey {
        let (dr, dc) = match self.result(anchor) {
            Some(Ok(Value::Num(n))) => match n.shape.as_slice() {
                [_] => (i, 0),
                [_, c] => (i / c.max(&1), i % c.max(&1)),
                _ => (0, 0),
            },
            _ => (0, 0),
        };
        let at = self.wb.pos(anchor).and_then(|(r, c)| self.wb.sheet(anchor.sheet)?.key(r + dr, c + dc));
        at.unwrap_or(anchor)
    }

    /// `growth` for a named cell, else `B4`.
    fn short_name(&self, k: CellKey, home: SheetId) -> String {
        self.name_of(k).map(str::to_string).unwrap_or_else(|| self.wb.cell_label(k, Some(home)))
    }

    fn goal_element(&self, k: CellKey, i: usize) -> Result<f64, String> {
        let label = || self.wb.cell_label(k, Some(k.sheet));
        match self.result(k) {
            Some(Ok(Value::Num(n))) => n.data.get(i).copied().ok_or_else(|| format!("{} has only {} values", label(), n.len())),
            Some(Ok(v)) => Err(format!("{} is a {}", label(), v.type_name())),
            Some(Err(e)) => Err(e.msg.clone()),
            None => Err(format!("{} is empty", label())),
        }
    }

    /// Solves `g` and returns the input's new text, leaving the document exactly as it was:
    /// the caller writes the text (one edit, one undo step). On failure, says why in a sentence.
    pub fn goal_seek(&mut self, g: &Goal) -> Result<Solved, String> {
        let home = Some(g.target.sheet);
        let name = self.short_name(g.input, g.target.sheet);
        let text = self.wb.cell_text(g.input);
        let lit = cell_literal(&text).ok_or_else(|| format!("{name} isn't a number"))?;
        let q: Quant = match self.result(g.target) {
            Some(Ok(Value::Num(n))) => n.q.clone(),
            _ => return Err(format!("{} isn't a number", self.wb.cell_label(g.target, home))),
        };
        let tlabel = self.wb.cell_label(self.element_cell(g.target, g.index), home);
        let decimals = if lit.is_date { 0 } else { g.decimals.unwrap_or(lit.decimals) };
        let range = self.display_range(g.input).unwrap_or_default();
        let (min_text, max_text) = self.range_text(g.input);
        let opts = Opts::for_literal(lit.value, decimals, lit.is_date, g.tol).within(range.min, range.max);
        let orig = self.wb.cell(g.input).cloned();
        // a trial value can make a spill grow the sheet; put the sheet sizes back too
        let axes: Vec<_> = self.wb.sheets.iter().map(|s| (s.id, s.rows.clone(), s.cols.clone())).collect();
        let res = {
            let mut f = |x: f64| {
                let s = if lit.is_date { format_lit(x, 0, true) } else { format!("{x}") };
                self.set_text(g.input, &replace_span(&text, &lit.span, &s));
                self.goal_element(g.target, g.index)
            };
            solve(&mut f, lit.value, g.want, &opts)
        };
        self.apply(Edit::Cells(vec![(g.input, orig)]));
        for (id, rows, cols) in axes {
            if let Some(s) = self.wb.sheet_mut(id) {
                (s.rows, s.cols) = (rows, cols);
            }
        }
        let unit = text[lit.span.end..].trim();
        let unit = unit.strip_prefix('[').and_then(|u| u.strip_suffix(']')).unwrap_or(unit);
        let input = |x: f64| {
            if lit.is_date {
                fmt_date(x)
            } else if unit.is_empty() {
                group_thousands(&fmt_num(x))
            } else {
                format!("{} {unit}", group_thousands(&fmt_num(x)))
            }
        };
        let out = |y: f64| fmt_quantity(y, &q);
        match res {
            Ok(r) => {
                let num = format_lit(r.x, decimals, lit.is_date);
                let value = if lit.is_date { r.x } else { num.parse().unwrap_or(r.x) };
                Ok(Solved { text: replace_span(&text, &lit.span, &num), value, evals: r.evals })
            }
            Err(Fail::OutOfReach { lo, hi, ymin, ymax, err, at_lo, at_hi }) => {
                // name the bounds the search ran into
                let min = min_text.as_deref().filter(|_| at_lo);
                let max = max_text.as_deref().filter(|_| at_hi);
                let within = match (min, max) {
                    (Some(a), Some(b)) => format!(" within {a} ≤ {name} ≤ {b}"),
                    (Some(a), None) => format!(" within {name} ≥ {a}"),
                    (None, Some(b)) => format!(" within {name} ≤ {b}"),
                    (None, None) => String::new(),
                };
                let why = if ymin == ymax {
                    format!("{tlabel} stays at {} for {name} from {} to {}", out(ymin), input(lo), input(hi))
                } else {
                    format!("with {name} from {} to {}, {tlabel} only reaches {} to {}", input(lo), input(hi), out(ymin), out(ymax))
                };
                let stop = err.map(|(x, m)| format!(" (at {name} = {} it's an error: {m})", input(x))).unwrap_or_default();
                Err(format!("out of reach{within}: {why}{stop}"))
            }
            Err(Fail::Jump { x, left, right }) => {
                Err(format!("{tlabel} jumps from {} to {} at {name} = {}, skipping {}", out(left), out(right), input(x), out(g.want)))
            }
            Err(Fail::Error { x, msg }) => Err(format!("at {name} = {}, {tlabel} is an error: {msg}", input(x))),
            Err(Fail::Evals(n)) => Err(format!("no answer after {n} tries")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(x0: f64, decimals: usize) -> Opts {
        Opts::for_literal(x0, decimals, false, 1e-9)
    }
    fn run(f: impl Fn(f64) -> Result<f64, String>, x0: f64, t: f64, o: &Opts) -> Result<Root, Fail> {
        let mut g = |x: f64| f(x);
        solve(&mut g, x0, t, o)
    }

    #[test]
    fn monotone() {
        // what growth rate makes 12 months of growth hit 2×?
        let r = run(|g| Ok((1.0 + g).powi(12)), 0.04, 2.0, &opts(0.04, 2)).unwrap();
        assert!((r.x - (2f64.powf(1.0 / 12.0) - 1.0)).abs() < 1e-9, "{r:?}");
        assert!(r.evals < 30, "{r:?}");
        // decreasing, and a target below the start
        let r = run(|x| Ok(100.0 - 3.0 * x), 10.0, 40.0, &opts(10.0, 1)).unwrap();
        assert!((r.x - 20.0).abs() < 1e-9, "{r:?}");
        // the start is already on target
        assert_eq!(run(|x| Ok(x * 2.0), 3.0, 6.0, &opts(3.0, 1)).unwrap().evals, 1);
    }

    #[test]
    fn crosses_zero_and_starts_at_zero() {
        let r = run(|x| Ok(x * 5.0), 2.0, -5.0, &opts(2.0, 1)).unwrap();
        assert!((r.x + 1.0).abs() < 1e-9, "{r:?}");
        let r = run(|x| Ok(x + 1000.0), 0.0, 1250.0, &opts(0.0, 2)).unwrap();
        assert!((r.x - 250.0).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn non_monotone_finds_the_nearer_root() {
        // (x−3)², from 4 toward 0.25: roots at 2.5 and 3.5; the nearer one wins
        let r = run(|x| Ok((x - 3.0).powi(2)), 4.0, 0.25, &opts(4.0, 1)).unwrap();
        assert!((r.x - 3.5).abs() < 1e-9, "{r:?}");
        // a bump that doubles back: sin over a wide range
        let r = run(|x| Ok(x.sin()), 1.0, 0.5, &opts(1.0, 2)).unwrap();
        assert!((r.x.sin() - 0.5).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn unreachable_target() {
        // x² never goes below 0
        match run(|x| Ok(x * x), 2.0, -1.0, &opts(2.0, 1)) {
            Err(Fail::OutOfReach { lo, hi, ymin, err: None, .. }) => {
                assert_eq!((lo, hi), (-2046.0, 2050.0));
                assert_eq!(ymin, 0.0);
            }
            other => panic!("{other:?}"),
        }
        // a flat function
        assert!(matches!(run(|_| Ok(7.0), 1.0, 8.0, &opts(1.0, 0)), Err(Fail::OutOfReach { ymin: 7.0, ymax: 7.0, .. })));
    }

    #[test]
    fn discontinuity_is_reported_not_guessed() {
        let step = |x: f64| Ok(if x > 1.3 { 10.0 } else { 0.0 });
        match run(step, 1.0, 5.0, &opts(1.0, 2)) {
            Err(Fail::Jump { x, left, right }) => {
                assert!((x - 1.3).abs() < 1e-9, "{x}");
                assert_eq!((left, right), (0.0, 10.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn errors_stop_a_side_or_the_search() {
        // sqrt-like: errors below zero; the target is only reachable going up
        let f = |x: f64| if x < 0.0 { Err("negative".to_string()) } else { Ok(x.sqrt()) };
        let r = run(f, 1.0, 3.0, &opts(1.0, 2)).unwrap();
        assert!((r.x - 9.0).abs() < 1e-9, "{r:?}");
        // unreachable, and the downward side stopped at an error
        match run(f, 1.0, -1.0, &opts(1.0, 2)) {
            Err(Fail::OutOfReach { err: Some((x, m)), .. }) => assert!(x < 0.0 && m == "negative", "{x} {m}"),
            other => panic!("{other:?}"),
        }
        // an error inside the bracket ends the search there
        let holey = |x: f64| if (2.1..2.3).contains(&x) { Err("hole".to_string()) } else { Ok(x) };
        assert!(matches!(run(holey, 1.0, 2.2, &opts(1.0, 2)), Err(Fail::Error { .. })));
        // an error at the start
        assert!(matches!(run(|_| Err("bad".into()), 1.0, 2.0, &opts(1.0, 2)), Err(Fail::Error { x: 1.0, .. })));
        // NaN counts as an error
        assert!(matches!(run(|_| Ok(f64::NAN), 1.0, 2.0, &opts(1.0, 2)), Err(Fail::Error { .. })));
    }

    #[test]
    fn whole_numbers_and_dates() {
        // decimals = 0: only whole numbers are tried, and the nearer one is the answer
        let tried = std::cell::RefCell::new(vec![]);
        let r = run(
            |x| {
                tried.borrow_mut().push(x);
                Ok(x * 10.0)
            },
            24.0,
            123.0,
            &opts(24.0, 0),
        )
        .unwrap();
        assert_eq!(r.x, 12.0);
        assert!(tried.borrow().iter().all(|x| x.fract() == 0.0), "{tried:?}");
        // a date (days): step 1 day, whole days
        let o = Opts::for_literal(20_000.0, 0, true, 1e-9);
        assert_eq!((o.step, o.whole), (1.0, true));
        let r = run(|d| Ok(d - 19_990.0), 20_000.0, 45.0, &o).unwrap();
        assert_eq!(r.x, 20_035.0);
    }

    #[test]
    fn units_scale_the_tolerance_not_the_search() {
        // canonical metres, target 1.5 km = 1500 m, input in km
        let r = run(|km| Ok(km * 1000.0), 1.0, 1500.0, &Opts::for_literal(1.0, 1, false, 0.5)).unwrap();
        assert!((r.x - 1.5).abs() < 1e-3, "{r:?}");
    }

    #[test]
    fn evaluation_cap() {
        let o = Opts { max_evals: 5, ..opts(1.0, 2) };
        assert!(matches!(run(|_| Ok(0.0), 1.0, 1.0e9, &o), Err(Fail::Evals(5))));
    }
}
