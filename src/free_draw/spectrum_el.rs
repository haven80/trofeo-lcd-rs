//! The `spectrum` element of a free layout (3.0.0): the music's bars in a row, a column or
//! around a circle, of one channel or both, of all the sounds or only the low, middle or high
//! ones, drawn in the styles of the main spectrum.

use super::*;
use trofeo_lcd::free::{SpecBand, SpecChannel, SpecShape, SpecStyle};

/// The bars the element asks for, ready to draw: `(values, peaks)` in the order they are
/// drawn (left to right, top to bottom, clockwise), 0-1 each.
pub fn bars_of(e: &Element, feed: Option<&spectrum::Feed>) -> (Vec<f32>, Vec<f32>) {
    let n = (e.bars as usize).clamp(4, 96);
    let (vals, peaks) = match feed {
        Some(f) => match e.channel {
            SpecChannel::Mix => (&f.mix, &f.mix_peaks),
            SpecChannel::Left => (&f.left, &f.left_peaks),
            SpecChannel::Right => (&f.right, &f.right_peaks),
        },
        None => return (vec![0.0; n], vec![0.0; n]),
    };
    let total = vals.len();
    if total == 0 {
        return (vec![0.0; n], vec![0.0; n]);
    }
    let (lo, hi) = match e.band {
        SpecBand::All => (0, total),
        SpecBand::Bass => (0, total / 3),
        SpecBand::Mid => (total / 3, total * 2 / 3),
        SpecBand::Treble => (total * 2 / 3, total),
    };
    // mirrored: half the bars, the low sounds in the middle of a row (at the start of a circle)
    let half = if e.sym { (n / 2).max(2) } else { n };
    let a = resample(&vals[lo..hi], half);
    let p = resample(&peaks[lo.min(peaks.len())..hi.min(peaks.len())], half);
    if !e.sym {
        return (a, p);
    }
    let mirror = |h: Vec<f32>| -> Vec<f32> {
        if e.shape == SpecShape::Circle {
            h.iter().copied().chain(h.iter().rev().copied()).collect()
        } else {
            h.iter().rev().copied().chain(h.iter().copied()).collect()
        }
    };
    (mirror(a), mirror(p))
}

/// `n` values from a slice: the loudest of the ones that fall in a bar, or a blend of the
/// neighbours when there are more bars than values.
pub fn resample(src: &[f32], n: usize) -> Vec<f32> {
    if src.is_empty() {
        return vec![0.0; n];
    }
    let len = src.len() as f32;
    (0..n)
        .map(|i| {
            let (a, b) = (i as f32 * len / n as f32, (i + 1) as f32 * len / n as f32);
            if b - a >= 1.0 {
                let (ia, ib) = (a.floor() as usize, (b.ceil() as usize).min(src.len()));
                src[ia..ib.max(ia + 1).min(src.len())].iter().copied().fold(0.0, f32::max)
            } else {
                let pos = ((a + b) / 2.0 - 0.5).clamp(0.0, len - 1.0);
                let (i0, f) = (pos.floor() as usize, pos.fract());
                let i1 = (i0 + 1).min(src.len() - 1);
                src[i0] * (1.0 - f) + src[i1] * f
            }
        })
        .collect()
}

struct Paint<'a> {
    e: &'a Element,
    /// The colour of the first bar, of the last one, and where the moving gradient is (< 0 = still).
    c1: Option<Rgb>,
    c2: Option<Rgb>,
    phase: f32,
}

impl Paint<'_> {
    /// The colour of bar `i` of `n` at a height `level`.
    fn at(&self, level: f32, pos: f32) -> Rgb {
        if self.e.hot.is_some_and(|h| level * 100.0 >= h) {
            return self.e.hot_color;
        }
        match (self.c1, self.c2) {
            (Some(a), Some(b)) => gradient_at(a, b, pos, self.phase),
            (Some(a), None) => a,
            _ => spectrum::classic(level.clamp(0.0, 1.0)),
        }
    }
}

fn lighten(c: Rgb, k: f32) -> Rgb {
    let m = |v: u8| (v as f32 + (255.0 - v as f32) * k).round() as u8;
    (m(c.0), m(c.1), m(c.2))
}

