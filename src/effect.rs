//! Animated background effects (plasma, aurora, lava, waves, the "digital rain" styles
//! matrix and rain, and the aquarium cross-section) that cost almost no CPU.
//!
//! How it stays cheap:
//!
//! * The effect is a smooth scalar field, so it is only evaluated on a coarse grid
//!   (one sample every [`CELL`] pixels: ~6 000 samples on a 1920x462 panel).
//! * The grid is stretched to the full panel with plain linear interpolation in
//!   fixed point (integer adds in the inner loop, no trig, no floats per pixel).
//! * Each pixel is one lookup in a 1024-entry colour table (the palette, already
//!   multiplied by the brightness). Changing palette or brightness only rebuilds
//!   that table.
//!
//! The colour table is cyclic, so the field can be left "unwrapped" while it is
//! interpolated (no seams) and wrapped only at the lookup.
//!
//! The picture itself is the expensive part of the program (JPEG encode + USB), so
//! the frame rate of the effect is a setting of its own (`effect_fps`).

use trofeo_lcd::Framebuffer;

pub type Rgb = (u8, u8, u8);

/// Pixels between two samples of the field.
const CELL: usize = 12;
/// Colour table size (a power of two: the lookup wraps with a mask).
const LUT_N: usize = 1024;
/// Colours a rain palette is sampled at.
const RAIN_N: usize = 256;
/// Entries of the aquarium's depth -> colour table.
const WATER_N: usize = 256;
/// Fractional bits of the field values (in units of one LUT entry).
const FRAC: u32 = 8;
/// Extra fractional bits kept while stepping along a row.
const STEP_FRAC: u32 = 4;
/// Length in pixels of "1.0" in the formulas below at `scale = 100`.
const UNIT_PX: f32 = 260.0;
/// Seconds a palette takes to cross-fade into the next one when rotating.
const FADE_SECS: f64 = 4.0;

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// The classic demoscene plasma: a sum of sines and a moving radial wave.
    Plasma,
    /// Slow undulating bands, like curtains of light.
    Aurora,
    /// A lava lamp: soft blobs drifting and merging.
    Lava,
    /// Smooth diagonal ripples, like silk.
    Waves,
    /// Sparse columns of falling characters with fading trails ("digital rain").
    Matrix,
    /// The same rain, minimal: thin falling streaks, no characters.
    Rain,
    /// A side view of an aquarium: a moving water surface, light rays, caustics, bubbles.
    Aquarium,
    /// Nodes drifting about, joined by thin lines while they are near each other.
    Network,
    /// A slowly turning sphere of nodes and lines.
    Globe,
}

impl Style {
    pub const NAMES: &'static str = "plasma | aurora | lava | waves | matrix | rain | aquarium | network | globe";
    #[cfg(test)]
    pub const ALL: [Style; 9] = [
        Style::Plasma, Style::Aurora, Style::Lava, Style::Waves, Style::Matrix, Style::Rain, Style::Aquarium,
        Style::Network, Style::Globe,
    ];

    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plasma" => Some(Style::Plasma),
            "aurora" | "curtains" | "tende" => Some(Style::Aurora),
            "lava" | "blobs" | "lavalamp" => Some(Style::Lava),
            "waves" | "silk" | "onde" | "seta" => Some(Style::Waves),
            "matrix" | "digital" | "code" | "codice" => Some(Style::Matrix),
            "rain" | "streaks" | "pioggia" | "minimal" => Some(Style::Rain),
            "aquarium" | "acquario" | "water" | "acqua" | "tank" => Some(Style::Aquarium),
            "network" | "net" | "rete" | "plexus" | "nodes" | "nodi" => Some(Style::Network),
            "globe" | "sphere" | "world" | "globo" | "sfera" | "mondo" => Some(Style::Globe),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Palettes
// ---------------------------------------------------------------------------

/// A looping colour gradient: the last stop blends back into the first.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub stops: Vec<Rgb>,
}

const PASTEL: [Rgb; 6] = [
    (0xFF, 0xB3, 0xBA), (0xFF, 0xDF, 0xBA), (0xFF, 0xFF, 0xBA), (0xBA, 0xFF, 0xC9), (0xBA, 0xE1, 0xFF), (0xDA, 0xC4, 0xFF),
];
const FLUO: [Rgb; 5] = [
    (0xFF, 0x00, 0x80), (0xFF, 0xF0, 0x00), (0x00, 0xFF, 0x78), (0x00, 0xC8, 0xFF), (0x96, 0x00, 0xFF),
];
const NEON: [Rgb; 4] = [(0xFF, 0x00, 0xC8), (0x6E, 0x00, 0xFF), (0x00, 0x8C, 0xFF), (0x00, 0xFF, 0xDC)];
const CANDY: [Rgb; 5] = [
    (0xFF, 0x8A, 0xC8), (0xFF, 0xC8, 0xE8), (0x9A, 0xE6, 0xFF), (0xB4, 0xFF, 0xDC), (0xD8, 0xA8, 0xFF),
];
const SUNSET: [Rgb; 5] = [
    (0x3A, 0x10, 0x70), (0xC8, 0x20, 0x80), (0xFF, 0x60, 0x30), (0xFF, 0xC0, 0x50), (0xB0, 0x20, 0x90),
];
const OCEAN: [Rgb; 4] = [(0x00, 0x20, 0x70), (0x00, 0x78, 0xD0), (0x00, 0xC8, 0xD0), (0x20, 0x50, 0xB0)];
const AURORA: [Rgb; 5] = [
    (0x00, 0x30, 0x40), (0x00, 0xC0, 0x80), (0x60, 0xFF, 0xB0), (0x70, 0x40, 0xD0), (0x10, 0x20, 0x70),
];
const FIRE: [Rgb; 5] = [
    (0x30, 0x00, 0x00), (0xC0, 0x20, 0x00), (0xFF, 0x80, 0x00), (0xFF, 0xD0, 0x40), (0x80, 0x10, 0x00),
];
const FOREST: [Rgb; 4] = [(0x00, 0x30, 0x18), (0x10, 0x80, 0x40), (0x90, 0xD0, 0x50), (0x00, 0x50, 0x40)];
const ICE: [Rgb; 4] = [(0x30, 0x50, 0xB0), (0x70, 0xB0, 0xFF), (0xD0, 0xF4, 0xFF), (0x60, 0x90, 0xE0)];
const MONO: [Rgb; 3] = [(0x20, 0x24, 0x30), (0xB8, 0xC0, 0xD0), (0x50, 0x58, 0x68)];
// One-colour palettes, made for the rain styles (matrix, rain). In the smooth styles they
// turn into a dark -> colour -> light gradient of that colour.
const MATRIX: [Rgb; 1] = [(0x00, 0xFF, 0x41)];
const AMBER: [Rgb; 1] = [(0xFF, 0xB0, 0x00)];
const VIOLET: [Rgb; 1] = [(0xB0, 0x60, 0xFF)];
const CRIMSON: [Rgb; 1] = [(0xFF, 0x30, 0x40)];
const SNOW: [Rgb; 1] = [(0xE6, 0xEB, 0xF0)];
// Water palettes, made for the aquarium (the lightest colour is the surface, the darkest the depths).
const LAGOON: [Rgb; 3] = [(0x7F, 0xF0, 0xE6), (0x10, 0xA8, 0xB8), (0x04, 0x4A, 0x6A)];
const DEEP: [Rgb; 3] = [(0x4A, 0x90, 0xE0), (0x10, 0x40, 0x90), (0x02, 0x0C, 0x30)];
const TROPICAL: [Rgb; 4] = [(0x8A, 0xFF, 0xC8), (0x20, 0xC0, 0xA0), (0x00, 0x70, 0x80), (0x00, 0x30, 0x50)];

/// Names of the built-in presets, in the order `all` / `rotate` goes through them.
const PRESETS: [(&str, &[Rgb]); 20] = [
    ("pastel", &PASTEL),
    ("fluo", &FLUO),
    ("neon", &NEON),
    ("candy", &CANDY),
    ("sunset", &SUNSET),
    ("ocean", &OCEAN),
    ("aurora", &AURORA),
    ("fire", &FIRE),
    ("forest", &FOREST),
    ("ice", &ICE),
    ("mono", &MONO),
    ("rainbow", &[]), // generated (full hue circle)
    ("matrix", &MATRIX),
    ("amber", &AMBER),
    ("violet", &VIOLET),
    ("crimson", &CRIMSON),
    ("snow", &SNOW),
    ("lagoon", &LAGOON),
    ("deep", &DEEP),
    ("tropical", &TROPICAL),
];
/// How many of the presets are multi-colour (what `all` / `rotate` goes through).
const MULTI_COLOUR_PRESETS: usize = 12;

/// The full hue circle as a looping palette.
fn rainbow_stops() -> Vec<Rgb> {
    (0..6)
        .map(|i| {
            let h = i as f32 * 60.0;
            hsv(h, 0.85, 1.0)
        })
        .collect()
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8)
}

impl Palette {
    pub const NAMES: &'static str =
        "pastel | fluo | neon | candy | sunset | ocean | aurora | fire | forest | ice | mono | rainbow | matrix | amber | violet | crimson | snow | lagoon | deep | tropical";

    pub fn custom(stops: Vec<Rgb>) -> Palette {
        Palette { stops }
    }

    pub fn parse(s: &str) -> Option<Palette> {
        let n = match s.trim().to_ascii_lowercase().as_str() {
            "pastel" | "pastello" => "pastel",
            "fluo" | "fluorescent" | "fluorescente" => "fluo",
            "neon" => "neon",
            "candy" | "caramella" => "candy",
            "sunset" | "tramonto" => "sunset",
            "ocean" | "oceano" => "ocean",
            "aurora" => "aurora",
            "fire" | "fuoco" | "lava" => "fire",
            "forest" | "foresta" => "forest",
            "ice" | "ghiaccio" => "ice",
            "mono" | "grey" | "gray" | "grigio" => "mono",
            "rainbow" | "arcobaleno" => "rainbow",
            "matrix" | "green" | "verde" => "matrix",
            "amber" | "ambra" => "amber",
            "violet" | "viola" | "purple" => "violet",
            "crimson" | "red" | "rosso" => "crimson",
            "snow" | "white" | "bianco" => "snow",
            "lagoon" | "laguna" => "lagoon",
            "deep" | "profondo" | "abyss" => "deep",
            "tropical" | "tropicale" => "tropical",
            _ => return None,
        };
        PRESETS.iter().find(|(name, _)| *name == n).map(|(_, stops)| Palette {
            stops: if stops.is_empty() { rainbow_stops() } else { stops.to_vec() },
        })
    }

