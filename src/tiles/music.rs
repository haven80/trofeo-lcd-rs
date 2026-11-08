//! The screens shown while a track plays (`music_layout = ...`). Eight designs share the same
//! readings: cover, title / artist / album, the position in the track and the spectrum.
//!
//! * `vinyl`     - a record that spins (the cover is its label) and a tonearm that follows the track;
//! * `poster`    - the cover big, the title in huge letters, a mirrored spectrum;
//! * `minimal`   - only the title, centred, a faint wave behind it and a thin progress line;
//! * `radial`    - the spectrum as a ring of rays around the cover, the progress as a circle;
//! * `equalizer` - a hi-fi equaliser made of LED blocks, with the peaks held;
//! * `wave`      - soft layered waves, the cover and the title above them;
//! * `cassette`  - a tape: the reels turn and swap their tape as the track goes on;
//! * `vu`        - two analogue VU meters and a small display, like an amplifier.
//!
//! `classic` is the original card (cover, text, spectrum; drawn by `music_screen.rs`, not here).
//!
//! The background is the usual `music_bg` (gradient of the cover, blurred cover, colour,
//! effect...), the progress bar is `music_progress` and the small system line at the bottom is
//! `music_stats`, so they work in every design. What is new is chosen with `music_layout`,
//! `music_colors`, `music_font`, `music_text_color`, `music_clock`, `music_album` and
//! `music_glow`. The spectrum is drawn by each design in its own way (`spectrum_style` is for
//! the classic card and the main screen).

use super::cover::Tex;
use super::game::{bar, blit_cover, fit_clip, glow_txt, hair, is_tall, txt, txt_c, txt_r, R};
use super::*;
use crate::media::{CoverArt, TrackInfo};
use crate::music_screen::{self, Stats};
use std::sync::Arc;

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// The card of earlier versions.
    Classic,
    Vinyl,
    Poster,
    Minimal,
    Radial,
    Equalizer,
    Wave,
    Cassette,
    Vu,
}

impl Layout {
    pub const NAMES: &'static str = "classic | vinyl | poster | minimal | radial | equalizer | wave | cassette | vu";
    #[allow(dead_code)]
    pub const ALL: [Layout; 9] = [
        Layout::Classic, Layout::Vinyl, Layout::Poster, Layout::Minimal, Layout::Radial,
        Layout::Equalizer, Layout::Wave, Layout::Cassette, Layout::Vu,
    ];

    pub fn parse(s: &str) -> Option<Layout> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "classic" | "card" | "old" | "classico" => Layout::Classic,
            "vinyl" | "record" | "vinile" | "disco" => Layout::Vinyl,
            "poster" | "big" => Layout::Poster,
            "minimal" | "minimale" | "zen" => Layout::Minimal,
            "radial" | "ring" | "circle" | "radiale" => Layout::Radial,
            "equalizer" | "equaliser" | "eq" | "led" => Layout::Equalizer,
            "wave" | "waves" | "onde" => Layout::Wave,
            "cassette" | "tape" | "cassetta" => Layout::Cassette,
            "vu" | "meters" | "amp" | "amplifier" => Layout::Vu,
            _ => return None,
        })
    }

    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Layout::Classic => "classic",
            Layout::Vinyl => "vinyl",
            Layout::Poster => "poster",
            Layout::Minimal => "minimal",
            Layout::Radial => "radial",
            Layout::Equalizer => "equalizer",
            Layout::Wave => "wave",
            Layout::Cassette => "cassette",
            Layout::Vu => "vu",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub layout: Layout,
    /// `None` = the colours of the design (taken from the cover, or its own).
    pub colors: Option<Vec<Rgb>>,
    pub font: Option<FontStyle>,
    pub text: Option<Rgb>,
    /// The clock in a corner.
    pub clock: bool,
    /// The album name under the artist.
    pub album: bool,
    /// 0-100: strength of the glow around the cover and the big text.
    pub glow: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { layout: Layout::Classic, colors: None, font: None, text: None, clock: true, album: true, glow: 50 }
    }
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get("music_layout") {
        s.layout = Layout::parse(v).ok_or_else(|| anyhow::anyhow!("music_layout: '{v}' not valid ({})", Layout::NAMES))?;
    }
    if let Some(v) = file.get("music_colors") {
        let list = parse_rgb_list("music_colors", v)?;
        if list.is_empty() {
            anyhow::bail!("music_colors: needs at least one color, e.g. #FF2BD6, #00E5FF");
        }
        s.colors = Some(list);
    }
    if let Some(v) = file.get("music_font") {
        s.font = match v.trim().to_ascii_lowercase().as_str() {
            "auto" | "design" | "" => None,
            "smooth" | "liscio" | "vector" | "ttf" => Some(FontStyle::Smooth),
            "pixel" | "normal" | "bitmap" => Some(FontStyle::Pixel),
            _ => anyhow::bail!("music_font: '{v}' not valid (auto | smooth | pixel)"),
        };
    }
    if let Some(v) = file.get("music_text_color") {
        s.text = Some(parse_rgb("music_text_color", v)?);
    }
    if let Some(v) = file.get_bool("music_clock").map_err(|e| anyhow::anyhow!(e))? {
        s.clock = v;
    }
    if let Some(v) = file.get_bool("music_album").map_err(|e| anyhow::anyhow!(e))? {
        s.album = v;
    }
    if let Some(v) = file.get_u32("music_glow").map_err(|e| anyhow::anyhow!(e))? {
        if v > 100 {
            anyhow::bail!("music_glow: must be between 0 and 100 (got {v})");
        }
        s.glow = v;
    }
    Ok(s)
}

impl Settings {
    /// The accent colours in use (never empty): the user's, or those of the design.
    fn accents(&self, accent: Rgb) -> Vec<Rgb> {
        match &self.colors {
            Some(c) if !c.is_empty() => c.clone(),
            _ => palette(self.layout, accent),
        }
    }
}

/// The colours of a design. Most take them from the cover's accent; the LED equaliser, the
/// amplifier and the cassette have colours of their own.
fn palette(layout: Layout, accent: Rgb) -> Vec<Rgb> {
    match layout {
        Layout::Equalizer => vec![(0x3B, 0xE0, 0x6A), (0xFF, 0xD2, 0x3F), (0xFF, 0x4D, 0x4D), (0xF5, 0xF5, 0xF5)],
        Layout::Vu => vec![(0xFF, 0xB4, 0x4D), (0xFF, 0x7A, 0x3D), (0xFF, 0xE0, 0x9A), (0xFF, 0xF0, 0xD0)],
        Layout::Cassette => vec![(0xE8, 0x5D, 0x4A), (0xF2, 0xC1, 0x4E), (0x4A, 0xA3, 0xE8), (0xF1, 0xE9, 0xD2)],
        _ => {
            let (h, s, v) = crate::spectrum::rgb_to_hsv(accent);
            vec![
                accent,
                crate::spectrum::hsv(h + 38.0, s.clamp(0.5, 0.9), v.max(0.9)),
                crate::spectrum::hsv(h - 38.0, s.clamp(0.4, 0.9), v.max(0.9)),
                lerp_rgb(accent, (255, 255, 255), 0.6),
            ]
        }
    }
}

