//! The screens shown while a game is running (`gaming_layout = ...`). Eight designs share the same
//! readings: the FPS of the game and its history, the load and temperature of GPU, CPU and RAM,
//! and the name and facts of the game (see `game_info.rs`).
//!
//! * `arena`     - the FPS big on the left with its graph, a ring per device on the right;
//! * `zen`       - almost nothing: one huge light number, a ghost graph, a line of readings;
//! * `cockpit`   - a racing dashboard: the FPS on a big gauge, GPU and CPU on two more;
//! * `tactical`  - a HUD: corner brackets, a target ring, segmented bars, a graph strip;
//! * `neon`      - synthwave: a striped sun, a grid to the horizon, glowing numbers;
//! * `telemetry` - a large FPS chart with grid and scale, and the numbers an esports player reads;
//! * `quest`     - a role-playing status screen: HP / MP / STA bars, a level, a golden frame;
//! * `arcade`    - pixel font, scanlines, block bars and a "HI-SCORE".
//!
//! `classic` is the old four-box dashboard (drawn by `main.rs`, not here).
//!
//! Everything is chosen in the config (`gaming_*`): the layout, the readings, the colours, the
//! background, the font, what the header shows. Like the other screens this one is painted on a
//! transparent layer that is redrawn only when something shown changes.

use super::dash::{self, PKind, Panel, Stat};
use super::*;
use crate::game_info::GameInfo;
use std::path::PathBuf;

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// The four boxes of earlier versions.
    Classic,
    Arena,
    Zen,
    Cockpit,
    Tactical,
    Neon,
    Telemetry,
    Quest,
    Arcade,
}

impl Layout {
    pub const NAMES: &'static str = "classic | arena | zen | cockpit | tactical | neon | telemetry | quest | arcade";
    #[allow(dead_code)]
    pub const ALL: [Layout; 9] = [
        Layout::Classic, Layout::Arena, Layout::Zen, Layout::Cockpit, Layout::Tactical,
        Layout::Neon, Layout::Telemetry, Layout::Quest, Layout::Arcade,
    ];

    pub fn parse(s: &str) -> Option<Layout> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "classic" | "boxes" | "old" | "classico" => Layout::Classic,
            "arena" => Layout::Arena,
            "zen" | "minimal" | "minimale" => Layout::Zen,
            "cockpit" | "racing" => Layout::Cockpit,
            "tactical" | "hud" | "tattico" => Layout::Tactical,
            "neon" | "synthwave" | "retrowave" => Layout::Neon,
            "telemetry" | "chart" | "telemetria" => Layout::Telemetry,
            "quest" | "rpg" => Layout::Quest,
            "arcade" | "retro" => Layout::Arcade,
            _ => return None,
        })
    }

    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Layout::Classic => "classic",
            Layout::Arena => "arena",
            Layout::Zen => "zen",
            Layout::Cockpit => "cockpit",
            Layout::Tactical => "tactical",
            Layout::Neon => "neon",
            Layout::Telemetry => "telemetry",
            Layout::Quest => "quest",
            Layout::Arcade => "arcade",
        }
    }

    /// The accent colours of the design (the first is the main one).
    fn palette(self) -> [Rgb; 4] {
        match self {
            Layout::Classic | Layout::Arena => [(0x8B, 0x7C, 0xFF), (0x38, 0xC6, 0xF4), (0xFF, 0x6F, 0xB1), (0xFF, 0xC0, 0x45)],
            Layout::Zen => [(0xE9, 0xEC, 0xF5), (0x9F, 0xB3, 0xD9), (0xC8, 0xB6, 0xFF), (0x8F, 0xE3, 0xC8)],
            Layout::Cockpit => [(0xFF, 0x3B, 0x30), (0xFF, 0x9F, 0x0A), (0xFF, 0xD6, 0x0A), (0x32, 0xD7, 0x4B)],
            Layout::Tactical => [(0x19, 0xE3, 0xFF), (0xA6, 0xFF, 0x4D), (0xFF, 0xB0, 0x20), (0xFF, 0x4D, 0x6D)],
            Layout::Neon => [(0xFF, 0x2B, 0xD6), (0x00, 0xE5, 0xFF), (0xFF, 0xD3, 0x00), (0x8A, 0x5C, 0xFF)],
            Layout::Telemetry => [(0x35, 0xE0, 0xA1), (0x4D, 0xA3, 0xFF), (0xFF, 0xB8, 0x4D), (0xFF, 0x6B, 0x6B)],
            Layout::Quest => [(0xE0, 0x44, 0x3E), (0x3B, 0x82, 0xF6), (0x34, 0xC7, 0x59), (0xF5, 0xC5, 0x42)],
            Layout::Arcade => [(0x33, 0xFF, 0x66), (0xFF, 0xEA, 0x00), (0xFF, 0x33, 0x55), (0x00, 0xE5, 0xFF)],
        }
    }

    /// The background of the design: the two ends of its gradient.
    fn backdrop(self) -> [Rgb; 2] {
        match self {
            Layout::Classic | Layout::Arena => [(0x0B, 0x0E, 0x1C), (0x1A, 0x10, 0x30)],
            Layout::Zen => [(0x07, 0x08, 0x0B), (0x12, 0x14, 0x1A)],
            Layout::Cockpit => [(0x09, 0x09, 0x0C), (0x1C, 0x0A, 0x0A)],
            Layout::Tactical => [(0x03, 0x10, 0x0F), (0x05, 0x10, 0x1C)],
            Layout::Neon => [(0x0E, 0x00, 0x26), (0x3A, 0x08, 0x4E)],
            Layout::Telemetry => [(0x06, 0x0A, 0x11), (0x0B, 0x13, 0x22)],
            Layout::Quest => [(0x0E, 0x12, 0x1B), (0x1E, 0x17, 0x28)],
            Layout::Arcade => [(0x00, 0x00, 0x00), (0x00, 0x00, 0x00)],
        }
    }

    fn default_font(self) -> FontStyle {
        if self == Layout::Arcade { FontStyle::Pixel } else { FontStyle::Smooth }
    }
}

/// A fact about the game shown in the header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detail {
    Session,
    Total,
    Resolution,
    Memory,
    Store,
}

impl Detail {
    pub const NAMES: &'static str = "session, total, resolution, memory, store";

    pub fn parse(s: &str) -> Option<Detail> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "session" | "sessione" | "time" => Detail::Session,
            "total" | "playtime" | "totale" | "hours" => Detail::Total,
            "resolution" | "res" | "risoluzione" => Detail::Resolution,
            "memory" | "ram" | "memoria" => Detail::Memory,
            "store" | "launcher" | "platform" => Detail::Store,
            _ => return None,
        })
    }
}

/// What is behind the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Back {
    /// The gradient of the design itself.
    Design,
    /// One of the program's usual backgrounds.
    Mode(BgMode),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub layout: Layout,
    pub items: Vec<PKind>,
    /// `None` = the design's own colours.
    pub colors: Option<Vec<Rgb>>,
    pub font: Option<FontStyle>,
    pub text: Option<Rgb>,
    pub back: Back,
    pub bg_colors: Vec<Rgb>,
    pub bg_image: Option<PathBuf>,
    pub bg_dim: u32,
    /// The graph of the FPS history (in the designs that have one).
    pub graph: bool,
    /// Seconds of FPS history.
    pub history: u32,
    /// FPS at the end of the gauges and of the chart scale.
    pub fps_max: u32,
    /// Temperature (°C) from which a reading turns red.
    pub warn_temp: u32,
    /// The big number of the GPU and CPU is the load (true) or the temperature.
    pub hero_load: bool,
    /// The name of the game and its facts.
    pub info: bool,
    pub details: Vec<Detail>,
    pub clock: bool,
    /// The cover of the game (Steam), where the design has room for it.
    pub cover: bool,
    /// 0-100: strength of the glow around numbers and lines.
    pub glow: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            layout: Layout::Arena,
            items: vec![PKind::Gpu, PKind::Cpu, PKind::Ram],
            colors: None,
            font: None,
            text: None,
            back: Back::Design,
            bg_colors: Vec::new(),
            bg_image: None,
            bg_dim: 0,
            graph: true,
            history: 60,
            fps_max: 144,
            warn_temp: 80,
            hero_load: true,
            info: true,
            details: vec![Detail::Session, Detail::Total, Detail::Resolution],
            clock: true,
            cover: true,
            glow: 60,
        }
    }
}

fn parse_items(raw: &str) -> anyhow::Result<Vec<PKind>> {
    let mut out = Vec::new();
    for item in raw.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let k = PKind::parse(item).ok_or_else(|| anyhow::anyhow!("gaming_items: '{item}' is not valid ({})", PKind::NAMES))?;
        if matches!(k, PKind::Clock | PKind::Weather | PKind::Music) {
            anyhow::bail!("gaming_items: '{item}' is not a reading of the game screen (use cpu, gpu, ram, fans, net, disk)");
        }
        out.push(k);
    }
    if out.is_empty() {
        anyhow::bail!("gaming_items: the list is empty (cpu, gpu, ram, fans, net, disk)");
    }
    if out.len() > 4 {
        anyhow::bail!("gaming_items: at most 4 entries (you listed {})", out.len());
    }
    Ok(out)
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get("gaming_layout") {
        s.layout = Layout::parse(v).ok_or_else(|| anyhow::anyhow!("gaming_layout: '{v}' not valid ({})", Layout::NAMES))?;
    }
    if let Some(v) = file.get("gaming_items") {
        s.items = parse_items(v)?;
    }
    let num = |key: &str, lo: u32, hi: u32| -> anyhow::Result<Option<u32>> {
        match file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            None => Ok(None),
            Some(v) if (lo..=hi).contains(&v) => Ok(Some(v)),
            Some(v) => anyhow::bail!("{key}: must be between {lo} and {hi} (got {v})"),
        }
    };
    let boolean = |key: &str| file.get_bool(key).map_err(|e| anyhow::anyhow!(e));
    if let Some(v) = file.get("gaming_palette") {
        let name = v.trim();
        if !name.eq_ignore_ascii_case("default") && !name.is_empty() {
            let pal = effect::Palette::parse(name)
                .ok_or_else(|| anyhow::anyhow!("gaming_palette: '{name}' not valid (default | {})", effect::Palette::NAMES))?;
            // the dark parts of a preset would vanish on a dark screen: lift them
            s.colors = Some(
                (0..6)
                    .map(|i| {
                        let c = pal.at(i as f32 / 6.0);
                        let l = dash::luma(c);
                        if l < 130.0 { lerp_rgb(c, (255, 255, 255), (130.0 - l) / (255.0 - l)) } else { c }
                    })
                    .collect(),
            );
        }
    }
    if let Some(v) = file.get("gaming_colors") {
        let list = parse_rgb_list("gaming_colors", v)?;
        if list.is_empty() {
            anyhow::bail!("gaming_colors: needs at least one color, e.g. #FF2BD6, #00E5FF");
        }
        s.colors = Some(list);
    }
    if let Some(v) = file.get("gaming_font") {
        s.font = match v.trim().to_ascii_lowercase().as_str() {
            "auto" | "design" | "" => None,
            "smooth" | "liscio" | "vector" | "ttf" => Some(FontStyle::Smooth),
            "pixel" | "normal" | "bitmap" => Some(FontStyle::Pixel),
            _ => anyhow::bail!("gaming_font: '{v}' not valid (auto | smooth | pixel)"),
        };
    }
    if let Some(v) = file.get("gaming_text_color") {
        s.text = Some(parse_rgb("gaming_text_color", v)?);
    }
    if let Some(v) = file.get("gaming_bg") {
        s.back = match v.trim().to_ascii_lowercase().as_str() {
            "design" | "layout" | "own" | "auto" | "" => Back::Design,
            other => Back::Mode(BgMode::parse(other).ok_or_else(|| anyhow::anyhow!("gaming_bg: '{v}' not valid (design | {})", BgMode::NAMES))?),
        };
    }
    if let Some(v) = file.get("gaming_bg_color") {
        let list = parse_rgb_list("gaming_bg_color", v)?;
        if list.is_empty() {
            anyhow::bail!("gaming_bg_color: needs at least one color, e.g. #10142A or #10142A, #241030");
        }
        s.bg_colors = list;
    }
    if let Some(v) = file.get("gaming_bg_image") {
        s.bg_image = Some(PathBuf::from(v));
    }
    if s.back == Back::Mode(BgMode::Image) && s.bg_image.is_none() {
        anyhow::bail!("gaming_bg = image needs gaming_bg_image = <file>");
    }
    s.bg_dim = num("gaming_bg_dim", 0, 100)?.unwrap_or(0);
    if let Some(v) = boolean("gaming_graph")? {
        s.graph = v;
    }
    if let Some(v) = num("gaming_history", 20, 600)? {
        s.history = v;
    }
    if let Some(v) = num("gaming_fps_max", 30, 1000)? {
        s.fps_max = v;
    }
    if let Some(v) = num("gaming_warn_temp", 50, 110)? {
        s.warn_temp = v;
    }
    if let Some(v) = file.get("gaming_hero") {
        s.hero_load = match v.trim().to_ascii_lowercase().as_str() {
            "load" | "usage" | "carico" | "percent" => true,
            "temp" | "temperature" | "temperatura" => false,
            _ => anyhow::bail!("gaming_hero: '{v}' not valid (load | temp)"),
        };
    }
    if let Some(v) = boolean("gaming_info")? {
        s.info = v;
    }
    if let Some(v) = file.get("gaming_details") {
        let mut out = Vec::new();
        for item in v.split(',') {
            let item = item.trim();
            if item.is_empty() || item.eq_ignore_ascii_case("none") {
                continue;
            }
            out.push(Detail::parse(item).ok_or_else(|| anyhow::anyhow!("gaming_details: '{item}' not valid ({})", Detail::NAMES))?);
        }
        s.details = out;
    }
    if let Some(v) = boolean("gaming_clock")? {
        s.clock = v;
    }
    if let Some(v) = boolean("gaming_cover")? {
        s.cover = v;
    }
    if let Some(v) = num("gaming_glow", 0, 100)? {
        s.glow = v;
    }
    Ok(s)
}

