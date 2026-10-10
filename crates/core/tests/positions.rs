//! Position keys and virtual rows: files saved before them load with every cell where it was,
//! rows past the stored ones cost nothing until an edit needs their place, and the edits that
//! materialise rows undo back to exactly what was stored.
//!
//! The fixtures in `fixtures/` were saved by main at f0c2ac6 (before position keys) with a
//! throwaway test in the app crate: `pre-keys-demo` is the demo workbook scrolled to 320 rows and
//! 40 columns, `pre-keys-edited` a workbook with inserted, sorted and deleted rows, a deleted
//! column, cells and references past row 200, sizes, names, a moved and a deleted sheet. Each
//! `.txt` is `listing` below, run by main on the file it had just saved and read back.

use std::fmt::Write;
use wbs_core::a1;
use wbs_core::engine::{Edit, Engine, Shown};
use wbs_core::ids::{CellKey, SheetId};
use wbs_core::model::Workbook;
use wbs_core::ops::{self, Rect};
use wbs_core::stdlib::default_workbook;

fn shown(e: &Engine, k: CellKey) -> String {
    match e.shown(k) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

/// Every visible cell by position with its text and value, spilled values near the top, sizes and
/// names: the same function main ran to write the `.txt` fixtures.
fn listing(e: &Engine) -> String {
    let mut out = String::new();
    for s in &e.wb.sheets {
        writeln!(out, "sheet {}", s.name).unwrap();
        let mut cells: Vec<(usize, usize, CellKey)> = s
            .cells
            .keys()
            .filter_map(|(r, c)| {
                let k = CellKey { sheet: s.id, row: *r, col: *c };
                s.pos(k).map(|(ri, ci)| (ri, ci, k))
            })
            .collect();
        cells.sort();
        for (r, c, k) in cells {
            writeln!(out, "  {} {:?} = {}", a1::cell_name(r, c), e.wb.cell_text(k), shown(e, k)).unwrap();
        }
        for r in 0..60 {
            // main's units sheet had only 6 columns; past them there was nothing to list
            for c in 0..if s.name == "units" { 6 } else { 16 } {
                let Some(k) = s.key(r, c) else { continue };
                if !s.cells.contains_key(&(k.row, k.col)) && !matches!(e.shown(k), Shown::Empty) {
                    writeln!(out, "  spilled {} = {}", a1::cell_name(r, c), shown(e, k)).unwrap();
                }
            }
        }
        let mut sizes: Vec<String> = s.row_heights.iter().filter_map(|(r, h)| Some(format!("  row {} height {h}", s.rows.index(*r)? + 1))).collect();
        sizes.extend(s.col_widths.iter().filter_map(|(c, w)| Some(format!("  col {} width {w}", a1::col_name(s.cols.index(*c)?)))));
        sizes.sort();
        for l in sizes {
            writeln!(out, "{l}").unwrap();
        }
    }
    for (n, d) in &e.wb.names {
        writeln!(out, "name {n} -> {} {}", e.wb.cell_label(d.cell, None), d.input).unwrap();
    }
    out
}

fn fixture(name: &str) -> (Workbook, String) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let json = std::fs::read_to_string(format!("{dir}/{name}.json")).unwrap();
    (serde_json::from_str(&json).unwrap(), std::fs::read_to_string(format!("{dir}/{name}.txt")).unwrap())
}

fn sid(e: &Engine, name: &str) -> SheetId {
    e.wb.sheet_by_name(name).unwrap().id
}

fn key(e: &Engine, sheet: SheetId, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheet(sheet).unwrap().key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, sheet: SheetId, r: &str, text: &str) -> Edit {
    let k = key(e, sheet, r);
    e.set_text(k, text)
}

fn filled(e: &Engine) -> Vec<(String, usize, usize, usize, usize)> {
    e.wb.sheets.iter().map(|s| (s.name.clone(), s.rows.len(), s.cols.len(), s.rows.filled(), s.cols.filled())).collect()
}

#[test]
fn files_saved_before_keys_load_with_every_cell_in_place() {
    for name in ["pre-keys-demo", "pre-keys-edited"] {
        let (wb, want) = fixture(name);
        let e = Engine::new(wb);
        assert_eq!(listing(&e), want, "{name}");
        // their rows become stored rows keyed `m…`, in order, before any virtual row
        for s in &e.wb.sheets {
            assert_eq!(s.rows.filled(), 0);
            assert!(s.rows.keys().all(|(_, k)| k.starts_with('m')), "{name}: {}", s.name);
        }
        // saved again in the new format, nothing moves
        let json = serde_json::to_string(&e.wb).unwrap();
        assert!(!json.contains("\"order\""));
        let again = Engine::new(serde_json::from_str(&json).unwrap());
        assert_eq!(listing(&again), want, "{name} saved again");
    }
}

