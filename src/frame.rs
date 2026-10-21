//! The `frame` layout: a digital picture frame. The pictures of a folder (`frame_folder`)
//! are laid out as a collage in the middle of the screen, with two slim columns of the glass
//! tiles of the `tiles` layout (clock, CPU, GPU, RAM ...) on the sides.
//!
//! * `frame_style = mosaic` (default) is a tidy mosaic of rounded photos with a small gap;
//!   `frame_style = polaroid` scatters white-bordered, slightly rotated prints with soft
//!   shadows and a piece of tape.
//! * Every `frame_interval` seconds ONE photo is replaced (cross-fade, or a wipe / a drop
//!   with `frame_transition = slide`); the rest of the collage stands still.
//! * Behind the collage (`frame_bg = blur`) there is the current photo, blurred and darkened.
//!
//! Cost: pictures are decoded and scaled once, on a background thread, straight to the size
//! of their place in the collage (a few ms of drawing per swap, nothing in between); the
//! finished scene is kept and only copied to the screen while nothing moves, so the program
//! idles at a couple of frames a second. The sides reuse the `tiles` look (`tiles_*` options).

use super::*;
use crate::tiles::{self, Rect, TileSpec, TilesScreen};
use image::{imageops, DynamicImage, ImageDecoder, RgbImage};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

type Rgb = (u8, u8, u8);

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Mosaic,
    Polaroid,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    Shuffle,
    Name,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    Fade,
    Slide,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bg {
    /// The current photo, blurred and darkened.
    Blur,
    Dark,
    /// Whatever is behind the interface on the other screens (background image/effect).
    Global,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub folder: Option<PathBuf>,
    pub style: Style,
    /// How many photos are on screen at once.
    pub photos: u32,
    /// Seconds between two swaps.
    pub interval: u32,
    pub order: Order,
    pub transition: Transition,
    /// Mosaic: pixels between the photos.
    pub gap: u32,
    /// Mosaic: corner radius of the photos.
    pub radius: u32,
    pub bg: Bg,
    /// 0-100: how dark the blurred background is.
    pub dim: u32,
    pub left: Vec<TileSpec>,
    pub right: Vec<TileSpec>,
    /// Width of a side column in % of the screen's long side.
    pub side_width: u32,
    pub recursive: bool,
    /// Seconds between two looks at the folder (new / removed pictures).
    pub refresh: u32,
}

pub const DEFAULT_LEFT: &str = "clock, ram:ring";
pub const DEFAULT_RIGHT: &str = "cpu:ring, gpu:ring";

fn flat_tiles(raw: &str) -> anyhow::Result<Vec<TileSpec>> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("off") || t.eq_ignore_ascii_case("nessuno") {
        return Ok(Vec::new());
    }
    Ok(tiles::parse_tiles(t)?.into_iter().flatten().map(|t| TileSpec { tall: false, ..t }).collect())
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            folder: None,
            style: Style::Mosaic,
            photos: 5,
            interval: 8,
            order: Order::Shuffle,
            transition: Transition::Fade,
            gap: 10,
            radius: 14,
            bg: Bg::Blur,
            dim: 60,
            left: flat_tiles(DEFAULT_LEFT).expect("default tiles"),
            right: flat_tiles(DEFAULT_RIGHT).expect("default tiles"),
            side_width: 17,
            recursive: false,
            refresh: 60,
        }
    }
}

impl Style {
    pub const NAMES: &'static str = "mosaic | polaroid";
}
impl Bg {
    pub const NAMES: &'static str = "blur | dark | global";
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get("frame_folder") {
        let v = v.trim().trim_matches('"');
        s.folder = (!v.is_empty()).then(|| PathBuf::from(v));
    }
    if let Some(v) = file.get("frame_style") {
        s.style = match v.trim().to_ascii_lowercase().as_str() {
            "mosaic" | "mosaico" | "collage" | "tidy" => Style::Mosaic,
            "polaroid" | "prints" | "scattered" => Style::Polaroid,
            _ => anyhow::bail!("frame_style: '{v}' not valid ({})", Style::NAMES),
        };
    }
    if let Some(v) = file.get("frame_order") {
        s.order = match v.trim().to_ascii_lowercase().as_str() {
            "shuffle" | "random" | "casuale" => Order::Shuffle,
            "name" | "alphabetical" | "nome" | "sorted" => Order::Name,
            _ => anyhow::bail!("frame_order: '{v}' not valid (shuffle | name)"),
        };
    }
    if let Some(v) = file.get("frame_transition") {
        s.transition = match v.trim().to_ascii_lowercase().as_str() {
            "fade" | "dissolve" | "dissolvenza" => Transition::Fade,
            "slide" | "wipe" | "drop" | "scorri" => Transition::Slide,
            _ => anyhow::bail!("frame_transition: '{v}' not valid (fade | slide)"),
        };
    }
    if let Some(v) = file.get("frame_bg") {
        s.bg = match v.trim().to_ascii_lowercase().as_str() {
            "blur" | "blurred" | "sfocato" | "photo" => Bg::Blur,
            "dark" | "black" | "scuro" | "nero" => Bg::Dark,
            "global" | "default" | "auto" => Bg::Global,
            _ => anyhow::bail!("frame_bg: '{v}' not valid ({})", Bg::NAMES),
        };
    }
    let num = |key: &str, lo: u32, hi: u32, def: u32| -> anyhow::Result<u32> {
        match file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            None => Ok(def),
            Some(v) if (lo..=hi).contains(&v) => Ok(v),
            Some(v) => anyhow::bail!("{key}: must be between {lo} and {hi} (got {v})"),
        }
    };
    s.photos = num("frame_photos", 1, 9, s.photos)?;
    s.interval = num("frame_interval", 2, 3600, s.interval)?;
    s.gap = num("frame_gap", 0, 40, s.gap)?;
    s.radius = num("frame_radius", 0, 60, s.radius)?;
    s.dim = num("frame_dim", 0, 100, s.dim)?;
    s.side_width = num("frame_side_width", 8, 35, s.side_width)?;
    s.refresh = num("frame_refresh", 5, 3600, s.refresh)?;
    if let Some(v) = file.get_bool("frame_recursive").map_err(|e| anyhow::anyhow!(e))? {
        s.recursive = v;
    }
    if let Some(v) = file.get("frame_left") {
        s.left = flat_tiles(v).map_err(|e| anyhow::anyhow!("frame_left: {e}"))?;
    }
    if let Some(v) = file.get("frame_right") {
        s.right = flat_tiles(v).map_err(|e| anyhow::anyhow!("frame_right: {e}"))?;
    }
    Ok(s)
}

// ----------------------------------------------------------------- small tools

/// xorshift64*: all the randomness needed (shuffles, jitter), seedable for the tests.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// 0.0 .. 1.0
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ------------------------------------------------------------------ geometry

/// Places of a tidy mosaic: `rect` is split recursively into `n` pieces with `gap` pixels
/// between them. The split ratios are slightly uneven (and fixed by `n`) so the pieces differ
/// in size and shape, like a hand-made collage, yet nothing jumps around between runs.
pub fn mosaic_slots(rect: Rect, n: usize, gap: u32) -> Vec<Rect> {
    let mut rng = Rng::new(0xC011A6E + n as u64 * 7919);
    let mut out = Vec::new();
    let (x, y, w, h) = rect;
    split_rec((x as f32, y as f32, w as f32, h as f32), n.max(1), gap as f32, &mut rng, &mut out);
    out.into_iter()
        .map(|(x, y, w, h)| (x.round() as u32, y.round() as u32, (w.round() as u32).max(1), (h.round() as u32).max(1)))
        .collect()
}

fn split_rec(r: (f32, f32, f32, f32), n: usize, gap: f32, rng: &mut Rng, out: &mut Vec<(f32, f32, f32, f32)>) {
    let (x, y, w, h) = r;
    if n <= 1 {
        out.push(r);
        return;
    }
    let mut n1 = n / 2;
    if n % 2 == 1 && rng.next() % 2 == 0 {
        n1 += 1;
    }
    let n2 = n - n1;
    // the bigger group gets the bigger share, with some variation
    let ratio = (n1 as f32 / n as f32 * (0.88 + rng.f() * 0.24)).clamp(0.22, 0.78);
    if w >= h * 1.05 {
        let w1 = ((w - gap) * ratio).max(1.0);
        split_rec((x, y, w1, h), n1, gap, rng, out);
        split_rec((x + w1 + gap, y, (w - w1 - gap).max(1.0), h), n2, gap, rng, out);
    } else {
        let h1 = ((h - gap) * ratio).max(1.0);
        split_rec((x, y, w, h1), n1, gap, rng, out);
        split_rec((x, y + h1 + gap, w, (h - h1 - gap).max(1.0)), n2, gap, rng, out);
    }
}

/// A scattered print: centre, card width (the height follows) and tilt in degrees.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Card {
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub angle: f32,
}

/// Height of a print over its width (photo + the wider white strip below).
const CARD_RATIO: f32 = 1.16;