impl Settings {
    fn font(&self) -> FontStyle {
        self.font.unwrap_or_else(|| self.layout.default_font())
    }

    /// The accent colours in use (never empty).
    fn accents(&self) -> Vec<Rgb> {
        match &self.colors {
            Some(c) if !c.is_empty() => c.clone(),
            _ => self.layout.palette().to_vec(),
        }
    }

    /// The tiles-style settings that paint the background.
    fn background(&self) -> tiles::Settings {
        let (mode, colors) = match self.back {
            Back::Design => (BgMode::Gradient, self.layout.backdrop().to_vec()),
            Back::Mode(m) => (m, if self.bg_colors.is_empty() { self.layout.backdrop().to_vec() } else { self.bg_colors.clone() }),
        };
        // the arcade is a plain black screen
        let colors = if self.back == Back::Design && self.layout == Layout::Arcade { vec![(0, 0, 0)] } else { colors };
        let mode = if self.back == Back::Design && self.layout == Layout::Arcade { BgMode::Color } else { mode };
        tiles::Settings { bg: mode, bg_colors: colors, bg_image: self.bg_image.clone(), bg_dim: self.bg_dim, ..tiles::Settings::default() }
    }
}

// -------------------------------------------------------------------- scene

pub(super) type R = (f32, f32, f32, f32);

/// The FPS the screen shows.
#[derive(Clone, Debug, PartialEq, Default)]
struct FpsView {
    now: Option<i32>,
    avg: Option<i32>,
    min: Option<i32>,
    max: Option<i32>,
}

impl FpsView {
    fn value(&self) -> String {
        self.now.filter(|f| *f > 0).map_or_else(|| "--".to_string(), |f| f.to_string())
    }

    /// "6.9"
    fn frametime(&self) -> Option<String> {
        self.now.filter(|f| *f > 0).map(|f| format!("{:.1}", 1000.0 / f as f32))
    }

    fn frac(&self, max: u32) -> f32 {
        self.now.map_or(0.0, |f| (f as f32 / max.max(1) as f32).clamp(0.0, 1.0))
    }
}

/// Everything a design needs to draw.
struct Scene<'a> {
    s: &'a Settings,
    font: FontStyle,
    ink: Rgb,
    col: Vec<Rgb>,
    items: Vec<Panel>,
    fps: FpsView,
    hist: Vec<f32>,
    game: Option<&'a GameInfo>,
    cover: Option<&'a cover::Tex>,
    clock: String,
    /// What to say when there is no FPS reading (e.g. the program is not run as administrator).
    hint: String,
    /// Changes every second (blinking things).
    beat: bool,
}

impl Scene<'_> {
    fn c(&self, i: usize) -> Rgb {
        self.col[i % self.col.len().max(1)]
    }

    fn case(&self, s: &str) -> String {
        if self.font == FontStyle::Pixel { s.to_uppercase() } else { s.to_string() }
    }

    #[allow(dead_code)]
    fn pixel(&self) -> bool {
        self.font == FontStyle::Pixel
    }

    /// The small readings of a panel (the FPS one is already the big number).
    fn stats<'p>(&self, panel: &'p Panel) -> Vec<&'p Stat> {
        panel.stats.iter().filter(|s| s.label != "FPS").collect()
    }

    /// Red when a temperature is above the limit.
    fn warm(&self, st: &Stat, base: Rgb) -> Rgb {
        let is_temp = st.label.contains("TEMP") || st.label.contains("HOT") || st.label == "SSD" || st.label == "BOARD";
        match st.value.trim().parse::<f32>() {
            Ok(v) if is_temp && v >= self.s.warn_temp as f32 => (255, 90, 90),
            _ => base,
        }
    }

    /// The colour of the big number of a panel: red when it is a temperature above the limit.
    fn hero_ink(&self, panel: &Panel, base: Rgb) -> Rgb {
        let is_temp = panel.unit.contains('C');
        match panel.hero.trim().parse::<f32>() {
            Ok(v) if is_temp && v >= self.s.warn_temp as f32 => (255, 90, 90),
            _ => base,
        }
    }

    /// The facts of the game for the header.
    fn details(&self) -> Vec<String> {
        let Some(g) = self.game else { return Vec::new() };
        let mut out = Vec::new();
        for d in &self.s.details {
            let t = match d {
                Detail::Session => Some(format!("Session {}", g.session_text())),
                Detail::Total => g.playtime_text().map(|t| format!("{t} played")),
                Detail::Resolution => g.resolution_text(),
                Detail::Memory => g.mem_mb.map(|m| format!("{:.1} GB RAM", m as f32 / 1024.0)),
                Detail::Store => g.store.label().map(str::to_string),
            };
            if let Some(t) = t {
                out.push(self.case(&t));
            }
        }
        out
    }

    fn name(&self) -> Option<String> {
        self.s.info.then(|| self.game.map(|g| g.name.clone())).flatten()
    }
}

// ------------------------------------------------------------- drawing helpers

pub(super) fn txt(p: &mut Painter, x: f32, base: f32, s: &str, cap: f32, w: W, c: Rgb, a: u8) -> f32 {
    p.text(x, base, s, cap, w, c, a)
}

pub(super) fn txt_c(p: &mut Painter, cx: f32, base: f32, s: &str, cap: f32, w: W, c: Rgb, a: u8) -> f32 {
    let wd = p.width(s, cap, w);
    p.text(cx - wd / 2.0, base, s, cap, w, c, a)
}

pub(super) fn txt_r(p: &mut Painter, rx: f32, base: f32, s: &str, cap: f32, w: W, c: Rgb, a: u8) -> f32 {
    let wd = p.width(s, cap, w);
    p.text(rx - wd, base, s, cap, w, c, a)
}

/// Text with a soft halo (`strength` 0-100).
pub(super) fn glow_txt(p: &mut Painter, x: f32, base: f32, s: &str, cap: f32, w: W, c: Rgb, strength: u32) {
    if strength > 0 && p.font == FontStyle::Smooth {
        let g = strength as f32 / 100.0;
        let d = (cap * 0.03).max(1.0);
        for (k, a) in [(1.0f32, 0.16f32), (2.2, 0.10), (3.6, 0.05)] {
            let alpha = (255.0 * a * g) as u8;
            for i in 0..8 {
                let ang = i as f32 * std::f32::consts::FRAC_PI_4;
                p.text(x + ang.cos() * d * k, base + ang.sin() * d * k, s, cap, w, c, alpha);
            }
        }
    } else if strength > 0 {
        // the pixel font: a dim copy one pixel step away on each side
        let step = Painter::pixel_scale(cap) as f32;
        let alpha = (50.0 * strength as f32 / 100.0) as u8;
        for (dx, dy) in [(-step, 0.0), (step, 0.0), (0.0, -step), (0.0, step)] {
            p.text(x + dx, base + dy, s, cap, w, c, alpha);
        }
    }
    p.text(x, base, s, cap, w, c, 255);
}

pub(super) fn glow_txt_c(p: &mut Painter, cx: f32, base: f32, s: &str, cap: f32, w: W, c: Rgb, strength: u32) {
    let wd = p.width(s, cap, w);
    glow_txt(p, cx - wd / 2.0, base, s, cap, w, c, strength);
}

/// Cuts `s` with dots so it fits `max_w`.
pub(super) fn fit_clip(p: &Painter, s: &str, cap: f32, max_w: f32, w: W) -> String {
    dash::clip(p, s, cap, max_w, w)
}

