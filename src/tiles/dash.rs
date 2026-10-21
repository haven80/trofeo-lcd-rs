//! Three more full screens built from the same pieces as the tiles (`layout = deck`,
//! `cards`, `orbit`). They show the same readings (CPU, GPU, RAM, fans, network, disk, clock,
//! weather, music), each in its own way:
//!
//! * `deck`  - instrument columns: a live gauge line on top of every column, a big light value,
//!   a sparkline of the last minutes and a 2x2 grid of small readings. Hairlines between the
//!   columns, no boxes.
//! * `cards` - flat, colourful cards in a mosaic (columns of one or more stacked cards). Every
//!   card has a soft gradient that melts into its neighbour's colour and a ghost silhouette of
//!   its own history along the bottom.
//! * `orbit` - one cluster of concentric rings per device, with the readings beside it and a
//!   legend of what each ring means.
//!
//! Everything is chosen in the config (`deck = ...`, `cards = ...`, `orbit = ...`, `dash_*`).
//! Like the tiles, the screen is painted on a transparent layer that is redrawn only when
//! something shown changes (about once a second, because of the graphs).

use super::*;
use std::collections::{HashMap, VecDeque};

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Deck,
    Cards,
    Orbit,
}

impl Style {
    pub fn from_layout(name: &str) -> Option<Style> {
        match name {
            "deck" => Some(Style::Deck),
            "cards" => Some(Style::Cards),
            "orbit" => Some(Style::Orbit),
            _ => None,
        }
    }
}

/// What a column / card / cluster can show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PKind {
    Cpu,
    Gpu,
    Ram,
    Fans,
    Net,
    Disk,
    Clock,
    Weather,
    Music,
}

impl PKind {
    pub const NAMES: &'static str = "cpu, gpu, ram, fans, net, disk, clock, weather, music";

    pub fn parse(s: &str) -> Option<PKind> {
        Some(match s.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "cpu" => PKind::Cpu,
            "gpu" => PKind::Gpu,
            "ram" | "mem" | "memory" => PKind::Ram,
            "fans" | "fan" | "ventole" => PKind::Fans,
            "net" | "network" | "rete" => PKind::Net,
            "disk" | "storage" | "disco" => PKind::Disk,
            "clock" | "time" | "orologio" => PKind::Clock,
            "weather" | "meteo" => PKind::Weather,
            "music" | "nowplaying" | "now_playing" | "musica" => PKind::Music,
            _ => return None,
        })
    }

    /// Weight of a column of cards / of a deck column (clock and text panels are wider).
    fn weight(self) -> f32 {
        match self {
            PKind::Clock => 1.15,
            PKind::Weather | PKind::Music => 1.1,
            _ => 1.0,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub font: FontStyle,
    pub text: Option<Rgb>,
    /// `dark` (white text on a dark plate) or `light` (dark text on a white one, pastel cards).
    pub theme: Theme,
    /// The plate behind `deck` and `orbit` (default by theme).
    pub plate_color: Option<Rgb>,
    /// 0-100: opacity of the cards.
    pub cards_opacity: u32,
    /// Accent colours (rings, cards, graph lines).
    pub colors: Vec<Rgb>,
    /// Seconds of history in the graphs.
    pub history: u32,
    pub graph: bool,
    /// 0-100: opacity of the plate behind `deck` and `orbit`.
    pub plate: Option<u32>,
    /// Network speed (MB/s) that fills a ring / gauge.
    pub net_max: u32,
    pub bg: BgMode,
    pub bg_colors: Vec<Rgb>,
    pub bg_image: Option<PathBuf>,
    pub bg_dim: u32,
    pub deck: Vec<PKind>,
    pub deck_gauge: bool,
    pub cards: Vec<Vec<PKind>>,
    pub cards_radius: Option<u32>,
    pub cards_gap: u32,
    pub orbit: Vec<PKind>,
    pub orbit_clock: bool,
    pub orbit_legend: bool,
}

pub const DEFAULT_COLORS: [Rgb; 5] = [
    (0xA7, 0x6B, 0xFF),
    (0x38, 0xC6, 0xF4),
    (0xFF, 0x6F, 0xB1),
    (0xFF, 0xC0, 0x45),
    (0x3D, 0xDC, 0x97),
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            font: FontStyle::Smooth,
            text: None,
            theme: Theme::Dark,
            plate_color: None,
            cards_opacity: 100,
            colors: DEFAULT_COLORS.to_vec(),
            history: 90,
            graph: true,
            plate: None,
            net_max: 25,
            bg: BgMode::Global,
            bg_colors: vec![(0x10, 0x14, 0x28), (0x24, 0x10, 0x30)],
            bg_image: None,
            bg_dim: 0,
            deck: vec![PKind::Cpu, PKind::Gpu, PKind::Ram, PKind::Clock],
            deck_gauge: true,
            cards: vec![
                vec![PKind::Clock],
                vec![PKind::Cpu],
                vec![PKind::Gpu],
                vec![PKind::Ram, PKind::Disk],
                vec![PKind::Music, PKind::Net],
            ],
            cards_radius: None,
            cards_gap: 12,
            orbit: vec![PKind::Cpu, PKind::Gpu, PKind::Ram],
            orbit_clock: true,
            orbit_legend: true,
        }
    }
}

fn parse_kinds(key: &str, raw: &str, max: usize) -> anyhow::Result<Vec<PKind>> {
    let mut out = Vec::new();
    for item in raw.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        out.push(PKind::parse(item).ok_or_else(|| anyhow::anyhow!("{key}: '{item}' is not valid ({})", PKind::NAMES))?);
    }
    if out.is_empty() {
        anyhow::bail!("{key}: the list is empty ({})", PKind::NAMES);
    }
    if out.len() > max {
        anyhow::bail!("{key}: at most {max} entries (you listed {})", out.len());
    }
    Ok(out)
}

