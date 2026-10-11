//! Renders a chart value into the rectangle of cells it spills over.

use eframe::egui::{self, epaint::TextShape, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};
use wbs_core::chart::{Chart, Mark, Xs};
use wbs_core::ids::CellKey;
use wbs_core::units::DispUnit;
use wbs_core::value::{fmt_date, fmt_num, Prov};

pub const SERIES: [Color32; 6] = [
    Color32::from_rgb(0x25, 0x63, 0xeb),
    Color32::from_rgb(0xf9, 0x73, 0x16),
    Color32::from_rgb(0x10, 0xb9, 0x81),
    Color32::from_rgb(0xa8, 0x55, 0xf7),
    Color32::from_rgb(0xef, 0x44, 0x44),
    Color32::from_rgb(0x0e, 0xa5, 0xe9),
];

pub struct PointHit {
    pub pos: Pos2,
    /// Where its y value came from.
    pub prov: Prov,
    /// Where its x value came from, on a mark whose points move in 2D (scatter, path); `None` on the others.
    pub xprov: Prov,
    /// A scatter or path point on a numeric x axis: it moves in 2D. Line and bar points move only up and down.
    pub two_d: bool,
    /// Which point: its label is built only when it's shown (`point_label`).
    pub layer: usize,
    pub index: usize,
}

/// `x → y` for a point, as its tooltip shows it.
pub fn point_label(chart: &Chart, layer: usize, i: usize) -> String {
    let Some(l) = chart.layers.get(layer) else { return String::new() };
    let x = match &l.xs {
        Xs::Num(n) => n.fmt_elem(i),
        Xs::Text(t) => t.data[i].to_string(),
    };
    format!("{x} → {}", l.ys.fmt_elem(i))
}

/// Maps between screen y and y values in the chart's display unit (and screen x and x values, on a numeric x axis).
#[derive(Clone)]
pub struct YAxis {
    pub plot: Rect,
    pub y0: f64,
    pub y1: f64,
    pub disp: DispUnit,
    /// The x range, in `xdisp`; `xdisp` is `None` for a category x axis.
    pub x0: f64,
    pub x1: f64,
    pub xdisp: Option<DispUnit>,
    /// The chart's cell, set by the grid once drawn.
    pub anchor: Option<CellKey>,
}

impl YAxis {
    pub fn to_screen(&self, v: f64) -> f32 {
        let t = if self.y1 > self.y0 { (v - self.y0) / (self.y1 - self.y0) } else { 0.5 };
        self.plot.bottom() - (t as f32) * self.plot.height()
    }
    pub fn from_screen(&self, y: f32) -> f64 {
        let t = ((self.plot.bottom() - y) / self.plot.height()) as f64;
        self.y0 + t * (self.y1 - self.y0)
    }
    pub fn x_to_screen(&self, v: f64) -> f32 {
        self.plot.left() + (((v - self.x0) / (self.x1 - self.x0)) as f32) * self.plot.width()
    }
    pub fn x_from_screen(&self, x: f32) -> f64 {
        let t = ((x - self.plot.left()) / self.plot.width()) as f64;
        self.x0 + t * (self.x1 - self.x0)
    }
}

fn nice_step(range: f64, target: f64) -> f64 {
    if range <= 0.0 || !range.is_finite() {
        return 1.0;
    }
    let raw = range / target;
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    let s = if n < 1.5 {
        1.0
    } else if n < 3.0 {
        2.0
    } else if n < 7.0 {
        5.0
    } else {
        10.0
    };
    s * mag
}

fn ticks(lo: f64, hi: f64, target: f64) -> Vec<f64> {
    let step = nice_step(hi - lo, target);
    let mut v = (lo / step).ceil() * step;
    let mut out = Vec::new();
    while v <= hi + step * 1e-9 && out.len() < 50 {
        out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        v += step;
    }
    out
}

