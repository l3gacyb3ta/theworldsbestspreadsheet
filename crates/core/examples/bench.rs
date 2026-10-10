//! Recalc benchmark: `cargo run --release -p wbs-core --example bench [seconds]`.
//! Builds the 3,000-cell scrub sheet from tests/engine.rs and scrubs its input
//! in a loop, printing per-step timings. Handy under a sampling profiler.

use std::time::{Duration, Instant};
use wbs_core::a1;
use wbs_core::engine::Engine;
use wbs_core::stdlib::default_workbook;

fn set(e: &mut Engine, r: &str, text: &str) {
    let a = a1::parse_ref(r).unwrap();
    let k = e.wb.sheets[0].key_grow(a.row, a.col);
    e.set_text(k, text);
}

fn main() {
    let secs: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let mut e = Engine::new(default_workbook());
    let t = Instant::now();
    set(&mut e, "A1", "1.05");
    for r in 2..=1001 {
        set(&mut e, &format!("A{r}"), &format!("=A{} A1 *", r - 1));
        set(&mut e, &format!("B{r}"), &format!("=A{r} 100 [USD] *"));
        set(&mut e, &format!("C{r}"), &format!("=B{r} 0.8 * to[EUR]"));
    }
    set(&mut e, "D1", "=C2:C1001 sum");
    println!("build: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    let end = Instant::now() + Duration::from_secs_f64(secs);
    let mut steps = Vec::new();
    let mut i = 0;
    while Instant::now() < end {
        let t = Instant::now();
        set(&mut e, "A1", &format!("1.0{}", i % 10));
        steps.push(t.elapsed().as_secs_f64() * 1000.0);
        i += 1;
    }
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "scrub: {} steps over {} cells — best {:.2} ms, median {:.2} ms",
        steps.len(),
        e.last_eval_count,
        steps[0],
        steps[steps.len() / 2]
    );
}