/// `cards = clock | cpu | gpu | ram, disk | music, net`: `|` separates the columns, `,` stacks cards.
pub fn parse_cards(raw: &str) -> anyhow::Result<Vec<Vec<PKind>>> {
    let mut cols = Vec::new();
    for col in raw.split('|') {
        if col.trim().is_empty() {
            continue;
        }
        let stack = parse_kinds("cards", col, 3)?;
        cols.push(stack);
    }
    if cols.is_empty() {
        anyhow::bail!("cards: the list is empty ({})", PKind::NAMES);
    }
    if cols.len() > 6 {
        anyhow::bail!("cards: at most 6 columns (you listed {})", cols.len());
    }
    Ok(cols)
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get("dash_font") {
        s.font = match v.trim().to_ascii_lowercase().as_str() {
            "smooth" | "liscio" | "vector" | "ttf" => FontStyle::Smooth,
            "pixel" | "normal" | "normale" | "bitmap" => FontStyle::Pixel,
            _ => anyhow::bail!("dash_font: '{v}' not valid (smooth | pixel)"),
        };
    }
    if let Some(v) = file.get("dash_theme") {
        s.theme = match v.trim().to_ascii_lowercase().as_str() {
            "dark" | "scuro" => Theme::Dark,
            "light" | "chiaro" | "white" | "bianco" => Theme::Light,
            _ => anyhow::bail!("dash_theme: '{v}' not valid (dark | light)"),
        };
    }
    if let Some(v) = file.get("dash_plate_color") {
        s.plate_color = Some(parse_rgb("dash_plate_color", v)?);
    }
    if let Some(v) = file.get("dash_text_color") {
        s.text = Some(parse_rgb("dash_text_color", v)?);
    }
    let num = |key: &str, lo: u32, hi: u32| -> anyhow::Result<Option<u32>> {
        match file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            None => Ok(None),
            Some(v) if (lo..=hi).contains(&v) => Ok(Some(v)),
            Some(v) => anyhow::bail!("{key}: must be between {lo} and {hi} (got {v})"),
        }
    };
    let boolean = |key: &str| file.get_bool(key).map_err(|e| anyhow::anyhow!(e));
    // accent colours: a preset of the animated backgrounds, or a list of your own
    if let Some(v) = file.get("dash_palette") {
        let name = v.trim();
        if !name.eq_ignore_ascii_case("default") && !name.is_empty() {
            let pal = effect::Palette::parse(name)
                .ok_or_else(|| anyhow::anyhow!("dash_palette: '{name}' not valid (default | {})", effect::Palette::NAMES))?;
            // the dark parts of a preset would vanish on a dark screen: lift them
            s.colors = (0..6)
                .map(|i| {
                    let c = pal.at(i as f32 / 6.0);
                    let l = luma(c);
                    if l < 130.0 { lerp_rgb(c, (255, 255, 255), (130.0 - l) / (255.0 - l)) } else { c }
                })
                .collect();
        }
    }
    if let Some(v) = file.get("dash_colors") {
        let list = parse_rgb_list("dash_colors", v)?;
        if list.is_empty() {
            anyhow::bail!("dash_colors: needs at least one color, e.g. #A76BFF, #38C6F4");
        }
        s.colors = list;
    }
    if let Some(v) = num("dash_history", 20, 600)? {
        s.history = v;
    }
    if let Some(v) = boolean("dash_graph")? {
        s.graph = v;
    }
    if let Some(v) = num("cards_opacity", 0, 100)? {
        s.cards_opacity = v;
    }
    if let Some(v) = num("dash_plate", 0, 100)? {
        s.plate = Some(v);
    }
    if let Some(v) = num("dash_net_max", 1, 1000)? {
        s.net_max = v;
    }
    if let Some(v) = file.get("dash_bg") {
        s.bg = BgMode::parse(v).ok_or_else(|| anyhow::anyhow!("dash_bg: '{v}' not valid ({})", BgMode::NAMES))?;
    }
    if let Some(v) = file.get("dash_bg_color") {
        let list = parse_rgb_list("dash_bg_color", v)?;
        if list.is_empty() {
            anyhow::bail!("dash_bg_color: needs at least one color, e.g. #10142A or #10142A, #241030");
        }
        s.bg_colors = list;
    }
    if let Some(v) = file.get("dash_bg_image") {
        s.bg_image = Some(PathBuf::from(v));
    }
    if s.bg == BgMode::Image && s.bg_image.is_none() {
        anyhow::bail!("dash_bg = image needs dash_bg_image = <file>");
    }
    s.bg_dim = num("dash_bg_dim", 0, 100)?.unwrap_or(0);
    if let Some(v) = file.get("deck") {
        s.deck = parse_kinds("deck", v, 5)?;
    }
    if let Some(v) = boolean("deck_gauge")? {
        s.deck_gauge = v;
    }
    if let Some(v) = file.get("cards") {
        s.cards = parse_cards(v)?;
    }
    s.cards_radius = num("cards_radius", 0, 60)?;
    if let Some(v) = num("cards_gap", 0, 40)? {
        s.cards_gap = v;
    }
    if let Some(v) = file.get("orbit") {
        s.orbit = parse_kinds("orbit", v, 4)?;
    }
    if let Some(v) = boolean("orbit_clock")? {
        s.orbit_clock = v;
    }
    if let Some(v) = boolean("orbit_legend")? {
        s.orbit_legend = v;
    }
    Ok(s)
}

// ------------------------------------------------------------------ history

/// The last readings of what the graphs draw, one per second.
pub struct History {
    series: HashMap<&'static str, VecDeque<f32>>,
    last: Option<f64>,
    /// Counts the samples taken (part of the redraw signature).
    rev: u64,
}

impl History {
    fn new() -> Self {
        History { series: HashMap::new(), last: None, rev: 0 }
    }