    /// Every preset (what `all` / `rotate` expands to).
    pub fn presets() -> Vec<Palette> {
        PRESETS
            .iter()
            .take(MULTI_COLOUR_PRESETS)
            .map(|(_, stops)| Palette { stops: if stops.is_empty() { rainbow_stops() } else { stops.to_vec() } })
            .collect()
    }

    /// Colour of one falling column of the rain styles: the one colour of a single-colour
    /// palette, or a spot on the loop of a multi-colour one.
    fn pick(&self, u: f32) -> Rgb {
        if self.stops.len() == 1 {
            self.stops[0]
        } else {
            self.at(u)
        }
    }

    /// Colour of the water at depth `u` (0 = surface, 1 = bottom): the palette's colours
    /// ordered from the lightest to the darkest, whatever order they were written in. A
    /// single colour becomes a light -> dark gradient of itself.
    fn depth_stops(&self) -> Vec<Rgb> {
        let mut st = self.stops.clone();
        match st.len() {
            0 => return vec![(0, 0, 0), (0, 0, 0)],
            1 => {
                let c = st[0];
                st = vec![lerp_rgb(c, (255, 255, 255), 0.35), c, dim(c, 0.25)];
            }
            _ => {}
        }
        let luma = |c: &Rgb| c.0 as u32 * 299 + c.1 as u32 * 587 + c.2 as u32 * 114;
        st.sort_by_key(|c| std::cmp::Reverse(luma(c)));
        st
    }

    /// Colour at `u` (0-1) of the closed loop.
    pub(crate) fn at(&self, u: f32) -> Rgb {
        match self.stops.len() {
            0 => (0, 0, 0),
            1 => {
                // dark -> colour -> light -> back, so a single colour still has some depth
                let c = self.stops[0];
                let t = u.rem_euclid(1.0);
                let tri = 1.0 - (t * 2.0 - 1.0).abs(); // 0 .. 1 .. 0
                let light = lerp_rgb(c, (255, 255, 255), 0.45);
                if tri < 0.5 {
                    lerp_rgb(dim(c, 0.25), c, tri * 2.0)
                } else {
                    lerp_rgb(c, light, (tri - 0.5) * 2.0)
                }
            }
            n => {
                let x = u.rem_euclid(1.0) * n as f32;
                let i = (x.floor() as usize).min(n - 1);
                lerp_rgb(self.stops[i], self.stops[(i + 1) % n], x - i as f32)
            }
        }
    }
}

/// Darken `c` to `k` (0-1) of its brightness. Light colours (pastels) would turn into
/// mud if they were simply multiplied, so the saturation rises a little as the
/// colour gets darker: a dim pastel pink becomes a dusty rose, not a grey-brown.
fn dim(c: Rgb, k: f32) -> Rgb {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    if mx <= 0.0 || k <= 0.0 {
        return (0, 0, 0);
    }
    let s = (mx - mn) / mx;
    let v = mx * k;
    let s2 = (s + (1.0 - s) * (1.0 - k) * 0.45).min(1.0);
    // Position of each channel between the darkest (0) and brightest (1) one.
    let h = |x: f32| if mx > mn { (x - mn) / (mx - mn) } else { 0.0 };
    let ch = |x: f32| ((v * (1.0 - s2 * (1.0 - h(x)))) * 255.0).round().clamp(0.0, 255.0) as u8;
    (ch(r), ch(g), ch(b))
}

/// Ink of the light variant: the colour a little darker, and paler the lower the brightness.
fn light_ink(c: Rgb, k: f32) -> Rgb {
    lerp_rgb((255, 255, 255), dim(c, 0.72), 0.45 + 0.55 * k)
}

/// Water of the light variant: a pale tint at the surface, deeper (more colour) towards the bottom.
fn light_water(c: Rgb, depth: f32, k: f32) -> Rgb {
    let m = (0.38 + 0.42 * depth) * (0.5 + 0.5 * k);
    lerp_rgb((255, 255, 255), c, m.min(1.0))
}

/// Colour at `u` (0-1) along an open gradient (at least two stops).
fn along(stops: &[Rgb], u: f32) -> Rgb {
    let n = stops.len();
    let x = u.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (x.floor() as usize).min(n - 2);
    lerp_rgb(stops[i], stops[i + 1], x - i as f32)
}

fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// What the user configures (`effect_*` keys).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// `None` = no effect (the normal background applies).
    pub style: Option<Style>,
    /// One palette is fixed; several rotate (cross-faded) every `palette_interval` s.
    pub palettes: Vec<Palette>,
    pub palette_interval: u32,
    /// 0 (frozen) - 100 (fast). 30 is a slow drift.
    pub speed: u32,
    /// Size of the features in % (100 = default, 200 = twice as large and calmer).
    pub scale: u32,
    /// 0-100: how bright the colours are (lower = easier to read the text on top).
    pub brightness: u32,
    /// Frames per second the effect is refreshed at while nothing else needs more.
    pub fps: u32,
    /// Aquarium: how high the water reaches, in % of the panel (0 = empty, 100 = full).
    /// Network and globe: how many nodes there are (0 = few, 100 = many).
    pub level: u32,
    /// Matrix, rain, aquarium, network and globe: a light (white) variant instead of the dark one.
    pub light: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            style: None,
            palettes: vec![Palette::parse("neon").expect("preset")],
            palette_interval: 60,
            speed: 30,
            scale: 100,
            brightness: 40,
            fps: 8,
            level: 50,
            light: false,
        }
    }
}

impl Settings {
    /// The style to draw: the configured one, or the plasma when the effect is
    /// wanted by another setting (`music_bg = effect`) without a style of its own.
    pub fn style_or_default(&self) -> Style {
        self.style.unwrap_or(Style::Plasma)
    }

    /// Does the picture change over time (so it needs a steady frame rate)?
    pub fn is_moving(&self) -> bool {
        self.speed > 0
    }
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

/// Identifies the colour table currently built (so it is rebuilt only when needed).
#[derive(Clone, Copy, PartialEq)]
struct LutKey {
    a: usize,
    b: usize,
    /// Cross-fade amount, quantised to 1/64.
    f: u8,
    brightness: u32,
    light: bool,
    /// Hash of the palette contents (changes when the user edits them).
    sig: u64,
}

pub struct Effect {
    start: std::time::Instant,
    lut: Vec<[u8; 3]>,
    /// Colours of the rain columns (one per 1/RAIN_N of the palette loop), brightness applied.
    rain_lut: Vec<[u8; 3]>,
    /// Colours of the aquarium's water from the surface (0) to the bottom, brightness applied.
    water_lut: Vec<[u8; 3]>,
    /// Brightness (0-1) the colour tables were built for.
    bright: f32,
    /// Light variant: the paper colour (air, rain background) and the colour of the water's edge.
    light: bool,
    paper: [u8; 3],
    edge: [u8; 3],
    lut_key: Option<LutKey>,
    /// Field samples (fixed point, in LUT entries), `cols * rows`.
    cells: Vec<i32>,
    cols: usize,
    rows: usize,
    /// One interpolated row of samples.
    row: Vec<i32>,
    /// Pixel position of each column / row of samples, in formula units.
    xs: Vec<f32>,
    ys: Vec<f32>,
    grid_key: Option<(usize, usize, u32)>,
}

impl Effect {
    pub fn new() -> Self {
        Effect {
            start: std::time::Instant::now(),
            lut: vec![[0; 3]; LUT_N],
            rain_lut: vec![[0; 3]; RAIN_N],
            water_lut: vec![[0; 3]; WATER_N],
            bright: 0.4,
            light: false,
            paper: [255; 3],
            edge: [0; 3],
            lut_key: None,
            cells: Vec::new(),
            cols: 0,
            rows: 0,
            row: Vec::new(),
            xs: Vec::new(),
            ys: Vec::new(),
            grid_key: None,
        }
    }

    /// Paint the effect over all of `fb` at the current time.
    pub fn render(&mut self, fb: &mut Framebuffer, s: &Settings, style: Style, brightness: u32) {
        let t = self.start.elapsed().as_secs_f64();
        self.render_at(fb, s, style, brightness, t);
    }

    /// Same, at an explicit time in seconds (tests, previews).
    pub fn render_at(&mut self, fb: &mut Framebuffer, s: &Settings, style: Style, brightness: u32, secs: f64) {
        let (w, h) = (fb.width() as usize, fb.height() as usize);
        if w == 0 || h == 0 || s.palettes.is_empty() {
            return;
        }
        self.update_lut(s, brightness, secs);
        if style == Style::Aquarium {
            let t = secs * (s.speed as f64 / 30.0);
            self.paint_aquarium(fb, s.level, s.scale, t);
            return;
        }
        if matches!(style, Style::Matrix | Style::Rain) {
            let t = secs * (s.speed as f64 / 30.0);
            self.paint_rain(fb, style, s.scale, t);
            return;
        }
        if matches!(style, Style::Network | Style::Globe) {
            let t = secs * (s.speed as f64 / 30.0);
            if style == Style::Network {
                self.paint_network(fb, s.level, s.scale, t);
            } else {
                self.paint_globe(fb, s.level, s.scale, t);
            }
            return;
        }
        self.update_grid(w, h, s.scale);
        // Time that drives the motion: speed 100 is ~3x the default (30).
        let t = secs * (s.speed as f64 / 30.0);
        self.fill_cells(style, t);
        self.paint(fb, w, h);
    }

    // ----- colour table -----

