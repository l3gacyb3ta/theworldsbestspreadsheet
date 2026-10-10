//! Edit benchmark on a workbook: `cargo run --release -p wbs-core --example bench_edits -- file.wbs.json sheet cell`.
//! Retypes a formula (same references), types a new formula that references the cell above,
//! inserts and deletes a row above the cell, and undoes each.
use std::time::Instant;
use wbs_core::engine::Engine;
use wbs_core::model::Workbook;

fn time(what: &str, n: usize, mut f: impl FnMut(usize)) {
    let mut v = Vec::new();
    for i in 0..n {
        let t = Instant::now();
        f(i);
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("{what:28} best {:7.2} ms  median {:7.2} ms", v[0], v[v.len() / 2]);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let wb: Workbook = serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let mut e = Engine::new(wb);
    let six = e.wb.sheets.iter().position(|s| s.name == a[2]).unwrap();
    let r = wbs_core::a1::parse_ref(&a[3]).unwrap();
    let k = e.wb.sheets[six].key(r.row, r.col).unwrap();
    let sid = e.wb.sheets[six].id;
    let orig = e.wb.cell_text(k);
    if std::env::var("ONLY_ROWS").is_ok() {
        time("insert+delete row (undo)", 12, |_| {
            let inv = e.apply(e.insert_rows_edit(sid, r.row, 1));
            e.apply(inv);
        });
        return;
    }
    time("retype formula", 10, |i| {
        let inv = e.set_text(k, &format!("{orig} {} +", i % 2));
        e.apply(inv);
    });
    time("insert+delete row (undo)", 6, |_| {
        let inv = e.apply(e.insert_rows_edit(sid, r.row, 1));
        e.apply(inv);
    });
    let blank = e.wb.sheets[six].key_grow(r.row, 30);
    time("new formula in empty cell", 10, |i| {
        e.set_text(blank, &format!("={} {i} *", wbs_core::a1::cell_name(r.row, r.col)));
    });
    time("clear it", 1, |_| {
        e.set_text(blank, "");
    });
}