impl Layer {
    /// A straight line with round ends.
    pub(super) fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, thick: f32, rgb: Rgb, alpha: u8) {
        let half = thick / 2.0;
        let (bx0, bx1) = ((x0.min(x1) - half - 1.0).floor() as i32, (x0.max(x1) + half + 1.0).ceil() as i32);
        let (by0, by1) = ((y0.min(y1) - half - 1.0).floor() as i32, (y0.max(y1) + half + 1.0).ceil() as i32);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = (dx * dx + dy * dy).max(0.0001);
        for py in by0..by1 {
            for px in bx0..bx1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let t = (((fx - x0) * dx + (fy - y0) * dy) / len2).clamp(0.0, 1.0);
                let d = (fx - (x0 + dx * t)).hypot(fy - (y0 + dy * t));
                let cov = (half + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.over(px, py, rgb, (alpha as f32 * cov).round() as u8);
                }
            }
        }
    }

    /// An arc of a circle. Angles are in degrees, 0 at the top and growing clockwise; the arc runs
    /// from `start` over `sweep` degrees and is filled up to `frac` of it. `color(t)`: t = 0-1
    /// along the whole sweep.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn gauge(&mut self, cx: f32, cy: f32, r_out: f32, thick: f32, start: f32, sweep: f32, frac: f32, color: &dyn Fn(f32) -> Rgb, alpha: u8) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.0 || alpha == 0 || r_out <= 1.0 {
            return;
        }
        let tau = std::f32::consts::TAU;
        let (a0, sw_all) = (start.to_radians(), sweep.to_radians().min(tau));
        let sw = sw_all * frac;
        let r_in = (r_out - thick).max(0.0);
        let r_mid = r_out - thick / 2.0;
        let cap = thick / 2.0;
        let ends = [(a0, true), (a0 + sw, false)].map(|(a, _)| (cx + r_mid * a.sin(), cy - r_mid * a.cos()));
        let (x0, x1) = ((cx - r_out - 1.0).floor() as i32, (cx + r_out + 1.0).ceil() as i32);
        let (y0, y1) = ((cy - r_out - 1.0).floor() as i32, (cy + r_out + 1.0).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let (dx, dy) = (fx - cx, fy - cy);
                let d = dx.hypot(dy);
                if d > r_out + 1.0 || d < r_in - 1.0 {
                    continue;
                }
                let radial = (r_out + 0.5 - d).clamp(0.0, 1.0) * (d - r_in + 0.5).clamp(0.0, 1.0);
                let mut ang = dx.atan2(-dy);
                if ang < 0.0 {
                    ang += tau;
                }
                let mut rel = (ang - a0).rem_euclid(tau);
                // the second half of the gap counts as "before the start"
                if rel > sw + (tau - sw) / 2.0 {
                    rel -= tau;
                }
                let body = ((rel * d + 0.5).clamp(0.0, 1.0)) * (((sw - rel) * d + 0.5).clamp(0.0, 1.0));
                let mut cov = radial * body;
                for (qx, qy) in ends {
                    cov = cov.max((cap + 0.5 - (fx - qx).hypot(fy - qy)).clamp(0.0, 1.0));
                }
                if cov > 0.0 {
                    let t = if sw_all > 0.0 { (rel.clamp(0.0, sw)) / sw_all } else { 0.0 };
                    self.over(px, py, color(t), (alpha as f32 * cov).round() as u8);
                }
            }
        }
    }

    /// Corner brackets around a rectangle.
    pub(super) fn brackets(&mut self, r: R, len: f32, thick: f32, rgb: Rgb, alpha: u8) {
        let (x, y, w, h) = r;
        let len = len.min(w / 2.0).min(h / 2.0);
        for (cx, cy, sx, sy) in [(x, y, 1.0, 1.0), (x + w, y, -1.0, 1.0), (x, y + h, 1.0, -1.0), (x + w, y + h, -1.0, -1.0)] {
            self.line(cx, cy, cx + sx * len, cy, thick, rgb, alpha);
            self.line(cx, cy, cx, cy + sy * len, thick, rgb, alpha);
        }
    }

    /// A vertical gradient filling a rectangle (whole pixels).
    pub(super) fn vgrad(&mut self, r: R, top: Rgb, bottom: Rgb, alpha: u8) {
        let (x, y, w, h) = r;
        if w < 1.0 || h < 1.0 {
            return;
        }
        for py in y.floor() as i32..(y + h).ceil() as i32 {
            let t = ((py as f32 + 0.5 - y) / h).clamp(0.0, 1.0);
            self.span_over(py, x.floor() as i32, (x + w).ceil() as i32, lerp_rgb(top, bottom, t), alpha);
        }
    }

    /// A polyline over a fixed vertical scale `lo..hi`, with the area under it fading out.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn chart(&mut self, r: R, data: &[f32], lo: f32, hi: f32, rgb: Rgb, thick: f32, fill_alpha: u8, end_dot: bool) {
        let (x, y, w, h) = r;
        if data.len() < 2 || w < 6.0 || h < 6.0 || hi <= lo {
            return;
        }
        let n = data.len().min((w / 2.0) as usize).max(2);
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let f = i as f32 / (n - 1) as f32;
                let src = f * (data.len() - 1) as f32;
                let (a, b) = (src.floor() as usize, (src.floor() as usize + 1).min(data.len() - 1));
                let v = data[a] + (data[b] - data[a]) * (src - a as f32);
                (x + f * w, y + h - ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * h)
            })
            .collect();
        if fill_alpha > 0 {
            let mut seg = 0;
            for px in x.floor() as i32..(x + w).ceil() as i32 {
                let fx = px as f32 + 0.5;
                while seg + 2 < pts.len() && pts[seg + 1].0 < fx {
                    seg += 1;
                }
                let (a, b) = (pts[seg], pts[seg + 1]);
                let t = ((fx - a.0) / (b.0 - a.0).max(0.001)).clamp(0.0, 1.0);
                let ly = a.1 + (b.1 - a.1) * t;
                let depth = (y + h - ly).max(1.0);
                for py in ly.floor() as i32..(y + h).ceil() as i32 {
                    let top = ((py as f32 + 1.0) - ly).clamp(0.0, 1.0);
                    let fade = 1.0 - ((py as f32 + 0.5 - ly) / depth).clamp(0.0, 1.0);
                    self.over(px, py, rgb, (fill_alpha as f32 * top * fade * fade).round() as u8);
                }
            }
        }
        for pair in pts.windows(2) {
            self.line(pair[0].0, pair[0].1, pair[1].0, pair[1].1, thick, rgb, 255);
        }
        if end_dot {
            if let Some(last) = pts.last() {
                self.dot(last.0, last.1, thick * 2.2 + 1.0, rgb, 60);
                self.dot(last.0, last.1, thick * 1.2 + 0.8, rgb, 255);
            }
        }
    }
}

/// A rounded bar: a faint track and a fill running from colour `a` to colour `b`.
pub(super) fn bar(p: &mut Painter, r: R, frac: f32, a: Rgb, b: Rgb, track: (Rgb, u8)) {
    let (x, y, w, h) = r;
    if w < 2.0 || h < 1.0 {
        return;
    }
    let rad = h / 2.0;
    p.layer.fill_rrect((x, y, w, h), rad, &move |_| track.0, track.1);
    let fw = (w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h.min(w) } else { 0.0 });
    if fw >= 1.0 {
        p.layer.fill_rrect((x, y, fw, h), rad, &move |t| lerp_rgb(a, b, t), 255);
    }
}

/// A bar made of blocks: `n` of them, the first ones lit up to `frac`.
#[allow(clippy::too_many_arguments)]
pub(super) fn blocks(p: &mut Painter, r: R, frac: f32, n: usize, gap: f32, color: &dyn Fn(f32) -> Rgb, round: f32, off: (Rgb, u8)) {
    let (x, y, w, h) = r;
    if n == 0 || w < 2.0 {
        return;
    }
    let bw = (w - gap * (n as f32 - 1.0)) / n as f32;
    let lit = (frac.clamp(0.0, 1.0) * n as f32).round() as usize;
    for i in 0..n {
        let bx = x + i as f32 * (bw + gap);
        let t = (i as f32 + 0.5) / n as f32;
        if i < lit {
            p.layer.fill_rrect((bx, y, bw, h), round, &move |_| color(t), 255);
        } else {
            p.layer.fill_rrect((bx, y, bw, h), round, &move |_| off.0, off.1);
        }
    }
}

/// A hairline.
pub(super) fn hair(p: &mut Painter, r: R, rgb: Rgb, alpha: u8) {
    p.layer.fill_rrect((r.0, r.1, r.2.max(1.0), r.3.max(1.0)), 0.0, &move |_| rgb, alpha);
}

pub(super) fn is_tall((_, _, w, h): R) -> bool {
    h > w * 1.1
}

/// The pieces of the header line: name, facts, clock. Returns the x where the facts stopped.
fn header_line(p: &mut Painter, sc: &Scene, x: f32, base: f32, w: f32, cap: f32, name_ink: Rgb) {
    let ink = sc.ink;
    let mut right = x + w;
    if sc.s.clock {
        let tw = txt_r(p, right, base, &sc.clock, cap, W::Medium, ink, 235);
        right -= tw + cap * 1.4;
    }
    let Some(name) = sc.name() else { return };
    let mut cx = x;
    let name = fit_clip(p, &name, cap * 1.12, (right - x) * 0.62, W::Medium);
    cx += txt(p, cx, base, &name, cap * 1.12, W::Medium, name_ink, 255) + cap * 1.4;
    for d in sc.details() {
        let dw = p.width(&d, cap * 0.86, W::Regular);
        if cx + dw > right {
            break;
        }
        txt(p, cx, base, &d, cap * 0.86, W::Regular, ink, 150);
        cx += dw + cap * 1.4;
    }
}

/// The "no reading" hint under the FPS number.
fn fps_foot(sc: &Scene) -> String {
    match sc.fps.frametime() {
        Some(ms) => format!("{ms} ms"),
        None => sc.hint.clone(),
    }
}

fn mini_stat(p: &mut Painter, x: f32, top: f32, label: &str, value: &str, cap: f32, ink: Rgb, la: u8) -> f32 {
    let lc = (cap * 0.62).max(7.0);
    let lw = txt(p, x, top + lc, label, lc, W::Regular, ink, la);
    let vw = txt(p, x, top + lc + cap * 1.4, value, cap, W::Medium, ink, 255);
    vw.max(lw)
}

/// "H A D E S"
pub(super) fn spaced(s: &str) -> String {
    s.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

fn opt_i(v: Option<i32>) -> String {
    v.filter(|f| *f > 0).map_or_else(|| "--".to_string(), |f| f.to_string())
}

/// The vertical scale of an FPS graph: from 0 up to a bit above the highest value, but never
/// less than a quarter of `fps_max`.
fn fps_scale(sc: &Scene) -> (f32, f32) {
    let top = sc.hist.iter().copied().fold(0.0f32, f32::max).max(sc.fps_max() * 0.25);
    (0.0, top * 1.12)
}

impl Scene<'_> {
    fn fps_max(&self) -> f32 {
        self.s.fps_max as f32
    }

    fn glow(&self) -> u32 {
        self.s.glow
    }
}

// ---------------------------------------------------------------------- arena

fn draw_arena(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let rad = (h.min(w) * 0.05).clamp(10.0, 24.0);
    p.layer.fill_rrect_solid(r, rad, (4, 5, 14), 96);
    p.layer.stroke_rrect(r, rad, 1.2, (255, 255, 255), 26);
    let pad = (h.min(w) * 0.045).clamp(10.0, 26.0);
    let hcap = (h.min(w) * 0.04).clamp(11.0, 22.0);
    let has_head = sc.s.clock || sc.name().is_some();
    let thumb = sc.cover.filter(|_| sc.s.cover && sc.name().is_some());
    let thumb_h = if thumb.is_some() { hcap * 3.0 } else { 0.0 };
    let head_h = if has_head { (hcap * 2.6).max(thumb_h) } else { 0.0 };
    if has_head {
        let mut tx = 0.0;
        if let Some(t) = thumb {
            let tw = thumb_h * t.w as f32 / t.h as f32;
            blit_cover(p.layer, t, (x + pad, y + pad - hcap * 0.4, tw, thumb_h), 4.0);
            tx = tw + hcap * 0.9;
        }
        header_line(p, sc, x + pad + tx, y + pad + hcap, w - 2.0 * pad - tx, hcap, sc.ink);
        let (a, b) = (sc.c(0), sc.c(1));
        p.layer.fill_rrect((x + pad, y + pad + head_h - hcap * 0.5, w - 2.0 * pad, 2.0), 1.0, &move |t| lerp_rgb(a, b, t), 150);
    }
    let gap = pad * 0.8;
    let top = y + pad + head_h + if has_head { pad * 0.4 } else { 0.0 };
    let body = (x + pad, top, w - 2.0 * pad, y + h - pad - top);
    let (hero, items) = if tall {
        let hh = body.3 * 0.32;
        ((body.0, body.1, body.2, hh), (body.0, body.1 + hh + gap, body.2, body.3 - hh - gap))
    } else {
        let hw = body.2 * 0.37;
        ((body.0, body.1, hw, body.3), (body.0 + hw + gap, body.1, body.2 - hw - gap, body.3))
    };
    arena_hero(p, sc, hero, tall);
    let n = sc.items.len().max(1);
    let (line, ..) = (sc.ink, 0);
    for (i, panel) in sc.items.iter().enumerate() {
        let cell = if tall {
            let ch = (items.3 - gap * (n as f32 - 1.0)) / n as f32;
            (items.0, items.1 + i as f32 * (ch + gap), items.2, ch)
        } else {
            let cw = (items.2 - gap * (n as f32 - 1.0)) / n as f32;
            (items.0 + i as f32 * (cw + gap), items.1, cw, items.3)
        };
        if i > 0 {
            if tall {
                hair(p, (cell.0, cell.1 - gap / 2.0, cell.2, 1.0), line, 30);
            } else {
                hair(p, (cell.0 - gap / 2.0, cell.1 + cell.3 * 0.06, 1.0, cell.3 * 0.88), line, 30);
            }
        }
        arena_item(p, sc, panel, i, cell, tall);
    }
}

