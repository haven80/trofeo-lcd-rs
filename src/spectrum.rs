//! Spectrum (EQ) drawing styles and color palettes.
//!
//! - [`Style`]: how the 48 bar heights are drawn (classic bars, LED segments,
//!   bars with falling peak caps, a smooth filled wave, a mirrored
//!   centre-out layout). One style is fixed; a list rotates over time.
//! - [`Palette`] / [`Colors`]: where the colors come from. Built-in presets
//!   (fire, ocean, ...), a scrolling rainbow, or a user-defined gradient; a
//!   list of palettes rotates over time with a short cross-fade.
//! - [`PeakTracker`]: the state behind the "peak cap" style (hold, then fall).
//!
//! Everything is pure drawing/maths on top of `Framebuffer`, so it is fully
//! unit-tested without a display. The classic look (no style, no palette) is
//! pixel-identical to what the program drew before this module existed.

use std::sync::OnceLock;
use std::time::Instant;
use trofeo_lcd::Framebuffer;

pub type Rgb = (u8, u8, u8);

/// Colors are asked for as `(level, pos)`: `level` (0-1) is the height the
/// color belongs to (the bar's height, or the vertical position inside a
/// segment/column), `pos` (0-1) is the horizontal position across the spectrum.
pub type ColorFn<'a> = &'a dyn Fn(f32, f32) -> Rgb;

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

static START: OnceLock<Instant> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static TEST_TIME: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// Seconds since the first call (monotonic) — the clock driving rotation and
/// the rainbow scroll.
pub fn now_secs() -> f32 {
    #[cfg(test)]
    if let Some(t) = TEST_TIME.with(|c| c.get()) {
        return t;
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

#[cfg(test)]
pub fn set_test_time(t: Option<f32>) {
    TEST_TIME.with(|c| c.set(t));
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Classic solid bars.
    Bars,
    /// Bars made of separate LED-like segments (dim "ghost" segments above).
    Led,
    /// Solid bars plus a cap that holds at the peak, then falls slowly.
    Peaks,
    /// Smooth filled wave with a bright outline.
    Area,
    /// Bars growing up AND down from a centre line.
    Mirror,
}

impl Style {
    pub const ALL: [Style; 5] = [Style::Bars, Style::Led, Style::Peaks, Style::Area, Style::Mirror];
    pub const NAMES: &'static str = "bars | led | peaks | area | mirror";

    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bars" | "bar" | "barre" => Some(Style::Bars),
            "led" | "vu" | "segments" | "segmenti" => Some(Style::Led),
            "peaks" | "peak" | "picchi" => Some(Style::Peaks),
            "area" | "wave" | "onda" => Some(Style::Area),
            "mirror" | "symmetric" | "specchio" => Some(Style::Mirror),
            _ => None,
        }
    }
}

/// The style active at `time`: empty list = classic bars, one entry = fixed,
/// several = rotate every `interval` seconds.
pub fn pick_style(styles: &[Style], interval: u32, time: f32) -> Style {
    match styles.len() {
        0 => Style::Bars,
        1 => styles[0],
        n => styles[((time / interval.max(1) as f32).floor().max(0.0) as usize) % n],
    }
}

// ---------------------------------------------------------------------------
// Palettes
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Palette {
    /// The original green -> yellow -> red level gradient.
    Default,
    /// Hue by horizontal position, scrolling over time.
    Rainbow,
    Fire,
    Ocean,
    Sunset,
    Neon,
    Ice,
    Matrix,
    Purple,
    /// User gradient (`spectrum_gradient`), evenly spaced stops, low -> high.
    Custom(Vec<Rgb>),
}