/// Positions of `n` prints spread over `rect`: along its long side, with a little vertical
/// jitter and alternating tilts, overlapping a bit when there are many.
pub fn polaroid_cards(rect: Rect, n: usize) -> Vec<Card> {
    // a little room for the tilted corners, the shadow and the tape
    let m = (rect.2.min(rect.3) as f32 * 0.035).round() + 3.0;
    let (x, y, w, h) = (rect.0 as f32 + m, rect.1 as f32 + m, (rect.2 as f32 - 2.0 * m).max(1.0), (rect.3 as f32 - 2.0 * m).max(1.0));
    let n = n.max(1);
    let wide = w >= h;
    let mut rng = Rng::new(0xB01A_201D + n as u64 * 104_729);
    // size: the print fits the short side, and n of them share the long side with some overlap
    let by_short = if wide { (h * 0.8) / CARD_RATIO } else { w * 0.78 };
    let by_long = if wide { w / n as f32 * 1.35 } else { h / n as f32 * 1.35 / CARD_RATIO };
    let cw = by_short.min(by_long).max(40.0);
    let ch = cw * CARD_RATIO;
    let mut out = Vec::new();
    for i in 0..n {
        let t = if n == 1 { 0.5 } else { i as f32 / (n - 1) as f32 };
        let jitter = (rng.f() - 0.5) * 0.5;
        let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
        let angle = sign * (3.0 + rng.f() * 6.0);
        let (cx, cy) = if wide {
            let span = (w - cw).max(0.0);
            (x + cw / 2.0 + span * t, y + h / 2.0 + (h - ch).max(0.0) * 0.5 * jitter * 0.9)
        } else {
            let span = (h - ch).max(0.0);
            (x + w / 2.0 + (w - cw).max(0.0) * 0.5 * jitter * 0.9, y + ch / 2.0 + span * t)
        };
        out.push(Card { cx, cy, cw, angle });
    }
    out
}

// ------------------------------------------------------------------- sprites

/// A finished picture, premultiplied RGBA, ready to be laid over the scene.
pub struct Sprite {
    pub w: u32,
    pub h: u32,
    /// Where the top-left corner sits, relative to the place's anchor.
    pub ax: i32,
    pub ay: i32,
    px: Vec<u8>,
}

/// What the loader thread must make of a picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spec {
    Mosaic { w: u32, h: u32, radius: u32 },
    Polaroid { cw: u32, angle: f32, tape: bool },
}

/// Opens a picture and applies its EXIF orientation. `None` = not a picture we can read.
pub fn decode(path: &Path) -> Option<RgbImage> {
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut dec = reader.into_decoder().ok()?;
    let orientation = dec.orientation().ok();
    let mut img = DynamicImage::from_decoder(dec).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Some(img.to_rgb8())
}

/// Scales and crops `img` to exactly `w` x `h` (what fills the box; the crop leans a little
/// towards the top, where faces usually are).
pub fn cover(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    let (iw, ih) = img.dimensions();
    let (w, h) = (w.max(1), h.max(1));
    let scale = (w as f32 / iw as f32).max(h as f32 / ih as f32);
    let cw = ((w as f32 / scale).round() as u32).clamp(1, iw);
    let ch = ((h as f32 / scale).round() as u32).clamp(1, ih);
    let cx = (iw - cw) / 2;
    let cy = ((ih - ch) as f32 * 0.4) as u32;
    let crop = imageops::crop_imm(img, cx, cy, cw, ch).to_image();
    // a big reduction is done in two steps: the cheap box filter, then a clean final pass
    let crop = if cw > w * 3 && ch > h * 3 { imageops::thumbnail(&crop, w * 2, h * 2) } else { crop };
    imageops::resize(&crop, w, h, imageops::FilterType::Triangle)
}

/// Coverage (0-1) of pixel (px, py) by a rounded rectangle: 1 inside, 0 outside, 1 px ramp.
fn rrect_cov(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (cx, cy) = (px - w / 2.0, py - h / 2.0);
    let (qx, qy) = (cx.abs() - (w / 2.0 - r), cy.abs() - (h / 2.0 - r));
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

fn premul(rgb: [u8; 3], a: f32) -> [u8; 4] {
    let a = a.clamp(0.0, 1.0);
    [(rgb[0] as f32 * a).round() as u8, (rgb[1] as f32 * a).round() as u8, (rgb[2] as f32 * a).round() as u8, (a * 255.0).round() as u8]
}

/// A photo of the mosaic: cropped to the place, rounded corners, a hair-line of light on the edge.
pub fn mosaic_sprite(img: &RgbImage, w: u32, h: u32, radius: u32) -> Sprite {
    let photo = cover(img, w, h);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let a = rrect_cov(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32, radius as f32);
            if a <= 0.0 {
                continue;
            }
            let p = photo.get_pixel(x, y).0;
            // 1 px rim: the edge catches a little light
            let inner = rrect_cov(x as f32 + 0.5, y as f32 + 0.5, w as f32 - 2.0, h as f32 - 2.0, radius as f32 - 1.0);
            let rim = if inner < 1.0 && x > 0 && y > 0 && x + 1 < w && y + 1 < h { (1.0 - inner) * 0.16 } else { 0.0 };
            let c = [0, 1, 2].map(|k| (p[k] as f32 + (255.0 - p[k] as f32) * rim) as u8);
            let o = ((y * w + x) * 4) as usize;
            px[o..o + 4].copy_from_slice(&premul(c, a));
        }
    }
    Sprite { w, h, ax: 0, ay: 0, px }
}

/// An RGBA canvas the card is drawn on before it is rotated (not premultiplied).
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        Canvas { w, h, px: vec![[0.0; 4]; (w * h) as usize] }
    }
    fn over(&mut self, x: u32, y: u32, rgb: [f32; 3], a: f32) {
        if x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let p = &mut self.px[(y * self.w + x) as usize];
        let out_a = a + p[3] * (1.0 - a);
        for k in 0..3 {
            p[k] = if out_a > 0.0 { (rgb[k] * a + p[k] * p[3] * (1.0 - a)) / out_a } else { 0.0 };
        }
        p[3] = out_a;
    }
    /// Bilinear sample at (x, y) in canvas pixels, as premultiplied.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |xi: i32, yi: i32| -> [f32; 4] {
            if xi < 0 || yi < 0 || xi >= self.w as i32 || yi >= self.h as i32 {
                [0.0; 4]
            } else {
                let p = self.px[(yi as u32 * self.w + xi as u32) as usize];
                [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]
            }
        };
        let (a, b, c, d) = (at(x0 as i32, y0 as i32), at(x0 as i32 + 1, y0 as i32), at(x0 as i32, y0 as i32 + 1), at(x0 as i32 + 1, y0 as i32 + 1));
        let mut o = [0.0; 4];
        for k in 0..4 {
            o[k] = lerp(lerp(a[k], b[k], fx), lerp(c[k], d[k], fx), fy);
        }
        o
    }
}

/// A print: white paper with the photo in it (square), the wide strip at the bottom, a soft
/// shadow, rotated by `angle` degrees. `tape` adds a translucent strip over the top edge.
pub fn polaroid_sprite(img: &RgbImage, cw: u32, angle: f32, tape: bool) -> Sprite {
    let cw = cw.max(40);
    let ch = (cw as f32 * CARD_RATIO).round() as u32;
    let border = (cw as f32 * 0.06).round().max(2.0) as u32;
    let top_margin = (cw as f32 * 0.07).round() as u32; // room for the tape
    let (canvas_w, canvas_h) = (cw, ch + top_margin);
    let mut cv = Canvas::new(canvas_w, canvas_h);
    // paper: warm white with the faintest diagonal shading
    for y in 0..ch {
        for x in 0..cw {
            let a = rrect_cov(x as f32 + 0.5, y as f32 + 0.5, cw as f32, ch as f32, 3.0);
            let shade = 1.0 - 0.035 * ((x as f32 / cw as f32) * 0.6 + (y as f32 / ch as f32) * 0.4);
            cv.over(x, y + top_margin, [247.0 * shade, 245.0 * shade, 240.0 * shade], a);
        }
    }
    // photo
    let pw = cw - 2 * border;
    let photo = cover(img, pw, pw);
    for y in 0..pw {
        for x in 0..pw {
            let p = photo.get_pixel(x, y).0;
            // the photo is slightly vignetted into the paper: a 1 px darker edge
            let edge = if x == 0 || y == 0 || x + 1 == pw || y + 1 == pw { 0.82 } else { 1.0 };
            cv.over(x + border, y + border + top_margin, [p[0] as f32 * edge, p[1] as f32 * edge, p[2] as f32 * edge], 1.0);
        }
    }
    // tape: a pale, translucent strip across the top edge, tilted the other way
    if tape {
        let (tw, th) = (cw as f32 * 0.34, cw as f32 * 0.10);
        let (tcx, tcy) = (cw as f32 * 0.5, top_margin as f32 + 1.0);
        let ta = (-angle.signum() * 7.0 + 2.0).to_radians();
        let (s, c) = ta.sin_cos();
        let r = (tw * tw + th * th).sqrt() / 2.0 + 1.0;
        for y in (tcy - r).floor().max(0.0) as u32..((tcy + r).ceil() as u32).min(canvas_h) {
            for x in (tcx - r).floor().max(0.0) as u32..((tcx + r).ceil() as u32).min(canvas_w) {
                let (dx, dy) = (x as f32 + 0.5 - tcx, y as f32 + 0.5 - tcy);
                let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
                let a = ((tw / 2.0 - u.abs()) + 0.5).clamp(0.0, 1.0) * ((th / 2.0 - v.abs()) + 0.5).clamp(0.0, 1.0);
                cv.over(x, y, [236.0, 222.0, 178.0], a * 0.62);
            }
        }
    }
    rotate_with_shadow(&cv, angle)
}

