//! SPIKE (docs/design/automerge.md): the Automerge 0.12 merge behaviours the
//! design note relies on. Each test names the section of the note it backs.
//! Throwaway evidence, not product code.

use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, Value, ROOT};

fn peer(base: &mut AutoCommit, actor: u8) -> AutoCommit {
    base.fork().with_actor(ActorId::from(vec![actor; 16]))
}

fn map(doc: &AutoCommit, at: &ObjId, key: &str) -> ObjId {
    match doc.get(at, key).unwrap() {
        Some((Value::Object(_), id)) => id,
        other => panic!("{key} is not an object: {other:?}"),
    }
}

fn str_at(doc: &AutoCommit, obj: &ObjId, key: &str) -> Option<String> {
    match doc.get(obj, key).unwrap() {
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Str(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn list_strs(doc: &AutoCommit, list: &ObjId) -> Vec<String> {
    (0..doc.length(list))
        .map(|i| match doc.get(list, i).unwrap() {
            Some((Value::Scalar(s), _)) => s.as_str().unwrap().to_string(),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn merged(a: &mut AutoCommit, b: &mut AutoCommit) {
    a.merge(b).unwrap();
    b.merge(a).unwrap();
    assert_eq!(a.get_heads(), b.get_heads());
}

/// §4 "same cell, same time": one value wins on every peer, and the loser is still readable.
#[test]
fn same_cell_conflict_is_deterministic_and_visible() {
    let mut base = AutoCommit::new();
    let cells = base.put_object(ROOT, "cells", ObjType::Map).unwrap();
    base.put(&cells, "r1:c1", "=1").unwrap();
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    a.put(&cells, "r1:c1", "=2").unwrap();
    b.put(&cells, "r1:c1", "=3").unwrap();
    merged(&mut a, &mut b);
    assert_eq!(str_at(&a, &cells, "r1:c1"), str_at(&b, &cells, "r1:c1"));
    let all = a.get_all(&cells, "r1:c1").unwrap();
    assert_eq!(all.len(), 2, "both concurrent values are kept as a conflict");
    // a later write by anyone who has seen both resolves the conflict
    a.put(&cells, "r1:c1", "=4").unwrap();
    assert_eq!(a.get_all(&cells, "r1:c1").unwrap().len(), 1);
}

/// §4 "row deleted while someone edits a cell in it": a map delete only removes the
/// values it saw, so a concurrent put survives. Hence: rows are hidden, cells are kept.
#[test]
fn put_survives_concurrent_delete() {
    let mut base = AutoCommit::new();
    let cells = base.put_object(ROOT, "cells", ObjType::Map).unwrap();
    base.put(&cells, "r1:c1", "old").unwrap();
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    a.delete(&cells, "r1:c1").unwrap();
    b.put(&cells, "r1:c1", "new").unwrap();
    merged(&mut a, &mut b);
    assert_eq!(str_at(&a, &cells, "r1:c1").as_deref(), Some("new"));
}

/// §2: concurrent runs appended at the same place in an Automerge list stay contiguous
/// (no interleaving of the two runs).
#[test]
fn list_runs_do_not_interleave() {
    let mut base = AutoCommit::new();
    let rows = base.put_object(ROOT, "rows", ObjType::List).unwrap();
    base.insert(&rows, 0, "r0").unwrap();
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    for (i, id) in ["a1", "a2", "a3"].iter().enumerate() {
        a.insert(&rows, 1 + i, *id).unwrap();
    }
    for (i, id) in ["b1", "b2", "b3"].iter().enumerate() {
        b.insert(&rows, 1 + i, *id).unwrap();
    }
    merged(&mut a, &mut b);
    let got = list_strs(&a, &rows).join(" ");
    assert!(got == "r0 a1 a2 a3 b1 b2 b3" || got == "r0 b1 b2 b3 a1 a2 a3", "{got}");
}

/// §2 why row order is NOT an Automerge list: there is no move op, and a sort written as
/// per-slot puts can merge into a list that duplicates one row and loses another.
#[test]
fn sorting_a_list_by_slot_puts_can_lose_a_row() {
    let mut base = AutoCommit::new();
    let rows = base.put_object(ROOT, "rows", ObjType::List).unwrap();
    for (i, id) in ["r1", "r2", "r3", "r4"].iter().enumerate() {
        base.insert(&rows, i, *id).unwrap();
    }
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    // A sorts slots 0..=2 to r3 r1 r2; B sorts slots 1..=3 to r4 r3 r2
    for (i, id) in ["r3", "r1", "r2"].iter().enumerate() {
        a.put(&rows, i, *id).unwrap();
    }
    for (i, id) in ["r4", "r3", "r2"].iter().enumerate() {
        b.put(&rows, 1 + i, *id).unwrap();
    }
    merged(&mut a, &mut b);
    let mut got = list_strs(&a, &rows);
    let shown = got.join(" ");
    got.sort();
    got.dedup();
    assert!(got.len() < 4, "expected a duplicate/lost row, got {shown}");
}

/// §2 the proposed axis: a map row-id → position key. Concurrent sorts can only change
/// which key each row has, so the result is always a permutation; when both sorts cover
/// the same rows and write them in id order, one sort wins whole.
#[test]
fn position_keys_keep_concurrent_sorts_a_permutation() {
    let ids = ["r1", "r2", "r3", "r4"];
    let mut base = AutoCommit::new();
    let rows = base.put_object(ROOT, "rows", ObjType::Map).unwrap();
    for (i, id) in ids.iter().enumerate() {
        let r = base.put_object(&rows, *id, ObjType::Map).unwrap();
        base.put(&r, "pos", format!("a{i}")).unwrap();
    }
    let order = |d: &AutoCommit| {
        let rows = map(d, &ROOT, "rows");
        let mut v: Vec<(String, String)> =
            ids.iter().map(|id| (str_at(d, &map(d, &rows, id), "pos").unwrap(), id.to_string())).collect();
        v.sort();
        v.into_iter().map(|(_, id)| id).collect::<Vec<_>>()
    };
    // a sort assigns fresh keys under one random prefix, writing rows in id order
    let sort = |d: &mut AutoCommit, new_order: &[&str], prefix: &str| {
        let rows = map(d, &ROOT, "rows");
        let mut by_id: Vec<(usize, &str)> = new_order.iter().copied().enumerate().collect();
        by_id.sort_by_key(|(_, id)| *id);
        for (slot, id) in by_id {
            let r = map(d, &rows, id);
            d.put(&r, "pos", format!("a0{prefix}{slot}")).unwrap();
        }
    };
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    sort(&mut a, &["r4", "r3", "r2", "r1"], "x");
    sort(&mut b, &["r2", "r1", "r4", "r3"], "y");
    merged(&mut a, &mut b);
    let got = order(&a);
    assert_eq!(got, order(&b));
    assert!(got == ["r4", "r3", "r2", "r1"] || got == ["r2", "r1", "r4", "r3"], "one sort wins whole: {got:?}");

    // overlapping but different blocks: may mix, but never duplicates or loses a row
    let mut c = peer(&mut a, 3);
    let mut d = peer(&mut a, 4);
    sort(&mut c, &["r1", "r2", "r3"], "p"); // c re-sorts three rows
    sort(&mut d, &["r1", "r2", "r3", "r4"], "q"); // d re-sorts all four
    merged(&mut c, &mut d);
    let mut got = order(&c);
    got.sort();
    assert_eq!(got, ids);
}

/// §2 implicit rows: two peers materialising the same deterministic row id with the same
/// position key converge to one row (the "conflict" is between equal values).
#[test]
fn materialising_the_same_virtual_row_is_idempotent() {
    let mut base = AutoCommit::new();
    let pos = base.put_object(ROOT, "row_pos", ObjType::Map).unwrap();
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    for d in [&mut a, &mut b] {
        d.put(&pos, "tail:57", "t0000057").unwrap();
    }
    merged(&mut a, &mut b);
    assert_eq!(a.keys(&pos).count(), 1);
    assert_eq!(str_at(&a, &pos, "tail:57").as_deref(), Some("t0000057"));
}

/// §2 why per-row/per-cell data is flat scalars, not nested maps: two peers creating the
/// same nested object concurrently get two objects, and the loser's fields disappear.
#[test]
fn concurrently_created_nested_objects_lose_fields() {
    let mut base = AutoCommit::new();
    let rows = base.put_object(ROOT, "rows", ObjType::Map).unwrap();
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    let ra = a.put_object(&rows, "tail:57", ObjType::Map).unwrap();
    a.put(&ra, "height", 40.0).unwrap();
    let rb = b.put_object(&rows, "tail:57", ObjType::Map).unwrap();
    b.put(&rb, "dead", true).unwrap();
    merged(&mut a, &mut b);
    let winner = map(&a, &rows, "tail:57");
    let fields: Vec<String> = a.keys(&winner).collect();
    assert_eq!(fields.len(), 1, "only the winning object's field is visible: {fields:?}");
}

/// §5 local undo: reverting your own change is a new change, and it reverts only the
/// cell you changed — a concurrent edit by someone else elsewhere stays.
#[test]
fn undo_is_a_new_change() {
    let mut base = AutoCommit::new();
    let cells = base.put_object(ROOT, "cells", ObjType::Map).unwrap();
    base.put(&cells, "x", "1").unwrap();
    let mut a = peer(&mut base, 1);
    let mut b = peer(&mut base, 2);
    a.put(&cells, "x", "2").unwrap();
    b.put(&cells, "y", "bob").unwrap();
    merged(&mut a, &mut b);
    a.put(&cells, "x", "1").unwrap(); // A's undo: write back what A's inverse says
    merged(&mut a, &mut b);
    assert_eq!(str_at(&b, &cells, "x").as_deref(), Some("1"));
    assert_eq!(str_at(&b, &cells, "y").as_deref(), Some("bob"));
}
