//! Paste CSV at scale: parse and paste a generated CSV, then undo it.
//! `cargo run --release -p wbs-core --example bench_csv -- [rows] [cols]`
use std::time::Instant;
use wbs_core::csv::{self, Sep};
use wbs_core::stdlib::default_workbook;
use wbs_core::Engine;

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<usize>().unwrap());
    let (rows, cols) = (args.next().unwrap_or(8670), args.next().unwrap_or(8));
    let mut text = String::new();
    for r in 0..rows {
        let line: Vec<String> = (0..cols)
            .map(|c| match c % 4 {
                0 => format!("{}", r * 7 + c),
                1 => format!("\"name {r}, {c}\""),
                2 => format!("2026-{:02}-{:02}", r % 12 + 1, r % 28 + 1),
                _ => format!("{}.5", r),
            })
            .collect();
        text.push_str(&line.join(","));
        text.push_str("\r\n");
    }
    let mut e = Engine::new(default_workbook());
    let s = e.wb.sheets[0].id;
    let t = Instant::now();
    let parsed = csv::parse(&text, Sep::Comma);
    println!("parse {rows}×{cols} ({} KB): {:.1} ms", text.len() / 1024, t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let ed = csv::paste(&e, &parsed, true, &[], s, (0, 0));
    println!("build edit: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let inv = e.apply(ed);
    println!("apply: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    e.apply(inv);
    let tsv: String = parsed.iter().map(|r| r.iter().map(|f| f.replace(['\t', '\n'], " ")).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n");
    let mut e2 = Engine::new(default_workbook());
    let s2 = e2.wb.sheets[0].id;
    let t2 = Instant::now();
    let ed = wbs_core::ops::paste_text(&mut e2, &tsv, s2, (0, 0));
    e2.apply(ed);
    println!("(plain ⌘V of the same as TSV, build + apply: {:.1} ms)", t2.elapsed().as_secs_f64() * 1e3);
    println!("undo: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
}