    fn update_lut(&mut self, s: &Settings, brightness: u32, secs: f64) {
        let n = s.palettes.len();
        let (a, b, f) = if n == 1 {
            (0, 0, 0.0)
        } else {
            let interval = s.palette_interval.max(5) as f64;
            let idx = (secs / interval).floor() as usize;
            let phase = secs % interval;
            let cur = idx % n;
            if phase < FADE_SECS {
                ((cur + n - 1) % n, cur, (phase / FADE_SECS) as f32)
            } else {
                (cur, cur, 0.0)
            }
        };
        let fq = (f * 64.0).round() as u8;
        let sig = palette_sig(&s.palettes[a]) ^ palette_sig(&s.palettes[b]).rotate_left(17);
        let key = LutKey { a, b, f: fq, brightness, light: s.light, sig };
        if self.lut_key == Some(key) {
            return;
        }
        let k = brightness.min(100) as f32 / 100.0;
        self.bright = k;
        self.light = s.light;
        let fade = fq as f32 / 64.0;
        let light = s.light;
        for (i, e) in self.rain_lut.iter_mut().enumerate() {
            let u = i as f32 / RAIN_N as f32;
            let ca = s.palettes[a].pick(u);
            let c = if a == b { ca } else { lerp_rgb(ca, s.palettes[b].pick(u), fade) };
            let d = if light { light_ink(c, k) } else { dim(c, k) };
            *e = [d.0, d.1, d.2];
        }
        let (sa, sb) = (s.palettes[a].depth_stops(), s.palettes[b].depth_stops());
        for (i, e) in self.water_lut.iter_mut().enumerate() {
            let u = i as f32 / (WATER_N - 1) as f32;
            // the light variant never goes all the way to the darkest colour: it would turn grey
            let uu = if light { u * 0.7 } else { u };
            let ca = along(&sa, uu);
            let c = if a == b { ca } else { lerp_rgb(ca, along(&sb, uu), fade) };
            let d = if light { light_water(c, u, k) } else { dim(c, k) };
            *e = [d.0, d.1, d.2];
        }
        if light {
            // a faint tint of the palette in the white, and a darker edge for the water surface
            let tint = |p: &Palette| lerp_rgb((255, 255, 255), p.pick(0.0), 0.06);
            let edge = |st: &[Rgb]| lerp_rgb((255, 255, 255), dim(along(st, 0.5), 0.55), 0.6 + 0.4 * k);
            let (pa, pb) = (tint(&s.palettes[a]), tint(&s.palettes[b]));
            let (ea, eb) = (edge(&sa), edge(&sb));
            let (p, e) = if a == b { (pa, ea) } else { (lerp_rgb(pa, pb, fade), lerp_rgb(ea, eb, fade)) };
            self.paper = [p.0, p.1, p.2];
            self.edge = [e.0, e.1, e.2];
        }
        for (i, e) in self.lut.iter_mut().enumerate() {
            let u = i as f32 / LUT_N as f32;
            let ca = s.palettes[a].at(u);
            let c = if a == b { ca } else { lerp_rgb(ca, s.palettes[b].at(u), fade) };
            let d = dim(c, k);
            *e = [d.0, d.1, d.2];
        }
        self.lut_key = Some(key);
    }

    // ----- field grid -----

    fn update_grid(&mut self, w: usize, h: usize, scale: u32) {
        let key = (w, h, scale);
        if self.grid_key == Some(key) {
            return;
        }
        self.cols = w / CELL + 2;
        self.rows = h / CELL + 2;
        let unit = UNIT_PX * scale.clamp(25, 400) as f32 / 100.0;
        // Centre the pattern on the panel so resizing doesn't make it jump around.
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        self.xs = (0..self.cols).map(|c| ((c * CELL) as f32 - cx) / unit).collect();
        self.ys = (0..self.rows).map(|r| ((r * CELL) as f32 - cy) / unit).collect();
        self.cells = vec![0; self.cols * self.rows];
        self.row = vec![0; self.cols];
        self.grid_key = Some(key);
    }

    /// Evaluate the field at every sample (the only trigonometry of a frame).
    fn fill_cells(&mut self, style: Style, secs: f64) {
        let ph = |rate: f64| ((secs * rate) % std::f64::consts::TAU) as f32;
        // Palette position -> fixed point LUT units.
        let to_fx = |pos: f32| (pos * (LUT_N as f32) * (1u32 << FRAC) as f32) as i32;
        let (cols, rows) = (self.cols, self.rows);
        match style {
            Style::Plasma => {
                let (p1, p2, p3, p4) = (ph(0.31), ph(0.23), ph(0.17), ph(0.29));
                let (cx, cy) = (1.4 * ph(0.11).sin(), 0.5 * ph(0.07).cos());
                let col_term: Vec<f32> = self.xs.iter().map(|&x| (x * 1.05 + p1).sin()).collect();
                let row_term: Vec<f32> = self.ys.iter().map(|&y| (y * 2.1 - p2).sin()).collect();
                for r in 0..rows {
                    let y = self.ys[r];
                    for c in 0..cols {
                        let x = self.xs[c];
                        let diag = ((x * 0.8 + y * 1.3) + p3).sin();
                        let (dx, dy) = (x - cx, y - cy);
                        let rad = ((dx * dx + dy * dy).sqrt() * 1.6 + p4).sin();
                        let sum = col_term[c] + row_term[r] + diag + rad; // -4 .. 4
                        self.cells[r * cols + c] = to_fx(0.5 + sum * 0.2);
                    }
                }
            }
            Style::Aurora => {
                let (p1, p2, p3) = (ph(0.19), ph(0.27), ph(0.13));
                let col_wave: Vec<f32> = self.xs.iter().map(|&x| (x * 0.9 + p1).sin() * 0.9 + (x * 2.1 - p2).sin() * 0.35).collect();
                for r in 0..rows {
                    let y = self.ys[r];
                    for c in 0..cols {
                        let x = self.xs[c];
                        let warp = ((x * 0.55 + y * 0.9) + p3).sin() * 0.45;
                        let v = y * 1.1 + col_wave[c] + warp;
                        self.cells[r * cols + c] = to_fx(0.5 + v * 0.30);
                    }
                }
            }
            Style::Lava => {
                // Five soft blobs on slow Lissajous paths; the field is their summed influence.
                let blobs: [(f64, f64, f32, f32, f32, f32); 5] = [
                    (0.19, 0.13, 1.6, 0.55, 0.0, 0.70),
                    (0.11, 0.21, 2.1, 0.45, 1.3, 0.60),
                    (0.23, 0.17, 1.1, 0.50, 2.6, 0.55),
                    (0.15, 0.09, 2.6, 0.40, 3.9, 0.65),
                    (0.07, 0.25, 0.7, 0.45, 5.1, 0.50),
                ];
                let pos: Vec<(f32, f32, f32)> = blobs
                    .iter()
                    .map(|&(fx, fy, ax, ay, off, r)| ((ph(fx) + off).sin() * ax, (ph(fy) + off * 0.7).cos() * ay, r))
                    .collect();
                for r in 0..rows {
                    let y = self.ys[r];
                    for c in 0..cols {
                        let x = self.xs[c];
                        let mut f = 0.0f32;
                        for &(bx, by, rad) in &pos {
                            let (dx, dy) = (x - bx, y - by);
                            f += rad * rad / (dx * dx + dy * dy + 0.05);
                        }
                        // Squash to 0..~0.7: the blob centres reach the palette's bright colours and the
                        // cyclic palette never wraps through a seam.
                        let v = 1.0 - 1.0 / (1.0 + f * 0.5);
                        self.cells[r * cols + c] = to_fx(v * 0.7);
                    }
                }
            }
            // The rain styles are not a smooth field: `render_at` paints them directly.
            Style::Matrix | Style::Rain | Style::Aquarium | Style::Network | Style::Globe => {}
            Style::Waves => {
                let (p1, p2, p3) = (ph(0.21), ph(0.15), ph(0.25));
                for r in 0..rows {
                    let y = self.ys[r];
                    for c in 0..cols {
                        let x = self.xs[c];
                        let bend = (x * 0.5 - p2).sin() * 1.2 + (y * 0.9 + p3).sin() * 0.5;
                        let v = (x * 0.7 + y * 1.4 + bend + p1).sin() * 0.6 + (x * 1.5 - y * 0.6 - p1 * 0.7).sin() * 0.25;
                        self.cells[r * cols + c] = to_fx(0.5 + v * 0.4);
                    }
                }
            }
        }
    }

    // ----- upscale + colour lookup -----

