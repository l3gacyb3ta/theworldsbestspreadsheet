//! Input ranges: an input's range limits scrubbing, chart dragging and goal-seek; typing a value
//! outside it is an error on that cell.

use wbs_core::a1;
use wbs_core::bounds::InputRange;
use wbs_core::engine::{Engine, ErrKind, Shown};
use wbs_core::ids::CellKey;
use wbs_core::model::Workbook;
use wbs_core::ops::{cell_literal, scrub_value};
use wbs_core::solve::Goal;
use wbs_core::stdlib::default_workbook;

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

/// A spring's damping (an input, B1) and what it feeds (B2).
fn spring() -> Engine {
    let mut e = Engine::new(default_workbook());
    set(&mut e, "B1", "0.3 [1/s]");
    set(&mut e, "B2", "=damping 2 *");
    let k = key(&e, "B1");
    e.set_name("damping", Some(k), true).unwrap();
    e
}

#[test]
fn a_typed_value_outside_the_range_is_an_error_on_the_input() {
    let mut e = spring();
    e.set_range("damping", "0 [1/s]", "").unwrap();
    assert_eq!(show(&e, "B2"), "0.6 1/s");
    set(&mut e, "B1", "-0.3 [1/s]");
    assert_eq!(show(&e, "B1"), "ERR damping must be ≥ 0 [1/s]");
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "-0.3 [1/s]", "kept as typed, never clamped");
    let Shown::Error(err) = e.shown(key(&e, "B2")) else { panic!() };
    assert_eq!(err.kind, ErrKind::Upstream(key(&e, "B1")));
    // the end itself is inside
    set(&mut e, "B1", "0 [1/s]");
    assert_eq!(show(&e, "B2"), "0 1/s");
    // scrubbing in and out, with the recalc plan reused, keeps getting it right
    for (v, b2) in [("0.5", "1 1/s"), ("0.4", "0.8 1/s"), ("-0.1", "ERR B1 has an error"), ("0.2", "0.4 1/s"), ("-0.2", "ERR B1 has an error")] {
        set(&mut e, "B1", &format!("{v} [1/s]"));
        assert_eq!(show(&e, "B2"), b2, "at {v}");
    }
    // a max too, in another unit of the same dimension
    e.set_range("damping", "0 [1/s]", "6 [1/min]").unwrap();
    set(&mut e, "B1", "0.2 [1/s]");
    assert_eq!(show(&e, "B1"), "ERR damping must be ≤ 6 [1/min]");
    set(&mut e, "B1", "0.1 [1/s]");
    assert_eq!(show(&e, "B2"), "0.2 1/s", "6/min is 0.1/s: the end is inside");
}

#[test]
fn ranges_in_units_and_percent() {
    let mut e = Engine::new(default_workbook());
    set(&mut e, "B1", "5 [%]");
    let k = key(&e, "B1");
    e.set_name("growth", Some(k), true).unwrap();
    // a plain number is a fraction: 0.5 is 50 %
    e.set_range("growth", "0 [%]", "0.5").unwrap();
    let r = e.input_range(k).unwrap();
    assert_eq!(r, InputRange { min: Some((0.0, "0 [%]".into())), max: Some((50.0, "0.5".into())) });
    set(&mut e, "B1", "60 [%]");
    assert_eq!(show(&e, "B1"), "ERR growth must be ≤ 0.5");
    // a bound in the wrong dimension is refused where it's set, and nothing changes
    let before = e.wb.names["growth"].clone();
    assert_eq!(e.set_range("growth", "0 [m]", "").unwrap_err(), "min 0 [m] is length, but growth is dimensionless");
    assert!(e.set_range("growth", "", "zero").unwrap_err().starts_with("max: "));
    assert_eq!(e.set_range("growth", "1", "0.5").unwrap_err(), "min 1 is more than max 0.5");
    assert!(e.set_range("growth", "=B2", "").is_err(), "an end is a number, not a formula");
    assert_eq!(e.wb.names["growth"], before);
    // only inputs have ranges
    set(&mut e, "C1", "3 [km]");
    let c1 = key(&e, "C1");
    e.set_name("dist", Some(c1), false).unwrap();
    assert_eq!(e.set_range("dist", "0 [m]", "").unwrap_err(), "dist isn't an input");
    e.set_name("dist", Some(c1), true).unwrap();
    e.set_range("dist", "500 [m]", "").unwrap();
    // in the literal's own numbers: 500 m is 0.5 km
    let r = e.input_range(c1).unwrap();
    assert!((r.min.as_ref().unwrap().0 - 0.5).abs() < 1e-12, "{r:?}");
    assert_eq!(r.pin(0.2), (r.min.as_ref().unwrap().0, Some("min 500 [m]".into())));
    // a cell whose unit no longer fits its range says so
    set(&mut e, "C1", "3 [s]");
    assert_eq!(show(&e, "C1"), "ERR dist's range: min 500 [m] is length, but dist is time");
    // dates
    set(&mut e, "D1", "2026-10-10");
    let d1 = key(&e, "D1");
    e.set_name("launch", Some(d1), true).unwrap();
    e.set_range("launch", "2026-10-01", "").unwrap();
    set(&mut e, "D1", "2026-09-30");
    assert_eq!(show(&e, "D1"), "ERR launch must be ≥ 2026-10-01");
}

