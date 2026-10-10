//! Goal-seek through the engine: solve one input so a computed value hits a target,
//! leaving the document untouched until the caller writes the answer.

use wbs_core::a1;
use wbs_core::engine::{Engine, Shown};
use wbs_core::ids::CellKey;
use wbs_core::solve::Goal;
use wbs_core::stdlib::default_workbook;
use wbs_core::value::{Prov, Value};

fn key(e: &Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheets[0].key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, r: &str, text: &str) {
    let a = a1::parse_ref(r).unwrap();
    let k = e.wb.sheets[0].key(a.row, a.col).unwrap();
    e.set_text(k, text);
}

fn show(e: &Engine, r: &str) -> String {
    match e.shown(key(e, r)) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

/// Every cell's text and what it shows, the stored rows and columns, and the whole saved document:
/// what "unchanged" means. (Trials whose spills reach past the stored rows write nothing.)
fn snapshot(e: &Engine) -> (Vec<String>, usize, usize, String) {
    let s = &e.wb.sheets[0];
    let mut out = Vec::new();
    for r in 0..60 {
        for c in 0..8 {
            let at = a1::cell_name(r, c);
            let k = s.key(r, c).unwrap();
            out.push(format!("{at}: {} = {}", e.wb.cell_text(k), show(e, &at)));
        }
    }
    (out, s.rows.len(), s.cols.len(), serde_json::to_string(&e.wb).unwrap())
}

/// The demo's growth model: monthly revenue for `months` months.
fn model() -> Engine {
    let mut e = Engine::new(default_workbook());
    for (at, text) in [
        ("B3", "120000 [USD]"),
        ("B4", "4.0 [%]"),
        ("B5", "24"),
        ("A10", "=months range 1 +"),
        ("B10", "=1 growth + A10 1 - ^ start *"),
        ("F3", "=A10 B10 line"),
    ] {
        set(&mut e, at, text);
    }
    for (name, at) in [("start", "B3"), ("growth", "B4"), ("months", "B5")] {
        let k = key(&e, at);
        e.set_name(name, Some(k), true).unwrap();
    }
    e
}

fn usd(e: &Engine, at: &str, v: f64) -> f64 {
    match e.result(key(e, at)) {
        Some(Ok(Value::Num(n))) => n.q.disp.to_canonical(v),
        other => panic!("{other:?}"),
    }
}

#[test]
fn chart_points_know_which_element_they_are() {
    let e = model();
    let Some(Ok(Value::Chart(c))) = e.result(key(&e, "F3")) else { panic!() };
    let prov = c.layers[0].ys.prov.as_ref().unwrap();
    assert_eq!(prov[11], Prov::Derived(key(&e, "B10"), 11));
    assert_eq!(e.element_cell(key(&e, "B10"), 11), key(&e, "B21"));
    // a reference into the spill is that element of its source
    let mut e = e;
    set(&mut e, "A40", "=B21 B22 join B21 B22 join line");
    let Some(Ok(Value::Chart(c))) = e.result(key(&e, "A40")) else { panic!() };
    assert_eq!(c.layers[0].ys.prov, None, "join computes: provenance is dropped");
    set(&mut e, "A40", "=A21:A22 B21:B22 line");
    let Some(Ok(Value::Chart(c))) = e.result(key(&e, "A40")) else { panic!() };
    assert_eq!(c.layers[0].ys.prov.as_deref(), Some(&vec![Prov::Derived(key(&e, "B10"), 11), Prov::Derived(key(&e, "B10"), 12)]));
}

#[test]
fn solve_growth_for_month_12_revenue() {
    let mut e = model();
    let before = snapshot(&e);
    // "what growth rate makes month 12 hit 1M?"
    let goal = Goal { target: key(&e, "B10"), index: 11, want: usd(&e, "B10", 1_000_000.0), tol: 100.0, input: key(&e, "B4"), decimals: None };
    let s = e.goal_seek(&goal).unwrap();
    assert_eq!(s.text, "21.3 [%]", "keeps the literal's one decimal and its unit");
    assert!(s.evals < 40, "{s:?}");
    assert_eq!(snapshot(&e), before, "solving doesn't change the document");
    // the caller writes it: one edit, whose inverse is the one undo step
    let undo = e.set_text(goal.input, &s.text);
    assert_eq!(show(&e, "B21"), "1,003,806.6 USD");
    e.apply(undo);
    assert_eq!(snapshot(&e), before);
    // more decimals on request (Shift in the app)
    let s = e.goal_seek(&Goal { decimals: Some(4), ..goal.clone() }).unwrap();
    assert_eq!(s.text, "21.2576 [%]");
    // solving the start revenue instead: whole dollars, the first within the tolerance (100 USD)
    let s = e.goal_seek(&Goal { input: key(&e, "B3"), ..goal }).unwrap();
    assert_eq!(s.text, "649569 [USD]");
}

#[test]
fn failures_leave_the_document_alone_and_say_why() {
    let mut e = model();
    let before = snapshot(&e);
    let b10 = key(&e, "B10");
    // month 1 is the start revenue, whatever the growth
    let goal = Goal { target: b10, index: 0, want: usd(&e, "B10", 500_000.0), tol: 1.0, input: key(&e, "B4"), decimals: None };
    let err = e.goal_seek(&goal).unwrap_err();
    assert_eq!(err, "out of reach: B10 stays at 120,000 USD for growth from -4,092 % to 4,100 %");
    assert_eq!(snapshot(&e), before);
    // months only changes how many there are; fewer than 12 and month 12 is gone. Trial values
    // spill far past the stored rows, into virtual ones: nothing is written.
    let err = e.goal_seek(&Goal { index: 11, input: key(&e, "B5"), ..goal.clone() }).unwrap_err();
    assert!(err.starts_with("out of reach: B21 stays at 184,734.49 USD for months from 12 to 24,600 "), "{err}");
    assert!(err.ends_with("(at months = 0 it's an error: B10 has only 0 values)"), "{err}");
    assert_eq!(snapshot(&e), before);
    // a target that's an error now
    set(&mut e, "C10", "=B10 1 [m] +");
    let err = e.goal_seek(&Goal { target: key(&e, "C10"), ..goal.clone() }).unwrap_err();
    assert_eq!(err, "C10 isn't a number");
    // an input that isn't a number literal
    let err = e.goal_seek(&Goal { input: key(&e, "A10"), ..goal }).unwrap_err();
    assert_eq!(err, "A10 isn't a number");
}

#[test]
fn jumps_are_reported() {
    let mut e = model();
    set(&mut e, "D3", "1.00");
    set(&mut e, "D4", "=D3 2 > 100 [USD] 0 [USD] if");
    let before = snapshot(&e);
    let goal = Goal { target: key(&e, "D4"), index: 0, want: usd(&e, "D4", 50.0), tol: 0.01, input: key(&e, "D3"), decimals: None };
    let err = e.goal_seek(&goal).unwrap_err();
    assert_eq!(err, "D4 jumps from 0 USD to 100 USD at D3 = 2, skipping 50 USD");
    assert_eq!(snapshot(&e), before);
}

#[test]
fn absolute_units_and_dates() {
    let mut e = model();
    set(&mut e, "B39", "20 [°C]");
    set(&mut e, "C39", "=B39 to[°F]");
    set(&mut e, "B40", "2026-12-24");
    set(&mut e, "C40", "=B40 2026-10-08 -");
    let c39 = key(&e, "C39");
    let want = match e.result(c39) {
        Some(Ok(Value::Num(n))) => n.q.disp.to_canonical(100.0),
        _ => panic!(),
    };
    let s = e.goal_seek(&Goal { target: c39, index: 0, want, tol: 0.01, input: key(&e, "B39"), decimals: None }).unwrap();
    assert_eq!(s.text, "38 [°C]");
    let s = e.goal_seek(&Goal { target: c39, index: 0, want, tol: 0.01, input: key(&e, "B39"), decimals: Some(2) }).unwrap();
    assert_eq!(s.text, "37.78 [°C]");
    // dates: whole days
    assert_eq!(show(&e, "C40"), "77 day");
    let c40 = key(&e, "C40");
    let want = match e.result(c40) {
        Some(Ok(Value::Num(n))) => n.q.disp.to_canonical(100.0),
        _ => panic!(),
    };
    let s = e.goal_seek(&Goal { target: c40, index: 0, want, tol: 1e-6, input: key(&e, "B40"), decimals: None }).unwrap();
    assert_eq!(s.text, "2027-01-16");
    let s = e.goal_seek(&Goal { target: c40, index: 0, want: want * 1.005, tol: 1e-6, input: key(&e, "B40"), decimals: Some(3) }).unwrap();
    assert_eq!(s.text, "2027-01-16", "the nearer whole day: dates have no decimals");
    assert_eq!(show(&e, "B40"), "2026-12-24");
}

/// A solve is a few dozen recalcs; on a 3,000-cell sheet it must stay well under a second.
#[test]
fn goal_seek_on_a_big_sheet() {
    let mut e = Engine::new(default_workbook());
    set(&mut e, "A1", "1.05");
    for r in 2..=1001 {
        set(&mut e, &format!("A{r}"), &format!("=A{} A1 *", r - 1));
        set(&mut e, &format!("B{r}"), &format!("=A{r} 100 [USD] *"));
        set(&mut e, &format!("C{r}"), &format!("=B{r} 0.8 * to[EUR]"));
    }
    set(&mut e, "D1", "=C2:C1001 /+");
    set(&mut e, "D2", "=C2:C13 /+");
    let d2 = key(&e, "D2");
    let want = match e.result(d2) {
        Some(Ok(Value::Num(n))) => n.data[0] * 2.0,
        _ => panic!(),
    };
    let goal = Goal { target: d2, index: 0, want, tol: want * 1e-4, input: key(&e, "A1"), decimals: Some(6) };
    let runs: Vec<(f64, usize)> = (0..3)
        .map(|_| {
            let t = std::time::Instant::now();
            let s = e.goal_seek(&goal).unwrap();
            (t.elapsed().as_secs_f64() * 1000.0, s.evals)
        })
        .collect();
    let best = runs.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    eprintln!("goal-seek over 3,000 dependent cells: best {best:.1} ms, {} evaluations", runs[0].1);
    assert!(runs[0].1 <= 30, "{runs:?}");
    assert!(best < 1000.0, "{best} ms");
}
