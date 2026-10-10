//! Frame-time benchmark: drives the real app through egui (no window, no GPU)
//! and times the UI pass and tessellation separately.
//!
//! `WBS_PERF_FILE=model.wbs.json WBS_PERF_SHEET=lorentz WBS_PERF_SCRUB=ts \
//!   cargo test --release -p wbs perf_frames -- --ignored --nocapture`

use super::*;
use std::time::Instant;

struct Rig {
    ctx: egui::Context,
    app: App,
    frame: eframe::Frame,
    time: f64,
    events: Vec<Event>,
    ui_ms: Vec<f64>,
    tess_ms: Vec<f64>,
    prims: usize,
    verts: usize,
}

impl Rig {
    fn frame(&mut self) {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        eframe::App::raw_input_hook(&mut self.app, &self.ctx, &mut input);
        self.time += 1.0 / 60.0;
        let t = Instant::now();
        let (app, frame) = (&mut self.app, &mut self.frame);
        let out = self.ctx.run_ui(input, |ui| eframe::App::ui(app, ui, frame));
        let t1 = Instant::now();
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let t2 = Instant::now();
        self.ui_ms.push((t1 - t).as_secs_f64() * 1000.0);
        self.tess_ms.push((t2 - t1).as_secs_f64() * 1000.0);
        self.prims = prims.len();
        self.verts = prims
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                _ => 0,
            })
            .sum();
    }

    fn report(&mut self, what: &str) {
        let stat = |v: &mut Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = v.len();
            (v[n / 2], v[n * 9 / 10], v[n - 1])
        };
        let (u50, u90, umax) = stat(&mut self.ui_ms);
        let (t50, t90, _) = stat(&mut self.tess_ms);
        println!(
            "{what:12} ui median {u50:6.2} ms p90 {u90:6.2} max {umax:6.2} | tessellate median {t50:5.2} p90 {t90:5.2} | {} meshes {} verts | recalc {:.2} ms",
            self.prims, self.verts, self.app.last_recalc_ms
        );
        self.ui_ms.clear();
        self.tess_ms.clear();
    }
}

#[test]
#[ignore]
fn perf_frames() {
    let file = std::env::var("WBS_PERF_FILE").expect("WBS_PERF_FILE");
    let frames: usize = std::env::var("WBS_PERF_FRAMES").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let ctx = egui::Context::default();
    if std::env::var("WBS_PERF_OPEN_STAGES").is_ok() {
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let s = std::fs::read_to_string(&file).unwrap();
        println!("read {:.1}", ms(t));
        let t = Instant::now();
        let wb: wbs_core::model::Workbook = serde_json::from_str(&s).unwrap();
        println!("parse {:.1}", ms(t));
        let t = Instant::now();
        let e = Engine::new(wb);
        println!("engine {:.1}", ms(t));
        let t = Instant::now();
        drop(e);
        println!("drop {:.1}", ms(t));
    }
    let t = Instant::now();
    let mut app = App::new(PathBuf::from(&file));
    println!("open: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = Instant::now();
    let fp = files::fingerprint(&app.eng.wb);
    println!("fingerprint: {:.2} ms ({fp:x})", t.elapsed().as_secs_f64() * 1000.0);
    if std::env::var("WBS_PERF_FP_ONLY").is_ok() {
        for _ in 0..300 {
            std::hint::black_box(files::fingerprint(&app.eng.wb));
        }
        return;
    }
    // never write the benchmark's file
    app.path = None;
    if let Ok(name) = std::env::var("WBS_PERF_SHEET") {
        app.sheet_ix = app.eng.wb.sheets.iter().position(|s| s.name == name).expect("sheet");
    }
    let mut r = Rig { ctx, app, frame: eframe::Frame::_new_kittest(), time: 0.0, events: vec![], ui_ms: vec![], tess_ms: vec![], prims: 0, verts: 0 };
    for _ in 0..5 {
        r.frame();
    }
    r.report("first frames");

    let mid = Pos2::new(400.0, 400.0);
    for i in 0..frames {
        r.events.push(Event::PointerMoved(mid + Vec2::new((i % 40) as f32 * 3.0, (i % 7) as f32 * 5.0)));
        r.frame();
    }
    r.report("hover");

    for _ in 0..frames {
        r.events.push(Event::PointerMoved(mid));
        r.events.push(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -30.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
        r.frame();
    }
    r.report("scroll down");
    for _ in 0..frames {
        r.events.push(Event::PointerMoved(mid));
        r.events.push(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, 60.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
        r.frame();
    }
    for _ in 0..30 {
        r.frame();
    }

    if let Ok(name) = std::env::var("WBS_PERF_SCRUB") {
        let k = r.app.eng.wb.names[&name].cell;
        let (row, col) = r.app.eng.wb.pos(k).unwrap();
        r.app.sheet_ix = r.app.eng.wb.sheet_index(k.sheet).unwrap();
        r.app.select(row, col, false);
        r.app.scroll_into_view = true;
        for _ in 0..3 {
            r.frame();
        }
        let p = r.app.geo.as_ref().unwrap().cell(row, col).center();
        r.events.push(Event::ModifiersChanged(Modifiers::ALT));
        r.events.push(Event::PointerMoved(p));
        r.frame();
        r.events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
        r.frame();
        r.ui_ms.clear();
        r.tess_ms.clear();
        let mut recalc = vec![];
        for i in 0..frames {
            let dx = ((i % 20) as f32 - 10.0).abs() * 2.0 + 1.0;
            r.events.push(Event::PointerMoved(p + Vec2::new(dx, 0.0)));
            r.frame();
            recalc.push(r.app.last_recalc_ms);
        }
        recalc.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("scrub {name}: source now {:?}, recalc median {:.2} ms", r.app.eng.wb.cell_text(k), recalc[recalc.len() / 2]);
        r.report("scrub");
        r.events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
        r.events.push(Event::ModifiersChanged(Modifiers::NONE));
        r.frame();
    }
}