#[test]
fn scrubbing_stops_at_the_range() {
    let mut e = spring();
    e.set_range("damping", "0 [1/s]", "0.45 [1/s]").unwrap();
    let k = key(&e, "B1");
    let r = e.input_range(k).unwrap();
    let lit = cell_literal(&e.wb.cell_text(k)).unwrap();
    let scrub = |steps: f64| {
        let (v, pin) = r.pin(scrub_value(&lit, steps));
        (r.format(v, lit.decimals, false), pin)
    };
    assert_eq!(scrub(-2.0), ("0.1".into(), None));
    assert_eq!(scrub(-5.0), ("0.0".into(), Some("min 0 [1/s]".into())));
    // an end with more decimals than the literal is written with them
    assert_eq!(scrub(3.0), ("0.45".into(), Some("max 0.45 [1/s]".into())));
    assert_eq!(scrub(1.0), ("0.4".into(), None));
    // no range: the plain scrub
    assert_eq!(InputRange::default().pin(scrub_value(&lit, -5.0)).0, -0.2);
}

#[test]
fn goal_seek_stays_inside_the_range() {
    let mut e = Engine::new(default_workbook());
    set(&mut e, "B1", "-0.5");
    set(&mut e, "B2", "=x x *");
    let k = key(&e, "B1");
    e.set_name("x", Some(k), true).unwrap();
    let goal = Goal { target: key(&e, "B2"), index: 0, want: 4.0, tol: 1e-9, input: k, decimals: Some(3) };
    // unbounded, the nearer root
    assert_eq!(e.goal_seek(&goal).unwrap().text, "-2.000");
    // x ≥ -1 leaves only the other one
    e.set_range("x", "-1", "").unwrap();
    assert_eq!(e.goal_seek(&goal).unwrap().text, "2.000");
    // and a max below it leaves none: the message names the ends it ran into
    e.set_range("x", "-1", "1.5").unwrap();
    let err = e.goal_seek(&goal).unwrap_err();
    assert!(err.starts_with("out of reach within -1 ≤ x ≤ 1.5: with x from -1 to 1.5, B2 only reaches 0 to "), "{err}");
    // an input outside its range has an error, and says so
    e.set_range("x", "0", "").unwrap();
    assert_eq!(show(&e, "B1"), "ERR x must be ≥ 0");
    assert_eq!(e.goal_seek(&goal).unwrap_err(), "x has an error: x must be ≥ 0");
    set(&mut e, "B1", "0.5");
    let err = e.goal_seek(&Goal { want: -1.0, ..goal.clone() }).unwrap_err();
    assert!(err.starts_with("out of reach within x ≥ 0: "), "{err}");
}

#[test]
fn range_changes_are_undoable_and_reevaluate_the_input() {
    let mut e = spring();
    set(&mut e, "B1", "-0.3 [1/s]");
    assert_eq!(show(&e, "B2"), "-0.6 1/s");
    let undo = e.set_range("damping", "0 [1/s]", "").unwrap();
    assert_eq!(show(&e, "B1"), "ERR damping must be ≥ 0 [1/s]");
    assert!(show(&e, "B2").starts_with("ERR"));
    let redo = e.apply(undo);
    assert_eq!(e.wb.names["damping"].min, None);
    assert_eq!(show(&e, "B2"), "-0.6 1/s");
    e.apply(redo);
    assert_eq!(e.wb.names["damping"].min.as_deref(), Some("0 [1/s]"));
    assert_eq!(show(&e, "B1"), "ERR damping must be ≥ 0 [1/s]");
    // a rename keeps the range; unticking "input" drops it (one edit: undo brings both back)
    let k = key(&e, "B1");
    e.set_name("zeta", Some(k), true).unwrap();
    assert_eq!(e.wb.names["zeta"].min.as_deref(), Some("0 [1/s]"));
    assert_eq!(show(&e, "B1"), "ERR zeta must be ≥ 0 [1/s]");
    let undo = e.set_name("zeta", Some(k), false).unwrap();
    assert_eq!(e.wb.names["zeta"].min, None);
    assert_eq!(show(&e, "B1"), "-0.3 1/s");
    e.apply(undo);
    assert_eq!(e.wb.names["zeta"].min.as_deref(), Some("0 [1/s]"));
}

#[test]
fn an_end_reads_the_units_it_is_written_in() {
    let mut e = Engine::new(default_workbook());
    set(&mut e, "C1", "2");
    set(&mut e, "C2", "[span] = C1 [m]");
    set(&mut e, "B1", "3 [m]");
    let k = key(&e, "B1");
    e.set_name("reach", Some(k), true).unwrap();
    e.set_range("reach", "", "2 [span]").unwrap();
    assert_eq!(show(&e, "B1"), "3 m");
    // a span of 1 m makes the max 2 m
    set(&mut e, "C1", "1");
    assert_eq!(show(&e, "B1"), "ERR reach must be ≤ 2 [span]");
    set(&mut e, "C1", "2");
    assert_eq!(show(&e, "B1"), "3 m");
}

#[test]
fn files_without_ranges_load_and_ranges_round_trip() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let json = std::fs::read_to_string(format!("{dir}/pre-keys-demo.json")).unwrap();
    let wb: Workbook = serde_json::from_str(&json).unwrap();
    assert!(!wb.names.is_empty() && wb.names.values().all(|d| d.min.is_none() && d.max.is_none()));
    // no range: nothing new is written
    let mut e = spring();
    assert!(!serde_json::to_string(&e.wb).unwrap().contains("\"min\""));
    e.set_range("damping", "0 [1/s]", "").unwrap();
    let json = serde_json::to_string(&e.wb).unwrap();
    let mut e2 = Engine::new(serde_json::from_str(&json).unwrap());
    assert_eq!(e2.wb.names["damping"], e.wb.names["damping"]);
    set(&mut e2, "B1", "-1 [1/s]");
    assert_eq!(show(&e2, "B1"), "ERR damping must be ≥ 0 [1/s]");
}
