//! The "music screen": while a track plays, the whole panel becomes a
//! now-playing card — album cover on the left (with a soft glow in the
//! cover's own color), artist / title / album next to it, a progress bar with
//! times, and the spectrum on the side, tinted with the cover's accent color.
//! The backdrop is the cover itself, blurred and darkened.
//!
//! Everything expensive (blurring, scaling, rounding, glow, accent color) is
//! done ONCE when the cover changes and baked into a cached backdrop; a frame
//! is then one memcpy plus a few text/bar draws.
//!
//! Landscape: `[cover] [text ............] [spectrum]`.
//! Portrait:  cover on top, text below it, spectrum at the bottom.
//! Without a cover (player publishes none) a stylised record is drawn instead.

use crate::media::{CoverArt, TrackInfo};
use crate::spectrum::{self, Rgb};
use crate::{fit_scale, Marquee, UiOptions, MARQUEE_GAP};
use image::imageops::{self, FilterType};
use image::RgbImage;
use std::sync::Arc;
use trofeo_lcd::{Framebuffer, Resolution};

/// Used when there is no cover to take a color from (Spotify green).
pub const DEFAULT_ACCENT: Rgb = (0x1D, 0xB9, 0x54);
/// Transparent key for the cover when it is blitted without a backdrop.
const KEY: Rgb = (1, 0, 2);
/// The spectrum style used here unless `spectrum_style` is configured.
pub const DEFAULT_STYLE: spectrum::Style = spectrum::Style::Bars;

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub cover: Rect,
    pub text: Rect,
    pub spectrum: Rect,
}

