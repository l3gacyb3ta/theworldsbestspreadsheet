//! Edits name rows, columns and sheets by id, so undo and redo still hit the right ones after
//! other edits (someone else's, later) moved things around. Deleting hides, so an edit made to a
//! deleted row, column or sheet comes back with it.

use wbs_core::a1;
use wbs_core::engine::{Edit, Engine, Shown};
use wbs_core::ids::{CellKey, SheetId};
use wbs_core::model::{Cell, NameDef, Piece, Workbook};
use wbs_core::ops::{self, Rect};
use wbs_core::stdlib::default_workbook;

fn sid(e: &Engine, name: &str) -> SheetId {
    e.wb.sheet_by_name(name).unwrap_or_else(|| panic!("no sheet {name}")).id
}

fn key(e: &mut Engine, sheet: SheetId, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheet(sheet).unwrap().key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, sheet: SheetId, r: &str, text: &str) {
    let k = key(e, sheet, r);
    e.set_text(k, text);
}

fn shown(e: &Engine, k: CellKey) -> String {
    match e.shown(k) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

fn show(e: &mut Engine, sheet: SheetId, r: &str) -> String {
    let k = key(e, sheet, r);
    shown(e, k)
}

fn text(e: &mut Engine, sheet: SheetId, r: &str) -> String {
    let k = key(e, sheet, r);
    e.wb.cell_text(k)
}

fn put_text(k: CellKey, t: &str) -> Edit {
    Edit::Cells(vec![(k, Some(Cell::new(vec![Piece::Text(t.into())])))])
}

/// Everything the user sees, by position: tab order, each visible cell's text and value, sizes, names.
fn snap(e: &Engine) -> Vec<String> {
    let mut out = Vec::new();
    for s in &e.wb.sheets {
        out.push(format!("sheet {}", s.name));
        if s.name == "units" {
            continue;
        }
        let mut lines: Vec<String> = s
            .cells
            .keys()
            .map(|(r, c)| CellKey { sheet: s.id, row: *r, col: *c })
            .filter(|k| s.pos(*k).is_some())
            .map(|k| format!("{}!{} {} = {}", s.name, e.wb.cell_label(k, Some(s.id)), e.wb.cell_text(k), shown(e, k)))
            .collect();
        lines.extend(s.row_heights.iter().filter_map(|(r, h)| Some(format!("{} row {} height {h}", s.name, s.rows.index(*r)? + 1))));
        lines.extend(s.col_widths.iter().filter_map(|(c, w)| Some(format!("{} col {} width {w}", s.name, a1::col_name(s.cols.index(*c)?)))));
        lines.sort();
        out.extend(lines);
    }
    out.extend(e.wb.names.iter().map(|(n, d)| format!("name {n} -> {} {}", e.wb.cell_label(d.cell, None), d.input)));
    out
}

/// Sheet1 with a block in B3:C8 and formulas reading it in row 10 (rows 1-2 and column A empty, so
/// deleting them moves the content), a `data` sheet, a name and some sizes. Tabs: Sheet1, data, units.
fn base() -> Workbook {
    let mut e = Engine::new(default_workbook());
    let add = e.add_sheet_edit(1);
    e.apply(add);
    let d = e.wb.sheets[1].id;
    let r = e.rename_sheet_edit(d, "data").unwrap();
    e.apply(r);
    let s1 = sid(&e, "Sheet1");
    set(&mut e, d, "A1", "7");
    set(&mut e, d, "A2", "=A1 1 +");
    for (i, v) in ["3", "1", "2", "6", "5", "4"].iter().enumerate() {
        let r = i + 3;
        set(&mut e, s1, &format!("B{r}"), v);
        set(&mut e, s1, &format!("C{r}"), &format!("=B{r} 10 *"));
    }
    set(&mut e, s1, "D10", "=B3:B8 sum");
    set(&mut e, s1, "E10", "=C4");
    set(&mut e, s1, "F10", "=data!A2 2 *");
    let b3 = key(&mut e, s1, "B3");
    e.set_name("top", Some(b3), true).unwrap();
    let s = e.wb.sheet(s1).unwrap();
    let (c, r) = (s.cols.get(2).unwrap(), s.rows.get(4).unwrap());
    e.apply(Edit::ColWidth { sheet: s1, col: c, width: Some(120.0) });
    e.apply(Edit::RowHeight { sheet: s1, row: r, height: Some(30.0) });
    e.wb
}

type Build = fn(&mut Engine) -> Edit;

/// Every structural (and size/name) edit the app makes, on the content of `base`.
fn ops() -> Vec<(&'static str, Build)> {
    vec![
        ("insert rows", |e| e.insert_rows_edit(sid(e, "Sheet1"), 4, 2)),
        ("insert rows at the end", |e| e.insert_rows_edit(sid(e, "Sheet1"), 200, 1)),
        ("delete rows", |e| e.delete_rows_edit(sid(e, "Sheet1"), 4, 2)),
        ("insert columns", |e| e.insert_cols_edit(sid(e, "Sheet1"), 2, 1)),
        ("delete columns", |e| e.delete_cols_edit(sid(e, "Sheet1"), 2, 2)),
        ("sort", |e| ops::sort_rows(e, Rect::span(sid(e, "Sheet1"), (2, 1), (7, 2)), 1, true)),
        ("sort descending", |e| ops::sort_rows(e, Rect::span(sid(e, "Sheet1"), (3, 1), (6, 2)), 1, false)),
        ("move cells", |e| ops::move_cells(e, Rect::span(sid(e, "Sheet1"), (2, 1), (3, 1)), sid(e, "Sheet1"), (10, 6)).unwrap()),
        ("name", |e| {
            let cell = key(e, sid(e, "Sheet1"), "B5");
            Edit::Name { name: "mid".into(), def: Some(NameDef::new(cell, false)) }
        }),
        ("unname", |_| Edit::Name { name: "top".into(), def: None }),
        ("row height", |e| {
            let s = e.wb.sheet(sid(e, "Sheet1")).unwrap();
            Edit::RowHeight { sheet: s.id, row: s.rows.get(5).unwrap(), height: Some(44.0) }
        }),
        ("column width", |e| {
            let s = e.wb.sheet(sid(e, "Sheet1")).unwrap();
            Edit::ColWidth { sheet: s.id, col: s.cols.get(2).unwrap(), width: None }
        }),
        ("add sheet", |e| e.add_sheet_edit(1)),
        ("duplicate sheet", |e| e.duplicate_sheet_edit(sid(e, "Sheet1")).unwrap()),
        ("delete sheet", |e| e.delete_sheet_edit(sid(e, "data")).unwrap()),
        ("move sheet", |e| e.move_sheet_edit(sid(e, "units"), 0)),
        ("rename sheet", |e| e.rename_sheet_edit(sid(e, "data"), "inputs").unwrap()),
    ]
}

/// Unrelated edits elsewhere that shift positions under the op, and the ops they aren't unrelated
/// to. Undo puts a sheet back at its old tab key, so it no longer depends on a neighbour; but a new
/// or moved sheet is placed "after" a sheet, so an edit placed next to the sheet the op moves, or
/// one that moves or deletes the op's own sheet, is a real conflict, not an unrelated edit.
fn others() -> Vec<(&'static str, Build, &'static [&'static str])> {
    vec![
        ("insert a row at the top", |e| e.insert_rows_edit(sid(e, "Sheet1"), 0, 1), &[]),
        ("delete the first row", |e| e.delete_rows_edit(sid(e, "Sheet1"), 0, 1), &[]),
        ("insert a column at A", |e| e.insert_cols_edit(sid(e, "Sheet1"), 0, 2), &[]),
        ("delete column A", |e| e.delete_cols_edit(sid(e, "Sheet1"), 0, 1), &[]),
        ("add a sheet in front", |e| e.add_sheet_edit(0), &[]),
        // after units, which "move sheet" moves
        ("add a sheet at the end", |e| e.add_sheet_edit(3), &["move sheet"]),
        ("move data last", |e| e.move_sheet_edit(sid(e, "data"), 2), &["delete sheet", "move sheet"]),
        ("delete units", |e| e.delete_sheet_edit(sid(e, "units")).unwrap(), &[]),
    ]
}

/// op, then the unrelated edit, then undo op, redo it, undo it again. Undo must leave exactly what
/// the unrelated edit alone gives, and redo what both gave: the undo stack holds no positions.
/// (With positional inverses, undoing a row delete after an insert above restores the wrong row.)
#[test]
fn undo_redo_after_an_unrelated_edit_hits_the_same_rows() {
    let wb = base();
    for (op_name, op) in ops() {
        for (other_name, other, conflicts) in others() {
            if conflicts.contains(&op_name) {
                continue;
            }
            let what = format!("{op_name} / {other_name}");
            let mut a = Engine::new(wb.clone());
            let op_e = op(&mut a);
            let other_e = other(&mut a);
            let mut b = Engine::new(wb.clone());
            b.apply(other_e.clone());
            let other_only = snap(&b);

            let undo = a.apply(op_e);
            a.apply(other_e);
            let both = snap(&a);
            let redo = a.apply(undo);
            assert_eq!(snap(&a), other_only, "{what}: undo");
            let undo = a.apply(redo);
            assert_eq!(snap(&a), both, "{what}: redo");
            a.apply(undo);
            assert_eq!(snap(&a), other_only, "{what}: undo again");
        }
    }
}

/// The matrix above only compares two histories; check one case by eye too.
#[test]
fn undoing_a_row_delete_after_an_insert_above_restores_the_same_row() {
    let mut e = Engine::new(base());
    let s1 = sid(&e, "Sheet1");
    let del = e.delete_rows_edit(s1, 4, 1); // row 5: B5 = 2
    let undo = e.apply(del);
    assert_eq!(text(&mut e, s1, "B5"), "6");
    assert_eq!(text(&mut e, s1, "D9"), "=B3:B7 sum");
    assert_eq!(show(&mut e, s1, "D9"), "19");
    let ins = e.insert_rows_edit(s1, 0, 1);
    e.apply(ins);
    e.apply(undo);
    // the deleted row comes back as row 6, below the new row 1, and the range covers it again
    assert_eq!(text(&mut e, s1, "B6"), "2");
    assert_eq!(text(&mut e, s1, "B7"), "6");
    assert_eq!(text(&mut e, s1, "D11"), "=B4:B9 sum");
    assert_eq!(show(&mut e, s1, "D11"), "21");
}

#[test]
fn sort_undo_after_an_insert_above() {
    let mut e = Engine::new(base());
    let s1 = sid(&e, "Sheet1");
    let sort = ops::sort_rows(&e, Rect::span(s1, (2, 1), (7, 2)), 1, true);
    let undo = e.apply(sort);
    let col = |e: &mut Engine, c: &str, from: usize| (from..from + 6).map(|r| text(e, s1, &format!("{c}{r}"))).collect::<Vec<_>>();
    assert_eq!(col(&mut e, "B", 3), ["1", "2", "3", "4", "5", "6"]);
    assert_eq!(text(&mut e, s1, "D10"), "=B3:B8 sum", "the range still covers the block");
    assert_eq!(text(&mut e, s1, "E10"), "=C3", "a single reference follows its row");
    let ins = e.insert_rows_edit(s1, 0, 2);
    e.apply(ins);
    let redo = e.apply(undo);
    assert_eq!(col(&mut e, "B", 5), ["3", "1", "2", "6", "5", "4"]);
    assert_eq!(col(&mut e, "C", 5), ["=B5 10 *", "=B6 10 *", "=B7 10 *", "=B8 10 *", "=B9 10 *", "=B10 10 *"]);
    assert_eq!(text(&mut e, s1, "D12"), "=B5:B10 sum");
    assert_eq!(text(&mut e, s1, "E12"), "=C6");
    assert_eq!(col(&mut e, "B", 1)[..2], ["", ""]);
    e.apply(redo);
    assert_eq!(col(&mut e, "B", 5), ["1", "2", "3", "4", "5", "6"]);
}

#[test]
fn an_edit_to_a_deleted_row_comes_back_with_it() {
    let mut e = Engine::new(base());
    let s1 = sid(&e, "Sheet1");
    let b5 = key(&mut e, s1, "B5");
    let del = e.delete_rows_edit(s1, 4, 1);
    let restore = e.apply(del);
    assert_eq!(show(&mut e, s1, "D9"), "19");
    // someone who hadn't seen the delete types into B5 (by id)
    e.apply(put_text(b5, "100"));
    assert_eq!(e.wb.pos(b5), None, "still hidden");
    assert_eq!(show(&mut e, s1, "D9"), "19", "a hidden cell counts for nothing");
    e.apply(restore);
    assert_eq!(text(&mut e, s1, "B5"), "100");
    assert_eq!(show(&mut e, s1, "C5"), "1,000");
    assert_eq!(show(&mut e, s1, "D10"), "119");
}

#[test]
fn an_edit_to_a_deleted_column_comes_back_with_it() {
    let mut e = Engine::new(base());
    let s1 = sid(&e, "Sheet1");
    let c4 = key(&mut e, s1, "C4");
    let del = e.delete_cols_edit(s1, 2, 1);
    let restore = e.apply(del);
    assert_eq!(text(&mut e, s1, "D10"), "=#ref!", "E10 moved left");
    assert!(show(&mut e, s1, "D10").starts_with("ERR"));
    e.apply(put_text(c4, "8"));
    e.apply(restore);
    assert_eq!(text(&mut e, s1, "C4"), "8");
    assert_eq!(text(&mut e, s1, "E10"), "=C4");
    assert_eq!(show(&mut e, s1, "E10"), "8");
}

#[test]
fn an_edit_to_a_deleted_sheet_comes_back_with_it() {
    let mut e = Engine::new(base());
    let (s1, d) = (sid(&e, "Sheet1"), sid(&e, "data"));
    let a1 = key(&mut e, d, "A1");
    let del = e.delete_sheet_edit(d).unwrap();
    let restore = e.apply(del);
    assert!(e.wb.sheet(d).is_none());
    assert_eq!(show(&mut e, s1, "F10"), "ERR reference to a deleted sheet");
    e.apply(put_text(a1, "9"));
    assert_eq!(show(&mut e, s1, "F10"), "ERR reference to a deleted sheet");
    e.apply(restore);
    assert_eq!(e.wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Sheet1", "data", "units"]);
    assert_eq!(show(&mut e, s1, "F10"), "20");
}

#[test]
fn deleted_content_survives_save_and_load() {
    let mut e = Engine::new(base());
    let (s1, d) = (sid(&e, "Sheet1"), sid(&e, "data"));
    let del = e.delete_rows_edit(s1, 4, 1);
    let restore_row = e.apply(del);
    let del = e.delete_sheet_edit(d).unwrap();
    let restore_sheet = e.apply(del);
    let json = serde_json::to_string(&e.wb).unwrap();
    let mut e2 = Engine::new(serde_json::from_str(&json).unwrap());
    assert_eq!(snap(&e2), snap(&e));
    e2.apply(restore_sheet);
    e2.apply(restore_row);
    assert_eq!(snap(&e2), snap(&Engine::new(base())));
}

#[test]
fn names_undo_per_name() {
    let mut e = Engine::new(base());
    let s1 = sid(&e, "Sheet1");
    let (b4, b5) = (key(&mut e, s1, "B4"), key(&mut e, s1, "B5"));
    let undo_x = e.set_name("x", Some(b4), false).unwrap();
    e.set_name("y", Some(b5), false).unwrap();
    // undoing x leaves y (and top) alone: undo replaces one name, not the table
    e.apply(undo_x);
    assert_eq!(e.wb.names.keys().collect::<Vec<_>>(), ["top", "y"]);
    // renaming a cell's name is a remove and a set, undone together
    let undo_z = e.set_name("z", Some(b5), true).unwrap();
    assert_eq!(e.wb.names.keys().collect::<Vec<_>>(), ["top", "z"]);
    let redo_z = e.apply(undo_z);
    assert_eq!(e.wb.names.keys().collect::<Vec<_>>(), ["top", "y"]);
    assert!(!e.wb.names["y"].input);
    e.apply(redo_z);
    assert_eq!(e.wb.names.keys().collect::<Vec<_>>(), ["top", "z"]);
    assert!(e.wb.names["z"].input);
    // a name set while another was changed elsewhere since
    let undo_rm = e.set_name("top", None, false).unwrap();
    e.set_name("w", Some(b4), false).unwrap();
    e.apply(undo_rm);
    assert_eq!(e.wb.names.keys().collect::<Vec<_>>(), ["top", "w", "z"]);
}

#[test]
fn sizes_are_edits() {
    let mut e = Engine::new(base());
    let s1 = sid(&e, "Sheet1");
    let col = e.wb.sheet(s1).unwrap().cols.get(3).unwrap();
    let evals = e.last_eval_count;
    let undo = e.apply(Edit::ColWidth { sheet: s1, col, width: Some(200.0) });
    assert_eq!(e.wb.sheet(s1).unwrap().col_widths.get(&col), Some(&200.0));
    assert_eq!(e.last_eval_count, evals, "a resize recalculates nothing");
    let redo = e.apply(undo);
    assert_eq!(e.wb.sheet(s1).unwrap().col_widths.get(&col), None, "back to the default, not to an explicit default width");
    e.apply(redo);
    assert_eq!(e.wb.sheet(s1).unwrap().col_widths.get(&col), Some(&200.0));
}
