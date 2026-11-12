//! The `steam` screen: the covers of the Steam games installed on this computer in a cover flow.
//! The cover in the middle is the game on show, the others lean away on both sides; every few
//! seconds the row slides to the next game. (Settings first; the drawing follows below.)

use super::*;
use std::path::PathBuf;

/// In which order the games come.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// The ones played last first.
    Recent,
    /// A to Z.
    Name,
    /// Mixed up (the same mix until the library changes).
    Random,
}

impl Order {
    pub const NAMES: &'static str = "recent | name | random";

    pub fn parse(s: &str) -> Option<Order> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "recent" | "last" | "played" | "recenti" => Order::Recent,
            "name" | "alpha" | "az" | "nome" => Order::Name,
            "random" | "shuffle" | "casuale" => Order::Random,
            _ => return None,
        })
    }
}

/// What is behind the covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Back {
    /// The cover in the middle, enlarged, blurred and darkened.
    Blur,
    Mode(BgMode),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    /// Where Steam is installed (`None` = look for it).
    pub path: Option<PathBuf>,
    /// Seconds each game stays in the middle.
    pub interval: u32,
    /// Milliseconds the row takes to slide to the next game.
    pub speed: u32,
    /// Covers shown on each side of the middle one.
    pub side: u32,
    pub order: Order,
    /// 0-100: strength of the reflection under the covers (0 = none).
    pub reflection: u32,
    /// 0-90: how much the covers at the sides turn away.
    pub tilt: u32,
    pub back: Back,
    pub bg_colors: Vec<Rgb>,
    pub bg_image: Option<PathBuf>,
    pub bg_dim: u32,
    /// The name of the game under the middle cover.
    pub title: bool,
    /// Play time and last play under the name.
    pub details: bool,
    pub clock: bool,
    /// Leave out the games Steam has no cover for.
    pub only_covers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            path: None,
            interval: 6,
            speed: 800,
            side: 3,
            order: Order::Recent,
            reflection: 45,
            tilt: 55,
            back: Back::Blur,
            bg_colors: vec![(0x0B, 0x0E, 0x1C), (0x1A, 0x10, 0x30)],
            bg_image: None,
            bg_dim: 0,
            title: true,
            details: true,
            clock: true,
            only_covers: false,
        }
    }
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get("steam_path") {
        let v = v.trim();
        if !v.is_empty() {
            s.path = Some(PathBuf::from(v));
        }
    }
    let num = |key: &str, lo: u32, hi: u32| -> anyhow::Result<Option<u32>> {
        match file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            None => Ok(None),
            Some(v) if (lo..=hi).contains(&v) => Ok(Some(v)),
            Some(v) => anyhow::bail!("{key}: must be between {lo} and {hi} (got {v})"),
        }
    };
    let boolean = |key: &str| file.get_bool(key).map_err(|e| anyhow::anyhow!(e));
    if let Some(v) = num("steam_interval", 2, 600)? {
        s.interval = v;
    }
    if let Some(v) = num("steam_speed", 100, 5000)? {
        s.speed = v;
    }
    if let Some(v) = num("steam_side", 1, 5)? {
        s.side = v;
    }
    if let Some(v) = file.get("steam_order") {
        s.order = Order::parse(v).ok_or_else(|| anyhow::anyhow!("steam_order: '{v}' not valid ({})", Order::NAMES))?;
    }
    if let Some(v) = num("steam_reflection", 0, 100)? {
        s.reflection = v;
    }
    if let Some(v) = num("steam_tilt", 0, 90)? {
        s.tilt = v;
    }
    if let Some(v) = file.get("steam_bg") {
        s.back = match v.trim().to_ascii_lowercase().as_str() {
            "blur" | "cover" | "blurred" | "sfocato" => Back::Blur,
            other => Back::Mode(BgMode::parse(other).ok_or_else(|| anyhow::anyhow!("steam_bg: '{v}' not valid (blur | {})", BgMode::NAMES))?),
        };
    }
    if let Some(v) = file.get("steam_bg_color") {
        let list = parse_rgb_list("steam_bg_color", v)?;
        if list.is_empty() {
            anyhow::bail!("steam_bg_color: needs at least one color, e.g. #10142A or #10142A, #241030");
        }
        s.bg_colors = list;
    }
    if let Some(v) = file.get("steam_bg_image") {
        s.bg_image = Some(PathBuf::from(v));
    }
    if s.back == Back::Mode(BgMode::Image) && s.bg_image.is_none() {
        anyhow::bail!("steam_bg = image needs steam_bg_image = <file>");
    }
    s.bg_dim = num("steam_bg_dim", 0, 100)?.unwrap_or(0);
    if let Some(v) = boolean("steam_title")? {
        s.title = v;
    }
    if let Some(v) = boolean("steam_details")? {
        s.details = v;
    }
    if let Some(v) = boolean("steam_clock")? {
        s.clock = v;
    }
    if let Some(v) = boolean("steam_only_covers")? {
        s.only_covers = v;
    }
    Ok(s)
}