const FIRE: [Rgb; 4] = [(0x60, 0x00, 0x00), (0xE0, 0x30, 0x00), (0xFF, 0xA0, 0x00), (0xFF, 0xF0, 0x80)];
const OCEAN: [Rgb; 4] = [(0x00, 0x30, 0x90), (0x00, 0x90, 0xE0), (0x00, 0xE0, 0xD0), (0xE0, 0xFF, 0xFF)];
const SUNSET: [Rgb; 4] = [(0x60, 0x20, 0xA0), (0xE0, 0x30, 0x90), (0xFF, 0x80, 0x30), (0xFF, 0xE0, 0x60)];
const NEON: [Rgb; 4] = [(0xFF, 0x00, 0xA0), (0xA0, 0x30, 0xFF), (0x00, 0xC0, 0xFF), (0x00, 0xFF, 0xD0)];
const ICE: [Rgb; 4] = [(0x40, 0x60, 0xC0), (0x70, 0xB0, 0xFF), (0xC0, 0xF0, 0xFF), (0xFF, 0xFF, 0xFF)];
const MATRIX: [Rgb; 4] = [(0x00, 0x50, 0x10), (0x00, 0xB0, 0x30), (0x40, 0xFF, 0x60), (0xD0, 0xFF, 0xD8)];
const PURPLE: [Rgb; 4] = [(0x40, 0x20, 0x90), (0x80, 0x40, 0xE0), (0xD0, 0x60, 0xF0), (0xFF, 0xB0, 0xF0)];

impl Palette {
    pub const NAMES: &'static str = "default | rainbow | fire | ocean | sunset | neon | ice | matrix | purple";

    /// Every built-in preset except `default` (what `all`/`rotate` expands to).
    pub fn presets() -> Vec<Palette> {
        vec![
            Palette::Rainbow,
            Palette::Fire,
            Palette::Ocean,
            Palette::Sunset,
            Palette::Neon,
            Palette::Ice,
            Palette::Matrix,
            Palette::Purple,
        ]
    }

    pub fn parse(s: &str) -> Option<Palette> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" | "classic" | "classico" => Some(Palette::Default),
            "rainbow" | "arcobaleno" => Some(Palette::Rainbow),
            "fire" | "fuoco" => Some(Palette::Fire),
            "ocean" | "oceano" => Some(Palette::Ocean),
            "sunset" | "tramonto" => Some(Palette::Sunset),
            "neon" => Some(Palette::Neon),
            "ice" | "ghiaccio" => Some(Palette::Ice),
            "matrix" => Some(Palette::Matrix),
            "purple" | "viola" => Some(Palette::Purple),
            _ => None,
        }
    }

    fn stops(&self) -> Option<&[Rgb]> {
        match self {
            Palette::Fire => Some(&FIRE),
            Palette::Ocean => Some(&OCEAN),
            Palette::Sunset => Some(&SUNSET),
            Palette::Neon => Some(&NEON),
            Palette::Ice => Some(&ICE),
            Palette::Matrix => Some(&MATRIX),
            Palette::Purple => Some(&PURPLE),
            Palette::Custom(v) => Some(v),
            Palette::Default | Palette::Rainbow => None,
        }
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    (lerp_u8(a.0, b.0, t), lerp_u8(a.1, b.1, t), lerp_u8(a.2, b.2, t))
}

/// Brighten a color toward white by `amount` (0-1).
fn lighten(c: Rgb, amount: f32) -> Rgb {
    lerp_rgb(c, (255, 255, 255), amount)
}

/// Scale a color's brightness (0-1).
fn dim(c: Rgb, factor: f32) -> Rgb {
    (
        (c.0 as f32 * factor) as u8,
        (c.1 as f32 * factor) as u8,
        (c.2 as f32 * factor) as u8,
    )
}

/// Sample evenly spaced gradient `stops` at `t` (0-1).
pub fn gradient(stops: &[Rgb], t: f32) -> Rgb {
    match stops.len() {
        0 => (255, 255, 255),
        1 => stops[0],
        n => {
            let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (x.floor() as usize).min(n - 2);
            lerp_rgb(stops[i], stops[i + 1], x - i as f32)
        }
    }
}