/// Rotates the canvas about its centre (bilinear) onto a canvas big enough for the corners
/// and a soft shadow, and returns it premultiplied.
fn rotate_with_shadow(cv: &Canvas, angle: f32) -> Sprite {
    let (cw, ch) = (cv.w as f32, cv.h as f32);
    let (s, c) = angle.to_radians().sin_cos();
    let bw = cw * c.abs() + ch * s.abs();
    let bh = cw * s.abs() + ch * c.abs();
    let blur = (cw * 0.035).round().max(3.0) as i32;
    let (off_x, off_y) = (blur as f32 * 0.35, blur as f32 * 0.8);
    let pad = blur * 2 + 2;
    let (ow, oh) = ((bw.ceil() as i32 + pad * 2) as u32, (bh.ceil() as i32 + pad * 2) as u32);
    let (ocx, ocy) = (ow as f32 / 2.0, oh as f32 / 2.0);
    // rotated card, premultiplied
    let mut card = vec![[0.0f32; 4]; (ow * oh) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let (dx, dy) = (x as f32 + 0.5 - ocx, y as f32 + 0.5 - ocy);
            // inverse rotation into canvas coordinates
            let (u, v) = (dx * c + dy * s + cw / 2.0, -dx * s + dy * c + ch / 2.0);
            if u < -1.0 || v < -1.0 || u > cw + 1.0 || v > ch + 1.0 {
                continue;
            }
            card[(y * ow + x) as usize] = cv.sample(u, v);
        }
    }
    // shadow: the alpha, offset and blurred (two box passes ~ a gaussian)
    let mut sh: Vec<f32> = vec![0.0; (ow * oh) as usize];
    for y in 0..oh as i32 {
        for x in 0..ow as i32 {
            let (sx, sy) = (x - off_x.round() as i32, y - off_y.round() as i32);
            if sx >= 0 && sy >= 0 && sx < ow as i32 && sy < oh as i32 {
                sh[(y as u32 * ow + x as u32) as usize] = card[(sy as u32 * ow + sx as u32) as usize][3];
            }
        }
    }
    for _ in 0..2 {
        box_blur(&mut sh, ow as usize, oh as usize, (blur / 2).max(1) as usize);
    }
    let mut px = vec![0u8; (ow * oh * 4) as usize];
    for i in 0..(ow * oh) as usize {
        let p = card[i];
        let sa = sh[i] * 0.55;
        // card over (black shadow): premultiplied source-over
        let a = p[3] + sa * (1.0 - p[3]);
        let rgb = [p[0], p[1], p[2]].map(|v| v.clamp(0.0, 255.0));
        px[i * 4] = rgb[0].round() as u8;
        px[i * 4 + 1] = rgb[1].round() as u8;
        px[i * 4 + 2] = rgb[2].round() as u8;
        px[i * 4 + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    Sprite { w: ow, h: oh, ax: -(ow as i32) / 2, ay: -(oh as i32) / 2, px }
}

/// Separable box blur of radius `r` (edges read as 0).
fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize) {
    let n = (2 * r + 1) as f32;
    let mut tmp = vec![0.0f32; v.len()];
    for y in 0..h {
        let row = &v[y * w..(y + 1) * w];
        let mut acc: f32 = row.iter().take(r + 1).sum();
        for x in 0..w {
            tmp[y * w + x] = acc / n;
            if x + r + 1 < w {
                acc += row[x + r + 1];
            }
            if x >= r {
                acc -= row[x - r];
            }
        }
    }
    for x in 0..w {
        let mut acc: f32 = (0..=r.min(h - 1)).map(|y| tmp[y * w + x]).sum();
        for y in 0..h {
            v[y * w + x] = acc / n;
            if y + r + 1 < h {
                acc += tmp[(y + r + 1) * w + x];
            }
            if y >= r {
                acc -= tmp[(y - r) * w + x];
            }
        }
    }
}

/// Lays `sp` over `fb` with its top-left at (ox, oy). `g` (0-1) is an overall opacity;
/// `wipe` (0-1) reveals it from the left with a soft edge.
pub fn blend_sprite(fb: &mut Framebuffer, sp: &Sprite, ox: i32, oy: i32, g: f32, wipe: Option<f32>) {
    let (fw, fh) = (fb.width() as i32, fb.height() as i32);
    let (x0, x1) = (ox.max(0), (ox + sp.w as i32).min(fw));
    let (y0, y1) = (oy.max(0), (oy + sp.h as i32).min(fh));
    if x0 >= x1 || y0 >= y1 || g <= 0.0 {
        return;
    }
    let soft = (sp.w as f32 * 0.3).max(1.0);
    let gcol: Vec<u32> = (x0..x1)
        .map(|x| {
            let col = (x - ox) as f32;
            let k = match wipe {
                None => 1.0,
                Some(p) => smooth((p * (sp.w as f32 + soft) - col) / soft),
            };
            (g * k * 256.0).round().clamp(0.0, 256.0) as u32
        })
        .collect();
    let stride = fw as usize * 3;
    let bytes = fb.as_bytes_mut();
    for y in y0..y1 {
        let src_row = ((y - oy) as usize * sp.w as usize) * 4;
        let dst_row = y as usize * stride;
        for x in x0..x1 {
            let gi = gcol[(x - x0) as usize];
            if gi == 0 {
                continue;
            }
            let s = src_row + (x - ox) as usize * 4;
            let a = sp.px[s + 3] as u32;
            if a == 0 {
                continue;
            }
            let sa = (a * gi) >> 8;
            let d = dst_row + x as usize * 3;
            for k in 0..3 {
                let sc = (sp.px[s + k] as u32 * gi) >> 8;
                let v = (bytes[d + k] as u32 * (255 - sa) + 127) / 255 + sc;
                bytes[d + k] = v.min(255) as u8;
            }
        }
    }
}

// ------------------------------------------------------------- the loader thread

/// A picture of the folder, as the scan saw it (size after the EXIF rotation).
#[derive(Clone, Debug, PartialEq)]
pub struct Photo {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
}

const EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "bmp", "gif", "jfif"];
const MAX_PHOTOS: usize = 5000;

fn is_picture(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with('.') {
        return false;
    }
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn collect_files(dir: &Path, recursive: bool, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if out.len() >= MAX_PHOTOS {
            return;
        }
        let p = e.path();
        if p.is_dir() {
            if recursive && depth < 6 {
                collect_files(&p, true, depth + 1, out);
            }
        } else if is_picture(&p) {
            out.push(p);
        }
    }
}