#[test]
fn deleted_rows_and_sheets_of_old_files_can_still_come_back() {
    let (wb, _) = fixture("pre-keys-edited");
    let mut e = Engine::new(wb);
    assert_eq!(e.wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["data", "Sheet1", "units"]);
    // the deleted sheet kept a key after the others
    let scratch = e.wb.deleted_sheets[0].id;
    e.apply(Edit::RestoreSheet { sheet: scratch });
    assert_eq!(e.wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["data", "Sheet1", "units", "scratch"]);
    // a deleted row comes back where it was: the end of the range E14 sums, which shrank
    let s1 = sid(&e, "Sheet1");
    assert_eq!(e.wb.cell_text(key(&e, s1, "E14")), "=C5:C12 sum");
    let s = e.wb.sheet(s1).unwrap();
    let dead: Vec<_> = s.rows.keys().filter(|(id, _)| s.rows.is_dead(*id)).map(|(id, _)| id).collect();
    assert_eq!(dead.len(), 3);
    e.apply(Edit::RestoreRows { sheet: s1, ids: vec![dead[0]] });
    assert_eq!(e.wb.cell_text(key(&e, s1, "E15")), "=C5:C13 sum");
    assert_eq!(e.wb.cell_text(key(&e, s1, "C13")), "9");
}

#[test]
fn a_new_sheet_stores_no_rows() {
    let e = Engine::new(default_workbook());
    for s in &e.wb.sheets {
        assert_eq!((s.rows.len(), s.cols.len(), s.rows.filled(), s.cols.filled()), (0, 0, 0, 0), "{}", s.name);
    }
    let json = serde_json::to_string(&e.wb.sheets[0]).unwrap();
    assert!(json.len() < 200, "{json}");
}

#[test]
fn writing_far_down_and_undoing_it_leaves_nothing_stored() {
    let mut e = Engine::new(default_workbook());
    let s1 = e.wb.sheets[0].id;
    let before = (filled(&e), serde_json::to_string(&e.wb).unwrap());
    let undo = set(&mut e, s1, "C5000", "42");
    let undo_c = e.apply(undo.clone());
    assert_eq!(serde_json::to_string(&e.wb).unwrap(), before.1, "write and undo: nothing stored");
    e.apply(undo_c);
    let undo2 = set(&mut e, s1, "A1", "=C5000 Z9000 + A7000:B7002 sum +");
    assert_eq!(filled(&e), before.0, "cells and references don't materialise rows");
    assert_eq!(e.wb.cell_text(key(&e, s1, "A1")), "=C5000 Z9000 + A7000:B7002 sum +");
    // a spill far down doesn't either
    let undo3 = set(&mut e, s1, "D6000", "=3000 range");
    assert_eq!(shown(&e, key(&e, s1, "D8999")), "2,999");
    assert_eq!(filled(&e), before.0);
    // an insert above moves them all down by one, and materialises only up to its anchor
    let ins = e.insert_rows_edit(s1, 9, 1);
    let undo_ins = e.apply(ins);
    assert_eq!(e.wb.sheets[0].rows.filled(), 10);
    assert_eq!(e.wb.cell_text(key(&e, s1, "C5001")), "42");
    assert_eq!(e.wb.cell_text(key(&e, s1, "A1")), "=C5001 Z9001 + A7001:B7003 sum +");
    assert_eq!(shown(&e, key(&e, s1, "D9000")), "2,999");
    e.apply(undo_ins);
    e.apply(undo3);
    e.apply(undo2);
    e.apply(undo);
    // the inserted row stays as a tombstone (with rows 1..=9 stored in front of it), as PR 1 undoes
    // an insert; everything else is as before
    assert_eq!(e.wb.sheets[0].rows.filled(), 9);
    assert_eq!(e.wb.cell_text(key(&e, s1, "C5000")), "");
}

#[test]
fn deleting_and_sorting_virtual_rows_undo_to_nothing_stored() {
    let mut e = Engine::new(default_workbook());
    let s1 = e.wb.sheets[0].id;
    for (i, v) in ["3", "1", "2"].iter().enumerate() {
        set(&mut e, s1, &format!("B{}", 300 + i), v);
    }
    set(&mut e, s1, "C1", "=B300:B302 sum");
    let before = serde_json::to_string(&e.wb).unwrap();
    let snap = listing(&e);
    // delete rows 100-101: rows 1..=101 get stored, and come off again on undo
    let del = e.delete_rows_edit(s1, 99, 2);
    let undo = e.apply(del);
    assert_eq!(e.wb.sheets[0].rows.filled(), 101);
    assert_eq!(e.wb.cell_text(key(&e, s1, "B298")), "3");
    assert_eq!(e.wb.cell_text(key(&e, s1, "C1")), "=B298:B300 sum");
    let redo = e.apply(undo);
    assert_eq!(serde_json::to_string(&e.wb).unwrap(), before);
    let undo = e.apply(redo);
    assert_eq!(e.wb.sheets[0].rows.filled(), 101);
    assert_eq!(e.wb.cell_text(key(&e, s1, "B298")), "3");
    e.apply(undo);
    assert_eq!(serde_json::to_string(&e.wb).unwrap(), before);
    // sorting a whole column's worth of rows stores only up to its last row with content
    let sort = ops::sort_rows(&e, Rect::span(s1, (0, 1), (999, 2)), 1, true);
    let undo = e.apply(sort);
    assert_eq!(e.wb.sheets[0].rows.filled(), 302);
    assert_eq!((1..4).map(|r| e.wb.cell_text(key(&e, s1, &format!("B{r}")))).collect::<Vec<_>>(), ["1", "2", "3"]);
    assert_eq!(e.wb.cell_text(key(&e, s1, "C4")), "=B300:B302 sum", "the formula's row moved; its range covers the same block");
    let redo = e.apply(undo);
    assert_eq!(serde_json::to_string(&e.wb).unwrap(), before);
    assert_eq!(listing(&e), snap);
    e.apply(redo);
    assert_eq!(e.wb.cell_text(key(&e, s1, "B1")), "1");
}