/// The original level gradient: green (quiet) -> yellow -> red (loud).
pub fn classic(level: f32) -> Rgb {
    let level = level.clamp(0.0, 1.0);
    if level < 0.6 {
        let t = level / 0.6;
        ((0x20 as f32 + t * (0xE0 - 0x20) as f32) as u8, 0xE0, 0x30)
    } else {
        let t = (level - 0.6) / 0.4;
        (0xE0, (0xE0 as f32 * (1.0 - t)) as u8, 0x30)
    }
}

/// HSV (hue in degrees, s/v 0-1) to RGB.
fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

/// The active palette set + clock: resolves `(level, pos)` to a color,
/// cross-fading between palettes when there are several.
pub struct Colors<'a> {
    pub palettes: &'a [Palette],
    /// Seconds each palette stays before the next one fades in.
    pub interval: f32,
    /// Rainbow hue scroll, degrees per second.
    pub rainbow_speed: f32,
    pub time: f32,
}

/// Cross-fade length at the end of each palette's slot.
const FADE_SECS: f32 = 2.0;

impl Colors<'_> {
    fn one(&self, p: &Palette, level: f32, pos: f32) -> Rgb {
        let level = level.clamp(0.0, 1.0);
        match p {
            Palette::Default => classic(level),
            Palette::Rainbow => hsv(pos * 300.0 + self.time * self.rainbow_speed, 1.0, 0.55 + 0.45 * level),
            other => gradient(other.stops().unwrap_or(&[]), level),
        }
    }

    pub fn at(&self, level: f32, pos: f32) -> Rgb {
        let n = self.palettes.len();
        match n {
            0 => classic(level),
            1 => self.one(&self.palettes[0], level, pos),
            _ => {
                let interval = self.interval.max(1.0);
                let slot = (self.time / interval).floor().max(0.0);
                let local = self.time - slot * interval;
                let idx = slot as usize % n;
                let cur = self.one(&self.palettes[idx], level, pos);
                let fade = (interval * 0.5).min(FADE_SECS);
                if local > interval - fade {
                    let t = ((local - (interval - fade)) / fade).clamp(0.0, 1.0);
                    let t = t * t * (3.0 - 2.0 * t); // smoothstep
                    lerp_rgb(cur, self.one(&self.palettes[(idx + 1) % n], level, pos), t)
                } else {
                    cur
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Peak caps
// ---------------------------------------------------------------------------

/// Seconds a peak cap stays put before it starts falling.
pub const PEAK_HOLD_SECS: f32 = 0.4;
/// Fall speed of a cap, in "full heights" per second.
pub const PEAK_FALL_PER_SEC: f32 = 0.5;

pub struct PeakTracker {
    peaks: Vec<f32>,
    hold: Vec<f32>,
}

impl PeakTracker {
    pub fn new(n: usize) -> Self {
        Self { peaks: vec![0.0; n], hold: vec![0.0; n] }
    }

    pub fn peaks(&self) -> &[f32] {
        &self.peaks
    }

    /// Advance by `dt` seconds given the current bar `heights` (0-1).
    pub fn update(&mut self, heights: &[f32], dt: f32) {
        for ((p, hold), &h) in self.peaks.iter_mut().zip(self.hold.iter_mut()).zip(heights) {
            if h >= *p {
                *p = h;
                *hold = PEAK_HOLD_SECS;
            } else if *hold > 0.0 {
                *hold -= dt;
            } else {
                *p = (*p - PEAK_FALL_PER_SEC * dt).max(h);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Gap between bars, as in the original look.
const GAP: u32 = 3;

fn pos_of(i: usize, n: usize) -> f32 {
    if n > 1 {
        i as f32 / (n - 1) as f32
    } else {
        0.0
    }
}

/// Draw the spectrum `heights` (0-1 each) in `area` = (x, y, w, h).
/// `peaks` is only used by [`Style::Peaks`] (same length as `heights`, or empty).
pub fn draw(
    fb: &mut Framebuffer,
    style: Style,
    area: (u32, u32, u32, u32),
    heights: &[f32],
    peaks: &[f32],
    color: ColorFn,
) {
    if heights.is_empty() || area.2 == 0 || area.3 == 0 {
        return;
    }
    match style {
        Style::Bars => draw_bars(fb, area, heights, None, color),
        Style::Peaks => draw_bars(fb, area, heights, Some(peaks), color),
        Style::Led => draw_led(fb, area, heights, color),
        Style::Area => draw_area(fb, area, heights, color),
        Style::Mirror => draw_mirror(fb, area, heights, color),
    }
}

fn bar_width(w: u32, n: usize) -> u32 {
    w.saturating_sub(GAP * (n as u32 + 1)) / n as u32
}

fn draw_bars(fb: &mut Framebuffer, area: (u32, u32, u32, u32), heights: &[f32], peaks: Option<&[f32]>, color: ColorFn) {
    let (left, top, w, sh) = area;
    let n = heights.len();
    let bw = bar_width(w, n);
    let cap_h = (sh / 100).clamp(2, 6);
    let mut x = left + GAP;
    for (i, &h) in heights.iter().enumerate() {
        let h = h.clamp(0.0, 1.0);
        let bar_h = (sh as f32 * h).round() as u32;
        let (r, g, b) = color(h, pos_of(i, n));
        fb.fill_rect(x, top + (sh - bar_h), bw, bar_h, r, g, b);
        if let Some(&p) = peaks.and_then(|p| p.get(i)) {
            let ph = ((sh as f32 * p.clamp(0.0, 1.0)).round() as u32).max(cap_h).min(sh);
            let (r, g, b) = lighten(color(p.clamp(0.0, 1.0), pos_of(i, n)), 0.35);
            fb.fill_rect(x, top + (sh - ph), bw, cap_h, r, g, b);
        }
        x += bw + GAP;
    }
}

fn draw_led(fb: &mut Framebuffer, area: (u32, u32, u32, u32), heights: &[f32], color: ColorFn) {
    let (left, top, w, sh) = area;
    let n = heights.len();
    let bw = bar_width(w, n);
    let pitch = (sh / 24).max(4);
    let nseg = (sh / pitch).max(1);
    let thick = pitch - 2;
    let bottom = top + sh;
    let mut x = left + GAP;
    for (i, &h) in heights.iter().enumerate() {
        let lit = (h.clamp(0.0, 1.0) * nseg as f32).round() as u32;
        for k in 0..nseg {
            let level = (k as f32 + 0.5) / nseg as f32;
            let c = color(level, pos_of(i, n));
            let (r, g, b) = if k < lit { c } else { dim(c, 0.10) };
            fb.fill_rect(x, bottom - (k + 1) * pitch + 2, bw, thick, r, g, b);
        }
        x += bw + GAP;
    }
}

/// Smooth per-pixel column heights (in pixels) across `w` by interpolating
/// between the bar centres with a smoothstep.
fn column_heights(heights: &[f32], w: u32, max_h: u32) -> Vec<u32> {
    let n = heights.len();
    (0..w)
        .map(|x| {
            let u = ((x as f32 + 0.5) / w as f32 * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
            let j = (u.floor() as usize).min(n.saturating_sub(2));
            let (a, b) = (heights[j].clamp(0.0, 1.0), heights[(j + 1).min(n - 1)].clamp(0.0, 1.0));
            let f = (u - j as f32).clamp(0.0, 1.0);
            let f = f * f * (3.0 - 2.0 * f);
            ((a + (b - a) * f) * max_h as f32).round() as u32
        })
        .collect()
}

/// Horizontal color buckets for the per-row fills of `area`/`mirror`.
const BUCKETS: u32 = 96;

fn draw_area(fb: &mut Framebuffer, area: (u32, u32, u32, u32), heights: &[f32], color: ColorFn) {
    let (left, top, w, sh) = area;
    let cols = column_heights(heights, w, sh);
    let max_h = cols.iter().copied().max().unwrap_or(0);
    let bucket = |x: u32| (x * BUCKETS / w).min(BUCKETS - 1);
    for r in 0..max_h {
        let level = (r as f32 + 0.5) / sh as f32;
        let y = top + sh - 1 - r;
        let mut x = 0u32;
        while x < w {
            if cols[x as usize] <= r {
                x += 1;
                continue;
            }
            let b = bucket(x);
            let start = x;
            while x < w && cols[x as usize] > r && bucket(x) == b {
                x += 1;
            }
            let (cr, cg, cb) = color(level, (b as f32 + 0.5) / BUCKETS as f32);
            fb.fill_rect(left + start, y, x - start, 1, cr, cg, cb);
        }
    }
    // Bright outline along the top of the wave.
    let line = (sh / 100).clamp(2, 4);
    for (x, &ch) in cols.iter().enumerate() {
        if ch == 0 {
            continue;
        }
        let t = line.min(ch);
        let (r, g, b) = lighten(color(ch as f32 / sh as f32, x as f32 / w as f32), 0.45);
        fb.fill_rect(left + x as u32, top + sh - ch, 1, t, r, g, b);
    }
}

fn draw_mirror(fb: &mut Framebuffer, area: (u32, u32, u32, u32), heights: &[f32], color: ColorFn) {
    let (left, top, w, sh) = area;
    let n = heights.len();
    let bw = bar_width(w, n);
    let half = sh.saturating_sub(2) / 2;
    if half == 0 {
        return;
    }
    let cy_up = top + half; // rows above the centre line end here (exclusive)
    let cy_dn = top + half + 2; // rows below start here
    let mut x = left + GAP;
    for (i, &h) in heights.iter().enumerate() {
        let bh = (half as f32 * h.clamp(0.0, 1.0)).round() as u32;
        for r in 0..bh {
            let (cr, cg, cb) = color((r as f32 + 0.5) / half as f32, pos_of(i, n));
            fb.fill_rect(x, cy_up - 1 - r, bw, 1, cr, cg, cb);
            fb.fill_rect(x, cy_dn + r, bw, 1, cr, cg, cb);
        }
        x += bw + GAP;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use trofeo_lcd::Resolution;

    fn canvas() -> Framebuffer {
        let mut fb = Framebuffer::new(Resolution { width: 600, height: 300 });
        fb.clear(8, 8, 16);
        fb
    }

    fn sample_heights() -> Vec<f32> {
        (0..48).map(|i| ((i as f32 * 0.37).sin() * 0.5 + 0.5) * 0.9 + 0.05).collect()
    }

    fn px(fb: &Framebuffer, x: u32, y: u32) -> Rgb {
        let i = ((y * fb.width() + x) * 3) as usize;
        let b = fb.as_bytes();
        (b[i], b[i + 1], b[i + 2])
    }

    #[test]
    fn classic_matches_the_original_gradient() {
        assert_eq!(classic(0.0), (0x20, 0xE0, 0x30));
        assert_eq!(classic(1.0), (0xE0, 0x00, 0x30));
        assert_eq!(classic(0.6), (0xE0, 0xE0, 0x30));
    }

    #[test]
    fn gradient_hits_its_stops() {
        let s = [(0, 0, 0), (100, 100, 100), (200, 0, 50)];
        assert_eq!(gradient(&s, 0.0), (0, 0, 0));
        assert_eq!(gradient(&s, 0.5), (100, 100, 100));
        assert_eq!(gradient(&s, 1.0), (200, 0, 50));
        assert_eq!(gradient(&s, 0.25), (50, 50, 50));
        assert_eq!(gradient(&s, 7.0), (200, 0, 50)); // clamped
    }

    #[test]
    fn hsv_primaries() {
        assert_eq!(hsv(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv(240.0, 1.0, 1.0), (0, 0, 255));
        assert_eq!(hsv(360.0, 1.0, 1.0), (255, 0, 0));
    }

    #[test]
    fn parse_styles_and_palettes() {
        assert_eq!(Style::parse("LED"), Some(Style::Led));
        assert_eq!(Style::parse(" specchio "), Some(Style::Mirror));
        assert_eq!(Style::parse("nope"), None);
        assert_eq!(Palette::parse("Fire"), Some(Palette::Fire));
        assert_eq!(Palette::parse("arcobaleno"), Some(Palette::Rainbow));
        assert_eq!(Palette::parse("nope"), None);
        assert_eq!(Palette::presets().len(), 8);
        assert!(!Palette::presets().contains(&Palette::Default));
    }

    #[test]
    fn every_preset_has_distinct_ends_and_is_visible() {
        for p in Palette::presets().into_iter().filter(|p| *p != Palette::Rainbow) {
            let s = p.stops().unwrap();
            assert_ne!(s[0], s[s.len() - 1], "{p:?}");
            // Even the quietest level must stay clearly visible on the dark background.
            let lo = gradient(s, 0.0);
            assert!(lo.0 as u32 + lo.1 as u32 + lo.2 as u32 >= 0x60, "{p:?} too dark at level 0: {lo:?}");
        }
    }

    #[test]
    fn style_rotation_cycles_and_handles_edges() {
        assert_eq!(pick_style(&[], 30, 99.0), Style::Bars);
        assert_eq!(pick_style(&[Style::Led], 30, 99.0), Style::Led);
        let l = [Style::Led, Style::Area, Style::Mirror];
        assert_eq!(pick_style(&l, 10, 0.0), Style::Led);
        assert_eq!(pick_style(&l, 10, 9.9), Style::Led);
        assert_eq!(pick_style(&l, 10, 10.0), Style::Area);
        assert_eq!(pick_style(&l, 10, 25.0), Style::Mirror);
        assert_eq!(pick_style(&l, 10, 30.0), Style::Led); // wraps
    }

    #[test]
    fn palette_rotation_fades_between_presets() {
        let pals = [Palette::Fire, Palette::Ocean];
        let at = |time: f32, level: f32| Colors { palettes: &pals, interval: 10.0, rainbow_speed: 0.0, time }.at(level, 0.0);
        let fire = gradient(&FIRE, 0.5);
        let ocean = gradient(&OCEAN, 0.5);
        assert_eq!(at(0.0, 0.5), fire);
        assert_eq!(at(7.9, 0.5), fire); // before the fade window (last 2 s)
        assert_eq!(at(10.0, 0.5), ocean); // slot 1, fade just finished
        assert_eq!(at(17.0, 0.5), ocean);
        let mid = at(9.0, 0.5); // halfway through the 8-10 s fade
        assert_ne!(mid, fire);
        assert_ne!(mid, ocean);
        assert_eq!(at(20.0, 0.5), fire); // wraps back to the first
    }

    #[test]
    fn rainbow_scrolls_with_time_and_is_static_at_zero_speed() {
        let pal = [Palette::Rainbow];
        let c = |time: f32, speed: f32, pos: f32| Colors { palettes: &pal, interval: 20.0, rainbow_speed: speed, time }.at(1.0, pos);
        assert_ne!(c(0.0, 30.0, 0.3), c(5.0, 30.0, 0.3));
        assert_eq!(c(0.0, 0.0, 0.3), c(50.0, 0.0, 0.3));
        assert_ne!(c(0.0, 0.0, 0.0), c(0.0, 0.0, 0.5)); // hue varies across the bars
    }

    #[test]
    fn peaks_hold_then_fall_and_never_sink_below_the_bar() {
        let mut t = PeakTracker::new(2);
        t.update(&[0.8, 0.2], 0.016);
        assert_eq!(t.peaks(), &[0.8, 0.2]);
        // Within the hold time: stays put although the bar dropped.
        t.update(&[0.1, 0.1], 0.2);
        assert_eq!(t.peaks()[0], 0.8);
        // After the hold: falls at PEAK_FALL_PER_SEC.
        t.update(&[0.1, 0.1], 0.3); // hold runs out (0.4 - 0.2 - 0.3 < 0)
        t.update(&[0.1, 0.1], 0.2);
        let p = t.peaks()[0];
        assert!(p < 0.8 && (p - (0.8 - PEAK_FALL_PER_SEC * 0.2)).abs() < 1e-5, "{p}");
        // Long fall bottoms out at the live bar height, not zero.
        for _ in 0..100 {
            t.update(&[0.1, 0.1], 0.1);
        }
        assert!((t.peaks()[0] - 0.1).abs() < 1e-6);
        // A louder bar pushes the cap up immediately.
        t.update(&[0.9, 0.1], 0.016);
        assert_eq!(t.peaks()[0], 0.9);
    }

    /// Reference copy of the pre-module bar drawing, to prove `Style::Bars` is pixel-identical.
    fn legacy_bars(fb: &mut Framebuffer, area: (u32, u32, u32, u32), heights: &[f32]) {
        let (left, top, sw, sh) = area;
        let gap = 3u32;
        let total_gap = gap * (heights.len() as u32 + 1);
        let bar_width = (sw.saturating_sub(total_gap)) / heights.len() as u32;
        let mut x = left + gap;
        for &h in heights {
            let bar_h = (sh as f32 * h).round() as u32;
            let y = top + (sh - bar_h);
            let (r, g, b) = classic(h);
            fb.fill_rect(x, y, bar_width, bar_h, r, g, b);
            x += bar_width + gap;
        }
    }

    #[test]
    fn bars_style_is_pixel_identical_to_the_original() {
        let h = sample_heights();
        let area = (20, 30, 560, 240);
        let mut a = canvas();
        let mut b = canvas();
        legacy_bars(&mut a, area, &h);
        draw(&mut b, Style::Bars, area, &h, &[], &|l, _| classic(l));
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn no_style_draws_outside_its_area_or_panics() {
        let h = sample_heights();
        let peaks: Vec<f32> = h.iter().map(|v| (v + 0.1).min(1.0)).collect();
        let area = (50u32, 40u32, 500u32, 220u32);
        for style in Style::ALL {
            let mut fb = canvas();
            draw(&mut fb, style, area, &h, &peaks, &|l, p| gradient(&FIRE, (l + p) / 2.0));
            let mut drew = false;
            for y in 0..fb.height() {
                for x in 0..fb.width() {
                    let inside = x >= area.0 && x < area.0 + area.2 && y >= area.1 && y < area.1 + area.3;
                    if px(&fb, x, y) != (8, 8, 16) {
                        assert!(inside, "{style:?} drew outside the area at ({x},{y})");
                        drew = true;
                    }
                }
            }
            assert!(drew, "{style:?} drew nothing");
        }
    }

    #[test]
    fn every_style_survives_silence_full_scale_and_tiny_areas() {
        for style in Style::ALL {
            for h in [0.0f32, 1.0, 1.5, -0.2] {
                let mut fb = canvas();
                draw(&mut fb, style, (0, 0, 600, 300), &vec![h; 48], &vec![h; 48], &|l, p| classic(l + p));
            }
            for area in [(0, 0, 1, 1), (0, 0, 10, 5), (590, 290, 10, 10), (0, 0, 0, 0)] {
                let mut fb = canvas();
                draw(&mut fb, style, area, &sample_heights(), &sample_heights(), &|l, _| classic(l));
            }
            // A single bar and an empty list must not divide by zero.
            let mut fb = canvas();
            draw(&mut fb, style, (0, 0, 600, 300), &[0.5], &[0.5], &|l, _| classic(l));
            draw(&mut fb, style, (0, 0, 600, 300), &[], &[], &|l, _| classic(l));
        }
    }

    #[test]
    fn full_height_reaches_the_top_and_silence_draws_nothing_solid() {
        let area = (0u32, 0u32, 600u32, 300u32);
        let mut fb = canvas();
        draw(&mut fb, Style::Area, area, &vec![1.0; 48], &[], &|_, _| (200, 10, 10));
        assert_eq!(px(&fb, 300, 0), lighten((200, 10, 10), 0.45)); // outline reaches the very top
        assert_eq!(px(&fb, 300, 299), (200, 10, 10));
        let mut fb = canvas();
        draw(&mut fb, Style::Mirror, area, &vec![0.0; 48], &[], &|_, _| (200, 10, 10));
        assert!(fb.as_bytes().chunks(3).all(|p| p == [8, 8, 16]));
    }

    #[test]
    fn mirror_is_symmetric_around_the_centre_line() {
        let mut fb = canvas();
        let area = (0u32, 0u32, 600u32, 300u32);
        draw(&mut fb, Style::Mirror, area, &sample_heights(), &[], &|l, p| gradient(&NEON, (l + p) / 2.0));
        // half = 149; top half ends at row 149 (exclusive), bottom half starts at row 151.
        for x in (0..600).step_by(7) {
            for r in 0..149 {
                assert_eq!(px(&fb, x, 148 - r), px(&fb, x, 151 + r), "x={x} r={r}");
            }
        }
    }

    #[test]
    fn led_lights_the_expected_number_of_segments() {
        let area = (0u32, 0u32, 600u32, 240u32);
        let mut fb = canvas();
        let mut h = vec![0.0f32; 48];
        h[0] = 0.5;
        draw(&mut fb, Style::Led, area, &h, &[], &|_, _| (200, 100, 0));
        // pitch = 240/24 = 10, 24 segments; bar 0 starts at x = 3.
        let x = 4;
        let lit = (0..24).filter(|k| px(&fb, x, 240 - (k + 1) * 10 + 3) == (200, 100, 0)).count();
        let ghost = (0..24).filter(|k| px(&fb, x, 240 - (k + 1) * 10 + 3) == (20, 10, 0)).count();
        assert_eq!(lit, 12);
        assert_eq!(ghost, 12);
    }

    #[test]
    fn peak_caps_sit_above_a_dropped_bar() {
        let area = (0u32, 0u32, 600u32, 300u32);
        let mut fb = canvas();
        let h = vec![0.2f32; 48];
        let p = vec![0.8f32; 48];
        draw(&mut fb, Style::Peaks, area, &h, &p, &|_, _| (100, 100, 100));
        let cap = lighten((100, 100, 100), 0.35);
        let y_cap = 300 - (0.8f32 * 300.0).round() as u32; // cap top row
        assert_eq!(px(&fb, 4, y_cap), cap);
        assert_eq!(px(&fb, 4, y_cap - 1), (8, 8, 16)); // nothing above the cap
        assert_eq!(px(&fb, 4, 299), (100, 100, 100)); // the bar itself is still there
        assert_eq!(px(&fb, 4, 150), (8, 8, 16)); // gap between bar and cap
    }

    #[test]
    fn column_heights_follow_the_bars_smoothly() {
        let h = [0.0f32, 1.0, 0.0];
        let cols = column_heights(&h, 300, 100);
        assert_eq!(cols.len(), 300);
        assert_eq!(*cols.iter().max().unwrap(), 100);
        // Peak in the middle, and monotonic up to it (no jaggies).
        let mid = 150;
        assert!(cols[mid] >= 99);
        assert!(cols[..mid].windows(2).all(|w| w[0] <= w[1]));
        assert!(cols[mid..].windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn now_secs_is_overridable_in_tests() {
        set_test_time(Some(12.5));
        assert_eq!(now_secs(), 12.5);
        set_test_time(None);
        assert!(now_secs() >= 0.0);
    }
}