// -------------------------------------------------------------------- scene

struct Scene<'a> {
    s: &'a Settings,
    ink: Rgb,
    col: Vec<Rgb>,
    title: String,
    artist: String,
    album: String,
    cover: &'a Tex,
    /// How far into the track (0-1), when the player says.
    frac: Option<f32>,
    pos: String,
    dur: String,
    bars: &'a [f32],
    peaks: &'a [f32],
    /// Left and right level of the meters (0-1, already smoothed).
    vu: [f32; 2],
    /// Angle of the record / the reels, in radians.
    spin: f32,
    clock: String,
    footer: Vec<(String, String)>,
}

impl Scene<'_> {
    fn c(&self, i: usize) -> Rgb {
        self.col[i % self.col.len().max(1)]
    }

    /// Colour of something at `level` 0-1 (quiet to loud).
    fn level_color(&self, level: f32) -> Rgb {
        lerp_rgb(self.c(0), self.c(1), level.clamp(0.0, 1.0))
    }

    fn vals(&self, n: usize) -> Vec<f32> {
        resample(self.bars, n)
    }

    fn tops(&self, n: usize) -> Vec<f32> {
        resample(self.peaks, n)
    }
}

/// `n` values from the `src` ones: interpolated when there are fewer, the highest of each group
/// when there are more.
fn resample(src: &[f32], n: usize) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    if src.is_empty() {
        return vec![0.0; n];
    }
    let last = src.len() - 1;
    (0..n)
        .map(|i| {
            let v = if n >= src.len() {
                let pos = if n > 1 { i as f32 * last as f32 / (n - 1) as f32 } else { 0.0 };
                let (a, b) = (pos.floor() as usize, (pos.floor() as usize + 1).min(last));
                src[a] + (src[b] - src[a]) * (pos - a as f32)
            } else {
                let a = i * src.len() / n;
                let b = ((i + 1) * src.len() / n).clamp(a + 1, src.len());
                src[a..b].iter().copied().fold(0.0, f32::max)
            };
            v.clamp(0.0, 1.0)
        })
        .collect()
}

/// A line of text that shrinks (down to `min` of `cap`) and then gets cut with dots to fit `w`.
#[allow(clippy::too_many_arguments)]
fn line(p: &mut Painter, x: f32, base: f32, s: &str, cap: f32, min: f32, w: f32, wt: W, c: Rgb, a: u8, center: bool) -> f32 {
    if s.is_empty() || w < 8.0 {
        return cap;
    }
    let cap = p.fit(s, cap, w, wt).max(min.min(cap));
    let t = fit_clip(p, s, cap, w, wt);
    if center {
        txt_c(p, x + w / 2.0, base, &t, cap, wt, c, a);
    } else {
        txt(p, x, base, &t, cap, wt, c, a);
    }
    cap
}

/// The small system line at the very bottom; returns what is left above it.
fn footer(p: &mut Painter, sc: &Scene, r: R) -> R {
    if sc.footer.is_empty() {
        return r;
    }
    let (x, y, w, h) = r;
    let cap = (h.min(w) * 0.032).clamp(9.0, 15.0);
    let base = y + h - 1.0;
    let mut cx = x;
    for (label, value) in &sc.footer {
        let lw = if label.is_empty() { 0.0 } else { p.width(label, cap, W::Regular) + cap * 0.6 };
        let vw = p.width(value, cap, W::Medium);
        if cx + lw + vw > x + w {
            break;
        }
        if !label.is_empty() {
            txt(p, cx, base, label, cap, W::Regular, sc.ink, 120);
        }
        txt(p, cx + lw, base, value, cap, W::Medium, sc.ink, 200);
        cx += lw + vw + cap * 2.2;
    }
    (x, y, w, h - cap * 2.4)
}

/// The clock, with its right edge at `right`.
fn clock(p: &mut Painter, sc: &Scene, right: f32, base: f32, cap: f32) -> f32 {
    if !sc.s.clock {
        return 0.0;
    }
    txt_r(p, right, base, &sc.clock, cap, W::Medium, sc.ink, 170)
}

/// "NOW PLAYING" with an accent tick in front. Returns the x where it stopped.
fn label(p: &mut Painter, sc: &Scene, x: f32, base: f32, cap: f32) -> f32 {
    let c = sc.c(0);
    p.layer.fill_rrect_solid((x.round(), (base - cap).round(), 4.0, cap.round()), 1.0, c, 255);
    let t = crate::i18n::t().now_playing.to_uppercase();
    x + 4.0 + cap * 0.7 + txt(p, x + 4.0 + cap * 0.7, base, &t, cap, W::Medium, c, 255)
}

/// The progress bar with the times under it. Returns the height it took (0 when it is off).
fn progress(p: &mut Painter, sc: &Scene, x: f32, y: f32, w: f32, cap: f32, thick: f32) -> f32 {
    let Some(f) = sc.frac else { return 0.0 };
    if w < 40.0 {
        return 0.0;
    }
    let (a, b) = (sc.c(0), sc.c(1));
    bar(p, (x, y, w, thick), f, a, b, (sc.ink, 36));
    p.layer.dot(x + (w * f).max(thick / 2.0), y + thick / 2.0, thick * 1.15, (255, 255, 255), 255);
    let base = y + thick + cap * 1.7;
    txt(p, x, base, &sc.pos, cap, W::Regular, sc.ink, 170);
    txt_r(p, x + w, base, &sc.dur, cap, W::Regular, sc.ink, 170);
    thick + cap * 2.4
}

/// Height `progress` takes (so a design can leave room for it).
fn progress_h(sc: &Scene, cap: f32, thick: f32) -> f32 {
    if sc.frac.is_some() { thick + cap * 2.4 } else { 0.0 }
}

/// Title, artist and album from `y` down; returns the y under them.
#[allow(clippy::too_many_arguments)]
fn title_block(p: &mut Painter, sc: &Scene, x: f32, y: f32, w: f32, title_cap: f32, center: bool, glow: bool) -> f32 {
    let mut y = y;
    let tc = p.fit(&sc.title, title_cap, w, W::Medium).max(title_cap * 0.42).min(title_cap);
    y += tc;
    let t = fit_clip(p, &sc.title, tc, w, W::Medium);
    if glow && sc.s.glow > 0 {
        let tw = p.width(&t, tc, W::Medium);
        let tx = if center { x + (w - tw) / 2.0 } else { x };
        glow_txt(p, tx, y, &t, tc, W::Medium, sc.ink, sc.s.glow / 2);
    } else if center {
        txt_c(p, x + w / 2.0, y, &t, tc, W::Medium, sc.ink, 255);
    } else {
        txt(p, x, y, &t, tc, W::Medium, sc.ink, 255);
    }
    if !sc.artist.is_empty() {
        y += tc * 0.42;
        let ac = (title_cap * 0.5).min(tc * 0.62).max(10.0);
        y += ac;
        line(p, x, y, &sc.artist, ac, ac * 0.55, w, W::Regular, lerp_rgb(sc.c(0), (255, 255, 255), 0.45), 255, center);
        if sc.s.album && !sc.album.trim().is_empty() {
            y += ac * 0.55;
            let bc = (ac * 0.68).max(9.0);
            y += bc;
            line(p, x, y, &sc.album, bc, bc * 0.6, w, W::Regular, sc.ink, 140, center);
            return y;
        }
        return y;
    }
    y
}

