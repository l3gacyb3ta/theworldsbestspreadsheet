//! Word values, `iterate` and `rk4`: recurrences and ODEs in one cell.

use wbs_core::a1;
use wbs_core::engine::{Engine, Shown};
use wbs_core::ids::CellKey;
use wbs_core::solve::Goal;
use wbs_core::stdlib::default_workbook;
use wbs_core::value::{Prov, Value};

fn eng() -> Engine {
    Engine::new(default_workbook())
}

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

/// A cell's numbers, in canonical units.
fn nums(e: &Engine, r: &str) -> Vec<f64> {
    match e.result(key(e, r)) {
        Some(Ok(Value::Num(n))) => n.data.to_vec(),
        other => panic!("{r}: {other:?}"),
    }
}

fn sdim(e: &Engine, r: &str) -> String {
    e.static_value(key(e, r)).map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

#[test]
fn word_values() {
    let mut e = eng();
    set(&mut e, "A1", ": sq dup * ;");
    set(&mut e, "B1", "='sq");
    assert_eq!(show(&e, "B1"), "'sq");
    assert_eq!(sdim(&e, "B1"), "'sq");
    // used as a number, a word is an ordinary type error, known without evaluating
    set(&mut e, "B2", "='sq 1 +");
    assert_eq!(show(&e, "B2"), "ERR + expects a number, got word");
    set(&mut e, "B3", "=B1 2 *");
    assert_eq!(show(&e, "B3"), "ERR * expects a number, got word");
    // a word value in a cell can be handed on, and follows the word's edits
    set(&mut e, "D1", "=3 2 B1 1 iterate");
    assert_eq!(show(&e, "D1"), "2");
    assert_eq!(nums(&e, "D1"), vec![2.0, 4.0, 16.0, 256.0]);
    set(&mut e, "A1", ": sq 1 + ;");
    assert_eq!(nums(&e, "D1"), vec![2.0, 3.0, 4.0, 5.0]);
    // builtins quote too; unknown words don't compile
    set(&mut e, "E1", "=3 1 2 'swap 2 iterate swap drop");
    assert_eq!(nums(&e, "E1"), vec![2.0, 1.0, 2.0, 1.0]);
    set(&mut e, "B6", "='nope");
    assert!(show(&e, "B6").contains("unknown word nope"), "{}", show(&e, "B6"));
    // a word definition cell isn't a word value
    set(&mut e, "B7", "=3 1 A1 1 iterate");
    assert_eq!(show(&e, "B7"), "ERR iterate expects a word ('name), got word definition");
    // sheet references still lex as references
    set(&mut e, "B8", "='Sheet1'!D1 'sq drop");
    assert_eq!(show(&e, "B8"), "2");
}

#[test]
fn iterate_matches_closed_forms() {
    let mut e = eng();
    // exponential decay: x ← x·(1 − r)
    set(&mut e, "A1", ": decay 0.9 * ;");
    set(&mut e, "B1", "=50 100 [kg] 'decay 1 iterate");
    let xs = nums(&e, "B1");
    assert_eq!(xs.len(), 51);
    for (i, x) in xs.iter().enumerate() {
        assert!((x - 100.0 * 0.9f64.powi(i as i32)).abs() < 1e-9, "{i}: {x}");
    }
    assert_eq!(sdim(&e, "B1"), "mass");
    assert_eq!(show(&e, "B1"), "100 kg");
    // a loan: balance and month, two states with different units
    set(&mut e, "A2", ": pay { b m } b 1.01 * 100 [USD] - m 1 + ;");
    set(&mut e, "C1", "=12 1000 [USD] 0 'pay 2 iterate drop");
    let b = nums(&e, "C1");
    let want = 1000.0 * 1.01f64.powi(12) - 100.0 * (1.01f64.powi(12) - 1.0) / 0.01;
    assert!((b[12] - want).abs() < 1e-6, "{} vs {want}", b[12]);
    set(&mut e, "D1", "=12 1000 [USD] 0 'pay 2 iterate swap drop");
    assert_eq!(nums(&e, "D1")[12], 12.0);
    // zero steps: just the start
    set(&mut e, "E1", "=0 7 'decay 1 iterate");
    assert_eq!(nums(&e, "E1"), vec![7.0]);
    // array states step elementwise
    set(&mut e, "F1", "=2 3 range 'decay 1 iterate");
    let Some(Ok(Value::Num(n))) = e.result(key(&e, "F1")) else { panic!() };
    assert_eq!(n.shape.as_slice(), &[3, 3]);
}

#[test]
fn rk4_matches_closed_forms() {
    let mut e = eng();
    // exponential decay dx/dt = −x/τ
    set(&mut e, "A1", "2 [s]");
    e.set_name("tau", Some(key(&e, "A1")), true).unwrap();
    set(&mut e, "A2", ": decay { t x } x neg tau / ;");
    set(&mut e, "B1", "=0 [s] 0.1 [s] 100 1 [m] 'decay 1 rk4");
    let xs = nums(&e, "B1");
    for (i, x) in xs.iter().enumerate() {
        let t = i as f64 * 0.1;
        assert!((x - (-t / 2.0).exp()).abs() < 1e-7, "t={t}: {x}");
    }
    assert_eq!(sdim(&e, "B1"), "length");
    // a harmonic oscillator, ω = 2/s: x = cos ωt, v = −ω sin ωt
    set(&mut e, "A3", "4 [N/m]");
    set(&mut e, "A4", "1 [kg]");
    e.set_name("k", Some(key(&e, "A3")), true).unwrap();
    e.set_name("m", Some(key(&e, "A4")), true).unwrap();
    set(&mut e, "A5", ": spring ( t x v -- dx dv ) { t x v } v k m / x * neg ;");
    set(&mut e, "C1", "=0 [s] 0.01 [s] 1000 1 [m] 0 [m/s] 'spring 2 rk4 drop");
    set(&mut e, "D1", "=0 [s] 0.01 [s] 1000 1 [m] 0 [m/s] 'spring 2 rk4 swap drop");
    let (xs, vs) = (nums(&e, "C1"), nums(&e, "D1"));
    assert_eq!(xs.len(), 1001);
    for i in (0..=1000).step_by(50) {
        let t = i as f64 * 0.01;
        assert!((xs[i] - (2.0 * t).cos()).abs() < 1e-6, "x({t}) = {}", xs[i]);
        assert!((vs[i] + 2.0 * (2.0 * t).sin()).abs() < 1e-6, "v({t}) = {}", vs[i]);
    }
    assert_eq!(sdim(&e, "C1"), "length");
    assert_eq!(sdim(&e, "D1"), "length/time");
    // the matching time column
    set(&mut e, "E1", "=1000 1 + range 0.01 [s] * 0 [s] +");
    assert_eq!(nums(&e, "E1")[1000], 10.0);
    // the time is passed in: dx/dt = t gives t²/2, exactly
    set(&mut e, "A6", ": ramp { t x } t 1 [m/s^2] * ;");
    set(&mut e, "F1", "=0 [s] 0.5 [s] 4 0 [m] 'ramp 1 rk4");
    assert_eq!(nums(&e, "F1"), vec![0.0, 0.125, 0.5, 1.125, 2.0]);
}

#[test]
fn units_check_the_physics() {
    let mut e = eng();
    set(&mut e, "A1", ": fall { t y v } v -9.81 [m/s^2] ;");
    // acceleration where velocity belongs
    set(&mut e, "B1", "=0 [s] 0.1 [s] 10 0 [m/s] 10 [m] 'fall 2 rk4 drop");
    assert_eq!(show(&e, "B1"), "ERR rk4 needs matching units: state 1 is length/time, but its derivative × the step is length*time");
    // known before any input is there
    set(&mut e, "B2", "=C9 drop 0 [s] 0.1 [s] 10 0 [m/s] 10 [m] 'fall 2 rk4 drop");
    assert_eq!(show(&e, "B2"), "ERR rk4 needs matching units: state 1 is length/time, but its derivative × the step is length*time");
    // time and step must agree, and the step is a difference
    set(&mut e, "B3", "=0 [m] 0.1 [s] 10 10 [m] 0 [m/s] 'fall 2 rk4 drop");
    assert_eq!(show(&e, "B3"), "ERR rk4 needs matching units: length vs time");
    set(&mut e, "B4", "=2026-01-01 2026-01-02 10 10 [m] 0 [m/s] 'fall 2 rk4 drop");
    assert!(show(&e, "B4").starts_with("ERR rk4: the step size is a difference"), "{}", show(&e, "B4"));
    // a step that changes a state's dimension
    set(&mut e, "A2", ": sq dup * ;");
    set(&mut e, "C1", "=C9 drop 3 1 [m] 'sq 1 iterate");
    assert_eq!(show(&e, "C1"), "ERR iterate needs matching units: 'sq turns state 1 from length into length^2");
    set(&mut e, "C2", "=3 20 [°C] 1 [Δ°C] 'swap 2 iterate drop");
    assert_eq!(show(&e, "C2"), "ERR iterate needs matching units: 'swap turns state 1 from temperature (absolute) into temperature");
    // ... even when it takes no steps
    set(&mut e, "C3", "=0 1 [m] 'sq 1 iterate");
    assert_eq!(show(&e, "C3"), "ERR iterate needs matching units: 'sq turns state 1 from length into length^2");
}

#[test]
fn counts_and_arity() {
    let mut e = eng();
    set(&mut e, "A1", ": sq dup * ;");
    set(&mut e, "A2", ": two dup ;");
    set(&mut e, "A3", ": rate { t x } x 1 [1/s] * ;");
    let cases = [
        ("=3 1 'sq 0 iterate", "ERR iterate: the state count must be a whole number ≥ 1, got 0"),
        ("=3 1 'sq 1.5 iterate", "ERR iterate: the state count must be a whole number ≥ 1, got 1.5"),
        ("=3 1 'sq 2 iterate", "ERR iterate needs more values on the stack"),
        ("=3 1 2 '+ 2 iterate", "ERR iterate: '+ should leave 2 values, left 1"),
        ("=3 1 'two 1 iterate", "ERR iterate: 'two should leave 1 value, left 2"),
        ("=3 1 sq 1 iterate", "ERR iterate expects a word ('name), got number"),
        ("=3 \"a\" 'sq 1 iterate", "ERR iterate expects a number, got text"),
        ("=-1 1 'sq 1 iterate", "ERR iterate: the step count must be a whole number from 0 to 1,000,000, got -1"),
        ("=2.5 1 'sq 1 iterate", "ERR iterate: the step count must be a whole number from 0 to 1,000,000, got 2.5"),
        ("=1000001 1 'sq 1 iterate", "ERR iterate: the step count must be a whole number from 0 to 1,000,000, got 1000001"),
        ("=3 [s] 1 'sq 1 iterate", "ERR iterate expects a dimensionless number, got time"),
        ("=1000000 100 range 'sq 1 iterate", "ERR iterate: 1000000 steps of 100 values each is more than 10,000,000 values"),
        ("=3 3 range 'sum 1 iterate", "ERR iterate: 'sum gives state 1 with shape [], but state 1 has shape [3]"),
        ("=0 [s] 1 [s] 3 1 [m] 'sq 1 rk4", "ERR rk4: 'sq should leave 1 value, left 2"),
        // an empty start time: the unit checks still come first
        ("=0 range 1 [s] * 1 [s] 3 1 [m] 'drop 1 rk4", "ERR rk4 needs matching units: state 1 is length, but its derivative × the step is time^2"),
        ("=0 range 1 [s] * 1 [s] 3 1 [m] 'rate 1 rk4", "ERR rk4: the start time and the step are single numbers"),
    ];
    for (i, (prog, want)) in cases.iter().enumerate() {
        let at = format!("B{}", i + 1);
        set(&mut e, &at, prog);
        assert_eq!(show(&e, &at), *want, "{prog}");
    }
}

#[test]
fn words_cant_reach_themselves_through_quotes() {
    let mut e = eng();
    set(&mut e, "A1", ": loop 3 swap 'loop 1 iterate ;");
    set(&mut e, "B1", "=1 loop");
    assert!(show(&e, "B1").starts_with("ERR"), "{}", show(&e, "B1"));
    // through another word
    set(&mut e, "A2", ": ping 3 swap 'pong 1 iterate drop ;");
    set(&mut e, "A3", ": pong ping ;");
    set(&mut e, "B2", "=1 pong");
    assert!(show(&e, "B2").starts_with("ERR"), "{}", show(&e, "B2"));
    // nesting is fine: iterate inside a step word
    set(&mut e, "A4", ": inner 2 swap 'dup 1 iterate ;");
    set(&mut e, "A5", ": add1 1 + ;");
    set(&mut e, "A6", ": outer 4 swap 'add1 1 iterate last ;");
    set(&mut e, "B3", "=3 1 'outer 1 iterate");
    assert_eq!(nums(&e, "B3"), vec![1.0, 5.0, 9.0, 13.0]);
}

#[test]
fn results_are_derived_so_charts_goal_seek_through_them() {
    let mut e = eng();
    set(&mut e, "A1", "0.5 [1/s]");
    e.set_name("rate", Some(key(&e, "A1")), true).unwrap();
    set(&mut e, "A2", ": decay { t x } x neg rate * ;");
    set(&mut e, "B1", "=0 [s] 0.1 [s] 10 1 [m] 'decay 1 rk4");
    set(&mut e, "C1", "=11 range 0.1 [s] * B1 line");
    let Some(Ok(Value::Chart(c))) = e.result(key(&e, "C1")) else { panic!() };
    assert_eq!(c.layers[0].ys.prov.get(10), Prov::Derived(key(&e, "B1"), 10));
    // drag x(1 s) to half a metre: the rate is ln 2
    let goal = Goal { target: key(&e, "B1"), index: 10, want: 0.5, tol: 1e-6, input: key(&e, "A1"), decimals: Some(4), newton: true };
    let s = e.goal_seek(&goal).unwrap();
    assert_eq!(s.text, "0.6931 [1/s]");
}

/// A thousand-step spring stays live while an input is scrubbed.
#[test]
fn scrubbing_a_simulation_is_fast() {
    let mut e = eng();
    set(&mut e, "A1", "4 [N/m]");
    set(&mut e, "A2", "1 [kg]");
    set(&mut e, "A3", "0.1 [kg/s]");
    for (n, at) in [("k", "A1"), ("m", "A2"), ("c", "A3")] {
        e.set_name(n, Some(key(&e, at)), true).unwrap();
    }
    set(&mut e, "A4", ": spring ( t x v -- dx dv ) { t x v } v   k m / x * neg   c m / v * - ;");
    set(&mut e, "B1", "=0 [s] 0.01 [s] 1000 1 [m] 0 [m/s] 'spring 2 rk4 drop");
    set(&mut e, "C1", "=1001 range 0.01 [s] * B1 line");
    assert!(show(&e, "B1").ends_with(" m"));
    let steps: Vec<f64> = (0..10)
        .map(|i| {
            let t = std::time::Instant::now();
            set(&mut e, "A3", &format!("0.{i} [kg/s]"));
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let best = steps.iter().cloned().fold(f64::INFINITY, f64::min);
    eprintln!("1000-step rk4 spring: best {best:.2} ms per scrub step");
    assert!(best < 16.0, "{best} ms per step is too slow for interactive scrubbing");
}

/// The spring model from the help page, with its inputs named and read only inside the word `msd`.
fn spring_model() -> Engine {
    let mut e = eng();
    for (at, text, name) in [
        ("B1", "4 [N/m]", Some("spring")),
        ("B2", "1 [kg]", Some("mass")),
        ("B3", "0.5 [N*s/m]", Some("damping")),
        ("B4", "0.25 [s]", Some("timestep")),
        ("B5", "40", Some("steps")),
        ("B6", "1 [m]", Some("x0")),
        ("B7", "0 [m/s]", Some("v0")),
        ("A9", ": msd ( t x v -- dx dv ) { t x v } v   spring mass / x * neg   damping mass / v * - ;", None),
        ("C1", "=0 [s] timestep steps x0 v0 'msd 2 rk4 drop", None),
    ] {
        set(&mut e, at, text);
        if let Some(n) = name {
            let k = key(&e, at);
            e.set_name(n, Some(k), true).unwrap();
        }
    }
    e
}

#[test]
fn inputs_read_inside_a_quoted_word_are_upstream() {
    let mut e = spring_model();
    let c1 = key(&e, "C1");
    let names = |e: &Engine, ks: Vec<CellKey>| ks.into_iter().filter_map(|k| e.name_of(k).map(str::to_string)).collect::<Vec<_>>();
    let mut up = names(&e, e.upstream_inputs(c1));
    up.sort();
    assert_eq!(up, ["damping", "mass", "spring", "steps", "timestep", "v0", "x0"]);
    // trace sees them too, and the word's inputs see C1 downstream
    let pre = names(&e, e.precedents(c1));
    for n in ["damping", "mass", "spring"] {
        assert!(pre.contains(&n.to_string()), "{pre:?}");
    }
    assert!(e.precedents(c1).contains(&key(&e, "A9")));
    assert!(e.dependents(key(&e, "B3")).contains(&c1));
    // scrubbing one updates the run
    let before = nums(&e, "C1");
    set(&mut e, "B3", "2 [N*s/m]");
    assert_ne!(nums(&e, "C1"), before);
    // and goal-seek solves for it: pull element 4 down to 0.1 m
    let g = Goal { target: c1, index: 4, want: 0.1, tol: 1e-6, input: key(&e, "B3"), decimals: Some(4), newton: true };
    let s = e.goal_seek(&g).unwrap();
    set(&mut e, "B3", &s.text);
    assert!((nums(&e, "C1")[4] - 0.1).abs() < 1e-3, "{} → {:?}", s.text, nums(&e, "C1"));
}

#[test]
fn inputs_read_by_a_word_a_quoted_word_calls_are_upstream() {
    let mut e = eng();
    set(&mut e, "B1", "1.1");
    e.set_name("rate", Some(key(&e, "B1")), true).unwrap();
    set(&mut e, "B2", "3");
    set(&mut e, "A1", ": scale rate * ;");
    set(&mut e, "A2", ": grow scale ;");
    set(&mut e, "C1", "=3 1 'grow 1 iterate");
    let c1 = key(&e, "C1");
    assert_eq!(e.upstream_inputs(c1), vec![key(&e, "B1")]);
    assert!(e.dependents(key(&e, "B1")).contains(&c1));
    set(&mut e, "B1", "2");
    assert_eq!(nums(&e, "C1"), vec![1.0, 2.0, 4.0, 8.0]);
    // a word that starts reading another input brings it along, without C1 being edited
    set(&mut e, "A1", ": scale rate * B2 * ;");
    assert_eq!(e.upstream_inputs(c1), vec![key(&e, "B1"), key(&e, "B2")]);
    assert_eq!(nums(&e, "C1"), vec![1.0, 6.0, 36.0, 216.0]);
    set(&mut e, "B2", "1");
    assert_eq!(nums(&e, "C1"), vec![1.0, 2.0, 4.0, 8.0]);
}

/// The spring's phase plot: position against speed, from separate cells.
fn phase_plot() -> Engine {
    let mut e = spring_model();
    set(&mut e, "C1", "=0 [s] timestep steps x0 v0 'msd 2 rk4 drop");
    set(&mut e, "D1", "=0 [s] timestep steps x0 v0 'msd 2 rk4 swap drop");
    set(&mut e, "F1", "=C1 D1 path");
    e
}

fn chart_provs(e: &Engine, at: &str) -> (Vec<Prov>, Vec<Prov>) {
    let Some(Ok(Value::Chart(c))) = e.result(key(e, at)) else { panic!("{}", show(e, at)) };
    let l = &c.layers[0];
    let wbs_core::chart::Xs::Num(xs) = &l.xs else { panic!() };
    (xs.prov.to_vec(xs.len()).unwrap(), l.ys.prov.to_vec(l.ys.len()).unwrap())
}

#[test]
fn a_history_starts_with_its_start_cells() {
    let mut e = phase_plot();
    let (xs, vs) = chart_provs(&e, "F1");
    // the first point is x0 and v0, unchanged: it's theirs
    assert_eq!((xs[0], vs[0]), (Prov::Literal(key(&e, "B6")), Prov::Literal(key(&e, "B7"))));
    // every other point is computed
    assert_eq!((xs[1], vs[1]), (Prov::Derived(key(&e, "C1"), 1), Prov::Derived(key(&e, "D1"), 1)));
    assert_eq!(e.literal_source(key(&e, "C1"), 0), Some(key(&e, "B6")));
    assert_eq!(e.literal_source(key(&e, "C1"), 1), None);
    // through any number of cells that only rearrange
    set(&mut e, "M1", "=C1");
    set(&mut e, "N1", "=D1 rev rev");
    set(&mut e, "P1", "=M1 N1 path");
    let (xs, vs) = chart_provs(&e, "P1");
    assert_eq!((xs[0], vs[0]), (Prov::Literal(key(&e, "B6")), Prov::Literal(key(&e, "B7"))));
    // not once something computes it
    set(&mut e, "N1", "=D1 1 *");
    let (_, vs) = chart_provs(&e, "P1");
    assert_eq!(vs[0], Prov::Derived(key(&e, "N1"), 0));
    // iterate too; a computed start isn't anyone's
    set(&mut e, "A12", ": half 0.5 * ;");
    set(&mut e, "C60", "=4 x0 'half 1 iterate");
    assert_eq!(e.literal_source(key(&e, "C60"), 0), Some(key(&e, "B6")));
    set(&mut e, "C60", "=4 x0 2 * 'half 1 iterate");
    assert_eq!(e.literal_source(key(&e, "C60"), 0), None);
}