    fn paint(&mut self, fb: &mut Framebuffer, w: usize, h: usize) {
        let mask = (LUT_N - 1) as i32;
        let shift = FRAC + STEP_FRAC;
        let cols = self.cols;
        let pixels = fb.as_bytes_mut();
        for y in 0..h {
            let (r, wy) = (y / CELL, (y % CELL) as i32);
            let top = &self.cells[r * cols..(r + 1) * cols];
            let bot = &self.cells[(r + 1) * cols..(r + 2) * cols];
            // Vertical blend of the two sample rows (cheap: `cols` values).
            for c in 0..cols {
                self.row[c] = top[c] + (bot[c] - top[c]) * wy / CELL as i32;
            }
            let out = &mut pixels[y * w * 3..(y + 1) * w * 3];
            let mut x = 0usize;
            for c in 0..cols - 1 {
                if x >= w {
                    break;
                }
                let run = CELL.min(w - x);
                let mut v = self.row[c] << STEP_FRAC;
                let dv = ((self.row[c + 1] - self.row[c]) << STEP_FRAC) / CELL as i32;
                for px in out[x * 3..(x + run) * 3].chunks_exact_mut(3) {
                    let col = self.lut[((v >> shift) & mask) as usize];
                    px[0] = col[0];
                    px[1] = col[1];
                    px[2] = col[2];
                    v += dv;
                }
                x += run;
            }
        }
    }
}


// ---------------------------------------------------------------------------
// Digital rain (matrix, rain)
// ---------------------------------------------------------------------------

/// Characters of the `matrix` style (all exist in the 5x7 font).
const RAIN_CHARS: &str = "0123456789ABCDEFHKLMNPRSTUXYZ:-/";
/// Share of the columns that have a drop at all (the rest stay empty: minimal).
const RAIN_DENSITY: f32 = 0.55;

fn mix(a: u64, b: u64) -> u64 {
    // splitmix64-style finaliser over two numbers: cheap, stateless randomness.
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ 0x94D0_49BB_1331_11EB;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Uniform 0..1 from a hash.
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// One falling drop of a column: where its head is (in cells/pixels along the fall) and how
/// long its trail is, at time `t`; `None` while the column is between two drops.
struct Drop {
    head: f32,
    len: f32,
    epoch: u64,
}

/// The drop of column `col` at time `t` (all quantities in `unit`s: cells for matrix, pixels
/// for rain). `travel` is the length of the screen in those units.
fn drop_of(col: u64, t: f64, travel: f32, speed: f32, len_lo: f32, len_hi: f32) -> Option<Drop> {
    if unit(mix(col, 2)) > RAIN_DENSITY {
        return None;
    }
    let speed_c = speed * (0.6 + 0.9 * unit(mix(col, 3)));
    let len = len_lo + (len_hi - len_lo) * unit(mix(col, 4));
    let gap = unit(mix(col, 5)) * travel * 1.3;
    let period = travel + len + gap;
    let pos = t as f32 * speed_c + unit(mix(col, 6)) * period;
    let epoch = (pos / period).floor();
    let head = pos - epoch * period;
    (head <= travel + len).then_some(Drop { head, len, epoch: epoch as u64 })
}

impl Effect {
    /// Paint the rain over all of `fb`: black with a trace of the palette's colour, and drops
    /// whose colour comes from the palette (`rain_lut`). Stateless: a pure function of `t`.
    fn paint_rain(&mut self, fb: &mut Framebuffer, style: Style, scale: u32, t: f64) {
        let (w, h) = (fb.width() as i32, fb.height() as i32);
        let base = self.rain_lut[0];
        let ground = if self.light { self.paper } else { [base[0] / 14, base[1] / 14, base[2] / 14] };
        for px in fb.as_bytes_mut().chunks_exact_mut(3) {
            px.copy_from_slice(&ground);
        }
        let g = ((2.0 * scale.clamp(25, 400) as f32 / 100.0).round() as i32).clamp(1, 8);
        match style {
            Style::Matrix => self.paint_matrix(fb, w, h, g, t),
            _ => self.paint_streaks(fb, w, h, g, t),
        }
    }

    /// The colour of a drop's tail at intensity `i` (1 = full): it fades into the background.
    fn shade(&self, c: [u8; 3], i: f32) -> [u8; 3] {
        if self.light {
            let p = self.paper;
            let f = |v: u8, g: u8| (g as f32 + (v as f32 - g as f32) * i) as u8;
            [f(c[0], p[0]), f(c[1], p[1]), f(c[2], p[2])]
        } else {
            [(c[0] as f32 * i) as u8, (c[1] as f32 * i) as u8, (c[2] as f32 * i) as u8]
        }
    }

    /// The head of a drop: lighter than its tail on dark, darker on light.
    fn head(&self, c: [u8; 3], white: u32) -> [u8; 3] {
        if self.light {
            [(c[0] as u32 * 3 / 4) as u8, (c[1] as u32 * 3 / 4) as u8, (c[2] as u32 * 3 / 4) as u8]
        } else {
            head_colour(c, white)
        }
    }

    fn paint_matrix(&mut self, fb: &mut Framebuffer, w: i32, h: i32, g: i32, t: f64) {
        let (cw, ch) = (8 * g, 10 * g);
        let (cols, rows) = (w / cw + 1, h / ch + 1);
        let chars: Vec<char> = RAIN_CHARS.chars().collect();
        let white = (255.0 * self.bright) as u32;
        for col in 0..cols {
            let Some(d) = drop_of(col as u64, t, rows as f32, 5.0, 5.0, 17.0) else { continue };
            let c = self.rain_lut[(unit(mix(col as u64, 7)) * RAIN_N as f32) as usize % RAIN_N];
            let head_row = d.head.floor() as i32;
            let flicker = (t * 9.0).floor() as u64;
            for k in 0..d.len.ceil() as i32 {
                let row = head_row - k;
                if row < 0 || row >= rows {
                    continue;
                }
                let i = (1.0 - k as f32 / d.len).clamp(0.0, 1.0);
                let i = i * i;
                if i < 0.04 {
                    continue;
                }
                let seed = mix(col as u64 * 7919 + row as u64, if k == 0 { flicker } else { d.epoch });
                let glyph = trofeo_lcd::font::glyph(chars[(seed % chars.len() as u64) as usize]);
                // the head is the brightest, a little whiter; the tail fades into the colour
                let rgb = if k == 0 { self.head(c, white) } else { self.shade(c, i) };
                let (x0, y0) = (col * cw + g, row * ch + g);
                for (ry, bits) in glyph.iter().enumerate() {
                    for cx in 0..5 {
                        if (bits >> (4 - cx)) & 1 == 1 {
                            fill(fb, x0 + cx * g, y0 + ry as i32 * g, g, g, rgb);
                        }
                    }
                }
            }
        }
    }

    fn paint_streaks(&mut self, fb: &mut Framebuffer, w: i32, h: i32, g: i32, t: f64) {
        let spacing = 10 * g;
        let cols = w / spacing + 1;
        let width = g.max(1);
        let white = (255.0 * self.bright) as u32;
        for col in 0..cols {
            let Some(d) = drop_of(col as u64, t, h as f32, 60.0 * g as f32, 70.0 * g as f32, 210.0 * g as f32) else { continue };
            let c = self.rain_lut[(unit(mix(col as u64, 7)) * RAIN_N as f32) as usize % RAIN_N];
            let x = col * spacing + (spacing - width) / 2;
            let head = d.head as i32;
            for k in 0..d.len as i32 {
                let y = head - k;
                if y < 0 || y >= h {
                    continue;
                }
                let i = (1.0 - k as f32 / d.len).clamp(0.0, 1.0).powf(1.6);
                let rgb = if k < width { self.head(c, white) } else { self.shade(c, i) };
                fill(fb, x, y, width, 1, rgb);
            }
        }
    }
}

/// The brightest point of a drop: its colour, half way to white (at the effect's brightness).
fn head_colour(c: [u8; 3], white: u32) -> [u8; 3] {
    let f = |v: u8| (v as u32 + white.saturating_sub(v as u32) / 2) as u8;
    [f(c[0]), f(c[1]), f(c[2])]
}

/// Fills a rectangle (clipped to the framebuffer).
fn fill(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3]) {
    let (fw, fh) = (fb.width() as i32, fb.height() as i32);
    let (x0, x1) = (x.max(0), (x + w).min(fw));
    let (y0, y1) = (y.max(0), (y + h).min(fh));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let px = fb.as_bytes_mut();
    for yy in y0..y1 {
        for p in px[((yy * fw + x0) * 3) as usize..((yy * fw + x1) * 3) as usize].chunks_exact_mut(3) {
            p.copy_from_slice(&rgb);
        }
    }
}

// ---------------------------------------------------------------------------
// Aquarium (a side view of the water)
// ---------------------------------------------------------------------------

/// Height of the water surface at column `x`: a few slow waves added together, all
/// measured from the resting level `y0`.
fn surface_at(x: f32, y0: f32, amp: f32, g: f32, t: f32) -> f32 {
    let (l1, l2, l3) = (110.0 * g, 61.0 * g, 33.0 * g);
    y0 + amp * (0.55 * (x / l1 + 0.9 * t).sin() + 0.30 * (x / l2 - 1.3 * t + 1.7).sin() + 0.15 * (x / l3 + 2.1 * t + 4.0).sin())
}

/// Number of bubbles for a panel `w` pixels wide.
fn bubble_count(w: usize, g: f32) -> u64 {
    ((w as f32 / (150.0 * g.clamp(0.5, 3.0))) as u64).clamp(3, 40)
}

impl Effect {
    /// A cross-section of an aquarium: air above, water below, with a clearly visible moving
    /// surface. Under it the colour darkens with depth, soft rays of light slant down from
    /// the surface, a net of caustics ripples near the top, and a few bubbles rise.
    /// Stateless: a pure function of `t`.
    fn paint_aquarium(&mut self, fb: &mut Framebuffer, level: u32, scale: u32, t: f64) {
        let (w, h) = (fb.width() as usize, fb.height() as usize);
        let g = scale.clamp(25, 400) as f32 / 100.0;
        let tf = (t % 100_000.0) as f32;
        let amp = (w.min(h) as f32 * 0.045) * g.sqrt();
        let y0 = h as f32 * (1.0 - level.min(100) as f32 / 100.0);
        let wl = &self.water_lut;
        let light_theme = self.light;
        let air = if light_theme {
            self.paper
        } else {
            let a = wl[WATER_N - 1];
            [a[0] / 2, a[1] / 2, a[2] / 2]
        };
        let px = fb.as_bytes_mut();
        if level == 0 {
            for p in px.chunks_exact_mut(3) {
                p.copy_from_slice(&air);
            }
            return;
        }
        let surf: Vec<f32> = (0..w).map(|x| surface_at(x as f32 + 0.5, y0, amp, g, tf)).collect();
        // 1-D patterns that are read along slanted lines: two crossing ridge patterns make the
        // caustics' net, a wide soft one makes the rays. Indexed by x plus a per-row offset.
        let (n_rays, n_c1, n_c2) = (w + h * 3 / 10 + 2, w + h * 45 / 100 + 2, w + h * 7 / 10 + 2);
        let rays: Vec<u8> = (0..n_rays)
            .map(|u| {
                let u = u as f32;
                let a = 0.5 + 0.25 * (u / (60.0 * g) + 0.5 * tf).sin() + 0.25 * (u / (143.0 * g) - 0.3 * tf + 2.0).sin();
                let v = ((a - 0.52) / 0.48).clamp(0.0, 1.0);
                (v * v * 255.0) as u8
            })
            .collect();
        let ridge = |u: f32, period: f32, phase: f32| {
            let v = 1.0 - (u / period + phase).sin().abs();
            (v * v * v * 255.0) as u8
        };
        let c1: Vec<u8> = (0..n_c1).map(|u| ridge(u as f32, 17.0 * g, 0.9 * tf)).collect();
        let c2: Vec<u8> = (0..n_c2).map(|u| ridge(u as f32, 23.0 * g, -0.7 * tf + 1.3)).collect();
        // how much light reaches each depth
        let fade: Vec<u32> = (0..WATER_N)
            .map(|i| {
                let d = 1.0 - i as f32 / (WATER_N - 1) as f32;
                (255.0 * d * d) as u32
            })
            .collect();
        let span = (h as f32 - y0 + amp).max(1.0);
        let inv = (WATER_N - 1) as f32 / span;
        let top = wl[0];
        // what the rays and caustics add: the surface colour (dark theme) or plain white
        let light = if light_theme { [255, 255, 255] } else { [top[0] as i32, top[1] as i32, top[2] as i32] };
        // the line on the surface stays clearly visible even when the water is dimmed
        let white = (255.0 * self.bright.sqrt()) as u32;
        let line = if light_theme {
            [self.edge[0] as i32, self.edge[1] as i32, self.edge[2] as i32]
        } else {
            let f = |v: u8| (v as u32 + white.saturating_sub(v as u32) * 3 / 5).min(255) as i32;
            [f(top[0]), f(top[1]), f(top[2])]
        };
        let glow_k = if light_theme { 30.0 } else { 90.0 };
        let air_i = [air[0] as i32, air[1] as i32, air[2] as i32];
        for y in 0..h {
            let yf = y as f32 + 0.5;
            let (o_ray, o_c1, o_c2) = (y * 3 / 10, y * 45 / 100, (h - y) * 7 / 10);
            let row = &mut px[y * w * 3..(y + 1) * w * 3];
            for (x, p) in row.chunks_exact_mut(3).enumerate() {
                let dy = yf - surf[x];
                if dy < -1.7 {
                    p.copy_from_slice(&air);
                    continue;
                }
                let idx = ((dy.max(0.0) * inv) as usize).min(WATER_N - 1);
                let b = wl[idx];
                let mut c = [b[0] as i32, b[1] as i32, b[2] as i32];
                // light: caustics (crossing ridges) + rays, weaker with depth
                let caustic = (c1[x + o_c1] as u32 * c2[x + o_c2] as u32) >> 8;
                let l = ((caustic * 140 + rays[x + o_ray] as u32 * 100) >> 8) * fade[idx] >> 8;
                let l = l.min(255) as i32;
                for k in 0..3 {
                    c[k] += (light[k] - c[k]) * l >> 8;
                }
                if dy < 10.0 {
                    // a glow just under the surface, the water's edge and the bright line on it
                    let glow = ((1.0 - dy.max(0.0) / 10.0).powi(2) * glow_k) as i32;
                    let cov = ((dy + 0.5).clamp(0.0, 1.0) * 256.0) as i32;
                    let ln = ((1.0 - dy.abs() / 1.7).clamp(0.0, 1.0) * 256.0) as i32;
                    for k in 0..3 {
                        c[k] += (line[k] - c[k]) * glow >> 8;
                        c[k] = air_i[k] + ((c[k] - air_i[k]) * cov >> 8);
                        c[k] += (line[k] - c[k]) * ln >> 8;
                    }
                }
                p[0] = c[0].clamp(0, 255) as u8;
                p[1] = c[1].clamp(0, 255) as u8;
                p[2] = c[2].clamp(0, 255) as u8;
            }
        }
        let tint = if light_theme { [255, 255, 255] } else { line };
        self.paint_bubbles(fb, y0, amp, g, tf, tint);
    }