/// Where everything goes on a `w` x `h` canvas.
pub fn geometry(w: u32, h: u32) -> Geometry {
    let p = (w.min(h) * 8 / 100).max(6);
    if w >= h {
        let cs = h.saturating_sub(2 * p).max(1);
        let text_x = p + cs + p;
        let avail = w.saturating_sub(text_x + p);
        let spec_w = avail * 40 / 100;
        let spec_x = w.saturating_sub(p + spec_w);
        let text_w = spec_x.saturating_sub(text_x + p);
        Geometry {
            cover: Rect { x: p, y: p, w: cs, h: cs },
            text: Rect { x: text_x, y: p, w: text_w, h: cs },
            spectrum: Rect { x: spec_x, y: p, w: spec_w, h: cs },
        }
    } else {
        let cs = w.saturating_sub(2 * p).max(1);
        let y0 = p + cs + p;
        let rest = h.saturating_sub(y0 + p);
        let text_h = rest * 45 / 100;
        Geometry {
            cover: Rect { x: p, y: p, w: cs, h: cs },
            text: Rect { x: p, y: y0, w: cs, h: text_h },
            spectrum: Rect { x: p, y: y0 + text_h + p, w: cs, h: rest.saturating_sub(text_h + p) },
        }
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| lerp(x as f32, y as f32, t).round().clamp(0.0, 255.0) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn scale_rgb(c: Rgb, f: f32) -> Rgb {
    let g = |x: u8| (x as f32 * f).round().clamp(0.0, 255.0) as u8;
    (g(c.0), g(c.1), g(c.2))
}

/// A vivid accent color for a cover: the average of its pixels weighted toward
/// the saturated, bright ones (so a big grey border doesn't win), then pushed
/// to a saturation/brightness that stays readable on a dark background.
/// Black-and-white covers give a near-white accent; empty input the default.
pub fn accent_from_rgb(rgb: &[u8]) -> Rgb {
    let (mut r, mut g, mut b, mut wsum) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for px in rgb.chunks_exact(3) {
        let (_, s, v) = spectrum::rgb_to_hsv((px[0], px[1], px[2]));
        let w = ((s * v) as f64).powi(2) + 0.01;
        r += px[0] as f64 * w;
        g += px[1] as f64 * w;
        b += px[2] as f64 * w;
        wsum += w;
    }
    if wsum == 0.0 {
        return DEFAULT_ACCENT;
    }
    let avg = ((r / wsum) as u8, (g / wsum) as u8, (b / wsum) as u8);
    let (h, s, v) = spectrum::rgb_to_hsv(avg);
    if s < 0.12 {
        return spectrum::hsv(h, s, v.max(0.85));
    }
    spectrum::hsv(h, s.clamp(0.55, 1.0), v.max(0.9))
}

/// The accent as a quiet -> loud gradient for the spectrum.
fn accent_stops(accent: Rgb) -> Vec<Rgb> {
    vec![scale_rgb(accent, 0.45), accent, mix(accent, (255, 255, 255), 0.55)]
}

// ---------------------------------------------------------------------------
// Cached backdrop (cover + blurred background + glow)
// ---------------------------------------------------------------------------

struct Cache {
    cover: Option<Arc<CoverArt>>,
    size: (u32, u32),
    blur: bool,
    accent: Rgb,
    /// Fully composed background (blurred cover, glow, the cover itself).
    backdrop: Option<Framebuffer>,
    /// The cover with transparent corners, for when there is no backdrop.
    keyed_cover: Option<Framebuffer>,
}

fn new_fb(w: u32, h: u32) -> Framebuffer {
    Framebuffer::new(Resolution { width: w.max(1), height: h.max(1) })
}

fn put(fb: &mut Framebuffer, x: u32, y: u32, c: Rgb) {
    fb.set_pixel(x, y, c.0, c.1, c.2);
}

fn get(fb: &Framebuffer, x: u32, y: u32) -> Rgb {
    let i = ((y * fb.width() + x) * 3) as usize;
    let b = fb.as_bytes();
    (b[i], b[i + 1], b[i + 2])
}

/// How much of pixel (x, y) of a `size` x `size` square with corner radius `r`
/// is inside the rounded shape (0-1, anti-aliased at the corners).
fn corner_coverage(x: u32, y: u32, size: u32, r: f32) -> f32 {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let s = size as f32;
    let cx = if fx < r { r } else if fx > s - r { s - r } else { return 1.0 };
    let cy = if fy < r { r } else if fy > s - r { s - r } else { return 1.0 };
    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0.0, 1.0)
}

/// The record drawn when a track has no cover.
fn placeholder(size: u32, accent: Rgb) -> RgbImage {
    let s = size as f32;
    RgbImage::from_fn(size, size, |x, y| {
        let (fx, fy) = (x as f32 + 0.5 - s / 2.0, y as f32 + 0.5 - s / 2.0);
        let d = (fx * fx + fy * fy).sqrt() / s; // 0 at the centre, 0.5 at the edge-middle
        let t = (x + y) as f32 / (2.0 * s); // diagonal gradient behind the record
        let bg = mix(scale_rgb(accent, 0.55), scale_rgb(accent, 0.12), t);
        let c = if d < 0.012 {
            (10, 10, 14) // spindle hole
        } else if d < 0.11 {
            accent // label
        } else if d < 0.36 {
            // grooves: faint alternating rings
            if ((d * 160.0) as u32) % 2 == 0 { (22, 22, 28) } else { (30, 30, 38) }
        } else if d < 0.372 {
            (50, 50, 60) // rim
        } else {
            bg
        };
        image::Rgb([c.0, c.1, c.2])
    })
}

fn build_cache(w: u32, h: u32, cover: &Option<Arc<CoverArt>>, blur: bool) -> Cache {
    let g = geometry(w, h);
    let cs = g.cover.w.min(g.cover.h).max(1);

    // Tiny version of the cover: source of the accent color and of the blur.
    let tiny: Option<RgbImage> = cover.as_ref().and_then(|c| {
        RgbImage::from_raw(c.width, c.height, c.rgb.clone()).map(|img| imageops::resize(&img, 24, 24, FilterType::Triangle))
    });
    let accent = tiny.as_ref().map(|t| accent_from_rgb(t.as_raw())).unwrap_or(DEFAULT_ACCENT);

    // The square cover image at its final size (or the record placeholder).
    let cover_img: RgbImage = match cover.as_ref().and_then(|c| RgbImage::from_raw(c.width, c.height, c.rgb.clone())) {
        Some(img) => imageops::resize(&img, cs, cs, FilterType::Lanczos3),
        None => placeholder(cs, accent),
    };
    let radius = cs as f32 / 16.0;

    if !blur {
        let mut k = new_fb(cs, cs);
        for y in 0..cs {
            for x in 0..cs {
                let p = cover_img.get_pixel(x, y).0;
                let c = if corner_coverage(x, y, cs, radius) < 0.5 { KEY } else { (p[0], p[1], p[2]) };
                put(&mut k, x, y, c);
            }
        }
        return Cache { cover: cover.clone(), size: (w, h), blur, accent, backdrop: None, keyed_cover: Some(k) };
    }

    // Blurred, darkened background: the 24x24 tiny cover stretched over the
    // whole panel with a smooth filter is already a convincing blur.
    let mut bd = new_fb(w, h);
    let big = tiny.as_ref().map(|t| imageops::resize(t, w.max(1), h.max(1), FilterType::CatmullRom));
    for y in 0..h {
        let ny = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
        for x in 0..w {
            let nx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
            let vignette = 1.0 - 0.30 * (nx * nx + ny * ny) / 2.0;
            let base = match &big {
                Some(b) => {
                    let p = b.get_pixel(x, y).0;
                    scale_rgb((p[0], p[1], p[2]), 0.27)
                }
                // No cover: a quiet vertical wash of the default accent.
                None => mix(scale_rgb(accent, 0.20), (6, 6, 12), y as f32 / h as f32),
            };
            put(&mut bd, x, y, scale_rgb(base, vignette));
        }
    }

    // Soft glow in the accent color around the cover.
    let cr = g.cover;
    let margin = (cs / 8).clamp(8, 48) as f32;
    let glow = mix(accent, (255, 255, 255), 0.1);
    let (x0, y0) = (cr.x.saturating_sub(margin as u32), cr.y.saturating_sub(margin as u32));
    let (x1, y1) = ((cr.x + cr.w + margin as u32).min(w), (cr.y + cr.h + margin as u32).min(h));
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (cr.x as f32 - x as f32).max(x as f32 - (cr.x + cr.w) as f32).max(0.0);
            let dy = (cr.y as f32 - y as f32).max(y as f32 - (cr.y + cr.h) as f32).max(0.0);
            let d = (dx * dx + dy * dy).sqrt();
            if d < margin {
                let a = 0.55 * (1.0 - d / margin).powi(2);
                let c = mix(get(&bd, x, y), glow, a);
                put(&mut bd, x, y, c);
            }
        }
    }

    // The cover itself, with anti-aliased rounded corners.
    for y in 0..cs {
        for x in 0..cs {
            let (px, py) = (cr.x + x, cr.y + y);
            if px >= w || py >= h {
                continue;
            }
            let p = cover_img.get_pixel(x, y).0;
            let a = corner_coverage(x, y, cs, radius);
            let c = mix(get(&bd, px, py), (p[0], p[1], p[2]), a);
            put(&mut bd, px, py, c);
        }
    }
    Cache { cover: cover.clone(), size: (w, h), blur, accent, backdrop: Some(bd), keyed_cover: None }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// `83_000` ms -> `1:23`; an hour or more -> `1:02:03`.
