//! The "music screen": while a track plays, the whole panel becomes a
//! now-playing card — album cover on the left (with a soft glow in the
//! cover's own color), artist / title / album next to it, a progress bar with
//! times, and the spectrum on the side, tinted with the cover's accent color.
//! The background is configurable (`music_bg`): a gradient of the cover's
//! dominant color (default), the blurred cover, a flat color, or nothing.
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
// Background options
// ---------------------------------------------------------------------------

/// What is painted behind the music screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgMode {
    /// The cover's dominant color fading to near-black (Spotify-like). Default.
    Gradient,
    /// The cover itself, blurred (see `Bg::blur` / `Bg::fit`).
    Blur,
    /// One flat color: the cover's dominant color.
    Color,
    /// One flat color chosen by the user (`Bg::color`).
    Solid,
    /// Nothing: whatever is already on the canvas (solid color / `background`).
    None,
}

impl BgMode {
    pub const NAMES: &'static str = "gradient | blur | color | solid | none";

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "gradient" | "gradiente" => BgMode::Gradient,
            "blur" | "blurred" | "sfocato" => BgMode::Blur,
            "color" | "colour" | "dominant" | "colore" => BgMode::Color,
            "solid" | "fixed" | "fisso" => BgMode::Solid,
            "none" | "off" | "no" | "false" | "nessuno" => BgMode::None,
            _ => return None,
        })
    }
}

/// How the blurred cover is fitted to the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgFit {
    /// A centred slice of the cover with the panel's proportions (not distorted).
    Center,
    /// The whole cover squeezed to the panel's size (the look of version 1.0.31).
    Stretch,
}

impl BgFit {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "center" | "centre" | "centered" | "centro" => BgFit::Center,
            "stretch" | "fill" | "adatta" => BgFit::Stretch,
            _ => return None,
        })
    }
}

/// All the background settings; part of the cache key, so any change rebuilds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bg {
    pub mode: BgMode,
    /// 0-100: how bright the background is (100 = the cover color at full strength).
    pub brightness: u32,
    /// 0-100: blur amount for `BgMode::Blur` (0 = barely blurred, 100 = a soft colour wash).
    pub blur: u32,
    pub fit: BgFit,
    /// The color of `BgMode::Solid`.
    pub color: Rgb,
}

impl Default for Bg {
    fn default() -> Self {
        Bg { mode: BgMode::Gradient, brightness: 40, blur: 70, fit: BgFit::Center, color: (0x12, 0x12, 0x18) }
    }
}

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
// Cached backdrop (cover + background + glow)
// ---------------------------------------------------------------------------

