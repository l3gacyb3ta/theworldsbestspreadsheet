use wbs_core::a1;
use wbs_core::engine::{Engine, ErrKind, Shown};
use wbs_core::ids::CellKey;
use wbs_core::stdlib::default_workbook;

fn eng() -> Engine {
    Engine::new(default_workbook())
}

fn key(e: &Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    let s = &e.wb.sheets[0];
    s.key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, r: &str, text: &str) {
    let a = a1::parse_ref(r).unwrap();
    let k = e.wb.sheets[0].key(a.row, a.col).unwrap();
    e.set_text(k, text);
}

fn show(e: &Engine, r: &str) -> String {
    let k = key(e, r);
    match e.shown(k) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

#[test]
fn arithmetic_and_refs() {
    let mut e = eng();
    set(&mut e, "A1", "5");
    set(&mut e, "A2", "=A1 2 *");
    assert_eq!(show(&e, "A2"), "10");
    set(&mut e, "A1", "7");
    assert_eq!(show(&e, "A2"), "14");
    set(&mut e, "A3", "=1 2 3");
    assert_eq!(show(&e, "A3"), "ERR 3 values left on stack");
    set(&mut e, "A4", "=1 +");
    assert!(show(&e, "A4").starts_with("ERR + needs"));
}

#[test]
fn ranges_reduce_scan() {
    let mut e = eng();
    for (i, v) in ["1", "2", "3", "4"].iter().enumerate() {
        set(&mut e, &format!("A{}", i + 1), v);
        set(&mut e, &format!("B{}", i + 1), "10");
    }
    set(&mut e, "C1", "=A1:A4 /+");
    assert_eq!(show(&e, "C1"), "10");
    set(&mut e, "D1", "=A1:A4 B1:B4 * sum");
    assert_eq!(show(&e, "D1"), "100");
    set(&mut e, "E1", "=A1:A4 \\+");
    assert_eq!(show(&e, "E1"), "1");
    assert_eq!(show(&e, "E4"), "10");
    set(&mut e, "F1", "=A1:A4 /max");
    assert_eq!(show(&e, "F1"), "4");
    set(&mut e, "G1", "=A1:A4 mean");
    assert_eq!(show(&e, "G1"), "2.5");
}

#[test]
fn units_basic() {
    let mut e = eng();
    set(&mut e, "A1", "=5 [km] 300 [m] +");
    assert_eq!(show(&e, "A1"), "5.3 km");
    set(&mut e, "A2", "=100 [km] 2 [h] /");
    assert_eq!(show(&e, "A2"), "50 km/h");
    set(&mut e, "A3", "=A2 to[m/s]");
    assert_eq!(show(&e, "A3"), "13.888889 m/s");
    set(&mut e, "A4", "=9.81 [m/s^2] 3 [s] *");
    assert_eq!(show(&e, "A4"), "29.43 m/s");
    set(&mut e, "A5", "=2 [kg] A4 * 3 [s] /");
    assert_eq!(show(&e, "A5"), "19.62 kg*m/s^2");
    set(&mut e, "A6", "=A5 to[N]");
    assert_eq!(show(&e, "A6"), "19.62 N");
    set(&mut e, "A7", "=4 [m^2] sqrt");
    assert_eq!(show(&e, "A7"), "2 m");
}

#[test]
fn unit_errors_reported_where_they_occur() {
    let mut e = eng();
    set(&mut e, "A1", "5 [m]");
    set(&mut e, "A2", "3 [s]");
    set(&mut e, "A3", "=A1 A2 +");
    set(&mut e, "A4", "=A3 2 *");
    let k3 = key(&e, "A3");
    match e.shown(k3) {
        Shown::Error(err) => {
            assert_eq!(err.kind, ErrKind::Local);
            assert!(err.msg.contains("length vs time"), "{}", err.msg);
            assert_eq!(err.span, Some(7..8)); // the `+`
        }
        _ => panic!("expected error"),
    }
    let k4 = key(&e, "A4");
    match e.shown(k4) {
        Shown::Error(err) => assert_eq!(err.kind, ErrKind::Upstream(k3)),
        _ => panic!("expected upstream error"),
    }
    set(&mut e, "A5", "=A1 to[s]");
    assert!(show(&e, "A5").starts_with("ERR can't show"));
}

#[test]
fn temperatures_and_dates() {
    let mut e = eng();
    set(&mut e, "A1", "=20 [°C] 5 [Δ°C] +");
    assert_eq!(show(&e, "A1"), "25 °C");
    set(&mut e, "A2", "=20 [°C] 10 [°C] +");
    assert!(show(&e, "A2").starts_with("ERR can't add two absolute"));
    set(&mut e, "A3", "=30 [°C] 20 [°C] -");
    assert_eq!(show(&e, "A3"), "10 Δ°C");
    set(&mut e, "A4", "=212 [°F] to[°C]");
    assert_eq!(show(&e, "A4"), "100 °C");
    set(&mut e, "A5", "2026-10-08");
    assert_eq!(show(&e, "A5"), "2026-10-08");
    set(&mut e, "A6", "=A5 30 [day] +");
    assert_eq!(show(&e, "A6"), "2026-11-07");
    set(&mut e, "A7", "=A6 A5 -");
    assert_eq!(show(&e, "A7"), "30 day");
}

#[test]
fn user_units_and_dims() {
    let mut e = eng();
    set(&mut e, "A1", "dim widgets");
    set(&mut e, "A2", "base [widget] widgets");
    set(&mut e, "B1", "=12 [USD/widget] 100 [widget] *");
    assert_eq!(show(&e, "B1"), "1,200 USD");
    set(&mut e, "B2", "=1 [widget] 1 [USD] +");
    assert!(show(&e, "B2").contains("widgets vs currency"), "{}", show(&e, "B2"));
    // exchange rates are inputs in the graph
    set(&mut e, "C1", "1.10");
    set(&mut e, "C2", "[XEU] = C1 [USD]");
    set(&mut e, "C3", "=100 [XEU] to[USD]");
    assert_eq!(show(&e, "C3"), "110 USD");
    set(&mut e, "C1", "1.20");
    assert_eq!(show(&e, "C3"), "120 USD");
}

#[test]
fn words() {
    let mut e = eng();
    set(&mut e, "A1", ": sq dup * ;");
    set(&mut e, "A2", "=3 sq");
    assert_eq!(show(&e, "A2"), "9");
    set(&mut e, "A1", ": sq dup dup * * ;");
    assert_eq!(show(&e, "A2"), "27");
    set(&mut e, "B1", ": npv { rate cfs } cfs 1 rate + cfs len range ^ / sum ;");
    set(&mut e, "B2", "=0.1 100 110 couple first 1 join 0 * 100 + npv");
    // cfs = [100, 100] → 100 + 100/1.1
    assert_eq!(show(&e, "B2"), "190.90909");
    set(&mut e, "B3", "=sq");
    assert!(show(&e, "B3").contains("in sq"), "{}", show(&e, "B3"));
}

#[test]
fn spill() {
    let mut e = eng();
    set(&mut e, "A1", "=5 range");
    assert_eq!(show(&e, "A1"), "0");
    assert_eq!(show(&e, "A5"), "4");
    set(&mut e, "B1", "=A3 10 *");
    assert_eq!(show(&e, "B1"), "20");
    set(&mut e, "B2", "=A1 sum");
    assert_eq!(show(&e, "B2"), "10");
    set(&mut e, "A1", "=6 range 1 +");
    assert_eq!(show(&e, "B1"), "30");
    assert_eq!(show(&e, "B2"), "21");
    // blocked
    set(&mut e, "A4", "x");
    assert!(show(&e, "A1").starts_with("ERR #spill blocked: A4"), "{}", show(&e, "A1"));
    assert_eq!(show(&e, "A5"), "<empty>");
    assert!(show(&e, "B1").starts_with("ERR"));
    set(&mut e, "A4", "");
    assert_eq!(show(&e, "A5"), "5");
    assert_eq!(show(&e, "B1"), "30");
    // 2-D
    set(&mut e, "D1", "=1 2 join 3 4 join couple");
    assert_eq!(show(&e, "E2"), "4");
}

#[test]
fn cycles() {
    let mut e = eng();
    set(&mut e, "A1", "=B1 1 +");
    set(&mut e, "B1", "=C1 1 +");
    set(&mut e, "C1", "=A1 1 +");
    for c in ["A1", "B1", "C1"] {
        let s = show(&e, c);
        assert!(s.starts_with("ERR cycle:"), "{c}: {s}");
    }
    assert_eq!(e.cycles.len(), 1);
    assert_eq!(e.cycles[0].len(), 3);
    set(&mut e, "C1", "1");
    assert_eq!(show(&e, "A1"), "3");
    assert!(e.cycles.is_empty());
}

#[test]
fn incremental() {
    let mut e = eng();
    set(&mut e, "A1", "1");
    for i in 2..=200 {
        set(&mut e, &format!("A{i}"), &format!("=A{} 1 +", i - 1));
    }
    set(&mut e, "B1", "7");
    set(&mut e, "B2", "=B1 2 *");
    assert_eq!(show(&e, "A200"), "200");
    set(&mut e, "A150", "0");
    assert_eq!(show(&e, "A200"), "50");
    assert_eq!(e.last_eval_count, 51);
    set(&mut e, "B1", "8");
    assert_eq!(e.last_eval_count, 2);
}

#[test]
fn structure_edits_keep_refs() {
    let mut e = eng();
    set(&mut e, "A1", "1");
    set(&mut e, "A2", "2");
    set(&mut e, "A3", "3");
    set(&mut e, "B1", "=A1:A3 sum A2 +");
    let sid = e.wb.sheets[0].id;
    let before = e.wb.sheets[0].rows.get(1);
    let undo = e.apply(wbs_core::Edit::InsertRows { sheet: sid, before, ids: vec![wbs_core::ids::RowId(42)] });
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "=A1:A4 sum A3 +");
    set(&mut e, "A2", "100");
    assert_eq!(show(&e, "B1"), "108");
    e.apply(wbs_core::Edit::Cells(vec![(key(&e, "A2"), None)]));
    e.apply(undo);
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "=A1:A3 sum A2 +");
    // delete last row of the range: it shrinks
    let inv = e.apply(e.delete_rows_edit(sid, 2, 1));
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "=A1:A2 sum A2 +");
    assert_eq!(show(&e, "B1"), "5");
    e.apply(inv);
    assert_eq!(show(&e, "B1"), "8");
}