    fn paint_bubbles(&mut self, fb: &mut Framebuffer, y0: f32, amp: f32, g: f32, t: f32, tint: [i32; 3]) {
        let (w, h) = (fb.width() as i32, fb.height() as i32);
        let gb = g.clamp(0.5, 3.0);
        let travel = (h as f32 - y0).max(1.0);
        let px = fb.as_bytes_mut();
        for i in 0..bubble_count(w as usize, g) {
            let r = (2.0 + 3.0 * unit(mix(i, 12))) * gb;
            let speed = (26.0 + 34.0 * unit(mix(i, 13))) * gb;
            let gap = unit(mix(i, 15)) * travel * 0.8;
            let period = travel + 2.0 * r + gap;
            let pos = (t * speed + unit(mix(i, 14)) * period) % period;
            if pos > travel + r {
                continue; // between two bubbles
            }
            let cy = h as f32 + r - pos;
            let cx = unit(mix(i, 11)) * w as f32 + 5.0 * gb * (pos * 0.045 + i as f32).sin();
            let ys = surface_at(cx, y0, amp, g, t);
            let alpha = ((cy - r - ys) / 24.0).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let (x0, x1) = ((cx - r - 2.0).floor().max(0.0) as i32, ((cx + r + 2.0).ceil() as i32).min(w - 1));
            let (y_0, y_1) = ((cy - r - 2.0).floor().max(0.0) as i32, ((cy + r + 2.0).ceil() as i32).min(h - 1));
            for yy in y_0..=y_1 {
                for xx in x0..=x1 {
                    let (dx, dy) = (xx as f32 + 0.5 - cx, yy as f32 + 0.5 - cy);
                    let d = (dx * dx + dy * dy).sqrt();
                    let ring = (1.0 - (d - r).abs() / 1.1).clamp(0.0, 1.0);
                    let body = if d < r { 0.10 } else { 0.0 };
                    let shine = (1.0 - ((dx + 0.4 * r).powi(2) + (dy + 0.4 * r).powi(2)).sqrt() / (0.35 * r + 0.8)).clamp(0.0, 1.0);
                    let a = ((ring * 0.65 + body + shine * 0.5) * alpha).min(1.0);
                    if a <= 0.0 {
                        continue;
                    }
                    let o = ((yy * w + xx) * 3) as usize;
                    for k in 0..3 {
                        let v = px[o + k] as f32;
                        px[o + k] = (v + (tint[k] as f32 - v) * a) as u8;
                    }
                }
            }
        }
    }
}

impl Default for Effect {
    fn default() -> Self {
        Self::new()
    }
}

fn palette_sig(p: &Palette) -> u64 {
    // FNV-1a over the stops: cheap, and only used to notice that a palette changed.
    let mut h = 0xcbf29ce484222325u64;
    for &(r, g, b) in &p.stops {
        for byte in [r, g, b] {
            h ^= byte as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h ^ p.stops.len() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Network and globe (nodes joined by thin lines)
// ---------------------------------------------------------------------------

/// Grey the lines lean towards (dark / light variant).
const NET_GREY_DARK: Rgb = (205, 212, 225);
const NET_GREY_LIGHT: Rgb = (58, 68, 84);

/// A tiny anti-aliased drawing surface over the framebuffer's RGB bytes.
struct Pen<'a> {
    px: &'a mut [u8],
    w: i32,
    h: i32,
}

impl Pen<'_> {
    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let i = ((y * self.w + x) * 3) as usize;
        for k in 0..3 {
            let d = self.px[i + k] as f32;
            self.px[i + k] = (d + (c[k] as f32 - d) * a + 0.5) as u8;
        }
    }

    /// A one-pixel anti-aliased line (Wu), `a` is its opacity.
    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [u8; 3], a: f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        if (x0 < 0.0 && x1 < 0.0) || (y0 < 0.0 && y1 < 0.0) || (x0 >= w && x1 >= w) || (y0 >= h && y1 >= h) {
            return;
        }
        let steep = (y1 - y0).abs() > (x1 - x0).abs();
        let (mut ax, mut ay, mut bx, mut by) = if steep { (y0, x0, y1, x1) } else { (x0, y0, x1, y1) };
        if ax > bx {
            std::mem::swap(&mut ax, &mut bx);
            std::mem::swap(&mut ay, &mut by);
        }
        let dx = bx - ax;
        let grad = if dx < 1e-4 { 0.0 } else { (by - ay) / dx };
        let lim = if steep { self.h } else { self.w };
        let xs = (ax.ceil() as i32).max(0);
        let xe = (bx.floor() as i32).min(lim - 1);
        for x in xs..=xe {
            let y = ay + grad * (x as f32 - ax);
            let yi = y.floor();
            let f = y - yi;
            let yi = yi as i32;
            if steep {
                self.blend(yi, x, c, a * (1.0 - f));
                self.blend(yi + 1, x, c, a * f);
            } else {
                self.blend(x, yi, c, a * (1.0 - f));
                self.blend(x, yi + 1, c, a * f);
            }
        }
    }

    /// A filled disc.
    fn dot(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 3], a: f32) {
        let m = r + 1.5;
        for y in (cy - m).floor() as i32..=(cy + m).ceil() as i32 {
            for x in (cx - m).floor() as i32..=(cx + m).ceil() as i32 {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                self.blend(x, y, c, (r + 0.5 - d).clamp(0.0, 1.0) * a);
            }
        }
    }

    /// A thin circle.
    fn ring(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 3], a: f32) {
        let m = r + 2.0;
        for y in (cy - m).floor() as i32..=(cy + m).ceil() as i32 {
            for x in (cx - m).floor() as i32..=(cx + m).ceil() as i32 {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                self.blend(x, y, c, (1.1 - (d - r).abs()).clamp(0.0, 1.0) * a);
            }
        }
    }
}

/// A node of the network / globe, in screen pixels.
struct Node {
    x: f32,
    y: f32,
    /// 0 (far) .. 1 (near); always 1 in the flat network.
    depth: f32,
    /// Hash-based 0..1 values: palette position, size.
    hue: f32,
    size: f32,
    ring: bool,
}

impl Effect {
    fn net_ground(&self, fb: &mut Framebuffer) {
        let base = self.rain_lut[0];
        let ground = if self.light { self.paper } else { [base[0] / 14, base[1] / 14, base[2] / 14] };
        for px in fb.as_bytes_mut().chunks_exact_mut(3) {
            px.copy_from_slice(&ground);
        }
    }

    /// Colour of a line joining nodes of hues `a` and `b`: their colours, leaning to grey.
    fn net_line_colour(&self, a: f32, b: f32) -> [u8; 3] {
        let pick = |u: f32| self.rain_lut[((u * RAIN_N as f32) as usize).min(RAIN_N - 1)];
        let (ca, cb) = (pick(a), pick(b));
        let mid = lerp_rgb((ca[0], ca[1], ca[2]), (cb[0], cb[1], cb[2]), 0.5);
        let k = self.bright;
        let grey = if self.light { lerp_rgb((255, 255, 255), NET_GREY_LIGHT, 0.45 + 0.55 * k) } else { dim(NET_GREY_DARK, (k * 1.3).min(1.0)) };
        let c = lerp_rgb(mid, grey, 0.5);
        [c.0, c.1, c.2]
    }

    /// Draws the dots (and the rings of the few "hub" nodes) of `nodes`.
    fn net_nodes(&self, pen: &mut Pen, nodes: &[Node], r0: f32) {
        let white = (255.0 * self.bright) as u32;
        for n in nodes {
            let c = self.rain_lut[((n.hue * RAIN_N as f32) as usize).min(RAIN_N - 1)];
            let r = r0 * (0.75 + 0.9 * n.size) * (0.55 + 0.45 * n.depth);
            let a = 0.4 + 0.6 * n.depth;
            pen.dot(n.x, n.y, r, c, a);
            if n.ring && n.depth > 0.45 {
                pen.ring(n.x, n.y, r * 2.6 + 1.0, self.head(c, white), 0.85 * a);
            }
        }
    }

    /// A flat network: nodes drifting on small orbits around their places, joined by thin
    /// lines while they are near each other (the lines fade as they get longer).
    fn paint_network(&mut self, fb: &mut Framebuffer, level: u32, scale: u32, t: f64) {
        self.net_ground(fb);
        let (w, h) = (fb.width() as f32, fb.height() as f32);
        let s = ((170.0 - 100.0 * level.min(100) as f32 / 100.0) * scale.clamp(25, 400) as f32 / 100.0).clamp(36.0, 600.0);
        let link = 1.38 * s;
        let (gx0, gy0) = (-1i32, -1i32);
        let (gx1, gy1) = ((w / s).ceil() as i32 + 1, (h / s).ceil() as i32 + 1);
        let mut nodes: Vec<Node> = Vec::new();
        for gy in gy0..=gy1 {
            for gx in gx0..=gx1 {
                let id = ((gx + 64) as u64) << 20 | (gy + 64) as u64;
                if unit(mix(id, 11)) < 0.10 {
                    continue; // an empty cell, so the net is not too regular
                }
                let hx = (gx as f32 + 0.15 + 0.7 * unit(mix(id, 1))) * s;
                let hy = (gy as f32 + 0.15 + 0.7 * unit(mix(id, 2))) * s;
                let amp = 0.40 * s * (0.45 + 0.55 * unit(mix(id, 3)));
                let (w1, w2) = (0.10 + 0.28 * unit(mix(id, 4)) as f64, 0.10 + 0.28 * unit(mix(id, 5)) as f64);
                let (p1, p2) = (unit(mix(id, 6)) as f64 * std::f64::consts::TAU, unit(mix(id, 7)) as f64 * std::f64::consts::TAU);
                nodes.push(Node {
                    x: hx + amp * (w1 * t + p1).sin() as f32,
                    y: hy + amp * (w2 * t + p2).cos() as f32,
                    depth: 1.0,
                    hue: unit(mix(id, 8)),
                    size: unit(mix(id, 9)),
                    ring: unit(mix(id, 10)) < 0.16,
                });
            }
        }
        let r0 = (2.0 * (s / 120.0).sqrt()).clamp(1.3, 7.0);
        let mut pen = Pen { px: fb.as_bytes_mut(), w: w as i32, h: h as i32 };
        for i in 0..nodes.len() {
            for j in i + 1..nodes.len() {
                let (a, b) = (&nodes[i], &nodes[j]);
                let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                if d >= link {
                    continue;
                }
                let al = (1.0 - d / link).powf(0.65) * 1.0;
                if al < 0.04 {
                    continue;
                }
                let c = self.net_line_colour(a.hue, b.hue);
                pen.line(a.x, a.y, b.x, b.y, c, al);
            }
        }
        self.net_nodes(&mut pen, &nodes, r0);
    }

