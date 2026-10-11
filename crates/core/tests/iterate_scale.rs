//! An hourly battery model, as in a real solar sheet: a year-long `iterate` whose step word `pick`s
//! from a named year-long array. `pick` used to split the whole array into rows each time, so this was
//! quadratic (15 s per recalc). See also `examples/bench_iterate.rs`.

use std::time::Instant;
use wbs_core::a1;
use wbs_core::engine::{Engine, Shown};
use wbs_core::ids::CellKey;
use wbs_core::stdlib::default_workbook;
use wbs_core::value::Value;

const HOURS: usize = 8760;

fn key(e: &Engine, r: &str) -> CellKey {
    let a = a1::parse_ref(r).unwrap();
    e.wb.sheets[0].key(a.row, a.col).unwrap()
}

fn set(e: &mut Engine, r: &str, text: &str) {
    let k = key(e, r);
    e.set_text(k, text);
}

fn num(e: &Engine, r: &str) -> Vec<f64> {
    match e.shown(key(e, r)) {
        Shown::Value { value: Value::Num(n), .. } => n.data.to_vec(),
        Shown::Error(err) => panic!("{r}: {}", err.msg),
        _ => panic!("{r}: not a number"),
    }
}

/// Generation minus consumption for hour `h`, in kWh: positive by day, negative at night.
fn net(h: usize) -> f64 {
    let gen = (((h % 24) as f64 - 6.0) / 12.0 * std::f64::consts::PI).sin().max(0.0) * 6.0;
    let used = 0.6 + 0.4 * ((h * 7 % 11) as f64 / 11.0);
    ((gen - used) * 1000.0).round() / 1000.0
}

/// The same model in plain Rust: charge (with losses) or discharge, limited by capacity and power.
fn reference(capacity: f64, power: f64, roundtrip: f64) -> Vec<f64> {
    let mut soc = vec![0.0];
    for h in 0..HOURS {
        let (n, s) = (net(h), soc[h]);
        soc.push(if n > 0.0 { s + (n * roundtrip).min(capacity - s).min(power) } else { s - (-n).min(s).min(power) });
    }
    soc
}

#[test]
fn a_year_long_iterate_with_pick_is_fast_and_right() {
    let mut e = Engine::new(default_workbook());
    for (r, text, name) in [("B1", "5.5 [kW*h]", "batteryCapacity"), ("B2", "3 [kW]", "batteryPower"), ("B3", "0.9", "roundtrip"), ("B4", "1 [h]", "dt")] {
        set(&mut e, r, text);
        e.set_name(name, Some(key(&e, r)), true).unwrap();
    }
    set(&mut e, "A6", ": chg ( n soc -- e ) { n soc } n roundtrip * batteryCapacity soc - min batteryPower dt * min ;");
    set(&mut e, "A7", ": dis ( n soc -- e ) { n soc } n neg soc min batteryPower dt * min ;");
    set(&mut e, "A8", ": step ( i soc -- i' soc' ) { i soc } i 1 + net i pick 0 [kW*h] > soc net i pick soc chg + soc net i pick soc dis - if ;");
    for h in 0..HOURS {
        set(&mut e, &format!("C{}", h + 11), &format!("{} [kW*h]", net(h)));
    }
    set(&mut e, "D10", &format!("=C11:C{}", HOURS + 10));
    e.set_name("net", Some(key(&e, "D10")), true).unwrap();

    let t = Instant::now();
    set(&mut e, "E10", "=net len 0 0 [kW*h] 'step 2 iterate swap drop");
    let first = t.elapsed();
    let t = Instant::now();
    set(&mut e, "B1", "6 [kW*h]");
    let rerun = t.elapsed();
    // ~0.1 s in a release build, minutes when it was quadratic: generous enough for a slow CI machine
    assert!(first.as_secs_f64() < 5.0 && rerun.as_secs_f64() < 5.0, "first {first:?}, rerun {rerun:?}");

    let kwh = num(&e, "B1")[0] / 6.0;
    let got: Vec<f64> = num(&e, "E10").iter().map(|x| x / kwh).collect();
    let want = reference(6.0, 3.0, 0.9);
    assert_eq!(got.len(), HOURS + 1);
    for (h, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-9, "hour {h}: {g} vs {w}");
    }
    assert!(want.iter().any(|&s| s > 5.9), "the battery fills up some days");
}