#[test]
fn names() {
    let mut e = eng();
    set(&mut e, "A1", "0.05");
    let k = key(&e, "A1");
    e.set_name("growth", Some(k), true).unwrap();
    set(&mut e, "B1", "=100 1 growth + *");
    assert_eq!(show(&e, "B1"), "105");
    assert!(e.set_name("sum", Some(k), true).is_err());
    assert!(e.set_name("B2", Some(k), true).is_err());
}

#[test]
fn charts() {
    let mut e = eng();
    set(&mut e, "A1", "1");
    set(&mut e, "A2", "2");
    set(&mut e, "A3", "3");
    set(&mut e, "B1", "=A1:A3 dup dup * line \"squares\" title");
    let k = key(&e, "B1");
    match e.shown(k) {
        Shown::Value { value: wbs_core::value::Value::Chart(c), .. } => {
            assert_eq!(c.title.as_deref(), Some("squares"));
            assert_eq!(&*c.layers[0].ys.data, &[1.0, 4.0, 9.0]);
        }
        _ => panic!("expected chart, got {}", show(&e, "B1")),
    }
    assert_eq!(e.spill_size(k), Some((14, 6)));
    set(&mut e, "C1", "=A1:A3 A1:A3 scatter");
    // C1 is inside B1's chart region: B1 is now blocked
    assert!(show(&e, "B1").starts_with("ERR #spill blocked"));
}

