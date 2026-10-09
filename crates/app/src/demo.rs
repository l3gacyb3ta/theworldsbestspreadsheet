//! The workbook shown on first launch: a small model exercising the features.

use wbs_core::a1;
use wbs_core::engine::Engine;
use wbs_core::stdlib::default_workbook;

const CELLS: &[(&str, &str)] = &[
    ("A1", "'Revenue model — alt-drag a yellow input (or use the Inputs panel) and watch everything move"),
    ("A3", "start revenue"),
    ("B3", "120000 [USD]"),
    ("A4", "monthly growth"),
    ("B4", "4.0 [%]"),
    ("A5", "months"),
    ("B5", "24"),
    ("A6", "cost ratio"),
    ("B6", "0.70"),
    ("A7", "fixed cost"),
    ("B7", "40000 [USD]"),
    ("A9", "month"),
    ("B9", "revenue"),
    ("C9", "cost"),
    ("D9", "profit"),
    ("A10", "=months range 1 +"),
    ("B10", "=1 growth + A10 1 - ^ start *"),
    ("C10", "=B10 cost_ratio * fixed +"),
    ("D10", "=B10 C10 -"),
    ("F3", "=A10 B10 line A10 C10 line layer \"revenue vs cost\" title 7 16 size"),
    ("F20", "total profit"),
    ("G20", "=D10 sum"),
    ("F21", "NPV at 1%/month"),
    ("G21", "=1 [%] D10 npv"),
    ("F22", "first profitable month"),
    ("G22", "=D10 0 [USD] > A10 1000 if /min"),
    ("M3", "words"),
    ("N3", ": npv { rate cfs } cfs 1 rate + cfs len range ^ / sum ;"),
    ("M6", "units"),
    ("N6", "dim widgets"),
    ("N7", "base [widget] widgets"),
    ("A36", "units"),
    ("A37", "trip"),
    ("B37", "250 [km]"),
    ("C37", "2.5 [h]"),
    ("D37", "=B37 C37 /"),
    ("E37", "=D37 to[mph]"),
    ("A38", "kettle energy"),
    ("B38", "1.5 [kg]"),
    ("C38", "80 [Δ°C]"),
    ("D38", "=B38 4186 [J/(kg*Δ°C)] * C38 * to[kWh]"),
    ("E38", "=D38 2 [kW] / to[min]"),
    ("A39", "temperature"),
    ("B39", "20 [°C]"),
    ("C39", "=B39 to[°F]"),
    ("D39", "=B39 15 [Δ°C] +"),
    ("A40", "deadline"),
    ("B40", "2026-12-24"),
    ("C40", "=B40 2026-10-08 -"),
    ("A41", "price"),
    ("B41", "49 [EUR]"),
    ("C41", "=B41 to[USD]"),
    ("A43", "quarterly targets — drag the bar tops"),
    ("A44", "Q1"),
    ("B44", "120 [widget]"),
    ("A45", "Q2"),
    ("B45", "150 [widget]"),
    ("A46", "Q3"),
    ("B46", "135 [widget]"),
    ("A47", "Q4"),
    ("B47", "180 [widget]"),
    ("A48", "total"),
    ("B48", "=B44:B47 sum"),
    ("A49", "revenue at $12"),
    ("B49", "=B48 12 [USD/widget] *"),
    ("D43", "=A44:A47 B44:B47 bar \"targets\" title 6 12 size"),
];

pub fn workbook() -> Engine {
    let mut wb = default_workbook();
    wb.sheets[0].name = "model".into();
    let mut eng = Engine::new(wb);
    let sid = eng.wb.sheets[0].id;
    for (at, text) in CELLS {
        let r = a1::parse_ref(at).unwrap();
        let k = eng.wb.sheet_mut(sid).unwrap().key_grow(r.row, r.col);
        eng.set_text(k, text);
    }
    for (name, at, input) in [
        ("start", "B3", true),
        ("growth", "B4", true),
        ("months", "B5", true),
        ("cost_ratio", "B6", true),
        ("fixed", "B7", true),
    ] {
        let r = a1::parse_ref(at).unwrap();
        let k = eng.wb.sheets[0].key(r.row, r.col).unwrap();
        eng.set_name(name, Some(k), input).expect("demo name");
    }
    let s = &mut eng.wb.sheets[0];
    for (c, w) in [(0, 150.0), (1, 140.0), (2, 140.0), (3, 140.0), (4, 120.0), (5, 150.0), (6, 130.0), (13, 380.0)] {
        let id = s.cols.get(c).unwrap();
        s.col_widths.insert(id, w);
    }
    eng
}

#[cfg(test)]
mod tests {
    use wbs_core::engine::Shown;

    #[test]
    fn demo_has_no_errors() {
        let eng = super::workbook();
        let s = &eng.wb.sheets[0];
        let mut errs = Vec::new();
        for (at, _) in super::CELLS {
            let r = wbs_core::a1::parse_ref(at).unwrap();
            let k = s.key(r.row, r.col).unwrap();
            match eng.shown(k) {
                Shown::Error(e) => errs.push(format!("{at}: {}", e.msg)),
                Shown::Value { value, .. } => println!("{at} = {}", value.display_at(0, 0)),
                Shown::Empty => {}
            }
        }
        assert!(errs.is_empty(), "{errs:#?}");
    }
}