fn arena_hero(p: &mut Painter, sc: &Scene, r: R, tall: bool) {
    let (x, y, w, h) = r;
    let lcap = (h.min(w) * 0.075).clamp(10.0, 20.0);
    let c0 = sc.c(0);
    let fw = txt(p, x, y + lcap, "FPS", lcap, W::Medium, c0, 255);
    let foot = fps_foot(sc);
    txt(p, x + fw + lcap * 0.9, y + lcap, &foot, lcap * 0.92, W::Regular, sc.ink, 150);
    let graph_h = if sc.s.graph && !sc.hist.is_empty() { h * if tall { 0.26 } else { 0.27 } } else { 0.0 };
    let stats_h = lcap * 3.5;
    let num_h = (h - lcap * 1.6 - graph_h - stats_h - if graph_h > 0.0 { lcap * 0.8 } else { 0.0 }).max(20.0);
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", num_h * 0.92, w, W::Light);
    let base = y + lcap * 1.6 + (num_h + cap) / 2.0;
    glow_txt(p, x, base, &val, cap, W::Light, sc.ink, sc.glow() / 2);
    // average, lowest, highest
    let cell = w / 3.0;
    let sy = base + lcap * 0.9;
    let scap = lcap * 1.15;
    for (i, (label, v)) in [("AVG", sc.fps.avg), ("MIN", sc.fps.min), ("MAX", sc.fps.max)].into_iter().enumerate() {
        mini_stat(p, x + i as f32 * cell, sy, label, &opt_i(v), scap, sc.ink, 130);
    }
    if graph_h > 0.0 {
        let (lo, hi) = fps_scale(sc);
        let g = (x, y + h - graph_h, w, graph_h);
        p.layer.chart(g, &sc.hist, lo, hi, c0, 2.0, 80, true);
    }
}

fn arena_item(p: &mut Painter, sc: &Scene, panel: &Panel, i: usize, r: R, tall: bool) {
    let (x, y, w, h) = r;
    let (a, b) = (sc.c(i + 1), sc.c(i + 2));
    let tcap = (h.min(w) * 0.07).clamp(10.0, 19.0);
    let stats = sc.stats(panel);
    let frac = panel.frac.unwrap_or(0.0);
    let ink = sc.ink;
    let hero_ink = sc.hero_ink(panel, ink);
    if tall {
        // ring on the left, name and readings on the right
        let rr = (h * 0.44).min(w * 0.2);
        let (cx, cy) = (x + rr + 4.0, y + h / 2.0);
        let th = rr * 0.17;
        p.layer.arc(cx, cy, rr, th, 1.0, &|_| ink, 28);
        p.layer.arc(cx, cy, rr, th, frac, &move |t| lerp_rgb(a, b, t), 255);
        let cap = dash::fit_vu(p, &panel.hero, &panel.unit, rr * 0.75, rr * 1.45, W::Medium);
        let wv = dash::vu_width(p, &panel.hero, &panel.unit, cap, W::Medium);
        dash::value_unit(p, cx - wv / 2.0, cy + cap * 0.36, &panel.hero, &panel.unit, cap, W::Medium, hero_ink, 255, 150);
        let tx = cx + rr + w * 0.06;
        txt(p, tx, y + h * 0.5 - tcap * 1.4, &panel.title, tcap * 1.2, W::Medium, ink, 200);
        let mut ly = y + h * 0.5 + tcap * 0.6;
        for st in stats.iter().take(3) {
            txt(p, tx, ly, &st.label, tcap * 0.85, W::Regular, ink, 130);
            let vw = p.width(&format!("{} {}", st.value, st.unit), tcap, W::Medium);
            txt(p, x + w - vw, ly, &format!("{} {}", st.value, st.unit), tcap, W::Medium, sc.warm(st, ink), 255);
            ly += tcap * 1.9;
        }
        return;
    }
    txt(p, x + 2.0, y + tcap, &panel.title, tcap * 1.15, W::Medium, ink, 200);
    let stats_h = tcap * 1.9 * stats.len().min(3) as f32;
    let rr = ((w * 0.5).min((h - tcap * 2.6 - stats_h) / 2.0) - 2.0).max(18.0);
    let (cx, cy) = (x + w / 2.0, y + tcap * 2.0 + rr + 2.0);
    let th = rr * 0.16;
    p.layer.arc(cx, cy, rr, th, 1.0, &|_| ink, 28);
    p.layer.arc(cx, cy, rr, th, frac, &move |t| lerp_rgb(a, b, t), 255);
    let cap = dash::fit_vu(p, &panel.hero, &panel.unit, rr * 0.8, rr * 1.4, W::Medium);
    let wv = dash::vu_width(p, &panel.hero, &panel.unit, cap, W::Medium);
    dash::value_unit(p, cx - wv / 2.0, cy + cap * 0.36, &panel.hero, &panel.unit, cap, W::Medium, hero_ink, 255, 150);
    let mut ly = cy + rr + tcap * 1.9;
    for st in stats.iter().take(3) {
        txt(p, x + w * 0.1, ly, &st.label, tcap * 0.85, W::Regular, ink, 130);
        let s = format!("{} {}", st.value, st.unit);
        txt_r(p, x + w * 0.9, ly, &s, tcap, W::Medium, sc.warm(st, ink), 255);
        ly += tcap * 1.9;
    }
}

// ------------------------------------------------------------------------ zen

fn draw_zen(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let ink = sc.ink;
    let pad = (h.min(w) * 0.05).clamp(10.0, 30.0);
    // a ghost of the history behind everything
    if sc.s.graph && sc.hist.len() > 1 {
        let g = if tall { (x, y + h * 0.18, w, h * 0.3) } else { (x, y + h * 0.28, w, h * 0.56) };
        p.layer.spark(g, &sc.hist, sc.fps_max() * 0.25, sc.c(0), 1.6, 26, 60, false);
    }
    let hcap = (h.min(w) * 0.034).clamp(10.0, 18.0);
    if let Some(name) = sc.name() {
        let n = fit_clip(p, &sc.case(&spaced(&name.to_uppercase())), hcap, w - 2.0 * pad - if sc.s.clock { w * 0.2 } else { 0.0 }, W::Regular);
        txt_c(p, x + w / 2.0, y + pad + hcap, &n, hcap, W::Regular, ink, 190);
    }
    if sc.s.clock {
        txt_r(p, x + w - pad, y + pad + hcap, &sc.clock, hcap, W::Regular, ink, 150);
    }
    let det = sc.details().join("   \u{b7}   ");
    if !det.is_empty() && sc.name().is_some() {
        let d = fit_clip(p, &det, hcap * 0.8, w - 2.0 * pad, W::Regular);
        txt_c(p, x + w / 2.0, y + pad + hcap * 2.7, &d, hcap * 0.8, W::Regular, ink, 100);
    }
    let rows_h = if tall { sc.items.len() as f32 * h * 0.075 } else { h * 0.2 };
    let zone_top = y + pad + hcap * 3.6;
    let zone_h = (y + h - pad - rows_h - zone_top - hcap * 3.6).max(40.0);
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", (zone_h * 0.9).min(w * 0.4), w * 0.8, W::Light);
    let base = zone_top + (zone_h - cap) / 2.0 + cap;
    glow_txt_c(p, x + w / 2.0, base, &val, cap, W::Light, ink, sc.glow() / 3);
    let ccap = (cap * 0.1).clamp(10.0, 22.0);
    let foot = format!("FPS   \u{b7}   {}", fps_foot(sc));
    txt_c(p, x + w / 2.0, base + ccap * 2.2, &foot, ccap, W::Regular, ink, 160);
    // readings, in a line (or a column)
    let n = sc.items.len().max(1);
    for (i, panel) in sc.items.iter().enumerate() {
        let acc = sc.c(i);
        let (cx, top, cw) = if tall {
            (x + w / 2.0, y + h - pad - rows_h + i as f32 * (rows_h / n as f32), w * 0.7)
        } else {
            let cw = (w - 2.0 * pad) / n as f32;
            (x + pad + cw * (i as f32 + 0.5), y + h - pad - rows_h, cw * 0.6)
        };
        if i > 0 && !tall {
            hair(p, (cx - (w - 2.0 * pad) / n as f32 / 2.0, top + 2.0, 1.0, rows_h * 0.8), ink, 26);
        }
        let tcap = (rows_h * 0.15).clamp(9.0, 17.0);
        let vcap = (rows_h * if tall { 0.42 } else { 0.34 }).clamp(14.0, 40.0);
        let vcap = dash::fit_vu(p, &panel.hero, &panel.unit, vcap, cw, W::Light);
        if tall {
            txt(p, cx - cw / 2.0, top + rows_h / n as f32 * 0.62, &panel.title, tcap, W::Regular, ink, 150);
            let wv = dash::vu_width(p, &panel.hero, &panel.unit, vcap, W::Light);
            dash::value_unit(p, cx + cw / 2.0 - wv, top + rows_h / n as f32 * 0.62, &panel.hero, &panel.unit, vcap, W::Light, sc.hero_ink(panel, ink), 255, 150);
        } else {
            txt_c(p, cx, top + tcap, &panel.title, tcap, W::Regular, ink, 150);
            let wv = dash::vu_width(p, &panel.hero, &panel.unit, vcap, W::Light);
            dash::value_unit(p, cx - wv / 2.0, top + tcap * 1.6 + vcap, &panel.hero, &panel.unit, vcap, W::Light, sc.hero_ink(panel, ink), 255, 150);
            bar(p, (cx - cw * 0.4, top + tcap * 1.6 + vcap + tcap * 1.0, cw * 0.8, 3.0), panel.frac.unwrap_or(0.0), acc, acc, (ink, 28));
        }
    }
}

// -------------------------------------------------------------------- cockpit

/// A gauge open at the bottom: 270 degrees from the lower left to the lower right.
const G_START: f32 = 225.0;
const G_SWEEP: f32 = 270.0;

fn gauge_ticks(p: &mut Painter, cx: f32, cy: f32, r: f32, len: f32, n: usize, ink: Rgb) {
    for i in 0..=n {
        let a = (G_START + G_SWEEP * i as f32 / n as f32).to_radians();
        let major = i % 5 == 0;
        let l = if major { len } else { len * 0.55 };
        let (s, c) = (a.sin(), -a.cos());
        p.layer.line(cx + s * (r - l), cy + c * (r - l), cx + s * r, cy + c * r, if major { 2.0 } else { 1.2 }, ink, if major { 120 } else { 60 });
    }
}

fn draw_cockpit(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let ink = sc.ink;
    let pad = (h.min(w) * 0.04).clamp(8.0, 24.0);
    let hcap = (h.min(w) * 0.036).clamp(10.0, 20.0);
    header_line(p, sc, x + pad, y + pad + hcap, w - 2.0 * pad, hcap, sc.c(0));
    // the big gauge
    let big = if tall { (w * 0.46).min(h * 0.17) } else { (h * 0.44).min(w * 0.2) };
    let (cx, cy) = if tall { (x + w / 2.0, y + h * 0.045 + big * 1.25) } else { (x + w / 2.0, y + h * 0.54) };
    let th = big * 0.12;
    let (c0, c1) = (sc.c(0), sc.c(1));
    p.layer.gauge(cx, cy, big, th, G_START, G_SWEEP, 1.0, &|_| ink, 24);
    gauge_ticks(p, cx, cy, big - th - 6.0, big * 0.07, 20, ink);
    let f = sc.fps.frac(sc.s.fps_max);
    if sc.glow() > 0 {
        let g = sc.glow() as f32 / 100.0;
        p.layer.gauge(cx, cy, big + th * 0.7, th * 2.4, G_START, G_SWEEP, f, &move |t| lerp_rgb(c0, c1, t), (40.0 * g) as u8);
        p.layer.gauge(cx, cy, big + th * 0.2, th * 1.5, G_START, G_SWEEP, f, &move |t| lerp_rgb(c0, c1, t), (60.0 * g) as u8);
    }
    p.layer.gauge(cx, cy, big, th, G_START, G_SWEEP, f, &move |t| lerp_rgb(c0, c1, t), 255);
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", big * 0.74, big * 1.25, W::Medium);
    glow_txt_c(p, cx, cy + cap * 0.38, &val, cap, W::Medium, ink, sc.glow() / 2);
    let lcap = (big * 0.1).clamp(9.0, 20.0);
    txt_c(p, cx, cy - cap * 0.62 - lcap * 0.7, "FPS", lcap, W::Medium, c0, 255);
    txt_c(p, cx, cy + big * 0.82, &fps_foot(sc), lcap * 0.95, W::Regular, ink, 160);
    txt(p, cx - big * 0.78, cy + big * 0.97, "0", lcap * 0.8, W::Regular, ink, 110);
    txt_r(p, cx + big * 0.78, cy + big * 0.97, &sc.s.fps_max.to_string(), lcap * 0.8, W::Regular, ink, 110);
    // the other readings
    let small = big * 0.7;
    let n = sc.items.len();
    if tall {
        // every reading on its own gauge, two to a row
        let sr = (w * 0.235).min(big * 0.85);
        let top = cy + big * 1.35 + sr * 0.7;
        let rows = n.div_ceil(2).max(1);
        let pitch = ((y + h - pad - sr * 1.4 - top) / rows as f32).clamp(sr * 2.0, sr * 4.4);
        for (i, panel) in sc.items.iter().enumerate() {
            let (col, row) = (i % 2, i / 2);
            let gx = if n == 1 { x + w / 2.0 } else { x + w * (0.27 + 0.46 * col as f32) };
            cockpit_gauge(p, sc, panel, i, gx, top + row as f32 * pitch, sr);
        }
        return;
    }
    let side: Vec<(usize, &Panel)> = sc.items.iter().enumerate().take(2).collect();
    let rest: Vec<(usize, &Panel)> = sc.items.iter().enumerate().skip(2).collect();
    for (k, (i, panel)) in side.iter().enumerate() {
        let (gx, gy) = if n == 1 {
            continue;
        } else {
            (if k == 0 { cx - w * 0.31 } else { cx + w * 0.31 }, cy)
        };
        cockpit_gauge(p, sc, panel, *i, gx, gy, small);
    }
    let base_y = cy - big * 0.3;
    for (k, (i, panel)) in rest.iter().enumerate() {
        let (rx, ry, rw) = (if k == 0 { cx - w * 0.155 - 75.0 } else { cx + w * 0.155 - 75.0 }, base_y, 150.0);
        let t = (h.min(w) * 0.03).clamp(9.0, 15.0);
        let acc = sc.c(*i);
        txt(p, rx, ry, &panel.title, t, W::Regular, ink, 150);
        let vc = dash::fit_vu(p, &panel.hero, &panel.unit, t * 3.0, rw * 0.7, W::Medium);
        let wv = dash::vu_width(p, &panel.hero, &panel.unit, vc, W::Medium);
        let _ = wv;
        dash::value_unit(p, rx, ry + vc * 1.35, &panel.hero, &panel.unit, vc, W::Medium, sc.hero_ink(panel, ink), 255, 150);
        bar(p, (rx, ry + vc * 1.35 + t * 1.2, rw, 4.0), panel.frac.unwrap_or(0.0), acc, acc, (ink, 28));
    }
}

