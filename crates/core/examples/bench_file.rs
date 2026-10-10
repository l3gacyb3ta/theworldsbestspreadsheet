use std::time::Instant;
use wbs_core::engine::Engine;
use wbs_core::model::Workbook;
fn ms(t: Instant) -> f64 { t.elapsed().as_secs_f64() * 1000.0 }
fn main() {
    println!("size_of Value {} Num {} Quant {} Provs {} CellResult {} Op {}", std::mem::size_of::<wbs_core::value::Value>(), std::mem::size_of::<wbs_core::value::Num>(), std::mem::size_of::<wbs_core::units::Quant>(), std::mem::size_of::<wbs_core::value::Provs>(), std::mem::size_of::<Result<wbs_core::value::Value, wbs_core::engine::CellError>>(), std::mem::size_of::<wbs_core::parse::Op>());
    let p = std::env::args().nth(1).unwrap();
    let t = Instant::now();
    let s = std::fs::read_to_string(&p).unwrap();
    println!("read {:.1} ms ({} bytes)", ms(t), s.len());
    let t = Instant::now();
    let wb: Workbook = serde_json::from_str(&s).unwrap();
    println!("parse {:.1} ms", ms(t));
    let t = Instant::now();
    let mut e = Engine::new(wb);
    println!("engine new {:.1} ms ({} evaluated)", ms(t), e.last_eval_count);
    let t = Instant::now();
    let snap = e.wb.clone();
    println!("clone workbook {:.1} ms", ms(t));
    drop(snap);
    let t = Instant::now();
    let out = serde_json::to_string(&e.wb).unwrap();
    println!("save serialize compact {:.1} ms ({} bytes)", ms(t), out.len());
    let t = Instant::now();
    let out = serde_json::to_string_pretty(&e.wb).unwrap();
    println!("save serialize {:.1} ms ({} bytes)", ms(t), out.len());
    let names: Vec<_> = e.wb.names.iter().filter(|(_, d)| e.wb.sheet(d.cell.sheet).is_some()).map(|(n, d)| (n.clone(), d.cell)).collect();
    let only = std::env::args().nth(2);
    let reps: usize = std::env::args().nth(3).and_then(|x| x.parse().ok()).unwrap_or(200);
    for (n, k) in names {
        if only.as_ref().is_some_and(|o| *o != n) { continue }
        let orig = e.wb.cell_text(k);
        let Some((span, _)) = wbs_core::model::number_literal(&orig) else { continue };
        let base: f64 = orig[span.clone()].trim().parse().unwrap_or(1.0);
        let mut steps = vec![];
        for i in 0..reps {
            let v = base * (1.0 + (i % 20) as f64 * 0.003);
            let txt = format!("{}{}{}", &orig[..span.start], v, &orig[span.end..]);
            let t = Instant::now();
            e.set_text(k, &txt);
            steps.push(ms(t));
        }
        e.set_text(k, &orig);
        steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("scrub {n:10} median {:.3} ms  p90 {:.3}  ({} cells)", steps[reps / 2], steps[reps * 9 / 10], e.last_eval_count);
    }
}