    /// A globe: nodes spread over a sphere that turns slowly, joined by thin lines, the near
    /// side clearer than the far one.
    fn paint_globe(&mut self, fb: &mut Framebuffer, level: u32, scale: u32, t: f64) {
        self.net_ground(fb);
        let (w, h) = (fb.width() as f32, fb.height() as f32);
        let sc = scale.clamp(25, 400) as f32 / 100.0;
        let r = 0.48 * h.min(w) * sc;
        let m = (70.0 + 190.0 * level.min(100) as f32 / 100.0).round() as usize;
        let sp = (4.0 * std::f32::consts::PI / m as f32).sqrt(); // spacing on the unit sphere
        let link = 1.75 * sp;
        let (yaw, tilt) = ((t * 0.12) as f32, 0.38 + 0.06 * (t * 0.07).sin() as f32);
        let (cy_, sy_) = (yaw.cos(), yaw.sin());
        let (ct, st) = (tilt.cos(), tilt.sin());
        let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
        let (cx, cy) = (w / 2.0, h / 2.0);
        let mut unit_pts: Vec<[f32; 3]> = Vec::with_capacity(m);
        let mut nodes: Vec<Node> = Vec::with_capacity(m);
        for i in 0..m {
            let yy = 1.0 - 2.0 * (i as f32 + 0.5) / m as f32;
            let rad = (1.0 - yy * yy).max(0.0).sqrt();
            let th = i as f32 * golden;
            let j = |k: u64| (unit(mix(i as u64, k)) - 0.5) * 0.5 * sp;
            let mut p = [rad * th.cos() + j(1), yy + j(2), rad * th.sin() + j(3)];
            let n = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            p = [p[0] / n, p[1] / n, p[2] / n];
            unit_pts.push(p);
            // spin about the vertical axis, then tilt
            let (x1, z1) = (p[0] * cy_ + p[2] * sy_, -p[0] * sy_ + p[2] * cy_);
            let (y2, z2) = (p[1] * ct - z1 * st, p[1] * st + z1 * ct);
            let persp = 1.0 / (1.0 - 0.16 * z2);
            nodes.push(Node {
                x: cx + r * x1 * persp,
                y: cy - r * y2 * persp,
                depth: (z2 + 1.0) / 2.0,
                hue: unit(mix(i as u64, 4)),
                size: unit(mix(i as u64, 5)),
                ring: unit(mix(i as u64, 6)) < 0.10,
            });
        }
        let mut pen = Pen { px: fb.as_bytes_mut(), w: w as i32, h: h as i32 };
        // the outline of the sphere
        let grey = self.net_line_colour(0.0, 0.0);
        pen.ring(cx, cy, r, grey, 0.45);
        for i in 0..m {
            for j in i + 1..m {
                let (a, b) = (&unit_pts[i], &unit_pts[j]);
                let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                if d >= link {
                    continue;
                }
                let (na, nb) = (&nodes[i], &nodes[j]);
                let depth = (na.depth + nb.depth) / 2.0;
                let al = (1.0 - d / link).powf(0.65) * (0.25 + 0.75 * depth * depth);
                if al < 0.03 {
                    continue;
                }
                let c = self.net_line_colour(na.hue, nb.hue);
                pen.line(na.x, na.y, nb.x, nb.y, c, al);
            }
        }
        let r0 = (1.8 * (r / 230.0).sqrt()).clamp(1.2, 7.0);
        self.net_nodes(&mut pen, &nodes, r0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trofeo_lcd::Resolution;

    fn fb(w: u32, h: u32) -> Framebuffer {
        Framebuffer::new(Resolution { width: w, height: h })
    }

    fn settings(style: Style, pal: &str) -> Settings {
        Settings { style: Some(style), palettes: vec![Palette::parse(pal).unwrap()], ..Settings::default() }
    }

    fn avg_luma(f: &Framebuffer) -> f32 {
        let b = f.as_bytes();
        b.iter().map(|&v| v as u64).sum::<u64>() as f32 / b.len() as f32
    }

    #[test]
    fn dimming_keeps_colours_colourful() {
        // Identity at full brightness, black at zero, same hue in between.
        for c in [(255, 179, 186), (0, 255, 120), (30, 60, 200), (128, 128, 128)] {
            assert_eq!(dim(c, 1.0), c);
            assert_eq!(dim(c, 0.0), (0, 0, 0));
        }
        let (r, g, b) = dim((255, 179, 186), 0.5);
        assert!(r > g && r >= b, "still a pink: {r},{g},{b}");
        // A dim pastel is more saturated than the plain product (which would be (127,89,93)).
        let sat = |c: Rgb| (c.0.max(c.1).max(c.2) - c.0.min(c.1).min(c.2)) as f32 / c.0.max(c.1).max(c.2) as f32;
        assert!(sat((r, g, b)) > sat((127, 89, 93)) + 0.05);
        // Brighter input never gets darker as k grows.
        let mut last = 0;
        for k in 0..=20 {
            let (r, g, b) = dim((200, 150, 255), k as f32 / 20.0);
            let l = r as u32 + g as u32 + b as u32;
            assert!(l >= last);
            last = l;
        }
        // Greys stay grey.
        let (r, g, b) = dim((128, 128, 128), 0.4);
        assert!(r == g && g == b);
    }

    #[test]
    fn names_and_aliases_parse() {
        for n in ["plasma", "AURORA", "lava", "waves", "silk", "onde"] {
            assert!(Style::parse(n).is_some(), "{n}");
        }
        assert!(Style::parse("fire").is_none());
        for n in ["pastel", "pastello", "fluo", "neon", "candy", "sunset", "ocean", "aurora", "fire", "forest", "ice", "mono", "rainbow"] {
            assert!(Palette::parse(n).is_some(), "{n}");
        }
        assert!(Palette::parse("beige").is_none());
        assert_eq!(Palette::presets().len(), 12);
        assert!(Palette::presets().iter().all(|p| p.stops.len() >= 3));
    }

    #[test]
    fn every_style_fills_every_pixel_and_moves() {
        for style in Style::ALL {
            let s = settings(style, "fluo");
            let mut e = Effect::new();
            let (mut a, mut b) = (fb(1920, 462), fb(1920, 462));
            a.clear(1, 2, 3);
            e.render_at(&mut a, &s, style, 60, 10.0);
            e.render_at(&mut b, &s, style, 60, 22.0);
            // Nothing is left at the clear colour...
            assert!(a.as_bytes().chunks_exact(3).all(|p| p != [1, 2, 3]), "{style:?} left pixels untouched");
            // ...it uses a good range of colours, and it changes with time.
            let distinct: std::collections::HashSet<_> = a.as_bytes().chunks_exact(3).step_by(97).map(|p| (p[0], p[1], p[2])).collect();
            assert!(distinct.len() > 40, "{style:?} looks flat ({} colours)", distinct.len());
            assert_ne!(a.as_bytes(), b.as_bytes(), "{style:?} does not move");
        }
    }

    #[test]
    fn same_time_same_picture() {
        let s = settings(Style::Plasma, "neon");
        let (mut a, mut b) = (fb(640, 200), fb(640, 200));
        Effect::new().render_at(&mut a, &s, Style::Plasma, 50, 5.0);
        Effect::new().render_at(&mut b, &s, Style::Plasma, 50, 5.0);
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn speed_zero_is_frozen_and_speed_scales_motion() {
        let mut s = settings(Style::Plasma, "ocean");
        s.speed = 0;
        let (mut a, mut b) = (fb(480, 160), fb(480, 160));
        let mut e = Effect::new();
        e.render_at(&mut a, &s, Style::Plasma, 50, 1.0);
        e.render_at(&mut b, &s, Style::Plasma, 50, 99.0);
        assert_eq!(a.as_bytes(), b.as_bytes(), "speed 0 must not move");
        assert!(!s.is_moving());
        s.speed = 30;
        assert!(s.is_moving());
        // Faster speed = same picture reached sooner.
        let (mut slow, mut fast) = (fb(480, 160), fb(480, 160));
        let mut s_slow = s.clone();
        s_slow.speed = 30;
        let mut s_fast = s.clone();
        s_fast.speed = 60;
        Effect::new().render_at(&mut slow, &s_slow, Style::Plasma, 50, 20.0);
        Effect::new().render_at(&mut fast, &s_fast, Style::Plasma, 50, 10.0);
        assert_eq!(slow.as_bytes(), fast.as_bytes());
    }

    #[test]
    fn brightness_controls_how_bright_it_is() {
        let s = settings(Style::Aurora, "pastel");
        let mut lum = Vec::new();
        for k in [0u32, 20, 40, 80, 100] {
            let mut f = fb(480, 160);
            Effect::new().render_at(&mut f, &s, Style::Aurora, k, 3.0);
            lum.push(avg_luma(&f));
        }
        assert_eq!(lum[0], 0.0, "brightness 0 is black");
        assert!(lum.windows(2).all(|w| w[0] < w[1]), "monotonic: {lum:?}");
        // The default keeps text readable: pastel at 40 stays well below mid-grey.
        assert!(lum[2] < 110.0, "{lum:?}");
    }

    #[test]
    fn pastel_is_softer_than_fluo_at_equal_brightness() {
        // Pastel colours are light (low saturation); fluo is saturated.
        let sat = |name: &str| {
            let s = settings(Style::Plasma, name);
            let mut f = fb(480, 160);
            Effect::new().render_at(&mut f, &s, Style::Plasma, 100, 7.0);
            let b = f.as_bytes();
            let n = (b.len() / 3) as f32;
            b.chunks_exact(3)
                .map(|p| {
                    let mx = p[0].max(p[1]).max(p[2]) as f32;
                    let mn = p[0].min(p[1]).min(p[2]) as f32;
                    (mx - mn) / mx.max(1.0)
                })
                .sum::<f32>()
                / n
        };
        assert!(sat("pastel") < sat("fluo") - 0.2, "{} vs {}", sat("pastel"), sat("fluo"));
    }

    #[test]
    fn scale_changes_feature_size() {
        // Bigger scale = smoother picture = smaller difference between neighbouring columns far apart.
        let var = |scale: u32| {
            let mut s = settings(Style::Plasma, "fluo");
            s.scale = scale;
            let mut f = fb(960, 240);
            Effect::new().render_at(&mut f, &s, Style::Plasma, 80, 4.0);
            let b = f.as_bytes();
            let mut acc = 0u64;
            for y in 0..240usize {
                for x in 0..959usize {
                    let (i, j) = ((y * 960 + x) * 3, (y * 960 + x + 1) * 3);
                    acc += (b[i + 1] as i32 - b[j + 1] as i32).unsigned_abs() as u64;
                }
            }
            acc
        };
        assert!(var(300) < var(100), "{} vs {}", var(300), var(100));
        assert!(var(100) < var(40), "{} vs {}", var(100), var(40));
    }

    #[test]
    fn works_at_any_size() {
        let s = settings(Style::Lava, "fire");
        for (w, h) in [(1, 1), (7, 3), (12, 12), (13, 25), (462, 1920), (1920, 462), (100, 462)] {
            let mut f = fb(w, h);
            Effect::new().render_at(&mut f, &s, Style::Lava, 50, 2.0);
            assert_eq!(f.as_bytes().len(), (w * h * 3) as usize);
        }
        // A buffer of a different size right after: the grid is rebuilt, not reused.
        let mut e = Effect::new();
        let mut a = fb(300, 100);
        e.render_at(&mut a, &s, Style::Lava, 50, 2.0);
        let mut b = fb(130, 400);
        e.render_at(&mut b, &s, Style::Lava, 50, 2.0);
    }

    #[test]
    fn rotating_palettes_cross_fade() {
        let mut s = settings(Style::Plasma, "pastel");
        s.palettes = vec![Palette::custom(vec![(255, 0, 0), (255, 0, 0)]), Palette::custom(vec![(0, 0, 255), (0, 0, 255)])];
        s.palette_interval = 10;
        let at = |secs: f64| {
            let mut f = fb(96, 48);
            Effect::new().render_at(&mut f, &s, Style::Plasma, 100, secs);
            let p = &f.as_bytes()[0..3];
            (p[0], p[2])
        };
        // Second 12: well after the fade into palette 1 (idx 1 -> blue).
        let (r, b) = at(18.0);
        assert!(r < 5 && b > 250, "blue expected: {r},{b}");
        // Second 20 = start of palette 0's turn: fade from blue (previous) into red begins.
        let (r0, b0) = at(20.0);
        assert!(b0 > 250 && r0 < 5, "fade starts from the previous palette: {r0},{b0}");
        let (rm, bm) = at(22.0);
        assert!(rm > 60 && bm > 60, "halfway through the fade both show: {rm},{bm}");
        let (r1, b1) = at(26.0);
        assert!(r1 > 250 && b1 < 5, "red after the fade: {r1},{b1}");
    }

    #[test]
    fn custom_palette_colors_are_used() {
        let mut s = settings(Style::Waves, "pastel");
        s.palettes = vec![Palette::custom(vec![(0, 200, 0), (0, 100, 0)])];
        let mut f = fb(240, 80);
        Effect::new().render_at(&mut f, &s, Style::Waves, 100, 1.0);
        assert!(f.as_bytes().chunks_exact(3).all(|p| p[0] == 0 && p[2] == 0 && p[1] >= 99));
    }

    #[test]
    fn lut_is_rebuilt_only_when_something_changed() {
        let s = settings(Style::Plasma, "neon");
        let mut e = Effect::new();
        let mut f = fb(120, 40);
        e.render_at(&mut f, &s, Style::Plasma, 40, 1.0);
        let key = e.lut_key;
        e.render_at(&mut f, &s, Style::Plasma, 40, 2.0);
        assert!(e.lut_key == key);
        e.render_at(&mut f, &s, Style::Plasma, 41, 2.0);
        assert!(e.lut_key != key);
    }

    #[test]
    fn no_wrap_seam_inside_one_picture() {
        // Neighbouring pixels never jump between the two ends of the palette
        // unless the palette itself does: with a smooth palette the biggest
        // horizontal step stays small.
        for style in Style::ALL {
            if matches!(style, Style::Matrix | Style::Rain | Style::Aquarium | Style::Network | Style::Globe) {
                continue; // glyphs, streaks and the water surface have hard edges by design
            }
            let s = settings(style, "ocean");
            let mut f = fb(1920, 462);
            Effect::new().render_at(&mut f, &s, style, 100, 13.0);
            let b = f.as_bytes();
            let mut worst = 0i32;
            for y in (0..462usize).step_by(7) {
                for x in 0..1919usize {
                    let (i, j) = ((y * 1920 + x) * 3, (y * 1920 + x + 1) * 3);
                    for k in 0..3 {
                        worst = worst.max((b[i + k] as i32 - b[j + k] as i32).abs());
                    }
                }
            }
            assert!(worst <= 12, "{style:?}: a seam of {worst} levels between two pixels");
        }
    }

    #[test]
    fn rain_styles_parse_and_have_one_colour_palettes() {
        assert_eq!(Style::parse("matrix"), Some(Style::Matrix));
        assert_eq!(Style::parse("CODICE"), Some(Style::Matrix));
        assert_eq!(Style::parse("pioggia"), Some(Style::Rain));
        assert_eq!(Style::ALL.len(), 9);
        for n in ["matrix", "green", "amber", "ambra", "violet", "crimson", "snow", "bianco", "neon"] {
            assert!(Palette::parse(n).is_some(), "{n}");
        }
        // the rotating "all" list is unchanged: only the multi-colour presets
        assert_eq!(Palette::presets().len(), MULTI_COLOUR_PRESETS);
    }

    #[test]
    fn rain_glyphs_all_exist_in_the_pixel_font() {
        for c in RAIN_CHARS.chars() {
            assert!(trofeo_lcd::font::glyph(c).iter().any(|&r| r != 0), "{c:?} has no pixels");
        }
    }

    #[test]
    fn rain_renders_on_every_panel_and_palette_and_moves() {
        for (w, h) in [(1920u32, 462u32), (1920, 400), (462, 1920), (640, 480)] {
            for style in [Style::Matrix, Style::Rain] {
                for pal in ["matrix", "amber", "violet", "crimson", "snow", "neon"] {
                    let s = settings(style, pal);
                    let mut a = fb(w, h);
                    Effect::new().render_at(&mut a, &s, style, 100, 5.0);
                    let mut b = fb(w, h);
                    Effect::new().render_at(&mut b, &s, style, 100, 9.0);
                    assert_ne!(a.as_bytes(), b.as_bytes(), "{style:?}/{pal} {w}x{h} does not move");
                    let mut c = fb(w, h);
                    Effect::new().render_at(&mut c, &s, style, 100, 5.0);
                    assert_eq!(a.as_bytes(), c.as_bytes(), "{style:?}/{pal} is not a function of time");
                }
            }
        }
    }

    #[test]
    fn rain_is_minimal_dark_and_in_the_chosen_colour() {
        let s = settings(Style::Matrix, "matrix");
        let mut f = fb(1920, 462);
        Effect::new().render_at(&mut f, &s, Style::Matrix, 100, 12.0);
        let b = f.as_bytes();
        let n = (b.len() / 3) as u64;
        let (mut r, mut g, mut bl, mut lit) = (0u64, 0u64, 0u64, 0u64);
        for px in b.chunks_exact(3) {
            r += px[0] as u64;
            g += px[1] as u64;
            bl += px[2] as u64;
            if px[1] > 60 {
                lit += 1;
            }
        }
        assert!(g > r * 2 && g > bl * 2, "the matrix palette is green ({r},{g},{bl})");
        assert!(lit * 100 < n * 25, "minimal: under a quarter of the pixels are lit ({lit}/{n})");
        assert!(lit > 0, "something is drawn");
    }

    /// With TROFEO_DUMP_DIR set, writes the rain styles in every palette as PPM files.
    #[test]
    fn dump_rain_previews() {
        let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") else { return };
        for style in [Style::Matrix, Style::Rain] {
            for pal in ["matrix", "amber", "violet", "crimson", "snow", "neon"] {
                let s = settings(style, pal);
                let mut f = fb(1920, 462);
                Effect::new().render_at(&mut f, &s, style, 100, 12.0);
                std::fs::write(
                    format!("{dir}/rain_{style:?}_{pal}.ppm"),
                    [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
    }

    #[test]
    fn rain_frame_cost_is_small() {
        for style in [Style::Matrix, Style::Rain] {
            let s = settings(style, "matrix");
            let mut f = fb(1920, 462);
            let mut e = Effect::new();
            e.render_at(&mut f, &s, style, 100, 1.0);
            let t = std::time::Instant::now();
            for k in 0..20 {
                e.render_at(&mut f, &s, style, 100, 2.0 + k as f64 * 0.1);
            }
            let ms = t.elapsed().as_secs_f64() * 50.0;
            eprintln!("{style:?}: {ms:.1} ms per frame");
            assert!(ms < 60.0, "{style:?} takes {ms} ms per frame");
        }
    }

    /// With TROFEO_DUMP_DIR set, writes aquarium previews as PPM files.
    #[test]
    fn dump_aquarium_previews() {
        let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") else { return };
        let cases = [("lagoon", 1920u32, 462u32, 50u32, 100u32, 20.0f64), ("deep", 1920, 462, 50, 100, 33.0), ("tropical", 1920, 462, 70, 100, 8.0),
            ("lagoon", 462, 1920, 50, 100, 20.0), ("neon", 1920, 462, 50, 100, 20.0), ("lagoon", 1920, 462, 50, 40, 20.0), ("lagoon", 1920, 462, 50, 65, 26.0)];
        for (n, (pal, w, h, level, bright, secs)) in cases.into_iter().enumerate() {
            let mut st = settings(Style::Aquarium, pal);
            st.level = level;
            let mut f = fb(w, h);
            Effect::new().render_at(&mut f, &st, Style::Aquarium, bright, secs);
            std::fs::write(
                format!("{dir}/aq_{n}_{pal}_{w}x{h}.ppm"),
                [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
            ).unwrap();
        }
    }

    #[test]
    fn aquarium_parses_and_has_water_palettes() {
        for n in ["aquarium", "ACQUARIO", "water", "tank"] {
            assert_eq!(Style::parse(n), Some(Style::Aquarium), "{n}");
        }
        for n in ["lagoon", "laguna", "deep", "profondo", "tropical", "tropicale"] {
            assert!(Palette::parse(n).is_some(), "{n}");
        }
        assert_eq!(Palette::presets().len(), MULTI_COLOUR_PRESETS, "the water presets are not part of `all`");
        assert_eq!(Settings::default().level, 50);
    }

    #[test]
    fn water_depth_goes_from_light_to_dark_whatever_the_palette() {
        let luma = |c: Rgb| c.0 as u32 * 299 + c.1 as u32 * 587 + c.2 as u32 * 114;
        for n in ["lagoon", "deep", "tropical", "neon", "ocean", "pastel", "matrix", "rainbow"] {
            let st = Palette::parse(n).unwrap().depth_stops();
            assert!(st.len() >= 2);
            let (top, bottom) = (along(&st, 0.0), along(&st, 1.0));
            assert!(luma(top) >= luma(bottom), "{n}");
        }
        assert_eq!(Palette::custom(vec![]).depth_stops().len(), 2);
    }

    #[test]
    fn aquarium_renders_at_every_level_size_and_palette_and_moves() {
        for (w, h) in [(1920u32, 462u32), (462, 1920), (640, 480), (64, 64)] {
            for level in [0u32, 1, 50, 99, 100] {
                for pal in ["lagoon", "deep", "tropical", "neon"] {
                    let mut st = settings(Style::Aquarium, pal);
                    st.level = level;
                    let mut a = fb(w, h);
                    Effect::new().render_at(&mut a, &st, Style::Aquarium, 65, 5.0);
                    let mut b = fb(w, h);
                    Effect::new().render_at(&mut b, &st, Style::Aquarium, 65, 9.0);
                    let mut c = fb(w, h);
                    Effect::new().render_at(&mut c, &st, Style::Aquarium, 65, 5.0);
                    assert_eq!(a.as_bytes(), c.as_bytes(), "{pal} {w}x{h} level {level}: not a function of time");
                    if level >= 1 {
                        assert_ne!(a.as_bytes(), b.as_bytes(), "{pal} {w}x{h} level {level}: does not move");
                    }
                }
            }
        }
    }

    #[test]
    fn aquarium_level_sets_the_water_line_and_the_surface_stands_out() {
        // The water is the brighter part: a column's brightness jumps at the surface.
        let col_luma = |f: &Framebuffer, x: usize, y: usize| {
            let o = (y * f.width() as usize + x) * 3;
            let b = f.as_bytes();
            b[o] as u32 + b[o + 1] as u32 + b[o + 2] as u32
        };
        let mut last_water = usize::MAX;
        for level in [20u32, 50, 80] {
            let mut st = settings(Style::Aquarium, "lagoon");
            st.level = level;
            let mut f = fb(1920, 462);
            Effect::new().render_at(&mut f, &st, Style::Aquarium, 65, 3.0);
            let x = 700;
            let surface = (0..462).find(|&y| col_luma(&f, x, y) > col_luma(&f, x, 0) + 150).expect("water found");
            // the water starts near the line (1 - level/100) of the height, within the wave height
            let expect = 462.0 * (1.0 - level as f32 / 100.0);
            assert!((surface as f32 - expect).abs() < 30.0, "level {level}: surface at {surface}, expected {expect}");
            assert!(surface < last_water, "more water, higher surface");
            last_water = surface;
        }
        // the air stays dark and the water is clearly lighter than it
        let mut f = fb(1920, 462);
        Effect::new().render_at(&mut f, &settings(Style::Aquarium, "lagoon"), Style::Aquarium, 65, 3.0);
        assert!(col_luma(&f, 100, 10) < 100 && col_luma(&f, 100, 300) > 200);
    }

    #[test]
    fn aquarium_frame_cost_is_small() {
        let st = settings(Style::Aquarium, "lagoon");
        let mut f = fb(1920, 462);
        let mut e = Effect::new();
        e.render_at(&mut f, &st, Style::Aquarium, 65, 1.0);
        let t = std::time::Instant::now();
        for k in 0..10 {
            e.render_at(&mut f, &st, Style::Aquarium, 65, 2.0 + k as f64 * 0.1);
        }
        let ms = t.elapsed().as_secs_f64() * 100.0;
        eprintln!("Aquarium: {ms:.1} ms per frame");
        assert!(ms < 80.0, "takes {ms} ms per frame");
    }

    /// With TROFEO_DUMP_DIR set, writes the light variants as PPM files.
    #[test]
    fn dump_light_previews() {
        let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") else { return };
        for (style, pals) in [(Style::Matrix, ["matrix", "violet", "crimson"]), (Style::Rain, ["matrix", "amber", "deep"]),
                              (Style::Aquarium, ["lagoon", "deep", "tropical"])] {
            for pal in pals {
                let mut st = settings(style, pal);
                st.light = true;
                let mut f = fb(1920, 462);
                Effect::new().render_at(&mut f, &st, style, 65, 12.0);
                std::fs::write(
                    format!("{dir}/light_{style:?}_{pal}.ppm"),
                    [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
    }

    #[test]
    fn network_and_globe_render_everywhere_move_and_obey_the_theme() {
        for style in [Style::Network, Style::Globe] {
            assert_eq!(Style::parse(if style == Style::Network { "plexus" } else { "mondo" }), Some(style));
            for (w, h) in [(1920u32, 462u32), (1920, 400), (462, 1920), (64, 64), (3, 2)] {
                for (level, scale) in [(0u32, 25u32), (50, 100), (100, 400)] {
                    let mut st = settings(style, "ice");
                    st.level = level;
                    st.scale = scale;
                    let mut a = fb(w, h);
                    Effect::new().render_at(&mut a, &st, style, 70, 5.0);
                    let mut b = fb(w, h);
                    Effect::new().render_at(&mut b, &st, style, 70, 9.0);
                    if w >= 64 && scale <= 100 {
                        assert_ne!(a.as_bytes(), b.as_bytes(), "{style:?} {w}x{h} must move");
                    }
                    // frozen: the same picture at any time
                    st.speed = 0;
                    let (mut c, mut d) = (fb(w, h), fb(w, h));
                    Effect::new().render_at(&mut c, &st, style, 70, 5.0);
                    Effect::new().render_at(&mut d, &st, style, 70, 9.0);
                    assert_eq!(c.as_bytes(), d.as_bytes(), "{style:?} speed 0");
                }
            }
            // dark = a dark page, light = a white page; both with lines and nodes on it
            let (mut dark, mut light) = (fb(1920, 462), fb(1920, 462));
            let mut st = settings(style, "crimson");
            Effect::new().render_at(&mut dark, &st, style, 70, 3.0);
            st.light = true;
            Effect::new().render_at(&mut light, &st, style, 70, 3.0);
            assert!(avg_luma(&dark) < 40.0, "{style:?} dark: {}", avg_luma(&dark));
            assert!(avg_luma(&light) > 215.0, "{style:?} light: {}", avg_luma(&light));
            let (bd, bl) = (dark.as_bytes(), light.as_bytes());
            assert!(bd.chunks_exact(3).any(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 200), "{style:?} dark has bright pixels");
            assert!(bl.chunks_exact(3).any(|p| (p[0] as u32 + p[1] as u32 + p[2] as u32) < 520), "{style:?} light has dark pixels");
            // a different palette gives a different picture
            let (mut x, mut y) = (fb(1920, 462), fb(1920, 462));
            Effect::new().render_at(&mut x, &settings(style, "crimson"), style, 70, 3.0);
            Effect::new().render_at(&mut y, &settings(style, "ocean"), style, 70, 3.0);
            assert_ne!(x.as_bytes(), y.as_bytes());
        }
    }

    #[test]
    fn network_and_globe_frame_cost_is_small() {
        for style in [Style::Network, Style::Globe] {
            let mut st = settings(style, "ice");
            st.level = 100;
            let mut e = Effect::new();
            let mut f = fb(1920, 462);
            e.render_at(&mut f, &st, style, 70, 1.0);
            let t0 = std::time::Instant::now();
            for k in 0..20 {
                e.render_at(&mut f, &st, style, 70, 2.0 + k as f64 * 0.1);
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0 / 20.0;
            eprintln!("{style:?}: {ms:.1} ms per frame");
            assert!(ms < 40.0, "{style:?} takes {ms:.1} ms");
        }
    }

    /// With TROFEO_DUMP_DIR set, writes network / globe previews as PPM files.
    #[test]
    fn dump_network_previews() {
        let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") else { return };
        for style in [Style::Network, Style::Globe] {
            for (light, pal, bright) in [(false, "ice", 65), (true, "ice", 65), (false, "crimson", 65), (true, "crimson", 65), (false, "neon", 65), (true, "ocean", 65)] {
                let mut st = settings(style, pal);
                st.light = light;
                let mut f = fb(1920, 462);
                Effect::new().render_at(&mut f, &st, style, bright, 12.0);
                std::fs::write(
                    format!("{dir}/net_{style:?}_{pal}_{}.ppm", if light { "light" } else { "dark" }),
                    [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
    }

    #[test]
    fn light_variants_are_white_backgrounds_and_still_move() {
        for style in [Style::Matrix, Style::Rain, Style::Aquarium] {
            for pal in ["matrix", "amber", "lagoon", "deep", "neon"] {
                for (w, h) in [(1920u32, 462u32), (462, 1920), (64, 64)] {
                    let mut st = settings(style, pal);
                    st.light = true;
                    let mut a = fb(w, h);
                    Effect::new().render_at(&mut a, &st, style, 65, 5.0);
                    let mut b = fb(w, h);
                    Effect::new().render_at(&mut b, &st, style, 65, 9.0);
                    assert_ne!(a.as_bytes(), b.as_bytes(), "{style:?}/{pal} {w}x{h} light does not move");
                    // the top-left corner (rain background / air) is nearly white
                    let px = &a.as_bytes()[0..3];
                    assert!(px.iter().all(|&v| v > 225), "{style:?}/{pal}: background {px:?} is not light");
                    let mut d = fb(w, h);
                    let mut dark = settings(style, pal);
                    dark.light = false;
                    Effect::new().render_at(&mut d, &dark, style, 65, 5.0);
                    assert!(avg_luma(&a) > avg_luma(&d) + 40.0, "{style:?}/{pal}: light is not lighter");
                }
            }
        }
    }

    #[test]
    fn light_aquarium_water_is_darker_than_the_air_and_the_surface_shows() {
        let mut st = settings(Style::Aquarium, "lagoon");
        st.light = true;
        let mut f = fb(1920, 462);
        Effect::new().render_at(&mut f, &st, Style::Aquarium, 65, 3.0);
        let sum = |y: usize| {
            let o = (y * 1920 + 700) * 3;
            f.as_bytes()[o..o + 3].iter().map(|&v| v as u32).sum::<u32>()
        };
        assert!(sum(5) > sum(400) + 120, "air {} vs deep water {}", sum(5), sum(400));
        // somewhere between the two there is a distinct edge (a pixel darker than both sides)
        let edge = (0..462).map(sum).min().unwrap();
        assert!(edge < sum(5) - 150);
    }
}
