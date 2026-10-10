//! The app icon, drawn here rather than shipped as an image: a dark tile with a few grid
//! lines, a highlighted input cell, and a curve through it with one point being dragged.
//! The same idea as the app: cells, a chart, and something to grab.

use eframe::egui::IconData;

const SIZE: u32 = 256;
/// Samples per pixel along each axis, for antialiased edges.
const SS: u32 = 4;

type Rgba = [f32; 4];

const BG: Rgba = [0.09, 0.10, 0.13, 1.0];
const GRID: Rgba = [0.22, 0.25, 0.31, 1.0];
const INPUT: Rgba = [0.98, 0.80, 0.25, 0.30];
const LINE: Rgba = [0.15, 0.39, 0.92, 1.0];
const POINT: Rgba = [0.98, 0.45, 0.09, 1.0];
const RING: Rgba = [1.0, 1.0, 1.0, 1.0];

pub fn icon() -> IconData {
    IconData { rgba: rgba(SIZE), width: SIZE, height: SIZE }
}

/// The icon at `size`×`size`, as straight (not premultiplied) RGBA bytes.
pub fn rgba(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let scale = 256.0 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                    let c = sample(px, py);
                    // accumulate premultiplied so edges blend correctly
                    for i in 0..3 {
                        acc[i] += c[i] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let px = if a > 0.0 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], a] } else { [0.0; 4] };
            out.extend(px.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    out
}

/// Colour at a point of the 256×256 design.
fn sample(x: f32, y: f32) -> Rgba {
    // the tile: a rounded square with a margin, as macOS and most docks expect
    let (m, r) = (16.0, 52.0);
    if !in_round_rect(x, y, m, m, 256.0 - m, 256.0 - m, r) {
        return [0.0; 4];
    }
    let mut c = BG;
    // the input cell, tinted like inputs in the grid
    if (60.0..128.0).contains(&x) && (164.0..196.0).contains(&y) {
        c = over(c, INPUT);
    }
    // grid lines: three columns, five rows
    for gx in [60.0, 128.0, 196.0] {
        if (x - gx).abs() < 1.5 && (36.0..220.0).contains(&y) {
            c = over(c, GRID);
        }
    }
    for gy in [68.0, 100.0, 132.0, 164.0, 196.0] {
        if (y - gy).abs() < 1.5 && (36.0..220.0).contains(&x) {
            c = over(c, GRID);
        }
    }
    // a rising curve, like a model's output
    let curve = |t: f32| 188.0 - 120.0 * (t / 168.0).powf(1.8);
    if (44.0..=212.0).contains(&x) {
        // distance to the curve, measured along the normal
        let t = x - 44.0;
        let dy = curve(t + 0.5) - curve(t - 0.5);
        let d = (y - curve(t)).abs() / (1.0 + dy * dy).sqrt();
        if d < 6.0 {
            c = over(c, LINE);
        }
    }
    // the point being dragged: orange, with a white ring
    let (pt, pr) = ((170.0, curve(170.0 - 44.0)), 17.0);
    let d = ((x - pt.0).powi(2) + (y - pt.1).powi(2)).sqrt();
    if d < pr + 5.0 {
        c = over(c, RING);
    }
    if d < pr {
        c = over(c, POINT);
    }
    c
}

fn in_round_rect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r && (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
}

/// `top` over `under`, both straight alpha.
fn over(under: Rgba, top: Rgba) -> Rgba {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a == 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0; 4];
    for i in 0..3 {
        out[i] = (top[i] * top[3] + under[i] * under[3] * (1.0 - top[3])) / a;
    }
    out[3] = a;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_a_tile_with_transparent_corners() {
        let i = icon();
        assert_eq!(i.rgba.len(), (i.width * i.height * 4) as usize);
        let at = |x: u32, y: u32| &i.rgba[((y * i.width + x) * 4) as usize..][..4];
        assert_eq!(at(0, 0)[3], 0, "corner outside the tile");
        assert_eq!(at(128, 30)[3], 255, "inside the tile");
    }
}