/// Draw the spectrum element `e` in the part of the screen `area`.
pub fn draw(fb: &mut Framebuffer, area: (u32, u32, u32, u32), e: &Element, vals: &[f32], peaks: &[f32]) {
    let (ax, ay, aw, ah) = (area.0 as f32, area.1 as f32, area.2 as f32, area.3 as f32);
    let (w_px, h_px) = ((e.w / 100.0 * aw).round(), (e.h / 100.0 * ah).round());
    if w_px < 2.0 || h_px < 2.0 || vals.is_empty() {
        return;
    }
    let (px, py) = free::place(e, aw, ah, w_px, h_px);
    let rect = (ax + px, ay + py, w_px, h_px);
    let phase = if e.color2.is_some() && e.flow > 0.0 && opts().bar_animation { ((anim_secs() / e.flow as f64).fract()) as f32 } else { -1.0 };
    let paint = Paint { e, c1: e.color.or(e.color2.map(|_| (0x00, 0xD4, 0xFF))), c2: e.color2, phase };
    match e.shape {
        SpecShape::Circle => circle(fb, area, rect, e, vals, peaks, &paint),
        shape => linear(fb, area, rect, e, vals, peaks, &paint, shape == SpecShape::Column),
    }
}

/// A row (bars grow upwards, or downwards when flipped) or a column (bars grow to the right,
/// or to the left when flipped). `u` runs along the bars, `v` in the direction they grow.
#[allow(clippy::too_many_arguments)]
fn linear(fb: &mut Framebuffer, area: (u32, u32, u32, u32), rect: (f32, f32, f32, f32), e: &Element, vals: &[f32], peaks: &[f32], paint: &Paint, column: bool) {
    let (x, y, w, h) = rect;
    let (u_len, v_len) = if column { (h, w) } else { (w, h) };
    let n = vals.len();
    let pitch = u_len / n as f32;
    let gap = if e.style == SpecStyle::Area { 0.0 } else { e.gap / 100.0 };
    let bar = (pitch * (1.0 - gap)).max(1.0);
    // a rectangle in the local space (u0..u1 along, v0..v1 growth from the base) -> the screen
    let put = |fb: &mut Framebuffer, u0: f32, u1: f32, v0: f32, v1: f32, c: Rgb, alpha: f32| {
        let (sx, sy, sw, sh) = if column {
            let vx = if e.flip { x + w - v1 } else { x + v0 };
            (vx, y + u0, v1 - v0, u1 - u0)
        } else {
            let vy = if e.flip { y + v0 } else { y + h - v1 };
            (x + u0, vy, u1 - u0, v1 - v0)
        };
        blend_rect(fb, area, sx, sy, sw, sh, c, alpha);
    };
    let mid = v_len / 2.0;
    for (i, &raw) in vals.iter().enumerate() {
        let v = raw.clamp(0.0, 1.0);
        let pos = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
        let u0 = i as f32 * pitch + (pitch - bar) / 2.0;
        let c = paint.at(v, pos);
        match e.style {
            SpecStyle::Bars | SpecStyle::Peaks => {
                let top = (v_len * v).max(1.0);
                put(fb, u0, u0 + bar, 0.0, top, c, 1.0);
                if e.style == SpecStyle::Peaks {
                    let p = peaks.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let cap = (v_len / 60.0).clamp(2.0, 6.0);
                    let at = (v_len * p).max(cap).min(v_len);
                    put(fb, u0, u0 + bar, at - cap, at, lighten(paint.at(p, pos), 0.35), 1.0);
                }
            }
            SpecStyle::Led => {
                let seg = (v_len / 20.0).max(4.0);
                let nseg = ((v_len / seg).floor() as usize).max(1);
                let lit = (v * nseg as f32).round() as usize;
                for k in 0..nseg {
                    let level = (k as f32 + 0.5) / nseg as f32;
                    let c = paint.at(level, pos);
                    let (v0, v1) = (k as f32 * seg + 1.0, (k as f32 + 1.0) * seg - 1.0);
                    if k < lit {
                        put(fb, u0, u0 + bar, v0, v1, c, 1.0);
                    } else {
                        put(fb, u0, u0 + bar, v0, v1, c, 0.12);
                    }
                }
            }
            SpecStyle::Area => {} // drawn column by column after the loop
            SpecStyle::Mirror => {
                let half = (mid * v).max(0.5);
                put(fb, u0, u0 + bar, mid - half, mid + half, c, 1.0);
            }
        }
    }
    if e.style == SpecStyle::Area && n > 1 {
        // a smooth hill: the height between the centres of two bars is a blend of both
        let cols = u_len.ceil() as usize;
        for col in 0..cols {
            let u = col as f32 + 0.5;
            let f = ((u - pitch / 2.0) / pitch).clamp(0.0, (n - 1) as f32);
            let (i0, fr) = (f.floor() as usize, f.fract());
            let i1 = (i0 + 1).min(n - 1);
            let v = (vals[i0] * (1.0 - fr) + vals[i1] * fr).clamp(0.0, 1.0);
            let c = paint.at(v, f / (n - 1) as f32);
            let top = (v_len * v).max(1.0);
            put(fb, col as f32, col as f32 + 1.0, 0.0, top, c, 0.55);
            put(fb, col as f32, col as f32 + 1.0, (top - 2.0).max(0.0), top, lighten(c, 0.3), 1.0);
        }
    }
}


