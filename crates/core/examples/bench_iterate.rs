//! An hourly battery model, as in a real solar sheet: an 8,760-step `iterate` whose step word
//! `pick`s from a named 8,760-row array three times. `pick` used to split the whole array into rows
//! each time (quadratic: ~15 s). `cargo run --release -p wbs-core --example bench_iterate [hours]`
use std::time::Instant;
use wbs_core::a1;
use wbs_core::engine::Engine;
use wbs_core::stdlib::default_workbook;

fn set(e: &mut Engine, r: &str, text: &str) {
    let a = a1::parse_ref(r).unwrap();
    let k = e.wb.sheets[0].key(a.row, a.col).unwrap();
    e.set_text(k, text);
}

fn name(e: &mut Engine, r: &str, n: &str) {
    let a = a1::parse_ref(r).unwrap();
    let k = e.wb.sheets[0].key(a.row, a.col).unwrap();
    e.set_name(n, Some(k), true).unwrap();
}

fn main() {
    let hours: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(8760);
    let mut e = Engine::new(default_workbook());
    let t = Instant::now();
    for (r, text, n) in [("B1", "5.5 [kW*h]", "batteryCapacity"), ("B2", "3 [kW]", "batteryPower"), ("B3", "0.9", "roundtrip"), ("B4", "1 [h]", "dt")] {
        set(&mut e, r, text);
        name(&mut e, r, n);
    }
    set(&mut e, "A6", ": chg ( n soc -- e ) { n soc } n roundtrip * batteryCapacity soc - min batteryPower dt * min ;");
    set(&mut e, "A7", ": dis ( n soc -- e ) { n soc } n neg soc min batteryPower dt * min ;");
    set(&mut e, "A8", ": step ( i soc -- i' soc' ) { i soc } i 1 + net i pick 0 [kW*h] > soc net i pick soc chg + soc net i pick soc dis - if ;");
    // generation minus consumption, hour by hour: positive by day, negative at night
    for h in 0..hours {
        let gen = (((h % 24) as f64 - 6.0) / 12.0 * std::f64::consts::PI).sin().max(0.0) * 6.0;
        let used = 0.6 + 0.4 * ((h * 7 % 11) as f64 / 11.0);
        set(&mut e, &format!("C{}", h + 11), &format!("{:.3} [kW*h]", gen - used));
    }
    set(&mut e, "D10", &format!("=C11:C{}", hours + 10));
    name(&mut e, "D10", "net");
    println!("build: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = Instant::now();
    set(&mut e, "E10", "=net len 0 0 [kW*h] 'step 2 iterate swap drop");
    println!("iterate {hours} steps (3 picks each): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    // scrubbing an input reruns it
    let t = Instant::now();
    set(&mut e, "B1", "6 [kW*h]");
    println!("rerun after an input change: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    let k = e.wb.sheets[0].key(9, 4).unwrap();
    match e.shown(k) {
        wbs_core::Shown::Value { value, .. } => println!("battery state: {}", value.summary(4)),
        wbs_core::Shown::Error(err) => println!("battery state: error {}", err.msg),
        wbs_core::Shown::Empty => println!("battery state: empty"),
    }
}
