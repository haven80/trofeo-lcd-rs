//! Free layouts (2.22.0): a screen you design yourself, from elements placed anywhere.
//!
//! A free layout is a text file `layouts/<name>.layout` next to the configuration file, one
//! element per line:
//!
//! ```text
//! # layouts/mine.layout
//! rect  x=0 y=0 w=100 h=100 color=#101820
//! text  x=50 y=12 anchor=c size=14 color=#00C8FF text="{time}"
//! text  x=5 y=60 size=5 text="CPU {cpu}  {cpu_temp}"
//! bar   x=5 y=75 w=40 h=6 value=cpu
//! ring  x=75 y=55 w=30 h=60 anchor=c value=gpu text="{gpu}"
//! ```
//!
//! Positions and sizes are percentages of the screen (the same file fits a landscape and a
//! portrait panel). `layout = mine` in the configuration file shows it like any other screen.
//! The settings editor has a visual builder that reads and writes the same file.
//!
//! This module is the model: reading, writing, checking. Drawing is in the program.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FOLDER: &str = "layouts";
pub const EXT: &str = "layout";
/// Environment variable that points to another layouts folder (the editor's preview runs the
/// program on a temporary configuration file).
pub const ENV_DIR: &str = "TROFEO_LAYOUTS_DIR";

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn folder_of(config: &Path) -> Option<PathBuf> {
    config.parent().map(|p| p.join(FOLDER))
}

pub fn set_dir(dir: Option<PathBuf>) {
    *DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// The folder with the free layouts, if one is known.
pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(ENV_DIR).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Bar,
    Ring,
    Rect,
    Image,
    /// The music spectrum (3.0.0): bars in a row or a column, or around a circle.
    Spectrum,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Text, Kind::Bar, Kind::Ring, Kind::Rect, Kind::Image, Kind::Spectrum];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Bar => "bar",
            Kind::Ring => "ring",
            Kind::Rect => "rect",
            Kind::Image => "image",
            Kind::Spectrum => "spectrum",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// Which point of the element is at `x`, `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Anchor::TopLeft => "tl",
            Anchor::Top => "t",
            Anchor::TopRight => "tr",
            Anchor::Left => "l",
            Anchor::Center => "c",
            Anchor::Right => "r",
            Anchor::BottomLeft => "bl",
            Anchor::Bottom => "b",
            Anchor::BottomRight => "br",
        }
    }

    pub fn parse(s: &str) -> Option<Anchor> {
        if s == "center" {
            return Some(Anchor::Center);
        }
        Anchor::ALL.into_iter().find(|a| a.name() == s)
    }

    /// (0, 0.5 or 1) horizontally and vertically.
    pub fn fractions(self) -> (f32, f32) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        }
    }
}

/// What a bar or ring shows: `(id, label, default maximum)`; the value runs from `min` to `max`.
pub const METRICS: &[(&str, &str, f32)] = &[
    ("cpu", "CPU load %", 100.0),
    ("gpu", "GPU load %", 100.0),
    ("ram", "Memory used %", 100.0),
    ("cpu_temp", "CPU temperature", 100.0),
    ("gpu_temp", "GPU temperature", 100.0),
    ("gpu_hotspot", "GPU hot spot", 110.0),
    ("cpu_power", "CPU watts", 300.0),
    ("gpu_power", "GPU watts", 400.0),
    ("cpu_mhz", "CPU MHz", 6000.0),
    ("fps", "FPS", 240.0),
    ("pihole_percent", "Pi-hole % blocked", 100.0),
    ("pihole_cached_percent", "Pi-hole % answered from cache", 100.0),
    ("pihole_forwarded_percent", "Pi-hole % forwarded upstream", 100.0),
];

/// What `{...}` in a text can be: every metric (written with its unit) and these.
pub const PLACEHOLDERS: &[&str] = &[
    "cpu", "gpu", "ram", "cpu_temp", "gpu_temp", "gpu_hotspot", "cpu_power", "gpu_power", "cpu_mhz", "fps", "cpu_freq",
    "ram_used", "ram_free", "ram_total", "net_down", "net_up", "disk_read", "disk_write", "song", "time", "time_short", "date",
    "weather_temp", "weather_city",
    "pihole_queries", "pihole_blocked", "pihole_allowed", "pihole_percent", "pihole_cached_percent", "pihole_forwarded_percent", "pihole_domains",
    "pihole_cached", "pihole_forwarded", "pihole_clients", "pihole_status",
];

pub fn metric_max(id: &str) -> f32 {
    METRICS.iter().find(|(m, _, _)| *m == id).map_or(100.0, |(_, _, max)| *max)
}


macro_rules! name_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $text:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub fn name(self) -> &'static str { match self { $($name::$var => $text),+ } }
            pub fn parse(s: &str) -> Option<$name> { $name::ALL.iter().copied().find(|v| v.name() == s) }
            pub const NAMES: &'static str = concat!($($text, " | "),+);
        }
    };
}

name_enum! {
    /// How the bars of a spectrum are drawn.
    SpecStyle { Bars = "bars", Led = "led", Peaks = "peaks", Area = "area", Mirror = "mirror" }
}
name_enum! {
    /// Where the bars of a spectrum grow: from the bottom of a row, from the left of a column,
    /// or outwards from a circle.
    SpecShape { Row = "row", Column = "column", Circle = "circle" }
}
name_enum! {
    /// Which sound: both channels mixed, or only the left or right one.
    SpecChannel { Mix = "mix", Left = "left", Right = "right" }
}
name_enum! {
    /// Which part of the frequencies: all, or the low, middle or high third.
    SpecBand { All = "all", Bass = "bass", Mid = "mid", Treble = "treble" }
}