/// Vertical bars growing from the bottom of `r`, with the peaks held.
fn bars_up(p: &mut Painter, sc: &Scene, r: R, n: usize, mirror: bool) {
    let (x, y, w, h) = r;
    if n == 0 || w < 8.0 || h < 6.0 {
        return;
    }
    let vals = sc.vals(n);
    let tops = sc.tops(n);
    let gap = (w / n as f32 * 0.28).max(1.0);
    let bw = (w - gap * (n as f32 - 1.0)) / n as f32;
    let base = if mirror { y + h / 2.0 } else { y + h };
    let span = if mirror { h / 2.0 } else { h };
    for i in 0..n {
        let bx = x + i as f32 * (bw + gap);
        let v = vals[i];
        let bh = (v * span).max(2.0);
        let c = sc.level_color(v);
        p.layer.fill_rrect_solid((bx, base - bh, bw, bh), (bw * 0.4).min(4.0), c, 255);
        if mirror {
            p.layer.fill_rrect_solid((bx, base + 1.0, bw, bh), (bw * 0.4).min(4.0), c, 90);
        }
        let ty = base - (tops[i] * span).max(bh + 3.0);
        if tops[i] > 0.04 {
            p.layer.fill_rrect_solid((bx, ty - 2.0, bw, 2.0), 1.0, (255, 255, 255), 170);
        }
    }
}