fn cockpit_gauge(p: &mut Painter, sc: &Scene, panel: &Panel, i: usize, cx: f32, cy: f32, r: f32) {
    let ink = sc.ink;
    let th = r * 0.13;
    let (a, b) = (sc.c(i + 1), sc.c(i + 2));
    let f = panel.frac.unwrap_or(0.0);
    p.layer.gauge(cx, cy, r, th, G_START, G_SWEEP, 1.0, &|_| ink, 24);
    gauge_ticks(p, cx, cy, r - th - 5.0, r * 0.07, 10, ink);
    if sc.glow() > 0 {
        p.layer.gauge(cx, cy, r + th * 0.4, th * 1.9, G_START, G_SWEEP, f, &move |t| lerp_rgb(a, b, t), (46.0 * sc.glow() as f32 / 100.0) as u8);
    }
    p.layer.gauge(cx, cy, r, th, G_START, G_SWEEP, f, &move |t| lerp_rgb(a, b, t), 255);
    let cap = dash::fit_vu(p, &panel.hero, &panel.unit, r * 0.52, r * 1.15, W::Medium);
    let wv = dash::vu_width(p, &panel.hero, &panel.unit, cap, W::Medium);
    dash::value_unit(p, cx - wv / 2.0, cy + cap * 0.2, &panel.hero, &panel.unit, cap, W::Medium, sc.hero_ink(panel, ink), 255, 150);
    let t = (r * 0.12).clamp(9.0, 17.0);
    txt_c(p, cx, cy - cap * 0.8 - t * 0.7, &panel.title, t, W::Medium, a, 255);
    // the first reading in the opening of the gauge
    if let Some(st) = sc.stats(panel).first() {
        let s = format!("{} {}  {}", st.label, st.value, st.unit);
        txt_c(p, cx, cy + r * 0.88, &s, t * 0.9, W::Regular, sc.warm(st, ink), 190);
    }
}

// --------------------------------------------------------------------- screen

fn draw_design(p: &mut Painter, sc: &Scene, r: R, layout: Layout) {
    match layout {
        Layout::Zen => draw_zen(p, sc, r),
        Layout::Cockpit => draw_cockpit(p, sc, r),
        Layout::Tactical => draw_tactical(p, sc, r),
        Layout::Neon => draw_neon(p, sc, r),
        Layout::Telemetry => draw_telemetry(p, sc, r),
        Layout::Quest => draw_quest(p, sc, r),
        Layout::Arcade => draw_arcade(p, sc, r),
        _ => draw_arena(p, sc, r),
    }
}

pub struct GameScreen {
    started: std::time::Instant,
    layer: Layer,
    sig: String,
    hist: dash::History,
    bg: TilesScreen,
    covers: cover::CoverCache,
    pub redraws: u32,
}

/// Height the cover pictures are scaled to for the info blocks of the designs.
const COVER_H: u32 = 280;

impl GameScreen {
    pub fn new() -> Self {
        GameScreen {
            started: std::time::Instant::now(),
            layer: Layer::new(1, 1),
            sig: String::new(),
            hist: dash::History::new(),
            bg: TilesScreen::new(),
            covers: cover::CoverCache::new(4),
            redraws: 0,
        }
    }