#[test]
fn inserting_past_the_stored_rows_anchors_on_the_virtual_row_there() {
    let (wb, _) = fixture("pre-keys-edited");
    let mut e = Engine::new(wb);
    let s1 = sid(&e, "Sheet1");
    let stored = e.wb.sheet(s1).unwrap().rows.len();
    assert_eq!(e.wb.cell_text(key(&e, s1, "A250")), "41");
    // the old file stored 300-odd rows; insert two rows at 400, past them
    let at = stored + 50;
    let below = key(&e, s1, &format!("A{}", at + 1));
    let ins = e.insert_rows_edit(s1, at, 2);
    let undo = e.apply(ins);
    let s = e.wb.sheet(s1).unwrap();
    assert_eq!(s.rows.filled(), 51, "virtual rows 0..=50 are stored, keyed after the old file's");
    assert_eq!(s.pos(below).unwrap().0, at + 2);
    let new: Vec<_> = (at..at + 2).map(|i| s.rows.get(i).unwrap()).collect();
    let k = |id| s.rows.pos_key(id).unwrap();
    assert!(new.iter().all(|id| k(*id).starts_with('t')));
    assert!(k(s.rows.get(at - 1).unwrap()) < k(new[0]) && k(new[1]) < k(below.row));
    e.apply(undo);
    assert_eq!(e.wb.sheet(s1).unwrap().pos(below).unwrap().0, at);
}

#[test]
fn sheet_undo_goes_back_to_the_old_tab_key() {
    let mut e = Engine::new(default_workbook());
    for at in [1, 2] {
        let add = e.add_sheet_edit(at);
        e.apply(add);
    }
    let names = |e: &Engine| e.wb.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&e), ["Sheet1", "Sheet3", "Sheet4", "units"]);
    let s3 = sid(&e, "Sheet3");
    let mv = e.move_sheet_edit(s3, 3);
    let undo = e.apply(mv);
    assert_eq!(names(&e), ["Sheet1", "Sheet4", "units", "Sheet3"]);
    // the old neighbour (Sheet1) moves away meanwhile: undo still puts Sheet3 back between them
    let s1 = sid(&e, "Sheet1");
    let mv = e.move_sheet_edit(s1, 2);
    e.apply(mv);
    assert_eq!(names(&e), ["Sheet4", "units", "Sheet1", "Sheet3"]);
    e.apply(undo);
    assert_eq!(names(&e), ["Sheet3", "Sheet4", "units", "Sheet1"]);
    // a deleted sheet keeps its key while its neighbours move
    let s4 = sid(&e, "Sheet4");
    let del = e.delete_sheet_edit(s4).unwrap();
    let restore = e.apply(del);
    let mv = e.move_sheet_edit(s3, 3);
    e.apply(mv);
    e.apply(restore);
    assert_eq!(names(&e), ["Sheet4", "units", "Sheet1", "Sheet3"]);
    // the tab order survives save and load
    let json = serde_json::to_string(&e.wb).unwrap();
    assert_eq!(names(&Engine::new(serde_json::from_str(&json).unwrap())), names(&e));
}

#[test]
fn duplicating_a_sheet_keeps_its_virtual_rows() {
    let mut e = Engine::new(default_workbook());
    let s1 = e.wb.sheets[0].id;
    set(&mut e, s1, "A900", "far");
    set(&mut e, s1, "B1", "=A900");
    let dup = e.duplicate_sheet_edit(s1).unwrap();
    e.apply(dup);
    let copy = sid(&e, "Sheet1 copy");
    assert_eq!(e.wb.cell_text(key(&e, copy, "B1")), "=A900");
    assert_eq!(shown(&e, key(&e, copy, "B1")), "far");
    assert_eq!(key(&e, copy, "A900").row, key(&e, s1, "A900").row);
}