pub type Rgb = (u8, u8, u8);

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub kind: Kind,
    /// Position of the anchor point, % of the screen's width and height.
    pub x: f32,
    pub y: f32,
    /// Size, % of the screen's width and height (bar, ring, rect, image; text: a limit on the
    /// width, 0 = none).
    pub w: f32,
    pub h: f32,
    pub anchor: Anchor,
    /// Text size (the pixel font's scale).
    pub size: u32,
    pub color: Option<Rgb>,
    /// The text (with `{...}`): text, and the middle of a ring.
    pub text: String,
    /// What a bar or ring shows (one of [`METRICS`]).
    pub value: String,
    pub min: f32,
    /// None = the default maximum of the value.
    pub max: Option<f32>,
    /// 0-100: rect and image only.
    pub opacity: u32,
    /// From this value up the bar or ring takes `hot_color` (None = never).
    pub hot: Option<f32>,
    pub hot_color: Rgb,
    /// The track behind a bar or ring.
    pub back: Rgb,
    /// Ring thickness, % of its radius.
    pub thick: f32,
    /// Image file.
    pub file: String,
    /// The font of a text (and of the text in a ring); None = the font of the configuration.
    pub font: Option<crate::TextFont>,
    /// The weight of the smooth font; None = the configuration's (or by size).
    pub weight: Option<crate::smooth_font::Weight>,
    /// Bar and ring: a second colour; the filled part then runs from `color` to `color2`.
    pub color2: Option<Rgb>,
    /// Bar and ring: seconds a loop of the moving gradient takes (0 = still).
    pub flow: f32,
    /// Spectrum only: the drawing style, the shape, the channel and the band of frequencies.
    pub style: SpecStyle,
    pub shape: SpecShape,
    pub channel: SpecChannel,
    pub band: SpecBand,
    /// Spectrum: how many bars (4-96).
    pub bars: u32,
    /// Spectrum: the space between bars, % of the room of a bar (0-80).
    pub gap: f32,
    /// Spectrum: mirrored left and right (row), top and bottom (column), or both halves of the
    /// circle: the low sounds in the middle, the high ones at the edges.
    pub sym: bool,
    /// Spectrum: grow the other way (downwards, to the left, inwards).
    pub flip: bool,
    /// Spectrum, circle: the hole in the middle, % of the radius (0-95).
    pub inner: f32,
    /// Spectrum, circle: where the first bar is, in degrees clockwise from the top.
    pub rot: f32,
}