/// Bars around a circle: the first at `rot` degrees clockwise from the top, growing outwards
/// from the hole (or inwards from the edge when flipped).
fn circle(fb: &mut Framebuffer, area: (u32, u32, u32, u32), rect: (f32, f32, f32, f32), e: &Element, vals: &[f32], peaks: &[f32], paint: &Paint) {
    let (x, y, w, h) = rect;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let radius = w.min(h) / 2.0;
    let r0 = radius * e.inner / 100.0;
    let span = (radius - r0).max(1.0);
    let n = vals.len();
    let gap = if e.style == SpecStyle::Area { 0.0 } else { e.gap / 100.0 };
    let Some((x0, y0, cw, ch)) = clip(fb, area, cx - radius - 1.0, cy - radius - 1.0, radius * 2.0 + 2.0, radius * 2.0 + 2.0) else { return };
    let stride = fb.width() as usize;
    let rot = e.rot / 360.0;
    let px = fb.as_bytes_mut();
    for yy in y0..y0 + ch {
        for xx in x0..x0 + cw {
            let (dx, dy) = (xx as f32 + 0.5 - cx, yy as f32 + 0.5 - cy);
            let dist = dx.hypot(dy);
            if dist > radius + 0.5 || dist < r0 - 0.5 {
                continue;
            }
            let mut turn = dx.atan2(-dy) / std::f32::consts::TAU - rot;
            turn = turn.rem_euclid(1.0);
            let slot = turn * n as f32;
            let i = (slot.floor() as usize).min(n - 1);
            let within = slot - i as f32;
            if within < gap / 2.0 || within > 1.0 - gap / 2.0 {
                continue;
            }
            // distance from the base of the bars, outwards (inwards when flipped)
            let d = if e.flip { radius - dist } else { dist - r0 };
            let v = if e.style == SpecStyle::Area {
                let j = (i + 1) % n;
                let f = within;
                (vals[i] * (1.0 - f) + vals[j] * f).clamp(0.0, 1.0)
            } else {
                vals[i].clamp(0.0, 1.0)
            };
            let pos = i as f32 / n.max(1) as f32;
            let c = paint.at(v, pos);
            let edge = |len: f32| (len + 0.5 - d).clamp(0.0, 1.0) * (d + 0.5).clamp(0.0, 1.0);
            let (col, alpha) = match e.style {
                SpecStyle::Bars | SpecStyle::Area => (c, edge((span * v).max(1.0))),
                SpecStyle::Peaks => {
                    let p = peaks.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let cap = (span / 40.0).clamp(2.0, 5.0);
                    let top = (span * p).max(cap).min(span);
                    if (d - top).abs() <= cap / 2.0 + 0.5 && d > span * v - 0.5 {
                        (lighten(paint.at(p, pos), 0.35), 1.0)
                    } else {
                        (c, edge((span * v).max(1.0)))
                    }
                }
                SpecStyle::Led => {
                    let seg = (span / 12.0).max(4.0);
                    let k = (d / seg).floor();
                    if d - k * seg < 1.0 || d - k * seg > seg - 1.0 {
                        continue;
                    }
                    let nseg = (span / seg).floor().max(1.0);
                    let lit = (v * nseg).round();
                    (paint.at((k + 0.5) / nseg, pos), if k < lit { 1.0 } else { 0.12 })
                }
                SpecStyle::Mirror => {
                    let rm = span / 2.0;
                    (c, (rm * v + 0.5 - (d - rm).abs()).clamp(0.0, 1.0))
                }
            };
            if alpha <= 0.0 {
                continue;
            }
            let i = (yy as usize * stride + xx as usize) * 3;
            mix(&mut px[i..i + 3], col, alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(line: &str) -> Element {
        free::parse(line).unwrap().elements.remove(0)
    }

    fn feed() -> spectrum::Feed {
        let ramp = |a: f32, b: f32| -> Vec<f32> { (0..48).map(|i| a + (b - a) * i as f32 / 47.0).collect() };
        spectrum::Feed {
            mix: ramp(0.5, 0.5),
            left: ramp(1.0, 0.0),
            right: ramp(0.0, 1.0),
            mix_peaks: ramp(0.6, 0.6),
            left_peaks: ramp(1.0, 0.0),
            right_peaks: ramp(0.0, 1.0),
        }
    }

    #[test]
    fn resampling_keeps_the_loudest_and_blends_when_there_are_more_bars() {
        assert_eq!(resample(&[0.1, 0.9, 0.2, 0.3], 2), vec![0.9, 0.3]);
        let up = resample(&[0.0, 1.0], 4);
        assert_eq!(up.len(), 4);
        assert!(up.windows(2).all(|w| w[0] <= w[1]), "{up:?}");
        assert_eq!(resample(&[], 3), vec![0.0; 3]);
    }

    #[test]
    fn a_spectrum_takes_its_channel_and_band() {
        let f = feed();
        let (left, _) = bars_of(&element("spectrum x=0 y=0 channel=left bars=8"), Some(&f));
        let (right, _) = bars_of(&element("spectrum x=0 y=0 channel=right bars=8"), Some(&f));
        assert!(left[0] > left[7] && right[0] < right[7], "left falls, right rises");
        let (mix, _) = bars_of(&element("spectrum x=0 y=0 bars=8"), Some(&f));
        assert!(mix.iter().all(|v| (*v - 0.5).abs() < 1e-5));
        // bass is the first third of the bars, treble the last: the left channel is louder in the bass
        let (bass, _) = bars_of(&element("spectrum x=0 y=0 channel=left band=bass bars=8"), Some(&f));
        let (treble, _) = bars_of(&element("spectrum x=0 y=0 channel=left band=treble bars=8"), Some(&f));
        assert!(bass.iter().sum::<f32>() > treble.iter().sum::<f32>() * 2.0);
        // no music: silence, with the number of bars asked for
        assert_eq!(bars_of(&element("spectrum x=0 y=0 bars=12"), None).0, vec![0.0; 12]);
    }

    #[test]
    fn a_mirrored_spectrum_has_the_low_sounds_in_the_middle() {
        let f = feed();
        let (v, _) = bars_of(&element("spectrum x=0 y=0 channel=left bars=16 sym=true"), Some(&f));
        assert_eq!(v.len(), 16);
        assert_eq!(v[..8].iter().rev().copied().collect::<Vec<_>>(), v[8..].to_vec(), "the two halves mirror");
        assert!(v[8] > v[15], "the low sounds (loud on the left channel) are in the middle");
        // round: the low sounds are at the start and the end of the circle
        let (c, _) = bars_of(&element("spectrum x=0 y=0 channel=left bars=16 sym=true shape=circle"), Some(&f));
        assert!(c[0] > c[7] && c[15] > c[8]);
    }

    fn lit(fb: &mut Framebuffer) -> usize {
        fb.as_bytes_mut().chunks(3).filter(|p| p.iter().any(|b| *b != 0)).count()
    }

    #[test]
    fn every_shape_and_style_draws_something_inside_its_box() {
        let f = feed();
        for shape in ["row", "column", "circle"] {
            for style in ["bars", "led", "peaks", "area", "mirror"] {
                let e = element(&format!("spectrum x=10 y=10 w=80 h=80 shape={shape} style={style} bars=20 color=#FF0000"));
                let (v, p) = bars_of(&e, Some(&f));
                let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(400, 400));
                draw(&mut fb, (0, 0, 400, 400), &e, &v, &p);
                let n = lit(&mut fb);
                assert!(n > 200, "{shape} {style}: {n} pixels");
                // nothing outside the box (10%..90%)
                let bytes = fb.as_bytes_mut();
                for y in 0..400usize {
                    for x in 0..400usize {
                        if x < 38 || x > 362 || y < 38 || y > 362 {
                            let i = (y * 400 + x) * 3;
                            assert_eq!(&bytes[i..i + 3], &[0, 0, 0], "{shape} {style} at {x},{y}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_flipped_row_grows_downwards_and_a_hot_bar_changes_colour() {
        let e = element("spectrum x=0 y=0 w=100 h=100 bars=4 gap=0 color=#00FF00 hot=50 hot_color=#FF0000");
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(100, 100));
        draw(&mut fb, (0, 0, 100, 100), &e, &[1.0, 0.2, 1.0, 0.2], &[0.0; 4]);
        let px = |fb: &mut Framebuffer, x: usize, y: usize| {
            let i = (y * 100 + x) * 3;
            let b = fb.as_bytes_mut();
            (b[i], b[i + 1], b[i + 2])
        };
        assert_eq!(px(&mut fb, 10, 90), (255, 0, 0), "a loud bar is hot, from the bottom");
        assert_eq!(px(&mut fb, 35, 95), (0, 255, 0), "a quiet one is not");
        assert_eq!(px(&mut fb, 35, 10), (0, 0, 0));
        let e = element("spectrum x=0 y=0 w=100 h=100 bars=4 gap=0 flip=true color=#00FF00");
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(100, 100));
        draw(&mut fb, (0, 0, 100, 100), &e, &[0.2; 4], &[0.0; 4]);
        assert_ne!(px(&mut fb, 10, 5), (0, 0, 0), "flipped: from the top");
        assert_eq!(px(&mut fb, 10, 95), (0, 0, 0));
    }
}