/// The pictures of `dir` (sizes from the headers only), sorted by name.
pub fn scan_folder(dir: &Path, recursive: bool) -> Vec<Photo> {
    let mut files = Vec::new();
    collect_files(dir, recursive, 0, &mut files);
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    files
        .into_iter()
        .filter_map(|path| {
            let mut dec = image::ImageReader::open(&path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
            let (w, h) = dec.dimensions();
            let swap = dec.orientation().ok().is_some_and(|o| {
                use image::metadata::Orientation::*;
                matches!(o, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)
            });
            let (w, h) = if swap { (h, w) } else { (w, h) };
            (w > 0 && h > 0).then_some(Photo { path, w, h })
        })
        .collect()
}

enum Request {
    Scan { folder: PathBuf, recursive: bool },
    Build { slot: usize, gen: u64, path: PathBuf, spec: Spec },
}

enum Reply {
    Scanned { folder: PathBuf, photos: Vec<Photo> },
    Built { slot: usize, gen: u64, path: PathBuf, sprite: Sprite, thumb: Thumb },
    Failed { slot: usize, gen: u64, path: PathBuf },
}

/// A tiny copy of a photo, enough to paint the blurred background.
pub struct Thumb {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

fn worker(rx: Receiver<Request>, tx: Sender<Reply>) {
    while let Ok(req) = rx.recv() {
        let reply = match req {
            Request::Scan { folder, recursive } => Reply::Scanned { photos: scan_folder(&folder, recursive), folder },
            Request::Build { slot, gen, path, spec } => match decode(&path) {
                None => Reply::Failed { slot, gen, path },
                Some(img) => {
                    let sprite = match spec {
                        Spec::Mosaic { w, h, radius } => mosaic_sprite(&img, w, h, radius),
                        Spec::Polaroid { cw, angle, tape } => polaroid_sprite(&img, cw, angle, tape),
                    };
                    let (iw, ih) = img.dimensions();
                    let k = 96.0 / iw.max(ih) as f32;
                    let t = imageops::thumbnail(&img, ((iw as f32 * k).round() as u32).max(1), ((ih as f32 * k).round() as u32).max(1));
                    let thumb = Thumb { w: t.width(), h: t.height(), px: t.into_raw() };
                    Reply::Built { slot, gen, path, sprite, thumb }
                }
            },
        };
        if tx.send(reply).is_err() {
            return;
        }
    }
}

// -------------------------------------------------------------------- the screen

/// Seconds a swap takes.
const TRANSITION_SECS: f64 = 1.4;
/// Frame rate while something moves / while nothing does.
pub const BUSY_FPS: f32 = 20.0;
pub const IDLE_FPS: f32 = 2.0;

struct Placed {
    sprite: Sprite,
    /// Top-left on the screen.
    x: i32,
    y: i32,
}

struct Slot {
    rect: Rect,
    card: Option<Card>,
    cur: Option<Placed>,
    old: Option<Placed>,
    /// When `cur` appeared (the transition starts then).
    born: f64,
    z: u32,
    /// The picture shown here (or being loaded for it).
    photo: Option<PathBuf>,
    pending: bool,
}

pub struct FrameScreen {
    started: std::time::Instant,
    left: TilesScreen,
    right: TilesScreen,
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    photos: Vec<Photo>,
    folder_key: String,
    scan_pending: bool,
    scanned: bool,
    last_scan: f64,
    slots: Vec<Slot>,
    geom_key: String,
    gen: u64,
    rng: Rng,
    bag: Vec<usize>,
    cursor: usize,
    last_swap: f64,
    last_slot: usize,
    z_next: u32,
    bg_cur: Option<Framebuffer>,
    bg_old: Option<Framebuffer>,
    bg_born: f64,
    scene: Option<Framebuffer>,
    scene_valid: bool,
    now: f64,
    /// Previews: no transitions, no folder re-reads, the first picture on screen is final.
    pub still: bool,
    /// Number of times the collage was composed (tests, tuning).
    pub composes: u32,
}

fn seed_from_clock() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

impl FrameScreen {
    pub fn new() -> Self {
        Self::with_seed(seed_from_clock())
    }

    pub fn with_seed(seed: u64) -> Self {
        let (tx, req_rx) = channel();
        let (rep_tx, rx) = channel();
        std::thread::Builder::new().name("frame-loader".into()).spawn(move || worker(req_rx, rep_tx)).ok();
        FrameScreen {
            started: std::time::Instant::now(),
            left: TilesScreen::new(),
            right: TilesScreen::new(),
            tx,
            rx,
            photos: Vec::new(),
            folder_key: String::new(),
            scan_pending: false,
            scanned: false,
            last_scan: 0.0,
            slots: Vec::new(),
            geom_key: String::new(),
            gen: 0,
            rng: Rng::new(seed),
            bag: Vec::new(),
            cursor: 0,
            last_swap: 0.0,
            last_slot: usize::MAX,
            z_next: 100,
            bg_cur: None,
            bg_old: None,
            bg_born: 0.0,
            scene: None,
            scene_valid: false,
            now: 0.0,
            still: false,
            composes: 0,
        }
    }

    /// True while pictures are being found or loaded (a preview waits for that).
    pub fn loading(&self) -> bool {
        self.scan_pending || self.slots.iter().any(|s| s.pending)
    }

    /// True while something moves: the main loop then wants a smooth frame rate.
    pub fn busy(&self) -> bool {
        self.loading() || self.moving()
    }

    /// A photo is fading / sliding in, or the background is changing.
    fn moving(&self) -> bool {
        self.bg_old.is_some() || self.slots.iter().any(|sl| sl.old.is_some() || (sl.cur.is_some() && self.now - sl.born < TRANSITION_SECS))
    }

    /// The frame rate the screen asks for.
    pub fn wanted_fps(&self) -> f32 {
        if self.busy() {
            BUSY_FPS
        } else {
            IDLE_FPS
        }
    }

    /// Where the side columns and the collage go inside `area`.
    pub fn rects(s: &Settings, area: Rect, gap: u32) -> (Option<Rect>, Option<Rect>, Rect) {
        let (ax, ay, aw, ah) = area;
        let long = aw.max(ah);
        let ss = (long as f32 * s.side_width as f32 / 100.0).round() as u32;
        let (has_l, has_r) = (!s.left.is_empty(), !s.right.is_empty());
        let sides = has_l as u32 + has_r as u32;
        if aw >= ah {
            let ss = ss.min(aw / 3);
            let left = has_l.then_some((ax, ay, ss, ah));
            let right = has_r.then_some((ax + aw - ss, ay, ss, ah));
            let x0 = ax + if has_l { ss } else { gap };
            let w = (aw - ss * sides).saturating_sub(if has_l { 0 } else { gap } + if has_r { 0 } else { gap });
            (left, right, (x0, ay + gap, w, ah.saturating_sub(gap * 2)))
        } else {
            let ss = ss.min(ah / 3);
            let top = has_l.then_some((ax, ay, aw, ss));
            let bottom = has_r.then_some((ax, ay + ah - ss, aw, ss));
            let y0 = ay + if has_l { ss } else { gap };
            let h = (ah - ss * sides).saturating_sub(if has_l { 0 } else { gap } + if has_r { 0 } else { gap });
            (top, bottom, (ax + gap, y0, aw.saturating_sub(gap * 2), h))
        }
    }

    fn effective_count(&self, s: &Settings) -> usize {
        (s.photos as usize).min(self.photos.len())
    }

    // ---------------------------------------------------------- bookkeeping

    fn request_scan(&mut self, s: &Settings) {
        if let Some(folder) = &s.folder {
            if self.tx.send(Request::Scan { folder: folder.clone(), recursive: s.recursive }).is_ok() {
                self.scan_pending = true;
            }
        }
        self.last_scan = self.now;
    }

    fn born_now(&self) -> f64 {
        if self.still {
            self.now - 1.0e6
        } else {
            self.now
        }
    }

    /// Takes in what the loader has finished.
    fn pump(&mut self, s: &Settings) {
        while let Ok(reply) = self.rx.try_recv() {
            match reply {
                Reply::Scanned { folder, photos } => {
                    if s.folder.as_ref() != Some(&folder) {
                        continue; // an answer for a folder that is no longer the one
                    }
                    self.scan_pending = false;
                    self.scanned = true;
                    if photos != self.photos {
                        self.photos = photos;
                        self.bag.clear();
                    }
                }
                Reply::Built { slot, gen, path, sprite, thumb } => {
                    if gen != self.gen || slot >= self.slots.len() {
                        continue;
                    }
                    let born = self.born_now();
                    let z = self.z_next;
                    let style = s.style;
                    let sl = &mut self.slots[slot];
                    sl.pending = false;
                    sl.photo = Some(path);
                    let (anchor_x, anchor_y) = match sl.card {
                        Some(c) if style == Style::Polaroid => (c.cx.round() as i32, c.cy.round() as i32),
                        _ => (sl.rect.0 as i32, sl.rect.1 as i32),
                    };
                    let placed = Placed { x: anchor_x + sprite.ax, y: anchor_y + sprite.ay, sprite };
                    sl.old = sl.cur.take();
                    sl.cur = Some(placed);
                    sl.born = born;
                    if style == Style::Polaroid {
                        sl.z = z;
                        self.z_next += 1;
                    }
                    self.scene_valid = false;
                    if s.bg == Bg::Blur {
                        self.set_hero(&thumb, s.dim);
                    }
                }
                Reply::Failed { slot, gen, path } => {
                    if gen != self.gen || slot >= self.slots.len() {
                        continue;
                    }
                    self.photos.retain(|p| p.path != path);
                    self.bag.clear();
                    let sl = &mut self.slots[slot];
                    sl.pending = false;
                    sl.photo = None;
                    self.fill_slot(s, slot);
                }
            }
        }
    }

    /// Picks the next picture for a place of aspect `aspect` (w / h), not already on screen.
    fn pick(&mut self, s: &Settings, aspect: f32) -> Option<usize> {
        let shown: HashSet<&PathBuf> = self.slots.iter().filter_map(|sl| sl.photo.as_ref()).collect();
        let len = self.photos.len();
        let free: Vec<bool> = self.photos.iter().map(|p| !shown.contains(&p.path)).collect();
        if !free.iter().any(|f| *f) {
            return None;
        }
        match s.order {
            Order::Name => {
                let start = self.cursor % len;
                let i = (0..len).map(|k| (start + k) % len).find(|i| free[*i])?;
                self.cursor = i + 1;
                Some(i)
            }
            Order::Shuffle => {
                self.bag.retain(|i| *i < len && free[*i]);
                if self.bag.is_empty() {
                    let mut all: Vec<usize> = (0..len).filter(|i| free[*i]).collect();
                    for i in (1..all.len()).rev() {
                        let j = self.rng.below(i + 1);
                        all.swap(i, j);
                    }
                    self.bag = all;
                }
                // of the next few, the one whose shape fits the place best
                let best = (0..self.bag.len().min(5))
                    .min_by(|&a, &b| {
                        let cost = |k: usize| {
                            let p = &self.photos[self.bag[k]];
                            (p.w as f32 / p.h as f32 / aspect).ln().abs()
                        };
                        cost(a).partial_cmp(&cost(b)).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(0);
                Some(self.bag.remove(best))
            }
        }
    }

    /// Asks the loader for a new picture for `slot`.
    fn fill_slot(&mut self, s: &Settings, slot: usize) {
        let aspect = match (s.style, self.slots.get(slot)) {
            (Style::Mosaic, Some(sl)) => sl.rect.2 as f32 / sl.rect.3 as f32,
            _ => 1.0,
        };
        let Some(i) = self.pick(s, aspect) else { return };
        let path = self.photos[i].path.clone();
        let spec = match (s.style, self.slots[slot].card) {
            (Style::Polaroid, Some(c)) => {
                let sign = if self.rng.next() % 2 == 0 { -1.0 } else { 1.0 };
                Spec::Polaroid { cw: c.cw.round() as u32, angle: sign * (3.0 + self.rng.f() * 6.0), tape: self.rng.next() % 2 == 0 }
            }
            _ => {
                let r = self.slots[slot].rect;
                Spec::Mosaic { w: r.2, h: r.3, radius: s.radius }
            }
        };
        let sl = &mut self.slots[slot];
        sl.photo = Some(path.clone());
        sl.pending = true;
        let _ = self.tx.send(Request::Build { slot, gen: self.gen, path, spec });
    }

    /// New places for the photos: when the area, the style or the count changed.
    fn rebuild(&mut self, s: &Settings, collage: Rect) {
        let n = self.effective_count(s);
        self.gen += 1;
        self.slots.clear();
        self.bag.clear();
        self.scene_valid = false;
        if n == 0 || collage.2 < 60 || collage.3 < 60 {
            return;
        }
        match s.style {
            Style::Mosaic => {
                for (i, rect) in mosaic_slots(collage, n, s.gap).into_iter().enumerate() {
                    self.slots.push(Slot { rect, card: None, cur: None, old: None, born: 0.0, z: i as u32, photo: None, pending: false });
                }
            }
            Style::Polaroid => {
                let mut cards = polaroid_cards(collage, n);
                // a random stacking order, so no print is always on top
                let mut z: Vec<u32> = (0..n as u32).collect();
                for i in (1..z.len()).rev() {
                    let j = self.rng.below(i + 1);
                    z.swap(i, j);
                }
                for (i, c) in cards.drain(..).enumerate() {
                    let r = (c.cx as u32, c.cy as u32, c.cw as u32, (c.cw * CARD_RATIO) as u32);
                    self.slots.push(Slot { rect: r, card: Some(c), cur: None, old: None, born: 0.0, z: z[i], photo: None, pending: false });
                }
            }
        }
        self.last_swap = self.now;
        self.last_slot = usize::MAX;
        for i in 0..self.slots.len() {
            self.fill_slot(s, i);
        }
    }

    /// The current photo becomes the (blurred, darkened) background.
    fn set_hero(&mut self, t: &Thumb, dim: u32) {
        let (w, h) = match &self.scene {
            Some(f) => (f.width(), f.height()),
            None => return,
        };
        let bg = blurred_background(t, w, h, dim);
        self.bg_old = self.bg_cur.take();
        self.bg_cur = Some(bg);
        self.bg_born = self.born_now();
    }

    /// Everything that happens in time: replies, folder re-reads, the next swap.
    pub fn update(&mut self, fb: &Framebuffer, area: Rect, s: &Settings, tiles_gap: u32, env: &tiles::Env) {
        let now = env.secs.unwrap_or_else(|| self.started.elapsed().as_secs_f64());
        self.now = now;
        if self.scene.as_ref().map(|f| (f.width(), f.height())) != Some((fb.width(), fb.height())) {
            self.scene = Some(Framebuffer::new(trofeo_lcd::Resolution::new(fb.width(), fb.height())));
            self.scene_valid = false;
            self.bg_cur = None;
            self.bg_old = None;
            self.geom_key.clear();
        }
        let folder_key = format!("{:?}|{}", s.folder, s.recursive);
        if folder_key != self.folder_key {
            self.folder_key = folder_key;
            self.photos.clear();
            self.scanned = false;
            self.scan_pending = false;
            self.geom_key.clear();
            self.request_scan(s);
        } else if !self.scan_pending && now - self.last_scan >= s.refresh as f64 && !self.still {
            self.request_scan(s);
        }
        self.pump(s);
        let (_, _, collage) = Self::rects(s, area, tiles_gap);
        let n = self.effective_count(s);
        let key = format!("{collage:?}|{:?}|{n}|{}|{}|{:?}", s.style, s.gap, s.radius, s.bg);
        if key != self.geom_key && (self.scanned || n > 0) {
            self.geom_key = key;
            self.rebuild(s, collage);
        }
        // the next swap
        let idle = !self.slots.iter().any(|sl| sl.pending);
        if idle
            && !self.slots.is_empty()
            && self.photos.len() > self.slots.len()
            && now - self.last_swap >= s.interval as f64
            && self.slots.iter().all(|sl| sl.cur.is_some())
        {
            let n = self.slots.len();
            let slot = match s.order {
                Order::Name => (self.last_slot.wrapping_add(1)) % n,
                Order::Shuffle => {
                    let mut k = self.rng.below(n);
                    if n > 1 && k == self.last_slot {
                        k = (k + 1) % n;
                    }
                    k
                }
            };
            self.last_slot = slot;
            self.last_swap = now;
            self.fill_slot(s, slot);
        }
        // transitions that are over
        let now = self.now;
        for sl in &mut self.slots {
            if sl.old.is_some() && now - sl.born >= TRANSITION_SECS {
                sl.old = None;
                self.scene_valid = false;
            }
        }
        if self.bg_old.is_some() && now - self.bg_born >= TRANSITION_SECS {
            self.bg_old = None;
            self.scene_valid = false;
        }
    }

    /// Draws the whole screen (the collage, then the side tiles) into `fb`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, fb: &mut Framebuffer, area: Rect, data: &crate::layouts::WidgetData, color_mode: ColorMode, s: &Settings, tcfg: &tiles::Settings, env: &tiles::Env) {
        self.update(fb, area, s, tcfg.gap, env);
        self.render(fb, area, data, color_mode, s, tcfg, env);
    }

    /// `draw`, but waits first until the pictures are loaded (previews, tests).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_settled(&mut self, fb: &mut Framebuffer, area: Rect, data: &crate::layouts::WidgetData, color_mode: ColorMode, s: &Settings, tcfg: &tiles::Settings, env: &tiles::Env) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            self.update(fb, area, s, tcfg.gap, env);
            if (!self.loading() && (self.scanned || s.folder.is_none())) || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.update(fb, area, s, tcfg.gap, env);
        self.render(fb, area, data, color_mode, s, tcfg, env);
    }

    #[allow(clippy::too_many_arguments)]
    fn render(&mut self, fb: &mut Framebuffer, area: Rect, data: &crate::layouts::WidgetData, color_mode: ColorMode, s: &Settings, tcfg: &tiles::Settings, env: &tiles::Env) {
        let (left, right, collage) = Self::rects(s, area, tcfg.gap);
        let moving = self.moving();
        let cached = s.bg != Bg::Global && !moving && self.scene_valid;
        if cached {
            if let Some(sc) = &self.scene {
                fb.copy_from(sc);
            }
        } else {
            self.compose(fb, s);
            self.composes += 1;
            if s.bg != Bg::Global && !moving {
                if let Some(sc) = &mut self.scene {
                    sc.copy_from(fb);
                    self.scene_valid = true;
                }
            }
        }
        if self.slots.is_empty() {
            self.placeholder(fb, s, collage);
        }
        // side columns: the glass tiles, in the look of the tiles layout
        let side_env = tiles::Env { fps: env.fps, ffmpeg: env.ffmpeg, effect: env.effect, secs: Some(self.now) };
        for (screen, list, rect) in [(&mut self.left, &s.left, left), (&mut self.right, &s.right, right)] {
            if let Some(rect) = rect {
                let mut ts = tcfg.clone();
                ts.rows = vec![list.clone()];
                ts.bg = tiles::BgMode::Global;
                screen.draw(fb, rect, data, color_mode, &ts, &side_env);
            }
        }
    }

    /// Base + every photo, with the transitions of the moment.
    fn compose(&self, fb: &mut Framebuffer, s: &Settings) {
        match s.bg {
            Bg::Global => {}
            Bg::Dark => fb.clear(10, 10, 16),
            Bg::Blur => match (&self.bg_old, &self.bg_cur) {
                (Some(old), Some(cur)) => {
                    let p = smooth(((self.now - self.bg_born) / TRANSITION_SECS) as f32);
                    mix_into(fb, old, cur, p);
                }
                (_, Some(cur)) => fb.copy_from(cur),
                _ => fb.clear(10, 10, 16),
            },
        }
        let mut order: Vec<&Slot> = self.slots.iter().collect();
        order.sort_by_key(|sl| sl.z);
        for sl in order {
            let p = ((self.now - sl.born) / TRANSITION_SECS).clamp(0.0, 1.0) as f32;
            let e = smooth(p);
            let in_transition = sl.old.is_some() && p < 1.0;
            let polaroid = s.style == Style::Polaroid;
            if let (true, Some(old)) = (in_transition, &sl.old) {
                let g = match (polaroid, s.transition) {
                    (false, _) => 1.0,
                    (true, Transition::Fade) => 1.0 - e,
                    (true, Transition::Slide) => 1.0 - (e * 2.0).min(1.0),
                };
                blend_sprite(fb, &old.sprite, old.x, old.y, g, None);
            }
            if let Some(cur) = &sl.cur {
                if !in_transition && sl.old.is_none() && p >= 1.0 {
                    blend_sprite(fb, &cur.sprite, cur.x, cur.y, 1.0, None);
                    continue;
                }
                match (polaroid, s.transition) {
                    (false, Transition::Fade) => blend_sprite(fb, &cur.sprite, cur.x, cur.y, e, None),
                    (false, Transition::Slide) => blend_sprite(fb, &cur.sprite, cur.x, cur.y, 1.0, Some(e)),
                    (true, Transition::Fade) => blend_sprite(fb, &cur.sprite, cur.x, cur.y, e, None),
                    (true, Transition::Slide) => {
                        // the new print drops in from above and settles
                        let drop = (1.0 - e).powi(2) * cur.sprite.h as f32 * 0.22;
                        blend_sprite(fb, &cur.sprite, cur.x, cur.y - drop.round() as i32, (e * 2.0).min(1.0), None)
                    }
                }
            }
        }
    }

    fn placeholder(&self, fb: &mut Framebuffer, s: &Settings, r: Rect) {
        if r.2 < 100 || r.3 < 60 {
            return;
        }
        let (title, hint) = match (&s.folder, self.scanned) {
            (None, _) => ("Digital frame".to_string(), "Set frame_folder = <folder with pictures>".to_string()),
            (Some(f), true) => ("No pictures found".to_string(), format!("in {}", f.display())),
            (Some(_), false) => ("Digital frame".to_string(), "Loading the pictures...".to_string()),
        };
        let scale = (r.3 / 90).clamp(2, 5);
        let hint_scale = (scale / 2).max(1);
        let hint: String = hint.chars().take(((r.2 / ((trofeo_lcd::font::GLYPH_WIDTH + 1) * hint_scale)) as usize).saturating_sub(2)).collect();
        let tw = Framebuffer::text_width(&title, scale);
        let hw = Framebuffer::text_width(&hint, hint_scale);
        let y = r.1 + r.3 / 2 - Framebuffer::text_height(scale);
        fb.draw_text(r.0 + (r.2.saturating_sub(tw)) / 2, y, &title, 230, 232, 240, scale);
        fb.draw_text(r.0 + (r.2.saturating_sub(hw)) / 2, y + Framebuffer::text_height(scale) + 12, &hint, 150, 156, 176, hint_scale);
    }
}

/// `out = a * (1 - t) + b * t`
fn mix_into(out: &mut Framebuffer, a: &Framebuffer, b: &Framebuffer, t: f32) {
    let k = (t.clamp(0.0, 1.0) * 256.0) as u32;
    let (pa, pb) = (a.as_bytes(), b.as_bytes());
    for (o, (x, y)) in out.as_bytes_mut().iter_mut().zip(pa.iter().zip(pb.iter())) {
        *o = ((*x as u32 * (256 - k) + *y as u32 * k) >> 8) as u8;
    }
}

/// The photo, as a very soft, dark, vignetted backdrop of `w` x `h`.
fn blurred_background(t: &Thumb, w: u32, h: u32, dim: u32) -> Framebuffer {
    let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(w, h));
    let (tw, th) = (t.w as f32, t.h as f32);
    // cover: the thumbnail fills the screen
    let scale = (w as f32 / tw).max(h as f32 / th);
    let (ox, oy) = ((tw * scale - w as f32) / 2.0, (th * scale - h as f32) / 2.0);
    let k = (100 - dim.min(100)) as f32 / 100.0;
    let bytes = fb.as_bytes_mut();
    let sample = |x: i32, y: i32, c: usize| -> f32 {
        let (x, y) = (x.clamp(0, t.w as i32 - 1) as usize, y.clamp(0, t.h as i32 - 1) as usize);
        t.px[(y * t.w as usize + x) * 3 + c] as f32
    };
    for y in 0..h {
        let v = (y as f32 + 0.5 + oy) / scale - 0.5;
        let (y0, fy) = (v.floor(), v - v.floor());
        let dy = y as f32 / h as f32 * 2.0 - 1.0;
        for x in 0..w {
            let u = (x as f32 + 0.5 + ox) / scale - 0.5;
            let (x0, fx) = (u.floor(), u - u.floor());
            let dx = x as f32 / w as f32 * 2.0 - 1.0;
            let vignette = 1.0 - 0.28 * (dx * dx * 0.7 + dy * dy * 0.3);
            let i = ((y * w + x) * 3) as usize;
            for c in 0..3 {
                let top = lerp(sample(x0 as i32, y0 as i32, c), sample(x0 as i32 + 1, y0 as i32, c), fx);
                let bot = lerp(sample(x0 as i32, y0 as i32 + 1, c), sample(x0 as i32 + 1, y0 as i32 + 1, c), fx);
                // a little darker overall, the saturation kept
                bytes[i + c] = (lerp(top, bot, fy) * k * vignette).clamp(0.0, 255.0) as u8;
            }
        }
    }
    fb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::WidgetData;

    fn cfg(txt: &str) -> anyhow::Result<Settings> {
        parse(&ConfigFile::parse(txt).unwrap())
    }

    /// A folder with `n` generated pictures of mixed shapes (and a few files that are not pictures).
    fn make_folder(name: &str, n: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("trofeo_frame_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shapes = [(160, 100), (100, 160), (140, 140), (180, 100), (120, 150)];
        for i in 0..n {
            let (w, h) = shapes[i % shapes.len()];
            let img = image::RgbImage::from_fn(w, h, |x, y| {
                image::Rgb([(i * 37 % 200) as u8 + (x * 55 / w) as u8, (i * 71 % 200) as u8 + (y * 55 / h) as u8, 90 + (i * 13 % 120) as u8])
            });
            let ext = if i % 3 == 0 { "jpg" } else { "png" };
            img.save(dir.join(format!("p{i:02}.{ext}"))).unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        std::fs::write(dir.join("broken.jpg"), "this is not a jpeg").unwrap();
        std::fs::write(dir.join(".hidden.png"), "x").unwrap();
        dir
    }

    fn data() -> WidgetData {
        WidgetData { cpu_pct: 37.0, cpu_temp: Some(64.0), gpu_pct: Some(46.0), gpu_temp: Some(58), used_mb: 11800, total_mb: 32768, ..WidgetData::default() }
    }

    fn eff() -> crate::effect::Settings {
        crate::effect::Settings::default()
    }

    fn env(e: &crate::effect::Settings, secs: f64) -> tiles::Env<'_> {
        tiles::Env { fps: 30.0, ffmpeg: None, effect: e, secs: Some(secs) }
    }

    #[test]
    fn options_parse_and_reject_nonsense() {
        let d = Settings::default();
        assert_eq!((d.style, d.photos, d.interval, d.order, d.bg), (Style::Mosaic, 5, 8, Order::Shuffle, Bg::Blur));
        assert_eq!(d.left.len(), 2);
        assert_eq!(d.right.len(), 2);
        assert!(d.folder.is_none());
        let s = cfg("frame_folder = \"/tmp/pics\"\nframe_style = polaroid\nframe_photos = 7\nframe_interval = 20\nframe_order = name\nframe_transition = slide\nframe_gap = 4\nframe_radius = 0\nframe_bg = global\nframe_dim = 30\nframe_side_width = 25\nframe_recursive = true\nframe_refresh = 120\nframe_left = clock\nframe_right = none\n").unwrap();
        assert_eq!(s.folder, Some(PathBuf::from("/tmp/pics")));
        assert_eq!((s.style, s.photos, s.interval, s.order, s.transition), (Style::Polaroid, 7, 20, Order::Name, Transition::Slide));
        assert_eq!((s.gap, s.radius, s.bg, s.dim, s.side_width, s.recursive, s.refresh), (4, 0, Bg::Global, 30, 25, true, 120));
        assert_eq!(s.left.len(), 1);
        assert!(s.right.is_empty());
        for bad in [
            "frame_style = grid", "frame_order = zigzag", "frame_transition = spin", "frame_bg = rainbow", "frame_photos = 0", "frame_photos = 10",
            "frame_interval = 1", "frame_gap = 41", "frame_dim = 101", "frame_side_width = 5", "frame_left = cpu, bogus", "frame_refresh = 2",
        ] {
            assert!(cfg(&format!("{bad}\n")).is_err(), "{bad}");
        }
    }

    #[test]
    fn mosaic_places_fit_the_area_do_not_overlap_and_keep_the_gap() {
        for area in [(0u32, 0u32, 1260u32, 440u32), (10, 20, 400, 1200), (0, 0, 800, 800), (0, 0, 1900, 300)] {
            for n in 1..=9usize {
                let slots = mosaic_slots(area, n, 10);
                assert_eq!(slots.len(), n);
                for (i, a) in slots.iter().enumerate() {
                    assert!(a.0 >= area.0 && a.1 >= area.1 && a.0 + a.2 <= area.0 + area.2 + 1 && a.1 + a.3 <= area.1 + area.3 + 1, "{area:?} n={n}: {a:?}");
                    assert!(a.2 >= 40 && a.3 >= 40 || n > 5, "{area:?} n={n} tiny: {a:?}");
                    for b in &slots[i + 1..] {
                        let apart_x = a.0 + a.2 + 8 <= b.0 || b.0 + b.2 + 8 <= a.0;
                        let apart_y = a.1 + a.3 + 8 <= b.1 || b.1 + b.3 + 8 <= a.1;
                        assert!(apart_x || apart_y, "{area:?} n={n}: {a:?} touches {b:?}");
                    }
                }
                // the same inputs always give the same mosaic
                assert_eq!(slots, mosaic_slots(area, n, 10));
                // and the places fill (almost) all of the area
                let covered: u32 = slots.iter().map(|r| r.2 * r.3).sum();
                assert!(covered as f32 > area.2 as f32 * area.3 as f32 * 0.6, "{area:?} n={n}: {covered}");
            }
        }
    }

    #[test]
    fn polaroid_cards_stay_inside_and_are_tilted_both_ways() {
        for area in [(0u32, 0u32, 1260u32, 440u32), (0, 0, 380, 1400), (0, 0, 600, 600)] {
            for n in 1..=9usize {
                let cards = polaroid_cards(area, n);
                assert_eq!(cards.len(), n);
                for c in &cards {
                    let (half_w, half_h) = (c.cw / 2.0, c.cw * CARD_RATIO / 2.0);
                    assert!(c.cx - half_w >= area.0 as f32 - 12.0 && c.cx + half_w <= (area.0 + area.2) as f32 + 12.0, "{area:?} n={n} {c:?}");
                    assert!(c.cy - half_h >= area.1 as f32 - 20.0 && c.cy + half_h <= (area.1 + area.3) as f32 + 20.0, "{area:?} n={n} {c:?}");
                    assert!(c.angle.abs() >= 3.0 && c.angle.abs() <= 9.0);
                }
                if n >= 2 {
                    assert!(cards.iter().any(|c| c.angle < 0.0) && cards.iter().any(|c| c.angle > 0.0));
                }
            }
        }
    }

    #[test]
    fn cover_fills_the_box_exactly() {
        let img = image::RgbImage::from_fn(300, 200, |x, _| image::Rgb([if x < 150 { 255 } else { 0 }, 0, 0]));
        for (w, h) in [(100, 100), (50, 120), (400, 90), (300, 200), (7, 7)] {
            let c = cover(&img, w, h);
            assert_eq!(c.dimensions(), (w, h));
        }
        // a square crop of a landscape picture keeps the middle: half red, half black
        let c = cover(&img, 100, 100);
        assert!(c.get_pixel(10, 50).0[0] > 200 && c.get_pixel(90, 50).0[0] < 50);
    }

    #[test]
    fn sprites_have_rounded_corners_and_soft_shadows() {
        let img = image::RgbImage::from_pixel(200, 150, image::Rgb([200, 80, 40]));
        let m = mosaic_sprite(&img, 120, 90, 16);
        assert_eq!((m.w, m.h), (120, 90));
        let a = |sp: &Sprite, x: u32, y: u32| sp.px[((y * sp.w + x) * 4 + 3) as usize];
        assert_eq!(a(&m, 0, 0), 0);
        assert_eq!(a(&m, 60, 45), 255);
        assert_eq!(a(&m, 60, 0).max(a(&m, 60, 1)), 255);
        // premultiplied: colour never exceeds alpha
        assert!(m.px.chunks(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));

        let p = polaroid_sprite(&img, 200, 6.0, true);
        assert!(p.w > 200 && p.h > 232, "{}x{}", p.w, p.h);
        assert_eq!((p.ax, p.ay), (-(p.w as i32) / 2, -(p.h as i32) / 2));
        assert_eq!(a(&p, 0, 0), 0, "the corner of the sprite is empty");
        assert_eq!(a(&p, p.w / 2, p.h / 2), 255, "the middle of the print is solid");
        // the shadow: something partly transparent around the card
        assert!(p.px.chunks(4).any(|q| q[3] > 10 && q[3] < 140 && q[0] <= 2));
        assert!(p.px.chunks(4).all(|q| q[0] <= q[3] && q[1] <= q[3] && q[2] <= q[3]));
        // mirrored tilts are mirror images of each other in shape (not identical)
        let q = polaroid_sprite(&img, 200, -6.0, false);
        assert_ne!(p.px, q.px);
    }

    #[test]
    fn blending_a_sprite_respects_opacity_wipe_and_the_edges() {
        let img = image::RgbImage::from_pixel(60, 60, image::Rgb([255, 255, 255]));
        let sp = mosaic_sprite(&img, 40, 40, 0);
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(100, 100));
        fb.clear(0, 0, 0);
        blend_sprite(&mut fb, &sp, 10, 10, 1.0, None);
        let px = |fb: &Framebuffer, x: u32, y: u32| fb.as_bytes()[((y * 100 + x) * 3) as usize];
        assert_eq!(px(&fb, 30, 30), 255);
        assert_eq!(px(&fb, 5, 30), 0);
        fb.clear(0, 0, 0);
        blend_sprite(&mut fb, &sp, 10, 10, 0.5, None);
        assert!((px(&fb, 30, 30) as i32 - 128).abs() <= 3);
        fb.clear(0, 0, 0);
        blend_sprite(&mut fb, &sp, 10, 10, 1.0, Some(0.5));
        assert_eq!(px(&fb, 12, 30), 255, "left part is revealed");
        assert_eq!(px(&fb, 48, 30), 0, "right part is not yet");
        // partly outside the screen: no panic, no wrap
        fb.clear(0, 0, 0);
        blend_sprite(&mut fb, &sp, -20, 80, 1.0, None);
        blend_sprite(&mut fb, &sp, 90, -30, 1.0, Some(1.0));
        assert_eq!(px(&fb, 5, 90), 255);
    }

    #[test]
    fn scanning_finds_pictures_only_and_reads_sizes() {
        let dir = make_folder("scan", 6);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        image::RgbImage::new(30, 20).save(dir.join("sub").join("deep.png")).unwrap();
        let flat = scan_folder(&dir, false);
        let names: Vec<_> = flat.iter().map(|p| p.path.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(names, ["p00.jpg", "p01.png", "p02.png", "p03.jpg", "p04.png", "p05.png"], "{names:?}");
        assert_eq!((flat[0].w, flat[0].h), (160, 100));
        assert_eq!((flat[1].w, flat[1].h), (100, 160));
        assert_eq!(scan_folder(&dir, true).len(), 7);
        assert!(scan_folder(&dir.join("nope"), true).is_empty());
        assert!(decode(&dir.join("broken.jpg")).is_none());
        assert_eq!(decode(&dir.join("p01.png")).unwrap().dimensions(), (100, 160));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn run(style: Style, order: Order, photos: u32, dir: &Path) -> (FrameScreen, Settings, Framebuffer) {
        let s = Settings { folder: Some(dir.to_path_buf()), style, order, photos, interval: 5, ..Settings::default() };
        let mut screen = FrameScreen::with_seed(3);
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(1920, 462));
        let e = eff();
        screen.draw_settled(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tiles::Settings::default(), &env(&e, 0.0));
        (screen, s, fb)
    }

    #[test]
    fn the_frame_shows_the_pictures_swaps_one_at_a_time_and_idles_cheaply() {
        let dir = make_folder("screen", 12);
        for style in [Style::Mosaic, Style::Polaroid] {
            let (mut screen, s, mut fb) = run(style, Order::Shuffle, 5, &dir);
            assert_eq!(screen.slots.len(), 5);
            assert!(screen.slots.iter().all(|sl| sl.cur.is_some()), "{style:?}: every place has a picture");
            let first: Vec<PathBuf> = screen.slots.iter().filter_map(|sl| sl.photo.clone()).collect();
            let distinct: HashSet<_> = first.iter().collect();
            assert_eq!(distinct.len(), 5, "no picture twice");
            // the screen is not blank in the middle
            let mid = &fb.as_bytes()[((231 * 1920 + 960) * 3) as usize..][..3];
            assert!(mid.iter().any(|v| *v > 20), "{style:?}: {mid:?}");

            // time passes without anything due: nothing is composed again
            let e = eff();
            let tcfg = tiles::Settings::default();
            // the arrival fade-in (1.4 s) is over after the first of these
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 1.5));
            let composes = screen.composes;
            for t in [2.0, 3.0, 4.0] {
                screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, t));
            }
            assert_eq!(screen.composes, composes, "{style:?}: an idle frame must come from the cache");
            assert!(!screen.busy());
            assert_eq!(screen.wanted_fps(), IDLE_FPS);

            // the interval is over: exactly one place changes
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 6.0));
            assert!(screen.loading() || screen.busy());
            assert_eq!(screen.wanted_fps(), BUSY_FPS);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while screen.loading() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
                screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 6.1));
            }
            let after: Vec<PathBuf> = screen.slots.iter().filter_map(|sl| sl.photo.clone()).collect();
            let changed = first.iter().zip(after.iter()).filter(|(a, b)| a != b).count();
            assert_eq!(changed, 1, "{style:?}: one picture is replaced");
            assert_eq!(after.iter().collect::<HashSet<_>>().len(), 5);
            // mid-transition: composed every frame; after it: back to the cache
            let c0 = screen.composes;
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 6.5));
            assert!(screen.composes > c0);
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 8.0));
            let c1 = screen.composes;
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 8.5));
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 9.0));
            assert!(screen.composes <= c1 + 1, "{style:?}: cache again after the transition");
            assert!(!screen.busy());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn name_order_walks_through_the_folder_and_every_picture_comes_up() {
        let dir = make_folder("order", 9);
        let (mut screen, s, mut fb) = run(Style::Mosaic, Order::Name, 3, &dir);
        let names = |sc: &FrameScreen| -> Vec<String> { sc.slots.iter().map(|sl| sl.photo.as_ref().unwrap().file_name().unwrap().to_string_lossy().to_string()).collect() };
        assert_eq!(names(&screen), ["p00.jpg", "p01.png", "p02.png"]);
        let (e, tcfg) = (eff(), tiles::Settings::default());
        let mut seen: HashSet<String> = names(&screen).into_iter().collect();
        let mut t = 0.0;
        for _ in 0..12 {
            t += 5.5;
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, t));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while screen.loading() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
                screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, t));
            }
            seen.extend(names(&screen));
        }
        assert_eq!(seen.len(), 9, "{seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn few_pictures_few_places_and_odd_folders_do_not_break_it() {
        let (e, tcfg) = (eff(), tiles::Settings::default());
        // 2 pictures for 5 places: 2 places, no swaps (nothing to swap in)
        let dir = make_folder("few", 2);
        let (mut screen, s, mut fb) = run(Style::Mosaic, Order::Shuffle, 5, &dir);
        assert_eq!(screen.slots.len(), 2);
        screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 50.0));
        assert!(!screen.loading());
        let _ = std::fs::remove_dir_all(&dir);
        // an empty folder, a missing one, no folder at all: a message, the side tiles, no panic
        let empty = std::env::temp_dir().join(format!("trofeo_frame_test_empty_{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        for folder in [Some(empty.clone()), Some(empty.join("missing")), None] {
            let s = Settings { folder, ..Settings::default() };
            let mut screen = FrameScreen::with_seed(1);
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(1920, 462));
            screen.draw_settled(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 0.0));
            assert!(screen.slots.is_empty());
            assert!(fb.as_bytes().iter().any(|v| *v > 100), "the message is drawn");
        }
        let _ = std::fs::remove_dir_all(&empty);
        // a tiny area and portrait
        let dir = make_folder("odd", 6);
        for (w, h, sides) in [(200u32, 120u32, true), (400, 1920, true), (1920, 462, false)] {
            let mut s = Settings { folder: Some(dir.clone()), ..Settings::default() };
            if !sides {
                s.left.clear();
                s.right.clear();
            }
            let mut screen = FrameScreen::with_seed(1);
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(w, h));
            screen.draw_settled(&mut fb, (0, 0, w, h), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 0.0));
            if sides {
                continue;
            }
            // without side columns the collage uses the whole width
            let (_, _, collage) = FrameScreen::rects(&s, (0, 0, w, h), tcfg.gap);
            assert_eq!(collage.2, w - tcfg.gap * 2);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn side_columns_and_strips_leave_the_rest_to_the_collage() {
        let s = Settings::default();
        let (l, r, c) = FrameScreen::rects(&s, (0, 0, 1920, 462), 14);
        let (l, r) = (l.unwrap(), r.unwrap());
        assert_eq!(l.0, 0);
        assert_eq!(r.0 + r.2, 1920);
        assert_eq!(c.0, l.2);
        assert_eq!(c.0 + c.2, r.0);
        // portrait: strips on top and bottom
        let (t, b, c) = FrameScreen::rects(&s, (0, 0, 400, 1920), 14);
        let (t, b) = (t.unwrap(), b.unwrap());
        assert_eq!((t.0, t.1, t.2), (0, 0, 400));
        assert_eq!(b.1 + b.3, 1920);
        assert_eq!(c.1, t.3);
        assert_eq!(c.1 + c.3, b.1);
    }

    #[test]
    fn a_changed_setting_rebuilds_the_collage() {
        let dir = make_folder("rebuild", 8);
        let (mut screen, s, mut fb) = run(Style::Mosaic, Order::Shuffle, 5, &dir);
        let (e, tcfg) = (eff(), tiles::Settings::default());
        let s2 = Settings { style: Style::Polaroid, photos: 3, ..s };
        screen.draw_settled(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s2, &tcfg, &env(&e, 0.0));
        assert_eq!(screen.slots.len(), 3);
        assert!(screen.slots.iter().all(|sl| sl.card.is_some() && sl.cur.is_some()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Times a swap and an idle frame at full size.
    #[test]
    fn frame_cost_is_small() {
        let dir = make_folder("cost", 12);
        let (e, tcfg) = (eff(), tiles::Settings::default());
        for style in [Style::Mosaic, Style::Polaroid] {
            let (mut screen, s, mut fb) = run(style, Order::Shuffle, 5, &dir);
            let t = std::time::Instant::now();
            for i in 0..50 {
                screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 1.0 + i as f64 * 0.01));
            }
            let idle = t.elapsed() / 50;
            // force transition frames by composing directly
            screen.scene_valid = false;
            let t = std::time::Instant::now();
            for _ in 0..20 {
                screen.compose(&mut fb, &s);
            }
            let compose = t.elapsed() / 20;
            println!("{style:?}: idle frame {idle:?}, compose {compose:?}");
            assert!(idle.as_millis() < 25, "{style:?} idle frame {idle:?}");
            assert!(compose.as_millis() < 60, "{style:?} compose {compose:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With TROFEO_DUMP_DIR + TROFEO_FRAME_PHOTOS set, saves previews of the looks.
    #[test]
    fn dump_previews() {
        let (Ok(dir), Ok(photos)) = (std::env::var("TROFEO_DUMP_DIR"), std::env::var("TROFEO_FRAME_PHOTOS")) else { return };
        let (e, tcfg) = (eff(), tiles::Settings::default());
        let cases: Vec<(&str, Settings, (u32, u32))> = vec![
            ("frame_mosaic", Settings::default(), (1920, 462)),
            ("frame_polaroid", Settings { style: Style::Polaroid, ..Settings::default() }, (1920, 462)),
            ("frame_mosaic7_slide_nosides", Settings { photos: 7, left: vec![], right: vec![], ..Settings::default() }, (1920, 462)),
            ("frame_polaroid_global", Settings { style: Style::Polaroid, bg: Bg::Dark, photos: 4, ..Settings::default() }, (1920, 462)),
            ("frame_mosaic_portrait", Settings { photos: 4, ..Settings::default() }, (462, 1920)),
            ("frame_polaroid_portrait", Settings { style: Style::Polaroid, photos: 3, ..Settings::default() }, (462, 1920)),
        ];
        // a swap caught half way
        for (name, style, transition) in [("frame_mid_fade", Style::Mosaic, Transition::Fade), ("frame_mid_wipe", Style::Mosaic, Transition::Slide), ("frame_mid_drop", Style::Polaroid, Transition::Slide)] {
            let s = Settings { folder: Some(PathBuf::from(&photos)), style, transition, interval: 5, ..Settings::default() };
            let mut screen = FrameScreen::with_seed(7);
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(1920, 462));
            screen.draw_settled(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 0.0));
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 3.0));
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 10.0));
            while screen.loading() {
                std::thread::sleep(std::time::Duration::from_millis(5));
                screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 10.0));
            }
            screen.draw(&mut fb, (0, 0, 1920, 462), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 10.6));
            std::fs::write(format!("{dir}/{name}.ppm"), [b"P6\n1920 462\n255\n".to_vec(), fb.as_bytes().to_vec()].concat()).unwrap();
        }
        for (name, mut s, (w, h)) in cases {
            s.folder = Some(PathBuf::from(&photos));
            let mut screen = FrameScreen::with_seed(7);
            screen.still = true;
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(w, h));
            screen.draw_settled(&mut fb, (0, 0, w, h), &data(), ColorMode::Default, &s, &tcfg, &env(&e, 0.0));
            std::fs::write(format!("{dir}/{name}.ppm"), [format!("P6\n{w} {h}\n255\n").into_bytes(), fb.as_bytes().to_vec()].concat()).unwrap();
        }
    }
}