impl Element {
    pub fn new(kind: Kind) -> Element {
        let (w, h) = match kind {
            Kind::Text => (0.0, 0.0),
            Kind::Bar => (30.0, 6.0),
            Kind::Ring => (24.0, 60.0),
            Kind::Rect => (30.0, 30.0),
            Kind::Image => (20.0, 40.0),
            Kind::Spectrum => (40.0, 24.0),
        };
        Element {
            kind,
            x: 10.0,
            y: 10.0,
            w,
            h,
            anchor: if kind == Kind::Ring { Anchor::Center } else { Anchor::TopLeft },
            size: 5,
            color: None,
            text: if kind == Kind::Text { "CPU {cpu}".to_string() } else { String::new() },
            value: "cpu".to_string(),
            min: 0.0,
            max: None,
            opacity: 100,
            hot: None,
            hot_color: (0xFF, 0x40, 0x40),
            back: (0x30, 0x30, 0x3C),
            thick: 20.0,
            file: String::new(),
            font: None,
            weight: None,
            color2: None,
            flow: 0.0,
            style: SpecStyle::Bars,
            shape: SpecShape::Row,
            channel: SpecChannel::Mix,
            band: SpecBand::All,
            bars: 32,
            gap: 25.0,
            sym: false,
            flip: false,
            inner: 40.0,
            rot: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Free {
    pub elements: Vec<Element>,
}

impl Free {
    /// Does something in it move (a bar or ring with a flowing gradient)?
    pub fn animated(&self) -> bool {
        self.elements.iter().any(|e| matches!(e.kind, Kind::Bar | Kind::Ring) && e.flow > 0.0)
    }

    /// Is there a spectrum in it (the screen then follows the music)?
    pub fn has_spectrum(&self) -> bool {
        self.elements.iter().any(|e| e.kind == Kind::Spectrum)
    }

    /// Does a spectrum in it listen to one channel (left or right)?
    pub fn has_stereo_spectrum(&self) -> bool {
        self.elements.iter().any(|e| e.kind == Kind::Spectrum && e.channel != SpecChannel::Mix)
    }
}

/// Names the `{...}` of a text refers to, or the first one that is not known.
pub fn check_placeholders(text: &str) -> Result<(), String> {
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let end = after.find('}').ok_or_else(|| format!("'{{' without a closing '}}' in \"{text}\""))?;
        let name = &after[..end];
        if !PLACEHOLDERS.contains(&name) {
            return Err(format!("{{{name}}} is not known (valid: {})", PLACEHOLDERS.join(", ")));
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

/// Replace every `{name}` with what `value` says about it.
pub fn substitute(text: &str, value: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(end) => {
                out.push_str(&value(&after[..end]));
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Split a line into words; a value may be in double quotes (`text="CPU {cpu}"`).
fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if quoted {
        return Err("a quote is not closed".to_string());
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn number(key: &str, v: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let n: f32 = v.trim_end_matches('%').parse().map_err(|_| format!("{key}: '{v}' is not a number"))?;
    if !n.is_finite() || n < lo || n > hi {
        return Err(format!("{key}: {n} is out of range ({lo} to {hi})"));
    }
    Ok(n)
}

fn boolean(key: &str, v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(format!("{key}: '{v}' is not true or false")),
    }
}

fn color(key: &str, v: &str) -> Result<Rgb, String> {
    crate::schema::parse_color(v).ok_or_else(|| format!("{key}: '{v}' is not a color (#RRGGBB)"))
}

fn parse_line(line: &str) -> Result<Element, String> {
    let w = words(line)?;
    let kind_word = w.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    let kind = Kind::parse(&kind_word).ok_or_else(|| format!("'{kind_word}' is not an element (text, bar, ring, rect, image, spectrum)"))?;
    let mut e = Element::new(kind);
    if kind == Kind::Text {
        e.text.clear();
    }
    for item in &w[1..] {
        let (k, v) = item.split_once('=').ok_or_else(|| format!("'{item}' is not key=value"))?;
        let k = k.to_ascii_lowercase();
        match k.as_str() {
            "x" => e.x = number(&k, v, -100.0, 200.0)?,
            "y" => e.y = number(&k, v, -100.0, 200.0)?,
            "w" => e.w = number(&k, v, 0.0, 200.0)?,
            "h" => e.h = number(&k, v, 0.0, 200.0)?,
            "anchor" => e.anchor = Anchor::parse(&v.to_ascii_lowercase()).ok_or_else(|| format!("anchor: '{v}' is not one of tl, t, tr, l, c, r, bl, b, br"))?,
            "size" => e.size = number(&k, v, 1.0, 80.0)? as u32,
            "color" => e.color = Some(color(&k, v)?),
            "text" => {
                check_placeholders(v)?;
                e.text = v.to_string();
            }
            "value" => {
                let id = v.to_ascii_lowercase();
                if !METRICS.iter().any(|(m, _, _)| *m == id) {
                    return Err(format!("value: '{v}' is not known (valid: {})", METRICS.iter().map(|m| m.0).collect::<Vec<_>>().join(", ")));
                }
                e.value = id;
            }
            "min" => e.min = number(&k, v, -100000.0, 100000.0)?,
            "max" => e.max = Some(number(&k, v, -100000.0, 100000.0)?),
            "opacity" => e.opacity = number(&k, v, 0.0, 100.0)? as u32,
            "hot" => e.hot = Some(number(&k, v, -100000.0, 100000.0)?),
            "hot_color" => e.hot_color = color(&k, v)?,
            "back" => e.back = color(&k, v)?,
            "thick" => e.thick = number(&k, v, 2.0, 100.0)?,
            "file" => e.file = v.to_string(),
            "color2" => e.color2 = Some(color(&k, v)?),
            "flow" => e.flow = number(&k, v, 0.0, 600.0)?,
            "style" => e.style = SpecStyle::parse(&v.to_ascii_lowercase()).ok_or_else(|| format!("style: '{v}' is not one of {}", SpecStyle::NAMES.trim_end_matches(" | ")))?,
            "shape" => e.shape = SpecShape::parse(&v.to_ascii_lowercase()).ok_or_else(|| format!("shape: '{v}' is not one of {}", SpecShape::NAMES.trim_end_matches(" | ")))?,
            "channel" => e.channel = SpecChannel::parse(&v.to_ascii_lowercase()).ok_or_else(|| format!("channel: '{v}' is not one of {}", SpecChannel::NAMES.trim_end_matches(" | ")))?,
            "band" => e.band = SpecBand::parse(&v.to_ascii_lowercase()).ok_or_else(|| format!("band: '{v}' is not one of {}", SpecBand::NAMES.trim_end_matches(" | ")))?,
            "bars" => e.bars = number(&k, v, 4.0, 96.0)? as u32,
            "gap" => e.gap = number(&k, v, 0.0, 80.0)?,
            "sym" => e.sym = boolean(&k, v)?,
            "flip" => e.flip = boolean(&k, v)?,
            "inner" => e.inner = number(&k, v, 0.0, 95.0)?,
            "rot" => e.rot = number(&k, v, 0.0, 360.0)?,
            "font" => {
                e.font = Some(match v.to_ascii_lowercase().as_str() {
                    "pixel" => crate::TextFont::Pixel,
                    "smooth" => crate::TextFont::Smooth,
                    _ => return Err(format!("font: '{v}' is not pixel or smooth")),
                })
            }
            "weight" => {
                e.weight = Some(match v.to_ascii_lowercase().as_str() {
                    "light" => crate::smooth_font::Weight::Light,
                    "regular" => crate::smooth_font::Weight::Regular,
                    "medium" => crate::smooth_font::Weight::Medium,
                    _ => return Err(format!("weight: '{v}' is not light, regular or medium")),
                })
            }
            other => return Err(format!("'{other}' is not a property")),
        }
    }
    if kind == Kind::Image && e.file.is_empty() {
        return Err("image: file=... is needed".to_string());
    }
    if e.max.is_some_and(|m| m <= e.min) {
        return Err("max must be more than min".to_string());
    }
    Ok(e)
}

/// Read a layout. The first problem is returned with its line number.
pub fn parse(text: &str) -> Result<Free, String> {
    let mut out = Free::default();
    for (i, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        out.elements.push(parse_line(line).map_err(|e| format!("line {}: {e}", i + 1))?);
    }
    if out.elements.len() > 200 {
        return Err("more than 200 elements".to_string());
    }
    Ok(out)
}

fn fmt_num(v: f32) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn fmt_color(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "'"))
}

/// The text of one element (only what differs from the defaults of its kind is written).
pub fn render_element(e: &Element) -> String {
    let d = Element::new(e.kind);
    let mut s = format!("{} x={} y={}", e.kind.name(), fmt_num(e.x), fmt_num(e.y));
    if e.kind != Kind::Text || e.w > 0.0 {
        s.push_str(&format!(" w={} h={}", fmt_num(e.w), fmt_num(e.h)));
    }
    if e.anchor != d.anchor {
        s.push_str(&format!(" anchor={}", e.anchor.name()));
    }
    if e.kind == Kind::Text {
        s.push_str(&format!(" size={}", e.size));
    }
    if let Some(c) = e.color {
        s.push_str(&format!(" color={}", fmt_color(c)));
    }
    if matches!(e.kind, Kind::Bar | Kind::Ring) {
        s.push_str(&format!(" value={}", e.value));
        if e.min != 0.0 {
            s.push_str(&format!(" min={}", fmt_num(e.min)));
        }
        if let Some(m) = e.max {
            s.push_str(&format!(" max={}", fmt_num(m)));
        }
        if let Some(h) = e.hot {
            s.push_str(&format!(" hot={} hot_color={}", fmt_num(h), fmt_color(e.hot_color)));
        }
        if e.back != d.back {
            s.push_str(&format!(" back={}", fmt_color(e.back)));
        }
    }
    if e.kind == Kind::Spectrum {
        if e.style != d.style {
            s.push_str(&format!(" style={}", e.style.name()));
        }
        if e.shape != d.shape {
            s.push_str(&format!(" shape={}", e.shape.name()));
        }
        if e.channel != d.channel {
            s.push_str(&format!(" channel={}", e.channel.name()));
        }
        if e.band != d.band {
            s.push_str(&format!(" band={}", e.band.name()));
        }
        if e.bars != d.bars {
            s.push_str(&format!(" bars={}", e.bars));
        }
        if e.gap != d.gap {
            s.push_str(&format!(" gap={}", fmt_num(e.gap)));
        }
        if e.sym {
            s.push_str(" sym=true");
        }
        if e.flip {
            s.push_str(" flip=true");
        }
        if e.shape == SpecShape::Circle {
            if e.inner != d.inner {
                s.push_str(&format!(" inner={}", fmt_num(e.inner)));
            }
            if e.rot != d.rot {
                s.push_str(&format!(" rot={}", fmt_num(e.rot)));
            }
        }
        if let Some(h) = e.hot {
            s.push_str(&format!(" hot={} hot_color={}", fmt_num(h), fmt_color(e.hot_color)));
        }
    }
    if matches!(e.kind, Kind::Bar | Kind::Ring | Kind::Spectrum) {
        if let Some(c) = e.color2 {
            s.push_str(&format!(" color2={}", fmt_color(c)));
        }
        if e.flow > 0.0 {
            s.push_str(&format!(" flow={}", fmt_num(e.flow)));
        }
    }
    if e.kind == Kind::Ring && e.thick != d.thick {
        s.push_str(&format!(" thick={}", fmt_num(e.thick)));
    }
    if matches!(e.kind, Kind::Rect | Kind::Image) && e.opacity != 100 {
        s.push_str(&format!(" opacity={}", e.opacity));
    }
    if matches!(e.kind, Kind::Text | Kind::Ring) {
        match e.font {
            Some(crate::TextFont::Pixel) => s.push_str(" font=pixel"),
            Some(crate::TextFont::Smooth) => s.push_str(" font=smooth"),
            None => {}
        }
        match e.weight {
            Some(crate::smooth_font::Weight::Light) => s.push_str(" weight=light"),
            Some(crate::smooth_font::Weight::Regular) => s.push_str(" weight=regular"),
            Some(crate::smooth_font::Weight::Medium) => s.push_str(" weight=medium"),
            None => {}
        }
    }
    if matches!(e.kind, Kind::Text | Kind::Ring) && !e.text.is_empty() {
        s.push_str(&format!(" text={}", quote(&e.text)));
    }
    if e.kind == Kind::Image {
        s.push_str(&format!(" file={}", quote(&e.file)));
    }
    s
}

/// The text of a layout file.
pub fn render(name: &str, f: &Free) -> String {
    let mut s = format!("# trofeo_lcd free layout: {name}\n# Show it with  layout = {name}  in the configuration file. Positions and sizes are % of the screen.\n");
    for e in &f.elements {
        s.push_str(&render_element(e));
        s.push('\n');
    }
    s
}

/// The designs that come with the program (2.23.0): `(name, English, Italian, file text)`.
/// `layout = halo` shows one without any file; a file of yours with the same name in the
/// layouts folder takes its place, and the builder can copy one to start from.
pub const BUILTIN: &[(&str, &str, &str, &str)] = &[
    ("halo", "Halo: two big rings (CPU, GPU), the clock in the middle, memory below", "Halo: due grandi anelli (CPU, GPU), orologio al centro, memoria sotto", include_str!("free_builtin/halo.layout")),
    ("gauges", "Gauges: four dials, load and temperature of CPU and GPU", "Quadranti: quattro indicatori, carico e temperatura di CPU e GPU", include_str!("free_builtin/gauges.layout")),
    ("terminal", "Terminal: green-on-black readout, one line per device", "Terminale: lettura verde su nero, una riga per dispositivo", include_str!("free_builtin/terminal.layout")),
    ("horizon", "Horizon: a huge clock and three thin lines of load", "Orizzonte: un orologio enorme e tre sottili linee di carico", include_str!("free_builtin/horizon.layout")),
    ("ribbon", "Ribbon: clock, song and weather above a ribbon of readings", "Nastro: orologio, brano e meteo sopra un nastro di valori", include_str!("free_builtin/ribbon.layout")),
    ("atelier", "Atelier: ivory and gold, a light clock and three fine rings", "Atelier: avorio e oro, un orologio sottile e tre anelli fini", include_str!("free_builtin/atelier.layout")),
    ("noir", "Noir: white on black, a huge light clock and four quiet figures", "Noir: bianco su nero, un orologio enorme e sottile e quattro cifre sobrie", include_str!("free_builtin/noir.layout")),
    ("pearl", "Pearl: cool and airy, the clock inside a fine ring", "Pearl: fresco e leggero, l'orologio dentro un anello sottile", include_str!("free_builtin/pearl.layout")),
    ("volt", "Volt: four loud colour blocks with giant figures", "Volt: quattro blocchi di colore con cifre giganti", include_str!("free_builtin/volt.layout")),
    ("blaze", "Blaze: CPU in orange and GPU in magenta, giant figures", "Blaze: CPU arancione e GPU magenta, cifre giganti", include_str!("free_builtin/blaze.layout")),
    ("studio", "Studio: paper and ink, a light spec sheet with one orange accent", "Studio: carta e inchiostro, una scheda tecnica chiara con un accento arancione", include_str!("free_builtin/studio.layout")),
    ("board", "Board: an airport departure board, every number on a tile", "Board: un tabellone da aeroporto, ogni numero su una tessera", include_str!("free_builtin/board.layout")),
    ("mineral", "Mineral: dark cards in sage green, a ring and six readings on each", "Mineral: card scure verde salvia, un anello e sei valori su ognuna", include_str!("free_builtin/mineral.layout")),
    ("cluster", "Cluster: a car instrument cluster, three big dials", "Cluster: un quadro strumenti d'auto, tre grandi quadranti", include_str!("free_builtin/cluster.layout")),
    ("swiss", "Swiss: a red block with a huge clock, big black figures on a light page", "Swiss: blocco rosso con orologio enorme, grandi cifre nere su pagina chiara", include_str!("free_builtin/swiss.layout")),
    ("dusk", "Dusk: a sunset gradient, a giant light clock, flowing bars", "Dusk: sfumatura da tramonto, orologio gigante leggero, barre che scorrono", include_str!("free_builtin/dusk.layout")),
    ("aurora", "Aurora: glass cards on a teal-to-violet gradient, flowing bars", "Aurora: card di vetro su sfumatura teal-viola, barre che scorrono", include_str!("free_builtin/aurora.layout")),
    ("carbon", "Carbon: matte black with amber lines, four columns, flowing bars", "Carbon: nero opaco con filetti ambra, quattro colonne, barre che scorrono", include_str!("free_builtin/carbon.layout")),
    ("eclipse", "Eclipse: three concentric rings that shimmer, the clock in the middle", "Eclipse: tre anelli concentrici che brillano, orologio al centro", include_str!("free_builtin/eclipse.layout")),
    ("ledger", "Ledger: a navy ledger with gold lines, one row per device, flowing bars", "Ledger: registro blu con filetti oro, una riga per dispositivo, barre che scorrono", include_str!("free_builtin/ledger.layout")),
    ("haiku", "Haiku: a huge light clock, lots of space, one line of readings", "Haiku: orologio enorme e leggero, molto spazio, una riga di valori", include_str!("free_builtin/haiku.layout")),
    ("peach", "Peach: a warm light page with three gradient rings", "Peach: pagina chiara e calda con tre anelli sfumati", include_str!("free_builtin/peach.layout")),
    ("pulse", "Pulse: a big clock over a wide mirrored spectrum", "Pulse: un grande orologio sopra uno spettro speculare a tutta larghezza", include_str!("free_builtin/pulse.layout")),
    ("radial", "Radial: the spectrum in a ring around the clock", "Radial: lo spettro in un anello attorno all'orologio", include_str!("free_builtin/radial.layout")),
    ("stereo", "Stereo: two rings, the left and the right channel, the clock between", "Stereo: due anelli, canale sinistro e destro, l'orologio in mezzo", include_str!("free_builtin/stereo.layout")),
    ("bands", "Bands: bass, mid and treble, each in a row of its own", "Bands: bassi, medi e alti, ognuno in una riga", include_str!("free_builtin/bands.layout")),
    ("pihole", "Pi-hole: a big ring with the share of blocked queries and the figures in a list", "Pi-hole: un grande anello con la quota di richieste bloccate e i valori in elenco", include_str!("free_builtin/pihole.layout")),
    ("pihole-cards", "Pi-hole cards: four cards (queries, blocked, share blocked, domains on the lists)", "Pi-hole a schede: quattro schede (richieste, bloccate, percentuale, domini nelle liste)", include_str!("free_builtin/pihole-cards.layout")),
    ("pihole-ring", "Pi-hole ring: one big ring in the middle, queries and blocked on the sides", "Pi-hole anello: un grande anello al centro, richieste e bloccate ai lati", include_str!("free_builtin/pihole-ring.layout")),
    ("pihole-console", "Pi-hole console: a pixel-font readout, one line per figure", "Pi-hole console: una lettura con font a pixel, una riga per valore", include_str!("free_builtin/pihole-console.layout")),
    ("pihole-minimal", "Pi-hole minimal: the share blocked, huge and light, one quiet line under it", "Pi-hole minimale: la percentuale bloccata enorme e sottile, una riga discreta sotto", include_str!("free_builtin/pihole-minimal.layout")),
    ("pihole-light", "Pi-hole light: a light page, figures in ink, one red accent", "Pi-hole chiaro: una pagina chiara, cifre a inchiostro, un accento rosso", include_str!("free_builtin/pihole-light.layout")),
    ("pihole-flow", "Pi-hole flow: three wide bars with flowing colours (blocked, cache, forwarded)", "Pi-hole flusso: tre barre larghe con colori che scorrono (bloccate, cache, inoltrate)", include_str!("free_builtin/pihole-flow.layout")),
];

/// The same designs for a tall, narrow panel (a portrait screen), `(name, file text)`.
pub const BUILTIN_PORTRAIT: &[(&str, &str)] = &[
    ("halo", include_str!("free_builtin/halo.portrait.layout")),
    ("gauges", include_str!("free_builtin/gauges.portrait.layout")),
    ("terminal", include_str!("free_builtin/terminal.portrait.layout")),
    ("horizon", include_str!("free_builtin/horizon.portrait.layout")),
    ("ribbon", include_str!("free_builtin/ribbon.portrait.layout")),
    ("atelier", include_str!("free_builtin/atelier.portrait.layout")),
    ("noir", include_str!("free_builtin/noir.portrait.layout")),
    ("pearl", include_str!("free_builtin/pearl.portrait.layout")),
    ("volt", include_str!("free_builtin/volt.portrait.layout")),
    ("blaze", include_str!("free_builtin/blaze.portrait.layout")),
    ("studio", include_str!("free_builtin/studio.portrait.layout")),
    ("board", include_str!("free_builtin/board.portrait.layout")),
    ("mineral", include_str!("free_builtin/mineral.portrait.layout")),
    ("cluster", include_str!("free_builtin/cluster.portrait.layout")),
    ("swiss", include_str!("free_builtin/swiss.portrait.layout")),
    ("dusk", include_str!("free_builtin/dusk.portrait.layout")),
    ("aurora", include_str!("free_builtin/aurora.portrait.layout")),
    ("carbon", include_str!("free_builtin/carbon.portrait.layout")),
    ("eclipse", include_str!("free_builtin/eclipse.portrait.layout")),
    ("ledger", include_str!("free_builtin/ledger.portrait.layout")),
    ("haiku", include_str!("free_builtin/haiku.portrait.layout")),
    ("peach", include_str!("free_builtin/peach.portrait.layout")),
    ("pulse", include_str!("free_builtin/pulse.portrait.layout")),
    ("radial", include_str!("free_builtin/radial.portrait.layout")),
    ("stereo", include_str!("free_builtin/stereo.portrait.layout")),
    ("bands", include_str!("free_builtin/bands.portrait.layout")),
    ("pihole", include_str!("free_builtin/pihole.portrait.layout")),
    ("pihole-cards", include_str!("free_builtin/pihole-cards.portrait.layout")),
    ("pihole-ring", include_str!("free_builtin/pihole-ring.portrait.layout")),
    ("pihole-console", include_str!("free_builtin/pihole-console.portrait.layout")),
    ("pihole-minimal", include_str!("free_builtin/pihole-minimal.portrait.layout")),
    ("pihole-light", include_str!("free_builtin/pihole-light.portrait.layout")),
    ("pihole-flow", include_str!("free_builtin/pihole-flow.portrait.layout")),
];

/// The text of a built-in design.
pub fn builtin(name: &str) -> Option<&'static str> {
    let n = name.trim().to_ascii_lowercase();
    BUILTIN.iter().find(|b| b.0 == n).map(|b| b.3)
}

/// A built-in design, read.
pub fn load_builtin(name: &str) -> Result<Free, String> {
    load_builtin_for(name, false)
}

/// A built-in design, read: the one for a portrait panel when `portrait`.
pub fn load_builtin_for(name: &str, portrait: bool) -> Result<Free, String> {
    let n = name.trim().to_ascii_lowercase();
    let text = if portrait { BUILTIN_PORTRAIT.iter().find(|b| b.0 == n).map(|b| b.1) } else { None };
    let text = text.or_else(|| builtin(&n)).ok_or_else(|| format!("'{name}' is not a built-in layout"))?;
    parse(text).map_err(|e| format!("built-in layout '{name}': {e}"))
}

pub fn valid_name(name: &str) -> Result<(), &'static str> {
    let n = name.trim();
    if n.is_empty() || n.len() > 40 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err("use lowercase letters, digits, - and _ (up to 40)");
    }
    Ok(())
}

pub fn path_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{EXT}", name.trim()))
}

pub fn list(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some(EXT) {
                return None;
            }
            p.file_stem()?.to_str().map(str::to_string)
        })
        .filter(|n| valid_name(n).is_ok())
        .collect();
    out.sort();
    out
}

