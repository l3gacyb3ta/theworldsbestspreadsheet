//! Open benchmark: `cargo run --release -p wbs-core --example bench_open -- file.wbs.json [times]`.
use std::time::Instant;
use wbs_core::engine::Engine;
use wbs_core::model::Workbook;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let s = std::fs::read_to_string(&p).unwrap();
    let (mut parse, mut new) = (vec![], vec![]);
    for _ in 0..n {
        let t = Instant::now();
        let wb: Workbook = serde_json::from_str(&s).unwrap();
        parse.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let e = Engine::new(wb);
        new.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(e);
    }
    let best = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    println!("parse best {:.1} ms, Engine::new best {:.1} ms", best(&parse), best(&new));
}
