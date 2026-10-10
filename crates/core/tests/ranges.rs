//! Input ranges: bounds on a named input, checked in the input's display unit.
//! (Written without a toolchain at hand: if one fails to compile, fix the test, not the rule.)

use wbs_core::a1;
use wbs_core::engine::{Engine, Shown};
use wbs_core::ids::CellKey;
use wbs_core::stdlib::default_workbook;

fn key(e: &Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheets[0].key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, r: &str, text: &str) {
    let a = a1::parse_ref(r).unwrap();
    let k = e.wb.sheets[0].key_grow(a.row, a.col);
    e.set_text(k, text);
}

fn show(e: &Engine, r: &str) -> String {
    match e.shown(key(e, r)) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

fn model() -> Engine {
    let mut e = Engine::new(default_workbook());
    e.wb.sheets[0].ensure_size(10, 4);
    set(&mut e, "B2", "0.5 [%]");
    set(&mut e, "B3", "=damping 2 *");
    let k = key(&e, "B2");
    e.set_name("damping", Some(k), true).unwrap();
    e
}

#[test]
fn typed_value_outside_the_range_is_an_error_and_kept() {
    let mut e = model();
    e.set_bounds("damping", "0 [%]", "100 [%]").unwrap();
    assert!(!show(&e, "B2").starts_with("ERR"), "{}", show(&e, "B2"));
    set(&mut e, "B2", "-0.3 [%]");
    assert!(show(&e, "B2").starts_with("ERR damping must be ≥ 0"), "{}", show(&e, "B2"));
    assert_eq!(e.wb.cell_text(key(&e, "B2")), "-0.3 [%]");
    assert!(show(&e, "B3").starts_with("ERR"), "{}", show(&e, "B3"));
    set(&mut e, "B2", "101 [%]");
    assert!(show(&e, "B2").contains("must be ≤"), "{}", show(&e, "B2"));
}

#[test]
fn bounds_must_have_the_inputs_dimension() {
    let mut e = model();
    assert!(e.set_bounds("damping", "0 [m]", "").is_err());
    assert!(e.set_bounds("damping", "5", "1").is_err());
    assert!(e.set_bounds("nothing", "0", "").is_err());
    assert_eq!(e.wb.names["damping"].min, None);
}

#[test]
fn bounds_in_display_units_and_undo() {
    let mut e = model();
    let undo = e.set_bounds("damping", "0 [%]", "").unwrap();
    let r = e.display_range(key(&e, "B2")).unwrap();
    assert_eq!((r.min, r.max), (Some(0.0), None));
    e.apply(undo);
    assert!(e.display_range(key(&e, "B2")).is_none());
}

#[test]
fn rename_keeps_the_range() {
    let mut e = model();
    e.set_bounds("damping", "0 [%]", "10 [%]").unwrap();
    let k = key(&e, "B2");
    e.set_name("zeta", Some(k), true).unwrap();
    assert_eq!(e.wb.names["zeta"].max.as_deref(), Some("10 [%]"));
}