fn pad(lo: f64, hi: f64) -> (f64, f64) {
    if !(lo.is_finite() && hi.is_finite()) {
        return (0.0, 1.0);
    }
    if (hi - lo).abs() < 1e-12 {
        let d = if lo == 0.0 { 1.0 } else { lo.abs() * 0.1 };
        return (lo - d, hi + d);
    }
    let d = (hi - lo) * 0.05;
    (lo - d, hi + d)
}

fn short(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{}G", fmt_num((v / 1e9 * 100.0).round() / 100.0))
    } else if a >= 1e6 {
        format!("{}M", fmt_num((v / 1e6 * 100.0).round() / 100.0))
    } else if a >= 1e4 {
        format!("{}k", fmt_num((v / 1e3 * 100.0).round() / 100.0))
    } else {
        fmt_num((v * 1e6).round() / 1e6)
    }
}

/// `fixed` keeps the y and x ranges (while a point is dragged, so it stays under the pointer).
pub fn draw(p: &Painter, rect: Rect, chart: &Chart, dark: bool, fixed: Option<((f64, f64), (f64, f64))>) -> (YAxis, Vec<PointHit>) {
    let (bg, fg, grid, muted) = if dark {
        (Color32::from_rgb(0x1f, 0x22, 0x28), Color32::from_gray(220), Color32::from_gray(60), Color32::from_gray(150))
    } else {
        (Color32::WHITE, Color32::from_gray(40), Color32::from_gray(232), Color32::from_gray(110))
    };
    let rect = rect.shrink(2.0);
    p.rect_filled(rect, 6.0, bg);
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, grid), StrokeKind::Inside);
    let small = FontId::proportional(10.5);
    let mut top = rect.top() + 8.0;
    if let Some(t) = &chart.title {
        p.text(Pos2::new(rect.center().x, top), Align2::CENTER_TOP, t, FontId::proportional(13.0), fg);
        top += 20.0;
    }
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 58.0, top + 6.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 34.0));
    let first = &chart.layers[0];
    let ydisp = first.ys.q.disp.clone();
    let categorical = matches!(first.xs, Xs::Text(_)) || chart.layers.iter().any(|l| l.mark == Mark::Bar);
    let xdisp = match &first.xs {
        Xs::Num(n) => Some(n.q.disp.clone()),
        Xs::Text(_) => None,
    };
    let x_is_date = matches!(&first.xs, Xs::Num(n) if n.q.disp.is_date() && n.q.absolute.is_some());

    // domains in display units
    let (mut ylo, mut yhi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut xlo, mut xhi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut ncat = 0usize;
    for l in &chart.layers {
        for v in l.ys.data.iter() {
            let s = ydisp.to_display(*v);
            if !s.is_finite() {
                continue;
            }
            ylo = ylo.min(s);
            yhi = yhi.max(s);
        }
        match &l.xs {
            Xs::Num(n) => {
                ncat = ncat.max(n.len());
                if let Some(d) = &xdisp {
                    for v in n.data.iter() {
                        let s = d.to_display(*v);
                        if !s.is_finite() {
                            continue;
                        }
                        xlo = xlo.min(s);
                        xhi = xhi.max(s);
                    }
                }
            }
            Xs::Text(t) => ncat = ncat.max(t.data.len()),
        }
    }
    if chart.layers.iter().any(|l| l.mark == Mark::Bar) {
        ylo = ylo.min(0.0);
        yhi = yhi.max(0.0);
    }
    let ((y0, y1), (x0, x1)) = fixed.unwrap_or_else(|| (pad(ylo, yhi), pad(xlo, xhi)));
    let yaxis = YAxis { plot, y0, y1, disp: ydisp.clone(), x0, x1, xdisp: xdisp.clone().filter(|_| !categorical), anchor: None };
    let xs = |v: f64| yaxis.x_to_screen(v);
    let cat_x = |i: usize| plot.left() + (i as f32 + 0.5) / ncat.max(1) as f32 * plot.width();

    // grid + y ticks
    for t in ticks(y0, y1, (plot.height() / 40.0).max(2.0) as f64) {
        let y = yaxis.to_screen(t);
        p.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0, grid));
        p.text(Pos2::new(plot.left() - 6.0, y), Align2::RIGHT_CENTER, short(t), small.clone(), muted);
    }
    // x ticks
    if categorical {
        let labels: Vec<String> = match &first.xs {
            Xs::Text(t) => t.data.iter().map(|s| s.to_string()).collect(),
            Xs::Num(n) => (0..n.len()).map(|i| n.fmt_elem(i)).collect(),
        };
        let every = (labels.len() as f32 * 50.0 / plot.width()).ceil().max(1.0) as usize;
        for (i, l) in labels.iter().enumerate().step_by(every) {
            p.text(Pos2::new(cat_x(i), plot.bottom() + 4.0), Align2::CENTER_TOP, l, small.clone(), muted);
        }
    } else {
        let target = (plot.width() / if x_is_date { 90.0 } else { 60.0 }).max(2.0) as f64;
        for t in ticks(x0, x1, target) {
            let x = xs(t);
            p.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.0, grid));
            let label = if x_is_date { fmt_date(t) } else { short(t) };
            p.text(Pos2::new(x, plot.bottom() + 4.0), Align2::CENTER_TOP, label, small.clone(), muted);
        }
    }
    p.line_segment([plot.left_bottom(), plot.right_bottom()], Stroke::new(1.0, muted));
    p.line_segment([plot.left_top(), plot.left_bottom()], Stroke::new(1.0, muted));
    // axis labels (units by default)
    let xl = chart.x_label();
    if !xl.is_empty() {
        p.text(Pos2::new(plot.center().x, rect.bottom() - 4.0), Align2::CENTER_BOTTOM, xl, small.clone(), muted);
    }
    let yl = chart.y_label();
    if !yl.is_empty() {
        let galley = p.layout_no_wrap(yl, small.clone(), muted);
        let pos = Pos2::new(rect.left() + 4.0, plot.center().y + galley.size().x / 2.0);
        p.add(TextShape::new(pos, galley, muted).with_angle(-std::f32::consts::FRAC_PI_2));
    }

    // layers
    let mut hits = Vec::new();
    let nbars = chart.layers.iter().filter(|l| l.mark == Mark::Bar).count().max(1);
    let mut bar_i = 0;
    let clip = p.with_clip_rect(plot.expand(4.0));
    for (li, l) in chart.layers.iter().enumerate() {
        let color = SERIES[li % SERIES.len()];
        // Non-finite values (NaN/inf, e.g. from a divide by zero) have no position; egui panics on NaN geometry.
        let pts: Vec<Option<Pos2>> = (0..l.ys.len())
            .map(|i| {
                let x = match (&l.xs, categorical) {
                    (Xs::Num(n), false) => xs(xdisp.as_ref().unwrap().to_display(n.data[i])),
                    _ => cat_x(i),
                };
                let y = yaxis.to_screen(ydisp.to_display(l.ys.data[i]));
                (x.is_finite() && y.is_finite()).then(|| Pos2::new(x, y))
            })
            .collect();
        let prov = |i: usize| l.ys.prov.get(i);
        let two_d = matches!(l.mark, Mark::Scatter | Mark::Path) && !categorical;
        let xprov = |i: usize| match &l.xs {
            Xs::Num(n) if two_d => n.prov.get(i),
            _ => Prov::None,
        };
        // a line with points closer than a few pixels apart is drawn without its markers
        // (they'd hide it and cost a mesh each); the points still answer the pointer
        let markers = matches!(l.mark, Mark::Scatter | Mark::Path) || (pts.len() as f32) * 4.0 < plot.width();
        match l.mark {
            Mark::Line | Mark::Scatter | Mark::Path => {
                if l.mark != Mark::Scatter {
                    for run in pts.split(|p| p.is_none()) {
                        let run: Vec<Pos2> = run.iter().flatten().copied().collect();
                        if run.len() > 1 {
                            clip.line(run, Stroke::new(2.0, color));
                        }
                    }
                }
                for (i, pt) in pts.iter().enumerate() {
                    let Some(pt) = pt else { continue };
                    if markers {
                        let draggable = matches!(prov(i), Prov::Literal(_)) || matches!(xprov(i), Prov::Literal(_));
                        let r = match l.mark {
                            Mark::Scatter => 4.0,
                            Mark::Path => 3.0,
                            _ => 2.5,
                        };
                        if draggable {
                            clip.circle_filled(*pt, r + 1.5, bg);
                            clip.circle_stroke(*pt, r + 1.5, Stroke::new(2.0, color));
                        } else {
                            clip.circle_filled(*pt, r, color);
                        }
                    }
                    hits.push(PointHit { pos: *pt, prov: prov(i), xprov: xprov(i), two_d, layer: li, index: i });
                }
            }
            Mark::Bar => {
                let slot = plot.width() / ncat.max(1) as f32;
                let bw = (slot * 0.7 / nbars as f32).max(1.0);
                let base = yaxis.to_screen(0.0f64.clamp(y0, y1));
                for (i, pt) in pts.iter().enumerate() {
                    let Some(pt) = pt else { continue };
                    let x = pt.x - slot * 0.35 + bw * bar_i as f32;
                    let r = Rect::from_min_max(Pos2::new(x, pt.y.min(base)), Pos2::new(x + bw - 1.0, pt.y.max(base)));
                    clip.rect_filled(r, 2.0, color);
                    let top = Pos2::new(x + bw / 2.0, pt.y);
                    if matches!(prov(i), Prov::Literal(_)) {
                        clip.line_segment([top - Vec2::new(bw / 2.0 - 2.0, 0.0), top + Vec2::new(bw / 2.0 - 3.0, 0.0)], Stroke::new(3.0, fg));
                    }
                    hits.push(PointHit { pos: top, prov: prov(i), xprov: Prov::None, two_d: false, layer: li, index: i });
                }
                bar_i += 1;
            }
        }
    }
    (yaxis, hits)
}

