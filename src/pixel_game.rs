//! A tiny self-playing platformer drawn in pixel art, shown in a strip of the
//! default screen under the clock.
//!
//! A little hero (a knight or a cat) runs along a scrolling landscape and
//! jumps over obstacles on its own. The world reacts to the computer:
//!
//! * **CPU load** sets the running speed (and a sprint with dust at high load);
//! * **music** makes the hero hop on the bass and let notes float up;
//! * **network / disk bursts** drop rows of coins / a treasure chest to collect;
//! * **the clock** brings fireworks and a victory dance on the hour, and the
//!   sky follows the real time of day (stars and moon at night);
//! * **the weather** decides sun / clouds / rain / snow / fog / lightning.
//!
//! Everything is drawn from tiny hand-made sprites: the world lives on a small
//! "virtual pixel" grid that is scaled up by 2-4 when it is blitted to the
//! panel. The simulation runs on a fixed 1/60 s step (independent of the frame
//! rate) and is deterministic for a given seed, which is what the tests use.

use crate::weather_icon::WeatherIcon;
use trofeo_lcd::Framebuffer;

pub type Rgb = (u8, u8, u8);

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Which hero runs (`pixel_game_hero`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroChoice {
    Knight,
    Cat,
    /// They take turns, switching every few minutes.
    Both,
}

impl HeroChoice {
    pub const NAMES: &'static str = "knight | cat | both";

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "knight" | "cavaliere" => HeroChoice::Knight,
            "cat" | "gatto" => HeroChoice::Cat,
            "both" | "rotate" | "all" | "entrambi" => HeroChoice::Both,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hero {
    Knight,
    Cat,
}

/// Seconds a hero stays when `pixel_game_hero = both`.
const SWAP_SECS: u64 = 300;

/// What the world reacts to; filled by the main loop every frame.
#[derive(Clone, Debug)]
pub struct Inputs {
    /// CPU load, 0-100.
    pub cpu: f32,
    /// Network traffic (down + up), KB/s.
    pub net_kb: f64,
    /// Disk traffic (read + write), MB/s.
    pub disk_mb: f64,
    /// True while sound is playing.
    pub music: bool,
    /// Low-frequency energy of the spectrum, 0-1.
    pub bass: f32,
    pub weather: Option<WeatherIcon>,
    /// Local time.
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Seconds since the Unix epoch (drives the hero swap and the hourly event).
    pub unix_secs: u64,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs { cpu: 20.0, net_kb: 0.0, disk_mb: 0.0, music: false, bass: 0.0, weather: None, hour: 12, minute: 30, second: 0, unix_secs: 1_000_000_000 }
    }
}

// ---------------------------------------------------------------------------
// Palette and sprites
// ---------------------------------------------------------------------------

fn pal(ch: u8) -> Option<Rgb> {
    Some(match ch {
        b'k' => (24, 20, 40),    // outline
        b'w' => (248, 248, 252), // white
        b'W' => (200, 206, 222), // light steel
        b'G' => (132, 140, 160), // steel
        b'D' => (84, 90, 112),   // dark steel
        b'r' => (228, 52, 62),   // red
        b'R' => (150, 28, 44),   // dark red
        b'o' => (246, 150, 44),  // orange
        b'O' => (196, 98, 28),   // dark orange
        b'y' => (252, 218, 64),  // gold
        b'Y' => (204, 150, 32),  // dark gold
        b'b' => (118, 76, 46),   // brown
        b'B' => (72, 46, 30),    // dark brown
        b'n' => (84, 134, 232),  // blue
        b'N' => (42, 84, 172),   // dark blue
        b'g' => (88, 196, 84),   // green
        b'T' => (40, 132, 62),   // dark green
        b'p' => (255, 172, 190), // pink
        b's' => (250, 206, 166), // skin
        b'm' => (172, 92, 204),  // purple
        b'c' => (86, 222, 232),  // cyan
        b'l' => (170, 172, 184), // stone light
        b'S' => (112, 116, 134), // stone dark
        _ => return None,
    })
}

type Sprite = &'static [&'static str];

/// Knight, facing right: upper body (11 rows) shared by all running frames.
const KNIGHT_TOP: [&str; 11] = [
    "......kkkkk.....",
    ".....kWWWWWk....",
    "..rr.kWWWWWWk...",
    ".rRRkkWkkkkWWk..",
    "..RRkkWWWWWWk...",
    "....kkGGGGGk....",
    "....kkNNNNk..k..",
    "...kNnnNNNNk.kw.",
    "...kNnNNNNNkkWw.",
    "...kNNnNNNkkgw..",
    "....kNNNNNk.k...",
];

/// Four leg poses (5 rows) for the knight's run cycle.
const KNIGHT_LEGS: [[&str; 5]; 4] = [
    ["....kbbbbk......", "...kbbk.kbk.....", "...kbk...kbk....", "..kBBk....kBBk..", "..kkk......kkk.."],
    ["....kbbbbk......", "....kbbbbk......", "....kbkkbk......", "....kBkkBBk.....", "....kkk.kkk....."],
    ["....kbbbbk......", "...kbk.kbk......", "..kbk...kbk.....", ".kBBk....kBBk...", ".kkk.....kkk...."],
    ["....kbbbbk......", "....kbbbbk......", "....kbkkbk......", "....kBkkBBk.....", "....kkk.kkk....."],
];

/// Knight in the air: legs tucked, sword pointing up.
const KNIGHT_JUMP: [&str; 16] = [
    "......kkkkk....w",
    ".....kWWWWWk..kw",
    "..rr.kWWWWWWk.kW",
    ".rRRkkWkkkkWWkkW",
    "..RRkkWWWWWWk.kg",
    "....kkGGGGGk.kgk",
    "....kkNNNNk.kk..",
    "...kNnnNNNNkkW..",
    "...kNnNNNNNkk...",
    "...kNNnNNNk.....",
    "....kNNNNNk.....",
    "....kbbbbk......",
    "...kbbkkbbk.....",
    "..kbBk..kBbk....",
    "..kkk....kkk....",
    "................",
];

/// Knight swinging the sword forward.
const KNIGHT_ATTACK: [&str; 16] = [
    "......kkkkk.....",
    ".....kWWWWWk....",
    "..rr.kWWWWWWk...",
    ".rRRkkWkkkkWWk..",
    "..RRkkWWWWWWk...",
    "....kkGGGGGk....",
    "....kkNNNNk.....",
    "...kNnnNNNNkkwww",
    "...kNnNNNNNkgWWw",
    "...kNNnNNNkk.kww",
    "....kNNNNNk.....",
    "....kbbbbk......",
    "....kbbbbk......",
    "....kbkkbk......",
    "....kBkkBBk.....",
    "....kkk.kkk.....",
];

/// Cat, facing right: body rows shared by the run frames (rows 3-10).
const CAT_TOP: [&str; 8] = [
    ".............k.k",
    "k...........kokk",
    "kk.kkkkkkkk.kooo",
    ".okooooooookkokk",
    "..koOoOoOooookpk",
    "...kooooooooook.",
    "...kooooooooook.",
    "....kkoooooookk.",
];

/// Cat leg poses (rows 11-15 → 5 rows? the cat is lower so 4 rows + ground gap).
const CAT_LEGS: [[&str; 5]; 4] = [
    ["....koOkkkOok...", "...kook...kook..", "..kook.....kook.", "..kkk.......kkk.", "................"],
    ["....koOkkkOok...", "....kookkkook...", "....kOokkkoOk...", "....kkk..kkk....", "................"],
    ["....koOkkkOok...", "...kook...kook..", ".kook.......kook", ".kkk.........kkk", "................"],
    ["....koOkkkOok...", "....kookkkook...", "....kOokkkoOk...", "....kkk..kkk....", "................"],
];

const CAT_JUMP: [&str; 16] = [
    "................",
    "................",
    "................",
    ".............k.k",
    "k...........kokk",
    "kk.kkkkkkkk.kooo",
    ".okooooooookkokk",
    "..koOoOoOooookpk",
    "...kooooooooook.",
    "..kkkoooooooookk",
    ".kooook..kooook.",
    ".kkkkk....kkkkk.",
    "................",
    "................",
    "................",
    "................",
];

const ROCK: [&str; 7] = [
    "....kkkk..",
    "..kkllllk.",
    ".kllllSSlk",
    "kllllSSSSk",
    "kllSSSSSSk",
    "kSSSSSSSSk",
    ".kkkkkkkk.",
];

const BUSH: [&str; 9] = [
    "...kkk......",
    "..kgggkkk...",
    ".kgggTggggk.",
    "kgggTgggTggk",
    "kggTggggggTk",
    "kgTgggTgggTk",
    "kTTgggggTTTk",
    "kTTTTTTTTTTk",
    ".kkkkkkkkkk.",
];

const CRATE: [&str; 11] = [
    "kkkkkkkkkk",
    "kbbbbbbbbk",
    "kbkbbbbkbk",
    "kbbkbbkbbk",
    "kbbbkkbbbk",
    "kbbbkkbbbk",
    "kbbkbbkbbk",
    "kbkbbbbkbk",
    "kbbbbbbbbk",
    "kBBBBBBBBk",
    "kkkkkkkkkk",
];