    /// Takes a sample when a second has passed since the last one.
    fn sample(&mut self, secs: f64, keep: usize, values: &[(&'static str, Option<f32>)]) {
        if self.last.is_some_and(|l| secs - l < 1.0 && secs >= l) {
            return;
        }
        self.last = Some(secs);
        self.rev += 1;
        for (k, v) in values {
            let q = self.series.entry(k).or_default();
            if let Some(v) = v {
                q.push_back(*v);
            }
            while q.len() > keep {
                q.pop_front();
            }
        }
    }

    /// For previews (fixed clock): a plausible wavy history around the current value.
    fn fake(&mut self, keep: usize, values: &[(&'static str, Option<f32>)]) {
        if self.rev > 0 {
            return;
        }
        self.rev = 1;
        for (k, v) in values {
            let Some(v) = v else { continue };
            let seed = (k.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 1000) as f32;
            let q = self.series.entry(k).or_default();
            let amp = (v.abs() * 0.12).max(1.0);
            for i in 0..keep {
                let t = i as f32;
                let w = (t * 0.21 + seed).sin() * 0.6 + (t * 0.07 + seed * 0.5).sin() * 0.4 + (t * 0.55 + seed).sin() * 0.15;
                q.push_back(v + w * amp);
            }
        }
    }

    fn get(&self, key: &str) -> Vec<f32> {
        self.series.get(key).map(|q| q.iter().copied().collect()).unwrap_or_default()
    }
}

// -------------------------------------------------------------------- panels

#[derive(Clone, PartialEq, Debug)]
struct Stat {
    label: String,
    value: String,
    unit: String,
}

#[derive(Clone, PartialEq, Debug)]
struct Ring {
    label: String,
    frac: f32,
}

/// What one column / card / cluster shows, already formatted.
#[derive(Clone, PartialEq, Debug)]
struct Panel {
    kind: PKind,
    title: String,
    sub: String,
    hero: String,
    unit: String,
    frac: Option<f32>,
    /// Name of the history the graph draws, and the smallest range it spans.
    series: Option<(&'static str, f32)>,
    stats: Vec<Stat>,
    rings: Vec<Ring>,
    /// Extra lines of the clock / weather / music panels.
    lines: Vec<String>,
    /// The hero is text (a title), not a number.
    text: bool,
}

fn stat(label: &str, value: impl Into<String>, unit: &str) -> Stat {
    Stat { label: label.to_string(), value: value.into(), unit: unit.to_string() }
}

/// 4350 -> ("4.35", "GHz"), 950 -> ("950", "MHz").
fn freq_parts(mhz: u32) -> (String, &'static str) {
    if mhz >= 1000 {
        (format!("{:.2}", mhz as f32 / 1000.0), "GHz")
    } else {
        (mhz.to_string(), "MHz")
    }
}

fn gb(mb: u64) -> String {
    format!("{:.1}", mb as f32 / 1024.0)
}

fn rate_parts(kb: f64) -> (String, &'static str) {
    if kb >= 1024.0 {
        (format!("{:.1}", kb / 1024.0), "MB/s")
    } else {
        (format!("{kb:.0}"), "KB/s")
    }
}

fn panel(kind: PKind, d: &WidgetData, hw: &lhm::Readings, s: &Settings, ts: &tiles::Settings, now: &chrono::DateTime<Local>) -> Panel {
    let smooth = s.font == FontStyle::Smooth;
    let deg = if smooth { "\u{b0}C" } else { "C" };
    let case = |t: String| if smooth { nice_case(&t) } else { t.to_uppercase() };
    let o = opts();
    let pct = |v: f32| format!("{:.*}", o.percent_decimals as usize, v);
    let mut p = Panel {
        kind,
        title: String::new(),
        sub: String::new(),
        hero: "--".into(),
        unit: String::new(),
        frac: None,
        series: None,
        stats: Vec::new(),
        rings: Vec::new(),
        lines: Vec::new(),
        text: false,
    };
    match kind {
        PKind::Cpu => {
            p.title = "CPU".into();
            match d.cpu_temp {
                Some(t) => {
                    p.hero = format!("{t:.0}");
                    p.unit = deg.into();
                    p.frac = Some(t / 100.0);
                    p.series = Some(("cpu_temp", 12.0));
                }
                None => {
                    p.hero = pct(d.cpu_pct);
                    p.unit = "%".into();
                    p.frac = Some(d.cpu_pct / 100.0);
                    p.series = Some(("cpu_pct", 30.0));
                }
            }
            p.stats.push(stat("LOAD", pct(d.cpu_pct), "%"));
            if let Some(w) = d.cpu_power {
                p.stats.push(stat("POWER", format!("{w:.0}"), "W"));
            }
            if let Some(m) = d.cpu_mhz {
                let (v, u) = freq_parts(m);
                p.stats.push(stat("CLOCK", v, u));
            }
            if let Some(r) = hw.cpu_fan {
                p.stats.push(stat("FAN", r.to_string(), "RPM"));
            }
            p.rings.push(Ring { label: "LOAD".into(), frac: d.cpu_pct / 100.0 });
            if let Some(t) = d.cpu_temp {
                p.rings.push(Ring { label: "TEMP".into(), frac: t / 100.0 });
            }
            if let Some(w) = d.cpu_power {
                p.rings.push(Ring { label: "POWER".into(), frac: w / ts.cpu_watt_max as f32 });
            }
        }
        PKind::Gpu => {
            p.title = "GPU".into();
            match d.gpu_temp {
                Some(t) => {
                    p.hero = t.to_string();
                    p.unit = deg.into();
                    p.frac = Some(t as f32 / 100.0);
                    p.series = Some(("gpu_temp", 12.0));
                }
                None => {
                    if let Some(g) = d.gpu_pct {
                        p.hero = pct(g);
                        p.unit = "%".into();
                        p.frac = Some(g / 100.0);
                        p.series = Some(("gpu_pct", 30.0));
                    }
                }
            }
            if let Some(g) = d.gpu_pct {
                p.stats.push(stat("LOAD", pct(g), "%"));
                p.rings.push(Ring { label: "LOAD".into(), frac: g / 100.0 });
            }
            if let Some(t) = d.gpu_temp {
                p.rings.push(Ring { label: "TEMP".into(), frac: t as f32 / 100.0 });
            }
            if let Some(w) = d.gpu_power {
                p.stats.push(stat("POWER", w.to_string(), "W"));
                p.rings.push(Ring { label: "POWER".into(), frac: w as f32 / ts.gpu_watt_max as f32 });
            }
            if let Some(r) = hw.gpu_fan {
                p.stats.push(stat("FAN", r.to_string(), "RPM"));
            }
            if let Some(f) = d.fps {
                p.stats.push(stat("FPS", f.to_string(), ""));
            }
        }
        PKind::Ram => {
            let pc = if d.total_mb > 0 { d.used_mb as f32 * 100.0 / d.total_mb as f32 } else { 0.0 };
            p.title = i18n::t().mem.to_string();
            p.hero = pct(pc);
            p.unit = "%".into();
            p.frac = Some(pc / 100.0);
            p.series = Some(("ram", 10.0));
            p.sub = format!("of {} GB", gb(d.total_mb));
            p.stats.push(stat("USED", gb(d.used_mb), "GB"));
            p.stats.push(stat("FREE", gb(d.total_mb.saturating_sub(d.used_mb)), "GB"));
            let (rv, ru) = rate_parts(d.disk_mb.0 * 1024.0);
            let (wv, wu) = rate_parts(d.disk_mb.1 * 1024.0);
            p.stats.push(stat("READ", rv, ru));
            p.stats.push(stat("WRITE", wv, wu));
            p.rings.push(Ring { label: "RAM".into(), frac: pc / 100.0 });
            p.rings.push(Ring { label: "DISK".into(), frac: ((d.disk_mb.0 + d.disk_mb.1) / 200.0) as f32 });
            p.rings.push(Ring { label: "NET".into(), frac: ((d.net_kb.0 + d.net_kb.1) / (s.net_max as f64 * 1024.0)) as f32 });
        }
        PKind::Fans => {
            p.title = "FANS".into();
            let rows = hw.fan_rows();
            if let Some((name, rpm)) = rows.first() {
                p.hero = rpm.to_string();
                p.unit = "RPM".into();
                p.frac = Some(*rpm as f32 / ts.fan_max as f32);
                p.series = Some(("fan", 600.0));
                p.sub = name.to_ascii_uppercase();
            }
            for (name, rpm) in rows.iter().skip(1) {
                p.stats.push(stat(&name.to_ascii_uppercase(), rpm.to_string(), "RPM"));
            }
            if let Some(t) = hw.mb_temp {
                p.stats.push(stat("BOARD", format!("{t:.0}"), deg));
            }
            for (name, rpm) in rows.iter().take(4) {
                p.rings.push(Ring { label: name.to_ascii_uppercase(), frac: *rpm as f32 / ts.fan_max as f32 });
            }
        }
        PKind::Net => {
            p.title = i18n::t().net.to_string();
            let (v, u) = rate_parts(d.net_kb.0);
            p.hero = v;
            p.unit = u.into();
            p.frac = Some((d.net_kb.0 / (s.net_max as f64 * 1024.0)) as f32);
            p.series = Some(("net", 400.0));
            p.sub = "DOWN".into();
            let (uv, uu) = rate_parts(d.net_kb.1);
            p.stats.push(stat("UP", uv, uu));
            p.rings.push(Ring { label: "DOWN".into(), frac: p.frac.unwrap_or(0.0) });
            p.rings.push(Ring { label: "UP".into(), frac: (d.net_kb.1 / (s.net_max as f64 * 1024.0)) as f32 });
        }
        PKind::Disk => {
            p.title = i18n::t().disk.to_string();
            let (v, u) = rate_parts(d.disk_mb.0 * 1024.0);
            p.hero = v;
            p.unit = u.into();
            p.frac = Some((d.disk_mb.0 / 200.0) as f32);
            p.series = Some(("disk", 5.0));
            p.sub = "READ".into();
            let (wv, wu) = rate_parts(d.disk_mb.1 * 1024.0);
            p.stats.push(stat("WRITE", wv, wu));
            if let Some(t) = hw.disk_temp {
                p.stats.push(stat("SSD", format!("{t:.0}"), deg));
            }
            p.rings.push(Ring { label: "READ".into(), frac: (d.disk_mb.0 / 200.0) as f32 });
            p.rings.push(Ring { label: "WRITE".into(), frac: (d.disk_mb.1 / 200.0) as f32 });
        }
        PKind::Clock => {
            p.title = case(i18n::weekday(now));
            p.sub = case(i18n::day_month_year(now));
            p.hero = now.format(if ts.clock_seconds { "%H:%M:%S" } else { "%H:%M" }).to_string();
            if let Some(w) = &d.weather {
                let f = o.weather_fahrenheit;
                let unit = if smooth { if f { "\u{b0}F" } else { "\u{b0}C" } } else if f { "F" } else { "C" };
                let city = w.city.clone().map(|c| format!(" - {c}")).unwrap_or_default();
                p.lines.push(format!("{:.0}{} {}", w.temp_in(f), unit, case(format!("{}{}", weather_icon::condition_name(w.icon), city))));
            }
            if let Some(t) = &d.now_playing {
                p.lines.push(t.clone());
            }
        }
        PKind::Weather => {
            p.text = false;
            p.title = i18n::t().weather.to_string();
            if let Some(w) = &d.weather {
                let f = o.weather_fahrenheit;
                p.hero = format!("{:.0}", w.temp_in(f));
                p.unit = if smooth { if f { "\u{b0}F" } else { "\u{b0}C" } } else if f { "F" } else { "C" }.into();
                p.frac = Some(w.temp_in(f) / if f { 120.0 } else { 50.0 });
                p.sub = case(weather_icon::condition_name(w.icon).to_string());
                if let Some(c) = &w.city {
                    p.lines.push(c.clone());
                }
                if let Some(h) = w.humidity {
                    p.stats.push(stat("HUMIDITY", h.to_string(), "%"));
                }
            }
        }
        PKind::Music => {
            p.title = i18n::t().now_playing.to_string();
            p.text = true;
            p.hero = d.now_playing.clone().unwrap_or_else(|| "-".into());
        }
    }
    p.frac = p.frac.map(|f| f.clamp(0.0, 1.0));
    for r in &mut p.rings {
        r.frac = r.frac.clamp(0.0, 1.0);
    }
    p
}

// ----------------------------------------------------------------- drawing

fn luma(c: Rgb) -> f32 {
    c.0 as f32 * 0.299 + c.1 as f32 * 0.587 + c.2 as f32 * 0.114
}

/// The `i`-th accent colour; on the light theme the pale ones are deepened so they show on white.
fn accent(s: &Settings, i: usize) -> Rgb {
    let c = s.colors[i % s.colors.len().max(1)];
    if s.theme == Theme::Light && luma(c) > 150.0 {
        darken(c, ((luma(c) - 150.0) / 255.0).clamp(0.0, 0.3) + 0.08)
    } else {
        c
    }
}

fn is_light(s: &Settings) -> bool {
    s.theme == Theme::Light
}

/// Text colour by theme (or `dash_text_color`).
fn text_color(s: &Settings) -> Rgb {
    s.text.unwrap_or(if is_light(s) { (0x1B, 0x22, 0x36) } else { (0xFF, 0xFF, 0xFF) })
}

/// Plate behind deck / orbit: colour and alpha.
fn plate(s: &Settings) -> (Rgb, u8) {
    let rgb = s.plate_color.unwrap_or(if is_light(s) { (0xFF, 0xFF, 0xFF) } else { (6, 8, 16) });
    (rgb, (s.plate.unwrap_or(if is_light(s) { 92 } else { 50 }) * 255 / 100) as u8)
}

/// Lines and outlines: pale on the dark theme, dark on the light one. (colour, alpha for outline, alpha for hairline)
fn line_style(s: &Settings) -> (Rgb, u8, u8) {
    if is_light(s) { ((0x1B, 0x22, 0x36), 60, 46) } else { ((255, 255, 255), 34, 30) }
}

fn darken(c: Rgb, t: f32) -> Rgb {
    lerp_rgb(c, (0, 0, 0), t)
}

impl Layer {
    /// A soft dot.
    fn dot(&mut self, cx: f32, cy: f32, r: f32, rgb: Rgb, alpha: u8) {
        let (x0, x1, y0, y1) = ((cx - r - 1.0).floor() as i32, (cx + r + 1.0).ceil() as i32, (cy - r - 1.0).floor() as i32, (cy + r + 1.0).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let d = (px as f32 + 0.5 - cx).hypot(py as f32 + 0.5 - cy);
                let cov = (r + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.over(px, py, rgb, (alpha as f32 * cov).round() as u8);
                }
            }
        }
    }

    /// A graph: a line with a fading area under it. `span_min` = the smallest range the vertical
    /// axis covers (so a quiet reading does not turn into noise).
    fn spark(&mut self, rect: (f32, f32, f32, f32), data: &[f32], span_min: f32, rgb: Rgb, thick: f32, fill_alpha: u8, line_alpha: u8, end_dot: bool) {
        let (x, y, w, h) = rect;
        if data.len() < 2 || w < 6.0 || h < 6.0 {
            return;
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for v in data {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        let span = (hi - lo).max(span_min);
        let mid = (hi + lo) / 2.0;
        let (lo, hi) = (mid - span / 2.0, mid + span / 2.0);
        let n = data.len().min((w / 2.0) as usize).max(2);
        let inset = thick + 1.0;
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let f = i as f32 / (n - 1) as f32;
                let src = f * (data.len() - 1) as f32;
                let (a, b) = (src.floor() as usize, (src.floor() as usize + 1).min(data.len() - 1));
                let v = data[a] + (data[b] - data[a]) * (src - a as f32);
                (x + f * w, y + h - inset - (v - lo) / (hi - lo) * (h - 2.0 * inset))
            })
            .collect();
        // area under the line
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
        // the line
        let half = thick / 2.0;
        for pair in pts.windows(2).filter(|_| line_alpha > 0) {
            let (a, b) = (pair[0], pair[1]);
            let (x0, x1) = ((a.0.min(b.0) - half - 1.0).floor() as i32, (a.0.max(b.0) + half + 1.0).ceil() as i32);
            let (y0, y1) = ((a.1.min(b.1) - half - 1.0).floor() as i32, (a.1.max(b.1) + half + 1.0).ceil() as i32);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = (dx * dx + dy * dy).max(0.0001);
            for py in y0..y1 {
                for px in x0..x1 {
                    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let t = (((fx - a.0) * dx + (fy - a.1) * dy) / len2).clamp(0.0, 1.0);
                    let d = (fx - (a.0 + dx * t)).hypot(fy - (a.1 + dy * t));
                    let cov = (half + 0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.over(px, py, rgb, (line_alpha as f32 * cov).round() as u8);
                    }
                }
            }
        }
        if end_dot {
            if let Some(last) = pts.last() {
                self.dot(last.0, last.1, thick * 1.5 + 1.0, rgb, 70);
                self.dot(last.0, last.1, thick * 0.9 + 0.8, rgb, 255);
            }
        }
    }
}

/// Everything the three styles share while drawing.
struct Ctx<'a> {
    s: &'a Settings,
    hist: &'a History,
    text: Rgb,
    color_mode: ColorMode,
}

fn value_unit(p: &mut Painter, x: f32, base: f32, value: &str, unit: &str, cap: f32, w: W, color: Rgb, alpha: u8, unit_alpha: u8) -> f32 {
    let vw = p.text(x, base, value, cap, w, color, alpha);
    if unit.is_empty() {
        return vw;
    }
    let ucap = (cap * 0.42).max(7.0);
    let gap = (cap * 0.1).max(2.0);
    vw + gap + p.text(x + vw + gap, base, unit, ucap, W::Regular, color, unit_alpha)
}

fn vu_width(p: &Painter, value: &str, unit: &str, cap: f32, w: W) -> f32 {
    let mut t = p.width(value, cap, w);
    if !unit.is_empty() {
        let ucap = (cap * 0.42).max(7.0);
        t += (cap * 0.1).max(2.0) + p.width(unit, ucap, W::Regular);
    }
    t
}

/// The biggest cap (<= `cap`) at which value + unit fit in `max_w`.
fn fit_vu(p: &Painter, value: &str, unit: &str, cap: f32, max_w: f32, w: W) -> f32 {
    let mut c = cap;
    for _ in 0..40 {
        if vu_width(p, value, unit, c, w) <= max_w || c <= 6.0 {
            break;
        }
        c = if p.font == FontStyle::Pixel { (c - 7.0).max(7.0) } else { (c * 0.94).floor() };
    }
    c
}

fn clip(p: &Painter, text: &str, cap: f32, max_w: f32, w: W) -> String {
    if p.width(text, cap, w) <= max_w {
        return text.to_string();
    }
    let mut t: String = text.to_string();
    while t.chars().count() > 3 && p.width(&format!("{t}..."), cap, w) > max_w {
        t.pop();
    }
    format!("{}...", t.trim_end())
}

fn graph_data(c: &Ctx, panel: &Panel) -> Option<(Vec<f32>, f32)> {
    let (key, span) = panel.series?;
    let d = c.hist.get(key);
    (c.s.graph && d.len() >= 2).then_some((d, span))
}

/// A small label over a value (+ unit); returns the width used.
fn stat_cell(p: &mut Painter, c: &Ctx, x: f32, y: f32, max_w: f32, st: &Stat, vcap: f32, ink: Rgb, label_alpha: u8) -> f32 {
    let lcap = (vcap * 0.4).max(7.0);
    p.text(x, y + lcap, &clip(p, &st.label, lcap, max_w, W::Regular), lcap, W::Regular, ink, label_alpha);
    let vcap = fit_vu(p, &st.value, &st.unit, vcap, max_w, W::Regular);
    let base = y + lcap + vcap * 1.22;
    let _ = c;
    value_unit(p, x, base, &st.value, &st.unit, vcap, W::Regular, ink, 255, label_alpha)
}

// -- deck

fn draw_deck(p: &mut Painter, c: &Ctx, panels: &[Panel], area: (f32, f32, f32, f32)) {
    let (x, y, w, h) = area;
    let radius = (h * 0.06).clamp(10.0, 26.0);
    let (plate_rgb, plate_alpha) = plate(c.s);
    let (line, outline_a, hair_a) = line_style(c.s);
    if plate_alpha > 0 {
        p.layer.fill_rrect_solid(area, radius, plate_rgb, plate_alpha);
    }
    p.layer.stroke_rrect(area, radius, 1.5, line, outline_a);
    let weights: Vec<f32> = panels.iter().map(|q| q.kind.weight()).collect();
    let cols = split(w as u32, 0, &weights);
    for (i, (panel, (off, cw))) in panels.iter().zip(cols).enumerate() {
        let cx = x + off as f32;
        let cw = cw as f32;
        let acc = accent(c.s, i);
        if i > 0 {
            p.layer.fill_rrect_solid((cx.round(), y + h * 0.08, 1.0, h * 0.84), 0.0, line, hair_a);
        }
        deck_column(p, c, panel, acc, (cx, y, cw, h));
    }
}

fn deck_column(p: &mut Painter, c: &Ctx, panel: &Panel, acc: Rgb, (x, y, w, h): (f32, f32, f32, f32)) {
    let pad = (h * 0.075).clamp(9.0, 26.0);
    let inner = w - 2.0 * pad;
    let ink = c.text;
    // the live gauge on top: a short line that grows with the main value
    if c.s.deck_gauge {
        if let Some(f) = panel.frac {
            let gy = y + (h * 0.012).max(3.0);
            let gh = (h * 0.012).clamp(2.0, 4.0);
            p.layer.fill_rrect_solid((x + pad, gy, inner, gh), gh / 2.0, ink, 28);
            let fw = (inner * f).max(gh);
            p.layer.fill_rrect((x + pad, gy, fw, gh), gh / 2.0, &move |t| lerp_rgb(acc, (255, 255, 255), t * 0.35), 255);
        }
    }
    let lcap = (h * 0.052).clamp(8.0, 17.0);
    let lbase = y + pad + lcap + 4.0;
    p.layer.dot(x + pad + lcap * 0.32, lbase - lcap * 0.5, lcap * 0.32, acc, 255);
    let title_x = x + pad + lcap * 0.9;
    p.text(title_x, lbase, &clip(p, &panel.title, lcap, inner - lcap, W::Medium), lcap, W::Medium, ink, 170);
    if panel.kind == PKind::Clock {
        deck_clock(p, c, panel, acc, (x, y, w, h), pad, lbase);
        return;
    }
    // hero value (+ graph to its right)
    let graph = graph_data(c, panel);
    let top = lbase + pad * 0.6;
    let hero_zone = h * 0.30;
    let hero_w = if graph.is_some() && !panel.text { inner * 0.6 } else { inner };
    let tw = if p.font == FontStyle::Smooth { W::Light } else { W::Regular };
    let hero_cap = if panel.text {
        (hero_zone * 0.30).min(lcap * 2.0)
    } else {
        fit_vu(p, &panel.hero, &panel.unit, hero_zone * 0.86, hero_w, tw)
    };
    let hero_base = top + hero_zone * 0.1 + hero_cap;
    if panel.text {
        let t = clip(p, &panel.hero, hero_cap, inner, W::Regular);
        p.text(x + pad, hero_base, &t, hero_cap, W::Regular, ink, 255);
    } else {
        value_unit(p, x + pad, hero_base, &panel.hero, &panel.unit, hero_cap, tw, ink, 255, 150);
    }
    if !panel.sub.is_empty() {
        let sc = lcap * 0.9;
        p.text(x + pad, hero_base + sc * 1.7, &clip(p, &panel.sub, sc, inner, W::Regular), sc, W::Regular, ink, 130);
    }
    if let Some((data, span)) = &graph {
        let gx = x + pad + hero_w + pad * 0.3;
        let gw = x + w - pad - gx;
        p.layer.spark((gx, top, gw, hero_zone * 0.78), data, *span, acc, (h * 0.006).clamp(1.5, 3.0), 70, 255, true);
    }
    // 2x2 grid of readings
    let grid_top = top + hero_zone + pad * 0.9 + if panel.sub.is_empty() { 0.0 } else { lcap * 0.9 };
    let rem = (y + h - pad - grid_top).max(10.0);
    let n = panel.stats.len().min(4);
    if n == 0 {
        return;
    }
    let rows = n.div_ceil(2);
    let row_h = rem / rows as f32;
    let vcap = (row_h * 0.46).min(hero_cap * 0.6).max(9.0);
    let cell_w = (inner - pad) / 2.0;
    for (i, st) in panel.stats.iter().take(4).enumerate() {
        let (col, row) = (i % 2, i / 2);
        stat_cell(p, c, x + pad + col as f32 * (cell_w + pad), grid_top + row as f32 * row_h, cell_w, st, vcap, ink, 140);
    }
}

fn deck_clock(p: &mut Painter, c: &Ctx, panel: &Panel, acc: Rgb, (x, y, w, h): (f32, f32, f32, f32), pad: f32, lbase: f32) {
    let inner = w - 2.0 * pad;
    let ink = c.text;
    let lcap = (h * 0.052).clamp(8.0, 17.0);
    let top = lbase + pad * 0.4;
    let tw = if p.font == FontStyle::Smooth { W::Light } else { W::Regular };
    let cap = p.fit(&panel.hero, h * 0.30, inner, tw);
    let base = top + cap * 1.08;
    p.text(x + pad, base, &panel.hero, cap, tw, ink, 255);
    let sc = lcap * 1.05;
    p.text(x + pad, base + sc * 1.9, &clip(p, &panel.sub, sc, inner, W::Regular), sc, W::Regular, ink, 160);
    // weather and now playing, one line each, with a dot in the accent
    let mut ly = base + sc * 1.9 + pad * 1.1;
    for (i, line) in panel.lines.iter().enumerate() {
        if ly > y + h - pad * 0.6 {
            break;
        }
        let lc = lcap * 1.15;
        if i > 0 {
            ly += pad * 0.2;
        }
        p.text(x + pad, ly, &clip(p, line, lc, inner, W::Regular), lc, W::Regular, ink, if i == 0 { 230 } else { 170 });
        ly += lc * 1.9;
    }
    let _ = acc;
}

// -- cards

fn draw_cards(p: &mut Painter, c: &Ctx, cols: &[Vec<PKind>], panels: &[Vec<Panel>], area: (f32, f32, f32, f32)) {
    let (x, y, w, h) = area;
    let gap = c.s.cards_gap;
    let weights: Vec<f32> = cols.iter().map(|col| if col.len() == 1 { col[0].weight() * 1.15 } else { 1.0 }).collect();
    let xs = split(w as u32, gap, &weights);
    let mut idx = 0usize;
    for ((stack, panels), (off, cw)) in cols.iter().zip(panels).zip(xs) {
        let n = stack.len() as u32;
        let ys = split(h as u32, gap, &vec![1.0; n as usize]);
        for (panel, (yoff, ch)) in panels.iter().zip(ys) {
            let rect = (x + off as f32, y + yoff as f32, cw as f32, ch as f32);
            card(p, c, panel, idx, rect);
            idx += 1;
        }
    }
}

fn card(p: &mut Painter, c: &Ctx, panel: &Panel, idx: usize, (x, y, w, h): (f32, f32, f32, f32)) {
    if w < 60.0 || h < 40.0 {
        return;
    }
    let radius = c.s.cards_radius.map_or_else(|| (w.min(h) * 0.16).clamp(10.0, 34.0), |r| r as f32);
    let paper = panel.kind == PKind::Clock;
    let light = is_light(c.s);
    // on the light theme the cards are pastel versions of the accents
    let tone = |col: Rgb| if light { lerp_rgb(col, (255, 255, 255), 0.5) } else { col };
    let a = if paper { if light { (0xFF, 0xFF, 0xFF) } else { (0xF3, 0xF1, 0xEC) } } else { tone(accent(c.s, idx)) };
    // melts toward the next card's colour
    let b = if paper { (0xE2, 0xDF, 0xD8) } else { lerp_rgb(a, tone(accent(c.s, idx + 1)), 0.38) };
    let b = if light { darken(b, 0.04) } else { darken(b, 0.1) };
    let alpha = (c.s.cards_opacity.min(100) * 255 / 100) as u8;
    p.layer.fill_rrect((x, y, w, h), radius, &move |t| lerp_rgb(a, b, t), alpha);
    let (line, _, _) = line_style(c.s);
    p.layer.stroke_rrect((x, y, w, h), radius, 1.2, line, 40);
    // dark text on light cards; when the cards are see-through the text follows the theme, since
    // what is behind them is the (dark) background
    let dark_ink = if light { true } else if c.s.cards_opacity < 65 { false } else { luma(lerp_rgb(a, b, 0.5)) > 140.0 };
    let ink: Rgb = if dark_ink { (0x13, 0x15, 0x22) } else { (0xFF, 0xFF, 0xFF) };
    let pad = (h.min(w * 0.6) * 0.1).clamp(9.0, 24.0);
    let inner = w - 2.0 * pad;
    let compact = h < 135.0;
    // ghost silhouette of the history along the bottom
    if let Some((data, span)) = graph_data(c, panel) {
        let gh = if compact { h * 0.5 } else { h * 0.46 };
        let rect = (x + 1.0, y + h - gh - 1.0, w - 2.0, gh);
        // clip by drawing inside the rounded corner: the area is faded at the bottom, the corners
        // are trimmed afterwards by the card outline, so keep a margin
        let rect = (rect.0 + radius * 0.25, rect.1, rect.2 - radius * 0.5, rect.3 - radius * 0.3);
        p.layer.spark(rect, &data, span, ink, 1.0, 54, 0, false);
    }
    let lcap = (h * 0.085).clamp(8.0, 17.0);
    let lbase = y + pad + lcap;
    let lw = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    let sub_w = if panel.sub.is_empty() { 0.0 } else { p.width(&panel.sub, lcap * 0.9, W::Regular) };
    let show_sub = !panel.sub.is_empty() && sub_w + pad < inner * 0.5 && panel.kind != PKind::Clock;
    p.text(x + pad, lbase, &clip(p, &panel.title, lcap, inner - if show_sub { sub_w + pad } else { 0.0 }, lw), lcap, lw, ink, 175);
    if show_sub {
        p.text(x + w - pad - sub_w, lbase, &panel.sub, lcap * 0.9, W::Regular, ink, 130);
    }
    if panel.kind == PKind::Clock {
        let tw = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
        let cap = p.fit(&panel.hero, h * 0.3, inner, tw);
        let base = lbase + pad * 0.5 + lcap * 1.3 + cap * 1.05;
        let sc = lcap * 0.95;
        p.text(x + pad, lbase + sc * 1.8, &clip(p, &panel.sub, sc, inner, W::Regular), sc, W::Regular, ink, 150);
        p.text(x + pad, base + sc * 1.2, &panel.hero, cap, tw, ink, 255);
        let mut ly = base + sc * 1.2 + pad * 1.2;
        for line in &panel.lines {
            if ly > y + h - pad * 0.4 {
                break;
            }
            p.text(x + pad, ly, &clip(p, line, sc, inner, W::Regular), sc, W::Regular, ink, 190);
            ly += sc * 1.8;
        }
        return;
    }
    // hero
    let hero_top = lbase + pad * 0.35;
    let stats_h = if compact { 0.0 } else { h * 0.26 };
    let avail = (y + h - pad - stats_h - hero_top).max(12.0);
    let tw = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    if panel.text {
        let cap = (avail * 0.5).clamp(10.0, lcap * 2.2);
        let t = clip(p, &panel.hero, cap, inner, W::Regular);
        p.text(x + pad, hero_top + (avail + cap) / 2.0, &t, cap, W::Regular, ink, 255);
        return;
    }
    // in a compact card the readings sit to the right of the value
    let right_w = if compact && !panel.stats.is_empty() { inner * 0.36 } else { 0.0 };
    let cap = fit_vu(p, &panel.hero, &panel.unit, (avail * 0.9).min(h * 0.5), inner - right_w - if right_w > 0.0 { pad } else { 0.0 }, tw);
    let base = hero_top + (avail + cap) / 2.0;
    value_unit(p, x + pad, base, &panel.hero, &panel.unit, cap, tw, ink, 255, 150);
    if compact {
        if let Some(st) = panel.stats.first() {
            let vcap = (h * 0.15).clamp(9.0, 22.0);
            stat_cell(p, c, x + w - pad - right_w, hero_top + (avail - vcap * 2.2) / 2.0, right_w, st, vcap, ink, 150);
        }
        return;
    }
    // up to three readings in a row at the bottom
    let n = panel.stats.len().min(3);
    if n > 0 {
        let cell = (inner - pad * (n as f32 - 1.0)) / n as f32;
        let vcap = (stats_h * 0.42).clamp(9.0, 26.0);
        for (i, st) in panel.stats.iter().take(3).enumerate() {
            stat_cell(p, c, x + pad + i as f32 * (cell + pad), y + h - pad - stats_h * 0.92, cell, st, vcap, ink, 150);
        }
    }
}

// -- orbit

fn draw_orbit(p: &mut Painter, c: &Ctx, panels: &[Panel], clock: Option<&Panel>, area: (f32, f32, f32, f32)) {
    let (x, y, w, h) = area;
    let radius = (h * 0.06).clamp(10.0, 26.0);
    let (plate_rgb, plate_alpha) = plate(c.s);
    let (line, outline_a, hair_a) = line_style(c.s);
    if plate_alpha > 0 {
        p.layer.fill_rrect_solid(area, radius, plate_rgb, plate_alpha);
    }
    p.layer.stroke_rrect(area, radius, 1.5, line, outline_a);
    let mut weights: Vec<f32> = vec![1.0; panels.len()];
    if clock.is_some() {
        weights.push(0.72);
    }
    let slots = split(w as u32, 0, &weights);
    let mut ring_color = 0usize;
    for (i, (panel, (off, sw))) in panels.iter().zip(slots.iter()).enumerate() {
        if i > 0 {
            p.layer.fill_rrect_solid((x + *off as f32, y + h * 0.1, 1.0, h * 0.8), 0.0, line, hair_a);
        }
        orbit_cluster(p, c, panel, ring_color, (x + *off as f32, y, *sw as f32, h));
        ring_color += panel.rings.len().max(1);
    }
    if let (Some(cp), Some((off, sw))) = (clock, slots.last()) {
        if !panels.is_empty() {
            p.layer.fill_rrect_solid((x + *off as f32, y + h * 0.1, 1.0, h * 0.8), 0.0, line, hair_a);
        }
        let pad = (h * 0.075).clamp(9.0, 26.0);
        let lcap = (h * 0.052).clamp(8.0, 17.0);
        let lbase = y + pad + lcap + 4.0;
        deck_clock_title(p, c, cp, (x + *off as f32, y, *sw as f32, h), pad, lbase);
        deck_clock(p, c, cp, accent(c.s, ring_color), (x + *off as f32, y, *sw as f32, h), pad, lbase);
    }
}

fn deck_clock_title(p: &mut Painter, c: &Ctx, panel: &Panel, (x, _y, w, _h): (f32, f32, f32, f32), pad: f32, lbase: f32) {
    let lcap = (_h * 0.052).clamp(8.0, 17.0);
    let inner = w - 2.0 * pad;
    p.text(x + pad, lbase, &clip(p, &panel.title, lcap, inner, W::Medium), lcap, W::Medium, c.text, 170);
}

fn orbit_cluster(p: &mut Painter, c: &Ctx, panel: &Panel, color0: usize, (x, y, w, h): (f32, f32, f32, f32)) {
    let pad = (h * 0.075).clamp(9.0, 24.0);
    let ink = c.text;
    let lcap = (h * 0.052).clamp(8.0, 17.0);
    // the name under the rings, the legend under the name
    let foot = lcap * 3.6 + pad * 0.4;
    let ring_h = (h - 2.0 * pad - foot).max(40.0);
    let wide = w > ring_h * 1.55;
    let ring_w = if wide { w * 0.5 } else { w - 2.0 * pad };
    let r = (ring_h / 2.0).min(ring_w / 2.0);
    let cx = if wide { x + pad + r } else { x + w / 2.0 };
    let cy = y + pad + ring_h / 2.0;
    let rings = &panel.rings;
    let n = rings.len().max(1);
    let thick = (r * 0.54 / (n as f32 + 0.3 * (n as f32 - 1.0))).min(r * 0.2).max(3.0);
    let gap = thick * 0.3;
    for (i, ring) in rings.iter().enumerate() {
        let r_out = r - i as f32 * (thick + gap);
        let col = accent(c.s, color0 + i);
        p.layer.arc(cx, cy, r_out, thick, 1.0, &move |_| col, 34);
        let c2 = lerp_rgb(col, (255, 255, 255), 0.3);
        p.layer.arc(cx, cy, r_out, thick, ring.frac.max(0.012), &move |t| lerp_rgb(col, c2, t), 255);
    }
    // the centre
    let inner_r = r - n as f32 * (thick + gap) + gap;
    let tw = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    let hero = if panel.text { clip(p, &panel.hero, lcap * 1.4, inner_r * 1.8, W::Regular) } else { panel.hero.clone() };
    let unit = if panel.text { "" } else { panel.unit.as_str() };
    let cap = fit_vu(p, &hero, unit, (inner_r * 0.8).max(10.0), inner_r * 1.65, tw);
    let tw_px = vu_width(p, &hero, unit, cap, tw);
    value_unit(p, cx - tw_px / 2.0, cy + cap / 2.0, &hero, unit, cap, tw, ink, 255, 150);
    // the readings beside (wide slot) or under (narrow slot) the rings
    if wide {
        let sx = cx + r + pad * 1.4;
        let sw = x + w - pad - sx;
        let n = panel.stats.len().min(4);
        if n > 0 && sw > 30.0 {
            let row_h = (ring_h / n as f32).min(ring_h / 3.0);
            let vcap = (row_h * 0.4).clamp(9.0, 24.0);
            let top = cy - row_h * n as f32 / 2.0;
            for (i, st) in panel.stats.iter().take(4).enumerate() {
                stat_cell(p, c, sx, top + i as f32 * row_h + (row_h - vcap * 1.95) / 2.0, sw, st, vcap, ink, 140);
            }
        }
    }
    // name, then the legend of the rings
    let name_cap = lcap * 1.15;
    let name_base = y + pad + ring_h + pad * 0.5 + name_cap;
    let name = if panel.sub.is_empty() || panel.kind == PKind::Fans { panel.title.clone() } else { format!("{}  {}", panel.title, panel.sub) };
    let nw = p.width(&name, name_cap, W::Medium);
    let name_cap2 = if nw > w - 2.0 * pad { p.fit(&name, name_cap, w - 2.0 * pad, W::Medium) } else { name_cap };
    let nw = p.width(&name, name_cap2, W::Medium);
    p.text(x + (w - nw) / 2.0, name_base, &name, name_cap2, W::Medium, ink, 235);
    if c.s.orbit_legend && !panel.rings.is_empty() {
        let lc = lcap * 0.85;
        let items: Vec<(&str, Rgb)> = panel.rings.iter().enumerate().map(|(i, r)| (r.label.as_str(), accent(c.s, color0 + i))).collect();
        let dot = lc * 0.55;
        let total: f32 = items.iter().map(|(l, _)| dot * 2.0 + lc * 0.5 + p.width(l, lc, W::Regular) + lc * 1.2).sum::<f32>() - lc * 1.2;
        let mut lx = x + (w - total.min(w - pad)) / 2.0;
        let ly = name_base + lc * 1.9;
        for (l, col) in items {
            p.layer.dot(lx + dot, ly - lc * 0.35, dot, col, 255);
            lx += dot * 2.0 + lc * 0.5;
            lx += p.text(lx, ly, l, lc, W::Regular, ink, 160) + lc * 1.2;
        }
    }
}

// ---------------------------------------------------------------------- screen

pub struct DashScreen {
    started: std::time::Instant,
    layer: Layer,
    sig: String,
    hist: History,
    /// Paints the background (the same engine as the tiles).
    bg: TilesScreen,
    pub redraws: u32,
}

impl DashScreen {
    pub fn new() -> Self {
        DashScreen { started: std::time::Instant::now(), layer: Layer::new(1, 1), sig: String::new(), hist: History::new(), bg: TilesScreen::new(), redraws: 0 }
    }

    /// Frames per second the screen wants: the graphs move once a second, a background effect
    /// or image needs its own rate.
    pub fn wanted_fps(s: &Settings, effect_fps: f32, active_fps: f32) -> f32 {
        match s.bg {
            BgMode::Effect => effect_fps,
            BgMode::Image => active_fps,
            _ => 4.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, fb: &mut Framebuffer, area: Rect, style: Style, data: &WidgetData, color_mode: ColorMode, s: &Settings, ts: &tiles::Settings, env: &Env) {
        // the background, through a Tiles settings value that carries only the background
        let bg = tiles::Settings { bg: s.bg, bg_colors: s.bg_colors.clone(), bg_image: s.bg_image.clone(), bg_dim: s.bg_dim, ..tiles::Settings::default() };
        self.bg.paint_background(fb, &bg, env);
        let (ax, ay, aw, ah) = area;
        let inset = (ah as f32 * 0.03).clamp(6.0, 16.0);
        let rect = (inset, inset, aw as f32 - 2.0 * inset, ah as f32 - 2.0 * inset);
        if rect.2 < 120.0 || rect.3 < 80.0 {
            return;
        }
        let now = Local::now();
        let hw = lhm::current();
        let secs = env.secs.unwrap_or_else(|| self.started.elapsed().as_secs_f64());
        let ram_pct = if data.total_mb > 0 { data.used_mb as f32 * 100.0 / data.total_mb as f32 } else { 0.0 };
        let sampled: [(&'static str, Option<f32>); 8] = [
            ("cpu_temp", data.cpu_temp),
            ("cpu_pct", Some(data.cpu_pct)),
            ("gpu_temp", data.gpu_temp.map(|t| t as f32)),
            ("gpu_pct", data.gpu_pct),
            ("ram", Some(ram_pct)),
            ("fan", hw.cpu_fan.or(hw.case_fan).or(hw.pump).or(hw.gpu_fan).map(|v| v as f32)),
            ("net", Some(data.net_kb.0 as f32)),
            ("disk", Some(data.disk_mb.0 as f32)),
        ];
        let keep = s.history as usize;
        if env.secs.is_some() {
            self.hist.fake(keep.min(90), &sampled);
        } else {
            self.hist.sample(secs, keep, &sampled);
        }
        let mk = |k: PKind| panel(k, data, &hw, s, ts, &now);
        let (kinds_flat, clock_panel): (Vec<PKind>, Option<Panel>) = match style {
            Style::Deck => (s.deck.clone(), None),
            Style::Cards => (s.cards.iter().flatten().copied().collect(), None),
            Style::Orbit => (s.orbit.clone(), s.orbit_clock.then(|| mk(PKind::Clock))),
        };
        let panels: Vec<Panel> = kinds_flat.iter().map(|k| mk(*k)).collect();
        let sig = format!("{style:?}|{s:?}|{area:?}|{panels:?}|{clock_panel:?}|{}|{:?}", self.hist.rev, color_mode);
        if sig != self.sig {
            if self.layer.w != aw as usize || self.layer.h != ah as usize {
                self.layer = Layer::new(aw, ah);
            } else {
                self.layer.clear();
            }
            let text = text_color(s);
            let ctx = Ctx { s, hist: &self.hist, text, color_mode };
            let mut painter = Painter { layer: &mut self.layer, font: s.font };
            match style {
                Style::Deck => draw_deck(&mut painter, &ctx, &panels, rect),
                Style::Cards => {
                    let mut it = panels.iter().cloned();
                    let grouped: Vec<Vec<Panel>> = s.cards.iter().map(|col| col.iter().filter_map(|_| it.next()).collect()).collect();
                    draw_cards(&mut painter, &ctx, &s.cards, &grouped, rect);
                }
                Style::Orbit => draw_orbit(&mut painter, &ctx, &panels, clock_panel.as_ref(), rect),
            }
            self.sig = sig;
            self.redraws += 1;
        }
        self.layer.composite_onto(fb, ax, ay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cfg(txt: &str) -> anyhow::Result<Settings> {
        parse(&ConfigFile::parse(txt).unwrap())
    }

    #[test]
    fn options_parse_and_are_checked() {
        let d = parse_cfg("").unwrap();
        assert_eq!(d, Settings::default());
        let s = parse_cfg("deck = cpu, fans, clock\ncards = clock | cpu, gpu | music\norbit = gpu, ram\norbit_clock = false\ndash_colors = #FF0000, #00FF00\ndash_history = 120\ndash_graph = false\n").unwrap();
        assert_eq!(s.deck, vec![PKind::Cpu, PKind::Fans, PKind::Clock]);
        assert_eq!(s.cards, vec![vec![PKind::Clock], vec![PKind::Cpu, PKind::Gpu], vec![PKind::Music]]);
        assert_eq!(s.orbit, vec![PKind::Gpu, PKind::Ram]);
        assert!(!s.orbit_clock && !s.graph);
        assert_eq!((s.colors.len(), s.history), (2, 120));
        assert!(parse_cfg("deck = cpu, tv\n").is_err());
        assert!(parse_cfg("deck = \n").is_err());
        assert!(parse_cfg("cards = a | b\n").is_err());
        assert!(parse_cfg("dash_history = 5\n").is_err());
        assert!(parse_cfg("dash_palette = nonsense\n").is_err());
        assert!(parse_cfg("dash_bg = image\n").is_err());
        assert_eq!(parse_cfg("dash_palette = ocean\n").unwrap().colors.len(), 6);
        let l = parse_cfg("dash_theme = light\ndash_plate_color = #F0F0F0\ndash_plate = 30\ncards_opacity = 40\n").unwrap();
        assert_eq!((l.theme, l.plate_color, l.plate, l.cards_opacity), (Theme::Light, Some((0xF0, 0xF0, 0xF0)), Some(30), 40));
        assert_eq!(Settings::default().plate, None);
        assert!(parse_cfg("dash_theme = grey\n").is_err());
        assert!(parse_cfg("cards_opacity = 101\n").is_err());
        assert!(parse_cfg("dash_plate_color = default\n").is_err());
    }

    #[test]
    fn history_takes_one_sample_a_second_and_keeps_the_last_ones() {
        let mut h = History::new();
        for i in 0..10 {
            h.sample(i as f64 * 0.4, 5, &[("a", Some(i as f32))]);
        }
        // 0.0, 1.2, 2.0(+0.8), ... : about one per second
        let a = h.get("a");
        assert!(a.len() <= 5 && a.len() >= 3, "{a:?}");
        assert!(a.windows(2).all(|w| w[0] < w[1]));
    }

    fn data() -> WidgetData {
        WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: Some("Artist - A very long song title that has to be cut with dots".into()),
            weather: Some(crate::weather::WeatherSnapshot {
                temp_c: 21.0, humidity: Some(55), code: 61,
                icon: weather_icon::WeatherIcon::Rain, city: Some("Milano".into()), forecast: Vec::new(),
            }),
            ticker: Vec::new(),
        }
    }

    fn painted(fb: &Framebuffer) -> usize {
        fb.as_bytes().chunks_exact(3).filter(|p| p[0] != 8 || p[1] != 8 || p[2] != 16).count()
    }

    #[test]
    fn every_style_draws_on_every_panel_with_and_without_readings() {
        let eff = effect::Settings::default();
        let env = Env { fps: 30.0, ffmpeg: None, effect: &eff, secs: Some(0.0) };
        let ts = tiles::Settings::default();
        let all = "cpu, gpu, ram, fans, net";
        let mut s = parse_cfg(&format!("deck = {all}\norbit = cpu, gpu, ram, fans\ncards = clock | cpu, gpu, ram | fans, net, disk | weather, music\n")).unwrap();
        for (font, theme) in [(FontStyle::Smooth, Theme::Dark), (FontStyle::Pixel, Theme::Light), (FontStyle::Smooth, Theme::Light)] {
            s.font = font;
            s.theme = theme;
            for (w, h) in [(1920u32, 462u32), (462, 1920), (1280, 480), (480, 480), (800, 600)] {
                for style in [Style::Deck, Style::Cards, Style::Orbit] {
                    for lhm_on in [false, true] {
                        lhm::set_current(if lhm_on { lhm::Readings::sample() } else { lhm::Readings::default() });
                        for full in [true, false] {
                            let mut d = data();
                            if !full {
                                d.cpu_temp = None;
                                d.cpu_power = None;
                                d.cpu_mhz = None;
                                d.gpu_pct = None;
                                d.gpu_temp = None;
                                d.gpu_power = None;
                                d.weather = None;
                                d.now_playing = None;
                            }
                            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(w, h));
                            fb.clear(8, 8, 16);
                            let mut screen = DashScreen::new();
                            screen.draw(&mut fb, (0, 0, w, h), style, &d, ColorMode::Default, &s, &ts, &env);
                            assert!(painted(&fb) > (w * h / 20) as usize, "{style:?} {w}x{h} {font:?} nothing drawn");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_plate_follows_the_theme_the_colour_and_the_opacity() {
        let eff = effect::Settings::default();
        let env = Env { fps: 30.0, ffmpeg: None, effect: &eff, secs: Some(0.0) };
        let ts = tiles::Settings::default();
        let at = |s: &Settings, style| {
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(1920, 462));
            fb.clear(20, 20, 20);
            DashScreen::new().draw(&mut fb, (0, 0, 1920, 462), style, &data(), ColorMode::Default, s, &ts, &env);
            let i = (400 * 1920 + 1500) * 3;
            let px = [fb.as_bytes()[i], fb.as_bytes()[i + 1], fb.as_bytes()[i + 2]];
            px
        };
        let dark = at(&Settings::default(), Style::Deck);
        let light = at(&Settings { theme: Theme::Light, ..Settings::default() }, Style::Deck);
        assert!(dark[0] < 30 && light[0] > 200, "{dark:?} {light:?}");
        let tint = at(&Settings { plate_color: Some((200, 0, 0)), plate: Some(100), ..Settings::default() }, Style::Orbit);
        assert!(tint[0] > 150 && tint[1] < 40, "{tint:?}");
        let none = at(&Settings { plate: Some(0), ..Settings::default() }, Style::Deck);
        assert_eq!(none, [20, 20, 20]);
        assert_eq!(text_color(&Settings::default()), (255, 255, 255));
        assert!(text_color(&Settings { theme: Theme::Light, ..Settings::default() }).0 < 60);
        // see-through cards let the background show
        let solid = at(&Settings::default(), Style::Cards);
        let glass = at(&Settings { cards_opacity: 20, ..Settings::default() }, Style::Cards);
        assert!(glass[0] < solid[0] || glass[1] < solid[1] || glass[2] < solid[2], "{solid:?} {glass:?}");
    }

    #[test]
    fn the_layer_is_redrawn_only_when_something_changes() {
        let eff = effect::Settings::default();
        let env = Env { fps: 30.0, ffmpeg: None, effect: &eff, secs: Some(0.0) };
        let (s, ts) = (Settings::default(), tiles::Settings::default());
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(1920, 462));
        let mut screen = DashScreen::new();
        let mut d = data();
        for _ in 0..4 {
            screen.draw(&mut fb, (0, 0, 1920, 462), Style::Deck, &d, ColorMode::Default, &s, &ts, &env);
        }
        assert_eq!(screen.redraws, 1);
        d.cpu_temp = Some(65.0);
        screen.draw(&mut fb, (0, 0, 1920, 462), Style::Deck, &d, ColorMode::Default, &s, &ts, &env);
        assert_eq!(screen.redraws, 2);
        screen.draw(&mut fb, (0, 0, 1920, 462), Style::Orbit, &d, ColorMode::Default, &s, &ts, &env);
        assert_eq!(screen.redraws, 3, "another style is another picture");
    }

    #[test]
    fn the_panels_carry_the_fan_readings() {
        let (s, ts) = (Settings::default(), tiles::Settings::default());
        let now = Local::now();
        let hw = lhm::Readings::sample();
        let p = panel(PKind::Cpu, &data(), &hw, &s, &ts, &now);
        assert!(p.stats.iter().any(|x| x.label == "FAN" && x.value == "1240"));
        let none = panel(PKind::Cpu, &data(), &lhm::Readings::default(), &s, &ts, &now);
        assert!(none.stats.iter().all(|x| x.label != "FAN"));
        let f = panel(PKind::Fans, &data(), &hw, &s, &ts, &now);
        assert_eq!((f.hero.as_str(), f.rings.len()), ("1240", 4));
        assert_eq!(panel(PKind::Fans, &data(), &lhm::Readings::default(), &s, &ts, &now).hero, "--");
    }

    #[test]
    fn the_sparkline_stays_inside_its_box() {
        let mut l = Layer::new(100, 60);
        let data: Vec<f32> = (0..80).map(|i| (i as f32 * 0.3).sin() * 5.0 + 50.0).collect();
        l.spark((10.0, 10.0, 80.0, 40.0), &data, 10.0, (255, 0, 0), 2.0, 100, 255, true);
        let mut any = false;
        for y in 0..60usize {
            for x in 0..100usize {
                let a = l.px[(y * 100 + x) * 4 + 3];
                if a > 0 {
                    any = true;
                    assert!((6..=94).contains(&x) && (6..=54).contains(&y), "pixel at {x},{y}");
                }
            }
        }
        assert!(any);
    }
}
