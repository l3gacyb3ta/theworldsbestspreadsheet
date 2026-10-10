//! Sheet edits (add, delete, move, rename, duplicate) and their inverses.

use wbs_core::a1;
use wbs_core::engine::{Edit, Engine, Shown};
use wbs_core::help::explain_error;
use wbs_core::ids::{CellKey, SheetId};
use wbs_core::stdlib::default_workbook;

fn eng() -> Engine {
    Engine::new(default_workbook())
}

fn sid(e: &Engine, name: &str) -> SheetId {
    e.wb.sheet_by_name(name).unwrap_or_else(|| panic!("no sheet {name}")).id
}

fn key(e: &mut Engine, sheet: SheetId, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheet_mut(sheet).unwrap().key_grow(a.row, a.col)
}

fn set(e: &mut Engine, sheet: SheetId, r: &str, text: &str) {
    let k = key(e, sheet, r);
    e.set_text(k, text);
}

fn show(e: &mut Engine, sheet: SheetId, r: &str) -> String {
    let k = key(e, sheet, r);
    match e.shown(k) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

fn text(e: &mut Engine, sheet: SheetId, r: &str) -> String {
    let k = key(e, sheet, r);
    e.wb.cell_text(k)
}

fn names(e: &Engine) -> Vec<String> {
    e.wb.sheets.iter().map(|s| s.name.clone()).collect()
}

/// A workbook with a `data` sheet (between Sheet1 and units) that Sheet1 reads.
fn with_data() -> (Engine, SheetId, SheetId) {
    let mut e = eng();
    let add = e.add_sheet_edit(1);
    e.apply(add);
    let d = e.wb.sheets[1].id;
    let r = e.rename_sheet_edit(d, "data").unwrap();
    e.apply(r);
    let s1 = sid(&e, "Sheet1");
    for (i, v) in ["1", "2", "3"].iter().enumerate() {
        set(&mut e, d, &format!("A{}", i + 1), v);
    }
    set(&mut e, d, "B1", "5 [m]");
    set(&mut e, s1, "A1", "=data!A1 10 *");
    set(&mut e, s1, "A2", "=data!A1:A3 sum");
    let b1 = key(&mut e, d, "B1");
    e.set_name("span", Some(b1), false).unwrap();
    set(&mut e, s1, "A3", "=span 2 *");
    set(&mut e, s1, "A4", "=A1 1 +");
    (e, s1, d)
}

#[test]
fn add_sheet_and_undo() {
    let mut e = eng();
    let add = e.add_sheet_edit(1);
    let inv = e.apply(add);
    assert_eq!(names(&e), ["Sheet1", "Sheet3", "units"]);
    let id = e.wb.sheets[1].id;
    let redo = e.apply(inv);
    assert_eq!(names(&e), ["Sheet1", "units"]);
    e.apply(redo);
    assert_eq!(names(&e), ["Sheet1", "Sheet3", "units"]);
    assert_eq!(e.wb.sheets[1].id, id, "redo brings back the same sheet");
}

#[test]
fn delete_sheet_breaks_refs_until_undo() {
    let (mut e, s1, d) = with_data();
    assert_eq!(show(&mut e, s1, "A1"), "10");
    assert_eq!(show(&mut e, s1, "A2"), "6");
    assert_eq!(show(&mut e, s1, "A3"), "10 m");
    let rows_before = e.wb.sheet(d).unwrap().rows.ids().to_vec();

    let del = e.delete_sheet_edit(d).unwrap();
    let undo = e.apply(del);
    assert_eq!(names(&e), ["Sheet1", "units"]);
    for at in ["A1", "A2", "A3"] {
        let msg = show(&mut e, s1, at);
        assert_eq!(msg, "ERR reference to a deleted sheet", "{at}");
        assert!(explain_error(&msg).is_some_and(|h| h.title.contains("deleted sheet")));
    }
    assert!(show(&mut e, s1, "A4").contains("has an error"));
    assert_eq!(text(&mut e, s1, "A1"), "=#ref! 10 *");
    // the error points at the reference
    let k = key(&mut e, s1, "A1");
    if let Shown::Error(err) = e.shown(k) {
        assert_eq!(err.span, Some(1..6));
    }

    let redo = e.apply(undo);
    assert_eq!(names(&e), ["Sheet1", "data", "units"]);
    assert_eq!(e.wb.sheets[1].id, d);
    assert_eq!(e.wb.sheet(d).unwrap().rows.ids(), &rows_before[..]);
    assert_eq!(text(&mut e, s1, "A1"), "=data!A1 10 *");
    assert_eq!(show(&mut e, s1, "A1"), "10");
    assert_eq!(show(&mut e, s1, "A2"), "6");
    assert_eq!(show(&mut e, s1, "A3"), "10 m");
    assert_eq!(show(&mut e, s1, "A4"), "11");

    e.apply(redo);
    assert_eq!(show(&mut e, s1, "A1"), "ERR reference to a deleted sheet");
}

#[test]
fn the_last_sheet_stays() {
    let mut e = eng();
    let units = sid(&e, "units");
    let del = e.delete_sheet_edit(units).unwrap();
    e.apply(del);
    let s1 = sid(&e, "Sheet1");
    assert!(e.delete_sheet_edit(s1).is_err());
    // applied raw it's a no-op
    e.apply(Edit::DeleteSheet { sheet: s1 });
    assert_eq!(names(&e), ["Sheet1"]);
}

#[test]
fn deleting_units_leaves_errors_not_panics() {
    let mut e = eng();
    let s1 = sid(&e, "Sheet1");
    let units = sid(&e, "units");
    assert!(e.declarations_on(units) > 20);
    assert_eq!(e.declarations_on(s1), 0);
    set(&mut e, s1, "A1", "5 [km]");
    set(&mut e, s1, "A2", "=A1 to[m]");
    set(&mut e, s1, "A3", "2026-10-08");
    set(&mut e, s1, "A4", "=1 2 +");
    let del = e.delete_sheet_edit(units).unwrap();
    let undo = e.apply(del);
    assert!(show(&mut e, s1, "A1").starts_with("ERR"), "{}", show(&mut e, s1, "A1"));
    assert!(show(&mut e, s1, "A2").starts_with("ERR"));
    assert!(show(&mut e, s1, "A3").starts_with("ERR"));
    assert_eq!(show(&mut e, s1, "A4"), "3");
    e.apply(undo);
    assert_eq!(show(&mut e, s1, "A2"), "5,000 m");
    assert_eq!(show(&mut e, s1, "A3"), "2026-10-08");
}

#[test]
fn move_sheet_and_undo() {
    let (mut e, _, d) = with_data();
    let inv = e.apply(e.move_sheet_edit(d, 2));
    assert_eq!(names(&e), ["Sheet1", "units", "data"]);
    let redo = e.apply(inv);
    assert_eq!(names(&e), ["Sheet1", "data", "units"]);
    e.apply(redo);
    let s1 = sid(&e, "Sheet1");
    e.apply(e.move_sheet_edit(s1, 99));
    assert_eq!(names(&e), ["units", "data", "Sheet1"]);
    assert_eq!(show(&mut e, s1, "A2"), "6");
}

#[test]
fn rename_updates_reference_text() {
    let (mut e, s1, d) = with_data();
    assert!(e.rename_sheet_edit(d, "  ").is_err());
    assert!(e.rename_sheet_edit(d, "units").is_err());
    assert!(e.rename_sheet_edit(d, "a\nb").is_err());
    assert!(e.rename_sheet_edit(d, "data").is_ok(), "keeping the name is fine");
    let r = e.rename_sheet_edit(d, " my inputs ").unwrap();
    let undo = e.apply(r);
    assert_eq!(names(&e), ["Sheet1", "my inputs", "units"]);
    assert_eq!(text(&mut e, s1, "A1"), "='my inputs'!A1 10 *");
    assert_eq!(text(&mut e, s1, "A2"), "='my inputs'!A1:A3 sum");
    assert_eq!(show(&mut e, s1, "A1"), "10");
    // typing the new name works too
    set(&mut e, s1, "B1", "='my inputs'!A2 1 +");
    assert_eq!(show(&mut e, s1, "B1"), "3");
    e.apply(undo);
    assert_eq!(text(&mut e, s1, "A1"), "=data!A1 10 *");
    assert_eq!(text(&mut e, s1, "B1"), "=data!A2 1 +");
}

#[test]
fn duplicate_points_local_refs_at_the_copy() {
    let mut e = eng();
    let s1 = sid(&e, "Sheet1");
    set(&mut e, s1, "A1", "1");
    set(&mut e, s1, "A2", "=A1 10 *");
    set(&mut e, s1, "A3", "=Sheet1!A1 100 *");
    set(&mut e, s1, "A4", "=A1:A2 sum");
    let dup = e.duplicate_sheet_edit(s1).unwrap();
    let undo = e.apply(dup);
    assert_eq!(names(&e), ["Sheet1", "Sheet1 copy", "units"]);
    let c = e.wb.sheets[1].id;
    assert_ne!(c, s1);
    assert_eq!(text(&mut e, c, "A2"), "=A1 10 *");
    set(&mut e, c, "A1", "2");
    assert_eq!(show(&mut e, c, "A2"), "20");
    assert_eq!(show(&mut e, c, "A4"), "22");
    assert_eq!(show(&mut e, s1, "A2"), "10", "the original is untouched");
    // an explicit sheet keeps pointing where it pointed
    assert_eq!(show(&mut e, c, "A3"), "100");
    set(&mut e, s1, "A1", "3");
    assert_eq!(show(&mut e, c, "A3"), "300");
    // a second copy gets a fresh name
    let dup2 = e.duplicate_sheet_edit(s1).unwrap();
    e.apply(dup2);
    assert_eq!(names(&e), ["Sheet1", "Sheet1 copy 2", "Sheet1 copy", "units"]);
    e.apply(Edit::DeleteSheet { sheet: e.wb.sheets[1].id });
    e.apply(undo);
    assert_eq!(names(&e), ["Sheet1", "units"]);
}

#[test]
fn duplicated_declarations_are_reported_twice() {
    let mut e = eng();
    let units = sid(&e, "units");
    let s1 = sid(&e, "Sheet1");
    set(&mut e, s1, "A1", "=3 [km] to[m]");
    let dup = e.duplicate_sheet_edit(units).unwrap();
    e.apply(dup);
    let c = sid(&e, "units copy");
    assert!(show(&mut e, c, "A1").contains("already defined"), "{}", show(&mut e, c, "A1"));
    assert_eq!(show(&mut e, s1, "A1"), "3,000 m");
}