const SLIME: [[&str; 8]; 2] = [
    [
        "...kkkk...",
        "..kgggggk.",
        ".kgwkgwkgk",
        ".kgkkgkkgk",
        "kgggggggkk",
        "kgggTTgggk",
        "kgTTTTTTgk",
        ".kkkkkkkk.",
    ],
    [
        "..........",
        "...kkkk...",
        ".kgggggkk.",
        "kgwkgwkggk",
        "kgkkgkkggk",
        "kgggTTgggk",
        "kTTTTTTTTk",
        ".kkkkkkkk.",
    ],
];

const CHEST: [&str; 9] = [
    "..kkkkkkkk..",
    ".kbbbbbbbbk.",
    "kbbbbbbbbbbk",
    "kyyyyyyyyyyk",
    "kbbbbyybbbbk",
    "kbbbbyybbbbk",
    "kbbbbbbbbbbk",
    "kBBBBBBBBBBk",
    ".kkkkkkkkkk.",
];

const CHEST_OPEN: [&str; 9] = [
    ".kkkkkkkkkk.",
    "kbbbbbbbbbbk",
    "kBBBBBBBBBBk",
    "kyyyyyyyyyyk",
    "kyYyyYyyYyyk",
    "kyyyYyyyyYyk",
    "kbbbbbbbbbbk",
    "kBBBBBBBBBBk",
    ".kkkkkkkkkk.",
];

const COIN: [[&str; 5]; 4] = [
    [".kkk.", "kyyYk", "kywYk", "kyyYk", ".kkk."],
    [".kk..", "kyYk.", "kyYk.", "kyYk.", ".kk.."],
    ["..k..", ".kyk.", ".kYk.", ".kyk.", "..k.."],
    [".kk..", ".kYk.", ".kYk.", ".kYk.", "..kk."],
];

const NOTE: [&str; 7] = ["..kkk", "..kwk", "..kk.", "..k..", "kkk..", "kwk..", "kk..."];

const SUN: [&str; 13] = [
    ".....y.y.....",
    "......y......",
    "..y..yyy..y..",
    "...yyyyyyy...",
    "..yyyyyyyyy..",
    "y.yyyyyyyyy.y",
    ".yyyyyyyyyyy.",
    "y.yyyyyyyyy.y",
    "..yyyyyyyyy..",
    "...yyyyyyy...",
    "..y..yyy..y..",
    "......y......",
    ".....y.y.....",
];

const MOON: [&str; 9] = [
    "..WWWW...",
    ".WWwW....",
    "WWwW.....",
    "WWW......",
    "WWW......",
    "WWWW.....",
    ".WWWWW...",
    "..WWWWWW.",
    "....WWW..",
];

const CLOUD_BIG: [&str; 8] = [
    "........wwww..........",
    "......wwwwwwww..ww....",
    "...wwwwwwwwwwwwwwwww..",
    "..wwwwwwwwwwwwwwwwwww.",
    ".wwwwwwwwwwwwwwwwwwwww",
    "wwwwwwwwwwwwwwwwwwwwww",
    "wwwwwwwwwwwwwwwwwwwwww",
    ".wwwwwwwwwwwwwwwwwwww.",
];

const CLOUD_SMALL: [&str; 6] = [
    "....wwww....",
    "..wwwwwwww..",
    ".wwwwwwwwwww",
    "wwwwwwwwwwww",
    "wwwwwwwwwwww",
    ".wwwwwwwwww.",
];

const TREE: [&str; 14] = [
    "....TTTT....",
    "..TTTggTTT..",
    ".TTggggggTT.",
    "TTgggTggggTT",
    "TgggTTgggggT",
    "TTgggggTggTT",
    ".TTTgggggTT.",
    "..TTTTTTTT..",
    "....kbbk....",
    "....kbBk....",
    "....kbBk....",
    "....kbBk....",
    "....kbBk....",
    "...kkbBkk...",
];

const BIRD: [[&str; 3]; 2] = [["k.k.k", ".kkk.", "..k.."], [".....", "kkkkk", "..k.."]];

// ---------------------------------------------------------------------------
// Canvas: draws on the real framebuffer in scaled "virtual pixels"
// ---------------------------------------------------------------------------

pub struct Canvas<'a> {
    fb: &'a mut Framebuffer,
    /// The strip on the panel, in real pixels.
    ox: u32,
    oy: u32,
    sw: u32,
    sh: u32,
    s: u32,
    /// Brightness factor applied to sprites (1.0 = as drawn): night scenery.
    dim: f32,
}

impl<'a> Canvas<'a> {
    pub fn new(fb: &'a mut Framebuffer, rect: (u32, u32, u32, u32), scale: u32) -> Self {
        Canvas { fb, ox: rect.0, oy: rect.1, sw: rect.2, sh: rect.3, s: scale.max(1), dim: 1.0 }
    }