// ------------------------------------------------------------------ the screen

use super::cover::{self, Tex};
use crate::game_info;
use crate::steam::{CoverKind, Library, SteamGame};
use std::collections::HashMap;
use std::sync::Arc;

/// Size of the picture every game is turned into (the covers are scaled to it once).
const CARD_W: u32 = 300;
const CARD_H: u32 = 450;

fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// The games in the order the settings ask for.
fn ordered<'a>(lib: &'a Library, s: &Settings) -> Vec<&'a SteamGame> {
    let mut v: Vec<&SteamGame> = lib.games.iter().filter(|g| !s.only_covers || g.cover.is_some()).collect();
    match s.order {
        Order::Recent => {}
        Order::Name => v.sort_by_key(|g| g.name.to_lowercase()),
        Order::Random => v.sort_by_key(|g| mix32(g.appid ^ 0x5eed)),
    }
    v
}

/// Where the row is at time `t`: the (fractional) slot of the cover in the middle. The games come
/// one after the other every `interval` seconds; the row slides during the first `speed` ms.
fn flow_pos(t: f64, interval: u32, speed: u32, n: usize) -> f32 {
    if n < 2 {
        return 0.0;
    }
    let iv = interval.max(1) as f64;
    let k = (t.max(0.0) / iv).floor();
    if k < 1.0 {
        return 0.0;
    }
    let phase = t - k * iv;
    let tr = (speed as f64 / 1000.0).clamp(0.05, iv * 0.9);
    let e = (phase / tr).clamp(0.0, 1.0);
    let e = e * e * (3.0 - 2.0 * e);
    (k - 1.0 + e) as f32
}

/// "3 days ago" for a unix time.
fn ago_text(now: u64, then: u64) -> Option<String> {
    if then == 0 || then > now + 60 {
        return None;
    }
    let d = (now - then) / 86_400;
    Some(match d {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=29 => format!("{d} days ago"),
        30..=364 => format!("{} months ago", d / 30),
        _ => format!("{} years ago", d / 365),
    })
}