/// The cover picture as a disc, turned by `ang`.
fn blit_round(layer: &mut Layer, tex: &Tex, cx: f32, cy: f32, r: f32, ang: f32) {
    let (s, c) = ang.sin_cos();
    for py in (cy - r - 1.0).floor() as i32..(cy + r + 1.0).ceil() as i32 {
        for px in (cx - r - 1.0).floor() as i32..(cx + r + 1.0).ceil() as i32 {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            let cov = (r + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let u = (dx * c + dy * s) / (2.0 * r) + 0.5;
            let v = (-dx * s + dy * c) / (2.0 * r) + 0.5;
            let t = tex.sample(u * tex.w as f32, v * tex.h as f32);
            layer.over(px, py, (t[0] as u8, t[1] as u8, t[2] as u8), (cov * 255.0).round() as u8);
        }
    }
}

/// A soft light around a rounded rectangle (the cover).
fn glow_box(p: &mut Painter, r: R, radius: f32, c: Rgb, strength: u32) {
    if strength == 0 {
        return;
    }
    let g = strength as f32 / 100.0;
    let (x, y, w, h) = r;
    let step = (w.min(h) * 0.018).max(1.5);
    for k in (1..=5).rev() {
        let e = step * k as f32 * 1.4;
        let a = (34.0 * g * (1.0 - k as f32 / 6.0).powi(2)) as u8;
        p.layer.fill_rrect_solid((x - e, y - e, w + 2.0 * e, h + 2.0 * e), radius + e, c, a);
    }
}

fn pad_of(r: R) -> f32 {
    (r.2.min(r.3) * 0.05).clamp(10.0, 30.0)
}

// --------------------------------------------------------------------- vinyl

fn draw_vinyl(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let pad = pad_of(r);
    let (cx, cy, rad) = if tall {
        let rad = (w * 0.5 - pad).min(h * 0.27).max(20.0);
        (x + w / 2.0 - rad * 0.08, y + pad + rad, rad)
    } else {
        let rad = ((h - 2.0 * pad) / 2.0).min(w * 0.3).max(20.0);
        (x + pad + rad, y + h / 2.0, rad)
    };
    // the record
    p.layer.dot(cx + rad * 0.02, cy + rad * 0.03, rad + rad * 0.03, (0, 0, 0), 80);
    p.layer.dot(cx, cy, rad, (12, 12, 16), 255);
    for i in 0..6u8 {
        let rr = rad * (0.42 + 0.1 * i as f32);
        p.layer.arc(cx, cy, rr, 1.0, 1.0, &|_| (90, 90, 104), 60u8.saturating_sub(i * 5));
    }
    p.layer.gauge(cx, cy, rad, rad * 0.62, 25.0, 55.0, 1.0, &|_| (255, 255, 255), 15);
    p.layer.gauge(cx, cy, rad, rad * 0.62, 205.0, 55.0, 1.0, &|_| (255, 255, 255), 15);
    p.layer.arc(cx, cy, rad, 2.0, 1.0, &|_| (60, 60, 72), 200);
    // the label is the cover
    let lr = rad * 0.36;
    blit_round(p.layer, sc.cover, cx, cy, lr, sc.spin);
    p.layer.arc(cx, cy, lr, 1.5, 1.0, &|_| (0, 0, 0), 120);
    p.layer.dot(cx, cy, rad * 0.035, (10, 10, 14), 255);
    p.layer.dot(cx, cy, rad * 0.02, (200, 200, 210), 255);
    // the tonearm follows the track
    let pivot = ((cx + rad * 1.1).min(x + w - pad * 0.6), cy - rad * 0.8);
    let f = sc.frac.unwrap_or(0.25);
    let rn = rad * (0.93 - 0.5 * f);
    let th = 38.0f32.to_radians();
    let tip = (cx + rn * th.sin(), cy - rn * th.cos());
    let (dx, dy) = (tip.0 - pivot.0, tip.1 - pivot.1);
    let len = dx.hypot(dy).max(1.0);
    let (ux, uy) = (dx / len, dy / len);
    p.layer.line(pivot.0 + 3.0, pivot.1 + 4.0, tip.0 + 3.0, tip.1 + 4.0, rad * 0.04, (0, 0, 0), 60);
    p.layer.line(pivot.0, pivot.1, tip.0, tip.1, rad * 0.035, (205, 208, 216), 245);
    p.layer.line(tip.0 - ux * rad * 0.02, tip.1 - uy * rad * 0.02, tip.0 + ux * rad * 0.1, tip.1 + uy * rad * 0.1, rad * 0.075, (36, 36, 44), 255);
    p.layer.dot(pivot.0, pivot.1, rad * 0.1, (52, 54, 62), 255);
    p.layer.dot(pivot.0, pivot.1, rad * 0.055, (150, 152, 162), 255);

    // text
    let (tx, tw, ty) = if tall { (x + pad, w - 2.0 * pad, cy + rad + pad) } else {
        let tx = cx + rad * 1.3 + pad;
        (tx, x + w - pad - tx, y + pad)
    };
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    let lend = label(p, sc, tx, ty + small, small);
    let _ = lend;
    clock(p, sc, tx + tw, ty + small, small * 1.15);
    let bottom = y + h - pad;
    let pcap = small * 0.95;
    let ph = progress_h(sc, pcap, 7.0);
    let y1 = title_block(p, sc, tx, ty + small * 2.2, tw, (h * 0.15).clamp(18.0, 80.0), false, true);
    let zone = (tx, y1 + pad * 0.7, tw, bottom - ph - y1 - pad * 1.2);
    if zone.3 > 24.0 {
        bars_up(p, sc, zone, ((tw / 10.0) as usize).clamp(12, 64), false);
    }
    progress(p, sc, tx, bottom - ph, tw, pcap, 7.0);
}

// -------------------------------------------------------------------- poster

fn draw_poster(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let pad = pad_of(r);
    let side = if tall { (w - 2.0 * pad).min(h * 0.4) } else { (h - 2.0 * pad).min(w * 0.4) };
    let cover = if tall { (x + (w - side) / 2.0, y + pad, side, side) } else { (x + pad, y + (h - side) / 2.0, side, side) };
    let rad = side * 0.05;
    glow_box(p, cover, rad, sc.c(0), sc.s.glow);
    blit_cover(p.layer, sc.cover, cover, rad);
    let (tx, tw, ty) = if tall { (x + pad, w - 2.0 * pad, cover.1 + side + pad) } else {
        let tx = cover.0 + side + pad * 1.6;
        (tx, x + w - pad - tx, y + pad)
    };
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    label(p, sc, tx, ty + small, small);
    clock(p, sc, tx + tw, ty + small, small * 1.15);
    let bottom = y + h - pad;
    let pcap = small * 0.95;
    let ph = progress_h(sc, pcap, 5.0);
    let y1 = title_block(p, sc, tx, ty + small * 2.0, tw, (h * 0.21).clamp(20.0, 110.0), false, true);
    let zone = (tx, y1 + pad * 0.6, tw, bottom - ph - y1 - pad);
    if zone.3 > 24.0 {
        bars_up(p, sc, zone, ((tw / 9.0) as usize).clamp(16, 96), true);
    }
    progress(p, sc, tx, bottom - ph, tw, pcap, 5.0);
}

// ------------------------------------------------------------------- minimal

fn draw_minimal(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let pad = pad_of(r);
    let wave = sc.vals(120);
    p.layer.spark((x, y + h * 0.3, w, h * 0.68), &wave, 1.0, sc.c(0), 1.6, 26, 80, false);
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    clock(p, sc, x + w - pad, y + pad + small, small * 1.15);
    let tw = w - 2.0 * pad - w * 0.08;
    let tcap = if is_tall(r) { (w * 0.16).clamp(18.0, 110.0) } else { (h * 0.2).min(w * 0.09).clamp(18.0, 110.0) };
    let ac = tcap * 0.4;
    let block_h = tcap + ac * 2.1 + if sc.s.album && !sc.album.trim().is_empty() { ac * 0.9 } else { 0.0 };
    let top = y + (h - block_h) / 2.0 - h * 0.04;
    title_block(p, sc, x + (w - tw) / 2.0, top, tw, tcap, true, true);
    let pw = (w * 0.5).min(520.0);
    progress(p, sc, x + (w - pw) / 2.0, y + h - pad - small * 3.4, pw, small, 3.0);
}

// -------------------------------------------------------------------- radial

fn draw_radial(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let pad = pad_of(r);
    let (cx, cy, ro) = if tall {
        let ro = (w * 0.5 - pad).min(h * 0.27).max(24.0);
        (x + w / 2.0, y + pad + ro, ro)
    } else {
        let ro = ((h - 2.0 * pad) / 2.0).min(w * 0.3).max(24.0);
        (x + pad + ro, y + h / 2.0, ro)
    };
    // the progress circle, then the rays, then the cover in the middle
    p.layer.arc(cx, cy, ro, 4.0, 1.0, &|_| sc.ink, 30);
    if let Some(f) = sc.frac {
        let (a, b) = (sc.c(0), sc.c(1));
        p.layer.arc(cx, cy, ro, 4.0, f, &move |t| lerp_rgb(a, b, t), 255);
    }
    let n = 72usize;
    let half = resample(sc.bars, n / 2);
    let (r0, rmax) = (ro * 0.66, ro * 0.27);
    for i in 0..n {
        let k = if i < n / 2 { i } else { n - 1 - i };
        let v = half[k];
        let ang = (i as f32 + 0.5) / n as f32 * std::f32::consts::TAU;
        let (s, c) = ang.sin_cos();
        let len = (v * rmax).max(3.0);
        let col = sc.level_color(v);
        p.layer.line(cx + s * r0, cy - c * r0, cx + s * (r0 + len), cy - c * (r0 + len), (ro * 0.028).max(2.0), col, 255);
    }
    let cr = ro * 0.6;
    p.layer.dot(cx, cy, cr + 3.0, sc.c(0), (sc.s.glow as f32 * 0.5) as u8);
    blit_round(p.layer, sc.cover, cx, cy, cr, 0.0);
    p.layer.arc(cx, cy, cr, 1.5, 1.0, &|_| (255, 255, 255), 70);

    let (tx, tw, ty) = if tall { (x + pad, w - 2.0 * pad, cy + ro + pad) } else {
        let tx = cx + ro + pad * 1.6;
        (tx, x + w - pad - tx, y + pad)
    };
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    label(p, sc, tx, ty + small, small);
    clock(p, sc, tx + tw, ty + small, small * 1.15);
    let y1 = title_block(p, sc, tx, ty + small * 2.2, tw, (h * 0.17).clamp(18.0, 90.0), false, true);
    // the times as one line (the circle is the progress bar)
    if sc.frac.is_some() {
        let c = (small * 1.5).clamp(12.0, 26.0);
        let t = format!("{}  /  {}", sc.pos, sc.dur);
        txt(p, tx, (y1 + c * 2.4).min(y + h - pad), &t, c, W::Medium, sc.ink, 210);
    }
    // a wave of the spectrum under the text
    let wy = y1 + small * 5.0;
    if y + h - pad - wy > 30.0 && tw > 80.0 {
        p.layer.spark((tx, wy, tw, y + h - pad - wy), &sc.vals(100), 1.0, sc.c(0), 1.8, 40, 200, true);
    }
}

// ----------------------------------------------------------------- equalizer

fn draw_equalizer(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let pad = pad_of(r);
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    // the top row: a small cover, the title, the clock (the title goes under the cover when tall)
    let row_h = if tall { (w * 0.34).clamp(40.0, 200.0) } else { (h * 0.27).clamp(40.0, 160.0) };
    let cover = (x + pad, y + pad, row_h, row_h);
    glow_box(p, cover, row_h * 0.08, sc.c(1), sc.s.glow / 2);
    blit_cover(p.layer, sc.cover, cover, row_h * 0.08);
    clock(p, sc, x + w - pad, y + pad + small * 1.15, small * 1.15);
    let head_bottom = if tall {
        let tcap = (w * 0.1).clamp(16.0, 70.0);
        let b = title_block(p, sc, x + pad, y + pad + row_h + pad * 0.6, w - 2.0 * pad, tcap, false, false);
        b + pad * 0.4
    } else {
        let tx = x + pad + row_h + pad;
        let clock_w = if sc.s.clock { p.width(&sc.clock, small * 1.15, W::Medium) + pad } else { 0.0 };
        let tw = x + w - pad - tx - clock_w;
        let tcap = (row_h * 0.46).clamp(16.0, 70.0);
        let ty = y + pad + (row_h - tcap * 1.0 - tcap * 0.5 * 1.5) / 2.0 - small * 0.3;
        title_block(p, sc, tx, ty.max(y + pad), tw, tcap, false, false);
        y + pad + row_h
    };
    let top = head_bottom + pad * 0.9;
    let pcap = small * 0.95;
    let ph = progress_h(sc, pcap, 5.0);
    let zone = (x + pad, top, w - 2.0 * pad, y + h - pad - ph - top - pad * 0.5);
    // the LED columns
    let pcap = small * 0.95;
    let ph = progress_h(sc, pcap, 5.0);
    let zone = (x + pad, y + pad + row_h + pad * 0.9, w - 2.0 * pad, y + h - pad - ph - (y + pad + row_h + pad * 0.9) - pad * 0.5);
    if zone.3 > 20.0 && zone.2 > 40.0 {
        let cell_w = (zone.3 * 0.11).clamp(12.0, 34.0);
        let n = ((zone.2 / (cell_w * 1.25)) as usize).clamp(12, 64);
        let rows = ((zone.3 / (cell_w * 0.62)) as usize).clamp(8, 28);
        let vals = sc.vals(n);
        let tops = sc.tops(n);
        let gx = zone.2 / n as f32;
        let gy = zone.3 / rows as f32;
        let (cw, ch) = (gx * 0.78, gy * 0.74);
        for i in 0..n {
            let lit = (vals[i] * rows as f32).round() as usize;
            let peak = ((tops[i] * rows as f32).round() as usize).min(rows).max(lit);
            for j in 0..rows {
                let t = j as f32 / rows as f32;
                let color = if t < 0.62 { sc.c(0) } else if t < 0.84 { sc.c(1) } else { sc.c(2) };
                let (bx, by) = (zone.0 + i as f32 * gx + (gx - cw) / 2.0, zone.1 + zone.3 - (j as f32 + 1.0) * gy + (gy - ch) / 2.0);
                if j < lit {
                    p.layer.fill_rrect_solid((bx, by, cw, ch), 1.5, color, 255);
                } else if peak > 0 && j + 1 == peak && peak > lit {
                    p.layer.fill_rrect_solid((bx, by, cw, ch), 1.5, sc.c(3), 210);
                } else {
                    p.layer.fill_rrect_solid((bx, by, cw, ch), 1.5, sc.ink, 14);
                }
            }
        }
    }
    progress(p, sc, x + pad, y + h - pad - ph, w - 2.0 * pad, pcap, 5.0);
}

// ---------------------------------------------------------------------- wave

fn smooth(v: &[f32]) -> Vec<f32> {
    (0..v.len())
        .map(|i| {
            let (a, b) = (v[i.saturating_sub(1)], v[(i + 1).min(v.len() - 1)]);
            (a + 2.0 * v[i] + b) / 4.0
        })
        .collect()
}

fn draw_wave(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let pad = pad_of(r);
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    // the waves come first: the text is over them
    let base = smooth(&smooth(&sc.vals(110)));
    let zone = (x, y + h * 0.36, w, h * 0.64);
    let rev: Vec<f32> = base.iter().rev().map(|v| v * 0.78).collect();
    let low: Vec<f32> = base.iter().map(|v| v * 0.5).collect();
    p.layer.chart(zone, &low, 0.0, 1.0, sc.c(2), 1.4, 60, false);
    p.layer.chart(zone, &rev, 0.0, 1.0, sc.c(1), 1.8, 80, false);
    p.layer.chart(zone, &base, 0.0, 1.0, sc.c(0), 2.4, 110, true);
    // the cover and the title (under the cover when the screen is tall)
    let tall = is_tall(r);
    let side = if tall { (w * 0.4).clamp(48.0, 220.0) } else { (h * 0.34).clamp(48.0, 220.0) };
    let cover = (x + pad, y + pad, side, side);
    glow_box(p, cover, side * 0.07, sc.c(0), sc.s.glow);
    blit_cover(p.layer, sc.cover, cover, side * 0.07);
    if tall {
        clock(p, sc, x + w - pad, y + pad + small * 1.15, small * 1.15);
        let b = title_block(p, sc, x + pad, y + pad + side + pad * 0.7, w - 2.0 * pad, (w * 0.12).clamp(16.0, 80.0), false, true);
        progress(p, sc, x + pad, b + pad * 0.8, w - 2.0 * pad, small * 0.95, 4.0);
        return;
    }
    let tx = x + pad + side + pad * 1.2;
    let tw = x + w - pad - tx;
    clock(p, sc, tx + tw, y + pad + small * 1.15, small * 1.15);
    label(p, sc, tx, y + pad + small, small);
    let y1 = title_block(p, sc, tx, y + pad + small * 1.9, tw, (side * 0.4).clamp(16.0, 80.0), false, true);
    let pcap = small * 0.95;
    if sc.frac.is_some() && y1 + pcap * 3.0 < y + pad + side + pad {
        progress(p, sc, tx, (y + pad + side - progress_h(sc, pcap, 4.0)).max(y1 + pad * 0.4), tw, pcap, 4.0);
    }
}

// ------------------------------------------------------------------ cassette

fn draw_cassette(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let pad = pad_of(r);
    let bw = if tall { w - 2.0 * pad } else { ((h - 2.0 * pad) * 1.6).min(w * 0.52) };
    let bh = bw / 1.6;
    let (bx, by) = if tall { (x + pad, y + pad) } else { (x + pad, y + (h - bh) / 2.0) };
    let body = (bx, by, bw, bh);
    let rr = bh * 0.06;
    p.layer.fill_rrect_solid((bx + 4.0, by + 7.0, bw, bh), rr, (0, 0, 0), 90);
    p.layer.fill_rrect_solid(body, rr, (36, 34, 42), 255);
    p.layer.stroke_rrect(body, rr, 1.5, (255, 255, 255), 44);
    for (fx, fy) in [(0.04, 0.07), (0.96, 0.07), (0.04, 0.93), (0.96, 0.93)] {
        p.layer.dot(bx + bw * fx, by + bh * fy, bh * 0.024, (150, 150, 160), 255);
        p.layer.dot(bx + bw * fx, by + bh * fy, bh * 0.01, (30, 30, 36), 255);
    }
    // the label: the cover, the title and the artist in ink
    let lab = (bx + bw * 0.06, by + bh * 0.09, bw * 0.88, bh * 0.4);
    let paper = (0xEC, 0xE4, 0xCF);
    p.layer.fill_rrect_solid(lab, bh * 0.03, paper, 255);
    p.layer.fill_rrect_solid((lab.0, lab.1 + lab.3 * 0.8, lab.2, lab.3 * 0.12), 0.0, sc.c(0), 255);
    let th = lab.3 * 0.8;
    let thumb = (lab.0 + lab.3 * 0.1, lab.1 + lab.3 * 0.1, th, th);
    blit_cover(p.layer, sc.cover, thumb, 3.0);
    let ink = (0x2A, 0x24, 0x20);
    let tx = thumb.0 + th + lab.3 * 0.16;
    let tw = lab.0 + lab.2 - tx - lab.3 * 0.1;
    let c1 = line(p, tx, lab.1 + lab.3 * 0.38, &sc.title, lab.3 * 0.25, lab.3 * 0.12, tw, W::Medium, ink, 255, false);
    let _ = c1;
    line(p, tx, lab.1 + lab.3 * 0.68, &sc.artist, lab.3 * 0.16, lab.3 * 0.1, tw, W::Regular, ink, 200, false);
    // the window with the two reels
    let win = (bx + bw * 0.2, by + bh * 0.54, bw * 0.6, bh * 0.28);
    p.layer.fill_rrect_solid(win, win.3 * 0.5, (12, 12, 16), 255);
    p.layer.stroke_rrect(win, win.3 * 0.5, 1.2, (255, 255, 255), 40);
    let f = sc.frac.unwrap_or(0.3).clamp(0.0, 1.0);
    let rmax = win.3 * 0.47;
    let hub = rmax * 0.46;
    for (side, cxr) in [(0usize, win.0 + win.2 * 0.27), (1usize, win.0 + win.2 * 0.73)] {
        let cyr = win.1 + win.3 / 2.0;
        let share = if side == 0 { 1.0 - f } else { f };
        let tape = hub + (rmax - hub) * (0.12 + 0.88 * share);
        p.layer.dot(cxr, cyr, tape, (74, 50, 38), 255);
        p.layer.dot(cxr, cyr, hub * 1.18, (232, 232, 236), 255);
        p.layer.dot(cxr, cyr, hub * 0.82, (22, 22, 28), 255);
        for k in 0..6 {
            let a = sc.spin * if side == 0 { 1.0 } else { 1.0 } + k as f32 * std::f32::consts::TAU / 6.0;
            let (s, c) = a.sin_cos();
            p.layer.line(cxr + s * hub * 0.35, cyr - c * hub * 0.35, cxr + s * hub * 0.86, cyr - c * hub * 0.86, (hub * 0.16).max(1.5), (232, 232, 236), 255);
        }
    }
    // the bottom of the cassette
    let foot = (bx + bw * 0.22, by + bh * 0.85, bw * 0.56, bh * 0.12);
    p.layer.fill_rrect_solid(foot, bh * 0.03, (22, 21, 27), 255);
    for fx in [0.2, 0.8] {
        p.layer.dot(foot.0 + foot.2 * fx, foot.1 + foot.3 / 2.0, foot.3 * 0.26, (8, 8, 10), 255);
    }

    // the text beside it (under it when the screen is tall)
    let (px, pw, py) = if tall { (x + pad, w - 2.0 * pad, by + bh + pad) } else {
        let px = bx + bw + pad * 1.8;
        (px, x + w - pad - px, y + pad)
    };
    let small = (h.min(w) * 0.034).clamp(9.0, 17.0);
    label(p, sc, px, py + small, small);
    clock(p, sc, px + pw, py + small, small * 1.15);
    let bottom = y + h - pad;
    let pcap = small * 0.95;
    let ph = progress_h(sc, pcap, 6.0);
    let y1 = title_block(p, sc, px, py + small * 2.2, pw, (h * 0.15).clamp(18.0, 80.0), false, true);
    let zone = (px, y1 + pad * 0.6, pw, bottom - ph - y1 - pad);
    if zone.3 > 24.0 {
        bars_up(p, sc, (zone.0, zone.1, zone.2, zone.3.min(h * 0.22)), ((pw / 12.0) as usize).clamp(10, 48), false);
    }
    progress(p, sc, px, bottom - ph, pw, pcap, 6.0);
}

// ------------------------------------------------------------------------ vu

fn draw_vu(p: &mut Painter, sc: &Scene, r: R) {
    let r = footer(p, sc, r);
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let pad = pad_of(r);
    let mw = if tall { w - 2.0 * pad } else { ((w - 2.0 * pad) * 0.28).min((h - 2.0 * pad) * 1.3) };
    let mh = (mw * 0.78).min(if tall { (h - 3.0 * pad) * 0.3 } else { h - 2.0 * pad });
    let mw = if tall { mh / 0.78 } else { mw };
    let meters: Vec<(f32, f32)> = if tall {
        vec![(x + (w - mw) / 2.0, y + pad), (x + (w - mw) / 2.0, y + pad * 2.0 + mh)]
    } else {
        vec![(x + pad, y + (h - mh) / 2.0), (x + w - pad - mw, y + (h - mh) / 2.0)]
    };
    for (i, (mx, my)) in meters.iter().enumerate() {
        let face = (*mx, *my, mw, mh);
        let paper = (0xF0, 0xE4, 0xBE);
        let ink = (0x2E, 0x26, 0x1C);
        p.layer.fill_rrect_solid((mx + 3.0, my + 5.0, mw, mh), mh * 0.06, (0, 0, 0), 90);
        p.layer.fill_rrect_solid(face, mh * 0.06, (28, 26, 30), 255);
        p.layer.fill_rrect_solid((mx + mh * 0.04, my + mh * 0.04, mw - mh * 0.08, mh - mh * 0.08), mh * 0.04, paper, 255);
        let (cx, cy) = (mx + mw / 2.0, my + mh * 0.9);
        let rad = mh * 0.72;
        let (start, sweep) = (-52.0f32, 104.0f32);
        // the scale: an arc, its red end, the ticks
        p.layer.gauge(cx, cy, rad, 2.0, start.rem_euclid(360.0), sweep, 1.0, &|_| ink, 230);
        p.layer.gauge(cx, cy, rad, 7.0, (start + sweep * 0.72).rem_euclid(360.0), sweep * 0.28, 1.0, &|_| (0xC8, 0x2E, 0x24), 235);
        for k in 0..=20 {
            let a = (start + sweep * k as f32 / 20.0).to_radians();
            let (s, c) = (a.sin(), -a.cos());
            let major = k % 5 == 0;
            let l = rad * if major { 0.13 } else { 0.07 };
            let col = if k as f32 / 20.0 >= 0.72 { (0xC8, 0x2E, 0x24) } else { ink };
            p.layer.line(cx + s * (rad - l - 4.0), cy + c * (rad - l - 4.0), cx + s * (rad - 4.0), cy + c * (rad - 4.0), if major { 2.0 } else { 1.2 }, col, 235);
        }
        let ncap = mh * 0.085;
        txt_c(p, cx, my + mh * 0.5, "VU", ncap * 2.2, W::Medium, ink, 200);
        txt(p, mx + mh * 0.12, my + mh * 0.2, if i == 0 { "L" } else { "R" }, ncap * 1.5, W::Medium, ink, 200);
        // the needle
        let lvl = sc.vu[i].clamp(0.0, 1.0);
        let a = (start + sweep * lvl).to_radians();
        let (s, c) = (a.sin(), -a.cos());
        p.layer.line(cx + 2.0, cy + 3.0, cx + s * rad * 0.97 + 2.0, cy + c * rad * 0.97 + 3.0, 3.0, (0, 0, 0), 50);
        p.layer.line(cx, cy, cx + s * rad * 0.97, cy + c * rad * 0.97, 2.4, (0xB0, 0x24, 0x1A), 255);
        // the cover over the lower part of the face hides the pivot, as on the real thing
        p.layer.fill_rrect_solid((mx + mh * 0.04, my + mh * 0.9, mw - mh * 0.08, mh * 0.06), 0.0, (28, 26, 30), 255);
        p.layer.dot(cx, cy, mh * 0.04, (28, 26, 30), 255);
    }
    // the display between them
    let (dx, dw, dy, dh) = if tall {
        (x + pad, w - 2.0 * pad, y + pad * 3.0 + mh * 2.0, (y + h - pad) - (y + pad * 3.0 + mh * 2.0))
    } else {
        let dx = x + pad + mw + pad;
        (dx, x + w - pad - mw - pad - dx, y + pad, h - 2.0 * pad)
    };
    if dw < 60.0 || dh < 40.0 {
        return;
    }
    p.layer.fill_rrect_solid((dx, dy, dw, dh), dh * 0.05, (10, 11, 14), 235);
    p.layer.stroke_rrect((dx, dy, dw, dh), dh * 0.05, 1.5, sc.c(0), 90);
    let ip = (dh * 0.07).clamp(8.0, 22.0);
    let small = (dh * 0.07).clamp(9.0, 16.0);
    let (ix, iw) = (dx + ip * 1.4, dw - ip * 2.8);
    label(p, sc, ix, dy + ip + small, small);
    clock(p, sc, ix + iw, dy + ip + small, small * 1.1);
    let pcap = small * 0.95;
    let ph = progress_h(sc, pcap, 5.0);
    let y1 = title_block(p, sc, ix, dy + ip + small * 2.2, iw, (dh * 0.17).clamp(14.0, 56.0), false, true);
    let zone = (ix, y1 + ip * 0.6, iw, dy + dh - ip - ph - y1 - ip * 1.2);
    if zone.3 > 18.0 {
        hair(p, (ix, zone.1 - ip * 0.3, iw, 1.0), sc.c(0), 60);
        bars_up(p, sc, zone, ((iw / 9.0) as usize).clamp(12, 64), false);
    }
    progress(p, sc, ix, dy + dh - ip - ph, iw, pcap, 5.0);
}

// -------------------------------------------------------------------- screen

struct Backdrop {
    size: (u32, u32),
    bg: music_screen::Bg,
    cover: Option<Arc<CoverArt>>,
    accent: Rgb,
    fb: Option<Framebuffer>,
}

struct CoverTex {
    cover: Option<Arc<CoverArt>>,
    tex: Tex,
}

pub struct MusicScreen {
    layer: Layer,
    started: std::time::Instant,
    last: Option<f64>,
    spin: f32,
    vu: [f32; 2],
    bd: Option<Backdrop>,
    cover: Option<CoverTex>,
    effect: crate::effect::Effect,
}

/// Side of the square the cover is kept at.
const COVER_PX: u32 = 480;

fn same_cover(a: &Option<Arc<CoverArt>>, b: &Option<Arc<CoverArt>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// The cover (or the record drawn when there is none) as a square picture.
fn make_tex(cover: &Option<Arc<CoverArt>>, accent: Rgb) -> Tex {
    let img = cover
        .as_ref()
        .and_then(|c| image::RgbImage::from_raw(c.width, c.height, c.rgb.clone()))
        .map(|img| {
            let (w, h) = img.dimensions();
            let side = w.min(h);
            let sq = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
            image::imageops::resize(&sq, COVER_PX, COVER_PX, image::imageops::FilterType::Triangle)
        })
        .unwrap_or_else(|| music_screen::placeholder(COVER_PX, accent));
    let mut px = Vec::with_capacity((COVER_PX * COVER_PX * 4) as usize);
    for p in img.pixels() {
        px.extend([p.0[0], p.0[1], p.0[2], 255]);
    }
    Tex { w: COVER_PX as usize, h: COVER_PX as usize, px }
}

impl MusicScreen {
    pub fn new() -> Self {
        MusicScreen {
            layer: Layer::new(1, 1),
            started: std::time::Instant::now(),
            last: None,
            spin: 0.0,
            vu: [0.0; 2],
            bd: None,
            cover: None,
            effect: crate::effect::Effect::new(),
        }
    }

    /// `secs`: a fixed time (the editor's preview); `None` = the real clock.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, fb: &mut Framebuffer, area: Rect, track: &TrackInfo, heights: &[f32], peaks: &[f32], stats: &Stats, o: &UiOptions, secs: Option<f64>) {
        let s = &o.music;
        if s.layout == Layout::Classic {
            return;
        }
        let MusicScreen { layer, started, last, spin, vu, bd, cover, effect } = self;
        let (ax, ay, aw, ah) = area;
        let (fw, fh) = (fb.width(), fb.height());
        // the background and the accent of the cover
        let stale = match bd {
            Some(b) => b.size != (fw, fh) || b.bg != o.music_bg || !same_cover(&b.cover, &track.cover),
            None => true,
        };
        if stale {
            let (accent, img) = music_screen::background(fw, fh, &track.cover, o.music_bg);
            *bd = Some(Backdrop { size: (fw, fh), bg: o.music_bg, cover: track.cover.clone(), accent, fb: img });
        }
        let Some(back) = bd.as_ref() else { return };
        match &back.fb {
            Some(img) => fb.copy_from(img),
            None if o.music_bg.mode == music_screen::BgMode::Effect => {
                effect.render(fb, &o.effect, o.effect.style_or_default(), o.music_bg.brightness);
            }
            None => {}
        }
        let accent = back.accent;
        if cover.as_ref().is_none_or(|c| !same_cover(&c.cover, &track.cover)) {
            *cover = Some(CoverTex { cover: track.cover.clone(), tex: make_tex(&track.cover, accent) });
        }
        let Some(tex) = cover.as_ref().map(|c| &c.tex) else { return };

        let now = secs.unwrap_or_else(|| started.elapsed().as_secs_f64());
        let playing = track.timeline.as_ref().is_none_or(|t| t.playing);
        let omega = 0.3 * std::f32::consts::TAU;
        match (secs, *last) {
            (Some(fixed), _) => *spin = fixed as f32 * omega,
            (None, Some(prev)) if playing => *spin += ((now - prev).clamp(0.0, 0.5) as f32) * omega,
            _ => {}
        }
        let dt = last.map_or(0.0, |p| (now - p).clamp(0.0, 0.5)) as f32;
        *last = Some(now);
        // the meters: the mean of each half of the spectrum, with a little inertia
        let half = heights.len() / 2;
        let mean = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };
        let targets = if heights.len() >= 2 { [mean(&heights[..half]), mean(&heights[half..])] } else { [0.0, 0.0] };
        for i in 0..2 {
            let t = (targets[i] * 1.5).clamp(0.0, 1.0);
            let k = if secs.is_some() { 1.0 } else { (dt * 9.0).clamp(0.05, 1.0) };
            vu[i] += (t - vu[i]) * k;
        }

        let (frac, pos, dur) = match track.timeline.as_ref().filter(|_| o.music_progress) {
            Some(tl) if tl.duration_ms > 0 => {
                let p = tl.position_now_ms();
                (Some((p as f32 / tl.duration_ms as f32).clamp(0.0, 1.0)), music_screen::fmt_time(p), music_screen::fmt_time(tl.duration_ms))
            }
            _ => (None, String::new(), String::new()),
        };
        let items: &[music_screen::StatItem] = if o.music_stats_set { &o.music_stats } else { &music_screen::DEFAULT_STATS };
        let font = s.font.unwrap_or(FontStyle::Smooth);
        let sc = Scene {
            s,
            ink: s.text.unwrap_or((0xF4, 0xF5, 0xFA)),
            col: s.accents(accent),
            title: if track.title.trim().is_empty() { "-".to_string() } else { track.title.clone() },
            artist: track.artist.clone(),
            album: track.album.clone(),
            cover: tex,
            frac,
            pos,
            dur,
            bars: heights,
            peaks,
            vu: *vu,
            spin: *spin,
            clock: chrono::Local::now().format("%H:%M").to_string(),
            footer: music_screen::footer_segments(items, stats, o),
        };
        if layer.w != aw as usize || layer.h != ah as usize {
            *layer = Layer::new(aw, ah);
        } else {
            layer.clear();
        }
        let inset = (ah as f32 * 0.03).clamp(6.0, 16.0);
        let rect = (inset, inset, aw as f32 - 2.0 * inset, ah as f32 - 2.0 * inset);
        if rect.2 < 120.0 || rect.3 < 80.0 {
            return;
        }
        let mut painter = Painter { layer, font };
        match s.layout {
            Layout::Classic => {}
            Layout::Vinyl => draw_vinyl(&mut painter, &sc, rect),
            Layout::Poster => draw_poster(&mut painter, &sc, rect),
            Layout::Minimal => draw_minimal(&mut painter, &sc, rect),
            Layout::Radial => draw_radial(&mut painter, &sc, rect),
            Layout::Equalizer => draw_equalizer(&mut painter, &sc, rect),
            Layout::Wave => draw_wave(&mut painter, &sc, rect),
            Layout::Cassette => draw_cassette(&mut painter, &sc, rect),
            Layout::Vu => draw_vu(&mut painter, &sc, rect),
        }
        layer.composite_onto(fb, ax, ay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Timeline;

    fn bars() -> Vec<f32> {
        (0..48).map(|i| 0.15 + 0.7 * ((i as f32 * 0.4).sin() * 0.5 + 0.5)).collect()
    }

    fn track(cover: bool) -> TrackInfo {
        let cover = cover.then(|| {
            let mut rgb = Vec::new();
            for y in 0..64u32 {
                for x in 0..64u32 {
                    rgb.extend([(x * 4) as u8, (y * 4) as u8, 180]);
                }
            }
            Arc::new(CoverArt { width: 64, height: 64, rgb })
        });
        TrackInfo {
            title: "A very long song title that has to be cut somewhere".into(),
            artist: "Some Artist".into(),
            album: "Some Album".into(),
            cover,
            timeline: Some(Timeline { position_ms: 83_000, duration_ms: 215_000, playing: true, sampled_at: std::time::Instant::now() }),
        }
    }

    fn opts(layout: Layout) -> UiOptions {
        let mut o = UiOptions::default();
        o.music_screen = true;
        o.music.layout = layout;
        o
    }

    fn lit(fb: &Framebuffer, w: u32, h: u32) -> usize {
        let b = fb.as_bytes();
        let mut n = 0;
        for i in 0..(w * h) as usize {
            if b[i * 3] > 60 || b[i * 3 + 1] > 60 || b[i * 3 + 2] > 60 {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn the_names_are_all_understood() {
        for l in Layout::ALL {
            assert_eq!(Layout::parse(l.name()), Some(l));
        }
        assert_eq!(Layout::parse("Vinile"), Some(Layout::Vinyl));
        assert_eq!(Layout::parse("nope"), None);
        assert!(Layout::NAMES.split(" | ").count() == Layout::ALL.len());
    }

    #[test]
    fn every_design_draws_on_every_shape_with_or_without_cover() {
        let b = bars();
        let st = Stats { cpu: Some(23.0), gpu: Some(40.0), ram_mb: Some((8000, 32000)), ..Stats::default() };
        for layout in Layout::ALL.into_iter().filter(|l| *l != Layout::Classic) {
            for (w, h) in [(1920u32, 462u32), (462, 1920), (800, 480), (480, 800), (1000, 1000), (320, 200)] {
                for cover in [true, false] {
                    let mut fb = Framebuffer::new(trofeo_lcd::Resolution { width: w, height: h });
                    let mut scr = MusicScreen::new();
                    let o = opts(layout);
                    scr.draw(&mut fb, (0, 0, w, h), &track(cover), &b, &b, &st, &o, Some(3.0));
                    assert!(lit(&fb, w, h) > (w * h / 200) as usize, "{layout:?} {w}x{h} cover {cover}: almost nothing drawn");
                }
            }
        }
    }

    #[test]
    fn the_classic_layout_leaves_the_screen_alone() {
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution { width: 400, height: 200 });
        let before = fb.as_bytes().to_vec();
        MusicScreen::new().draw(&mut fb, (0, 0, 400, 200), &track(true), &bars(), &bars(), &Stats::default(), &opts(Layout::Classic), Some(1.0));
        assert_eq!(fb.as_bytes(), &before[..]);
    }

    #[test]
    fn options_parse_and_are_checked() {
        assert_eq!(parse(&ConfigFile::parse("").unwrap()).unwrap(), Settings::default());
        let s = parse(&ConfigFile::parse("music_layout = cassette\nmusic_colors = #FF0000, #00FF00\nmusic_font = pixel\nmusic_clock = false\nmusic_album = no\nmusic_glow = 10\nmusic_text_color = #FFFFFF\n").unwrap()).unwrap();
        assert_eq!(s.layout, Layout::Cassette);
        assert_eq!(s.colors.as_ref().map(|c| c.len()), Some(2));
        assert_eq!((s.font, s.clock, s.album, s.glow, s.text), (Some(FontStyle::Pixel), false, false, 10, Some((255, 255, 255))));
        for bad in ["music_layout = disco-ball\n", "music_glow = 101\n", "music_font = comic\n", "music_colors = nonsense\n"] {
            assert!(parse(&ConfigFile::parse(bad).unwrap()).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_spectrum_is_resampled_both_ways() {
        let v = [0.0, 1.0];
        let up = resample(&v, 5);
        assert_eq!((up[0], up[4]), (0.0, 1.0));
        assert!(up.windows(2).all(|w| w[0] <= w[1]));
        let down = resample(&[0.1, 0.9, 0.2, 0.3], 2);
        assert_eq!(down, vec![0.9, 0.3]);
        assert_eq!(resample(&[], 3), vec![0.0; 3]);
    }

    #[test]
    fn the_design_colours_are_never_empty() {
        for l in Layout::ALL {
            assert!(!palette(l, (10, 200, 30)).is_empty());
        }
        let s = Settings { colors: Some(vec![(1, 2, 3)]), ..Settings::default() };
        assert_eq!(s.accents((9, 9, 9)), vec![(1, 2, 3)]);
    }
}
