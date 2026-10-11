//! Sensitivities (forward-mode derivatives): every word's tangent against a central finite
//! difference, on random inputs, through the real engine.

use wbs_core::a1;
use wbs_core::engine::{Engine, Shown};
use wbs_core::ids::CellKey;
use wbs_core::parse::BUILTINS;
use wbs_core::stdlib::default_workbook;
use wbs_core::value::{fmt_quantity, Value};

fn key(e: &Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheets[0].key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, r: &str, text: &str) {
    let k = key(e, r);
    e.set_text(k, text);
}

fn show(e: &Engine, r: &str) -> String {
    match e.shown(key(e, r)) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

/// A cell's numbers, canonical.
fn nums(e: &Engine, k: CellKey) -> Vec<f64> {
    match e.result(k) {
        Some(Ok(Value::Num(n))) => n.data.to_vec(),
        other => panic!("{}: {other:?}", e.wb.cell_label(k, None)),
    }
}

/// A small deterministic generator, so a failure reproduces.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

/// Inputs `a` (A1) and `b` (A2), unit suffixes optional, and words that read them.
fn model(a: f64, b: f64, ua: &str, ub: &str) -> Engine {
    let mut e = Engine::new(default_workbook());
    set(&mut e, "A1", &format!("{a} {ua}"));
    set(&mut e, "A2", &format!("{b} {ub}"));
    e.set_name("a", Some(key(&e, "A1")), true).unwrap();
    e.set_name("b", Some(key(&e, "A2")), true).unwrap();
    set(&mut e, "A4", ": grow b * ;");
    set(&mut e, "A5", ": wobble { t x v } v  x b * neg  v a * - ;");
    set(&mut e, "A6", ": both + ;");
    e
}

/// The tangent of `out` with respect to `input` against a central difference in the input's literal.
fn check(e: &mut Engine, input: &str, out: &str, prog: &str) {
    let (ik, ok) = (key(e, input), key(e, out));
    let d = match e.sensitivities(ik, &[ok]) {
        Ok(mut v) => v.remove(0),
        Err(m) => panic!("`{prog}` d/d{input}: {m}"),
    };
    let text = e.wb.cell_text(ik);
    let t = text.trim();
    let (x, unit) = t.split_once(' ').unwrap_or((t, ""));
    let x: f64 = x.parse().unwrap_or_else(|_| panic!("{text:?}"));
    let unit = unit.to_string();
    let h = 1e-6 * x.abs().max(1.0);
    let at = |e: &mut Engine, x: f64| {
        set(e, input, &format!("{x} {unit}"));
        (nums(e, ik)[0], nums(e, ok))
    };
    let (xp, fp) = at(e, x + h);
    let (xm, fm) = at(e, x - h);
    set(e, input, &text);
    assert_eq!(d.data.len(), fp.len(), "`{prog}`: shape");
    for i in 0..fp.len() {
        let fd = (fp[i] - fm[i]) / (xp - xm);
        let ad = d.data[i];
        let tol = 1e-5 * fd.abs().max(ad.abs()).max(1.0);
        assert!((ad - fd).abs() <= tol, "`{prog}` d/d{input} [{i}] at a={} b={}: tangent {ad}, finite difference {fd}", show(e, "A1"), show(e, "A2"));
    }
}

/// Each program, with both inputs drawn from `ra`/`rb`, differentiated by each input.
fn run(words: &[&str], cases: &[(&str, (f64, f64), (f64, f64))], units: (&str, &str), covered: &mut Vec<String>) {
    let mut rng = Lcg(0x5eed);
    for (prog, ra, rb) in cases {
        for _ in 0..8 {
            let (a, b) = (rng.between(ra.0, ra.1), rng.between(rb.0, rb.1));
            let mut e = model(a, b, units.0, units.1);
            set(&mut e, "C1", &format!("={prog}"));
            assert!(!show(&e, "C1").starts_with("ERR"), "`{prog}`: {}", show(&e, "C1"));
            check(&mut e, "A1", "C1", prog);
            check(&mut e, "A2", "C1", prog);
        }
    }
    covered.extend(words.iter().map(|w| w.to_string()));
}

#[test]
fn every_word_matches_a_finite_difference() {
    let p = (0.5, 3.0);
    let any = (-3.0, 3.0);
    let mut covered = Vec::new();
    run(
        &["+", "-", "*", "/", "^", "neg", "abs", "sqrt", "exp", "log", "log10", "log2", "sin", "cos", "tan", "min", "max", "pi"],
        &[
            ("a b +", any, any),
            ("a b -", any, any),
            ("a b *", any, any),
            ("a b /", any, p),
            ("a b ^", p, any),
            ("a 3 ^ b *", any, any),
            ("a neg b *", any, any),
            ("a abs b *", any, any),
            ("a sqrt b *", p, any),
            ("a exp b *", any, any),
            ("a log b *", p, any),
            ("a log10 b log2 +", p, p),
            ("a sin b cos *", any, any),
            ("a tan b *", (-1.2, 1.2), any),
            ("a b min a b max *", any, any),
            ("pi a * b +", any, any),
        ],
        ("", ""),
        &mut covered,
    );
    // flat words: zero, away from their jumps
    run(
        &["floor", "ceil", "round", "<", ">", "<=", ">=", "=", "!=", "not", "len", "range"],
        &[
            ("a floor a ceil + a round + b *", (0.1, 0.4), any),
            ("a b < a b > + a b <= + a b >= + a b = + a b != + a not + b +", any, any),
            ("4 range a * len b *", any, any),
        ],
        ("", ""),
        &mut covered,
    );
    // stack and array words move tangents with the values
    run(
        &["dup", "drop", "swap", "over", "rot", "if", "sum", "mean", "rev", "join", "first", "last", "pick", "transpose", "reshape", "couple"],
        &[
            ("a dup * b drop b swap -", any, any),
            ("a b over * + b 2 rot * +", any, any),
            ("a b > a b * a b + if", any, any),
            ("4 range a * b - 0 > 4 range a * 4 range b * if", any, any),
            ("4 range a * b + sum 4 range b * a + mean *", any, any),
            ("4 range a * rev b * a join b join", any, any),
            ("a b join a b * join dup first swap last *", any, any),
            ("a b join b a * join 2 pick", any, any),
            ("a b couple a b * b couple couple transpose", any, any),
            ("6 range a * b + 2 3 join reshape", any, any),
        ],
        ("", ""),
        &mut covered,
    );
    // reductions and scans through any word
    run(
        &[],
        &[("4 range a * b + /+", any, any), ("4 range a * b + \\*", any, any), ("4 range a * b + /max", any, any), ("4 range a * b + /both", any, any)],
        ("", ""),
        &mut covered,
    );
    // words as values: iterate's step and rk4's derivative read the inputs
    run(
        &["iterate", "rk4"],
        &[
            ("6 a 'grow 1 iterate", any, (0.5, 1.5)),
            ("0 a 0.1 * 20 1 0 'wobble 2 rk4 drop", (0.2, 1.0), (1.0, 4.0)),
            ("a b 30 1 0 'wobble 2 rk4 swap drop", (-1.0, 1.0), (0.05, 0.2)),
        ],
        ("", ""),
        &mut covered,
    );
    let charts = ["line", "scatter", "path", "bar", "layer", "title", "xlabel", "ylabel", "size", "sensitivity"];
    let missing: Vec<&str> = BUILTINS.iter().map(|b| b.0).filter(|n| !covered.iter().any(|c| c == n) && !charts.contains(n)).collect();
    assert!(missing.is_empty(), "words without a derivative test: {missing:?}");
}

#[test]
fn units_scale_derivatives_and_type_them() {
    let any = (-3.0, 3.0);
    let mut covered = Vec::new();
    // canonical per canonical, whatever the display units
    run(
        &[],
        &[
            ("a b +", any, any),
            ("a b * to[mi*km]", any, any),
            ("a a * sqrt b +", (0.5, 3.0), any),
        ],
        ("[km]", "[m]"),
        &mut covered,
    );
    // absolute units: a difference's derivative, the offset gone
    run(&[], &[("a [°C] b [Δ°C] + to[°F]", any, any), ("a [°C] b [°C] - b *", any, any)], ("", ""), &mut covered);
    // the result is a quantity in output units per input unit
    let mut e = model(2.0, 3.0, "[km]", "[h]");
    set(&mut e, "C1", "=a b /");
    let d = e.sensitivities(key(&e, "A2"), &[key(&e, "C1")]).unwrap().remove(0);
    // d(a/b)/db = -a/b² = -2/9 km/h²
    assert_eq!(fmt_quantity(d.data[0], &d.q), "-0.22222222 km/h^2");
    assert_eq!(d.q.dim.to_string(), "length/time^2");
    // an output that doesn't depend on the input: zero, in its units
    set(&mut e, "C2", "=b 2 *");
    let d = e.sensitivities(key(&e, "A1"), &[key(&e, "C2")]).unwrap().remove(0);
    assert_eq!(fmt_quantity(d.data[0], &d.q), "0 h/km");
    // percent inputs: per percentage point
    set(&mut e, "D1", "4 [%]");
    set(&mut e, "D2", "=100 [USD] D1 *");
    let d = e.sensitivities(key(&e, "D1"), &[key(&e, "D2")]).unwrap().remove(0);
    assert_eq!(fmt_quantity(d.data[0], &d.q), "1 USD/%");
}

#[test]
fn a_pass_leaves_the_document_alone() {
    let mut e = model(2.0, 3.0, "", "");
    set(&mut e, "C1", "=a b * 4 range *");
    set(&mut e, "C6", "=C2 a +");
    let before = serde_json::to_string(&e.wb).unwrap();
    let shown: Vec<String> = (1..=8).map(|r| show(&e, &format!("C{r}"))).collect();
    // a reference into a spill is that element's derivative
    let d = e.sensitivities(key(&e, "A1"), &[key(&e, "C1"), key(&e, "C6"), key(&e, "C3")]).unwrap();
    assert_eq!(d[0].data.to_vec(), vec![0.0, 3.0, 6.0, 9.0]);
    assert_eq!(d[1].data.to_vec(), vec![4.0]);
    assert_eq!(d[2].data.to_vec(), vec![0.0, 3.0, 6.0, 9.0]);
    assert_eq!(serde_json::to_string(&e.wb).unwrap(), before);
    assert_eq!((1..=8).map(|r| show(&e, &format!("C{r}"))).collect::<Vec<_>>(), shown);
    // the same pass twice: bit for bit
    let again = e.sensitivities(key(&e, "A1"), &[key(&e, "C1"), key(&e, "C6")]).unwrap();
    assert_eq!(again[0].data.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), d[0].data.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
    // a plain recalc after it has no tangents anywhere
    set(&mut e, "A2", "4");
    match e.result(key(&e, "C1")) {
        Some(Ok(Value::Num(n))) => assert!(n.tan.is_none()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn what_a_pass_cant_do_it_says() {
    let mut e = model(2.0, 3.0, "", "");
    set(&mut e, "C1", "=a b *");
    // not a number cell
    assert_eq!(e.sensitivities(key(&e, "C1"), &[key(&e, "C1")]).unwrap_err(), "Sheet1!C1 isn't a number cell");
    // through a unit definition (an exchange rate as an input)
    set(&mut e, "E1", "[zorkmid] = a [USD]");
    set(&mut e, "C2", "=3 [zorkmid]");
    let err = e.sensitivities(key(&e, "A1"), &[key(&e, "C2")]).unwrap_err();
    assert!(err.contains("unit definition"), "{err}");
    // an error downstream
    set(&mut e, "C3", "=a \"x\" +");
    assert!(e.sensitivities(key(&e, "A1"), &[key(&e, "C3")]).unwrap_err().contains("has an error"));
}

/// `out`'s derivative by `input` (a named cell), against a central difference.
fn golden(e: &mut Engine, input: &str, out: &str) -> f64 {
    let k = e.wb.names[input].cell;
    let at = e.wb.cell_label(k, None);
    let at = at.strip_prefix("Sheet1!").unwrap_or(&at).to_string();
    let ok = key(e, out);
    let d = e.sensitivities(k, &[ok]).unwrap().remove(0);
    let text = e.wb.cell_text(k);
    let (x, unit) = text.trim().split_once(' ').unwrap_or((text.trim(), ""));
    let (x, unit): (f64, String) = (x.parse().unwrap(), unit.into());
    let h = 1e-5 * x.abs().max(1e-3);
    let mut f = |x: f64| {
        set(e, &at, &format!("{x} {unit}"));
        (nums(e, k)[0], *nums(e, ok).last().unwrap())
    };
    let ((xp, fp), (xm, fm)) = (f(x + h), f(x - h));
    set(e, &at, &text);
    let fd = (fp - fm) / (xp - xm);
    let ad = *d.data.last().unwrap();
    assert!((ad - fd).abs() <= 1e-4 * fd.abs().max(ad.abs()).max(1e-9), "d {out}/d {input}: tangent {ad}, finite difference {fd}");
    ad
}

/// The spring from the help page: position over 40 steps of rk4.
fn spring() -> Engine {
    let mut e = Engine::new(default_workbook());
    for (at, text, name) in [
        ("B1", "4 [N/m]", "spring"),
        ("B2", "1.5 [kg]", "mass"),
        ("B3", "0.5 [N*s/m]", "damping"),
        ("B4", "0.25 [s]", "timestep"),
        ("B5", "1 [m]", "x0"),
        ("B6", "0.3 [m/s]", "v0"),
    ] {
        set(&mut e, at, text);
        e.set_name(name, Some(key(&e, at)), true).unwrap();
    }
    set(&mut e, "A9", ": msd { t x v } v   spring mass / x * neg   damping mass / v * - ;");
    set(&mut e, "C1", "=0 [s] timestep 40 x0 v0 'msd 2 rk4 drop");
    e
}

#[test]
fn spring_sensitivities_match_finite_differences() {
    let mut e = spring();
    for input in ["spring", "mass", "damping", "timestep", "x0", "v0"] {
        golden(&mut e, input, "C1");
    }
    // typed: metres of final position per N·s/m of damping
    let d = e.sensitivities(e.wb.names["damping"].cell, &[key(&e, "C1")]).unwrap().remove(0);
    assert_eq!(d.q.dim.to_string(), "length*time/mass");
}

const HOURS: usize = 24 * 14;

/// A home solar and battery system: an hourly battery `iterate` over two weeks, scaled to a year of
/// savings, and the years until the system pays for itself.
fn solar() -> Engine {
    let mut e = Engine::new(default_workbook());
    for (at, text, name) in [
        ("B1", "5 [kW]", "solarSize"),
        ("B2", "1200 [USD/kW]", "solarCost"),
        ("B3", "5.5 [kW*h]", "batteryCapacity"),
        ("B4", "3 [kW]", "batteryPower"),
        ("B5", "0.9", "roundtrip"),
        ("B6", "400 [USD/(kW*h)]", "batteryCost"),
        ("B7", "0.3 [USD/(kW*h)]", "price"),
    ] {
        set(&mut e, at, text);
        e.set_name(name, Some(key(&e, at)), true).unwrap();
    }
    set(&mut e, "A1", "1 [h]");
    e.set_name("dt", Some(key(&e, "A1")), false).unwrap();
    set(&mut e, "A2", ": chg { n soc } n roundtrip * batteryCapacity soc - min batteryPower dt * min ;");
    set(&mut e, "A3", ": dis { n soc } n neg soc min batteryPower dt * min ;");
    set(
        &mut e,
        "A4",
        ": step { i soc cum } i 1 +   net i pick 0 [kW*h] > soc net i pick soc chg + soc net i pick soc dis - if   cum net i pick 0 [kW*h] > 0 [kW*h] net i pick soc dis if + ;",
    );
    // sun per kW of panels, and the house's use
    for h in 0..HOURS {
        let sun = (((h % 24) as f64 - 6.0) / 12.0 * std::f64::consts::PI).sin().max(0.0) * (0.7 + 0.3 * ((h / 24 * 5 % 7) as f64 / 7.0));
        let used = 0.6 + 0.4 * ((h * 7 % 11) as f64 / 11.0);
        set(&mut e, &format!("C{}", h + 1), &format!("{sun:.3}"));
        set(&mut e, &format!("D{}", h + 1), &format!("{used:.3} [kW*h]"));
    }
    set(&mut e, "E1", &format!("=C1:C{HOURS} solarSize * dt *"));
    e.set_name("gen", Some(key(&e, "E1")), false).unwrap();
    set(&mut e, "F1", &format!("=E1:E{HOURS} D1:D{HOURS} -"));
    e.set_name("net", Some(key(&e, "F1")), false).unwrap();
    set(&mut e, "G1", "=net len 0 0 [kW*h] 0 [kW*h] 'step 3 iterate last swap drop swap drop");
    set(&mut e, "G2", &format!("=E1:E{HOURS} D1:D{HOURS} min sum G1 + price * {} 1 [yr] / *", 8760.0 / HOURS as f64));
    set(&mut e, "G3", "=solarSize solarCost * batteryCapacity batteryCost * + G2 /");
    e
}

#[test]
fn solar_breakeven_sensitivities_match_finite_differences() {
    let mut e = solar();
    assert!(show(&e, "G3").ends_with(" yr"), "{}", show(&e, "G3"));
    for input in ["solarSize", "solarCost", "batteryCapacity", "batteryPower", "roundtrip", "batteryCost", "price"] {
        golden(&mut e, input, "G3");
    }
    // years of payback per USD/kW of panel price
    let d = e.sensitivities(e.wb.names["solarCost"].cell, &[key(&e, "G3")]).unwrap().remove(0);
    assert_eq!(d.q.dim.to_string(), "length^2*mass/(currency*time^2)");
    assert!(fmt_quantity(d.data[0], &d.q).ends_with(" yr*kW/USD"), "{}", fmt_quantity(d.data[0], &d.q));
}

#[test]
fn newton_goal_seek_takes_fewer_evaluations() {
    use wbs_core::solve::Goal;
    let mut e = solar();
    let g3 = key(&e, "G3");
    let yr = |e: &Engine| nums(e, g3)[0];
    let now = yr(&e);
    let mut report = Vec::new();
    for (input, want) in [("solarCost", now * 0.8), ("solarSize", now * 0.95), ("batteryCapacity", now * 1.05), ("price", now * 0.7)] {
        let k = e.wb.names[input].cell;
        let goal = Goal::new(g3, 0, want, want * 1e-6, k, Some(6));
        let n = e.goal_seek(&goal).unwrap_or_else(|m| panic!("{input}: {m}"));
        let b = e.goal_seek(&Goal { newton: false, ..goal.clone() }).unwrap_or_else(|m| panic!("{input}: {m}"));
        report.push(format!("{input}: newton {} evals ({}), bracketing {} ({})", n.evals, n.text, b.evals, b.text));
        assert!(n.evals < b.evals, "{}", report.join("\n"));
        // both land on the target
        for s in [&n, &b] {
            let before = e.wb.cell_text(k);
            e.set_text(k, &s.text);
            assert!((yr(&e) - want).abs() < want * 1e-4, "{input} = {} gives {} yr, not {want}", s.text, yr(&e));
            e.set_text(k, &before);
        }
    }
    println!("{}", report.join("\n"));
}

#[test]
fn tornado_chart_of_named_inputs() {
    let mut e = solar();
    // ranges on two inputs: their bars span them; the rest swing ±10%
    e.set_range("solarCost", "800 [USD/kW]", "1600 [USD/kW]").unwrap();
    e.set_range("price", "0.2 [USD/(kW*h)]", "0.4 [USD/(kW*h)]").unwrap();
    set(&mut e, "J1", "=G3 sensitivity");
    let Some(Ok(Value::Chart(c))) = e.result(key(&e, "J1")) else { panic!("{}", show(&e, "J1")) };
    let wbs_core::chart::Xs::Text(cats) = &c.layers[0].xs else { panic!() };
    let ys = &c.layers[0].ys;
    assert_eq!(cats.data.len(), 7, "{cats:?}");
    // largest swing first
    assert!(ys.data.windows(2).all(|w| w[0].abs() >= w[1].abs()), "{:?}", ys.data);
    // the solar cost bar: d(payback)/d(cost) across 800 USD/kW (canonically 0.8 USD/W)
    let i = cats.data.iter().position(|s| &**s == "solarCost (range)").unwrap();
    let d = e.sensitivities(e.wb.names["solarCost"].cell, &[key(&e, "G3")]).unwrap().remove(0);
    assert!((ys.data[i] - d.data[0] * 0.8).abs() < 1e-9 * ys.data[i].abs(), "{} vs {}", ys.data[i], d.data[0] * 0.8);
    assert_eq!(fmt_quantity(ys.data[i], &ys.q).split(' ').nth(1), Some("yr"));
    assert!(cats.data.iter().any(|s| &**s == "roundtrip (±10%)"), "{cats:?}");
    assert_eq!(ys.q.dim.to_string(), "time");
    // follows the model: a dearer system pays back slower, so the chart changes
    let before = ys.data.to_vec();
    set(&mut e, "B6", "500 [USD/(kW*h)]");
    let Some(Ok(Value::Chart(c))) = e.result(key(&e, "J1")) else { panic!() };
    assert_ne!(c.layers[0].ys.data.to_vec(), before);
    // the inspector's list: per step of each input, in its own unit
    let list = e.input_sensitivities(key(&e, "G3"), 0).unwrap();
    let cost = list.iter().find(|s| s.name == "solarCost").unwrap();
    let Some(Ok(Value::Num(out))) = e.result(key(&e, "G3")) else { panic!() };
    assert!(cost.per_step(&out.q).ends_with(" yr per +100 USD/kW"), "{}", cost.per_step(&out.q));
}