/// Splits `text` into at most two lines that fit `max_w`.
fn wrap_lines(p: &Painter, text: &str, cap: f32, max_w: f32, w: W) -> Vec<String> {
    if p.width(text, cap, w) <= max_w {
        return vec![text.to_string()];
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut first = String::new();
    let mut i = 0;
    while i < words.len() {
        let t = if first.is_empty() { words[i].to_string() } else { format!("{first} {}", words[i]) };
        if p.width(&t, cap, w) > max_w && !first.is_empty() {
            break;
        }
        first = t;
        i += 1;
    }
    let rest = words[i..].join(" ");
    if rest.is_empty() {
        vec![first]
    } else {
        vec![dash::clip(p, &first, cap, max_w, w), dash::clip(p, &rest, cap, max_w, w)]
    }
}

/// The picture of one game, `CARD_W` x `CARD_H`: its box art, a banner on a coloured card, or a
/// card drawn with the name when Steam has no picture for it.
fn make_card(game: &SteamGame, art: Option<(&Tex, CoverKind)>) -> Tex {
    let (w, h) = (CARD_W as f32, CARD_H as f32);
    let mut layer = Layer::new(CARD_W, CARD_H);
    let hue = (mix32(game.appid) % 360) as f32;
    let base = hsv(hue, 0.55, 0.55);
    let dark = hsv(hue, 0.7, 0.14);
    layer.vgrad((0.0, 0.0, w, h), base, dark, 255);
    let mut p = Painter { layer: &mut layer, font: FontStyle::Smooth };
    let mut name_y = h * 0.74;
    match art {
        Some((tex, CoverKind::Portrait)) => {
            super::game::blit_cover(p.layer, tex, (0.0, 0.0, w, h), 0.0);
            return into_tex(layer);
        }
        Some((tex, CoverKind::Wide)) => {
            let avg = tex.average();
            p.layer.vgrad((0.0, 0.0, w, h), lerp_rgb(avg, (0, 0, 0), 0.45), lerp_rgb(avg, (0, 0, 0), 0.85), 255);
            let ph = w * tex.h as f32 / tex.w as f32;
            super::game::blit_cover(p.layer, tex, (0.0, h * 0.3, w, ph), 0.0);
            name_y = h * 0.3 + ph + h * 0.1;
        }
        None => {
            let letter: String = game.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            let cap = h * 0.3;
            let wd = p.width(&letter, cap, W::Medium);
            p.text((w - wd) / 2.0, h * 0.42, &letter, cap, W::Medium, lerp_rgb(base, (255, 255, 255), 0.55), 255);
        }
    }
    let cap = w * 0.085;
    let mut y = name_y;
    for line in wrap_lines(&p, &game.name, cap, w * 0.86, W::Medium) {
        let wd = p.width(&line, cap, W::Medium);
        p.text((w - wd) / 2.0, y, &line, cap, W::Medium, (255, 255, 255), 255);
        y += cap * 1.45;
    }
    into_tex(layer)
}

fn into_tex(layer: Layer) -> Tex {
    // every pixel is opaque, so the premultiplied values are the straight ones
    Tex { w: layer.w, h: layer.h, px: layer.px }
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let hp = (h / 60.0) % 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

/// Draws one cover in perspective: `x_off` from the centre, turned by `theta` (radians, the
/// edge nearer the middle is the farther one), with its bottom on `floor_y`, and its reflection
/// below. `dist` is the distance of the viewer.
#[allow(clippy::too_many_arguments)]
fn blit_flow(layer: &mut Layer, tex: &Tex, cx: f32, floor_y: f32, wc: f32, hc: f32, x_off: f32, theta: f32, side: f32, dist: f32, shade: f32, refl: f32, refl_len: f32, fade: f32) {
    let a = wc * theta.cos();
    let b = -side * wc * theta.sin();
    let proj = |v: f32| cx + (x_off + a * v) * dist / (dist + b * v);
    let (e0, e1) = (proj(-0.5), proj(0.5));
    let (sx0, sx1) = (e0.min(e1), e0.max(e1));
    for px in (sx0.floor() as i32 - 1)..(sx1.ceil() as i32 + 1) {
        let dxs = px as f32 + 0.5 - cx;
        let denom = dist * a - dxs * b;
        if denom.abs() < 1e-3 {
            continue;
        }
        let v = dist * (dxs - x_off) / denom;
        // soft edge: how much of this pixel column the cover covers
        let edge = ((0.5 - v.abs()) * (sx1 - sx0).max(1.0) + 0.5).clamp(0.0, 1.0) * fade;
        if edge <= 0.0 {
            continue;
        }
        let k = dist / (dist + b * v);
        let hh = hc * k;
        let u = (v + 0.5) * tex.w as f32;
        let top = floor_y - hh;
        for py in top.floor() as i32..floor_y.ceil() as i32 {
            let fy = py as f32 + 0.5;
            let vv = (fy - top) / hh;
            if !(0.0..=1.0).contains(&vv) {
                continue;
            }
            let c = tex.sample(u, vv * tex.h as f32);
            layer.over(px, py, ((c[0] * shade) as u8, (c[1] * shade) as u8, (c[2] * shade) as u8), (c[3] * edge) as u8);
        }
        if refl > 0.0 {
            let rl = refl_len * k;
            for py in floor_y.ceil() as i32..(floor_y + rl).ceil() as i32 {
                let dy = py as f32 + 0.5 - floor_y;
                let t = (dy / rl).clamp(0.0, 1.0);
                let vv = 1.0 - dy / hh;
                if vv < 0.0 {
                    continue;
                }
                let c = tex.sample(u, vv * tex.h as f32);
                let alpha = refl * 0.7 * (1.0 - t).powf(1.7) * edge * (c[3] / 255.0);
                layer.over(px, py, ((c[0] * shade) as u8, (c[1] * shade) as u8, (c[2] * shade) as u8), (255.0 * alpha) as u8);
            }
        }
    }
}

pub struct SteamScreen {
    started: std::time::Instant,
    layer: Layer,
    sig: String,
    bg: TilesScreen,
    cards: HashMap<u32, Tex>,
    /// The blurred cover behind everything: (what it shows, the picture).
    blur: Option<(String, Framebuffer)>,
    pub redraws: u32,
}

impl SteamScreen {
    pub fn new() -> Self {
        SteamScreen { started: std::time::Instant::now(), layer: Layer::new(1, 1), sig: String::new(), bg: TilesScreen::new(), cards: HashMap::new(), blur: None, redraws: 0 }
    }

    /// Frames per second wanted now: smooth while the row slides, a few while it rests (more when
    /// the background is an effect or a picture).
    pub fn wanted_fps(&self, s: &Settings, n: usize, effect_fps: f32, active_fps: f32) -> f32 {
        let t = self.started.elapsed().as_secs_f64();
        let moving = n > 1 && {
            let iv = s.interval.max(1) as f64;
            let k = (t / iv).floor();
            k >= 1.0 && t - k * iv < s.speed as f64 / 1000.0 + 0.1
        };
        let bg = match s.back {
            Back::Mode(BgMode::Effect) => effect_fps,
            Back::Mode(BgMode::Image) => active_fps,
            _ => 4.0,
        };
        if moving { active_fps.max(24.0).min(30.0).max(bg) } else { bg }
    }

    /// The picture of a game, read now when the budget allows (one per frame); otherwise a card
    /// drawn with the name.
    fn card(&mut self, g: &SteamGame, budget: &mut bool) -> Tex {
        if let Some(t) = self.cards.get(&g.appid) {
            return Tex { w: t.w, h: t.h, px: t.px.clone() };
        }
        if !*budget {
            return make_card(g, None);
        }
        *budget = false;
        let art = g.cover.as_ref().and_then(|c| {
            let height = if c.kind == CoverKind::Portrait { CARD_H } else { 260 };
            cover::load_tex(&c.path, height).map(|t| (t, c.kind))
        });
        let tex = make_card(g, art.as_ref().map(|(t, k)| (t, *k)));
        // keep a few: the ones around the middle are asked for again at once
        if self.cards.len() >= 24 {
            self.cards.clear();
        }
        let copy = Tex { w: tex.w, h: tex.h, px: tex.px.clone() };
        self.cards.insert(g.appid, copy);
        tex
    }

    /// Paints the background chosen by `steam_bg`.
    fn paint_background(&mut self, fb: &mut Framebuffer, s: &Settings, env: &Env, near: [Option<u32>; 2], mixt: f32) {
        match s.back {
            Back::Blur => {
                let q = (mixt * 24.0).round() as u32;
                let key = format!("{near:?}|{q}|{}x{}|{}", fb.width(), fb.height(), self.cards.len());
                if self.blur.as_ref().map_or(true, |b| b.0 != key) {
                    let (gw, gh) = (24usize, 14usize);
                    let grid = |id: Option<u32>, cards: &HashMap<u32, Tex>| -> Vec<[f32; 3]> {
                        let mut g = vec![[10.0, 10.0, 16.0]; gw * gh];
                        if let Some(t) = id.and_then(|i| cards.get(&i)) {
                            for gy in 0..gh {
                                for gx in 0..gw {
                                    let mut acc = [0.0f32; 3];
                                    for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                                        let c = t.sample((gx as f32 + sx) / gw as f32 * t.w as f32, (gy as f32 + sy) / gh as f32 * t.h as f32);
                                        for k in 0..3 {
                                            acc[k] += c[k] / 4.0;
                                        }
                                    }
                                    g[gy * gw + gx] = acc;
                                }
                            }
                        }
                        g
                    };
                    let (ga, gb) = (grid(near[0], &self.cards), grid(near[1], &self.cards));
                    let (w, h) = (fb.width() as usize, fb.height() as usize);
                    let mut out = Framebuffer::new(trofeo_lcd::Resolution::new(fb.width(), fb.height()));
                    let bytes = out.as_bytes_mut();
                    let e = q as f32 / 24.0;
                    let at = |gx: usize, gy: usize| -> [f32; 3] {
                        let (i, j) = (gy * gw + gx, gy * gw + gx);
                        [ga[i][0] + (gb[j][0] - ga[i][0]) * e, ga[i][1] + (gb[j][1] - ga[i][1]) * e, ga[i][2] + (gb[j][2] - ga[i][2]) * e]
                    };
                    for y in 0..h {
                        let fy = ((y as f32 + 0.5) / h as f32 * gh as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
                        let (y0, ty) = (fy.floor() as usize, fy.fract());
                        let y1 = (y0 + 1).min(gh - 1);
                        for x in 0..w {
                            let fx = ((x as f32 + 0.5) / w as f32 * gw as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
                            let (x0, tx) = (fx.floor() as usize, fx.fract());
                            let x1 = (x0 + 1).min(gw - 1);
                            let (c00, c10, c01, c11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
                            // vignette: darker toward the corners
                            let (nx, ny) = (x as f32 / w as f32 * 2.0 - 1.0, y as f32 / h as f32 * 2.0 - 1.0);
                            let vig = 1.0 - 0.38 * (nx * nx * 0.6 + ny * ny * 0.4).min(1.0);
                            let i = (y * w + x) * 3;
                            for k in 0..3 {
                                let top = c00[k] + (c10[k] - c00[k]) * tx;
                                let bot = c01[k] + (c11[k] - c01[k]) * tx;
                                bytes[i + k] = ((top + (bot - top) * ty) * 0.42 * vig + 6.0).clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                    self.blur = Some((key, out));
                }
                if let Some((_, f)) = &self.blur {
                    fb.copy_from(f);
                }
            }
            Back::Mode(m) => {
                let ts = tiles::Settings { bg: m, bg_colors: s.bg_colors.clone(), bg_image: s.bg_image.clone(), bg_dim: s.bg_dim, ..tiles::Settings::default() };
                self.bg.paint_background(fb, &ts, env);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, fb: &mut Framebuffer, area: Rect, lib: Option<&Arc<Library>>, color_mode: ColorMode, s: &Settings, env: &Env) {
        let games: Vec<&SteamGame> = lib.map(|l| ordered(l, s)).unwrap_or_default();
        let n = games.len();
        let (ax, ay, aw, ah) = area;
        let secs = env.secs.unwrap_or_else(|| self.started.elapsed().as_secs_f64());
        let cpos = flow_pos(secs, s.interval, s.speed, n);
        // the games in the middle now and next (for the blurred background and the title)
        let base = cpos.floor();
        let at = |j: i64| -> Option<&SteamGame> { (n > 0).then(|| games[j.rem_euclid(n as i64) as usize]) };
        let near = [at(base as i64).map(|g| g.appid), at(base as i64 + 1).map(|g| g.appid)];
        // a preview is one frame: read them all now
        let mut budget = true;
        let preview = env.secs.is_some();
        // read the pictures around the middle (one per frame)
        let side = (s.side as usize).min(n.saturating_sub(1) / 2 + 1) as i64;
        let mut visible: Vec<(i64, Tex)> = Vec::new();
        if n > 0 {
            let (lo, hi) = (cpos.floor() as i64 - side - 1, cpos.ceil() as i64 + side + 1);
            for j in lo..=hi {
                let g = games[j.rem_euclid(n as i64) as usize];
                budget = budget || preview;
                let t = self.card(g, &mut budget);
                visible.push((j, t));
            }
        }
        self.paint_background(fb, s, env, near, cpos - base);
        let inset = (ah as f32 * 0.02).clamp(4.0, 12.0);
        let rect = (inset, inset, aw as f32 - 2.0 * inset, ah as f32 - 2.0 * inset);
        if rect.2 < 120.0 || rect.3 < 80.0 {
            return;
        }
        let now = Local::now();
        let clock = crate::fmt_clock(&now, false);
        let sig = format!("{:.4}|{area:?}|{s:?}|{n}|{clock}|{}|{:?}|{}", cpos, self.cards.len(), color_mode, lib.map_or(0, |l| l.games.len()));
        if sig != self.sig {
            if self.layer.w != aw as usize || self.layer.h != ah as usize {
                self.layer = Layer::new(aw, ah);
            } else {
                self.layer.clear();
            }
            let mut painter = Painter { layer: &mut self.layer, font: FontStyle::Smooth };
            draw_flow(&mut painter, rect, &visible, &games, cpos, s, &clock, lib.is_some(), now.timestamp().max(0) as u64);
            self.sig = sig;
            self.redraws += 1;
        }
        self.layer.composite_onto(fb, ax, ay);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_flow(p: &mut Painter, rect: (f32, f32, f32, f32), visible: &[(i64, Tex)], games: &[&SteamGame], cpos: f32, s: &Settings, clock: &str, has_steam: bool, now: u64) {
    let (x, y, w, h) = rect;
    let ink = (0xF2, 0xF4, 0xFA);
    let tall = h > w * 1.1;
    let hcap = (h.min(w) * 0.04).clamp(11.0, 21.0);
    if s.clock {
        txt_r_flow(p, x + w - 6.0, y + hcap + 4.0, clock, hcap, W::Medium, ink, 220);
    }
    if games.is_empty() {
        let msg = if has_steam { "No installed Steam games found" } else { "Steam was not found on this computer" };
        let cap = (h.min(w) * 0.07).clamp(12.0, 34.0);
        let cx = x + w / 2.0;
        let wd = p.width(msg, cap, W::Medium);
        p.text(cx - wd / 2.0, y + h / 2.0, msg, cap, W::Medium, ink, 230);
        let hint = "Set steam_path in the settings if Steam is somewhere else";
        let hc = cap * 0.55;
        let hw = p.width(hint, hc, W::Regular);
        p.text(cx - hw / 2.0, y + h / 2.0 + cap * 1.6, hint, hc, W::Regular, ink, 150);
        return;
    }
    p.text(x + 6.0, y + hcap + 4.0, &format!("STEAM   {} games", games.len()), hcap * 0.9, W::Medium, ink, 150);
    // geometry
    let hc = if tall { (h * 0.34).min(w * 0.87) } else { h * 0.6 };
    let wc = hc * CARD_W as f32 / CARD_H as f32;
    let floor_y = y + if tall { h * 0.5 + hc * 0.5 } else { h * 0.73 };
    let cx = x + w / 2.0;
    let dist = wc * 3.2;
    let tilt = (s.tilt as f32).to_radians();
    let gap_c = wc * if tall { 0.68 } else { 1.0 };
    let gap_s = wc * if tall { 0.26 } else { 0.55 };
    let side = (s.side as usize).min(games.len().saturating_sub(1) / 2 + 1) as i64;
    let mut order: Vec<&(i64, Tex)> = visible.iter().collect();
    order.sort_by(|a, b| (b.0 as f32 - cpos).abs().partial_cmp(&(a.0 as f32 - cpos).abs()).unwrap_or(std::cmp::Ordering::Equal));
    let refl_len = hc * 0.36;
    for (j, tex) in order.into_iter().map(|(j, t)| (*j, t)) {
        let d = j as f32 - cpos;
        let a = d.abs();
        let sgn = if d < 0.0 { -1.0 } else { 1.0 };
        let off = sgn * if a <= 1.0 { a * gap_c } else { gap_c + (a - 1.0) * gap_s };
        let turn = a.min(1.0);
        let turn = turn * turn * (3.0 - 2.0 * turn);
        let size = 1.0 - 0.16 * a.min(1.0) - 0.05 * (a - 1.0).clamp(0.0, 3.0);
        let shade = (1.0 - 0.42 * a.min(1.0) - 0.08 * (a - 1.0).clamp(0.0, 3.0)).clamp(0.2, 1.0);
        let fade = (side as f32 + 1.0 - a).clamp(0.0, 1.0);
        if fade <= 0.0 {
            continue;
        }
        blit_flow(p.layer, tex, cx, floor_y, wc * size, hc * size, off, tilt * turn, sgn, dist, shade, s.reflection as f32 / 100.0, refl_len * size, fade);
    }
    // title and facts of the game in the middle
    if s.title {
        let near = cpos.round();
        let alpha = (1.0 - (cpos - near).abs() * 2.0).clamp(0.0, 1.0);
        if let Some(g) = games.get((near as i64).rem_euclid(games.len() as i64) as usize) {
            let cap = (h.min(w) * if tall { 0.05 } else { 0.058 }).clamp(12.0, 34.0);
            let ty = floor_y + if tall { refl_len + cap * 2.2 } else { h * 0.12 };
            let name = dash::clip(p, &g.name, cap, w * 0.8, W::Medium);
            let nw = p.width(&name, cap, W::Medium);
            let mut facts: Vec<String> = Vec::new();
            if s.details {
                if let Some(m) = g.playtime_min {
                    facts.push(format!("{} played", game_info::playtime_text(m)));
                }
                if let Some(a) = ago_text(now, g.last_played) {
                    facts.push(format!("last played {a}"));
                }
            }
            let line = facts.join("   \u{b7}   ");
            let fcap = cap * 0.62;
            let fw = if line.is_empty() { 0.0 } else { p.width(&line, fcap, W::Regular) };
            let pw = nw.max(fw) + cap * 1.6;
            let ph = cap * if line.is_empty() { 2.0 } else { 3.5 };
            p.layer.fill_rrect((cx - pw / 2.0, ty - cap * 1.4, pw, ph), cap * 0.6, &|_| (0, 0, 0), (120.0 * alpha) as u8);
            p.text(cx - nw / 2.0, ty, &name, cap, W::Medium, ink, (255.0 * alpha) as u8);
            if !line.is_empty() {
                p.text(cx - fw / 2.0, ty + cap * 1.3, &line, fcap, W::Regular, ink, (190.0 * alpha) as u8);
            }
        }
    }
}

fn txt_r_flow(p: &mut Painter, rx: f32, base: f32, s: &str, cap: f32, w: W, c: Rgb, a: u8) {
    let wd = p.width(s, cap, w);
    p.text(rx - wd, base, s, cap, w, c, a);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cfg(txt: &str) -> anyhow::Result<Settings> {
        parse(&ConfigFile::parse(txt).unwrap())
    }

    #[test]
    fn options_parse_and_are_checked() {
        assert_eq!(parse_cfg("").unwrap(), Settings::default());
        let s = parse_cfg("steam_interval = 10\nsteam_speed = 1200\nsteam_side = 2\nsteam_order = name\nsteam_reflection = 0\nsteam_tilt = 30\nsteam_bg = gradient\nsteam_bg_color = #101010, #202040\nsteam_title = false\nsteam_details = false\nsteam_clock = false\nsteam_only_covers = true\nsteam_path = D:\\Steam\n").unwrap();
        assert_eq!((s.interval, s.speed, s.side, s.order, s.reflection, s.tilt), (10, 1200, 2, Order::Name, 0, 30));
        assert_eq!((s.back, s.title, s.details, s.clock, s.only_covers), (Back::Mode(BgMode::Gradient), false, false, false, true));
        assert_eq!(s.path.as_deref(), Some(std::path::Path::new("D:\\Steam")));
        assert_eq!(parse_cfg("steam_bg = blur\n").unwrap().back, Back::Blur);
        for bad in ["steam_interval = 1\n", "steam_side = 0\n", "steam_order = sideways\n", "steam_tilt = 91\n", "steam_bg = image\n", "steam_bg = purple\n", "steam_reflection = 101\n"] {
            assert!(parse_cfg(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_row_rests_then_slides_to_the_next_game_on_time() {
        // 6 s per game, 0.8 s to slide
        assert_eq!(flow_pos(0.0, 6, 800, 5), 0.0);
        assert_eq!(flow_pos(5.9, 6, 800, 5), 0.0);
        let mid = flow_pos(6.4, 6, 800, 5);
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(flow_pos(7.0, 6, 800, 5), 1.0);
        assert_eq!(flow_pos(12.0 + 0.8, 6, 800, 5), 2.0);
        // it never moves with fewer than two games
        assert_eq!(flow_pos(40.0, 6, 800, 1), 0.0);
        assert_eq!(flow_pos(40.0, 6, 800, 0), 0.0);
    }

    #[test]
    fn the_order_follows_the_setting_and_covers_can_be_required() {
        let g = |id: u32, name: &str, last: u64, cover: bool| SteamGame {
            appid: id, name: name.into(), dir: "x".into(), last_played: last, playtime_min: None,
            cover: cover.then(|| crate::steam::Cover { path: "c.jpg".into(), kind: CoverKind::Portrait }),
        };
        let lib = Library { root: "x".into(), games: vec![g(1, "Zelda", 30, true), g(2, "alpha", 20, false), g(3, "Mario", 10, true)] };
        let names = |s: &Settings| ordered(&lib, s).iter().map(|g| g.name.clone()).collect::<Vec<_>>();
        let mut s = Settings::default();
        assert_eq!(names(&s), vec!["Zelda", "alpha", "Mario"]);
        s.order = Order::Name;
        assert_eq!(names(&s), vec!["alpha", "Mario", "Zelda"]);
        s.order = Order::Random;
        let r1 = names(&s);
        assert_eq!(r1, names(&s), "the same mix every time");
        assert_eq!(r1.len(), 3);
        s.only_covers = true;
        assert_eq!(names(&s).len(), 2);
    }

    #[test]
    fn last_play_is_told_in_days_months_and_years() {
        let day = 86_400;
        assert_eq!(ago_text(1_000_000, 0), None);
        assert_eq!(ago_text(1_000_000, 1_000_000 - 100).as_deref(), Some("today"));
        assert_eq!(ago_text(10 * day, 9 * day).as_deref(), Some("yesterday"));
        assert_eq!(ago_text(100 * day, 90 * day).as_deref(), Some("10 days ago"));
        assert_eq!(ago_text(200 * day, 10 * day).as_deref(), Some("6 months ago"));
        assert_eq!(ago_text(800 * day, 10 * day).as_deref(), Some("2 years ago"));
    }

    #[test]
    fn a_cover_in_the_middle_is_flat_and_the_side_ones_lean() {
        let tex = Tex { w: 30, h: 45, px: vec![200; 30 * 45 * 4] };
        let mut flat = Layer::new(400, 300);
        blit_flow(&mut flat, &tex, 200.0, 250.0, 120.0, 180.0, 0.0, 0.0, 1.0, 380.0, 1.0, 0.0, 60.0, 1.0);
        let cols_painted = |l: &Layer, x: usize| (0..300).filter(|y| l.px[(y * 400 + x) * 4 + 3] > 128).count();
        assert!(cols_painted(&flat, 200) > 170 && cols_painted(&flat, 200) <= 182);
        assert_eq!(cols_painted(&flat, 100), 0);
        // a cover turned away: narrower, and its far edge shorter than the near one
        let mut lean = Layer::new(400, 300);
        blit_flow(&mut lean, &tex, 200.0, 250.0, 120.0, 180.0, -120.0, 0.96, -1.0, 380.0, 1.0, 0.0, 60.0, 1.0);
        let (mut left, mut right) = (usize::MAX, 0);
        for x in 0..400 {
            if cols_painted(&lean, x) > 0 {
                left = left.min(x);
                right = right.max(x);
            }
        }
        assert!(right - left < 110, "narrower than the flat 120: {}", right - left);
        assert!(cols_painted(&lean, left + 3) > cols_painted(&lean, right - 3), "the inner (right) edge is farther, so shorter");
        // the reflection paints under the floor and fades
        let mut r = Layer::new(400, 300);
        blit_flow(&mut r, &tex, 200.0, 200.0, 120.0, 150.0, 0.0, 0.0, 1.0, 380.0, 1.0, 0.8, 60.0, 1.0);
        let alpha = |y: usize| r.px[(y * 400 + 200) * 4 + 3];
        assert!(alpha(202) > alpha(240) && alpha(240) > 0 && alpha(270) == 0);
    }

    #[test]
    fn the_screen_draws_with_covers_without_and_without_steam() {
        let dir = std::env::temp_dir().join(format!("trofeo_flow_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut games = Vec::new();
        for i in 0..7u32 {
            let cover = if i % 3 == 0 {
                let f = dir.join(format!("c{i}.png"));
                image::RgbaImage::from_fn(60, 90, |x, y| image::Rgba([(x * 4) as u8, (y * 2) as u8, 80 + i as u8 * 20, 255])).save(&f).unwrap();
                Some(crate::steam::Cover { path: f, kind: CoverKind::Portrait })
            } else if i % 3 == 1 {
                let f = dir.join(format!("w{i}.png"));
                image::RgbaImage::from_fn(92, 43, |x, _| image::Rgba([200, (x * 2) as u8, 60, 255])).save(&f).unwrap();
                Some(crate::steam::Cover { path: f, kind: CoverKind::Wide })
            } else {
                None
            };
            games.push(SteamGame { appid: 100 + i, name: format!("Game number {i} with a rather long title"), dir: "x".into(), last_played: 1_700_000_000 - i as u64 * 90_000, playtime_min: Some(60 * i), cover });
        }
        let lib = Arc::new(Library { root: "x".into(), games });
        let eff = effect::Settings::default();
        for (w, h) in [(1920u32, 462u32), (462, 1920), (800, 600)] {
            for back in [Back::Blur, Back::Mode(BgMode::Color), Back::Mode(BgMode::Gradient)] {
                let s = Settings { back, ..Settings::default() };
                for t in [0.0, 6.4, 7.5, 13.0] {
                    let env = Env { fps: 30.0, ffmpeg: None, effect: &eff, secs: Some(t) };
                    let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(w, h));
                    let mut screen = SteamScreen::new();
                    for _ in 0..8 {
                        screen.draw(&mut fb, (0, 0, w, h), Some(&lib), ColorMode::Default, &s, &env);
                    }
                    assert!(screen.redraws >= 1);
                    assert!(fb.as_bytes().chunks_exact(3).any(|p| p[0] > 40 || p[1] > 40), "something is drawn");
                }
            }
            // no Steam, no games
            let env = Env { fps: 30.0, ffmpeg: None, effect: &eff, secs: Some(0.0) };
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(w, h));
            SteamScreen::new().draw(&mut fb, (0, 0, w, h), None, ColorMode::Default, &Settings::default(), &env);
            let empty = Arc::new(Library::default());
            SteamScreen::new().draw(&mut fb, (0, 0, w, h), Some(&empty), ColorMode::Default, &Settings::default(), &env);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