struct Cache {
    cover: Option<Arc<CoverArt>>,
    size: (u32, u32),
    bg: Bg,
    accent: Rgb,
    /// Fully composed background (gradient/blur/color, glow, the cover itself).
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

/// Side (in pixels) of the small image the blur is made from: the lower, the blurrier.
fn blur_side(blur: u32) -> u32 {
    let k = 1.0 - blur.min(100) as f32 / 100.0;
    (8.0 + k * k * 248.0).round() as u32
}

/// The cover shrunk to a small image (shorter side = `side`) with the panel's
/// proportions — a centred slice of it, or the whole cover squeezed — ready to
/// be stretched over the panel with a smooth filter, which is the blur.
fn blur_source(cover: &RgbImage, w: u32, h: u32, blur: u32, fit: BgFit) -> RgbImage {
    let side = blur_side(blur);
    let (cw, ch) = cover.dimensions();
    let small = match fit {
        BgFit::Stretch => imageops::resize(cover, side, side, FilterType::Triangle),
        BgFit::Center => {
            // Largest slice of the cover with the panel's aspect ratio, centred.
            let (sw, sh) = if (w as u64) * (ch as u64) >= (h as u64) * (cw as u64) {
                (cw, ((cw as u64 * h as u64) / w.max(1) as u64).max(1) as u32)
            } else {
                (((ch as u64 * w as u64) / h.max(1) as u64).max(1) as u32, ch)
            };
            let (sw, sh) = (sw.min(cw).max(1), sh.min(ch).max(1));
            let slice = imageops::crop_imm(cover, (cw - sw) / 2, (ch - sh) / 2, sw, sh).to_image();
            let (tw, th) = if w >= h {
                ((side as u64 * w as u64 / h.max(1) as u64).clamp(side as u64, 1024) as u32, side)
            } else {
                (side, (side as u64 * h as u64 / w.max(1) as u64).clamp(side as u64, 1024) as u32)
            };
            imageops::resize(&slice, tw, th, FilterType::Triangle)
        }
    };
    // A Gaussian pass on the small image rounds off hard edges (cheap there):
    // the stronger the blur setting, the wider it is relative to the image.
    let sigma = 0.8 + blur.min(100) as f32 / 100.0 * side as f32 * 0.3;
    imageops::blur(&small, sigma)
}

/// Paint the background of the whole panel for `bg.mode` (never `None`).
fn paint_background(bd: &mut Framebuffer, bg: &Bg, accent: Rgb, cover: Option<&RgbImage>) {
    let (w, h) = (bd.width(), bd.height());
    let k = bg.brightness.min(100) as f32 / 100.0;
    let landscape = w >= h;
    // A cover-less blur falls back to the gradient of the default accent.
    let mode = if bg.mode == BgMode::Blur && cover.is_none() { BgMode::Gradient } else { bg.mode };
    let blurred = match (mode, cover) {
        (BgMode::Blur, Some(c)) => {
            let small = blur_source(c, w, h, bg.blur, bg.fit);
            Some(imageops::resize(&small, w.max(1), h.max(1), FilterType::CatmullRom))
        }
        _ => None,
    };
    let base = scale_rgb(accent, k);
    let dark = (6, 6, 12);
    for y in 0..h {
        let ny = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let nx = (x as f32 + 0.5) / w as f32;
            let (vx, vy) = (nx * 2.0 - 1.0, ny * 2.0 - 1.0);
            let vignette = 1.0 - 0.30 * (vx * vx + vy * vy) / 2.0;
            let c = match mode {
                BgMode::Solid => bg.color,
                BgMode::Color => scale_rgb(base, vignette),
                BgMode::Blur => {
                    let p = blurred.as_ref().map(|b| b.get_pixel(x, y).0).unwrap_or([0, 0, 0]);
                    scale_rgb((p[0], p[1], p[2]), k * vignette)
                }
                // Brightest on the cover side, fading to near-black away from it.
                _ => {
                    let t = if landscape { 0.75 * nx + 0.25 * ny } else { ny };
                    let t = t * t * (3.0 - 2.0 * t); // smoothstep
                    scale_rgb(mix(base, dark, t * 0.9), vignette)
                }
            };
            put(bd, x, y, c);
        }
    }
}

