//! Goal-seek: change one number cell so a computed value hits a target.
//!
//! The 1-D search first brackets outward from the input's current value
//! (steps that double, alternating up and down, nearest first), then narrows
//! the bracket: Brent's method for inputs with decimals, bisection over whole
//! numbers for dates and inputs written without decimals. It only ever returns
//! a value from a bracket that straddles the target; when there is none it
//! says why (out of reach, a jump, an error) instead of picking the closest.
//!
//! With the derivative at hand (`Engine::sensitivities`), Newton steps go first:
//! a few steps from the current value, each needing one recalc and one
//! sensitivity pass. Any step that can't be taken (a flat or failed derivative,
//! a step out of the input's range, no convergence in a few steps) hands over
//! to the bracketing search from the start, so its answers and its reasons for
//! having none stay what they were. Whole-number inputs only bracket.
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
    /// The input's range: nothing outside `lo..=hi` is tried.
    pub lo: f64,
    pub hi: f64,
}

impl Opts {
    /// The same search kept inside `lo..=hi` (the whole numbers inside, for a whole-number search).
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
    /// Evaluations: of the value, and (Newton) of its derivative, one each.
    pub evals: usize,
}

/// What `solve_newton` asks of the model.
pub enum Ask {
    /// The value at `x` (which becomes the current input).
    Value(f64),
    /// The derivative of the value at the current input, per unit of `x`.
    Slope,
}

/// How many Newton steps to try before bracketing instead.
const NEWTON_STEPS: usize = 6;

/// `solve`, trying Newton steps first: `f` gives values and, asked right after a value, the slope
/// there. See the module notes for when it brackets instead.
pub fn solve_newton(f: &mut dyn FnMut(Ask) -> Result<f64, String>, x0: f64, target: f64, o: &Opts) -> Result<Root, Fail> {
    let mut n = 0;
    if !o.whole {
        let mut x = x0.max(o.lo).min(o.hi);
        for step in 0..=NEWTON_STEPS {
            n += 1;
            let Ok(y) = f(Ask::Value(x)).map(|y| if y.is_finite() { y } else { f64::NAN }) else { break };
            if !y.is_finite() {
                break;
            }
            if (y - target).abs() <= o.tol {
                return Ok(Root { x, y, evals: n });
            }
            if step == NEWTON_STEPS {
                break;
            }
            n += 1;
            let Ok(d) = f(Ask::Slope) else { break };
            let next = x + (target - y) / d;
            if !next.is_finite() || d == 0.0 || next < o.lo || next > o.hi || next == x {
                break;
            }
            x = next;
        }
    }
    let mut value = |x: f64| f(Ask::Value(x));
    let o = Opts { max_evals: o.max_evals.saturating_sub(n).max(1), ..o.clone() };
    match solve(&mut value, x0, target, &o) {
        Ok(r) => Ok(Root { evals: r.evals + n, ..r }),
        Err(Fail::Evals(m)) => Err(Fail::Evals(m + n)),
        Err(e) => Err(e),
    }
}

/// Why there is no answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    /// No two tried inputs straddle the target. Over inputs `lo..=hi` the value stayed in `ymin..=ymax`;
    /// `err` is the first input where the value was an error (the search stops there on that side).
    /// `bounded`: the search stopped at the range's (lower, upper) end.
    OutOfReach { lo: f64, hi: f64, ymin: f64, ymax: f64, err: Option<(f64, String)>, bounded: (bool, bool) },
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
    // a start outside the range (a typed value that's an error) starts at its end
    let x0 = x0.max(o.lo).min(o.hi);
    let y0 = fx.need(x0)?;
    if (y0 - target).abs() <= o.tol {
        return Ok(Root { x: x0, y: y0, evals: fx.n });
    }
    let (mut lo, mut hi, mut ymin, mut ymax) = (x0, x0, y0, y0);
    let mut err = None;
    let mut bounded = [false, false];
    // the last point tried going up, and going down; None once a side hits an error or the range's end
    let mut paths = [Some((x0, y0)), Some((x0, y0))];
    for k in 0..=o.doublings {
        for (d, dir) in [1.0, -1.0].into_iter().enumerate() {
            let Some((px, py)) = paths[d] else { continue };
            let mut x = x0 + dir * o.step * 2f64.powi(k as i32);
            if o.whole {
                x = x.round();
            }
            // the last step on a side stops at the range's end
            let end = if d == 0 { o.hi } else { o.lo };
            if (x - end) * dir >= 0.0 {
                x = end;
                bounded[d] = true;
            }
            if x == px {
                if bounded[d] {
                    paths[d] = None;
                }
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
                    paths[d] = Some((x, y));
                }
            }
        }
        if paths.iter().all(Option::is_none) {
            break;
        }
    }
    Err(Fail::OutOfReach { lo, hi, ymin, ymax, err, bounded: (bounded[1], bounded[0]) })
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
    /// Try Newton steps (with the exact derivative) before bracketing. On by default; off is the
    /// bracketing search alone.
    pub newton: bool,
}

