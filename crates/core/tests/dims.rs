//! The static dimension pass agrees with evaluation.

use wbs_core::a1;
use wbs_core::dims::SVal;
use wbs_core::engine::{Engine, ErrKind, Shown};
use wbs_core::ids::CellKey;
use wbs_core::parse::BUILTINS;
use wbs_core::stdlib::default_workbook;
use wbs_core::value::Value;

fn eng() -> Engine {
    Engine::new(default_workbook())
}

fn key(e: &mut Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheets[0].key_grow(a.row, a.col)
}

fn set(e: &mut Engine, r: &str, text: &str) {
    let k = key(e, r);
    e.set_text(k, text);
}

fn show(e: &mut Engine, r: &str) -> String {
    let k = key(e, r);
    match e.shown(k) {
        Shown::Empty => "<empty>".into(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(err) => format!("ERR {}", err.msg),
    }
}

fn sdim(e: &mut Engine, r: &str) -> String {
    let k = key(e, r);
    e.static_value(k).map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

/// A sheet with a bit of everything programs can refer to.
fn fixture() -> Engine {
    let mut e = eng();
    for (at, text) in [
        ("A1", "5 [m]"),
        ("A2", "3 [s]"),
        ("A3", "hello"),
        ("A4", "20 [°C]"),
        ("A5", "2"),
        ("A6", "3 [km]"),
        // A7 is empty
        ("A8", "=1 \"x\" +"),
        ("A9", "2026-01-01"),
        ("B1", "1 [m]"),
        ("B2", "2 [m]"),
        ("B3", "3 [m]"),
        ("C1", "=3 range"),
        ("D1", ": sq dup * ;"),
        ("D2", ": hyp { a b } a a * b b * + sqrt ;"),
        ("D3", ": bad 1 [m] + ;"),
        ("D4", ": deep deep ;"),
        ("E1", "=2 3 [m] *"),
        ("E2", "=A7 1 [m] +"),
        // units defined from cells: one known, one whose factor is empty
        ("F1", "[crate] = A5 [m]"),
        ("F2", "[box] = A7 [s]"),
    ] {
        set(&mut e, at, text);
    }
    e
}

/// Static error ⇒ evaluation fails with the same message at the same token
/// (or earlier, on a check that depends on values); static dimension ⇒ the
/// evaluated value has it.
#[test]
fn static_pass_agrees_with_evaluation() {
    let mut e = fixture();
    let sid = e.wb.sheets[0].id;
    let mut pool: Vec<String> = [
        "1", "2", "0.5", "-1", "0", "1 2 /", "3", "[m]", "[s]", "[km]", "[%]", "[°C]", "[Δ°C]", "[m/s]", "[m^2]", "[date]", "[day]",
        "to[m]", "to[km]", "to[°C]", "to[K]", "to[s]", "to[day]", "\"t\"", "A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A9", "B1:B3",
        "A1:A2", "B1:B3?", "A5:A7?", "A1:A6", "C1", "C2", "C1:C3", "E1", "E2", "sq", "hyp", "bad", "deep", "/+", "\\+", "/*", "/max",
        "/sq", "\\hyp", "/-", "\\*", "[crate]", "[box]", "to[crate]", "F1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // half the programs use only the common words, so that more of them get somewhere
    let common: Vec<String> =
        pool.iter().cloned().chain(["+", "-", "*", "/", "^", "sqrt", "dup", "swap", "sum", "if", "min"].map(String::from)).collect();
    pool.extend(BUILTINS.iter().map(|(n, _, _)| n.to_string()));
    let k = key(&mut e, "H30");
    // DIMS_SEED=n tries other programs
    let mut seed: u64 = std::env::var("DIMS_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x2545_f491_4f6c_dd1d);
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let (mut errors, mut dims, mut earlier) = (0, 0, 0);
    let mut bad = Vec::new();
    for _ in 0..60_000 {
        let words = if rnd(2) == 0 { &pool } else { &common };
        let n = 1 + rnd(7);
        let prog: Vec<&str> = (0..n).map(|_| words[rnd(words.len())].as_str()).collect();
        let text = format!("={}", prog.join(" "));
        e.set_text(k, &text);
        let run = e.eval_scratch(&text, sid).result;
        match (e.static_error(k).cloned(), &run) {
            (Some(se), Ok(v)) => bad.push(format!("`{text}`: static error {:?} but evaluates to {}", se.msg, v.summary(4))),
            (Some(se), Err(re)) => {
                errors += 1;
                if se.msg != re.msg || se.span != re.span {
                    // only a value-dependent check at an earlier token may fail first
                    let before = match (&re.span, &se.span) {
                        (Some(r), Some(s)) => r.start < s.start,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    if before {
                        earlier += 1;
                    } else {
                        bad.push(format!("`{text}`: static {:?} at {:?}, evaluation {:?} at {:?}", se.msg, se.span, re.msg, re.span));
                    }
                }
                // the cell shows the static error
                match e.shown(k) {
                    Shown::Error(ce) if ce.msg == se.msg && ce.kind == ErrKind::Local => {}
                    _ => bad.push(format!("`{text}`: cell doesn't show the static error")),
                }
            }
            (None, Ok(v)) => {
                let ok = match (e.static_value(k).unwrap(), v) {
                    (SVal::Num(s), Value::Num(n)) => {
                        if s.dim.is_some() {
                            dims += 1;
                        }
                        s.dim.as_ref().is_none_or(|d| *d == n.q.dim) && s.abs.is_none_or(|a| a == n.q.absolute.is_some())
                    }
                    (SVal::Text, Value::Text(_)) | (SVal::Chart, Value::Chart(_)) | (SVal::Any, _) => true,
                    (SVal::Other(t), v) => *t == v.type_name(),
                    _ => false,
                };
                if !ok {
                    bad.push(format!(
                        "`{text}`: static {} but evaluates to {} ({:?})",
                        e.static_value(k).unwrap(),
                        v.summary(4),
                        v.type_name()
                    ));
                }
            }
            (None, Err(_)) => {}
        }
    }
    bad.sort();
    bad.dedup();
    assert!(bad.is_empty(), "{} disagreements:\n{}", bad.len(), bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
    eprintln!("{errors} static errors ({earlier} preceded by a value-dependent one), {dims} known dimensions");
    assert!(errors > 1000 && dims > 1000, "the generator should exercise both: {errors} errors, {dims} dims");
    assert!(earlier * 10 < errors, "most static errors should be exactly evaluation's: {earlier} of {errors} weren't");
}

#[test]
fn unit_errors_show_without_inputs() {
    let mut e = eng();
    // A1 is empty: evaluation would stop there, but the mismatch is certain
    set(&mut e, "B1", "=A1 1 [m] 1 [s] + +");
    assert_eq!(show(&mut e, "B1"), "ERR + needs matching units: length vs time");
    // an upstream error doesn't hide this cell's own unit error
    set(&mut e, "A2", "=A1 1 [m] +");
    assert_eq!(show(&mut e, "A2"), "ERR Sheet1!A1 is empty");
    assert_eq!(sdim(&mut e, "A2"), "length");
    set(&mut e, "B2", "=A2 2 [m] 3 [s] * -");
    assert_eq!(show(&mut e, "B2"), "ERR - needs matching units: length vs length*time");
    let k = key(&mut e, "B2");
    assert_eq!(e.static_error(k).unwrap().span, Some(18..19));
    // once the input is there, evaluation reports the same
    set(&mut e, "A1", "4 [m]");
    assert_eq!(show(&mut e, "A2"), "5 m");
    assert_eq!(show(&mut e, "B2"), "ERR - needs matching units: length vs length*time");
}

#[test]
fn dimensions_are_known_without_values() {
    let mut e = eng();
    set(&mut e, "B1", "=A1 drop 5 [m/s]");
    assert_eq!(show(&mut e, "B1"), "ERR Sheet1!A1 is empty");
    assert_eq!(sdim(&mut e, "B1"), "length/time");
    set(&mut e, "B2", "=B1 2 [s] *");
    assert!(show(&mut e, "B2").starts_with("ERR"));
    assert_eq!(sdim(&mut e, "B2"), "length");
    // an empty input's dimension is unknown
    set(&mut e, "B3", "=A1 [m]");
    assert_eq!(sdim(&mut e, "B3"), "?");
    set(&mut e, "A1", "3");
    assert_eq!(sdim(&mut e, "B3"), "length");
    assert_eq!(show(&mut e, "B3"), "3 m");
    // temperatures are absolute; their differences aren't
    set(&mut e, "C1", "20 [°C]");
    set(&mut e, "C2", "=C1 10 [°C] -");
    assert_eq!(sdim(&mut e, "C1"), "temperature (absolute)");
    assert_eq!(sdim(&mut e, "C2"), "temperature");
    // ranges take their cells' dimension; spills report their source's
    set(&mut e, "D1", "1 [kg]");
    set(&mut e, "D2", "2 [kg]");
    set(&mut e, "D3", "=D1:D2 sum");
    assert_eq!(sdim(&mut e, "D3"), "mass");
    set(&mut e, "E1", "=3 range 1 [s] *");
    assert_eq!(sdim(&mut e, "E3"), "time");
    // exponents written in the program are static; from a cell, they aren't
    set(&mut e, "F1", "=2 [m] 1 2 / ^");
    assert_eq!(sdim(&mut e, "F1"), "length^1/2");
    set(&mut e, "F2", "2");
    set(&mut e, "F3", "=2 [m] F2 ^");
    assert_eq!(sdim(&mut e, "F3"), "?");
    assert_eq!(show(&mut e, "F3"), "4 m^2");
    // words are followed through their bodies
    set(&mut e, "G1", ": speed { d t } d t / ;");
    set(&mut e, "G2", "=A9 drop 10 [km] 2 [h] speed");
    assert_eq!(sdim(&mut e, "G2"), "length/time");
    set(&mut e, "G3", "=1 [m] 1 [s] speed 1 [s] +");
    assert_eq!(show(&mut e, "G3"), "ERR + needs matching units: length/time vs time");
}

#[test]
fn mixed_range_is_reported_statically() {
    let mut e = eng();
    set(&mut e, "A1", "1 [m]");
    set(&mut e, "A2", "2 [s]");
    set(&mut e, "A3", "=A1:A2 sum");
    assert_eq!(show(&mut e, "A3"), "ERR range mixes units: Sheet1!A1 is length, Sheet1!A2 is time");
    // a gap that may be filled with anything stops the check
    set(&mut e, "B1", "1 [m]");
    set(&mut e, "B3", "2 [s]");
    set(&mut e, "B4", "=B1:B3 sum");
    assert_eq!(show(&mut e, "B4"), "ERR Sheet1!B2 is empty");
    // ...but `?` ranges skip it
    set(&mut e, "B5", "=B1:B3? sum");
    assert_eq!(show(&mut e, "B5"), "ERR range mixes units: Sheet1!B1 is length, Sheet1!B3 is time");
}

#[test]
fn if_branches_must_match() {
    let mut e = eng();
    set(&mut e, "A1", "=1 1 [m] 1 [s] if");
    assert_eq!(show(&mut e, "A1"), "ERR if needs matching units: length vs time");
    set(&mut e, "A2", "=1 1 [m] 2 [km] if");
    assert_eq!(show(&mut e, "A2"), "1 m");
    set(&mut e, "A3", "=0 \"a\" \"b\" if");
    assert_eq!(show(&mut e, "A3"), "b");
}

#[test]
fn static_pass_follows_edits() {
    let mut e = eng();
    set(&mut e, "A1", "5 [m]");
    set(&mut e, "A2", "=A1 2 *");
    set(&mut e, "A3", "=A2 1 [m] +");
    assert_eq!(show(&mut e, "A3"), "11 m");
    // a new unit changes the dimension downstream, and the error appears where it occurs
    set(&mut e, "A1", "5 [s]");
    assert_eq!(show(&mut e, "A3"), "ERR + needs matching units: time vs length");
    assert_eq!(sdim(&mut e, "A2"), "time");
    // scrubbing keeps the static result
    set(&mut e, "A1", "6 [s]");
    assert_eq!(show(&mut e, "A3"), "ERR + needs matching units: time vs length");
    set(&mut e, "A1", "6 [m]");
    assert_eq!(show(&mut e, "A3"), "13 m");
    // unit definitions: a unit's dimension is its definition's
    set(&mut e, "B1", "dim widgets");
    set(&mut e, "B2", "base [widget] widgets");
    set(&mut e, "B3", "[crate] = B4 [widget]");
    set(&mut e, "B4", "12");
    set(&mut e, "C1", "=2 [crate] 1 [m] +");
    assert_eq!(sdim(&mut e, "B3"), "widgets");
    assert_eq!(show(&mut e, "C1"), "ERR + needs matching units: widgets vs length");
    // the factor is dynamic: scrubbing it keeps the dimension
    set(&mut e, "B4", "10");
    assert_eq!(show(&mut e, "C1"), "ERR + needs matching units: widgets vs length");
    set(&mut e, "C2", "=2 [crate] to[widget]");
    assert_eq!(show(&mut e, "C2"), "20 widget");
}