#[test]
fn percent_is_absorbed_by_dimensioned_quantities() {
    let mut e = eng();
    set(&mut e, "A1", "=4 [%] 100 [USD] *");
    assert_eq!(show(&e, "A1"), "4 USD");
    set(&mut e, "A2", "=5 [%] 2 *");
    assert_eq!(show(&e, "A2"), "10 %");
    set(&mut e, "A3", "=1 5 [%] +");
    assert_eq!(show(&e, "A3"), "1.05");
}

#[test]
fn save_load_roundtrip() {
    let mut e = eng();
    set(&mut e, "A1", "2");
    set(&mut e, "B1", "=A1 3 [m] *");
    let k = key(&e, "A1");
    e.set_name("x", Some(k), true).unwrap();
    let c = e.wb.sheets[0].cols.get(1).unwrap();
    e.wb.sheets[0].col_widths.insert(c, 150.0);
    e.apply(e.delete_rows_edit(e.wb.sheets[0].id, 5, 1));
    let json = serde_json::to_string(&e.wb).unwrap();
    let wb2: wbs_core::model::Workbook = serde_json::from_str(&json).unwrap();
    let e2 = Engine::new(wb2);
    assert_eq!(show(&e2, "B1"), "6 m");
    assert_eq!(e2.wb.names.get("x").unwrap().cell, k);
    assert_eq!(e2.wb.sheets[0].col_widths.get(&c), Some(&150.0));
}