fn build_cache(w: u32, h: u32, cover: &Option<Arc<CoverArt>>, bg: Bg) -> Cache {
    let g = geometry(w, h);
    let cs = g.cover.w.min(g.cover.h).max(1);

    let source: Option<RgbImage> = cover.as_ref().and_then(|c| RgbImage::from_raw(c.width, c.height, c.rgb.clone()));
    // Tiny version of the cover: source of the accent color.
    let tiny: Option<RgbImage> = source.as_ref().map(|img| imageops::resize(img, 24, 24, FilterType::Triangle));
    let accent = tiny.as_ref().map(|t| accent_from_rgb(t.as_raw())).unwrap_or(DEFAULT_ACCENT);

    // The square cover image at its final size (or the record placeholder).
    let cover_img: RgbImage = match &source {
        Some(img) => imageops::resize(img, cs, cs, FilterType::Lanczos3),
        None => placeholder(cs, accent),
    };
    let radius = cs as f32 / 16.0;

    if bg.mode == BgMode::None {
        let mut k = new_fb(cs, cs);
        for y in 0..cs {
            for x in 0..cs {
                let p = cover_img.get_pixel(x, y).0;
                let c = if corner_coverage(x, y, cs, radius) < 0.5 { KEY } else { (p[0], p[1], p[2]) };
                put(&mut k, x, y, c);
            }
        }
        return Cache { cover: cover.clone(), size: (w, h), bg, accent, backdrop: None, keyed_cover: Some(k) };
    }

    let mut bd = new_fb(w, h);
    paint_background(&mut bd, &bg, accent, source.as_ref());

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
    Cache { cover: cover.clone(), size: (w, h), bg, accent, backdrop: Some(bd), keyed_cover: None }
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

    fn ensure(&mut self, w: u32, h: u32, cover: &Option<Arc<CoverArt>>, bg: Bg) {
        let same_cover = |a: &Option<Arc<CoverArt>>, b: &Option<Arc<CoverArt>>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        let fresh = match &self.cache {
            Some(c) => c.size != (w, h) || c.bg != bg || !same_cover(&c.cover, cover),
            None => true,
        };
        if fresh {
            self.cache = Some(build_cache(w, h, cover, bg));
        }
    }

    /// Draw the whole screen onto `fb` (which is fully overwritten when the
    /// background is painted).
    pub fn draw(&mut self, fb: &mut Framebuffer, track: &TrackInfo, heights: &[f32], peaks: &[f32], o: &UiOptions) {
        let (w, h) = (fb.width(), fb.height());
        self.ensure(w, h, &track.cover, o.music_bg);
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
        assert!(corner.0 < 185, "{corner:?}");
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
        o.music_bg.mode = BgMode::None;
        m.draw(&mut fb2, &t1, &bars(), &bars(), &o);
        assert!(m.cache.as_ref().unwrap().backdrop.is_none());
    }

    #[test]
    fn progress_bar_reflects_the_position() {
        let mut o = opts();
        o.music_bg.mode = BgMode::None; // plain background: easy to read pixels
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
        o.music_bg.mode = BgMode::None;
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
        o.music_bg.mode = BgMode::None;
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
            for mode in [BgMode::Gradient, BgMode::Blur, BgMode::Color, BgMode::Solid, BgMode::None] {
              for fit in [BgFit::Center, BgFit::Stretch] {
                let mut o = opts();
                o.music_bg = Bg { mode, fit, ..Bg::default() };
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
    }

    #[test]
    fn user_palette_and_style_override_the_cover_tint() {
        let mut o = opts();
        o.music_bg.mode = BgMode::None;
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

    fn bg_with(mode: BgMode) -> UiOptions {
        let mut o = opts();
        o.music_bg = Bg { mode, ..Bg::default() };
        o
    }

    fn draw_bg(o: &UiOptions, cover: Option<Arc<CoverArt>>) -> Framebuffer {
        let mut fb = new_fb(1920, 462);
        MusicScreen::new().draw(&mut fb, &track("S", "A", cover, None), &vec![0.0; 48], &vec![0.0; 48], o);
        fb
    }

    fn luma(c: Rgb) -> u32 {
        c.0 as u32 + c.1 as u32 + c.2 as u32
    }

    #[test]
    fn gradient_takes_the_cover_hue_and_fades_away_from_the_cover() {
        let fb = draw_bg(&bg_with(BgMode::Gradient), Some(solid_cover((30, 60, 200), 32)));
        let g = geometry(1920, 462);
        let left = get(&fb, g.cover.x / 2, 20); // just left of the cover: brightest part
        let right = get(&fb, 1910, 450); // far corner
        assert!(left.2 > left.0 && left.2 > left.1, "bluish expected: {left:?}");
        assert!(luma(left) > luma(right) * 2, "must fade out: {left:?} -> {right:?}");
        assert!(right.0 < 40 && right.1 < 40 && right.2 < 60, "far side is near-black: {right:?}");
    }

    #[test]
    fn brightness_scales_the_background() {
        let dim = |b: u32| {
            let mut o = bg_with(BgMode::Color);
            o.music_bg.brightness = b;
            luma(get(&draw_bg(&o, Some(solid_cover((200, 30, 30), 16))), 1000, 230))
        };
        let (a, b, c) = (dim(0), dim(40), dim(100));
        assert_eq!(a, 0);
        assert!(b > 60 && c > b * 2, "{a} {b} {c}");
    }

    #[test]
    fn color_and_solid_are_flat_and_solid_ignores_the_cover() {
        let o = bg_with(BgMode::Solid);
        let fb = draw_bg(&o, Some(solid_cover((200, 30, 30), 16)));
        assert_eq!(get(&fb, 1900, 5), o.music_bg.color);
        assert_eq!(get(&fb, 1000, 230), o.music_bg.color); // no vignette either
        let fb2 = draw_bg(&o, Some(solid_cover((30, 200, 30), 16)));
        assert_eq!(get(&fb2, 1900, 5), o.music_bg.color);
        let mut o = bg_with(BgMode::Color);
        o.music_bg.brightness = 50;
        let fb = draw_bg(&o, Some(solid_cover((30, 30, 200), 16)));
        let p = get(&fb, 1000, 230);
        assert!(p.2 > p.0 + 40 && p.2 > p.1 + 40, "{p:?}");
    }

    #[test]
    fn blur_centered_keeps_proportions_and_stretch_does_not() {
        // A cover that is dark red on the left half of its middle band and blue
        // elsewhere: a centred slice shows mostly the band, a squeeze shows it all.
        let n = 64u32;
        let mut px = Vec::new();
        for y in 0..n {
            for _x in 0..n {
                px.extend(if (24..40).contains(&y) { [220u8, 30, 30] } else { [30u8, 30, 220] });
            }
        }
        let cover = Arc::new(CoverArt { width: n, height: n, rgb: px });
        let mut o = bg_with(BgMode::Blur);
        o.music_bg.blur = 0;
        o.music_bg.brightness = 100;
        o.music_bg.fit = BgFit::Center;
        let c = draw_bg(&o, Some(cover.clone()));
        o.music_bg.fit = BgFit::Stretch;
        let s = draw_bg(&o, Some(cover));
        // Top-right corner area (outside every block): the centred slice is all red band,
        // the stretched whole cover still has its blue top there.
        let (pc, ps) = (get(&c, 1800, 20), get(&s, 1800, 20));
        assert!(pc.0 > pc.2, "centered should be the red band: {pc:?}");
        assert!(ps.2 > ps.0, "stretched should show the blue top: {ps:?}");
    }

    #[test]
    fn more_blur_means_less_detail() {
        // Checkerboard cover: a sharp background keeps contrast, a blurry one is grey.
        let n = 64u32;
        let mut px = Vec::new();
        for y in 0..n {
            for x in 0..n {
                px.extend(if (x / 4 + y / 4) % 2 == 0 { [250u8, 250, 250] } else { [5u8, 5, 5] });
            }
        }
        let cover = Arc::new(CoverArt { width: n, height: n, rgb: px });
        let spread = |blur: u32| {
            let mut o = bg_with(BgMode::Blur);
            o.music_bg.blur = blur;
            o.music_bg.brightness = 100;
            let fb = draw_bg(&o, Some(cover.clone()));
            let vals: Vec<u32> = (0..80).map(|i| luma(get(&fb, 1500 + i * 4, 230))).collect();
            vals.iter().max().unwrap() - vals.iter().min().unwrap()
        };
        assert!(spread(0) > spread(100) + 100, "{} vs {}", spread(0), spread(100));
        assert!(blur_side(0) > blur_side(50) && blur_side(50) > blur_side(100) && blur_side(100) >= 8);
    }

    #[test]
    fn none_leaves_the_canvas_and_every_bg_setting_rebuilds_the_cache() {
        let fb = {
            let mut fb = new_fb(1920, 462);
            fb.clear(40, 50, 60);
            MusicScreen::new().draw(&mut fb, &track("S", "A", Some(solid_cover((200, 30, 30), 16)), None), &bars(), &bars(), &bg_with(BgMode::None));
            fb
        };
        assert_eq!(get(&fb, 1900, 5), (40, 50, 60));
        let cover = Some(solid_cover((10, 200, 10), 16));
        let t = track("A", "B", cover, None);
        let mut m = MusicScreen::new();
        let mut fb = new_fb(1920, 462);
        let mut o = opts();
        m.draw(&mut fb, &t, &bars(), &bars(), &o);
        let first = m.cache.as_ref().unwrap().bg;
        for tweak in [
            |b: &mut Bg| b.mode = BgMode::Blur,
            |b: &mut Bg| b.brightness = 90,
            |b: &mut Bg| b.blur = 5,
            |b: &mut Bg| b.fit = BgFit::Stretch,
            |b: &mut Bg| b.color = (1, 2, 3),
        ] {
            o.music_bg = Bg::default();
            m.draw(&mut fb, &t, &bars(), &bars(), &o);
            tweak(&mut o.music_bg);
            m.draw(&mut fb, &t, &bars(), &bars(), &o);
            assert_ne!(m.cache.as_ref().unwrap().bg, Bg::default());
            assert_eq!(m.cache.as_ref().unwrap().bg, o.music_bg);
        }
        assert_eq!(first, Bg::default());
    }

    #[test]
    fn a_blurred_background_without_a_cover_falls_back_to_the_gradient() {
        let fb = draw_bg(&bg_with(BgMode::Blur), None);
        let g = geometry(1920, 462);
        let p = get(&fb, g.cover.x / 2, 20);
        assert!(p.1 > p.0 && p.1 > p.2, "default accent is green: {p:?}");
    }
}