impl Goal {
    pub fn new(target: CellKey, index: usize, want: f64, tol: f64, input: CellKey, decimals: Option<usize>) -> Goal {
        Goal { target, index, want, tol, input, decimals, newton: true }
    }
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
        // (outside its range, say)
        if let Some(Err(e)) = self.result(g.input) {
            return Err(format!("{name} has an error: {}", e.msg));
        }
        let q: Quant = match self.result(g.target) {
            Some(Ok(Value::Num(n))) => n.q.clone(),
            _ => return Err(format!("{} isn't a number", self.wb.cell_label(g.target, home))),
        };
        let tlabel = self.wb.cell_label(self.element_cell(g.target, g.index), home);
        let decimals = if lit.is_date { 0 } else { g.decimals.unwrap_or(lit.decimals) };
        let range = self.input_range(g.input).unwrap_or_default();
        let opts = Opts::for_literal(lit.value, decimals, lit.is_date, g.tol).within(range.min.as_ref().map(|m| m.0), range.max.as_ref().map(|m| m.0));
        // trials only write the input cell (a spill past the stored rows reaches virtual ones)
        let orig = self.wb.cell(g.input).cloned();
        // the derivative per unit of the literal as written (canonical = literal × factor + offset)
        let factor = match self.result(g.input) {
            Some(Ok(Value::Num(n))) => n.q.disp.factor,
            _ => 1.0,
        };
        let res = {
            let mut f = |a: Ask| match a {
                Ask::Value(x) => {
                    let s = if lit.is_date { format_lit(x, 0, true) } else { format!("{x}") };
                    self.set_text(g.input, &replace_span(&text, &lit.span, &s));
                    self.goal_element(g.target, g.index)
                }
                Ask::Slope => {
                    let d = self.sensitivities(g.input, &[g.target])?.remove(0);
                    d.data.get(g.index).map(|d| d * factor).ok_or_else(|| "no such element".to_string())
                }
            };
            if g.newton {
                solve_newton(&mut f, lit.value, g.want, &opts)
            } else {
                let mut value = |x: f64| f(Ask::Value(x));
                solve(&mut value, lit.value, g.want, &opts)
            }
        };
        self.apply(Edit::Cells(vec![(g.input, orig)]));
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
                // rounding to the decimals stays inside the range
                let num = range.format(r.x, decimals, lit.is_date);
                let value = if lit.is_date { r.x } else { num.parse().unwrap_or(r.x) };
                Ok(Solved { text: replace_span(&text, &lit.span, &num), value, evals: r.evals })
            }
            Err(Fail::OutOfReach { lo, hi, ymin, ymax, err, bounded }) => {
                let within = match (range.min.as_ref().filter(|_| bounded.0), range.max.as_ref().filter(|_| bounded.1)) {
                    (Some((_, a)), Some((_, b))) => format!(" within {a} ≤ {name} ≤ {b}"),
                    (Some((_, a)), None) => format!(" within {name} ≥ {a}"),
                    (None, Some((_, b))) => format!(" within {name} ≤ {b}"),
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

/// A 2D goal: put a chart point, element `x.1` of cell `x.0` across and element `y.1` of `y.0` up,
/// at `want` (canonical units, each within its `tol`) by changing the number literals in both `inputs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Goal2 {
    pub x: (CellKey, usize),
    pub y: (CellKey, usize),
    pub want: [f64; 2],
    pub tol: [f64; 2],
    pub inputs: [CellKey; 2],
    /// Decimal places to write each input with; `None` keeps the literal's own.
    pub decimals: [Option<usize>; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Solved2 {
    /// Each input's new text, rounded to its decimals, with its unit kept.
    pub texts: [String; 2],
    pub evals: usize,
}

/// Newton steps for a 2D goal before giving up.
const NEWTON_2D_STEPS: usize = 24;

impl Engine {
    /// Solves `g` by Newton's method on the 2×2 Jacobian (one sensitivity pass per input), with damped
    /// steps that stay inside both inputs' ranges, starting from their current values, so the answer is
    /// the one continuously connected to where they are. Leaves the document exactly as it was; on
    /// failure says why (inputs that don't move the point independently, out of reach, no
    /// convergence) and never settles for the closest point.
    pub fn goal_seek_2d(&mut self, g: &Goal2) -> Result<Solved2, String> {
        let home = g.y.0.sheet;
        let [a, b] = g.inputs;
        if a == b {
            return Err("a 2D goal needs two different inputs".into());
        }
        let names = [self.short_name(a, home), self.short_name(b, home)];
        let texts = [self.wb.cell_text(a), self.wb.cell_text(b)];
        let mut lits = Vec::new();
        for i in 0..2 {
            let lit = cell_literal(&texts[i]).ok_or_else(|| format!("{} isn't a number", names[i]))?;
            if let Some(Err(e)) = self.result(g.inputs[i]) {
                return Err(format!("{} has an error: {}", names[i], e.msg));
            }
            lits.push(lit);
        }
        for (k, _) in [g.x, g.y] {
            if !matches!(self.result(k), Some(Ok(Value::Num(_)))) {
                return Err(format!("{} isn't a number", self.wb.cell_label(k, Some(home))));
            }
        }
        let ranges = [self.input_range(a).unwrap_or_default(), self.input_range(b).unwrap_or_default()];
        let bounds: Vec<(f64, f64)> = ranges.iter().map(|r| (r.min.as_ref().map_or(f64::NEG_INFINITY, |m| m.0), r.max.as_ref().map_or(f64::INFINITY, |m| m.0))).collect();
        // derivatives per unit of each literal as written (canonical = literal × factor + offset)
        let factors: Vec<f64> = g
            .inputs
            .iter()
            .map(|k| match self.result(*k) {
                Some(Ok(Value::Num(n))) => n.q.disp.factor,
                _ => 1.0,
            })
            .collect();
        let point = self.wb.cell_label(self.element_cell(g.y.0, g.y.1), Some(home));
        let origs = [self.wb.cell(a).cloned(), self.wb.cell(b).cloned()];
        let mut evals = 0;
        let res = (|| -> Result<[f64; 2], String> {
            let at = |u: [f64; 2]| format!("{} = {}, {} = {}", names[0], fmt_num(u[0]), names[1], fmt_num(u[1]));
            let eval = |e: &mut Engine, u: [f64; 2], evals: &mut usize| -> Result<[f64; 2], String> {
                *evals += 1;
                let mut cells = Vec::new();
                for i in 0..2 {
                    let s = if lits[i].is_date { format_lit(u[i], 0, true) } else { format!("{}", u[i]) };
                    let text = replace_span(&texts[i], &lits[i].span, &s);
                    cells.push((g.inputs[i], Some(crate::model::Cell::new(e.wb.parse_text(&text, g.inputs[i].sheet)))));
                }
                e.apply(Edit::Cells(cells));
                let f = [e.goal_element(g.x.0, g.x.1), e.goal_element(g.y.0, g.y.1)];
                match f {
                    [Ok(x), Ok(y)] if x.is_finite() && y.is_finite() => Ok([x, y]),
                    [Err(m), _] | [_, Err(m)] => Err(format!("at {}, {point} is an error: {m}", at(u))),
                    _ => Err(format!("at {}, {point} isn't a finite number", at(u))),
                }
            };
            // misses scaled by the tolerances, so x and y count alike whatever their units
            let merit = |f: [f64; 2]| ((g.want[0] - f[0]) / g.tol[0]).powi(2) + ((g.want[1] - f[1]) / g.tol[1]).powi(2);
            let mut u = [lits[0].value.clamp(bounds[0].0, bounds[0].1), lits[1].value.clamp(bounds[1].0, bounds[1].1)];
            let mut f = eval(self, u, &mut evals)?;
            for _ in 0..NEWTON_2D_STEPS {
                let r = [g.want[0] - f[0], g.want[1] - f[1]];
                if r[0].abs() <= g.tol[0] && r[1].abs() <= g.tol[1] {
                    return Ok(u);
                }
                // the Jacobian: column i is how (x, y) move per unit of input i
                let mut jac = [[0.0; 2]; 2];
                for i in 0..2 {
                    evals += 1;
                    let d = self.sensitivities(g.inputs[i], &[g.x.0, g.y.0]).map_err(|m| format!("can't reach: {m}"))?;
                    jac[0][i] = d[0].data.get(g.x.1).copied().unwrap_or(f64::NAN) * factors[i];
                    jac[1][i] = d[1].data.get(g.y.1).copied().unwrap_or(f64::NAN) * factors[i];
                }
                // scaled by the tolerances, so "independent" doesn't depend on units
                let sc = [[jac[0][0] / g.tol[0], jac[0][1] / g.tol[0]], [jac[1][0] / g.tol[1], jac[1][1] / g.tol[1]]];
                let col = |i: usize| sc[0][i].hypot(sc[1][i]);
                for i in 0..2 {
                    if col(i) == 0.0 || !col(i).is_finite() {
                        return Err(format!("can't reach: {} doesn't move this point here, so {} and {} can't place it in 2D", names[i], names[0], names[1]));
                    }
                }
                let det = sc[0][0] * sc[1][1] - sc[0][1] * sc[1][0];
                if det.abs() <= 1e-9 * col(0) * col(1) {
                    return Err(format!("can't reach: {} and {} don't move this point independently here", names[0], names[1]));
                }
                let det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0];
                let step = [(jac[1][1] * r[0] - jac[0][1] * r[1]) / det, (jac[0][0] * r[1] - jac[1][0] * r[0]) / det];
                // stay inside both ranges: the longest part of the step that does
                let mut lam: f64 = 1.0;
                let mut blocked = None;
                for i in 0..2 {
                    let to = u[i] + step[i];
                    let edge = if to < bounds[i].0 { Some(bounds[i].0) } else if to > bounds[i].1 { Some(bounds[i].1) } else { None };
                    if let Some(e) = edge {
                        let l = (e - u[i]) / step[i];
                        if l < lam {
                            lam = l.max(0.0);
                            blocked = Some(i);
                        }
                    }
                }
                if lam <= 0.0 {
                    let i = blocked.unwrap_or(0);
                    let end = if step[i] < 0.0 { ranges[i].min.as_ref() } else { ranges[i].max.as_ref() };
                    let end = end.map(|e| format!(" ({} {})", if step[i] < 0.0 { "min" } else { "max" }, e.1)).unwrap_or_default();
                    return Err(format!("out of reach: {} is at the end of its range{end}, and the point needs it further", names[i]));
                }
                // damped: halve the step until the miss shrinks
                let m0 = merit(f);
                let mut next = None;
                for _ in 0..10 {
                    let v = [u[0] + lam * step[0], u[1] + lam * step[1]];
                    match eval(self, v, &mut evals) {
                        Ok(fv) if merit(fv) < m0 => {
                            next = Some((v, fv));
                            break;
                        }
                        // an error part-way: a shorter step may stay clear of it
                        _ => lam /= 2.0,
                    }
                }
                let Some((v, fv)) = next else {
                    return Err(format!("can't reach: no {} and {} near their current values put the point there", names[0], names[1]));
                };
                (u, f) = (v, fv);
            }
            Err(format!("can't reach: no answer after {NEWTON_2D_STEPS} steps from the current {} and {}", names[0], names[1]))
        })();
        self.apply(Edit::Cells(vec![(a, origs[0].clone()), (b, origs[1].clone())]));
        let u = res?;
        let mut out = [String::new(), String::new()];
        for i in 0..2 {
            let decimals = if lits[i].is_date { 0 } else { g.decimals[i].unwrap_or(lits[i].decimals) };
            let num = ranges[i].format(u[i], decimals, lits[i].is_date);
            out[i] = replace_span(&texts[i], &lits[i].span, &num);
        }
        Ok(Solved2 { texts: out, evals })
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
    fn nothing_outside_the_range_is_tried() {
        let tried = std::cell::RefCell::new(vec![]);
        let f = |x: f64| {
            tried.borrow_mut().push(x);
            Ok(x * x)
        };
        // from -0.5, the nearer root is -2; x ≥ -1 leaves 2
        let r = run(f, -0.5, 4.0, &opts(-0.5, 2).within(Some(-1.0), None)).unwrap();
        assert!((r.x - 2.0).abs() < 1e-9, "{r:?}");
        assert!(tried.borrow().iter().all(|x| *x >= -1.0), "{tried:?}");
        // out of reach, and which end stopped it
        tried.borrow_mut().clear();
        match run(f, 0.5, 9.0, &opts(0.5, 2).within(Some(0.0), Some(2.0))) {
            Err(Fail::OutOfReach { lo, hi, bounded, .. }) => assert_eq!((lo, hi, bounded), (0.0, 2.0, (true, true))),
            other => panic!("{other:?}"),
        }
        assert!(tried.borrow().iter().all(|x| (0.0..=2.0).contains(x)), "{tried:?}");
        // whole numbers: the whole numbers inside
        let o = opts(3.0, 0).within(Some(0.5), Some(7.5));
        assert_eq!((o.lo, o.hi), (1.0, 7.0));
    }

    /// `solve_newton` on `f` with derivative `df`, counting value and slope calls.
    fn newton(f: impl Fn(f64) -> f64, df: impl Fn(f64) -> Result<f64, String>, x0: f64, t: f64, o: &Opts) -> Result<Root, Fail> {
        let mut x = x0;
        let mut g = |a: Ask| match a {
            Ask::Value(v) => {
                x = v;
                Ok(f(v))
            }
            Ask::Slope => df(x),
        };
        solve_newton(&mut g, x0, t, o)
    }

    #[test]
    fn newton_steps_then_brackets() {
        // monotone: a few steps, fewer evaluations than bracketing
        let r = newton(|g| (1.0 + g).powi(12), |g| Ok(12.0 * (1.0 + g).powi(11)), 0.04, 2.0, &opts(0.04, 2)).unwrap();
        let b = run(|g| Ok((1.0 + g).powi(12)), 0.04, 2.0, &opts(0.04, 2)).unwrap();
        assert!((r.x - b.x).abs() < 1e-9 && r.evals < b.evals, "{r:?} vs {b:?}");
        // the nearer root of a parabola, as bracketing finds
        let r = newton(|x| (x - 3.0).powi(2), |x| Ok(2.0 * (x - 3.0)), 4.0, 0.25, &opts(4.0, 1)).unwrap();
        assert!((r.x - 3.5).abs() < 1e-9, "{r:?}");
        // a flat start (the bottom of the parabola): bracketing from the start
        let r = newton(|x| (x - 3.0).powi(2), |x| Ok(2.0 * (x - 3.0)), 3.0, 1.0, &opts(3.0, 1)).unwrap();
        assert!(((r.x - 3.0).abs() - 1.0).abs() < 1e-9, "{r:?}");
        // no slope at all (a pass that can't be done): bracketing
        let r = newton(|x| x * 2.0, |_| Err("no".into()), 1.0, 5.0, &opts(1.0, 2)).unwrap();
        assert!((r.x - 2.5).abs() < 1e-9, "{r:?}");
        // a step out of the range: bracketing, which then says it's out of reach
        let o = opts(1.0, 2).within(Some(0.0), Some(2.0));
        assert!(matches!(newton(|x| x, |_| Ok(1.0), 1.0, 5.0, &o), Err(Fail::OutOfReach { hi: 2.0, .. })));
        // a jump: Newton can't land on it, and bracketing reports it as before
        let step = |x: f64| if x > 1.3 { 10.0 } else { 0.0 };
        assert!(matches!(newton(step, |_| Ok(0.0), 1.0, 5.0, &opts(1.0, 2)), Err(Fail::Jump { .. })));
        // whole numbers only bracket
        let r = newton(|x| x * 10.0, |_| Ok(10.0), 24.0, 123.0, &opts(24.0, 0)).unwrap();
        assert_eq!(r.x, 12.0);
    }

    #[test]
    fn evaluation_cap() {
        let o = Opts { max_evals: 5, ..opts(1.0, 2) };
        assert!(matches!(run(|_| Ok(0.0), 1.0, 1.0e9, &o), Err(Fail::Evals(5))));
    }
}