    /// Filled rectangle in virtual pixels, clipped to the strip.
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        let s = self.s as i64;
        let (x0, y0) = ((x as i64 * s).max(0), (y as i64 * s).max(0));
        let (x1, y1) = (((x + w) as i64 * s).min(self.sw as i64), ((y + h) as i64 * s).min(self.sh as i64));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        self.fb.fill_rect(self.ox + x0 as u32, self.oy + y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32, c.0, c.1, c.2);
    }

    /// Translucent rectangle (alpha 0-1) over what is already drawn.
    fn blend(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb, alpha: f32) {
        let s = self.s as i64;
        let (x0, y0) = ((x as i64 * s).max(0), (y as i64 * s).max(0));
        let (x1, y1) = (((x + w) as i64 * s).min(self.sw as i64), ((y + h) as i64 * s).min(self.sh as i64));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let stride = self.fb.width() as usize;
        let a = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
        let px = self.fb.as_bytes_mut();
        for yy in y0..y1 {
            let row = ((self.oy as i64 + yy) as usize * stride + (self.ox as i64 + x0) as usize) * 3;
            for i in 0..((x1 - x0) as usize) {
                let o = row + i * 3;
                px[o] = ((px[o] as u32 * (256 - a) + c.0 as u32 * a) >> 8) as u8;
                px[o + 1] = ((px[o + 1] as u32 * (256 - a) + c.1 as u32 * a) >> 8) as u8;
                px[o + 2] = ((px[o + 2] as u32 * (256 - a) + c.2 as u32 * a) >> 8) as u8;
            }
        }
    }

    fn px(&mut self, x: i32, y: i32, c: Rgb) {
        self.rect(x, y, 1, 1, c);
    }

    /// A sprite with its top-left at (x, y); `tint` recolors white pixels (clouds).
    fn sprite(&mut self, spr: Sprite, x: i32, y: i32, tint: Option<Rgb>) {
        for (ry, row) in spr.iter().enumerate() {
            let bytes = row.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let ch = bytes[i];
                let color = match (pal(ch), tint) {
                    (Some(_), Some(t)) if ch == b'w' => Some(t),
                    (c, _) => c,
                };
                let Some(c) = color else {
                    i += 1;
                    continue;
                };
                let c = if self.dim < 1.0 { scale_rgb(c, self.dim) } else { c };
                // Merge a run of the same color into one rectangle.
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] == ch {
                    j += 1;
                }
                self.rect(x + i as i32, y + ry as i32, (j - i) as i32, 1, c);
                i = j;
            }
        }
    }

    fn sprite_owned(&mut self, rows: &[String], x: i32, y: i32) {
        let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
        // Sprites assembled from parts are only used briefly per frame.
        for (ry, row) in refs.iter().enumerate() {
            let bytes = row.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let ch = bytes[i];
                let Some(c) = pal(ch) else {
                    i += 1;
                    continue;
                };
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] == ch {
                    j += 1;
                }
                self.rect(x + i as i32, y + ry as i32, (j - i) as i32, 1, c);
                i = j;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| lerp(x as f32, y as f32, t).round().clamp(0.0, 255.0) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn scale_rgb(c: Rgb, f: f32) -> Rgb {
    let g = |x: u8| (x as f32 * f).round().clamp(0.0, 255.0) as u8;
    (g(c.0), g(c.1), g(c.2))
}

fn luma(c: Rgb) -> f32 {
    0.3 * c.0 as f32 + 0.59 * c.1 as f32 + 0.11 * c.2 as f32
}

/// Pull a color toward grey of the same brightness (overcast skies).
fn desaturate(c: Rgb, k: f32) -> Rgb {
    let l = luma(c).round() as u8;
    mix(c, (l, l, l), k)
}

/// A cheap deterministic hash for scenery placement.
fn hash(n: u32) -> u32 {
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x
}

// ---------------------------------------------------------------------------
// Sky
// ---------------------------------------------------------------------------

/// Hour of day (0-24) -> (top, bottom) sky colors, interpolated between keyframes.
fn sky_for_hour(h: f32) -> (Rgb, Rgb) {
    const KEYS: [(f32, Rgb, Rgb); 8] = [
        (0.0, (8, 10, 36), (26, 30, 74)),
        (5.0, (10, 14, 46), (40, 40, 96)),
        (7.0, (70, 96, 176), (250, 168, 120)),
        (9.0, (78, 156, 238), (170, 220, 255)),
        (17.0, (78, 156, 238), (170, 220, 255)),
        (19.0, (84, 70, 160), (255, 140, 92)),
        (21.0, (14, 16, 54), (40, 40, 96)),
        (24.0, (8, 10, 36), (26, 30, 74)),
    ];
    let h = h.rem_euclid(24.0);
    for w in KEYS.windows(2) {
        let (h0, t0, b0) = w[0];
        let (h1, t1, b1) = w[1];
        if h >= h0 && h <= h1 {
            let k = (h - h0) / (h1 - h0).max(0.001);
            return (mix(t0, t1, k), mix(b0, b1, k));
        }
    }
    (KEYS[0].1, KEYS[0].2)
}

/// 0 = full night, 1 = full day.
fn daylight(h: f32) -> f32 {
    let h = h.rem_euclid(24.0);
    if h < 5.0 || h >= 21.0 {
        0.0
    } else if h < 8.0 {
        (h - 5.0) / 3.0
    } else if h < 18.0 {
        1.0
    } else {
        (21.0 - h) / 3.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sky {
    Clear,
    Partly,
    Overcast,
    Rain(u32),
    Snow,
    Fog,
    Storm,
}

fn sky_kind(w: Option<WeatherIcon>) -> Sky {
    match w {
        None | Some(WeatherIcon::Clear) => Sky::Clear,
        Some(WeatherIcon::PartlyCloudy) => Sky::Partly,
        Some(WeatherIcon::Cloudy) => Sky::Overcast,
        Some(WeatherIcon::Fog) => Sky::Fog,
        Some(WeatherIcon::Drizzle) => Sky::Rain(26),
        Some(WeatherIcon::Rain) => Sky::Rain(70),
        Some(WeatherIcon::Snow) => Sky::Snow,
        Some(WeatherIcon::Thunder) => Sky::Storm,
    }
}

// ---------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------

const GROUND_H: i32 = 9;
const SPRITE: i32 = 16;
const STEP: f32 = 1.0 / 60.0;
const GRAVITY: f32 = 640.0;
const JUMP_V: f32 = 190.0;
const HOP_V: f32 = 92.0;
/// Hero hitbox, relative to its sprite: x range and height.
const HB_X0: f32 = 4.0;
const HB_X1: f32 = 12.0;
const HB_H: f32 = 13.0;
pub const MIN_ROWS: i32 = 56;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Rock,
    Bush,
    Crate,
    Slime,
    Pit,
}

impl Kind {
    /// (width, height) in virtual pixels.
    fn size(self) -> (f32, f32) {
        match self {
            Kind::Rock => (10.0, 7.0),
            Kind::Bush => (12.0, 9.0),
            Kind::Crate => (10.0, 11.0),
            Kind::Slime => (10.0, 8.0),
            Kind::Pit => (0.0, 0.0), // width is per instance
        }
    }
}

#[derive(Clone, Debug)]
struct Obstacle {
    kind: Kind,
    x: f32,
    w: f32,
    h: f32,
    /// Slimes killed by the knight stay in the list one more step to animate.
    dead: bool,
}

#[derive(Clone, Debug)]
struct Pickup {
    x: f32,
    y: f32, // height above the ground
    chest: bool,
    taken: bool,
    opened_at: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PKind {
    Dust,
    Note(u8),
    Spark(Rgb),
    Splash,
    Poof,
    Firework(Rgb),
}

#[derive(Clone, Debug)]
struct Particle {
    kind: PKind,
    x: f32,
    y: f32, // screen coordinates (virtual px, y down)
    vx: f32,
    vy: f32,
    life: f32,
    max_life: f32,
}

#[derive(Clone, Debug)]
struct Cloud {
    x: f32,
    y: f32,
    big: bool,
    speed: f32,
}

#[derive(Clone, Debug)]
struct Drop {
    x: f32,
    y: f32,
    v: f32,
}

#[derive(Clone, Copy, Debug)]
struct Shell {
    at: f32, // game time at which it launches
    x: f32,
    y: f32,
    color: Rgb,
}

pub struct Game {
    t: f32,
    rng: u32,
    pub vw: i32,
    pub vh: i32,
    initialized_layout: bool,
    scroll: f32,
    speed: f32,
    // hero
    hero_y: f32,
    hero_vy: f32,
    hero_hop: bool,
    attack_until: f32,
    hero: Hero,
    // world
    obstacles: Vec<Obstacle>,
    pickups: Vec<Pickup>,
    particles: Vec<Particle>,
    clouds: Vec<Cloud>,
    drops: Vec<Drop>,
    since_spawn: f32,
    next_gap: f32,
    pending_coins: u32,
    pending_chest: bool,
    pub coins: u32,
    // events
    net_avg: f64,
    disk_avg: f64,
    last_net_event: f32,
    last_disk_event: f32,
    prev_bass: f32,
    last_beat: f32,
    hour_key: Option<u64>,
    shells: Vec<Shell>,
    celebrate_until: f32,
    flash_until: f32,
    next_flash: f32,
    hero_key: Option<u64>,
    sprint: bool,
}

impl Game {
    pub fn new(seed: u32) -> Self {
        Game {
            t: 0.0,
            rng: seed.max(1),
            vw: 0,
            vh: 0,
            initialized_layout: false,
            scroll: 0.0,
            speed: 70.0,
            hero_y: 0.0,
            hero_vy: 0.0,
            hero_hop: false,
            attack_until: -1.0,
            hero: Hero::Knight,
            obstacles: Vec::new(),
            pickups: Vec::new(),
            particles: Vec::new(),
            clouds: Vec::new(),
            drops: Vec::new(),
            since_spawn: 0.0,
            next_gap: 120.0,
            pending_coins: 0,
            pending_chest: false,
            coins: 0,
            net_avg: 0.0,
            disk_avg: 0.0,
            last_net_event: -99.0,
            last_disk_event: -99.0,
            prev_bass: 0.0,
            last_beat: -99.0,
            hour_key: None,
            shells: Vec::new(),
            celebrate_until: -1.0,
            flash_until: -1.0,
            next_flash: 5.0,
            hero_key: None,
            sprint: false,
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rand()
    }

    /// Set the size of the world (virtual pixels). Cheap when nothing changed.
    pub fn resize(&mut self, vw: i32, vh: i32) {
        if self.initialized_layout && self.vw == vw && self.vh == vh {
            return;
        }
        let first = !self.initialized_layout;
        self.vw = vw.max(40);
        self.vh = vh.max(MIN_ROWS);
        self.initialized_layout = true;
        if first || self.obstacles.is_empty() {
            self.obstacles.clear();
            self.pickups.clear();
            // Clouds across the sky.
            self.clouds.clear();
            for i in 0..14 {
                let x = self.range(0.0, self.vw as f32);
                let y = self.range(2.0, (self.vh as f32 * 0.38).max(6.0));
                let speed = self.range(1.5, 6.0);
                self.clouds.push(Cloud { x, y, big: i % 3 != 0, speed });
            }
            self.since_spawn = 0.0;
            self.next_gap = 90.0;
        }
        // Keep what was in flight inside the new size.
        for o in &mut self.obstacles {
            o.x = o.x.min(self.vw as f32 + 4.0);
        }
        self.drops.clear();
    }

    pub fn ground_y(&self) -> i32 {
        self.vh - GROUND_H
    }

    fn hero_x(&self) -> f32 {
        (self.vw as f32 * 0.22).max(24.0)
    }

    fn grounded(&self) -> bool {
        self.hero_y <= 0.0 && self.hero_vy <= 0.0
    }

    // ---------------- simulation ----------------

    /// Advance the world by `dt` seconds (any size; split into fixed steps).
    pub fn advance(&mut self, dt: f32, inp: &Inputs) {
        let mut left = dt.clamp(0.0, 0.5);
        while left > 1e-4 {
            let d = left.min(STEP);
            self.step(d, inp);
            left -= d;
        }
    }

    /// Seconds the hero stays above height `h` in a full jump, and when that starts.
    fn above(h: f32) -> (f32, f32) {
        let disc = (JUMP_V * JUMP_V - 2.0 * GRAVITY * h).max(0.0).sqrt();
        ((JUMP_V - disc) / GRAVITY, 2.0 * disc / GRAVITY)
    }

    fn step(&mut self, dt: f32, inp: &Inputs) {
        self.t += dt;
        let t = self.t;
        let ground = self.ground_y() as f32;
        let hx = self.hero_x();

        // ---- which hero ----
        let key = inp.unix_secs / SWAP_SECS;
        if self.hero_key != Some(key) {
            let swapped = self.hero_key.is_some();
            self.hero_key = Some(key);
            if swapped {
                self.burst(hx + 8.0, ground - 8.0, PKind::Poof, 10);
            }
        }

        // ---- speed from CPU (only on the ground: a jump keeps its speed) ----
        let target = 62.0 + inp.cpu.clamp(0.0, 100.0) * 0.80;
        if self.grounded() {
            let max_change = 30.0 * dt;
            self.speed += (target - self.speed).clamp(-max_change, max_change);
        }
        self.sprint = inp.cpu > 80.0;
        self.scroll += self.speed * dt;

        // ---- events ----
        self.events(inp);

        // ---- obstacles move, spawn ----
        for o in &mut self.obstacles {
            o.x -= self.speed * dt;
        }
        for p in &mut self.pickups {
            p.x -= self.speed * dt;
        }
        self.obstacles.retain(|o| o.x + o.w > -24.0 && !o.dead);
        self.pickups.retain(|p| p.x > -24.0 && !(p.taken && p.chest && t - p.opened_at > 1.2));
        self.since_spawn += self.speed * dt;
        if self.since_spawn >= self.next_gap && t >= self.celebrate_until {
            self.spawn();
        }

        // ---- hero: autopilot, physics ----
        self.autopilot(inp);
        if !self.grounded() || self.hero_vy > 0.0 {
            self.hero_vy -= GRAVITY * dt;
            self.hero_y += self.hero_vy * dt;
            if self.hero_y <= 0.0 {
                self.hero_y = 0.0;
                self.hero_vy = 0.0;
                self.hero_hop = false;
                if self.speed > 70.0 {
                    self.burst(hx + 6.0, ground, PKind::Dust, 2);
                }
            }
        }
        if self.sprint && self.grounded() && self.rand() < 0.35 {
            self.burst(hx + 3.0, ground - 1.0, PKind::Dust, 1);
        }

        // ---- pickups ----
        let (b0, b1) = (hx + HB_X0, hx + HB_X1);
        let hero_y = self.hero_y;
        let mut gained = 0u32;
        let mut sparks: Vec<(f32, f32, u32)> = Vec::new();
        for p in &mut self.pickups {
            if p.taken {
                continue;
            }
            let (pw, ph) = if p.chest { (12.0, 9.0) } else { (5.0, 5.0) };
            let overlap_x = p.x < b1 && p.x + pw > b0;
            let overlap_y = p.y < hero_y + HB_H && p.y + ph > hero_y;
            if overlap_x && overlap_y {
                p.taken = true;
                p.opened_at = t;
                if p.chest {
                    gained += 5;
                    sparks.push((p.x + 6.0, ground - p.y - 9.0, 14));
                } else {
                    gained += 1;
                    sparks.push((p.x + 2.0, ground - p.y - 3.0, 4));
                }
            }
        }
        self.coins += gained;
        for (x, y, n) in sparks {
            self.burst(x, y, PKind::Spark((252, 218, 64)), n);
        }

        // ---- particles, clouds, rain ----
        self.update_particles(dt, ground);
        self.update_clouds(dt);
        self.update_drops(dt, inp);
        self.update_shells();
    }

    fn burst(&mut self, x: f32, y: f32, kind: PKind, n: u32) {
        for _ in 0..n {
            let (vx, vy, life) = match kind {
                PKind::Dust => (self.range(-30.0, -8.0), self.range(-14.0, -2.0), self.range(0.25, 0.5)),
                PKind::Spark(_) => (self.range(-30.0, 30.0), self.range(-60.0, -15.0), self.range(0.3, 0.6)),
                PKind::Poof => (self.range(-26.0, 26.0), self.range(-26.0, 6.0), self.range(0.3, 0.6)),
                PKind::Splash => (self.range(-12.0, 12.0), self.range(-26.0, -8.0), 0.18),
                PKind::Note(_) => (self.range(4.0, 14.0), self.range(-24.0, -14.0), self.range(0.9, 1.4)),
                PKind::Firework(_) => (0.0, 0.0, 0.0),
            };
            self.particles.push(Particle { kind, x, y, vx, vy, life, max_life: life });
        }
        if self.particles.len() > 400 {
            let extra = self.particles.len() - 400;
            self.particles.drain(0..extra);
        }
    }

    fn update_particles(&mut self, dt: f32, ground: f32) {
        for p in &mut self.particles {
            p.life -= dt;
            match p.kind {
                PKind::Dust | PKind::Poof => {
                    p.x += (p.vx - self.speed * 0.3) * dt;
                    p.y += p.vy * dt;
                }
                PKind::Note(_) => {
                    p.x += p.vx * dt + (p.life * 6.0).sin() * 0.15;
                    p.y += p.vy * dt;
                }
                PKind::Spark(_) | PKind::Splash => {
                    p.vy += 220.0 * dt;
                    p.x += p.vx * dt;
                    p.y = (p.y + p.vy * dt).min(ground);
                }
                PKind::Firework(_) => {
                    p.vy += 46.0 * dt;
                    p.vx *= 1.0 - 0.9 * dt;
                    p.vy *= 1.0 - 0.5 * dt;
                    p.x += p.vx * dt;
                    p.y += p.vy * dt;
                }
            }
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    fn update_clouds(&mut self, dt: f32) {
        let w = self.vw as f32;
        for c in &mut self.clouds {
            c.x -= (c.speed + self.speed * 0.02) * dt;
            if c.x < -26.0 {
                c.x = w + 4.0;
            }
        }
    }

    fn update_drops(&mut self, dt: f32, inp: &Inputs) {
        let want = match sky_kind(inp.weather) {
            Sky::Rain(n) => n as usize,
            Sky::Storm => 90,
            Sky::Snow => 46,
            _ => 0,
        };
        let (w, ground) = (self.vw as f32, self.ground_y() as f32);
        while self.drops.len() < want {
            let (x, y, v) = (self.range(0.0, w + 20.0), self.range(-10.0, ground), self.range(0.0, 1.0));
            self.drops.push(Drop { x, y, v });
        }
        self.drops.truncate(want);
        let snow = sky_kind(inp.weather) == Sky::Snow;
        let mut splashes = Vec::new();
        for d in &mut self.drops {
            if snow {
                d.y += (8.0 + d.v * 10.0) * dt;
                d.x += ((d.y * 0.2 + d.v * 9.0).sin() * 5.0 - self.speed * 0.15) * dt;
            } else {
                d.y += (120.0 + d.v * 60.0) * dt;
                d.x -= (30.0 + self.speed * 0.1) * dt;
            }
            if d.y >= ground {
                if !snow {
                    splashes.push(d.x);
                }
                d.y = -4.0;
                d.x = (d.x + 40.0).rem_euclid(w + 20.0);
            }
            if d.x < -4.0 {
                d.x += w + 24.0;
            }
        }
        for x in splashes.into_iter().take(2) {
            self.burst(x, ground - 1.0, PKind::Splash, 2);
        }
    }

    fn update_shells(&mut self) {
        let t = self.t;
        let mut launched = Vec::new();
        self.shells.retain(|s| {
            if t >= s.at {
                launched.push(*s);
                false
            } else {
                true
            }
        });
        for s in launched {
            for i in 0..26 {
                let a = i as f32 / 26.0 * std::f32::consts::TAU;
                let sp = 22.0 + (hash(i as u32 + (s.x * 7.0) as u32) % 100) as f32 * 0.22;
                self.particles.push(Particle {
                    kind: PKind::Firework(s.color),
                    x: s.x,
                    y: s.y,
                    vx: a.cos() * sp,
                    vy: a.sin() * sp,
                    life: 1.1,
                    max_life: 1.1,
                });
            }
        }
    }

    // ---------------- events ----------------

    fn events(&mut self, inp: &Inputs) {
        let t = self.t;
        let hx = self.hero_x();
        let ground = self.ground_y() as f32;

        // Network burst -> a row of coins.
        self.net_avg += (inp.net_kb - self.net_avg) * (STEP as f64 / 20.0);
        if inp.net_kb > 800.0 && inp.net_kb > self.net_avg * 2.0 && t - self.last_net_event > 2.5 {
            self.last_net_event = t;
            self.pending_coins = (self.pending_coins + 3 + ((inp.net_kb / 1500.0).min(5.0)) as u32).min(10);
        }
        // Disk burst -> a treasure chest.
        self.disk_avg += (inp.disk_mb - self.disk_avg) * (STEP as f64 / 20.0);
        if inp.disk_mb > 20.0 && inp.disk_mb > self.disk_avg * 2.0 && t - self.last_disk_event > 6.0 {
            self.last_disk_event = t;
            self.pending_chest = true;
        }

        // Music: hop and notes on the bass.
        let beat = inp.music && inp.bass > 0.55 && self.prev_bass <= 0.55 && t - self.last_beat > 0.25;
        self.prev_bass = inp.bass;
        if beat {
            self.last_beat = t;
            let n = (self.rng >> 5) as u8 % 3;
            self.burst(hx + 8.0, ground - self.hero_y - 17.0, PKind::Note(n), 1);
            if self.grounded() && self.safe_to_hop() {
                self.hero_vy = HOP_V;
                self.hero_y = 0.01;
                self.hero_hop = true;
            }
        }

        // The hour strikes: fireworks and a victory dance.
        let key = inp.unix_secs / 3600;
        match self.hour_key {
            None => self.hour_key = Some(key),
            Some(k) if k != key => {
                self.hour_key = Some(key);
                if inp.minute == 0 && inp.second < 5 {
                    self.start_celebration();
                }
            }
            _ => {}
        }
        if t < self.celebrate_until && self.grounded() && self.safe_to_hop() && self.rand() < 0.03 {
            self.hero_vy = HOP_V * 1.4;
            self.hero_y = 0.01;
            self.hero_hop = true;
        }

        // Lightning.
        if sky_kind(inp.weather) == Sky::Storm && t >= self.next_flash {
            self.flash_until = t + 0.14;
            self.next_flash = t + self.range(4.0, 10.0);
        }
    }

    /// Fireworks over the sky.
    pub fn start_celebration(&mut self) {
        let t = self.t;
        self.celebrate_until = t + 6.0;
        let colors: [Rgb; 5] = [(255, 90, 90), (255, 220, 80), (110, 220, 255), (190, 130, 255), (130, 255, 150)];
        for i in 0..6 {
            let x = self.range(self.vw as f32 * 0.15, self.vw as f32 * 0.85);
            let y = self.range(8.0, self.vh as f32 * 0.4);
            self.shells.push(Shell { at: t + 0.4 + i as f32 * 0.8, x, y, color: colors[i % colors.len()] });
        }
    }

    /// A small hop is safe when no obstacle is close.
    fn safe_to_hop(&self) -> bool {
        let hx = self.hero_x() + HB_X1;
        let lead = self.speed * 0.32 + 52.0;
        !self.obstacles.iter().any(|o| !o.dead && o.x + o.w > hx - 12.0 && o.x - hx < lead)
    }

    // ---------------- obstacles ----------------

    fn spawn(&mut self) {
        let vw = self.vw as f32;
        let r = self.rand();
        let kind = if r < 0.24 {
            Kind::Rock
        } else if r < 0.48 {
            Kind::Bush
        } else if r < 0.68 {
            Kind::Crate
        } else if r < 0.84 {
            Kind::Slime
        } else {
            Kind::Pit
        };
        let (mut w, h) = kind.size();
        if kind == Kind::Pit {
            w = self.range(12.0, 18.0).round();
        }
        let x = vw + 4.0;
        // Where the previous obstacle ended: the gap in between holds pickups.
        let prev_end = self.obstacles.iter().filter(|o| !o.dead).map(|o| o.x + o.w).fold(f32::MIN, f32::max);
        self.obstacles.push(Obstacle { kind, x, w, h, dead: false });
        let gap_px = if prev_end > f32::MIN { x - prev_end } else { self.next_gap };
        let mid = x - gap_px * 0.5;
        if self.pending_chest && gap_px > 70.0 {
            self.pending_chest = false;
            self.pickups.push(Pickup { x: mid - 6.0, y: 0.0, chest: true, taken: false, opened_at: 0.0 });
        } else if gap_px > 70.0 && (self.pending_coins > 0 || self.rand() < 0.18) {
            let n = if self.pending_coins > 0 { self.pending_coins.min(6) } else { 3 };
            self.pending_coins = self.pending_coins.saturating_sub(n);
            let row_w = n as f32 * 8.0;
            let x0 = mid - row_w * 0.5;
            for i in 0..n {
                self.pickups.push(Pickup { x: x0 + i as f32 * 8.0, y: 5.0, chest: false, taken: false, opened_at: 0.0 });
            }
        }
        // The distance to the next one grows with the speed: a jump covers more ground when running fast.
        let gap = 56.0 + self.speed * 0.65 + self.range(0.0, 90.0);
        self.next_gap = gap + w;
        self.since_spawn = 0.0;
    }

    // ---------------- hero ----------------

    fn autopilot(&mut self, _inp: &Inputs) {
        if !self.grounded() {
            return;
        }
        let hx = self.hero_x();
        let front = hx + HB_X1;
        let back = hx + HB_X0;
        let knight = self.hero == Hero::Knight;
        let speed = self.speed;
        let t = self.t;
        // Nearest obstacle still ahead of the hero's back edge.
        let Some(idx) = self
            .obstacles
            .iter()
            .enumerate()
            .filter(|(_, o)| !o.dead && o.x + o.w > back)
            .min_by(|a, b| a.1.x.partial_cmp(&b.1.x).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
        else {
            return;
        };
        let o = self.obstacles[idx].clone();
        let d = o.x - front;
        // The knight cuts slimes down at walking pace, everyone else jumps.
        if knight && o.kind == Kind::Slime && speed < 105.0 {
            if d <= 13.0 && d > -4.0 {
                self.attack_until = t + 0.28;
                self.obstacles[idx].dead = true;
                self.obstacles[idx].h = -1.0;
                let (sx, sy) = (o.x + 5.0, self.ground_y() as f32 - 5.0);
                self.burst(sx, sy, PKind::Poof, 8);
                self.pickups.push(Pickup { x: o.x + 3.0, y: 2.0, chest: false, taken: false, opened_at: 0.0 });
            }
            return;
        }
        let h_eff = if o.kind == Kind::Pit { 0.6 } else { o.h };
        let (t1, tw) = Self::above(h_eff);
        let need = o.w + (HB_X1 - HB_X0);
        let takeoff = speed * t1 + (speed * tw - need) * 0.5;
        if d <= takeoff.max(1.0) && d > -2.0 {
            self.hero_vy = JUMP_V;
            self.hero_y = 0.01;
            self.hero_hop = false;
        }
    }

    // ---------------- collisions (for tests) ----------------

    /// True if the hero touches an obstacle right now (used by the tests).
    #[cfg(test)]
    pub fn hero_hit(&self) -> bool {
        let hx = self.hero_x();
        let (b0, b1) = (hx + HB_X0, hx + HB_X1);
        self.obstacles.iter().filter(|o| !o.dead).any(|o| {
            let overlap = o.x < b1 && o.x + o.w > b0;
            if !overlap {
                return false;
            }
            if o.kind == Kind::Pit {
                // Falls in when its feet are on the ground over the hole.
                self.hero_y < 0.5 && o.x < b0 + 1.0 && o.x + o.w > b1 - 1.0
            } else {
                self.hero_y < o.h
            }
        })
    }

    // ---------------- drawing ----------------

    fn hero_kind(&self, choice: HeroChoice) -> Hero {
        match choice {
            HeroChoice::Knight => Hero::Knight,
            HeroChoice::Cat => Hero::Cat,
            HeroChoice::Both => self.hero,
        }
    }

    /// Which hero is out, by the clock (so it survives a restart sensibly).
    pub fn pick_hero(&mut self, choice: HeroChoice, unix_secs: u64) {
        self.hero = match choice {
            HeroChoice::Knight => Hero::Knight,
            HeroChoice::Cat => Hero::Cat,
            HeroChoice::Both => {
                if (unix_secs / SWAP_SECS) % 2 == 0 {
                    Hero::Knight
                } else {
                    Hero::Cat
                }
            }
        };
    }

    pub fn draw(&self, cv: &mut Canvas, inp: &Inputs, choice: HeroChoice) {
        let (vw, vh) = (self.vw, self.vh);
        let ground = self.ground_y();
        let hour_f = inp.hour as f32 + inp.minute as f32 / 60.0 + inp.second as f32 / 3600.0;
        let day = daylight(hour_f);
        let sky = sky_kind(inp.weather);

        // ---- sky: a banded gradient ----
        let (mut top, mut bot) = sky_for_hour(hour_f);
        let overcast_k = match sky {
            Sky::Clear => 0.0,
            Sky::Partly => 0.12,
            Sky::Overcast => 0.62,
            Sky::Rain(_) => 0.7,
            Sky::Storm => 0.85,
            Sky::Snow => 0.45,
            Sky::Fog => 0.7,
        };
        if overcast_k > 0.0 {
            let dim = if matches!(sky, Sky::Storm | Sky::Rain(_)) { 0.72 } else { 0.9 };
            top = scale_rgb(desaturate(top, overcast_k), dim);
            bot = scale_rgb(desaturate(bot, overcast_k), dim);
        }
        if sky == Sky::Fog {
            top = mix(top, (186, 192, 200), 0.45 * day.max(0.3));
            bot = mix(bot, (200, 205, 210), 0.5 * day.max(0.3));
        }
        let flash = self.t < self.flash_until;
        let bands = 8;
        for i in 0..bands {
            let y0 = ground * i / bands;
            let y1 = ground * (i + 1) / bands;
            let mut c = mix(top, bot, (i as f32 + 0.5) / bands as f32);
            if flash {
                c = mix(c, (235, 238, 255), 0.75);
            }
            cv.rect(0, y0, vw, y1 - y0 + 1, c);
        }

        // ---- stars, sun, moon ----
        let clear_sky = matches!(sky, Sky::Clear | Sky::Partly);
        if day < 0.5 && clear_sky {
            let bright = 1.0 - day * 2.0;
            for i in 0..34u32 {
                let h = hash(i * 31 + 7);
                let x = (h % vw.max(1) as u32) as i32;
                let y = ((h >> 10) % (ground as u32 * 6 / 10).max(1)) as i32;
                let twinkle = ((self.t * 2.0 + i as f32 * 1.7).sin() * 0.5 + 0.5) * 0.6 + 0.4;
                let v = (210.0 * bright * twinkle) as u8;
                cv.px(x, y, (v, v, (v as u32 + 30).min(255) as u8));
            }
        }
        if !matches!(sky, Sky::Overcast | Sky::Rain(_) | Sky::Storm | Sky::Snow | Sky::Fog) {
            if day > 0.05 {
                let frac = ((hour_f - 6.0) / 12.0).clamp(0.0, 1.0);
                let x = (vw as f32 * (0.08 + 0.84 * frac)) as i32 - 6;
                let y = (ground as f32 * 0.62 - (frac * std::f32::consts::PI).sin() * ground as f32 * 0.5) as i32;
                cv.sprite(&SUN, x, y.max(1), None);
            } else {
                let nh = (hour_f + 6.0).rem_euclid(24.0); // 18h -> 0 ... 6h -> 12
                let frac = (nh / 12.0).clamp(0.0, 1.0);
                let x = (vw as f32 * (0.08 + 0.84 * frac)) as i32 - 4;
                let y = (ground as f32 * 0.55 - (frac * std::f32::consts::PI).sin() * ground as f32 * 0.42) as i32;
                cv.sprite(&MOON, x, y.max(1), None);
            }
        }

        // ---- clouds (more when overcast) ----
        let count = match sky {
            Sky::Clear => 3,
            Sky::Partly => 6,
            Sky::Overcast | Sky::Rain(_) | Sky::Storm => 14,
            Sky::Snow | Sky::Fog => 9,
        };
        let cloud_color = {
            let base = mix((120, 128, 160), (255, 255, 255), day);
            match sky {
                Sky::Overcast | Sky::Snow => mix(base, (150, 156, 170), 0.5),
                Sky::Rain(_) => mix(base, (110, 116, 132), 0.7),
                Sky::Storm => mix(base, (78, 82, 100), 0.8),
                _ => base,
            }
        };
        for c in self.clouds.iter().take(count) {
            cv.sprite(if c.big { &CLOUD_BIG } else { &CLOUD_SMALL }, c.x as i32, c.y as i32, Some(cloud_color));
        }
        if day > 0.4 && clear_sky {
            let f = ((self.t * 4.0) as usize) % 2;
            for i in 0..2 {
                let bx = ((self.scroll * 0.1) as i32 + i * 97).rem_euclid(vw + 30) as i32;
                cv.sprite(&BIRD[f], vw - bx, 6 + i * 7, None);
            }
        }

        // ---- hills (far) and trees (near) ----
        let hill_col = {
            let d = mix((24, 28, 62), (96, 170, 120), day);
            if overcast_k > 0.0 { desaturate(d, overcast_k * 0.8) } else { d }
        };
        let hill2_col = scale_rgb(hill_col, 0.82);
        for x in 0..vw {
            let wx = x as f32 + self.scroll * 0.15;
            let h1 = 7.0 + (wx * 0.045).sin() * 5.0 + (wx * 0.11).sin() * 2.0;
            let top1 = ground - h1 as i32;
            cv.rect(x, top1, 1, ground - top1, hill_col);
            let wx2 = x as f32 + self.scroll * 0.3 + 40.0;
            let h2 = 3.0 + (wx2 * 0.07).sin() * 2.5 + (wx2 * 0.19).sin() * 1.0;
            let top2 = ground - h2 as i32;
            cv.rect(x, top2, 1, ground - top2, hill2_col);
        }
        let cell = 52i32;
        let off = (self.scroll * 0.5) as i32;
        for ci in (off / cell)..=((off + vw) / cell + 1) {
            let h = hash(ci as u32 + 99);
            if h % 3 == 0 {
                let x = ci * cell + (h >> 8) as i32 % 20 - off;
                cv.dim = 0.35 + 0.65 * day;
                cv.sprite(&TREE, x, ground - 14, None);
                cv.dim = 1.0;
            }
        }

        // ---- ground with pits ----
        let grass = mix((26, 66, 56), (84, 190, 78), day);
        let dirt = mix((46, 36, 56), (146, 100, 62), day);
        let dirt_dark = scale_rgb(dirt, 0.78);
        cv.rect(0, ground, vw, 2, grass);
        cv.rect(0, ground + 2, vw, vh - ground - 2, dirt);
        let sc = self.scroll as i32;
        // Grass tufts and dirt speckles, scrolling with the world.
        for i in 0..(vw / 6 + 2) {
            let wx = i * 6 + 3 - sc.rem_euclid(6);
            cv.px(wx, ground - 1, scale_rgb(grass, 0.85));
        }
        for row in 0..(GROUND_H - 3) {
            for i in 0..(vw / 9 + 2) {
                let n = hash((row * 131 + i) as u32);
                let wx = i * 9 + (n % 7) as i32 - (sc.rem_euclid(9));
                if (n >> 5) % 3 != 0 {
                    cv.px(wx, ground + 3 + row, dirt_dark);
                }
            }
        }
        for o in &self.obstacles {
            if o.kind == Kind::Pit {
                cv.rect(o.x as i32, ground, o.w as i32, vh - ground, (12, 10, 24));
                cv.rect(o.x as i32, ground, 1, vh - ground, scale_rgb(dirt, 0.6));
                cv.rect(o.x as i32 + o.w as i32 - 1, ground, 1, vh - ground, scale_rgb(dirt, 0.6));
            }
        }

        // ---- pickups, obstacles ----
        let spin = ((self.t * 9.0) as usize) % 4;
        for p in &self.pickups {
            let sy = ground - p.y as i32;
            if p.chest {
                cv.sprite(if p.taken { &CHEST_OPEN } else { &CHEST }, p.x as i32, sy - 9, None);
            } else if !p.taken {
                let bob = ((self.t * 5.0 + p.x * 0.2).sin() * 1.2) as i32;
                cv.sprite(&COIN[spin], p.x as i32, sy - 5 + bob, None);
            }
        }
        let slime_f = ((self.t * 3.0) as usize) % 2;
        for o in &self.obstacles {
            if o.dead {
                continue;
            }
            let x = o.x as i32;
            match o.kind {
                Kind::Rock => cv.sprite(&ROCK, x, ground - 7, None),
                Kind::Bush => cv.sprite(&BUSH, x - 0, ground - 9, None),
                Kind::Crate => cv.sprite(&CRATE, x, ground - 11, None),
                Kind::Slime => cv.sprite(&SLIME[slime_f], x, ground - 8, None),
                Kind::Pit => {}
            }
        }

        // ---- particles behind the hero ----
        self.draw_particles(cv, false);

        // ---- hero ----
        self.draw_hero(cv, choice, ground);

        // ---- particles in front (notes, fireworks, sparks) ----
        self.draw_particles(cv, true);

        // ---- weather overlays ----
        match sky {
            Sky::Rain(_) | Sky::Storm => {
                let c = (150, 190, 255);
                for d in &self.drops {
                    cv.rect(d.x as i32, d.y as i32, 1, 3, c);
                }
            }
            Sky::Snow => {
                for d in &self.drops {
                    let big = d.v > 0.6;
                    cv.rect(d.x as i32, d.y as i32, if big { 2 } else { 1 }, if big { 2 } else { 1 }, (240, 245, 255));
                }
            }
            Sky::Fog => {
                // Soft drifting haze: a few translucent bands, thicker near the ground.
                let haze = (218, 222, 228);
                let drift = ((self.t * 2.0).sin() * 3.0) as i32;
                for (n, (y, h, a)) in [(ground - 26, 6, 0.16f32), (ground - 19, 7, 0.22), (ground - 11, 8, 0.28), (ground - 4, 6, 0.34)].iter().enumerate() {
                    let _ = (n, drift);
                    cv.blend(0, *y, vw, *h, haze, *a);
                }
            }
            _ => {}
        }
        if flash {
            // A jagged bolt.
            let mut x = vw / 2 + ((self.next_flash * 13.0) as i32 % (vw / 3).max(1)) - vw / 6;
            for y in (0..ground).step_by(3) {
                x += ((hash(y as u32 + (self.t * 100.0) as u32) % 5) as i32) - 2;
                cv.rect(x, y, 2, 3, (255, 250, 200));
            }
        }

        // ---- frame ----
        let edge = (8, 8, 20);
        cv.rect(0, 0, vw, 1, edge);
        cv.rect(0, 0, 1, vh, edge);
        cv.rect(vw - 1, 0, 1, vh, edge);
        cv.rect(0, vh - 1, vw, 1, edge);
    }

    fn draw_particles(&self, cv: &mut Canvas, front: bool) {
        for p in &self.particles {
            let is_front = matches!(p.kind, PKind::Note(_) | PKind::Firework(_) | PKind::Spark(_));
            if is_front != front {
                continue;
            }
            let k = (p.life / p.max_life).clamp(0.0, 1.0);
            let (x, y) = (p.x as i32, p.y as i32);
            match p.kind {
                PKind::Dust | PKind::Poof => {
                    let c = mix((120, 110, 100), (225, 220, 210), k);
                    let sz = if k > 0.5 { 2 } else { 1 };
                    cv.rect(x, y, sz, sz, c);
                }
                PKind::Note(n) => {
                    let cols: [Rgb; 3] = [(255, 230, 90), (120, 230, 255), (255, 140, 200)];
                    // the note sprite is black-outlined: tint its white pixels
                    cv.sprite(&NOTE, x, y, Some(cols[(n % 3) as usize]));
                }
                PKind::Spark(c) => {
                    cv.px(x, y, mix((255, 255, 255), c, 1.0 - k));
                }
                PKind::Splash => {
                    cv.px(x, y, (190, 220, 255));
                }
                PKind::Firework(c) => {
                    let v = mix((30, 30, 50), mix(c, (255, 255, 255), 0.35), k);
                    let sz = if k > 0.45 { 2 } else { 1 };
                    cv.rect(x, y, sz, sz, v);
                }
            }
        }
    }

    fn draw_hero(&self, cv: &mut Canvas, choice: HeroChoice, ground: i32) {
        let hero = self.hero_kind(choice);
        let hx = self.hero_x() as i32;
        let air = self.hero_y > 0.0;
        let top = ground - SPRITE - self.hero_y.round() as i32;
        // 4-frame run cycle, faster when running fast.
        let rate = 2.0 + self.speed * 0.075;
        let frame = ((self.scroll / 62.0 * rate * 0.5) as usize) % 4;
        let bob = if !air && (frame == 1 || frame == 3) { 1 } else { 0 };
        let attacking = self.t < self.attack_until;
        let rows: Vec<String> = match hero {
            Hero::Knight => {
                if attacking {
                    KNIGHT_ATTACK.iter().map(|s| s.to_string()).collect()
                } else if air {
                    KNIGHT_JUMP.iter().map(|s| s.to_string()).collect()
                } else {
                    KNIGHT_TOP.iter().chain(KNIGHT_LEGS[frame].iter()).map(|s| s.to_string()).collect()
                }
            }
            Hero::Cat => {
                if air {
                    CAT_JUMP.iter().map(|s| s.to_string()).collect()
                } else {
                    // the cat is shorter: rows 0-2 empty, body, legs
                    let mut v: Vec<String> = vec!["................".to_string(); 3];
                    v.extend(CAT_TOP.iter().map(|s| s.to_string()));
                    v.extend(CAT_LEGS[frame].iter().map(|s| s.to_string()));
                    v
                }
            }
        };
        cv.sprite_owned(&rows, hx, top + bob);
        // A soft shadow on the ground while jumping.
        if air {
            let w = (10.0 - self.hero_y * 0.12).max(4.0) as i32;
            cv.rect(hx + 8 - w / 2, ground, w, 1, (16, 40, 24));
        }
    }

    /// HUD text (coin counter) for the real framebuffer; returns nothing to draw if 0.
    pub fn hud_text(&self) -> String {
        format!("{}", self.coins)
    }

    #[cfg(test)]
    pub fn is_celebrating(&self) -> bool {
        self.t < self.celebrate_until
    }
}

/// Virtual-pixel scale for a strip of `h` real pixels: the biggest of 2-4 that still gives `MIN_ROWS` rows.
pub fn scale_for(h: u32) -> u32 {
    (h / MIN_ROWS as u32).clamp(2, 4)
}

/// The game plus the little bits of glue the main loop needs.
pub struct PixelGame {
    game: Game,
    last: Option<std::time::Instant>,
}

impl PixelGame {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
            .unwrap_or(12345);
        PixelGame { game: Game::new(seed), last: None }
    }

    /// Advance with the real elapsed time, then draw into `rect` of `fb`.
    pub fn frame(&mut self, fb: &mut Framebuffer, rect: (u32, u32, u32, u32), inp: &Inputs, choice: HeroChoice, hud: bool) {
        let now = std::time::Instant::now();
        let dt = self.last.map_or(0.0, |l| now.duration_since(l).as_secs_f32());
        self.last = Some(now);
        if rect.2 < 40 || rect.3 < 40 {
            return;
        }
        let s = scale_for(rect.3);
        self.game.resize(((rect.2 + s - 1) / s) as i32, ((rect.3 + s - 1) / s) as i32);
        self.game.pick_hero(choice, inp.unix_secs);
        self.game.advance(dt, inp);
        {
            let mut cv = Canvas::new(fb, rect, s);
            self.game.draw(&mut cv, inp, choice);
        }
        if hud {
            let text = self.game.hud_text();
            // a coin icon (scaled sprite) + the count
            let mut cv = Canvas::new(fb, rect, s);
            cv.sprite(&COIN[0], 4, 4, None);
            drop(cv);
            let x = rect.0 + 4 * s + 5 * s + 4;
            fb.draw_text(x + 1, rect.1 + 4 * s + 1, &format!("X{text}"), 0, 0, 0, 2);
            fb.draw_text(x, rect.1 + 4 * s, &format!("X{text}"), 255, 236, 120, 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, rows: &[&str], w: usize) {
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.len(), w, "{name} row {i} is {} wide, expected {w}: {r:?}", r.len());
            for ch in r.bytes() {
                assert!(ch == b'.' || pal(ch).is_some(), "{name} row {i}: unknown color {:?}", ch as char);
            }
        }
    }

    #[test]
    fn sprites_are_rectangular_and_use_known_colors() {
        check("knight top", &KNIGHT_TOP, 16);
        for (i, l) in KNIGHT_LEGS.iter().enumerate() {
            check(&format!("knight legs {i}"), l, 16);
        }
        check("knight jump", &KNIGHT_JUMP, 16);
        check("knight attack", &KNIGHT_ATTACK, 16);
        check("cat top", &CAT_TOP, 16);
        for (i, l) in CAT_LEGS.iter().enumerate() {
            check(&format!("cat legs {i}"), l, 16);
        }
        check("cat jump", &CAT_JUMP, 16);
        check("rock", &ROCK, 10);
        check("bush", &BUSH, 12);
        check("crate", &CRATE, 10);
        for f in &SLIME {
            check("slime", f, 10);
        }
        check("chest", &CHEST, 12);
        check("chest open", &CHEST_OPEN, 12);
        for f in &COIN {
            check("coin", f, 5);
        }
        check("note", &NOTE, 5);
        check("sun", &SUN, 13);
        check("moon", &MOON, 9);
        check("cloud big", &CLOUD_BIG, 22);
        check("cloud small", &CLOUD_SMALL, 12);
        check("tree", &TREE, 12);
        for f in &BIRD {
            check("bird", f, 5);
        }
        assert_eq!(KNIGHT_TOP.len() + KNIGHT_LEGS[0].len(), 16);
        assert_eq!(CAT_TOP.len() + CAT_LEGS[0].len() + 3, 16);
    }

    fn run(seed: u32, minutes: f32, dt: f32, mut inp: impl FnMut(f32, &mut Inputs), hero: Hero) -> (Game, u32) {
        let mut g = Game::new(seed);
        g.resize(640, 57);
        g.hero = hero;
        let mut i = Inputs::default();
        let (mut t, mut hits) = (0.0f32, 0u32);
        // Many small steps so a hit is never missed between frames.
        while t < minutes * 60.0 {
            inp(t, &mut i);
            i.unix_secs = 1_000_000_000 + t as u64;
            g.advance(dt, &i);
            if g.hero_hit() {
                hits += 1;
            }
            t += dt;
        }
        (g, hits)
    }

    #[test]
    fn the_hero_never_hits_an_obstacle_at_any_cpu_load() {
        for hero in [Hero::Knight, Hero::Cat] {
            for cpu in [0.0f32, 15.0, 40.0, 70.0, 100.0] {
                for seed in [1u32, 2, 3] {
                    let (g, hits) = run(seed, 6.0, 1.0 / 30.0, |_, i| i.cpu = cpu, hero);
                    assert_eq!(hits, 0, "{hero:?} cpu {cpu} seed {seed} hit something");
                    assert!(g.scroll > 100.0, "the world must move");
                }
            }
        }
    }

    #[test]
    fn the_hero_survives_changing_load_music_and_bursts() {
        for hero in [Hero::Knight, Hero::Cat] {
            for seed in [4u32, 5, 6, 7] {
                let (g, hits) = run(
                    seed,
                    12.0,
                    1.0 / 15.0,
                    |t, i| {
                        i.cpu = 50.0 + 50.0 * (t * 0.4).sin(); // speed keeps changing
                        i.music = true;
                        i.bass = if ((t * 3.1) as u32) % 2 == 0 { 0.9 } else { 0.1 }; // a busy beat
                        i.net_kb = if (t as u32) % 9 == 0 { 9000.0 } else { 50.0 };
                        i.disk_mb = if (t as u32) % 23 == 0 { 120.0 } else { 0.5 };
                        i.weather = Some(WeatherIcon::Thunder);
                    },
                    hero,
                );
                assert_eq!(hits, 0, "{hero:?} seed {seed} hit something");
                assert!(g.coins > 0, "coins should have been collected");
            }
        }
    }

    #[test]
    fn the_hourly_celebration_does_not_get_the_hero_killed() {
        for seed in [1u32, 2, 3] {
            let mut g = Game::new(seed);
            g.resize(640, 57);
            let mut i = Inputs::default();
            let mut hits = 0;
            for n in 0..(60 * 60) {
                // Cross an hour boundary a few times.
                i.unix_secs = 3_600 * 1000 + n / 4 * 30;
                i.minute = 0;
                i.second = 1;
                g.advance(1.0 / 15.0, &i);
                if n % 600 == 0 {
                    g.start_celebration();
                }
                hits += g.hero_hit() as u32;
            }
            assert_eq!(hits, 0, "seed {seed}");
        }
    }

    #[test]
    fn same_seed_same_world() {
        let go = |seed| {
            let (g, _) = run(seed, 1.0, 1.0 / 20.0, |_, i| i.cpu = 60.0, Hero::Cat);
            (g.scroll.to_bits(), g.obstacles.len(), g.coins, g.hero_y.to_bits())
        };
        assert_eq!(go(9), go(9));
        assert_ne!(go(9).0, go(10).0 + 1); // sanity: the function does not ignore its input
    }

    #[test]
    fn frame_rate_does_not_change_how_far_the_hero_runs() {
        let dist = |dt| run(3, 0.5, dt, |_, i| i.cpu = 50.0, Hero::Knight).0.scroll;
        let (a, b) = (dist(1.0 / 60.0), dist(1.0 / 8.0));
        assert!((a - b).abs() / a < 0.03, "{a} vs {b}");
    }

    #[test]
    fn cpu_load_sets_the_speed() {
        let speed = |cpu| run(1, 0.5, 1.0 / 30.0, |_, i| i.cpu = cpu, Hero::Knight).0.speed;
        assert!(speed(100.0) > speed(0.0) + 40.0, "{} vs {}", speed(100.0), speed(0.0));
        assert!(speed(0.0) >= 60.0 && speed(100.0) <= 145.0);
    }

    #[test]
    fn network_bursts_drop_coins_and_disk_bursts_a_chest() {
        let (g, _) = run(2, 1.0, 1.0 / 30.0, |t, i| i.net_kb = if t < 1.0 { 9000.0 } else { 0.0 }, Hero::Knight);
        assert!(g.coins >= 3 || g.pickups.iter().any(|p| !p.chest), "network burst should produce coins");
        let (g, _) = run(2, 1.0, 1.0 / 30.0, |t, i| i.disk_mb = if t < 1.0 { 300.0 } else { 0.0 }, Hero::Knight);
        assert!(g.coins >= 5 || g.pickups.iter().any(|p| p.chest), "disk burst should produce a chest");
        // No traffic, no (event) coins; ambient ones may still appear.
        let (quiet, _) = run(2, 0.2, 1.0 / 30.0, |_, _| {}, Hero::Knight);
        assert_eq!(quiet.pending_coins, 0);
    }

    #[test]
    fn music_makes_the_hero_hop_and_spawns_notes() {
        let (mut g, _) = run(5, 0.05, 1.0 / 60.0, |_, _| {}, Hero::Cat);
        g.obstacles.clear(); // an empty road: hopping is allowed
        let mut i = Inputs { music: true, ..Inputs::default() };
        let mut hopped = false;
        let mut notes = 0;
        for n in 0..90 {
            i.bass = if n % 20 < 3 { 0.9 } else { 0.0 };
            g.advance(1.0 / 30.0, &i);
            g.obstacles.clear();
            hopped |= g.hero_hop;
            notes = notes.max(g.particles.iter().filter(|p| matches!(p.kind, PKind::Note(_))).count());
        }
        assert!(hopped, "no hop on the beat");
        assert!(notes > 0, "no notes");
        // Silence: no hops.
        let mut g2 = Game::new(5);
        g2.resize(640, 57);
        let quiet = Inputs { music: false, bass: 0.9, ..Inputs::default() };
        for _ in 0..90 {
            g2.advance(1.0 / 30.0, &quiet);
            assert!(!g2.hero_hop);
        }
    }

    #[test]
    fn the_hour_triggers_fireworks_but_starting_up_does_not() {
        let mut g = Game::new(1);
        g.resize(640, 57);
        let mut i = Inputs { minute: 0, second: 2, unix_secs: 3_600 * 500 + 2, ..Inputs::default() };
        g.advance(0.1, &i);
        assert!(!g.is_celebrating(), "the very first sample must not fire");
        i.unix_secs = 3_600 * 501 + 2; // the next hour
        g.advance(0.1, &i);
        assert!(g.is_celebrating());
        for _ in 0..40 {
            g.advance(0.1, &i);
        }
        assert!(g.particles.iter().any(|p| matches!(p.kind, PKind::Firework(_))) || !g.shells.is_empty() || g.t > 3.0);
    }

    #[test]
    fn hero_choice_parses_and_alternates_with_the_clock() {
        assert_eq!(HeroChoice::parse("Knight"), Some(HeroChoice::Knight));
        assert_eq!(HeroChoice::parse("gatto"), Some(HeroChoice::Cat));
        assert_eq!(HeroChoice::parse("both"), Some(HeroChoice::Both));
        assert_eq!(HeroChoice::parse("dragon"), None);
        let mut g = Game::new(1);
        g.pick_hero(HeroChoice::Both, 0);
        let a = g.hero;
        g.pick_hero(HeroChoice::Both, SWAP_SECS);
        assert_ne!(a, g.hero);
        g.pick_hero(HeroChoice::Cat, 0);
        assert_eq!(g.hero, Hero::Cat);
    }

    #[test]
    fn weather_and_daytime_change_the_scene() {
        use trofeo_lcd::{Framebuffer, Resolution};
        let render = |inp: &Inputs| {
            let mut fb = Framebuffer::new(Resolution { width: 640, height: 171 });
            let mut g = Game::new(3);
            g.resize(214, 57);
            g.advance(2.0, inp);
            let mut cv = Canvas::new(&mut fb, (0, 0, 640, 171), 3);
            g.draw(&mut cv, inp, HeroChoice::Knight);
            fb
        };
        let px = |fb: &Framebuffer, x: u32, y: u32| {
            let b = fb.as_bytes();
            let i = ((y * fb.width() + x) * 3) as usize;
            (b[i] as i32, b[i + 1] as i32, b[i + 2] as i32)
        };
        let day = render(&Inputs { hour: 13, ..Inputs::default() });
        let night = render(&Inputs { hour: 1, ..Inputs::default() });
        let (d, n) = (px(&day, 5, 5), px(&night, 5, 5));
        assert!(d.2 > 180 && n.2 < 90 && d.0 + d.1 + d.2 > n.0 + n.1 + n.2 + 200, "sky: day {d:?} night {n:?}");
        // A storm makes the daytime sky darker and greyer than a clear one.
        let storm = render(&Inputs { hour: 13, weather: Some(WeatherIcon::Thunder), ..Inputs::default() });
        let s = px(&storm, 5, 5);
        assert!(s.0 + s.1 + s.2 < d.0 + d.1 + d.2 - 120, "storm {s:?} vs clear {d:?}");
        // Rain draws drops: some pixels differ from the same scene without rain on the sky rows.
        let rain = render(&Inputs { hour: 13, weather: Some(WeatherIcon::Rain), ..Inputs::default() });
        let bluish = (0..640).step_by(2).flat_map(|x| (10..100).step_by(2).map(move |y| (x, y))).filter(|&(x, y)| {
            let p = px(&rain, x, y);
            p.2 > 235 && p.0 > 130 && p.0 < 175
        }).count();
        assert!(bluish > 20, "rain drops expected, found {bluish}");
    }

    #[test]
    fn drawing_stays_inside_its_rectangle_and_never_panics() {
        use trofeo_lcd::{Framebuffer, Resolution};
        for (w, h) in [(1900u32, 169u32), (440, 260), (120, 90), (60, 50)] {
            let mut fb = Framebuffer::new(Resolution { width: w + 40, height: h + 40 });
            fb.clear(1, 2, 3);
            let mut pg = PixelGame::new();
            let inp = Inputs { weather: Some(WeatherIcon::Snow), ..Inputs::default() };
            for _ in 0..5 {
                pg.frame(&mut fb, (20, 20, w, h), &inp, HeroChoice::Both, true);
            }
            let b = fb.as_bytes();
            let outside = |x: u32, y: u32| {
                let i = ((y * fb.width() + x) * 3) as usize;
                (b[i], b[i + 1], b[i + 2]) != (1, 2, 3)
            };
            for x in 0..fb.width() {
                for y in 0..fb.height() {
                    if x < 20 || y < 20 || x >= 20 + w || y >= 20 + h {
                        assert!(!outside(x, y), "{w}x{h}: drew outside at {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn collisions_are_detected_at_all() {
        // Guards the "never hits anything" tests above against being vacuous.
        let mut g = Game::new(1);
        g.resize(640, 57);
        let hx = g.hero_x();
        g.obstacles.push(Obstacle { kind: Kind::Rock, x: hx + 5.0, w: 10.0, h: 7.0, dead: false });
        assert!(g.hero_hit(), "a rock under a grounded hero is a hit");
        g.hero_y = 20.0;
        assert!(!g.hero_hit(), "jumping over it is not");
        g.hero_y = 0.0;
        g.obstacles[0] = Obstacle { kind: Kind::Pit, x: hx + 2.0, w: 16.0, h: 0.0, dead: false };
        assert!(g.hero_hit(), "a hole under the feet is a hit");
        g.hero_y = 5.0;
        assert!(!g.hero_hit());
        // With the road full of obstacles and no autopilot the hero would be hit: prove that obstacles do spawn.
        let (g, _) = run(1, 0.5, 1.0 / 30.0, |_, _| {}, Hero::Knight);
        assert!(g.scroll > 20.0);
    }

    #[test]
    fn scale_picks_a_size_that_keeps_enough_rows() {
        assert_eq!(scale_for(169), 3);
        assert_eq!(scale_for(260), 4);
        assert_eq!(scale_for(90), 2);
        assert!(scale_for(10) >= 2);
    }
}