    /// Frames per second the screen wants: the numbers change once a second, a background effect
    /// or image needs its own rate.
    pub fn wanted_fps(s: &Settings, effect_fps: f32, active_fps: f32) -> f32 {
        match s.back {
            Back::Mode(BgMode::Effect) => effect_fps,
            Back::Mode(BgMode::Image) => active_fps,
            _ => 4.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, fb: &mut Framebuffer, area: Rect, data: &WidgetData, game: Option<&GameInfo>, hint: &str, color_mode: ColorMode, s: &Settings, ts: &tiles::Settings, env: &Env) {
        let GameScreen { started, layer, sig, hist, bg, covers, redraws } = self;
        if s.layout == Layout::Classic {
            return;
        }
        bg.paint_background(fb, &s.background(), env);
        let (ax, ay, aw, ah) = area;
        let inset = (ah as f32 * 0.03).clamp(6.0, 16.0);
        let rect = (inset, inset, aw as f32 - 2.0 * inset, ah as f32 - 2.0 * inset);
        if rect.2 < 120.0 || rect.3 < 80.0 {
            return;
        }
        let now = Local::now();
        let hw = lhm::current();
        let secs = env.secs.unwrap_or_else(|| started.elapsed().as_secs_f64());
        let fps_now = data.fps.filter(|f| *f > 0);
        let ram_pct = if data.total_mb > 0 { data.used_mb as f32 * 100.0 / data.total_mb as f32 } else { 0.0 };
        let sampled: [(&'static str, Option<f32>); 3] = [("fps", fps_now.map(|f| f as f32)), ("gpu_pct", data.gpu_pct), ("ram", Some(ram_pct))];
        let keep = s.history as usize;
        if env.secs.is_some() {
            hist.fake(keep.min(90), &sampled);
        } else {
            hist.sample(secs, keep, &sampled);
        }
        let series = hist.get("fps");
        // the cover: one read per frame at most
        let mut cover_path = None;
        if s.cover {
            if let Some(c) = game.and_then(|g| g.cover.as_ref()) {
                let mut allow = true;
                let _ = covers.get(&c.path, COVER_H, &mut allow);
                cover_path = Some(c.path.clone());
            }
        }
        let dash_s = dash::Settings { font: s.font(), hero_load: s.hero_load, ..dash::Settings::default() };
        let items: Vec<Panel> = s.items.iter().map(|k| dash::panel(*k, data, &hw, &dash_s, ts, &now)).collect();
        let (avg, min, max) = match game {
            Some(g) if g.fps.avg().is_some() => (g.fps.avg(), g.fps.min, g.fps.max),
            _ if series.len() >= 2 => {
                let sum: f32 = series.iter().sum();
                (
                    Some((sum / series.len() as f32).round() as i32),
                    Some(series.iter().copied().fold(f32::MAX, f32::min).round() as i32),
                    Some(series.iter().copied().fold(0.0, f32::max).round() as i32),
                )
            }
            _ => (None, None, None),
        };
        let fps = FpsView { now: fps_now, avg, min, max };
        let clock = crate::fmt_clock(&now, false);
        let beat = (secs as u64) % 2 == 0;
        let game_sig = game.map(|g| (g.name.clone(), g.session().as_secs(), g.mem_mb.map(|m| m / 64), g.window, g.playtime_min, g.store, g.fps));
        let new_sig = format!("{s:?}|{area:?}|{items:?}|{fps:?}|{series:?}|{game_sig:?}|{cover_path:?}|{clock}|{hint}|{:?}|{}", color_mode, s.layout == Layout::Arcade && beat);
        if new_sig != *sig {
            if layer.w != aw as usize || layer.h != ah as usize {
                *layer = Layer::new(aw, ah);
            } else {
                layer.clear();
            }
            let tex = cover_path.as_deref().and_then(|p| covers.peek(p, COVER_H));
            let sc = Scene {
                s,
                font: s.font(),
                ink: s.text.unwrap_or((0xF2, 0xF4, 0xFA)),
                col: s.accents(),
                items,
                fps,
                hist: series,
                game,
                cover: tex,
                clock,
                hint: hint.to_string(),
                beat,
            };
            let mut painter = Painter { layer, font: s.font() };
            draw_design(&mut painter, &sc, rect, s.layout);
            *sig = new_sig;
            *redraws += 1;
        }
        layer.composite_onto(fb, ax, ay);
    }
}

// ------------------------------------------------------------------- tactical

fn draw_tactical(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let (c0, c1, c2, c3) = (sc.c(0), sc.c(1), sc.c(2), sc.c(3));
    let ink = sc.ink;
    let step = 24.0;
    let mut gy = y + step;
    while gy < y + h {
        let mut gx = x + step;
        while gx < x + w {
            p.layer.dot(gx, gy, 1.0, c0, 20);
            gx += step;
        }
        gy += step;
    }
    p.layer.stroke_rrect(r, 3.0, 1.0, c0, 70);
    p.layer.brackets(r, h.min(w) * 0.1, 3.0, c0, 235);
    let pad = (h.min(w) * 0.05).clamp(12.0, 28.0);
    let hcap = (h.min(w) * 0.04).clamp(11.0, 21.0);
    // header: a block, [ NAME ], facts, clock
    let hb = y + pad + hcap;
    let mut cx = x + pad;
    if let Some(name) = sc.name() {
        p.layer.fill_rrect_solid((cx.round(), (hb - hcap).round(), 5.0, hcap.round()), 0.0, c0, 255);
        cx += 16.0;
        let n = fit_clip(p, &format!("[ {} ]", name.to_uppercase()), hcap * 1.1, w * 0.45, W::Medium);
        cx += txt(p, cx, hb, &n, hcap * 1.1, W::Medium, c0, 255) + hcap * 1.4;
        for d in sc.details() {
            let dw = p.width(&d, hcap * 0.85, W::Regular);
            if cx + dw > x + w - pad - w * 0.1 {
                break;
            }
            txt(p, cx, hb, &d.to_uppercase(), hcap * 0.85, W::Regular, ink, 150);
            cx += dw + hcap * 1.4;
        }
    }
    if sc.s.clock {
        txt_r(p, x + w - pad, hb, &sc.clock, hcap * 1.1, W::Medium, c1, 255);
    }
    hair(p, (x + pad, hb + hcap * 0.7, w - 2.0 * pad, 1.0), c0, 90);
    let top = hb + hcap * 1.4;
    let body = (x + pad, top, w - 2.0 * pad, y + h - pad - top);
    let gap = pad;
    let (target, rows, graph) = if tall {
        let th = body.3 * 0.36;
        let gh = body.3 * 0.22;
        ((body.0, body.1, body.2, th), (body.0, body.1 + th + gap, body.2, body.3 - th - gh - 2.0 * gap), (body.0, body.1 + body.3 - gh, body.2, gh))
    } else {
        let tw = body.2 * 0.27;
        let gw = body.2 * 0.31;
        ((body.0, body.1, tw, body.3), (body.0 + tw + gap, body.1, body.2 - tw - gw - 2.0 * gap, body.3), (body.0 + body.2 - gw, body.1, gw, body.3))
    };
    tactical_target(p, sc, target);
    let n = sc.items.len().max(1);
    let rh = rows.3 / n as f32;
    for (i, panel) in sc.items.iter().enumerate() {
        let ry = rows.1 + i as f32 * rh;
        let tcap = (rh * 0.17).clamp(10.0, 20.0);
        txt(p, rows.0, ry + tcap * 1.2, &format!("[ {} ]", panel.title.to_uppercase()), tcap, W::Medium, c0, 255);
        let vcap = dash::fit_vu(p, &panel.hero, &panel.unit, (rh * 0.34).clamp(14.0, 56.0), rows.2 * 0.4, W::Medium);
        let vw = dash::vu_width(p, &panel.hero, &panel.unit, vcap, W::Medium);
        dash::value_unit(p, rows.0 + rows.2 - vw, ry + tcap * 1.2 + (vcap - tcap) * 0.9, &panel.hero, &panel.unit, vcap, W::Medium, sc.hero_ink(panel, ink), 255, 150);
        let f = panel.frac.unwrap_or(0.0);
        let by = ry + rh * 0.5;
        blocks(p, (rows.0, by, rows.2, (rh * 0.13).clamp(6.0, 16.0)), f, 24, 3.0, &move |t| if t < 0.6 { c1 } else if t < 0.85 { c2 } else { c3 }, 1.5, (ink, 26));
        let line: Vec<String> = sc.stats(panel).iter().take(3).map(|st| format!("{} {} {}", st.label, st.value, st.unit)).collect();
        let mut lx = rows.0;
        for (st, t) in sc.stats(panel).iter().take(3).zip(line.iter()) {
            let sw = p.width(t.trim_end(), tcap * 0.82, W::Regular);
            if lx + sw > rows.0 + rows.2 {
                break;
            }
            txt(p, lx, ry + rh * 0.86, t.trim_end(), tcap * 0.82, W::Regular, sc.warm(st, ink), 190);
            lx += sw + tcap * 1.6;
        }
        if i + 1 < n {
            hair(p, (rows.0, ry + rh - 2.0, rows.2, 1.0), c0, 40);
        }
    }
    // the graph box
    p.layer.brackets(graph, 14.0, 2.0, c0, 190);
    let lcap = (graph.3.min(graph.2) * 0.07).clamp(9.0, 16.0);
    txt(p, graph.0 + 12.0, graph.1 + lcap * 1.6, "FPS / TIME", lcap, W::Regular, c0, 220);
    let chip_h = lcap * 3.4;
    let plot = (graph.0 + 12.0, graph.1 + lcap * 2.6, graph.2 - 24.0, graph.3 - lcap * 2.6 - chip_h - 10.0);
    if sc.s.graph && sc.hist.len() > 1 && plot.3 > 20.0 {
        let (lo, hi) = fps_scale(sc);
        for k in 1..4 {
            hair(p, (plot.0, plot.1 + plot.3 * k as f32 / 4.0, plot.2, 1.0), c0, 30);
        }
        p.layer.chart(plot, &sc.hist, lo, hi, c0, 2.0, 70, true);
    }
    let cw = (graph.2 - 24.0) / 3.0;
    for (i, (label, v)) in [("AVG", sc.fps.avg), ("MIN", sc.fps.min), ("MAX", sc.fps.max)].into_iter().enumerate() {
        mini_stat(p, graph.0 + 12.0 + i as f32 * cw, graph.1 + graph.3 - chip_h, label, &opt_i(v), lcap * 1.4, ink, 140);
    }
}

fn tactical_target(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let (c0, c1) = (sc.c(0), sc.c(1));
    let ink = sc.ink;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let rr = (w.min(h) / 2.0 - 6.0).max(30.0);
    p.layer.gauge(cx, cy, rr, 2.0, 0.0, 360.0, 1.0, &|_| c0, 90);
    for i in 0..72 {
        let a = (i as f32 * 5.0).to_radians();
        let l = if i % 18 == 0 { 14.0 } else if i % 6 == 0 { 9.0 } else { 5.0 };
        let (s, c) = (a.sin(), -a.cos());
        p.layer.line(cx + s * (rr - 4.0 - l), cy + c * (rr - 4.0 - l), cx + s * (rr - 4.0), cy + c * (rr - 4.0), 1.4, c0, if i % 18 == 0 { 220 } else { 90 });
    }
    let ring = rr - 26.0;
    let th = (rr * 0.07).max(4.0);
    let f = sc.fps.frac(sc.s.fps_max);
    p.layer.gauge(cx, cy, ring, th, 0.0, 360.0, 1.0, &|_| ink, 22);
    if sc.glow() > 0 {
        p.layer.gauge(cx, cy, ring + th * 0.6, th * 2.2, 0.0, 360.0, f, &move |t| lerp_rgb(c0, c1, t), (50.0 * sc.glow() as f32 / 100.0) as u8);
    }
    p.layer.gauge(cx, cy, ring, th, 0.0, 360.0, f, &move |t| lerp_rgb(c0, c1, t), 255);
    for k in 0..4 {
        let a = (k as f32 * 90.0).to_radians();
        let (s, c) = (a.sin(), -a.cos());
        p.layer.line(cx + s * ring * 0.6, cy + c * ring * 0.6, cx + s * ring * 0.74, cy + c * ring * 0.74, 2.0, c0, 190);
    }
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", ring * 0.9, ring * 1.0, W::Medium);
    glow_txt_c(p, cx, cy + cap * 0.38, &val, cap, W::Medium, ink, sc.glow() / 2);
    let l = (ring * 0.12).clamp(9.0, 20.0);
    txt_c(p, cx, cy - cap * 0.62 - l * 0.8, "FPS", l, W::Medium, c0, 255);
    txt_c(p, cx, cy + ring * 0.6, &fps_foot(sc), l, W::Regular, ink, 170);
}

// ------------------------------------------------------------------------ neon

fn neon_sun(p: &mut Painter, cx: f32, cy: f32, rad: f32, horizon: f32, top_c: Rgb, bot_c: Rgb, glow: u32) {
    if glow > 0 {
        for (k, a) in [(1.9f32, 9.0f32), (1.55, 15.0), (1.25, 24.0)] {
            p.layer.dot(cx, cy, rad * k, bot_c, (a * glow as f32 / 100.0) as u8);
        }
    }
    let y0 = (cy - rad).floor() as i32;
    let y1 = ((cy + rad).ceil()).min(horizon) as i32;
    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        let u = (fy - cy) / rad;
        if u > 0.0 {
            // stripes: the lower half is cut by bands that get thicker toward the horizon
            let phase = ((fy - cy) / (rad * 0.17)).fract();
            if phase < 0.1 + 0.55 * u {
                continue;
            }
        }
        let t = ((fy - (cy - rad)) / (2.0 * rad)).clamp(0.0, 1.0);
        let col = lerp_rgb(top_c, bot_c, t);
        let half = (rad * rad - (fy - cy) * (fy - cy)).max(0.0).sqrt();
        for px in (cx - half - 1.0).floor() as i32..(cx + half + 1.0).ceil() as i32 {
            let cov = (half + 0.5 - (px as f32 + 0.5 - cx).abs()).clamp(0.0, 1.0);
            p.layer.over(px, py, col, (255.0 * cov) as u8);
        }
    }
}

fn neon_grid(p: &mut Painter, r: R, horizon: f32, vx: f32, ch: Rgb, cv: Rgb) {
    let (x, y, w, h) = r;
    let bottom = y + h;
    p.layer.vgrad((x, horizon, w, bottom - horizon), (14, 0, 36), (2, 0, 10), 170);
    let spread = w * 0.075;
    for i in -16..=16 {
        let xb = vx + i as f32 * spread;
        if xb < x - spread || xb > x + w + spread {
            continue;
        }
        let xt = vx + i as f32 * w * 0.004;
        p.layer.line(xt, horizon, xb, bottom, 1.4, cv, 120);
    }
    for k in 1..=9 {
        let t = (k as f32 / 9.0).powf(2.2);
        let yy = horizon + t * (bottom - horizon);
        p.layer.line(x, yy, x + w, yy, 1.5, ch, (40.0 + 110.0 * t) as u8);
    }
    p.layer.line(x, horizon, x + w, horizon, 2.4, cv, 190);
}

fn draw_neon(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let (c0, c1, c2, c3) = (sc.c(0), sc.c(1), sc.c(2), sc.c(3));
    let ink = sc.ink;
    let pad = (h.min(w) * 0.045).clamp(10.0, 26.0);
    let hcap = (h.min(w) * 0.04).clamp(11.0, 22.0);
    let horizon = if tall { y + h * 0.31 } else { y + h * 0.64 };
    let rad = if tall { (w * 0.34).min(h * 0.2) } else { (h * 0.40).min(w * 0.16) };
    let (sx, sy) = if tall { (x + w / 2.0, horizon - rad * 0.12) } else { (x + w * 0.27, horizon - rad * 0.1) };
    neon_sun(p, sx, sy, rad, horizon, c2, c0, sc.glow());
    neon_grid(p, r, horizon, if tall { x + w / 2.0 } else { x + w * 0.5 }, c1, c0);
    // the FPS on the sun
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", rad * 0.95, rad * 1.7, W::Medium);
    glow_txt_c(p, sx, sy + cap * 0.1, &val, cap, W::Medium, (255, 255, 255), sc.glow().max(30));
    let l = (rad * 0.15).clamp(10.0, 24.0);
    txt_c(p, sx, sy - cap * 0.9 - l * 0.4, "FPS", l, W::Medium, (255, 255, 255), 255);
    txt_c(p, sx, sy + cap * 0.1 + l * 2.0, &fps_foot(sc), l, W::Medium, (255, 255, 255), 230);
    // header
    let hb = y + pad + hcap;
    if let Some(name) = sc.name() {
        let n = fit_clip(p, &name.to_uppercase(), hcap * 1.25, w * 0.5, W::Medium);
        let nw = p.width(&n, hcap * 1.25, W::Medium);
        glow_txt(p, x + pad, hb, &n, hcap * 1.25, W::Medium, c1, sc.glow());
        let mut cx = x + pad + nw + hcap * 1.6;
        for d in sc.details() {
            let dw = p.width(&d, hcap * 0.85, W::Regular);
            if cx + dw > x + w - pad - w * 0.1 {
                break;
            }
            txt(p, cx, hb, &d, hcap * 0.85, W::Regular, ink, 170);
            cx += dw + hcap * 1.4;
        }
    }
    if sc.s.clock {
        let cw = p.width(&sc.clock, hcap * 1.25, W::Medium);
        glow_txt(p, x + w - pad - cw, hb, &sc.clock, hcap * 1.25, W::Medium, c0, sc.glow());
    }
    // average, lowest, highest on the left of the sun
    if !tall {
        let sx0 = x + pad;
        for (i, (label, v)) in [("AVG", sc.fps.avg), ("MIN", sc.fps.min), ("MAX", sc.fps.max)].into_iter().enumerate() {
            mini_stat(p, sx0, y + h * 0.16 + i as f32 * h * 0.165, label, &opt_i(v), hcap * 1.25, ink, 150);
        }
    }
    // the readings as neon plates
    let n = sc.items.len().max(1);
    let zone = if tall {
        (x + pad, horizon + pad + hcap * 2.2, w - 2.0 * pad, y + h - horizon - 2.0 * pad - hcap * 2.2)
    } else {
        (x + w * 0.52, y + pad * 2.0 + hcap * 2.0, w * 0.48 - pad, y + h - pad - (y + pad * 2.0 + hcap * 2.0))
    };
    let gap = pad * 0.6;
    let ph = (zone.3 - gap * (n as f32 - 1.0)) / n as f32;
    for (i, panel) in sc.items.iter().enumerate() {
        let col = [c1, c0, c2, c3][i % 4];
        let rect = (zone.0, zone.1 + i as f32 * (ph + gap), zone.2, ph);
        p.layer.fill_rrect_solid((rect.0.round(), rect.1.round(), rect.2.round(), rect.3.round()), 12.0, (6, 0, 18), 190);
        if sc.glow() > 0 {
            p.layer.stroke_rrect((rect.0 - 2.0, rect.1 - 2.0, rect.2 + 4.0, rect.3 + 4.0), 14.0, 4.0, col, (34.0 * sc.glow() as f32 / 100.0) as u8);
        }
        p.layer.stroke_rrect(rect, 12.0, 2.0, col, 230);
        let t = (ph * 0.2).clamp(10.0, 19.0);
        let inx = rect.0 + ph * 0.28;
        txt(p, inx, rect.1 + ph * 0.36, &panel.title, t * 1.15, W::Medium, col, 255);
        let sts: Vec<String> = sc.stats(panel).iter().take(2).map(|st| format!("{} {} {}", st.label, st.value, st.unit)).collect();
        txt(p, inx, rect.1 + ph * 0.72, sts.join("   ").trim(), t * 0.82, W::Regular, ink, 170);
        let vcap = dash::fit_vu(p, &panel.hero, &panel.unit, (ph * 0.44).clamp(14.0, 60.0), rect.2 * 0.34, W::Medium);
        let vw = dash::vu_width(p, &panel.hero, &panel.unit, vcap, W::Medium);
        dash::value_unit(p, rect.0 + rect.2 - ph * 0.28 - vw, rect.1 + ph * 0.5 + vcap * 0.4, &panel.hero, &panel.unit, vcap, W::Medium, sc.hero_ink(panel, (255, 255, 255)), 255, 170);
        bar(p, (rect.0 + ph * 0.28, rect.1 + ph - ph * 0.14, rect.2 - ph * 0.56, 4.0), panel.frac.unwrap_or(0.0), col, lerp_rgb(col, (255, 255, 255), 0.4), (ink, 24));
    }
}

// ------------------------------------------------------------------- telemetry

fn draw_telemetry(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let (c0, c1, c2, c3) = (sc.c(0), sc.c(1), sc.c(2), sc.c(3));
    let ink = sc.ink;
    let rad = (h.min(w) * 0.04).clamp(8.0, 18.0);
    p.layer.fill_rrect_solid(r, rad, (3, 6, 12), 120);
    p.layer.stroke_rrect(r, rad, 1.2, (255, 255, 255), 24);
    let pad = (h.min(w) * 0.04).clamp(10.0, 24.0);
    let hcap = (h.min(w) * 0.04).clamp(11.0, 21.0);
    let has_head = sc.s.clock || sc.name().is_some();
    if has_head {
        header_line(p, sc, x + pad, y + pad + hcap, w - 2.0 * pad, hcap, c0);
    }
    let top = y + pad + if has_head { hcap * 2.4 } else { 0.0 };
    let body = (x + pad, top, w - 2.0 * pad, y + h - pad - top);
    let gap = pad;
    let (chart, side) = if tall {
        let ch = body.3 * 0.36;
        ((body.0, body.1, body.2, ch), (body.0, body.1 + ch + gap, body.2, body.3 - ch - gap))
    } else {
        let cw = body.2 * 0.62;
        ((body.0, body.1, cw, body.3), (body.0 + cw + gap, body.1, body.2 - cw - gap, body.3))
    };
    // the chart with its scale
    let lab = (chart.3.min(chart.2) * 0.045).clamp(9.0, 15.0);
    let plot = (chart.0 + lab * 3.6, chart.1 + lab * 0.6, chart.2 - lab * 3.6, chart.3 - lab * 2.4);
    let (lo, hi) = fps_scale(sc);
    let step = nice_step(hi - lo);
    let mut v = 0.0f32;
    while v <= hi {
        let yy = plot.1 + plot.3 - (v - lo) / (hi - lo) * plot.3;
        hair(p, (plot.0, yy, plot.2, 1.0), ink, if v == 0.0 { 60 } else { 28 });
        txt_r(p, plot.0 - lab * 0.6, yy + lab * 0.35, &format!("{v:.0}"), lab * 0.9, W::Regular, ink, 130);
        v += step;
    }
    txt(p, plot.0, chart.1 + chart.3 - lab * 0.2, &format!("-{}s", sc.s.history.min(sc.hist.len().max(1) as u32 * 1)), lab * 0.85, W::Regular, ink, 110);
    txt_r(p, plot.0 + plot.2, chart.1 + chart.3 - lab * 0.2, "now", lab * 0.85, W::Regular, ink, 110);
    if sc.s.graph && sc.hist.len() > 1 {
        if sc.glow() > 0 {
            p.layer.chart(plot, &sc.hist, lo, hi, c0, 7.0, 0, false);
        }
        p.layer.chart(plot, &sc.hist, lo, hi, c0, 2.6, 90, true);
    }
    // the number over the chart, top left
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", (plot.3 * 0.42).min(plot.2 * 0.3), plot.2 * 0.4, W::Medium);
    let (nx, ny) = (plot.0 + lab * 1.4, plot.1 + lab * 1.0 + cap);
    p.layer.fill_rrect_solid((nx - lab * 0.8, plot.1 + lab * 0.2, p.width(&val, cap, W::Medium) + lab * 7.0, cap + lab * 2.6), 10.0, (3, 6, 12), 150);
    glow_txt(p, nx, ny, &val, cap, W::Medium, ink, sc.glow() / 3);
    let vw = p.width(&val, cap, W::Medium);
    txt(p, nx + vw + lab * 0.8, ny - cap * 0.55, "FPS", lab * 1.3, W::Medium, c0, 255);
    txt(p, nx + vw + lab * 0.8, ny, &fps_foot(sc), lab * 1.1, W::Regular, ink, 160);
    // numbers of the session
    let cells: [(&str, String, Rgb); 4] = [
        ("AVERAGE", opt_i(sc.fps.avg), c1),
        ("LOWEST", opt_i(sc.fps.min), c3),
        ("HIGHEST", opt_i(sc.fps.max), c2),
        ("FRAME TIME", sc.fps.frametime().map_or_else(|| "--".into(), |m| format!("{m} ms")), c0),
    ];
    let (stats, items) = if tall {
        let sh = side.3 * 0.36;
        ((side.0, side.1, side.2, sh), (side.0, side.1 + sh + gap, side.2, side.3 - sh - gap))
    } else {
        let sw = side.2 * 0.46;
        ((side.0, side.1, sw, side.3), (side.0 + sw + gap, side.1, side.2 - sw - gap, side.3))
    };
    let cols = if tall { 2 } else { 1 };
    let rows = 4 / cols;
    let (cw, chh) = (stats.2 / cols as f32, stats.3 / rows as f32);
    for (i, (label, value, col)) in cells.iter().enumerate() {
        let (cx, cy) = (stats.0 + (i % cols) as f32 * cw, stats.1 + (i / cols) as f32 * chh);
        p.layer.dot(cx + 5.0, cy + chh * 0.2, 4.0, *col, 255);
        let t = (chh * 0.15).clamp(9.0, 15.0);
        txt(p, cx + 16.0, cy + chh * 0.2 + t * 0.35, label, t, W::Regular, ink, 150);
        let vc = dash::fit_vu(p, value, "", (chh * 0.38).clamp(14.0, 48.0), cw - 20.0, W::Medium);
        txt(p, cx + 4.0, cy + chh * 0.28 + vc * 1.2, value, vc, W::Medium, ink, 255);
    }
    // the readings of the machine
    let n = sc.items.len().max(1);
    let ih = items.3 / n as f32;
    for (i, panel) in sc.items.iter().enumerate() {
        let ry = items.1 + i as f32 * ih;
        let col = sc.c(i + 1);
        let t = (ih * 0.17).clamp(9.0, 16.0);
        txt(p, items.0, ry + ih * 0.3, &panel.title, t * 1.1, W::Medium, col, 255);
        let vc = dash::fit_vu(p, &panel.hero, &panel.unit, (ih * 0.34).clamp(13.0, 44.0), items.2 * 0.5, W::Medium);
        let vw = dash::vu_width(p, &panel.hero, &panel.unit, vc, W::Medium);
        dash::value_unit(p, items.0 + items.2 - vw, ry + ih * 0.3 + vc * 0.3, &panel.hero, &panel.unit, vc, W::Medium, sc.hero_ink(panel, ink), 255, 150);
        bar(p, (items.0, ry + ih * 0.5, items.2, 4.0), panel.frac.unwrap_or(0.0), col, lerp_rgb(col, (255, 255, 255), 0.35), (ink, 24));
        let sts: Vec<String> = sc.stats(panel).iter().take(2).map(|st| format!("{} {} {}", st.label, st.value, st.unit)).collect();
        txt(p, items.0, ry + ih * 0.5 + t * 2.0, sts.join("   ").trim(), t * 0.9, W::Regular, ink, 150);
    }
}

/// A round step for the lines of a scale: 10, 20, 50, 100 ...
fn nice_step(range: f32) -> f32 {
    let raw = (range / 4.0).max(1.0);
    let mag = 10f32.powf(raw.log10().floor());
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if raw <= m * mag {
            return m * mag;
        }
    }
    10.0 * mag
}

// ---------------------------------------------------------------------- quest

/// A cover picture inside a rounded rectangle.
pub(super) fn blit_cover(layer: &mut Layer, tex: &cover::Tex, r: R, radius: f32) {
    let (x, y, w, h) = r;
    for py in y.floor() as i32..(y + h).ceil() as i32 {
        for px in x.floor() as i32..(x + w).ceil() as i32 {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let cov = rrect_cov(fx, fy, x, y, w, h, radius);
            if cov <= 0.0 {
                continue;
            }
            let c = tex.sample((fx - x) / w * tex.w as f32, (fy - y) / h * tex.h as f32);
            layer.over(px, py, (c[0] as u8, c[1] as u8, c[2] as u8), (c[3] / 255.0 * cov * 255.0).round() as u8);
        }
    }
}

fn draw_quest(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let gold = (0xD4, 0xAF, 0x5A);
    let ink = sc.ink;
    let rad = (h.min(w) * 0.045).clamp(10.0, 22.0);
    p.layer.fill_rrect_solid(r, rad, (10, 12, 20), 225);
    p.layer.stroke_rrect(r, rad, 3.0, gold, 235);
    let inner = (x + 8.0, y + 8.0, w - 16.0, h - 16.0);
    p.layer.stroke_rrect(inner, rad * 0.6, 1.0, gold, 100);
    for (cx, cy) in [(x + 8.0, y + 8.0), (x + w - 8.0, y + 8.0), (x + 8.0, y + h - 8.0), (x + w - 8.0, y + h - 8.0)] {
        p.layer.dot(cx, cy, 5.5, gold, 255);
        p.layer.dot(cx, cy, 2.5, (10, 12, 20), 255);
    }
    let pad = (h.min(w) * 0.06).clamp(14.0, 32.0);
    let body = (x + pad, y + pad, w - 2.0 * pad, h - 2.0 * pad);
    let gap = pad * 0.9;
    let (card, plaque, bars) = if tall {
        let ch = body.3 * 0.22;
        let ph = body.3 * 0.25;
        ((body.0, body.1, body.2, ch), (body.0, body.1 + ch + gap, body.2, ph), (body.0, body.1 + ch + ph + 2.0 * gap, body.2, body.3 - ch - ph - 2.0 * gap))
    } else {
        let cw = body.2 * 0.25;
        let pw = body.2 * 0.29;
        ((body.0, body.1, cw, body.3), (body.0 + cw + gap, body.1, pw, body.3), (body.0 + cw + pw + 2.0 * gap, body.1, body.2 - cw - pw - 2.0 * gap, body.3))
    };
    // the card: cover or an emblem with the first letter, the name and the level
    let name = sc.game.map(|g| g.name.clone()).unwrap_or_else(|| "Player One".into());
    let text_h = card.3 * if tall { 0.0 } else { 0.28 };
    let pic_h = if tall { card.3 } else { card.3 - text_h };
    let aspect = sc.cover.filter(|_| sc.s.cover).map_or(0.74, |t| t.w as f32 / t.h as f32);
    let pic_w = (pic_h * aspect).min(card.2 * if tall { 0.4 } else { 1.0 });
    let pic = (card.0 + if tall { 0.0 } else { (card.2 - pic_w) / 2.0 }, card.1, pic_w, pic_h);
    match sc.cover.filter(|_| sc.s.cover) {
        Some(t) => blit_cover(p.layer, t, pic, 8.0),
        None => {
            p.layer.fill_rrect(pic, 8.0, &move |t| lerp_rgb((0x3A, 0x2C, 0x14), (0x12, 0x14, 0x22), t), 255);
            let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            let cap = pic_h * 0.5;
            txt_c(p, pic.0 + pic.2 / 2.0, pic.1 + pic.3 / 2.0 + cap * 0.5, &letter, cap, W::Medium, gold, 255);
        }
    }
    p.layer.stroke_rrect(pic, 8.0, 2.0, gold, 255);
    let ncap = (card.3.min(card.2) * if tall { 0.14 } else { 0.1 }).clamp(11.0, 26.0);
    let lv = sc.game.and_then(|g| g.playtime_min).map(|m| format!("Lv. {}", (m / 60).max(1)));
    let sub: String = {
        let d = sc.details();
        d.into_iter().find(|t| t.starts_with("Session") || t.starts_with("SESSION")).unwrap_or_default()
    };
    if tall {
        let tx = pic.0 + pic.2 + gap;
        let n = fit_clip(p, &name, ncap * 1.2, card.0 + card.2 - tx, W::Medium);
        txt(p, tx, card.1 + card.3 * 0.4, &n, ncap * 1.2, W::Medium, ink, 255);
        let line = fit_clip(p, format!("{}   {}", lv.clone().unwrap_or_default(), sub).trim(), ncap * 0.9, card.0 + card.2 - tx, W::Regular);
        txt(p, tx, card.1 + card.3 * 0.4 + ncap * 2.0, &line, ncap * 0.9, W::Regular, gold, 230);
    } else {
        let n = fit_clip(p, &name, ncap * 1.25, card.2, W::Medium);
        txt_c(p, card.0 + card.2 / 2.0, card.1 + pic_h + ncap * 1.9, &n, ncap * 1.25, W::Medium, ink, 255);
        let sub = format!("{}{}", lv.clone().unwrap_or_default(), if sc.s.clock { format!("{}{}", if lv.is_some() { "   " } else { "" }, sc.clock) } else { String::new() });
        if !sub.is_empty() {
            txt_c(p, card.0 + card.2 / 2.0, card.1 + pic_h + ncap * 3.7, &sub, ncap, W::Medium, gold, 255);
        }
    }
    // the plaque with the FPS
    let pl = (plaque.0 + plaque.2 / 2.0, plaque.1);
    hair(p, (plaque.0, plaque.1 + plaque.3 * 0.06, plaque.2, 1.0), gold, 110);
    hair(p, (plaque.0, plaque.1 + plaque.3 * 0.94, plaque.2, 1.0), gold, 110);
    p.layer.dot(plaque.0, plaque.1 + plaque.3 * 0.06 + 0.5, 3.0, gold, 255);
    p.layer.dot(plaque.0 + plaque.2, plaque.1 + plaque.3 * 0.06 + 0.5, 3.0, gold, 255);
    p.layer.dot(plaque.0, plaque.1 + plaque.3 * 0.94 + 0.5, 3.0, gold, 255);
    p.layer.dot(plaque.0 + plaque.2, plaque.1 + plaque.3 * 0.94 + 0.5, 3.0, gold, 255);
    let lcap = (plaque.3.min(plaque.2) * 0.08).clamp(10.0, 20.0);
    txt_c(p, pl.0, plaque.1 + plaque.3 * 0.06 + lcap * 2.2, &spaced("FRAME RATE"), lcap, W::Medium, gold, 255);
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", plaque.3 * 0.36, plaque.2 * 0.9, W::Medium);
    let by = plaque.1 + plaque.3 * 0.5 + cap * 0.55;
    glow_txt_c(p, pl.0, by, &val, cap, W::Medium, (0xFF, 0xE9, 0xB0), sc.glow() / 2);
    txt_c(p, pl.0, by + lcap * 2.0, &format!("FPS   {}", fps_foot(sc)), lcap * 0.95, W::Regular, ink, 180);
    txt_c(p, pl.0, plaque.1 + plaque.3 * 0.94 - lcap * 0.8, &format!("AVG {}    MIN {}    MAX {}", opt_i(sc.fps.avg), opt_i(sc.fps.min), opt_i(sc.fps.max)), lcap * 0.9, W::Regular, ink, 150);
    // the bars
    let n = sc.items.len().max(1);
    let rh = bars.3 / n as f32;
    let tags = ["HP", "MP", "STA", "XP"];
    for (i, panel) in sc.items.iter().enumerate() {
        let col = sc.c(i);
        let ry = bars.1 + i as f32 * rh;
        let tcap = (rh * 0.24).clamp(11.0, 26.0);
        let tag_w = tcap * 3.2;
        txt(p, bars.0, ry + rh * 0.44, tags[i % 4], tcap * 1.15, W::Medium, col, 255);
        let vcap = (rh * 0.26).clamp(11.0, 30.0);
        let vw = dash::vu_width(p, &panel.hero, &panel.unit, vcap, W::Medium);
        let bx = bars.0 + tag_w;
        let bw = bars.2 - tag_w - vw - tcap * 1.2;
        let bh = (rh * 0.3).clamp(10.0, 30.0);
        let by0 = ry + rh * 0.44 - bh * 0.78;
        let track = (bx, by0, bw, bh);
        p.layer.fill_rrect(track, bh / 2.0, &|_| (0, 0, 0), 170);
        let f = panel.frac.unwrap_or(0.0);
        let fw = (bw * f).max(if f > 0.0 { bh } else { 0.0 });
        if fw > 0.0 {
            p.layer.fill_rrect((bx, by0, fw, bh), bh / 2.0, &move |t| lerp_rgb(darken_c(col, 0.25), lerp_rgb(col, (255, 255, 255), 0.25), t), 255);
            p.layer.fill_rrect((bx + bh * 0.3, by0 + bh * 0.12, (fw - bh * 0.6).max(0.0), bh * 0.32), bh * 0.16, &|_| (255, 255, 255), 55);
        }
        p.layer.stroke_rrect(track, bh / 2.0, 1.4, gold, 170);
        dash::value_unit(p, bars.0 + bars.2 - vw, ry + rh * 0.44, &panel.hero, &panel.unit, vcap, W::Medium, sc.hero_ink(panel, ink), 255, 160);
        let sts: Vec<String> = sc.stats(panel).iter().take(3).map(|st| format!("{} {} {}", st.label, st.value, st.unit)).collect();
        let line = fit_clip(p, format!("{}   {}", panel.title, sts.join("   ")).trim(), tcap * 0.72, bars.0 + bars.2 - bx, W::Regular);
        txt(p, bx, ry + rh * 0.44 + bh * 0.8 + tcap * 0.9, &line, tcap * 0.72, W::Regular, ink, 150);
    }
}

fn darken_c(c: Rgb, t: f32) -> Rgb {
    lerp_rgb(c, (0, 0, 0), t)
}

// --------------------------------------------------------------------- arcade

fn draw_arcade(p: &mut Painter, sc: &Scene, r: R) {
    let (x, y, w, h) = r;
    let tall = is_tall(r);
    let (green, yellow, red, cyan) = (sc.c(0), sc.c(1), sc.c(2), sc.c(3));
    let ink = sc.ink;
    let pc = |k: f32| ((k / 7.0).round().max(1.0)) * 7.0;
    let pad = (h.min(w) * 0.05).clamp(12.0, 30.0);
    let c1 = pc((h.min(w) * 0.045).clamp(14.0, 28.0));
    let hb = y + pad + c1;
    // top row: 1UP, name, hi-score, clock
    if sc.beat {
        txt(p, x + pad, hb, "1UP", c1, W::Regular, green, 255);
    }
    if let Some(name) = sc.name() {
        let n = fit_clip(p, &name.to_uppercase(), c1, w * if tall { 0.55 } else { 0.28 }, W::Regular);
        txt(p, x + pad + c1 * 5.0, hb, &n, c1, W::Regular, ink, 255);
    }
    if !tall {
        txt_c(p, x + w * 0.56, hb, "HI-SCORE", c1, W::Regular, red, 255);
        txt_c(p, x + w * 0.56, hb + c1 * 1.7, &opt_i(sc.fps.max), c1, W::Regular, ink, 255);
    }
    if sc.s.clock {
        txt_r(p, x + w - pad, hb, &sc.clock, c1, W::Regular, yellow, 255);
    }
    let top = hb + c1 * 3.0;
    let foot = y + h - pad - c1;
    let body = (x + pad, top, w - 2.0 * pad, foot - top - c1 * 0.8);
    let (hero, rows) = if tall {
        let hh = body.3 * 0.36;
        ((body.0, body.1, body.2, hh), (body.0, body.1 + hh + pad, body.2, body.3 - hh - pad))
    } else {
        let hw = body.2 * 0.42;
        ((body.0, body.1, hw, body.3), (body.0 + hw + pad * 1.5, body.1, body.2 - hw - pad * 1.5, body.3))
    };
    // the FPS
    txt(p, hero.0, hero.1 + c1, "FPS", c1, W::Regular, green, 255);
    let val = sc.fps.value();
    let cap = dash::fit_vu(p, &val, "", pc(hero.3 * 0.7), hero.2, W::Regular);
    let base = hero.1 + c1 * 2.0 + cap + (hero.3 - c1 * 2.0 - cap - c1 * 2.2).max(0.0) / 2.0;
    glow_txt(p, hero.0, base, &val, cap, W::Regular, yellow, sc.glow() / 2);
    txt(p, hero.0, base + c1 * 2.0, &format!("{}  AVG {}", fps_foot(sc).to_uppercase(), opt_i(sc.fps.avg)), pc(c1 * 0.8), W::Regular, ink, 200);
    // the readings
    let n = sc.items.len().max(1);
    let rh = rows.3 / n as f32;
    for (i, panel) in sc.items.iter().enumerate() {
        let ry = rows.1 + i as f32 * rh;
        let lc = pc((rh * 0.2).clamp(14.0, 28.0));
        let title: String = panel.title.chars().take(3).collect::<String>().to_uppercase();
        txt(p, rows.0, ry + lc, &title, lc, W::Regular, cyan, 255);
        let vc = pc((rh * 0.2).clamp(14.0, 28.0));
        let vtxt = format!("{}{}", panel.hero, panel.unit);
        let vw = p.width(&vtxt, vc, W::Regular);
        txt_r(p, rows.0 + rows.2, ry + lc, &vtxt, vc, W::Regular, sc.hero_ink(panel, ink), 255);
        let bx = rows.0 + lc * 4.4;
        let bw = rows.2 - lc * 4.4 - vw - lc;
        let f = panel.frac.unwrap_or(0.0);
        blocks(p, (bx, ry + lc * 0.1, bw, lc * 0.9), f, 22, 3.0, &move |t| if t < 0.6 { green } else if t < 0.85 { yellow } else { red }, 0.0, (ink, 24));
        let sts: Vec<String> = sc.stats(panel).iter().take(3).map(|st| format!("{} {}{}", st.label, st.value, st.unit.replace('\u{b0}', ""))).collect();
        let line = fit_clip(p, &sts.join("  ").to_uppercase(), pc(lc * 0.6), rows.2, W::Regular);
        txt(p, rows.0, ry + lc * 2.4, &line, pc(lc * 0.6), W::Regular, ink, 150);
    }
    // the bottom line
    if sc.beat {
        txt_c(p, x + w / 2.0, y + h - pad, "PRESS START", c1, W::Regular, yellow, 255);
    }
    if !tall {
        txt(p, x + pad, y + h - pad, "CREDIT 01", c1, W::Regular, ink, 160);
        if let Some(d) = sc.details().into_iter().next() {
            txt_r(p, x + w - pad, y + h - pad, &d.to_uppercase(), c1, W::Regular, ink, 160);
        }
    }
    // the screen of an old monitor: dark lines
    let mut py = y.floor() as i32;
    while (py as f32) < y + h {
        p.layer.span_over(py, x.floor() as i32, (x + w).ceil() as i32, (0, 0, 0), 70);
        py += 3;
    }
}