pub fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Draw `text` with a drop shadow, scrolling it (marquee) when it is wider than `w`.
#[allow(clippy::too_many_arguments)]
fn draw_line(fb: &mut Framebuffer, m: &mut Marquee, text: &str, x: u32, y: u32, w: u32, scale: u32, color: Rgb) {
    if text.is_empty() || w == 0 {
        return;
    }
    m.scale = scale;
    let off = (scale / 4).max(1);
    let shadow = (0u8, 0u8, 0u8);
    if Framebuffer::text_width(text, scale) <= w || !m.tick(text, w) {
        fb.draw_text(x + off, y + off, text, shadow.0, shadow.1, shadow.2, scale);
        fb.draw_text(x, y, text, color.0, color.1, color.2, scale);
        return;
    }
    let loop_text = format!("{text}{MARQUEE_GAP}");
    let loop_w = Framebuffer::text_width(&loop_text, scale) as i64;
    let base = x as i64 - m.offset_px as i64;
    let x1 = x + w;
    for (dx, c) in [(off as i64, shadow), (0, color)] {
        fb.draw_text_clipped(base + dx, y + if dx == 0 { 0 } else { off }, &loop_text, c.0, c.1, c.2, scale, x, x1);
        fb.draw_text_clipped(base + dx + loop_w, y + if dx == 0 { 0 } else { off }, &loop_text, c.0, c.1, c.2, scale, x, x1);
    }
}

