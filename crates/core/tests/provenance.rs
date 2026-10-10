//! Per-element provenance (which cell a value came from) is carried through the words that only
//! rearrange values, permuted or sliced with the data, and dropped by the words that compute.

use wbs_core::a1;
use wbs_core::engine::Engine;
use wbs_core::ids::CellKey;
use wbs_core::stdlib::default_workbook;
use wbs_core::value::{Prov, Value};

fn key(e: &Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheets[0].key(a.row, a.col).unwrap()
}

/// A1:A3 and B1:B3 are literals, C1 spills a computed column, D1 a computed 2×2 array.
fn sheet() -> Engine {
    let mut e = Engine::new(default_workbook());
    for (at, text) in [("A1", "1"), ("A2", "2"), ("G1", "2 [km]"), ("A3", "3"), ("B1", "10"), ("B2", "20"), ("B3", "30"), ("C1", "=A1:A3 2 *"), ("D1", "=1 2 couple 3 4 couple couple")] {
        let k = key(&e, at);
        e.set_text(k, text);
    }
    e
}

/// The provenance of each element of `text`'s value, evaluated on the sheet, as `A1` / `C1#2` / `-`.
fn provs(e: &Engine, text: &str) -> Vec<String> {
    let n = match e.eval_scratch(&format!("={text}"), e.wb.sheets[0].id).result {
        Ok(Value::Num(n)) => n,
        other => panic!("{text}: {other:?}"),
    };
    let name = |k: CellKey| {
        let (r, c) = e.wb.pos(k).unwrap();
        a1::cell_name(r, c)
    };
    (0..n.len())
        .map(|i| match n.prov.get(i) {
            Prov::None => "-".to_string(),
            Prov::Literal(k) => name(k),
            Prov::Derived(k, j) => format!("{}#{j}", name(k)),
        })
        .collect()
}

#[test]
fn join_keeps_both_sides() {
    let e = sheet();
    assert_eq!(provs(&e, "A1:A3 B1:B2 join"), ["A1", "A2", "A3", "B1", "B2"]);
    assert_eq!(provs(&e, "A1 B1 join"), ["A1", "B1"]);
    assert_eq!(provs(&e, "C1 A1 join"), ["C1#0", "C1#1", "C1#2", "A1"]);
    // a number typed in the program has no cell
    assert_eq!(provs(&e, "A1:A2 5 join"), ["A1", "A2", "-"]);
}

#[test]
fn couple_keeps_both_rows() {
    let e = sheet();
    assert_eq!(provs(&e, "A1:A3 B1:B3 couple"), ["A1", "A2", "A3", "B1", "B2", "B3"]);
}

#[test]
fn rev_reverses_it() {
    let e = sheet();
    assert_eq!(provs(&e, "A1:A3 rev"), ["A3", "A2", "A1"]);
    assert_eq!(provs(&e, "C1 rev"), ["C1#2", "C1#1", "C1#0"]);
    assert_eq!(provs(&e, "D1 rev"), ["D1#2", "D1#3", "D1#0", "D1#1"]);
}

#[test]
fn first_last_and_pick_select_it() {
    let e = sheet();
    assert_eq!(provs(&e, "B1:B3 first"), ["B1"]);
    assert_eq!(provs(&e, "B1:B3 last"), ["B3"]);
    assert_eq!(provs(&e, "B1:B3 1 pick"), ["B2"]);
    assert_eq!(provs(&e, "C1 2 pick"), ["C1#2"]);
    assert_eq!(provs(&e, "D1 last"), ["D1#2", "D1#3"]);
}

#[test]
fn transpose_permutes_it() {
    let e = sheet();
    assert_eq!(provs(&e, "D1 transpose"), ["D1#0", "D1#2", "D1#1", "D1#3"]);
    assert_eq!(provs(&e, "A1:A3 B1:B3 couple transpose"), ["A1", "B1", "A2", "B2", "A3", "B3"]);
    assert_eq!(provs(&e, "C1 transpose"), ["C1#0", "C1#1", "C1#2"]);
}

#[test]
fn to_unit_keeps_it() {
    let e = sheet();
    assert_eq!(provs(&e, "G1 to[m]"), ["G1"]);
    assert_eq!(provs(&e, "C1 [km] to[m]"), ["-", "-", "-"], "applying a unit computes");
}

#[test]
fn computing_words_drop_it() {
    let e = sheet();
    for text in ["A1:A3 1 +", "A1:A3 2 *", "A1:A3 neg", "A1:A3 B1:B3 max", "A1:A3 \\+", "A1:A3 dup + 2 /"] {
        assert!(provs(&e, text).iter().all(|p| p == "-"), "{text}");
    }
    for text in ["A1:A3 sum", "A1 /+", "A1:A3 /+", "A1:A3 mean"] {
        assert_eq!(provs(&e, text), ["-"], "{text}");
    }
    // rearranging a computed result keeps nothing to trace
    assert_eq!(provs(&e, "A1:A3 1 + rev"), ["-", "-", "-"]);
}

#[test]
fn chart_points_built_with_rearranging_words_are_traced() {
    let mut e = sheet();
    let k = key(&e, "A20");
    e.set_text(k, "=A1:A3 B1:B2 join rev A1:A3 B1:B2 join rev line");
    let Some(Ok(Value::Chart(c))) = e.result(k) else { panic!() };
    assert_eq!(c.layers[0].ys.prov.get(0), Prov::Literal(key(&e, "B2")));
    assert_eq!(c.layers[0].ys.prov.get(4), Prov::Literal(key(&e, "A1")));
}