pub fn load(dir: &Path, name: &str) -> Result<Free, String> {
    valid_name(name).map_err(|e| format!("layout '{name}': {e}"))?;
    let path = path_of(dir, name);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("layout '{name}': cannot read {}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn save(dir: &Path, name: &str, f: &Free) -> Result<PathBuf, String> {
    valid_name(name).map_err(|e| format!("'{}': {e}", name.trim()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = path_of(dir, name);
    std::fs::write(&path, render(name.trim(), f)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// The top-left corner and size, in pixels, of an element whose own size is `(w, h)` pixels,
/// on a canvas of `(cw, ch)` pixels.
pub fn place(e: &Element, cw: f32, ch: f32, w: f32, h: f32) -> (f32, f32) {
    let (fx, fy) = e.anchor.fractions();
    (e.x / 100.0 * cw - fx * w, e.y / 100.0 * ch - fy * h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment\n\nrect x=0 y=0 w=100 h=100 color=#101820\ntext x=50 y=12 anchor=c size=14 color=#00C8FF text=\"{time}\"\ntext x=5 y=60 size=5 text=\"CPU {cpu}  {cpu_temp}\"\nbar x=5 y=75 w=40 h=6 value=cpu hot=85 hot_color=#FF0000\nring x=75 y=55 w=30 h=60 anchor=c value=gpu text=\"{gpu}\" thick=15\nimage x=90 y=5 w=8 h=20 anchor=tr file=\"logo.png\" opacity=80\n";

    #[test]
    fn a_layout_is_read() {
        let f = parse(SAMPLE).unwrap();
        assert_eq!(f.elements.len(), 6);
        assert_eq!(f.elements[1].kind, Kind::Text);
        assert_eq!(f.elements[1].anchor, Anchor::Center);
        assert_eq!(f.elements[1].color, Some((0, 0xC8, 0xFF)));
        assert_eq!(f.elements[2].text, "CPU {cpu}  {cpu_temp}");
        assert_eq!(f.elements[3].hot, Some(85.0));
        assert_eq!(f.elements[4].thick, 15.0);
        assert_eq!(f.elements[5].file, "logo.png");
    }

    #[test]
    fn writing_then_reading_gives_the_same_layout() {
        let f = parse(SAMPLE).unwrap();
        let text = render("mine", &f);
        assert_eq!(parse(&text).unwrap(), f, "{text}");
        // and the text is stable
        assert_eq!(render("mine", &parse(&text).unwrap()), text);
        // a default element is short
        let mut d = Free::default();
        d.elements.push(Element::new(Kind::Bar));
        assert_eq!(render_element(&d.elements[0]), "bar x=10 y=10 w=30 h=6 value=cpu");
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        for (text, why) in [
            ("circle x=1\n", "not an element"),
            ("text x=abc\n", "not a number"),
            ("text x=1 y\n", "not key=value"),
            ("text x=1 colour=#FFF\n", "not a property"),
            ("text text=\"{nope}\"\n", "{nope} is not known"),
            ("text text=\"{cpu\"\n", "closing"),
            ("bar value=weather\n", "not known"),
            ("bar color=notacolor\n", "not a color"),
            ("text anchor=middle\n", "anchor"),
            ("text size=500\n", "out of range"),
            ("image x=1\n", "file="),
            ("bar min=10 max=5\n", "max must be more"),
            ("text text=\"a\n", "quote"),
        ] {
            let e = parse(&format!("# first line\n{text}")).unwrap_err();
            assert!(e.starts_with("line 2:") && e.contains(why), "{text:?} -> {e}");
        }
    }

    #[test]
    fn placeholders_are_checked_and_replaced() {
        assert!(check_placeholders("CPU {cpu} / {gpu_temp}").is_ok());
        assert!(check_placeholders("no braces").is_ok());
        assert!(check_placeholders("{x}").is_err());
        for p in PLACEHOLDERS {
            assert!(check_placeholders(&format!("{{{p}}}")).is_ok());
        }
        for (m, _, _) in METRICS {
            assert!(PLACEHOLDERS.contains(m), "{m} can be written in a text too");
        }
        assert_eq!(substitute("a {cpu} b {gpu}!", &|n| n.to_uppercase()), "a CPU b GPU!");
        assert_eq!(substitute("open {cpu", &|_| "x".into()), "open {cpu");
    }

    #[test]
    fn positions_follow_the_anchor() {
        let mut e = Element::new(Kind::Rect);
        e.x = 50.0;
        e.y = 50.0;
        e.anchor = Anchor::Center;
        assert_eq!(place(&e, 1000.0, 400.0, 200.0, 100.0), (400.0, 150.0));
        e.anchor = Anchor::BottomRight;
        assert_eq!(place(&e, 1000.0, 400.0, 200.0, 100.0), (300.0, 100.0));
        e.anchor = Anchor::TopLeft;
        assert_eq!(place(&e, 1000.0, 400.0, 200.0, 100.0), (500.0, 200.0));
    }

    #[test]
    fn layouts_are_saved_listed_and_loaded() {
        let dir = std::env::temp_dir().join(format!("trofeo_free_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = parse(SAMPLE).unwrap();
        save(&dir, "my-design", &f).unwrap();
        save(&dir, "second", &f).unwrap();
        assert_eq!(list(&dir), vec!["my-design".to_string(), "second".to_string()]);
        assert_eq!(load(&dir, "my-design").unwrap(), f);
        assert!(save(&dir, "Bad Name", &f).is_err());
        assert!(load(&dir, "../x").is_err());
        std::fs::write(path_of(&dir, "broken"), "text x=\n").unwrap();
        assert!(load(&dir, "broken").unwrap_err().contains("line 1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_built_in_design_is_read_and_has_something_to_show() {
        for (name, en, it, _) in BUILTIN {
            valid_name(name).unwrap();
            assert!(!en.is_empty() && !it.is_empty());
            let f = load_builtin(name).unwrap_or_else(|e| panic!("{e}"));
            assert!(f.elements.len() >= 8 || ["blaze", "volt", "pulse", "radial", "stereo", "bands", "pihole-ring", "pihole-minimal", "pihole-flow"].contains(name), "{name}");
            // what is written out is read back the same
            assert_eq!(parse(&render(name, &f)).unwrap(), f, "{name}");
        }
        for (name, _, _, _) in BUILTIN {
            let f = load_builtin_for(name, true).unwrap_or_else(|e| panic!("{e}"));
            assert!(f.elements.len() >= 8 || ["blaze", "volt", "pulse", "radial", "stereo", "bands", "pihole-ring", "pihole-minimal", "pihole-flow"].contains(name), "{name} (portrait)");
            assert_ne!(f, load_builtin(name).unwrap(), "{name}");
        }
        assert_eq!(BUILTIN_PORTRAIT.len(), BUILTIN.len());
        assert!(builtin("HALO").is_some());
        assert!(builtin("nothing").is_none());
    }

    #[test]
    fn the_pihole_names_of_the_layouts_are_the_ones_the_module_knows() {
        let ph: Vec<&str> = PLACEHOLDERS.iter().copied().filter(|p| p.starts_with("pihole_")).collect();
        assert_eq!(ph, crate::pihole::PLACEHOLDERS);
        let ms: Vec<&str> = METRICS.iter().map(|m| m.0).filter(|m| m.starts_with("pihole_")).collect();
        assert_eq!(ms, crate::pihole::METRICS);
    }

    #[test]
    fn the_font_of_a_text_is_read_and_written() {
        let f = parse("text x=5 y=5 size=9 font=smooth weight=light text=\"{time}\"\nring x=50 y=50 value=cpu font=pixel text=\"{cpu}\"\ntext x=1 y=1 text=\"a\"").unwrap();
        assert_eq!((f.elements[0].font, f.elements[0].weight), (Some(crate::TextFont::Smooth), Some(crate::smooth_font::Weight::Light)));
        assert_eq!((f.elements[1].font, f.elements[1].weight), (Some(crate::TextFont::Pixel), None));
        assert_eq!((f.elements[2].font, f.elements[2].weight), (None, None));
        assert_eq!(parse(&render("x", &f)).unwrap(), f);
        assert!(parse("text x=1 y=1 font=fancy text=\"a\"").unwrap_err().contains("line 1"));
        assert!(parse("text x=1 y=1 weight=heavy text=\"a\"").is_err());
    }

    #[test]
    fn a_gradient_and_its_flow_are_read_and_written() {
        let f = parse("bar x=1 y=1 value=cpu color=#FF0000 color2=#0000FF flow=2.5\nring x=5 y=5 value=gpu color2=#00FF00\nbar x=1 y=9 value=ram").unwrap();
        assert_eq!((f.elements[0].color2, f.elements[0].flow), (Some((0, 0, 255)), 2.5));
        assert_eq!((f.elements[1].color2, f.elements[1].flow), (Some((0, 255, 0)), 0.0));
        assert_eq!((f.elements[2].color2, f.elements[2].flow), (None, 0.0));
        assert_eq!(parse(&render("x", &f)).unwrap(), f);
        assert!(f.animated());
        assert!(!parse("bar x=1 y=1 value=cpu color2=#0000FF").unwrap().animated());
        assert!(parse("bar x=1 y=1 value=cpu flow=-1").is_err());
        assert!(parse("bar x=1 y=1 value=cpu color2=nope").is_err());
    }

    #[test]
    fn the_built_in_designs_with_flowing_bars_are_the_ones_that_say_so() {
        for name in ["dusk", "aurora", "carbon", "eclipse", "ledger"] {
            assert!(load_builtin(name).unwrap().animated(), "{name}");
            assert!(load_builtin_for(name, true).unwrap().animated(), "{name} (portrait)");
        }
        for name in ["swiss", "haiku", "peach", "atelier", "studio", "board"] {
            assert!(!load_builtin(name).unwrap().animated(), "{name}");
        }
    }

    #[test]
    fn a_spectrum_is_read_and_written_with_all_its_options() {
        let f = parse(
            "spectrum x=2 y=3 w=40 h=20 shape=circle style=led channel=left band=bass bars=24 gap=10 sym=true flip=true inner=30 rot=90 hot=80 hot_color=#FF0000 color=#00FF00 color2=#0000FF flow=3\nspectrum x=1 y=1",
        )
        .unwrap();
        let e = &f.elements[0];
        assert_eq!((e.kind, e.shape, e.style, e.channel, e.band), (Kind::Spectrum, SpecShape::Circle, SpecStyle::Led, SpecChannel::Left, SpecBand::Bass));
        assert_eq!((e.bars, e.gap, e.sym, e.flip, e.inner, e.rot, e.hot), (24, 10.0, true, true, 30.0, 90.0, Some(80.0)));
        assert_eq!((e.color, e.color2, e.flow), (Some((0, 255, 0)), Some((0, 0, 255)), 3.0));
        // the second one is all defaults and is written without any option
        let d = &f.elements[1];
        assert_eq!((d.shape, d.style, d.channel, d.band, d.bars), (SpecShape::Row, SpecStyle::Bars, SpecChannel::Mix, SpecBand::All, 32));
        assert_eq!(render_element(d), "spectrum x=1 y=1 w=40 h=24");
        assert_eq!(parse(&render("x", &f)).unwrap(), f);
        assert!(f.has_spectrum() && f.has_stereo_spectrum());
        assert!(!parse("spectrum x=1 y=1").unwrap().has_stereo_spectrum());
        assert!(!parse("bar x=1 y=1 value=cpu").unwrap().has_spectrum());
        for bad in ["shape=ring", "style=wave", "channel=center", "band=sub", "bars=2", "bars=200", "gap=90", "sym=maybe", "inner=99"] {
            assert!(parse(&format!("spectrum x=1 y=1 {bad}")).is_err(), "{bad}");
        }
    }
}