/// Scrubbing must stay interactive on sheets of a few thousand cells.
#[test]
fn scrub_recalc_is_fast() {
    let mut e = eng();
    set(&mut e, "A1", "1.05");
    // 3000 formula cells: 1000 rows × (growth, units, running reference)
    for r in 2..=1001 {
        set(&mut e, &format!("A{r}"), &format!("=A{} A1 *", r - 1));
        set(&mut e, &format!("B{r}"), &format!("=A{r} 100 [USD] *"));
        set(&mut e, &format!("C{r}"), &format!("=B{r} 0.8 * to[EUR]"));
    }
    set(&mut e, "D1", "=C2:C1001 sum");
    // Time each step and judge the fastest: a busy machine (parallel builds, a shared
    // CI runner) slows some steps down, but the best step still shows the real cost.
    let steps: Vec<f64> = (0..30)
        .map(|i| {
            let t = std::time::Instant::now();
            set(&mut e, "A1", &format!("1.0{}", i % 10));
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let best = steps.iter().cloned().fold(f64::INFINITY, f64::min);
    let mean = steps.iter().sum::<f64>() / steps.len() as f64;
    eprintln!("recalc of {} cells: best {best:.2} ms, mean {mean:.2} ms per scrub step", e.last_eval_count);
    assert!(e.last_eval_count >= 3000);
    assert!(best < 16.0, "{best} ms per step (best of 30) is too slow for interactive scrubbing");
}

/// Two spills that would overlap are both blocked, whichever came first.
#[test]
fn overlapping_spills_block_each_other() {
    let outcome = |first: (&str, &str), second: (&str, &str)| {
        let mut e = eng();
        set(&mut e, first.0, first.1);
        set(&mut e, second.0, second.1);
        ["A1", "A2", "A3", "B1", "B2", "B3", "C1", "C2", "C3"].map(|c| show(&e, c))
    };
    // A1 wants A1:A3, A2 wants A2:B3: A2 is in A1's way, and the regions overlap
    let (a, b) = (("A1", "=3 range"), ("A2", "=1 2 join 3 4 join couple"));
    let one = outcome(a, b);
    assert_eq!(one, outcome(b, a));
    assert_eq!(one[0], "ERR #spill blocked: A2 is in the way");
    assert_eq!(one[1], "ERR #spill blocked: overlaps the spill from A1");
    assert_eq!(one[2], "<empty>");
    assert_eq!(one[4], "<empty>");
    // B1 wants B1:C2, A2 wants A2:B3: they share B2, neither source is in the other's way
    let (a, b) = (("B1", "=1 2 join 3 4 join couple 3 *"), ("A2", "=1 2 join 3 4 join couple"));
    let one = outcome(a, b);
    assert_eq!(one, outcome(b, a));
    assert_eq!(one[3], "ERR #spill blocked: overlaps the spill from A2");
    assert_eq!(one[1], "ERR #spill blocked: overlaps the spill from B1");
    for i in [2, 4, 5, 6, 7] {
        assert_eq!(one[i], "<empty>");
    }
}

#[test]
fn overlapping_spills_unblock_when_one_shrinks() {
    let mut e = eng();
    set(&mut e, "Z1", "3");
    set(&mut e, "A1", "=Z1 range");
    set(&mut e, "A3", "=2 range 10 +");
    set(&mut e, "C1", "=A2 A4 +");
    assert!(show(&e, "A1").starts_with("ERR #spill blocked"));
    assert!(show(&e, "A3").starts_with("ERR #spill blocked"));
    assert!(show(&e, "C1").starts_with("ERR"));
    // A1 shrinks to two rows: no overlap, both spill
    let undo = e.set_text(key(&e, "Z1"), "2");
    assert_eq!(show(&e, "A2"), "1");
    assert_eq!(show(&e, "A4"), "11");
    assert_eq!(show(&e, "C1"), "12");
    // undo restores the blocked state, redo the unblocked one
    let redo = e.apply(undo);
    assert!(show(&e, "A1").starts_with("ERR #spill blocked"));
    assert!(show(&e, "A3").starts_with("ERR #spill blocked"));
    assert_eq!(show(&e, "A4"), "<empty>");
    assert!(show(&e, "C1").starts_with("ERR"));
    e.apply(redo);
    assert_eq!(show(&e, "A1"), "0");
    assert_eq!(show(&e, "A3"), "10");
    assert_eq!(show(&e, "C1"), "12");
    // clearing one of them unblocks the other
    set(&mut e, "Z1", "5");
    assert!(show(&e, "A3").starts_with("ERR #spill blocked"));
    set(&mut e, "A3", "");
    assert_eq!(show(&e, "A5"), "4");
}

#[test]
fn overlapping_spills_same_after_reload() {
    let mut e = eng();
    set(&mut e, "B2", "=2 range");
    set(&mut e, "A3", "=1 2 join 3 4 join couple");
    let cells = ["A3", "B2", "B3", "B4"];
    let before = cells.map(|c| show(&e, c));
    let json = serde_json::to_string(&e.wb).unwrap();
    let e2 = Engine::new(serde_json::from_str(&json).unwrap());
    assert_eq!(before, cells.map(|c| show(&e2, c)));
    assert_eq!(before[0], "ERR #spill blocked: overlaps the spill from B2");
    assert_eq!(before[1], "ERR #spill blocked: overlaps the spill from A3");
}

#[test]
fn range_with_gaps_skips_empty_cells() {
    let mut e = eng();
    set(&mut e, "A1", "1");
    set(&mut e, "A2", "2");
    set(&mut e, "A4", "4");
    set(&mut e, "B1", "=A1:A5 sum");
    match e.shown(key(&e, "B1")) {
        Shown::Error(err) => {
            // same message and place as before `?` existed
            assert_eq!(err.msg, "Sheet1!A3 is empty");
            assert_eq!(err.kind, ErrKind::Local);
            assert_eq!(err.span, Some(1..6));
        }
        _ => panic!("expected error"),
    }
    set(&mut e, "B2", "=A1:A5? sum");
    assert_eq!(show(&e, "B2"), "7");
    set(&mut e, "B3", "=A1:A5? len");
    assert_eq!(show(&e, "B3"), "3");
    // filling the gap updates it
    set(&mut e, "A3", "3");
    assert_eq!(show(&e, "B2"), "10");
    assert_eq!(show(&e, "B1"), "ERR Sheet1!A5 is empty");
    // the non-empty cells are still checked
    set(&mut e, "A5", "5 [m]");
    assert!(show(&e, "B2").contains("mixes units"), "{}", show(&e, "B2"));
    set(&mut e, "A5", "x");
    assert!(show(&e, "B2").contains("mixes text and numbers"), "{}", show(&e, "B2"));
    set(&mut e, "A5", "=1 0 couple 2 pick");
    assert!(show(&e, "B2").contains("A5 has an error"), "{}", show(&e, "B2"));
    // tables: non-empty cells row by row, units kept
    set(&mut e, "D1", "1 [m]");
    set(&mut e, "E1", "2 [m]");
    set(&mut e, "E2", "3 [m]");
    set(&mut e, "F1", "=D1:E2?");
    assert_eq!(show(&e, "F1"), "1 m");
    assert_eq!(show(&e, "F3"), "3 m");
    set(&mut e, "G1", "=C1:C9? sum");
    assert_eq!(show(&e, "G1"), "0");
    // `?` is for ranges only
    set(&mut e, "G2", "=A1? 1 +");
    match e.shown(key(&e, "G2")) {
        Shown::Error(err) => {
            assert!(err.msg.starts_with("? only applies to a range"), "{}", err.msg);
            assert_eq!(err.span, Some(1..4));
        }
        _ => panic!("expected error"),
    }
}

/// The `?` lives outside the range's stored piece, so it survives
/// rendering, row inserts, fill and copy.
#[test]
fn range_gaps_marker_round_trips() {
    let mut e = eng();
    set(&mut e, "A1", "1");
    set(&mut e, "A3", "3");
    set(&mut e, "B1", "=A1:A3? sum");
    set(&mut e, "C1", "=$A$1:$A$3? sum");
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "=A1:A3? sum");
    assert_eq!(show(&e, "B1"), "4");
    let sid = e.wb.sheets[0].id;
    let before = e.wb.sheets[0].rows.get(1);
    let undo = e.apply(wbs_core::Edit::InsertRows { sheet: sid, before, ids: vec![wbs_core::ids::RowId(77)] });
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "=A1:A4? sum");
    assert_eq!(e.wb.cell_text(key(&e, "C1")), "=$A$1:$A$4? sum");
    assert_eq!(show(&e, "B1"), "4");
    e.apply(undo);
    assert_eq!(e.wb.cell_text(key(&e, "B1")), "=A1:A3? sum");
    // fill down one row: relative range shifts, `?` stays
    let ed = wbs_core::ops::fill(&mut e, wbs_core::ops::Rect::cell(sid, 0, 1), wbs_core::ops::Rect::span(sid, (0, 1), (1, 1)));
    e.apply(ed);
    assert_eq!(e.wb.cell_text(key(&e, "B2")), "=A2:A4? sum");
    assert_eq!(show(&e, "B2"), "3");
    // copy/paste
    let clip = wbs_core::ops::copy(&e, wbs_core::ops::Rect::cell(sid, 0, 2));
    let ed = wbs_core::ops::paste(&mut e, &clip, sid, (4, 3));
    e.apply(ed);
    assert_eq!(e.wb.cell_text(key(&e, "D5")), "=$A$1:$A$3? sum");
    // other sheets
    set(&mut e, "E1", "=Sheet1!A1:A3? sum");
    assert_eq!(e.wb.cell_text(key(&e, "E1")), "=Sheet1!A1:A3? sum");
    assert_eq!(show(&e, "E1"), "4");
}