/// A line of a [`tooltip_lines`] tooltip; the kind sets how loud it is.
pub enum Tip {
    /// The data under the pointer: largest and brightest, read first.
    Value(String),
    /// What you can do here: the accent colour.
    Action(String),
    /// Where the data came from, or why you can't act on it: small and dim.
    Note(String),
}

/// A plain tooltip: one line style, for a single fact (like a number too wide for its cell).
pub fn tooltip(ctx: &egui::Context, pos: Pos2, text: &str) {
    tooltip_lines(ctx, pos, &[Tip::Note(text.to_string())]);
}

/// A tooltip that ranks its lines: the value first and loudest, then the actions, then notes.
pub fn tooltip_lines(ctx: &egui::Context, pos: Pos2, lines: &[Tip]) {
    use egui::text::{LayoutJob, TextFormat};
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("wbs_tip")));
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: 320.0, ..Default::default() }, ..Default::default() };
    for (i, l) in lines.iter().enumerate() {
        let (text, format) = match l {
            Tip::Value(t) => (t, TextFormat::simple(FontId::proportional(13.5), Color32::WHITE)),
            Tip::Action(t) => (t, TextFormat::simple(FontId::proportional(12.0), Color32::from_rgb(0x7d, 0xd3, 0xfc))),
            Tip::Note(t) => (t, TextFormat::simple(FontId::proportional(11.0), Color32::from_gray(165))),
        };
        let sep = if i == 0 { "" } else { "\n" };
        job.append(&format!("{sep}{text}"), 0.0, format);
    }
    let galley = p.layout_job(job);
    let r = Rect::from_min_size(pos + Vec2::new(14.0, 14.0), galley.size() + Vec2::splat(12.0));
    p.rect_filled(r, 5.0, Color32::from_rgba_unmultiplied(20, 22, 28, 235));
    p.galley(r.min + Vec2::splat(6.0), galley, Color32::WHITE);
}