fn fill_circle(fb: &mut Framebuffer, cx: i64, cy: i64, r: i64, c: Rgb) {
    for dy in -r..=r {
        let half = ((r * r - dy * dy) as f64).sqrt() as i64;
        let x = (cx - half).max(0);
        let y = cy + dy;
        if y < 0 || cx + half < 0 {
            continue;
        }
        fb.fill_rect(x as u32, y as u32, (2 * half + 1) as u32, 1, c.0, c.1, c.2);
    }
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

pub struct MusicScreen {
    cache: Option<Cache>,
    title_m: Marquee,
    artist_m: Marquee,
    album_m: Marquee,
}

/// Height of the progress block: bar + gap + times line.
const PROGRESS_H: u32 = 8 + 10 + 14;

impl MusicScreen {
    pub fn new() -> Self {
        Self { cache: None, title_m: Marquee::new(), artist_m: Marquee::new(), album_m: Marquee::new() }
    }

    /// The accent color currently in use (for tests/tools).
    #[allow(dead_code)]
    pub fn accent(&self) -> Rgb {
        self.cache.as_ref().map_or(DEFAULT_ACCENT, |c| c.accent)
    }

    fn ensure(&mut self, w: u32, h: u32, cover: &Option<Arc<CoverArt>>, blur: bool) {
        let same_cover = |a: &Option<Arc<CoverArt>>, b: &Option<Arc<CoverArt>>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        let fresh = match &self.cache {
            Some(c) => c.size != (w, h) || c.blur != blur || !same_cover(&c.cover, cover),
            None => true,
        };
        if fresh {
            self.cache = Some(build_cache(w, h, cover, blur));
        }
    }

    /// Draw the whole screen onto `fb` (which is fully overwritten when the
    /// blurred backdrop is on).
    pub fn draw(&mut self, fb: &mut Framebuffer, track: &TrackInfo, heights: &[f32], peaks: &[f32], o: &UiOptions) {
        let (w, h) = (fb.width(), fb.height());
        self.ensure(w, h, &track.cover, o.music_blur);
        let g = geometry(w, h);
        let landscape = w >= h;
        let cache = self.cache.as_ref().expect("cache was just built");
        let accent = cache.accent;
        match (&cache.backdrop, &cache.keyed_cover) {
            (Some(bd), _) => fb.copy_from(bd),
            (None, Some(k)) => fb.blit_keyed(k, g.cover.x, g.cover.y, KEY),
            _ => {}
        }

        // ---- text block ----
        let t = g.text;
        let tr = crate::i18n::t();
        let label_s = fit_scale(tr.now_playing, 3, t.w);
        let label_h = 7 * label_s;
        let show_progress = o.music_progress && track.timeline.is_some();
        let pb = if show_progress { PROGRESS_H } else { 0 };

        let min_ts = if landscape { 4 } else { 3 };
        let ts = fit_scale(&track.title, 9, t.w).max(min_ts);
        let as_max = (ts * 6 / 10).clamp(3, 6);
        let as_ = fit_scale(&track.artist, as_max, t.w).max(3);
        let has_album = !track.album.trim().is_empty();
        let (title_h, artist_h, album_h) = (7 * ts, 7 * as_, if has_album { 14 } else { 0 });
        let group = title_h + 2 * ts + if track.artist.is_empty() { 0 } else { artist_h + 2 * as_ } + if has_album { album_h + 4 } else { 0 };
        let mid0 = t.y + label_h + 12;
        let mid1 = (t.y + t.h).saturating_sub(pb + 12);
        let mut y = mid0 + mid1.saturating_sub(mid0).saturating_sub(group) / 2;

        draw_label(fb, tr.now_playing, t.x, t.y, label_s, accent);
        draw_line(fb, &mut self.title_m, &track.title, t.x, y, t.w, ts, (0xF5, 0xF5, 0xF5));
        y += title_h + 2 * ts;
        if !track.artist.is_empty() {
            draw_line(fb, &mut self.artist_m, &track.artist, t.x, y, t.w, as_, mix(accent, (255, 255, 255), 0.5));
            y += artist_h + 2 * as_;
        }
        if has_album {
            draw_line(fb, &mut self.album_m, &track.album, t.x, y, t.w, 2, (0x9A, 0x9A, 0xA6));
        }

        // ---- progress ----
        if let (true, Some(tl)) = (show_progress, track.timeline.as_ref()) {
            if t.w > 40 && tl.duration_ms > 0 {
                draw_progress(fb, t, tl.position_now_ms(), tl.duration_ms, accent);
            }
        }

        // ---- spectrum ----
        let sp = g.spectrum;
        if sp.w > 8 && sp.h > 8 && !heights.is_empty() {
            let time = spectrum::now_secs();
            let style = if o.spectrum_styles.is_empty() {
                DEFAULT_STYLE
            } else {
                spectrum::pick_style(&o.spectrum_styles, o.spectrum_style_interval, time)
            };
            let own = [spectrum::Palette::Custom(accent_stops(accent))];
            let palettes: &[spectrum::Palette] = if o.spectrum_palettes.is_empty() { &own } else { &o.spectrum_palettes };
            let colors = spectrum::Colors {
                palettes,
                interval: o.spectrum_palette_interval as f32,
                rainbow_speed: o.spectrum_rainbow_speed as f32,
                time,
            };
            spectrum::draw(fb, style, (sp.x, sp.y, sp.w, sp.h), heights, peaks, &|level, pos| colors.at(level, pos));
        }
    }
}

fn draw_label(fb: &mut Framebuffer, text: &str, x: u32, y: u32, scale: u32, accent: Rgb) {
    // A short accent tick before the label, then the label in the accent color.
    let tick_w = 4 * scale;
    fb.fill_rect(x, y + 3 * scale, tick_w, scale, accent.0, accent.1, accent.2);
    fb.draw_text(x + tick_w + 3 * scale, y, text, accent.0, accent.1, accent.2, scale);
}

fn draw_progress(fb: &mut Framebuffer, t: Rect, pos_ms: u64, dur_ms: u64, accent: Rgb) {
    let y = t.y + t.h - PROGRESS_H;
    let frac = (pos_ms as f64 / dur_ms as f64).clamp(0.0, 1.0);
    let fill_w = (frac * t.w as f64).round() as u32;
    let track_c = mix(scale_rgb(accent, 0.25), (20, 20, 26), 0.5);
    fb.fill_rect(t.x, y + 1, t.w, 6, track_c.0, track_c.1, track_c.2);
    if fill_w > 0 {
        fb.fill_rect(t.x, y + 1, fill_w, 6, accent.0, accent.1, accent.2);
        let hi = mix(accent, (255, 255, 255), 0.35);
        fb.fill_rect(t.x, y + 1, fill_w, 2, hi.0, hi.1, hi.2);
    }
    let r = 8i64;
    let cx = (t.x as i64 + fill_w as i64).clamp(t.x as i64 + r, (t.x + t.w) as i64 - r);
    fill_circle(fb, cx, y as i64 + 4, r + 1, (0, 0, 0));
    fill_circle(fb, cx, y as i64 + 4, r, (0xF5, 0xF5, 0xF5));

    let ty = y + 8 + 10;
    let (left, right) = (fmt_time(pos_ms), fmt_time(dur_ms));
    let dim = (0xB4, 0xB4, 0xBE);
    fb.draw_text(t.x, ty, &left, dim.0, dim.1, dim.2, 2);
    let rw = Framebuffer::text_width(&right, 2);
    fb.draw_text((t.x + t.w).saturating_sub(rw), ty, &right, dim.0, dim.1, dim.2, 2);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Timeline;
    use std::time::Instant;

    fn solid_cover(rgb: (u8, u8, u8), size: u32) -> Arc<CoverArt> {
        let mut px = Vec::new();
        for _ in 0..size * size {
            px.extend([rgb.0, rgb.1, rgb.2]);
        }
        Arc::new(CoverArt { width: size, height: size, rgb: px })
    }

    fn track(title: &str, artist: &str, cover: Option<Arc<CoverArt>>, tl: Option<Timeline>) -> TrackInfo {
        TrackInfo { title: title.into(), artist: artist.into(), album: "Some Album".into(), cover, timeline: tl }
    }

    fn timeline(pos: u64, dur: u64) -> Timeline {
        Timeline { position_ms: pos, duration_ms: dur, playing: false, sampled_at: Instant::now() }
    }

    fn bars() -> Vec<f32> {
        (0..48).map(|i| 0.2 + 0.6 * ((i as f32 * 0.5).sin() * 0.5 + 0.5)).collect()
    }

    fn opts() -> UiOptions {
        UiOptions { music_screen: true, ..UiOptions::default() }
    }

    #[test]
    fn geometry_fits_inside_the_canvas_without_overlaps() {
        for (w, h) in [(1920u32, 462u32), (462, 1920), (800, 480), (480, 800), (1000, 1000), (100, 60), (40, 40)] {
            let g = geometry(w, h);
            for (name, r) in [("cover", g.cover), ("text", g.text), ("spectrum", g.spectrum)] {
                assert!(r.x + r.w <= w && r.y + r.h <= h, "{name} {r:?} outside {w}x{h}");
            }
            // The three blocks never overlap each other.
            let overlap = |a: Rect, b: Rect| a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
            if g.text.w > 0 && g.text.h > 0 && g.spectrum.w > 0 && g.spectrum.h > 0 {
                assert!(!overlap(g.text, g.spectrum), "{w}x{h} text/spectrum overlap");
            }
            if g.text.w > 0 && g.text.h > 0 {
                assert!(!overlap(g.cover, g.text), "{w}x{h} cover/text overlap");
            }
            if g.spectrum.w > 0 && g.spectrum.h > 0 {
                assert!(!overlap(g.cover, g.spectrum), "{w}x{h} cover/spectrum overlap");
            }
        }
        let g = geometry(1920, 462);
        assert_eq!(g.cover, Rect { x: 36, y: 36, w: 390, h: 390 }); // square, full height
        assert!(g.text.x > g.cover.x + g.cover.w && g.spectrum.x > g.text.x + g.text.w);
        let p = geometry(462, 1920);
        assert!(p.text.y > p.cover.y + p.cover.h && p.spectrum.y > p.text.y + p.text.h); // stacked
    }

    #[test]
    fn accent_follows_the_cover_color() {
        let hue = |c: Rgb| spectrum::rgb_to_hsv(c).0;
        let near = |a: f32, b: f32| ((a - b + 540.0) % 360.0 - 180.0).abs() < 8.0;
        for (rgb, expect) in [((200u8, 30u8, 30u8), 0.0f32), ((30, 200, 40), 120.0), ((30, 40, 200), 240.0)] {
            let a = accent_from_rgb(&solid_cover(rgb, 4).rgb);
            assert!(near(hue(a), expect), "{rgb:?} -> {a:?}");
            let (_, s, v) = spectrum::rgb_to_hsv(a);
            assert!(s >= 0.55 && v >= 0.9, "{a:?} not vivid enough");
        }
        // A dull brown is lifted to something readable on a dark background.
        let (_, s, v) = spectrum::rgb_to_hsv(accent_from_rgb(&solid_cover((90, 60, 40), 4).rgb));
        assert!(s >= 0.55 && v >= 0.9);
        // Black & white stays (nearly) neutral, never a random tint.
        let g = accent_from_rgb(&solid_cover((120, 120, 120), 4).rgb);
        assert!(spectrum::rgb_to_hsv(g).1 < 0.12 && spectrum::rgb_to_hsv(g).2 >= 0.85);
        assert_eq!(accent_from_rgb(&[]), DEFAULT_ACCENT);
    }

    #[test]
    fn saturated_pixels_beat_a_big_grey_border() {
        // 90% grey with a small red patch: the accent must still be red.
        let mut px = Vec::new();
        for i in 0..100 {
            px.extend(if i < 10 { [220u8, 20, 20] } else { [128u8, 128, 128] });
        }
        let a = accent_from_rgb(&px);
        let (h, s, _) = spectrum::rgb_to_hsv(a);
        assert!(s > 0.5 && (h < 15.0 || h > 345.0), "{a:?}");
    }

    #[test]
    fn corner_coverage_rounds_only_the_corners() {
        assert_eq!(corner_coverage(100, 100, 200, 12.0), 1.0); // middle
        assert_eq!(corner_coverage(100, 0, 200, 12.0), 1.0); // edge middle
        assert_eq!(corner_coverage(0, 0, 200, 12.0), 0.0); // the very corner is cut off
        assert_eq!(corner_coverage(199, 199, 200, 12.0), 0.0);
        let aa = corner_coverage(1, 5, 200, 12.0);
        assert!(aa > 0.0 && aa < 1.0, "anti-aliased pixel expected, got {aa}");
    }

    #[test]
    fn fmt_time_formats() {
        assert_eq!(fmt_time(0), "0:00");
        assert_eq!(fmt_time(5_000), "0:05");
        assert_eq!(fmt_time(83_000), "1:23");
        assert_eq!(fmt_time(354_999), "5:54");
        assert_eq!(fmt_time(3_723_000), "1:02:03");
    }

    #[test]
    fn cover_is_baked_in_the_right_place_with_glow_and_blur() {
        let cover = solid_cover((200, 30, 30), 64);
        let mut fb = new_fb(1920, 462);
        let mut m = MusicScreen::new();
        m.draw(&mut fb, &track("Song", "Band", Some(cover), None), &bars(), &bars(), &opts());
        let g = geometry(1920, 462);
        // Centre of the cover = the cover color (scaled solid colour stays solid).
        let c = get(&fb, g.cover.x + g.cover.w / 2, g.cover.y + g.cover.h / 2);
        assert!(c.0 > 190 && c.1 < 45 && c.2 < 45, "{c:?}");
        // The very corner of the cover rectangle is cut by the rounding: not cover colored.
        let corner = get(&fb, g.cover.x, g.cover.y);
        assert!(corner.0 < 150, "{corner:?}");
        // The background is the darkened cover color, reddish but dark.
        let bgp = get(&fb, 1900, 5);
        assert!(bgp.0 > bgp.1 && bgp.0 < 90, "{bgp:?}");
        // Right next to the cover the glow is brighter than the far background.
        let near = get(&fb, g.cover.x + g.cover.w + 3, g.cover.y + g.cover.h / 2);
        assert!(near.0 as u32 > bgp.0 as u32 + 15, "glow {near:?} vs bg {bgp:?}");
        // Accent is red-ish.
        let a = m.accent();
        assert!(a.0 > 200 && a.1 < 120 && a.2 < 120, "{a:?}");
    }

    #[test]
    fn missing_cover_draws_a_record_in_the_default_accent() {
        let mut fb = new_fb(1920, 462);
        let mut m = MusicScreen::new();
        m.draw(&mut fb, &track("Song", "Band", None, None), &bars(), &bars(), &opts());
        assert_eq!(m.accent(), DEFAULT_ACCENT);
        let g = geometry(1920, 462);
        // The record's label (centre ring) is the accent color.
        let cx = g.cover.x + g.cover.w / 2;
        let cy = g.cover.y + g.cover.h / 2;
        assert_eq!(get(&fb, cx + g.cover.w / 14, cy), DEFAULT_ACCENT);
    }

    #[test]
    fn cache_is_reused_between_frames_and_rebuilt_on_change() {
        let c1 = solid_cover((10, 200, 10), 32);
        let c2 = solid_cover((10, 10, 200), 32);
        let mut fb = new_fb(1920, 462);
        let mut m = MusicScreen::new();
        let t1 = track("A", "B", Some(c1.clone()), None);
        m.draw(&mut fb, &t1, &bars(), &bars(), &opts());
        let first = m.cache.as_ref().unwrap().backdrop.as_ref().unwrap().as_bytes().as_ptr();
        m.draw(&mut fb, &t1, &bars(), &bars(), &opts());
        assert_eq!(first, m.cache.as_ref().unwrap().backdrop.as_ref().unwrap().as_bytes().as_ptr(), "rebuilt needlessly");
        let green = m.accent();
        m.draw(&mut fb, &track("A", "B", Some(c2), None), &bars(), &bars(), &opts());
        assert_ne!(m.accent(), green, "new cover must change the accent");
        // Toggling the blur or resizing also rebuilds.
        let mut fb2 = new_fb(462, 1920);
        m.draw(&mut fb2, &t1, &bars(), &bars(), &opts());
        assert_eq!(m.cache.as_ref().unwrap().size, (462, 1920));
        let mut o = opts();
        o.music_blur = false;
        m.draw(&mut fb2, &t1, &bars(), &bars(), &o);
        assert!(m.cache.as_ref().unwrap().backdrop.is_none());
    }

    #[test]
    fn progress_bar_reflects_the_position() {
        let mut o = opts();
        o.music_blur = false; // plain background: easy to read pixels
        let g = geometry(1920, 462);
        let bar_y = g.text.y + g.text.h - PROGRESS_H + 4;
        let filled = |pos: u64| {
            let mut fb = new_fb(1920, 462);
            fb.clear(8, 8, 16);
            let mut m = MusicScreen::new();
            m.draw(&mut fb, &track("Song", "Band", Some(solid_cover((200, 30, 30), 16)), Some(timeline(pos, 200_000))), &bars(), &bars(), &o);
            // Count accent-ish (reddish bright) pixels along the bar row.
            (g.text.x..g.text.x + g.text.w).filter(|&x| { let p = get(&fb, x, bar_y); p.0 > 150 && p.1 < 120 }).count()
        };
        let (a, b, c) = (filled(0), filled(100_000), filled(200_000));
        assert!(a < 20, "start: {a}"); // just the knob area at most
        assert!(b > a + 200 && c > b + 100, "{a} {b} {c}");
        // The bar disappears when the option is off or there is no timeline.
        o.music_progress = false;
        let mut fb = new_fb(1920, 462);
        fb.clear(8, 8, 16);
        MusicScreen::new().draw(&mut fb, &track("S", "B", None, Some(timeline(100_000, 200_000))), &bars(), &bars(), &o);
        assert_eq!(get(&fb, g.text.x + 10, bar_y), (8, 8, 16));
    }

    #[test]
    fn without_blur_the_existing_background_is_left_alone() {
        let mut o = opts();
        o.music_blur = false;
        let mut fb = new_fb(1920, 462);
        fb.clear(40, 50, 60);
        MusicScreen::new().draw(&mut fb, &track("Song", "Band", Some(solid_cover((200, 30, 30), 16)), None), &bars(), &bars(), &o);
        assert_eq!(get(&fb, 1900, 5), (40, 50, 60));
        let g = geometry(1920, 462);
        assert_eq!(get(&fb, g.cover.x, g.cover.y), (40, 50, 60)); // rounded corner stays transparent
        let c = get(&fb, g.cover.x + g.cover.w / 2, g.cover.y + g.cover.h / 2);
        assert!(c.0 > 190, "{c:?}");
    }

    #[test]
    fn text_goes_in_the_text_block_and_never_over_the_cover_or_spectrum() {
        let mut o = opts();
        o.music_blur = false;
        let mut fb = new_fb(1920, 462);
        fb.clear(0, 0, 0);
        // A silent spectrum draws (almost) nothing; a very long title must scroll, not spill.
        let long = "An Extremely Long Song Title That Definitely Does Not Fit In The Text Column At All";
        let t = track(long, "Some Artist With A Long Name Too, Another Artist, And One More", None, Some(timeline(5_000, 300_000)));
        MusicScreen::new().draw(&mut fb, &t, &vec![0.0; 48], &vec![0.0; 48], &o);
        let g = geometry(1920, 462);
        // Between the text block and the spectrum / cover columns, pure black gutters stay empty.
        let gutter = |x0: u32, x1: u32| (x0..x1).all(|x| (0..462).all(|y| get(&fb, x, y) == (0, 0, 0)));
        assert!(gutter(g.text.x + g.text.w, g.spectrum.x), "text spilled into the gutter before the spectrum");
        assert!(gutter(g.cover.x + g.cover.w + 1, g.text.x), "text spilled into the gutter after the cover");
    }

    #[test]
    fn every_layout_and_input_combination_draws_without_panicking() {
        let covers = [None, Some(solid_cover((90, 120, 200), 8)), Some(solid_cover((0, 0, 0), 2))];
        let timelines = [None, Some(timeline(0, 1)), Some(timeline(5, 10)), Some(timeline(999_999_999, 1_000))];
        let texts = [("", ""), ("Song", ""), ("Song", "Artist"), ("日本語のタイトル", "Ünïcödé Ärtist"), ("X", "Y")];
        for (w, h) in [(1920u32, 462u32), (462, 1920), (200, 100), (60, 60)] {
            for blur in [true, false] {
                let mut o = opts();
                o.music_blur = blur;
                let mut m = MusicScreen::new();
                let mut fb = new_fb(w, h);
                for cover in &covers {
                    for tl in &timelines {
                        for (title, artist) in texts {
                            m.draw(&mut fb, &track(title, artist, cover.clone(), *tl), &bars(), &bars(), &o);
                        }
                    }
                }
                // No bars at all, and an empty peaks slice.
                m.draw(&mut fb, &track("S", "A", None, None), &[], &[], &o);
            }
        }
    }

    #[test]
    fn user_palette_and_style_override_the_cover_tint() {
        let mut o = opts();
        o.music_blur = false;
        o.spectrum_styles = vec![spectrum::Style::Led];
        o.spectrum_palettes = vec![spectrum::Palette::Matrix];
        let mut fb = new_fb(1920, 462);
        fb.clear(0, 0, 0);
        MusicScreen::new().draw(&mut fb, &track("S", "A", Some(solid_cover((200, 30, 30), 16)), None), &vec![1.0; 48], &vec![1.0; 48], &o);
        let sp = geometry(1920, 462).spectrum;
        // Matrix palette = greens: look at a pixel in the spectrum area.
        let mut greenish = 0;
        let mut reddish = 0;
        for y in (sp.y..sp.y + sp.h).step_by(3) {
            for x in (sp.x..sp.x + sp.w).step_by(3) {
                let p = get(&fb, x, y);
                if p.1 as i32 > p.0 as i32 + 30 { greenish += 1; }
                if p.0 as i32 > p.1 as i32 + 30 { reddish += 1; }
            }
        }
        assert!(greenish > 200 && reddish == 0, "green {greenish} red {reddish}");
    }
}