/// Recalc plans are reused while the same cells change and the graph doesn't.
/// Interleave scrubs that reuse a plan with edits that change the graph (new
/// dependents, changed references, spills growing, blocked and unblocked) and
/// check every displayed value against an engine rebuilt from scratch.
#[test]
fn cached_recalc_plans_stay_correct() {
    let mut e = eng();
    set(&mut e, "A1", "2");
    set(&mut e, "A2", "=A1 3 *");
    set(&mut e, "A3", "=A2 A1 +");
    set(&mut e, "B1", "=3 range A1 *"); // fixed-size spill: scrubbing A1 reuses the plan
    set(&mut e, "C1", "=B1 sum");
    set(&mut e, "C2", "=B2 10 *");
    let check = |e: &Engine, step: &str| {
        let fresh = Engine::new(serde_json::from_str(&serde_json::to_string(&e.wb).unwrap()).unwrap());
        for r in ["A1", "A2", "A3", "B1", "B2", "B3", "C1", "C2", "C3", "D1", "F1", "F2", "F3", "F4", "F5"] {
            assert_eq!(show(e, r), show(&fresh, r), "{r} after {step}");
        }
    };
    let steps: &[(&str, &str)] = &[
        ("A1", "3"),
        ("A1", "4"),
        ("A1", "5"), // scrubs reusing one plan
        ("C3", "=A3 2 *"),
        ("A1", "6"),
        ("A1", "7"), // a new dependent appears
        ("A2", "=A1 4 *"),
        ("A1", "8"), // formula edit with the same references
        ("A2", "=C1 1 +"),
        ("A1", "9"),
        ("A1", "2"), // references changed
        // the scrubbed cell itself gains references, closing a cycle
        ("A1", "=C2 1 +"),
        ("A1", "3"),
        ("A1", "4"),
        ("F1", "=A1 range"),
        ("A1", "3"),
        ("A1", "4"), // a spill that grows with every scrub
        ("F3", "x"),
        ("A1", "5"),
        ("A1", "2"), // blocked, then shrinks clear of the blocker
        ("F3", ""),
        ("A1", "4"),
        ("D1", "=A1 A3 +"),
        ("A1", "5"),
        ("A1", "6"),
    ];
    for (cell, text) in steps {
        set(&mut e, cell, text);
        check(&e, &format!("{cell} := {text}"));
    }
}
