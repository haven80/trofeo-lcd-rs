//! Audio visualizer (bar EQ) + system info for the Trofeo Vision 9.16 LCD.
//!
//! Displays:
//! - EQ bars from audio currently playing on the computer (loopback — not
//!   the microphone). Actually implemented on **Windows** (WASAPI) and **Linux**
//!   (PulseAudio/PipeWire) — see `src/audio.rs`; on other OSes a synthetic
//!   source is used so the code still compiles & can be tested, but it's not real audio.
//! - Info line: CPU usage, RAM used/total, system uptime, clock, date.
//!
//! FPS is **adaptive**: it drops to the idle FPS (default 2) when no
//! sound is detected (saving CPU — JPEG encoding + USB sending is the biggest
//! CPU cost in this program), and rises to the active FPS (default 15) as soon as
//! sound is present again. All these values + the "silence" threshold can be set via
//! command-line arguments — run with `--help` for the full list.
//!
//! Other quick tuning knobs are in the `NUM_BARS`, `FFT_SIZE`, etc. constants below.

mod audio;
mod cpu_freq;
mod cpu_sensor;
mod deepcool;
mod foreground;
mod fps_etw;
mod gpu_nvml;
mod layouts;
mod gpu;
mod gpu_amd;
mod media;
mod netdisk;
mod openrgb_sync;
mod pawnio;
mod weather;
mod weather_icon;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;
use sysinfo::System;
use trofeo_lcd::background::Background;
use trofeo_lcd::config::{ConfigFile, Margins};
use trofeo_lcd::i18n::Lang;
use trofeo_lcd::layout::{Anchor, BgLayout, Fit, Show};
use trofeo_lcd::i18n;
use trofeo_lcd::{Framebuffer, LyLcd, Orientation};
use trofeo_lcd::{hotkey, png_save};

/// Number of EQ bars drawn.
const NUM_BARS: usize = 48;
/// FFT window size (samples). Larger = finer frequency resolution,
/// but the window responds more slowly (higher latency).
const FFT_SIZE: usize = 1024;
/// Frequency range mapped to bars (Hz). Outside this range is ignored.
const FREQ_MIN: f32 = 40.0;
const FREQ_MAX: f32 = 16_000.0;

/// Status text scale (used for the CPU/GPU/NET/DISK/VOL & now-playing lines).
const STATUS_TEXT_SCALE: u32 = 3;
/// Scroll speed for an overly long "now playing" text, in pixels/second.
const MARQUEE_SPEED_PX_S: f32 = 45.0;
/// Empty gap between text repetitions while scrolling (so it looks like a
/// continuous running text, instead of butting right up against the next repetition).
const MARQUEE_GAP: &str = "     ";

/// Scroll (marquee) state for a "now playing" text too long to fit
/// the screen width. When the song title changes, it automatically resets to the start
/// position — and if the title fits without scrolling, it stays idle (offset always 0).
struct Marquee {
    text: String,
    offset_px: f32,
    last_tick: Instant,
    /// Text scale (used to measure width).
    scale: u32,
}

impl Marquee {
    fn new() -> Self {
        Self {
            text: String::new(),
            offset_px: 0.0,
            last_tick: Instant::now(),
            scale: STATUS_TEXT_SCALE,
        }
    }

    /// Call every frame before drawing, with the text to display
    /// right now (e.g. the latest song title) and the width of the area available to
    /// display it (pixels). Returns `true` if it needs to scroll
    /// (text wider than the area), `false` if it can just be drawn statically.
    fn tick(&mut self, current_text: &str, available_width: u32) -> bool {
        if current_text != self.text {
            self.text = current_text.to_string();
            self.offset_px = 0.0;
            self.last_tick = Instant::now();
        }

        let text_width = Framebuffer::text_width(&self.text, self.scale);
        if text_width <= available_width {
            self.offset_px = 0.0;
            return false;
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;

        let loop_text = format!("{}{}", self.text, MARQUEE_GAP);
        let loop_width = Framebuffer::text_width(&loop_text, self.scale).max(1) as f32;
        self.offset_px = (self.offset_px + MARQUEE_SPEED_PX_S * dt) % loop_width;
        true
    }
}

/// Default CLI argument values (see `Config` & `parse_args`) — all can
/// be overridden via `--idle-fps`, `--active-fps`, etc.
const DEFAULT_IDLE_FPS: f32 = 2.0;
const DEFAULT_ACTIVE_FPS: f32 = 15.0;
/// Peak time-domain amplitude threshold (0.0-1.0) to be considered "silent".
/// Pure digital audio (not microphone noise) is usually exactly 0
/// when silent, so this small threshold is mainly a safeguard against
/// very small noise/DC offset from the WASAPI capture path.
const DEFAULT_SILENCE_THRESHOLD: f32 = 0.005;
/// How long it must stay CONTINUOUSLY silent before dropping to `idle-fps` — so it
/// doesn't "flicker" between FPS levels during a short gap between songs/sounds.
/// Rising to `active-fps`, on the other hand, is ALWAYS immediate (no delay) as soon as
/// there's sound, to keep the visualizer responsive.
const DEFAULT_SILENCE_TIMEOUT_MS: u64 = 800;
/// How often system info (CPU/mem, fairly expensive) is refreshed.
const SYSINFO_REFRESH_INTERVAL: Duration = Duration::from_millis(500);

/// Manual screen rotation override. Set to `true` if the display on your
/// screen is upside-down; `false` if it's already correct. The
/// `Handshake::rotate_180` field is now ALWAYS `false` (its automatic heuristic
/// proved unreliable on real hardware) — so this is the only
/// place to set the rotation.
const ROTATE_180_OVERRIDE: bool = false;

/// EQ bar color mode: the default gradient (green->yellow->red based on
/// level), or a single fixed custom color (its brightness still follows the
/// sound level so the visual dynamics aren't lost).
#[derive(Clone, Copy, Debug)]
enum ColorMode {
    Default,
    Custom(u8, u8, u8),
}

/// Default OpenRGB color polling interval (see `--openrgb-poll-ms`).
const DEFAULT_OPENRGB_POLL_MS: u64 = 300;

/// Default interval for sending data to the DeepCool display (see
/// `--deepcool-update-ms`).
const DEFAULT_DEEPCOOL_UPDATE_MS: u64 = 1000;

/// Configuration from command-line arguments (see `parse_args`).
struct Config {
    idle_fps: f32,
    active_fps: f32,
    silence_threshold: f32,
    silence_timeout: Duration,
    color_mode: ColorMode,
    /// If `Some`, the EQ bar color follows (by polling) the color of the OpenRGB
    /// device whose name contains this string — see `src/openrgb_sync.rs`.
    /// The `color_mode` above is used as a fallback until there's been
    /// a first successful read (OpenRGB not running yet / device not
    /// found yet).
    openrgb_device: Option<String>,
    openrgb_poll_ms: u64,
    /// If `true`, the terminal window is hidden (`FreeConsole`) as soon as
    /// arguments finish parsing — used to run from Task Scheduler/a
    /// shortcut without showing a window. Does not write logs to any file.
    /// Only applies on Windows — ignored on other OSes (with a warning).
    hide_console: bool,
    /// DeepCool Digital integration (sends CPU data to the DeepCool
    /// cooler/case display via HID) — enabled by default, can be disabled with
    /// `--no-deepcool`.
    deepcool_enabled: bool,
    /// Interval for sending data to the DeepCool display, ms (clamped to 100-2000).
    deepcool_update_ms: u64,
    /// Global hotkey to save an LCD screenshot as a PNG to the Desktop
    /// — (virtual-key code, raw label from the argument). `None` = DISABLED
    /// (default); enabled only if `--screenshot-key` is given.
    screenshot_key: Option<(u32, String)>,
    /// `landscape` (default) or `portrait` (screen mounted upright) —
    /// from `--orientation` or `orientation = ...` in trofeo.conf.
    orientation: Orientation,
    /// Extra 180° rotation (`--flip` / `flip = true`), if the result comes out upside down.
    flip: bool,
    /// Margins (px) to keep the interface away from the screen edges.
    margins: Margins,
    language: Lang,
    /// Background: an image (jpg/png/bmp), an animated gif, or a video (requires ffmpeg).
    background: Option<std::path::PathBuf>,
    /// How much to darken the background, 0-100 (for better text readability).
    background_dim: u8,
    /// Path to ffmpeg (default: next to the program, then PATH).
    ffmpeg: Option<String>,
    /// Brightness 0-100.
    brightness: u8,
    /// FPS via ETW (admin), for any GPU.
    fps_monitor: bool,
    /// Explicit config file (`--config`), for hot reloading.
    config_explicit: Option<std::path::PathBuf>,
    /// City name for the weather module (`--weather-city` / `weather_city = ...`).
    /// `None` = auto-detect the location from the machine's public IP address.
    /// Changing it requires a restart (it re-spawns the background fetch thread).
    weather_city: Option<String>,
    /// Background position/fit.
    bg_layout: BgLayout,
    ui: UiOptions,
}

fn print_help() {
    println!(
        "Usage: trofeo_lcd [OPTIONS]\n\
         \n\
         The screen's send FPS is adaptive: it drops to --idle-fps when no\n\
         sound is detected, and rises to --active-fps as soon as there's sound again.\n\
         \n\
         Options:\n\
         \x20\x20--idle-fps <N>            FPS while idle (default: {DEFAULT_IDLE_FPS})\n\
         \x20\x20--active-fps <N>          FPS while there's sound (default: {DEFAULT_ACTIVE_FPS})\n\
         \x20\x20--silence-threshold <N>   Peak amplitude threshold (0.0-1.0) to be\n\
         \x20\x20                          considered silent (default: {DEFAULT_SILENCE_THRESHOLD})\n\
         \x20\x20--silence-timeout-ms <N>  How long it must stay silent before dropping to\n\
         \x20\x20                          idle-fps, in milliseconds (default: {DEFAULT_SILENCE_TIMEOUT_MS})\n\
         \x20\x20--color <MODE>            EQ bar color: 'default' (green->\n\
         \x20\x20                          yellow->red gradient, this is the default value), or a\n\
         \x20\x20                          fixed custom single color in the format\n\
         \x20\x20                          '#RRGGBB' or 'R,G,B' (e.g. '--color red',\n\
         \x20\x20                          '--color #ff0000', or '--color 255,0,0')\n\
         \x20\x20--openrgb-device <NAME>   Sync the EQ bar color with the color of the OpenRGB\n\
         \x20\x20                          device whose name contains <NAME> (partial,\n\
         \x20\x20                          case-insensitive match; see the exact\n\
         \x20\x20                          name in the left panel of the OpenRGB app).\n\
         \x20\x20                          Requires OpenRGB running + SDK Server enabled\n\
         \x20\x20                          (Settings > SDK Server > Enable). Until it's\n\
         \x20\x20                          connected/the device is found, --color\n\
         \x20\x20                          (or the default) is used as a fallback. This is POLLING\n\
         \x20\x20                          (periodically reads a color snapshot), not\n\
         \x20\x20                          registering trofeo-lcd as an OpenRGB device —\n\
         \x20\x20                          animation effects on the source device won't be reflected smoothly.\n\
         \x20\x20--openrgb-poll-ms <N>     OpenRGB polling interval in ms\n\
         \x20\x20                          (default: {DEFAULT_OPENRGB_POLL_MS})\n\
         \x20\x20--no-deepcool              Disable DeepCool integration (sends CPU data to\n\
         \x20\x20                          the DeepCool cooler/case display via HID).\n\
         \x20\x20                          Default: ENABLED.\n\
         \x20\x20--deepcool-update-ms <N>   Interval for sending data to the DeepCool display, ms\n\
         \x20\x20                          (100-2000; default: {DEFAULT_DEEPCOOL_UPDATE_MS})\n\
         \x20\x20--orientation <O>       'landscape' (default) or 'portrait' (screen mounted upright)\n\
         \x20\x20--flip                   Extra 180° rotation (if the display comes out upside down)\n\
         \x20\x20--margin <PX>            Keep the interface away from the edges (all sides);\n\
         \x20\x20--margin-top/-bottom/-left/-right <PX>  margin for a single side\n\
         \x20\x20--language <en|it>       Interface language (default: en)\n\
         \x20\x20--background <FILE>      Background: jpg/png/bmp, animated gif, or video\n\
         \x20\x20                          (videos require ffmpeg)\n\
         \x20\x20--background-dim <0-100> Darkens the background (default: 40)\n\
         \x20\x20--ffmpeg <PATH>      Path to ffmpeg (default: next to the exe/PATH)\n\
         \x20\x20--weather-city <NAME>    City for the weather module (default: auto-detect\n\
         \x20\x20                          from the machine's public IP address)\n\
         \x20\x20--show <LIST>           Show ONLY these elements (comma-separated):\n\
         \x20\x20                          cpu, gpu, uptime, time, date, mem, net, disk, volume,\n\
         \x20\x20                          nowplaying, weather, spectrum, clock, clock_date, dashboard\n\
         \x20\x20--hide <LIST>           Hides these elements (same names)\n\
         \x20\x20--text-color <COLOR>    Text color ('#RRGGBB', 'R,G,B' or a name)\n\
         \x20\x20--clock-color <COLOR>   Large clock color\n\
         \x20\x20--status-position <P>    Info block position; P = top-left (default), top,\n\
         \x20\x20                          top-right, center-left, center, center-right,\n\
         \x20\x20                          bottom-left, bottom, bottom-right\n\
         \x20\x20--clock-position <P>     Large clock position (default: center)\n\
         \x20\x20--spectrum-position <P>  Spectrum position (default: center)\n\
         \x20\x20--spectrum-width/-height <PERCENT>  Spectrum size (default: 100)\n\
         \x20\x20--status-style <auto|lines|list>  Info block as long lines or as a list\n\
         \x20\x20--background-fit <cover|stretch|contain|original>  Background fit\n\
         \x20\x20--background-position <P>  Background anchor (default: center)\n\
         \x20\x20--background-offset-x/-y <PX>  Shift the background (+ = right/down)\n\
         \x20\x20--brightness <0-100>    Panel brightness (default: 100)\n\
         \x20\x20--deepcool <true|false>  Enable/disable DeepCool control (same as --no-deepcool)\n\
         \x20\x20--fps-monitor <true|false>  Game FPS via ETW, requires admin (default: true)\n\
         \x20\x20--layout <NAME>          Preset layout with large panels ('--layout list' for the list).\n\
         \x20\x20                          Several separated by commas rotate; each can set its own\n\
         \x20\x20                          duration with 'name:seconds' (e.g. 'default:30,weather:5')\n\
         \x20\x20--layout-spectrum <true|false>  With a layout, still show the spectrum when music is playing\n\
         \x20\x20--config <FILE>          Config file (default: trofeo.conf in the program's\n\
         \x20\x20                          folder or the working folder)\n\
         \x20\x20--hide-console            Hide the terminal window as soon as the program\n\
         \x20\x20                          starts running. Handy when run via a shortcut/\n\
         \x20\x20                          Task Scheduler at login. Does not write logs\n\
         \x20\x20                          to a file. Only applies on Windows.\n\
         \x20\x20-k, --screenshot-key <KEY>  Global hotkey to save an LCD frame\n\
         \x20\x20                          screenshot as a PNG to the Desktop\n\
         \x20\x20                          (f1-f12 or printscreen). Default: DISABLED.\n\
         \x20\x20-h, --help                Show this help"
    );
}

/// Common color names that can be used directly without knowing the hex/RGB code.
fn named_color(name: &str) -> Option<(u8, u8, u8)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "red" | "merah" | "rosso" => (0xFF, 0x00, 0x00),
        "green" | "hijau" | "verde" => (0x00, 0xFF, 0x00),
        "blue" | "biru" | "blu" => (0x00, 0x00, 0xFF),
        "yellow" | "kuning" | "giallo" => (0xFF, 0xFF, 0x00),
        "cyan" | "azzurro" | "ciano" => (0x00, 0xFF, 0xFF),
        "magenta" | "pink" | "rosa" => (0xFF, 0x00, 0xFF),
        "white" | "putih" | "bianco" => (0xFF, 0xFF, 0xFF),
        "orange" | "oranye" | "arancione" => (0xFF, 0xA5, 0x00),
        "purple" | "ungu" | "viola" => (0x80, 0x00, 0x80),
        _ => return None,
    })
}

/// Parse the `--color` argument: `"default"`, a common color name (`"red"`, `"merah"`,
/// etc.), `"#RRGGBB"`, or `"R,G,B"`.
fn parse_color(raw: &str) -> anyhow::Result<ColorMode> {
    let raw = raw.trim();

    if raw.eq_ignore_ascii_case("default") {
        return Ok(ColorMode::Default);
    }

    if let Some((r, g, b)) = named_color(raw) {
        return Ok(ColorMode::Custom(r, g, b));
    }

    if let Some(hex) = raw.strip_prefix('#') {
        if hex.len() == 6 {
            let parse_byte = |s: &str| {
                u8::from_str_radix(s, 16)
                    .map_err(|_| anyhow::anyhow!("--color: '{raw}' is not valid hex RGB"))
            };
            let r = parse_byte(&hex[0..2])?;
            let g = parse_byte(&hex[2..4])?;
            let b = parse_byte(&hex[4..6])?;
            return Ok(ColorMode::Custom(r, g, b));
        }
        anyhow::bail!("--color: '{raw}' must be formatted as '#RRGGBB' (6 hex digits)");
    }

    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if parts.len() == 3 {
        let parse_component = |s: &str| {
            s.parse::<u8>()
                .map_err(|_| anyhow::anyhow!("--color: '{raw}' is not a valid 'R,G,B' format (each component 0-255)"))
        };
        let r = parse_component(parts[0])?;
        let g = parse_component(parts[1])?;
        let b = parse_component(parts[2])?;
        return Ok(ColorMode::Custom(r, g, b));
    }

    anyhow::bail!(
        "--color: '{raw}' not recognized (use 'default', a color name like 'red', \
         '#RRGGBB', or 'R,G,B')"
    );
}

/// Parse command-line arguments, filling in defaults where not given.
fn parse_args() -> anyhow::Result<Config> {
    let mut idle_fps = DEFAULT_IDLE_FPS;
    let mut active_fps = DEFAULT_ACTIVE_FPS;
    // Also settable from the config file (see below, near `file.get_f32`) —
    // the CLI flags stay as the highest-priority override.
    let mut silence_threshold_cli: Option<f32> = None;
    let mut silence_timeout_ms_cli: Option<u64> = None;
    let mut color_mode = ColorMode::Default;
    let mut openrgb_device: Option<String> = None;
    let mut openrgb_poll_ms = DEFAULT_OPENRGB_POLL_MS;
    let mut hide_console = false;
    let mut deepcool_enabled = true;
    let mut deepcool_update_ms = DEFAULT_DEEPCOOL_UPDATE_MS;
    let mut screenshot_key: Option<(u32, String)> = None;
    let mut orientation_cli: Option<Orientation> = None;
    let mut flip_cli: Option<bool> = None;
    let mut config_path: Option<std::path::PathBuf> = None;
    let mut margin_all: Option<u32> = None;
    let (mut m_top, mut m_bottom, mut m_left, mut m_right) = (None, None, None, None);
    let mut language_cli: Option<Lang> = None;
    let mut background_cli: Option<std::path::PathBuf> = None;
    let mut background_dim_cli: Option<u32> = None;
    let mut ffmpeg_cli: Option<String> = None;
    let mut weather_city_cli: Option<String> = None;
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut color_set = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--idle-fps" => idle_fps = next_f32(&mut args, "--idle-fps")?,
            "--active-fps" => active_fps = next_f32(&mut args, "--active-fps")?,
            "--silence-threshold" => {
                silence_threshold_cli = Some(next_f32(&mut args, "--silence-threshold")?)
            }
            "--silence-timeout-ms" => {
                silence_timeout_ms_cli = Some(next_u64(&mut args, "--silence-timeout-ms")?)
            }
            "--color" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--color needs a value after it"))?;
                color_mode = parse_color(&raw)?;
                color_set = true;
            }
            "--openrgb-device" => {
                let raw = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--openrgb-device needs a value after it")
                })?;
                if raw.trim().is_empty() {
                    anyhow::bail!("--openrgb-device cannot be empty");
                }
                openrgb_device = Some(raw);
            }
            "--openrgb-poll-ms" => {
                openrgb_poll_ms = next_u64(&mut args, "--openrgb-poll-ms")?;
            }
            "--no-deepcool" => deepcool_enabled = false,
            "--deepcool-update-ms" => {
                deepcool_update_ms = next_u64(&mut args, "--deepcool-update-ms")?;
            }
            "--hide-console" => hide_console = true,
            "--flip" => flip_cli = Some(true),
            "--margin" => margin_all = Some(next_u64(&mut args, "--margin")? as u32),
            "--margin-top" => m_top = Some(next_u64(&mut args, "--margin-top")? as u32),
            "--margin-bottom" => m_bottom = Some(next_u64(&mut args, "--margin-bottom")? as u32),
            "--margin-left" => m_left = Some(next_u64(&mut args, "--margin-left")? as u32),
            "--margin-right" => m_right = Some(next_u64(&mut args, "--margin-right")? as u32),
            "--language" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--language requires 'en' or 'it'"))?;
                language_cli = Some(Lang::parse(&raw).ok_or_else(|| {
                    anyhow::anyhow!("--language: '{raw}' not valid (en | it)")
                })?);
            }
            "--background" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--background requires a file path"))?;
                background_cli = Some(raw.into());
            }
            "--background-dim" => {
                background_dim_cli = Some(next_u64(&mut args, "--background-dim")? as u32)
            }
            "--ffmpeg" => {
                ffmpeg_cli = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--ffmpeg requires a path"))?,
                )
            }
            "--weather-city" => {
                weather_city_cli = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--weather-city requires a city name"))?,
                )
            }
            "--diag" => {
                // Handled in `main()` after parsing, so it can see `weather_city`;
                // here it just needs to not be rejected as an unrecognized argument.
            }
            "--orientation" => {
                let raw = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--orientation requires 'landscape' or 'portrait'")
                })?;
                orientation_cli = Some(Orientation::parse(&raw).ok_or_else(|| {
                    anyhow::anyhow!("--orientation: '{raw}' not valid (landscape | portrait)")
                })?);
            }
            "--config" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--config requires a file path"))?;
                config_path = Some(raw.into());
            }
            "-k" | "--screenshot-key" => {
                let raw = args.next().ok_or_else(|| {
                    anyhow::anyhow!("--screenshot-key needs a key name (f1-f12, printscreen)")
                })?;
                screenshot_key = Some((hotkey::parse_key_name(&raw)?, raw.trim().to_ascii_lowercase()));
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other if is_override_key(&other.trim_start_matches("--").replace('-', "_")) => {
                let key = other.trim_start_matches("--").replace('-', "_");
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{other} requires a value"))?;
                if key == "layout" && raw.eq_ignore_ascii_case("list") {
                    println!("Available layouts (--layout NAME, or layout = NAME in trofeo.conf):");
                    for l in layouts::LAYOUTS {
                        println!("  {:<13} {}", l.name, l.description);
                    }
                    std::process::exit(0);
                }
                overrides.push((key, raw));
            }
            other => {
                anyhow::bail!("unrecognized argument: '{other}' (use --help for the list of options)");
            }
        }
    }

    if !(idle_fps > 0.0) || !(active_fps > 0.0) {
        anyhow::bail!("--idle-fps and --active-fps must be numbers > 0");
    }
    if openrgb_poll_ms == 0 {
        anyhow::bail!("--openrgb-poll-ms must be > 0");
    }
    if deepcool_update_ms == 0 {
        anyhow::bail!("--deepcool-update-ms must be > 0");
    }

    // Priority: defaults < config file < command line.
    let mut file = ConfigFile::load(config_path.as_deref()).map_err(|e| anyhow::anyhow!(e))?;
    for (k, v) in &overrides {
        file.set(k, v);
    }
    if !color_set {
        if let Some(v) = file.get("color") {
            color_mode = parse_color(v)?;
        }
    }
    let brightness = file.get_u32("brightness").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(100);
    if brightness > 100 {
        anyhow::bail!("brightness must be between 0 and 100");
    }
    // Peak amplitude (0.0-1.0) below which audio is considered "silent" (EQ
    // bars stay down, FPS drops to idle). Loopback capture reflects the
    // system/app volume, so a quiet listening level can dip under the
    // default threshold and look like silence — lower this if the EQ bars
    // don't react at low volume.
    let silence_threshold = match silence_threshold_cli {
        Some(v) => v,
        None => file.get_f32("silence_threshold").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(DEFAULT_SILENCE_THRESHOLD),
    };
    if !(0.0..=1.0).contains(&silence_threshold) {
        anyhow::bail!("silence_threshold: must be between 0.0 and 1.0");
    }
    let silence_timeout_ms = match silence_timeout_ms_cli {
        Some(v) => v,
        None => file.get_u32("silence_timeout_ms").map_err(|e| anyhow::anyhow!(e))?.map(u64::from).unwrap_or(DEFAULT_SILENCE_TIMEOUT_MS),
    };
    if let Some(v) = file.get_bool("deepcool").map_err(|e| anyhow::anyhow!(e))? {
        // `--no-deepcool` on the command line always wins.
        deepcool_enabled = deepcool_enabled && v;
    }
    let fps_monitor = file.get_bool("fps_monitor").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(true);
    let ui = parse_ui_options(&file)?;
    let bg_layout = parse_bg_layout(&file)?;
    let orientation = match orientation_cli {
        Some(o) => o,
        None => file.orientation().map_err(|e| anyhow::anyhow!(e))?.unwrap_or_default(),
    };
    let flip = match flip_cli {
        Some(f) => f,
        None => file.get_bool("flip").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false),
    };
    if let Some(p) = &file.path {
        println!("Config file: {}", p.display());
    }
    let mut margins = file.margins().map_err(|e| anyhow::anyhow!(e))?;
    if let Some(v) = margin_all {
        margins = Margins { top: v, bottom: v, left: v, right: v };
    }
    margins.top = m_top.unwrap_or(margins.top);
    margins.bottom = m_bottom.unwrap_or(margins.bottom);
    margins.left = m_left.unwrap_or(margins.left);
    margins.right = m_right.unwrap_or(margins.right);
    let language = match language_cli {
        Some(l) => l,
        None => match file.get("language") {
            None => Lang::default(),
            Some(v) => Lang::parse(v)
                .ok_or_else(|| anyhow::anyhow!("language: '{v}' not valid (en | it)"))?,
        },
    };
    let background = background_cli.or_else(|| file.get("background").map(Into::into));
    let background_dim = match background_dim_cli {
        Some(v) => v,
        None => file.get_u32("background_dim").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(40),
    };
    if background_dim > 100 {
        anyhow::bail!("--background-dim must be between 0 and 100");
    }
    let ffmpeg = ffmpeg_cli.or_else(|| file.get("ffmpeg").map(str::to_string));
    let weather_city = weather_city_cli.or_else(|| file.get("weather_city").map(str::to_string));

    Ok(Config {
        config_explicit: config_path.clone(),
        weather_city,
        idle_fps,
        active_fps,
        silence_threshold,
        silence_timeout: Duration::from_millis(silence_timeout_ms),
        color_mode,
        openrgb_device,
        openrgb_poll_ms,
        hide_console,
        deepcool_enabled,
        deepcool_update_ms,
        screenshot_key,
        orientation,
        flip,
        margins,
        language,
        background,
        background_dim: background_dim as u8,
        ffmpeg,
        brightness: brightness as u8,
        fps_monitor,
        bg_layout,
        ui,
    })
}

/// Hide the terminal window (`FreeConsole`) — the window disappears, but
/// `stdout`/`stderr` are NOT redirected to a log file (no file gets
/// written). MUST be called before `println!`/`eprintln!` are used for the
/// first time in this program, so Rust hasn't yet "memorized" the old
/// console handle (stdout/stderr are lazily cached on first use, before
/// that they can still be redirected).
/// Keys settable both from `trofeo.conf` and from the command line (`--key value`,
/// with '-' in place of '_').
const OVERRIDE_KEYS: &[&str] = &[
    "show", "hide", "text_color", "clock_color", "status_position", "clock_position",
    "spectrum_position", "spectrum_width", "spectrum_height", "status_style",
    "background_fit", "background_position", "background_offset_x", "background_offset_y",
    "brightness", "deepcool", "fps_monitor", "layout", "layout_spectrum",
    "layout_interval", "panel_opacity", "text_backdrop",
];

/// Network (`net_unit`) and RAM (`mem_unit`) units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetUnit {
    Kb,
    Mb,
    Auto,
}

fn fmt_net(kb: f64, unit: NetUnit) -> String {
    match unit {
        NetUnit::Kb => format!("{kb:.0}KB/S"),
        NetUnit::Mb => format!("{:.1}MB/S", kb / 1024.0),
        NetUnit::Auto => {
            if kb >= 1024.0 { format!("{:.1}MB/S", kb / 1024.0) } else { format!("{kb:.0}KB/S") }
        }
    }
}

fn fmt_mem(used_mb: u64, total_mb: u64, gb: bool) -> String {
    if gb {
        format!("{:.1}/{:.0}GB", used_mb as f64 / 1024.0, total_mb as f64 / 1024.0)
    } else {
        format!("{used_mb}/{total_mb}MB")
    }
}

const ITEM_NAMES: [&str; 11] =
    ["cpu", "gpu", "uptime", "time", "date", "mem", "net", "disk", "volume", "nowplaying", "weather"];

/// `<item>_size|_position|_color` (with `ram` = `mem`): item index + kind.
fn item_key(key: &str) -> Option<(usize, &str)> {
    for kind in ["size", "position", "color", "backdrop"] {
        if let Some(name) = key.strip_suffix(&format!("_{kind}")) {
            let name = if name == "ram" { "mem" } else { name };
            if let Some(i) = ITEM_NAMES.iter().position(|n| *n == name) {
                return Some((i, kind));
            }
        }
    }
    None
}

fn is_override_key(key: &str) -> bool {
    OVERRIDE_KEYS.contains(&key) || item_key(key).is_some() || key == "net_unit" || key == "mem_unit" || key == "ram_unit" || key == "nowplaying_width" || key == "nowplaying_label" || key == "clock_backdrop" || key == "clock_time_size" || key == "clock_date_size" || key == "weather_unit"
}

/// Style for a single item of the info block.
#[derive(Clone, Copy, Debug, Default)]
struct ItemStyle {
    size: Option<u32>,
    pos: Option<Anchor>,
    color: Option<(u8, u8, u8)>,
    /// Panel behind this item (None = follows `text_backdrop`).
    backdrop: Option<bool>,
}

fn parse_size_opt(file: &ConfigFile, key: &str, default: u32) -> anyhow::Result<u32> {
    let max = if key == "clock_time_size" { 60 } else { 30 };
    match file.get(key) {
        None => Ok(default),
        Some(v) => match v.trim().parse::<u32>() {
            Ok(n) if (1..=max).contains(&n) => Ok(n),
            _ => anyhow::bail!("{key}: '{v}' not valid (integer from 1 to {max})"),
        },
    }
}

fn parse_anchor_opt(file: &ConfigFile, key: &str, default: Anchor) -> anyhow::Result<Anchor> {
    match file.get(key) {
        None => Ok(default),
        Some(v) => Anchor::parse(v).ok_or_else(|| {
            anyhow::anyhow!(
                "{key}: '{v}' not valid (top-left, top, top-right, center-left, center, \
                 center-right, bottom-left, bottom, bottom-right)"
            )
        }),
    }
}

fn parse_percent(file: &ConfigFile, key: &str) -> anyhow::Result<u32> {
    let v = file.get_u32(key).map_err(|e| anyhow::anyhow!(e))?.unwrap_or(100);
    if !(1..=100).contains(&v) {
        anyhow::bail!("{key}: must be between 1 and 100");
    }
    Ok(v)
}

fn parse_color_opt(file: &ConfigFile, key: &str) -> anyhow::Result<Option<(u8, u8, u8)>> {
    match file.get(key) {
        None => Ok(None),
        Some(v) => match parse_color(v).map_err(|e| anyhow::anyhow!("{key}: {e}"))? {
            ColorMode::Default => Ok(None),
            ColorMode::Custom(r, g, b) => Ok(Some((r, g, b))),
        },
    }
}

fn parse_ui_options(file: &ConfigFile) -> anyhow::Result<UiOptions> {
    let layout_interval = file.get_u32("layout_interval").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(15);
    if !(2..=3600).contains(&layout_interval) {
        anyhow::bail!("layout_interval: must be between 2 and 3600 seconds");
    }
    // `layout = a` or `layout = a, b, c` (rotates every `layout_interval` seconds,
    // or per-entry with `layout = a:30, b:5` — each name can carry its own
    // ":seconds" so the rotation doesn't have to give every layout equal airtime).
    let mut layouts_sel: Vec<(&'static layouts::LayoutDef, u32)> = Vec::new();
    if let Some(list) = file.get("layout") {
        for raw in list.split(',') {
            let entry = raw.trim();
            if entry.is_empty() || entry.eq_ignore_ascii_case("none") || entry.eq_ignore_ascii_case("off") {
                continue;
            }
            let (name, secs) = match entry.split_once(':') {
                Some((n, s)) => (n.trim(), Some(s.trim())),
                None => (entry, None),
            };
            let def = layouts::find(name).ok_or_else(|| {
                anyhow::anyhow!("layout: '{name}' unknown (valid: {})", layouts::names())
            })?;
            let secs = match secs {
                Some(s) => {
                    let v: u32 = s.parse().map_err(|_| {
                        anyhow::anyhow!("layout: '{name}:{s}' — '{s}' is not a valid number of seconds")
                    })?;
                    if !(2..=3600).contains(&v) {
                        anyhow::bail!("layout: '{name}:{v}' — seconds must be between 2 and 3600");
                    }
                    v
                }
                None => layout_interval,
            };
            layouts_sel.push((def, secs));
        }
    }
    let panel_opacity = file.get_u32("panel_opacity").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(100);
    if panel_opacity > 100 {
        anyhow::bail!("panel_opacity: must be between 0 and 100");
    }
    let layout_spectrum = file.get_bool("layout_spectrum").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false);
    // With a layout: base = everything off (the panels already show the info); the game
    // dashboard is on; `show` ADDS elements, `hide` removes them.
    // `default` only: equivalent to no layout.
    if layouts_sel.len() == 1 && layouts_sel[0].0.name == "default" {
        layouts_sel.clear();
    }
    // `show` = elements of the standard screen; `layout_show` = elements shown while
    // a panel layout is active (base: only the game dashboard; `show` adds, `hide` removes).
    let mut show = Show::default();
    let mut layout_show = Show::all(false);
    layout_show.dashboard = true;
    layout_show.spectrum = layout_spectrum;
    if let Some(list) = file.get("show") {
        show = Show::all(false);
        show.apply_list(list, true).map_err(|e| anyhow::anyhow!("show: {e}"))?;
        layout_show.apply_list(list, true).map_err(|e| anyhow::anyhow!("show: {e}"))?;
    }
    if let Some(list) = file.get("hide") {
        show.apply_list(list, false).map_err(|e| anyhow::anyhow!("hide: {e}"))?;
        layout_show.apply_list(list, false).map_err(|e| anyhow::anyhow!("hide: {e}"))?;
    }
    let status_style = match file.get("status_style").map(|v| v.to_ascii_lowercase()).as_deref() {
        None | Some("auto") => StatusStyle::Auto,
        Some("lines") | Some("righe") => StatusStyle::Lines,
        Some("list") | Some("elenco") => StatusStyle::List,
        Some("items") | Some("voci") => StatusStyle::Items,
        Some(v) => anyhow::bail!("status_style: '{v}' not valid (auto | lines | list)"),
    };
    let mut items = [ItemStyle::default(); 11];
    for (i, name) in ITEM_NAMES.iter().enumerate() {
        let size_key = format!("{name}_size");
        let ram_alias = |k: &str| if *name == "mem" { file.get(&k.replace("mem", "ram")).is_some() } else { false };
        let key_or_alias = |k: String| if file.get(&k).is_none() && ram_alias(&k) { k.replace("mem", "ram") } else { k };
        let size_key = key_or_alias(size_key);
        if file.get(&size_key).is_some() {
            items[i].size = Some(parse_size_opt(file, &size_key, 3)?);
        }
        let pos_key = key_or_alias(format!("{name}_position"));
        if file.get(&pos_key).is_some() {
            items[i].pos = Some(parse_anchor_opt(file, &pos_key, Anchor::TOP_LEFT)?);
        }
        let col_key = key_or_alias(format!("{name}_color"));
        items[i].color = parse_color_opt(file, &col_key)?;
        let bd_key = key_or_alias(format!("{name}_backdrop"));
        items[i].backdrop = file.get_bool(&bd_key).map_err(|e| anyhow::anyhow!("{bd_key}: {e}"))?;
    }
    // Without an explicit `layout`, per-item options = the "default2" screen (compatibility).
    if layouts_sel.is_empty()
        && file.get("layout").is_none()
        && items.iter().any(|it| it.size.is_some() || it.pos.is_some() || it.color.is_some())
    {
        layouts_sel.push((layouts::find("default2").unwrap(), layout_interval));
    }
    Ok(UiOptions {
        items,
        nowplaying_width: parse_percent(file, "nowplaying_width")?,
        net_unit: match file.get("net_unit").map(|v| v.to_ascii_lowercase()).as_deref() {
            None | Some("kb") => NetUnit::Kb,
            Some("mb") => NetUnit::Mb,
            Some("auto") => NetUnit::Auto,
            Some(v) => anyhow::bail!("net_unit: '{v}' not valid (kb | mb | auto)"),
        },
        mem_gb: match file.get("mem_unit").or_else(|| file.get("ram_unit")).map(|v| v.to_ascii_lowercase()).as_deref() {
            None | Some("mb") => false,
            Some("gb") => true,
            Some(v) => anyhow::bail!("mem_unit: '{v}' not valid (mb | gb)"),
        },
        nowplaying_label: file.get_bool("nowplaying_label").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(true),
        weather_fahrenheit: match file.get("weather_unit").map(|v| v.to_ascii_lowercase()).as_deref() {
            None | Some("c") | Some("celsius") => false,
            Some("f") | Some("fahrenheit") => true,
            Some(v) => anyhow::bail!("weather_unit: '{v}' not valid (c | f)"),
        },
        clock_backdrop: file.get_bool("clock_backdrop").map_err(|e| anyhow::anyhow!(e))?,
        clock_time_size: parse_size_opt(file, "clock_time_size", 20)?,
        clock_date_size: parse_size_opt(file, "clock_date_size", 6)?,
        show,
        layout_show,
        status_pos: parse_anchor_opt(file, "status_position", Anchor::TOP_LEFT)?,
        clock_pos: parse_anchor_opt(file, "clock_position", Anchor::CENTER)?,
        spectrum_pos: parse_anchor_opt(file, "spectrum_position", Anchor::CENTER)?,
        spectrum_w: parse_percent(file, "spectrum_width")?,
        spectrum_h: parse_percent(file, "spectrum_height")?,
        status_style,
        text_color: parse_color_opt(file, "text_color")?,
        clock_color: parse_color_opt(file, "clock_color")?,
        layouts: layouts_sel,
        panel_opacity: panel_opacity as u8,
        text_backdrop: file.get_bool("text_backdrop").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false),
    })
}

fn parse_bg_layout(file: &ConfigFile) -> anyhow::Result<BgLayout> {
    let fit = match file.get("background_fit") {
        None => Fit::Cover,
        Some(v) => Fit::parse(v).ok_or_else(|| {
            anyhow::anyhow!("background_fit: '{v}' not valid (cover | stretch | contain | original)")
        })?,
    };
    let int = |key: &str| -> anyhow::Result<i32> {
        match file.get(key) {
            None => Ok(0),
            Some(v) => v.parse::<i32>().map_err(|_| anyhow::anyhow!("{key}: '{v}' is not an integer")),
        }
    };
    Ok(BgLayout {
        fit,
        anchor: parse_anchor_opt(file, "background_position", Anchor::CENTER)?,
        offset: (int("background_offset_x")?, int("background_offset_y")?),
    })
}

#[cfg(windows)]
fn hide_console_window() -> anyhow::Result<()> {
    use windows::Win32::System::Console::FreeConsole;
    unsafe { FreeConsole()? };
    Ok(())
}

fn next_f32(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<f32> {
    let raw = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("{flag} needs a numeric value after it"))?;
    raw.parse::<f32>()
        .map_err(|_| anyhow::anyhow!("{flag}: '{raw}' is not a valid number"))
}

fn next_u64(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<u64> {
    let raw = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("{flag} needs a numeric value after it"))?;
    raw.parse::<u64>()
        .map_err(|_| anyhow::anyhow!("{flag}: '{raw}' is not a valid integer"))
}

/// "Silence" detection: the peak time-domain amplitude (max `|sample|` in
/// this window) is below `threshold`. Computed from raw samples (not the FFT
/// result), so it doesn't depend on the EQ bar's windowing/normalization scale.
fn is_silent(samples: &[f32], threshold: f32) -> bool {
    samples.iter().fold(0f32, |acc, &s| acc.max(s.abs())) < threshold
}

/// Everything that depends on orientation, margins and background (rebuilt on reload).
struct Display {
    resolution: trofeo_lcd::Resolution,
    background: Option<Background>,
    idle_fps_effective: f32,
    composite: bool,
    fb: Framebuffer,
    ui: Framebuffer,
}

const UI_KEY: (u8, u8, u8) = (1, 0, 2);

fn build_display(config: &Config) -> anyhow::Result<Display> {
    let resolution = config.orientation.canvas();
    let (ui_w, ui_h) = config
        .margins
        .inner(resolution.width, resolution.height)
        .map_err(|e| anyhow::anyhow!(e))?;
    let ui_resolution = trofeo_lcd::Resolution::new(ui_w, ui_h);
    let background: Option<Background> = match &config.background {
        Some(path) => match Background::load(
            path,
            resolution,
            config.background_dim,
            config.active_fps,
            config.ffmpeg.as_deref(),
            &config.bg_layout,
        ) {
            Ok(b) => {
                println!("Background: {}", path.display());
                Some(b)
            }
            Err(e) => {
                eprintln!("WARNING: background not loaded: {e}");
                None
            }
        },
        None => None,
    };
    // With an animated background, the idle fps would look choppy: use the active fps instead.
    let idle_fps_effective = if background.as_ref().is_some_and(|b| b.is_animated()) {
        config.idle_fps.max(config.active_fps)
    } else {
        config.idle_fps
    };
    let composite = background.is_some() || !config.margins.is_zero() || config.ui.panel_opacity < 100;
    let fb = Framebuffer::new(resolution);
    let ui = Framebuffer::new(if composite { ui_resolution } else { trofeo_lcd::Resolution::new(1, 1) });
    Ok(Display { resolution, background, idle_fps_effective, composite, fb, ui })
}

/// With the console hidden, errors aren't visible: they're written to `trofeo-errors.txt`
/// next to the program (empty string = delete the file).
fn report_config_error(msg: &str) {
    let path = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("trofeo-errors.txt")));
    if msg.is_empty() {
        if let Some(p) = path {
            let _ = std::fs::remove_file(p);
        }
    } else {
        eprintln!("{msg}");
        if let Some(p) = path {
            let _ = std::fs::write(p, format!("{msg}\r\n"));
        }
    }
}

/// Config file signature, used to detect changes.
fn config_stamp(explicit: Option<&std::path::Path>) -> Option<(std::path::PathBuf, std::time::SystemTime, u64)> {
    let p = ConfigFile::find_path(explicit)?;
    let m = std::fs::metadata(&p).ok()?;
    Some((p, m.modified().ok()?, m.len()))
}

fn main() -> anyhow::Result<()> {
    let diag_mode = std::env::args().any(|a| a == "--diag");
    let mut config = parse_args()?;
    if diag_mode {
        println!("=== NVIDIA GPU Diagnostics ===");
        print!("{}", gpu_nvml::diag());
        let amd = gpu_amd::GpuAmdSensor::new().sample();
        println!("ADL (AMD): temp={:?} power={:?}", amd.temp_edge_c, amd.power_w);
        if let Some(n) = gpu_nvml::NvSensor::new() {
            std::thread::sleep(Duration::from_secs(2));
            println!("Sensor chosen: {} -> {:?}", n.describe(), n.sample());
        } else {
            println!("Sensor chosen: none");
        }
        println!();
        weather::diag(config.weather_city.as_deref());
        return Ok(());
    }

    // MUST be the very first thing after parse_args — before any other
    // println!/eprintln! below this (see this function's docs).
    #[cfg(windows)]
    if config.hide_console {
        hide_console_window()?;
    }
    #[cfg(not(windows))]
    if config.hide_console {
        eprintln!(
            "WARNING: --hide-console only works on Windows, ignored in this build."
        );
    }

    let lcd = LyLcd::open()?;
    let mut hs = lcd.handshake()?;
    hs.rotate_180 = ROTATE_180_OVERRIDE;
    hs.rotation = config.orientation.output_rotation(config.flip);
    hs.brightness = config.brightness;
    println!("Connected: {:?}, PM={} SUB={}", lcd.variant(), hs.pm, hs.sub);
    println!(
        "Orientation: {:?} (canvas {}x{}, rotation {}°)",
        config.orientation,
        config.orientation.canvas().width,
        config.orientation.canvas().height,
        hs.rotation.degrees()
    );
    println!(
        "FPS: idle={:.1} active={:.1} (silence-threshold={} timeout={}ms)",
        config.idle_fps,
        config.active_fps,
        config.silence_threshold,
        config.silence_timeout.as_millis()
    );
    match config.color_mode {
        ColorMode::Default => println!("Color: default (green->yellow->red gradient)"),
        ColorMode::Custom(r, g, b) => {
            println!("Color: custom #{r:02X}{g:02X}{b:02X} (brightness follows level)")
        }
    }

    // If --openrgb-device is given, the color above is only used as a
    // FALLBACK until this poller has successfully connected+read a color for
    // the first time (see src/openrgb_sync.rs for details & limitations).
    let openrgb_color: Option<openrgb_sync::SharedColor> = config.openrgb_device.as_ref().map(|d| {
        println!(
            "OpenRGB: sync active, looking for a device containing '{d}' (polling every {}ms)",
            config.openrgb_poll_ms
        );
        openrgb_sync::spawn(d.clone(), Duration::from_millis(config.openrgb_poll_ms))
    });

    // CPU temperature + power sensor (AMD Zen1-Zen4 only) — PawnIO on Windows,
    // sysfs hwmon (k10temp/amd_energy or zenpower) on Linux.
    // Graceful: if the driver isn't present / the CPU isn't supported, a warning is
    // printed to stderr and execution continues — the data is shown as N/A.
    // Wrapped in `Arc<Mutex<_>>` because it's shared with the DeepCool
    // integration thread (so both screens use a single sensor instance / a single
    // PawnIO handle — instead of opening two).
    let cpu_sensor = Arc::new(Mutex::new(cpu_sensor::CpuSensor::new()));
    // Baseline energy counter before the loop starts — used for the first
    // power draw snapshot on the first sysinfo refresh iteration (see the loop section).
    let mut last_cpu_energy = cpu_sensor.lock().expect("cpu_sensor lock").sample_energy();

    // DeepCool Digital integration: sends CPU temperature/usage/power/frequency to
    // the DeepCool cooler/case display via HID, run on a background
    // thread. If the device isn't found, the thread just retries every few
    // seconds without disrupting the main loop.
    if config.deepcool_enabled {
        println!(
            "DeepCool: active, interval {}ms (disable with --no-deepcool)",
            config.deepcool_update_ms
        );
        deepcool::spawn(
            Arc::clone(&cpu_sensor),
            deepcool::Options {
                update_ms: config.deepcool_update_ms,
            },
        );
    }

    // Real-time CPU frequency reader (PDH on Windows, sysfs cpufreq on
    // Linux) — see cpu_freq.rs. Graceful: if it fails, the info line / CPU
    // panel shows N/A.
    let cpu_freq = cpu_freq::CpuFreq::new();

    // LCD screenshot hotkey (global, DISABLED by default — enabled only
    // if the --screenshot-key argument is given). The frame saved is the
    // last visualizer frame shown on screen.
    let mut snap_hotkey: Option<hotkey::Hotkey> = None;
    if let Some((vk, label)) = config.screenshot_key.clone() {
        match hotkey::register(vk) {
            Ok(h) => {
                println!(
                    "Screenshot hotkey: {} (global) — press it to save the LCD frame as a PNG to the Desktop",
                    label.to_uppercase()
                );
                snap_hotkey = Some(h);
            }
            Err(e) => eprintln!("WARNING: screenshot hotkey not active: {e}"),
        }
    }

    // AMD GPU sensor (Edge temperature, ASIC power, fan RPM) via ADL PMLog.
    // Graceful: if the driver isn't present / the GPU isn't AMD, a warning is printed and execution continues (N/A).
    let gpu_amd = gpu_amd::GpuAmdSensor::new();
    // NVIDIA GPU: temperature/power via NVML (ADL only covers AMD).
    let nvml = gpu_nvml::NvSensor::new();
    if let Some(n) = &nvml {
        println!("NVIDIA GPU: temperature/power via {}", n.describe());
    }
    // FPS for any DirectX game via ETW (requires administrator).
    if config.fps_monitor {
        match fps_etw::start() {
            Ok(()) => println!("FPS: ETW monitor active (DirectX 9-12)"),
            Err(e) => eprintln!("FPS: monitor not active ({e}). Run the program as administrator."),
        }
    }
    let mut latest_gpu_data = gpu_amd::GpuAmdData::default();

    // Weather (Open-Meteo): current temperature/humidity/condition for a city given
    // in the config, or auto-detected from the machine's public IP otherwise. Runs
    // on its own background thread (network calls, refreshed every ~15 minutes) so
    // the render loop never blocks on it.
    let weather_monitor = weather::WeatherMonitor::spawn(config.weather_city.clone());
    let mut latest_weather: Option<weather::WeatherSnapshot> = None;

    let audio_ring = audio::spawn_capture()?;
    #[cfg(not(any(windows, target_os = "linux")))]
    println!(
        "WARNING: this build is not Windows/Linux, so the EQ bar uses a \
         synthetic audio source (not real audio) — see src/audio.rs."
    );

    // Additional monitors: GPU usage, network+disk IO, volume, song/media title.
    // If one of them fails to initialize (e.g. the GPU Engine counter isn't
    // available on this system), the program keeps running — only that info line
    // shows "N/A", the program doesn't exit.
    let mut gpu_monitor = gpu::GpuMonitor::new().ok();
    if gpu_monitor.is_none() {
        eprintln!("WARNING: GPU usage is not available on this system, the info line will show N/A.");
    }
    let mut netdisk_monitor = netdisk::NetDiskMonitor::new();
    let audio_endpoint = media::AudioMonitor::new().ok();
    if audio_endpoint.is_none() {
        eprintln!("WARNING: master volume could not be read, the info line will show N/A.");
    }
    let now_playing = media::spawn_now_playing_watcher()?;
    let mut now_playing_marquee = Marquee::new();

    let mut latest_gpu_percent: Option<f32> = None;
    let mut latest_net_kb = (0.0f64, 0.0f64); // (down, up)
    let mut latest_disk_mb = (0.0f64, 0.0f64); // (read, write)
    let mut latest_volume: Option<(f32, bool)> = None; // (percent, muted)
    let mut latest_cpu_temp: Option<f32> = None; // °C
    let mut latest_cpu_power: Option<f32> = None; // Watts
    let mut latest_cpu_mhz: Option<u32> = None; // real-time frequency

    i18n::set_language(config.language);
    set_ui_options(config.ui.clone());
    let mut ui_layouts = config.ui.layouts.clone();
    let mut layouts_started = Instant::now();
    let mut layout_index = usize::MAX;
    let mut layout_marquee = Marquee::new();
    let Display { mut resolution, mut background, mut idle_fps_effective, mut composite, mut fb, mut ui } =
        build_display(&config)?;
    let _ = resolution;
    let mut config_seen = config_stamp(config.config_explicit.as_deref());
    let mut last_config_check = Instant::now();
    let mut sys = System::new_all();
    sys.refresh_all();
    let mut last_sysinfo_refresh = Instant::now();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let hann = hann_window(FFT_SIZE);
    // FFT bin range per bar (depends on FREQ_MIN/FREQ_MAX/NUM_BARS/FFT_SIZE,
    // all constants) is computed ONCE here, not every frame — avoiding
    // repeated `powf()` calls (transcendental, relatively expensive) in the hot loop.
    let bar_bins = precompute_bar_bins(FFT_SIZE);

    let mut bar_heights = vec![0f32; NUM_BARS];
    let mut running_max: f32 = 1e-6;

    // The framebuffer is allocated ONCE outside the loop and then reused every
    // frame (only `clear()`-ed, not a ~2.6MB Vec reallocation 15x/second) —
    // reducing unnecessary heap churn & page faults.
    // When sound (not silence) was last detected. Initialized to
    // "just now" so the program starts at `active_fps` (a grace period
    // for `silence_timeout`), instead of immediately dropping to `idle_fps` if
    // it happens to be silent at startup.
    let mut last_sound = Instant::now();

    // Anti-spam between two consecutive screenshots (ms) — prevents saving
    // dozens of files while the hotkey is held down.
    let mut last_snap = Instant::now() - Duration::from_millis(500);

    loop {
        let frame_start = Instant::now();

        // Hot reload: if trofeo.conf changes, reapply the settings without restarting.
        if last_config_check.elapsed() >= Duration::from_secs(1) {
            last_config_check = Instant::now();
            let stamp = config_stamp(config.config_explicit.as_deref());
            if stamp != config_seen {
                config_seen = stamp;
                match parse_args() {
                    Ok(new_cfg) => {
                        let restart_needed = new_cfg.deepcool_enabled != config.deepcool_enabled
                            || new_cfg.fps_monitor != config.fps_monitor
                            || new_cfg.openrgb_device != config.openrgb_device
                            || new_cfg.hide_console != config.hide_console
                            || new_cfg.screenshot_key != config.screenshot_key
                            || new_cfg.weather_city != config.weather_city;
                        match build_display(&new_cfg) {
                            Ok(d) => {
                                resolution = d.resolution;
                                background = d.background;
                                idle_fps_effective = d.idle_fps_effective;
                                composite = d.composite;
                                fb = d.fb;
                                ui = d.ui;
                                i18n::set_language(new_cfg.language);
                                set_ui_options(new_cfg.ui.clone());
                                ui_layouts = new_cfg.ui.layouts.clone();
                                layouts_started = Instant::now();
                                layout_index = usize::MAX;
                                layout_marquee = Marquee::new();
                                hs.rotation = new_cfg.orientation.output_rotation(new_cfg.flip);
                                hs.brightness = new_cfg.brightness;
                                println!("Configuration reloaded.");
                                report_config_error("");
                                if restart_needed {
                                    println!("(deepcool, fps_monitor, openrgb, hide_console, screenshot_key and weather_city require a restart)");
                                }
                                config = new_cfg;
                            }
                            Err(e) => report_config_error(&format!("Configuration not applied: {e}")),
                        }
                    }
                    Err(e) => report_config_error(&format!("Configuration not applied (keeping the previous one): {e}")),
                }
            }
        }

        // Screenshot hotkey (global — active even when the window doesn't have focus).
        // `fb` still holds the last displayed frame, so the result matches exactly
        // what's shown on the LCD screen.
        if let Some(h) = &snap_hotkey {
            if hotkey::triggered(h.id) && last_snap.elapsed() >= Duration::from_millis(500) {
                match png_save::save(&fb, "trofeo_lcd") {
                    Ok(p) => println!("Screenshot saved: {}", p.display()),
                    Err(e) => eprintln!("Failed to save screenshot: {e}"),
                }
                last_snap = Instant::now();
            }
        }

        // 1) Grab the latest audio window, check whether it's silent, & compute the spectrum.
        let samples = audio::take_latest(&audio_ring, FFT_SIZE);
        if !is_silent(&samples, config.silence_threshold) {
            last_sound = Instant::now();
        }
        // Without the spectrum (show/hide), it always stays in "idle" mode:
        // no bars, large clock, and reduced fps even during playback.
        // Active layout (rotation): computed first since it changes `show`.
        let mut active_def: Option<&'static layouts::LayoutDef> = None;
        if !ui_layouts.is_empty() {
            // Each layout can carry its own seconds (`layout = default:30, weather:5`):
            // walk the cumulative durations to find which entry the elapsed time
            // (modulo the total cycle length) currently falls into.
            let total: u64 = ui_layouts.iter().map(|(_, secs)| *secs as u64).sum::<u64>().max(1);
            let t = layouts_started.elapsed().as_secs() % total;
            let mut acc = 0u64;
            let mut idx = ui_layouts.len() - 1;
            for (i, (_, secs)) in ui_layouts.iter().enumerate() {
                acc += *secs as u64;
                if t < acc {
                    idx = i;
                    break;
                }
            }
            if idx != layout_index {
                layout_index = idx;
                layout_marquee = Marquee::new();
            }
            let def = ui_layouts[idx].0;
            active_def = (!def.standard).then_some(def);
        }
        ACTIVE_PANEL.store(active_def.is_some(), std::sync::atomic::Ordering::Relaxed);
        ACTIVE_ITEMS.store(
            !ui_layouts.is_empty() && ui_layouts[layout_index.min(ui_layouts.len() - 1)].0.name == "default2",
            std::sync::atomic::Ordering::Relaxed,
        );
        let ui_show = opts().show;
        let is_idle = !ui_show.spectrum || last_sound.elapsed() >= config.silence_timeout;
        let target_fps = if is_idle {
            idle_fps_effective
        } else {
            config.active_fps
        };
        let target_frame_time = Duration::from_secs_f32(1.0 / target_fps);

        let bars = compute_bars(&samples, &hann, fft.as_ref(), &bar_bins, &mut running_max);
        for (h, &target) in bar_heights.iter_mut().zip(bars.iter()) {
            if target > *h {
                *h = target; // attack cepat
            } else {
                *h = *h * 0.75 + target * 0.25; // decay lebih pelan
            }
        }

        // 2) Refresh system info only as often as needed (not every frame).
        // Note: `System::uptime()` does NOT follow this rule — it's a
        // static call that only reads an OS counter (not an expensive
        // process/CPU/RAM snapshot), so it's safe to call every frame in `draw_status_line`.
        if last_sysinfo_refresh.elapsed() >= SYSINFO_REFRESH_INTERVAL {
            // Measure the elapsed time before resetting — used to compute power draw in Watts.
            let sysinfo_delta_ms = last_sysinfo_refresh.elapsed().as_millis() as u64;

            sys.refresh_cpu();
            sys.refresh_memory();
            last_sysinfo_refresh = Instant::now();

            if let Some(gpu) = gpu_monitor.as_mut() {
                latest_gpu_percent = gpu.sample().ok();
            }
            let (down, up, read, write) = netdisk_monitor.sample();
            latest_net_kb = (down, up);
            latest_disk_mb = (read, write);
            if let Some(audio_ep) = audio_endpoint.as_ref() {
                latest_volume = audio_ep.sample().ok();
            }

            // CPU temperature + power (AMD Zen1-Zen4 only, see cpu_sensor.rs).
            let sensor = cpu_sensor.lock().expect("cpu_sensor lock");
            latest_cpu_temp = sensor.get_temp_c();
            latest_cpu_power =
                sensor.calc_power_watts(last_cpu_energy, sysinfo_delta_ms);
            last_cpu_energy = sensor.sample_energy();
            drop(sensor);

            // Real-time CPU frequency (see cpu_freq.rs).
            latest_cpu_mhz = cpu_freq.sample_mhz();

            // Weather: just reads whatever the background thread last fetched
            // (non-blocking); stays `None` until the first fetch succeeds.
            latest_weather = weather_monitor.sample();

            // AMD GPU sensor: Edge temperature, ASIC power, fan RPM via ADL PMLog.
            latest_gpu_data = gpu_amd.sample();
            // NVIDIA takes priority: on a PC with an AMD iGPU, ADL would report the
            // iGPU's power (with no temperature) and would hide the real NVIDIA card.
            if let Some(n) = &nvml {
                let (t, w, c) = n.sample();
                if t.is_some() || w.is_some() {
                    latest_gpu_data.temp_edge_c = t;
                    latest_gpu_data.power_w = w;
                    latest_gpu_data.clock_mhz = c;
                }
            }
            if latest_gpu_data.fps.is_none() {
                #[cfg(windows)]
                if let Some(pid) = foreground::foreground_pid() {
                    latest_gpu_data.fps = fps_etw::fps_for_pid(pid).map(|f| f as i32);
                }
            }
        }
        // "Gaming" mode detection: GPU usage > 50%. Used for two things:
        // swap the "NOW PLAYING" content for the foreground exe name, AND replace the
        // EQ bar/clock area with the performance dashboard (see draw_game_dashboard).
        let gaming_mode = latest_gpu_percent.is_some_and(|p| p > 50.0);

        // The "NOW PLAYING" title is usually the song/media being played, BUT if GPU
        // usage is high (indicating gaming), it's replaced with the name of the
        // .exe program that's currently the foreground window (e.g. the game's
        // own name) — more useful than a background song title while gaming.
        let now_playing_title = if gaming_mode {
            foreground::foreground_exe_name()
                .or_else(|| now_playing.lock().ok().and_then(|g| g.clone()))
        } else {
            now_playing.lock().ok().and_then(|g| g.clone())
        };

        // 3) Draw (reuse the same framebuffer, just `clear()`-ed).
        // While idle, drawing the EQ bar flat is pointless
        // (its content is zero/decaying towards zero) — instead of wasting a blank
        // screen, that area is used for a large digital clock. The small info line above
        // (`draw_status_lines`) is still always shown as usual.
        // This frame's effective color: if OpenRGB sync is active AND a color
        // has already been read successfully at least once, use that; if not (just
        // started / OpenRGB not running yet / device not found yet), fall back to
        // --color / default as usual.
        let color_mode = match &openrgb_color {
            Some(shared) => match *shared.lock().unwrap() {
                Some((r, g, b)) => ColorMode::Custom(r, g, b),
                None => config.color_mode,
            },
            None => config.color_mode,
        };

        // Base: a background (image/gif/video) or a solid color; the interface goes
        // on the `ui` buffer (transparent) when there are margins or a background.
        if composite {
            match background.as_mut() {
                Some(bg) => bg.render_into(&mut fb),
                None => fb.clear(0x08, 0x08, 0x10),
            }
            ui.clear(UI_KEY.0, UI_KEY.1, UI_KEY.2);
        } else {
            fb.clear(0x08, 0x08, 0x10);
        }
        let target: &mut Framebuffer = if composite { &mut ui } else { &mut fb };
        if gaming_mode && ui_show.dashboard {
            draw_game_dashboard(
                target,
                &sys,
                latest_gpu_percent,
                &latest_gpu_data,
                latest_cpu_temp,
                latest_cpu_power,
                latest_cpu_mhz,
                color_mode,
            );
        } else if is_idle {
            if let Some(def) = active_def {
                let wd = layouts::WidgetData {
                    cpu_pct: sys.global_cpu_info().cpu_usage(),
                    cpu_temp: latest_cpu_temp,
                    cpu_power: latest_cpu_power,
                    cpu_mhz: latest_cpu_mhz,
                    gpu_pct: latest_gpu_percent,
                    gpu_temp: latest_gpu_data.temp_edge_c,
                    gpu_power: latest_gpu_data.power_w,
                    fps: latest_gpu_data.fps,
                    used_mb: sys.used_memory() / 1024 / 1024,
                    total_mb: sys.total_memory() / 1024 / 1024,
                    net_kb: latest_net_kb,
                    disk_mb: latest_disk_mb,
                    now_playing: now_playing_title.clone(),
                    weather: latest_weather.clone(),
                };
                layouts::draw_layout(target, def, &wd, color_mode, &mut layout_marquee);
            } else if ui_show.clock {
                draw_idle_clock(target, color_mode);
            }
        } else {
            draw_bars(target, &bar_heights, color_mode);
        }
        draw_status_lines(
            target,
            &sys,
            latest_gpu_percent,
            &latest_gpu_data,
            latest_net_kb,
            latest_disk_mb,
            latest_volume,
            latest_cpu_temp,
            latest_cpu_power,
            latest_cpu_mhz,
            now_playing_title.as_deref(),
            latest_weather.as_ref(),
            &mut now_playing_marquee,
        );
        if composite {
            let panel = (config.ui.panel_opacity < 100)
                .then_some((PANEL_KEY, PANEL_COLOR, config.ui.panel_opacity));
            fb.blit_ui(&ui, config.margins.left, config.margins.top, UI_KEY, panel);
        }

        // 4) Send to the screen.
        lcd.send_framebuffer(&hs, &fb, 75)?;

        // 5) Adjust the pace to approach the current `target_fps` (idle or
        // active — can differ each iteration), without forcing it if it's
        // actually slower than that.
        let elapsed = frame_start.elapsed();
        if elapsed < target_frame_time {
            std::thread::sleep(target_frame_time - elapsed);
        }
    }
}

/// Standard Hann window (reduces spectral leakage before the FFT).
fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = (std::f32::consts::PI * i as f32 / (n - 1) as f32).sin();
            x * x
        })
        .collect()
}

/// Compute the FFT bin range `(bin_lo, bin_hi)` per bar, just once at startup
/// (called outside the main loop). This used to be recomputed every frame
/// inside `compute_bars`, even though the result is always the same as long as `FFT_SIZE`
/// doesn't change — including two `powf()` calls per bar (96 calls per
/// frame for `NUM_BARS = 48`) that were being wastefully repeated in the hot loop.
fn precompute_bar_bins(fft_size: usize) -> Vec<(usize, usize)> {
    let bin_hz = audio::SAMPLE_RATE as f32 / fft_size as f32;
    let nyquist_bin = fft_size / 2;

    (0..NUM_BARS)
        .map(|i| {
            let f_lo = FREQ_MIN * (FREQ_MAX / FREQ_MIN).powf(i as f32 / NUM_BARS as f32);
            let f_hi = FREQ_MIN * (FREQ_MAX / FREQ_MIN).powf((i + 1) as f32 / NUM_BARS as f32);
            let bin_lo = ((f_lo / bin_hz) as usize).min(nyquist_bin.saturating_sub(1));
            let bin_hi = (((f_hi / bin_hz) as usize) + 1).clamp(bin_lo + 1, nyquist_bin);
            (bin_lo, bin_hi)
        })
        .collect()
}

/// Windowing + FFT + logarithmic bucketing into `NUM_BARS` (using `bar_bins`,
/// already computed once up front via `precompute_bar_bins`), normalized
/// 0..1 using auto-gain (`running_max` decays slowly, used as the reference).
fn compute_bars(
    samples: &[f32],
    hann: &[f32],
    fft: &dyn rustfft::Fft<f32>,
    bar_bins: &[(usize, usize)],
    running_max: &mut f32,
) -> Vec<f32> {
    let mut buf: Vec<Complex32> = samples
        .iter()
        .zip(hann.iter())
        .map(|(s, w)| Complex32::new(s * w, 0.0))
        .collect();
    fft.process(&mut buf);

    let n = buf.len();
    let nyquist_bin = n / 2;

    // Magnitude per bin (only need the first half, the rest is a mirror).
    let magnitudes: Vec<f32> = buf[..nyquist_bin].iter().map(|c| c.norm()).collect();

    let mut frame_max = 1e-6f32;
    let mut bars = vec![0f32; NUM_BARS];

    for (bar, &(bin_lo, bin_hi)) in bars.iter_mut().zip(bar_bins.iter()) {
        let mag = magnitudes[bin_lo..bin_hi]
            .iter()
            .fold(0f32, |acc, &m| acc.max(m));
        *bar = mag;
        frame_max = frame_max.max(mag);
    }

    // Auto-gain: raise the reference quickly when it's louder, lower it slowly
    // as it gets quieter, so the bars stay proportional-looking at any volume.
    *running_max = if frame_max > *running_max {
        frame_max
    } else {
        *running_max * 0.98
    };
    let reference = running_max.max(1e-6);

    for bar in bars.iter_mut() {
        *bar = (*bar / reference).clamp(0.0, 1.0);
    }
    bars
}

/// Portrait canvas (462x1920)? Detected from the fb dimensions, so all drawing
/// functions adjust automatically without an extra parameter.
fn is_portrait(fb: &Framebuffer) -> bool {
    fb.height() > fb.width()
}

/// Info block style: `Lines` = 3 long lines (landscape, as originally);
/// `List` = short vertical list (portrait / a positioned block).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatusStyle {
    Auto,
    Lines,
    List,
    /// Each item with its own size/position/color (`cpu_size`, `ram_position`, ...).
    Items,
}

/// Appearance options read from config/CLI (global, like the language).
#[derive(Clone, Debug)]
struct UiOptions {
    show: Show,
    /// Elements shown while a panel layout is active.
    layout_show: Show,
    status_pos: Anchor,
    clock_pos: Anchor,
    spectrum_pos: Anchor,
    /// Spectrum size as a % of the content area (100 = all of it).
    spectrum_w: u32,
    spectrum_h: u32,
    status_style: StatusStyle,
    text_color: Option<(u8, u8, u8)>,
    clock_color: Option<(u8, u8, u8)>,
    /// Preset layout(s) (large panels in place of the clock), each with its own
    /// rotation duration in seconds (`layout = default:30, weather:5`; an entry
    /// without `:seconds` uses `layout_interval`, already resolved into this list
    /// by `parse_ui_options` — nothing downstream needs the raw default again).
    layouts: Vec<(&'static layouts::LayoutDef, u32)>,
    /// Panel opacity 0-100 (100 = solid, 0 = outline only).
    panel_opacity: u8,
    /// Panel behind status lines and the clock (same opacity as `panel_opacity`).
    text_backdrop: bool,
    items: [ItemStyle; 11],
    /// Maximum scale of the large clock's time and date.
    clock_time_size: u32,
    clock_date_size: u32,
    /// Panel behind the large clock (None = follows `text_backdrop`).
    clock_backdrop: Option<bool>,
    /// Maximum width (% of the canvas) of the track line in items mode.
    nowplaying_width: u32,
    net_unit: NetUnit,
    mem_gb: bool,
    /// Show the "NOW PLAYING:" label before the title (items mode).
    nowplaying_label: bool,
    /// Weather temperature unit: `true` = Fahrenheit, `false` = Celsius (default).
    weather_fahrenheit: bool,
}

impl Default for UiOptions {
    fn default() -> Self {
        UiOptions {
            show: Show::default(),
            layout_show: Show::default(),
            status_pos: Anchor::TOP_LEFT,
            clock_pos: Anchor::CENTER,
            spectrum_pos: Anchor::CENTER,
            spectrum_w: 100,
            spectrum_h: 100,
            status_style: StatusStyle::Auto,
            text_color: None,
            clock_color: None,
            layouts: Vec::new(),
            panel_opacity: 100,
            text_backdrop: false,
            items: [ItemStyle::default(); 11],
            clock_backdrop: None,
            nowplaying_width: 100,
            net_unit: NetUnit::Kb,
            mem_gb: false,
            nowplaying_label: true,
            clock_time_size: 20,
            clock_date_size: 6,
            weather_fahrenheit: false,
        }
    }
}

static UI_OPTS: std::sync::RwLock<Option<UiOptions>> = std::sync::RwLock::new(None);

fn set_ui_options(o: UiOptions) {
    if let Ok(mut g) = UI_OPTS.write() {
        *g = Some(o);
    }
}

#[cfg(test)]
thread_local! {
    static TEST_OPTS: std::cell::RefCell<Option<UiOptions>> = const { std::cell::RefCell::new(None) };
}

/// true while a panel layout (not `default`) is active: `opts().show` uses `layout_show`.
/// true while the `default2` layout (items with their own style) is active.
static ACTIVE_ITEMS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static ACTIVE_PANEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn opts() -> UiOptions {
    #[cfg(test)]
    if let Some(o) = TEST_OPTS.with(|t| t.borrow().clone()) {
        return o;
    }
    let mut o = UI_OPTS.read().ok().and_then(|g| g.clone()).unwrap_or_default();
    if ACTIVE_PANEL.load(std::sync::atomic::Ordering::Relaxed) {
        o.show = o.layout_show;
    }
    o
}

/// Panel color and the "placeholder" color used when opacity is < 100:
/// in `ui`, the panel is drawn with `PANEL_KEY` and the compositing blends it
/// with the background (see `Framebuffer::blit_ui`).
const PANEL_COLOR: (u8, u8, u8) = (0x14, 0x14, 0x1C);
const PANEL_KEY: (u8, u8, u8) = (2, 0, 1);

fn panel_fill() -> (u8, u8, u8) {
    if opts().panel_opacity < 100 { PANEL_KEY } else { PANEL_COLOR }
}

/// Semi-transparent panel behind text (status lines / clock) to make it easier to read.
fn draw_backdrop(fb: &mut Framebuffer, x: u32, y: u32, w: u32, h: u32) {
    draw_backdrop_if(fb, opts().text_backdrop, 12, x, y, w, h);
}

fn draw_backdrop_if(fb: &mut Framebuffer, on: bool, pad: u32, x: u32, y: u32, w: u32, h: u32) {
    if !on {
        return;
    }
    let fill = panel_fill();
    fb.fill_rect(x.saturating_sub(pad), y.saturating_sub(pad), w + 2 * pad, h + 2 * pad, fill.0, fill.1, fill.2);
}

const STATUS_LINE_H: u32 = STATUS_TEXT_SCALE * 7 + 6;

fn status_style(fb: &Framebuffer) -> StatusStyle {
    match opts().status_style {
        StatusStyle::Auto => {
            if ACTIVE_ITEMS.load(std::sync::atomic::Ordering::Relaxed) {
                StatusStyle::Items
            } else if is_portrait(fb) {
                StatusStyle::List
            } else {
                StatusStyle::Lines
            }
        }
        s => s,
    }
}

fn enabled_items(sh: &Show) -> Vec<usize> {
    let flags = [
        sh.cpu, sh.gpu, sh.uptime, sh.time, sh.date, sh.mem, sh.net, sh.disk, sh.volume,
        sh.nowplaying, sh.weather,
    ];
    (0..11).filter(|&i| flags[i]).collect()
}

/// Anchor and line height of each item (Items mode).
fn item_anchor(o: &UiOptions, i: usize) -> Anchor {
    o.items[i].pos.unwrap_or(o.status_pos)
}
fn item_scale(o: &UiOptions, i: usize) -> u32 {
    o.items[i].size.unwrap_or(STATUS_TEXT_SCALE)
}
fn item_line_h(scale: u32) -> u32 {
    scale * 7 + 6
}

/// Height (px) of the item groups anchored at the top / bottom.
fn items_bands(o: &UiOptions) -> (u32, u32) {
    let mut groups: Vec<((u8, u8), u32)> = Vec::new();
    for i in enabled_items(&o.show) {
        let a = item_anchor(o, i);
        let h = item_line_h(item_scale(o, i));
        match groups.iter_mut().find(|g| g.0 == (a.ax, a.ay)) {
            Some(g) => g.1 += h,
            None => groups.push(((a.ax, a.ay), h)),
        }
    }
    let band = |ay: u8| groups.iter().filter(|g| (g.0).1 == ay).map(|g| g.1 - 6 + 8 + 11).max().unwrap_or(10);
    (band(0), band(2))
}

/// Number of lines in the info block (depends only on the style and `show`, not
/// the data, so `content_rect` and `draw_status_lines` stay consistent).
fn status_line_count(fb: &Framebuffer) -> u32 {
    let sh = opts().show;
    match status_style(fb) {
        StatusStyle::Items => enabled_items(&sh).len() as u32,
        StatusStyle::Lines | StatusStyle::Auto => {
            (sh.cpu || sh.gpu || sh.uptime || sh.time || sh.date) as u32
                + (sh.mem || sh.net || sh.disk) as u32
                + (sh.volume || sh.nowplaying) as u32
        }
        StatusStyle::List => {
            (sh.time || sh.date) as u32
                + sh.uptime as u32
                + 2 * sh.cpu as u32
                + 2 * sh.gpu as u32
                + sh.mem as u32
                + sh.net as u32
                + sh.disk as u32
                + sh.volume as u32
                + 2 * sh.nowplaying as u32
        }
    }
}

/// Area (x, y, w, h) for the spectrum / clock / dashboard: the whole canvas
/// minus the band occupied by the info block when it's anchored at the top or
/// bottom (landscape default: 100px at the top, 10px at the bottom, as before).
fn content_rect(fb: &Framebuffer) -> (u32, u32, u32, u32) {
    if status_style(fb) == StatusStyle::Items {
        let (t, b) = items_bands(&opts());
        return (0, t, fb.width(), fb.height().saturating_sub(t + b));
    }
    let n = status_line_count(fb);
    let band = if n > 0 { 8 + n * STATUS_LINE_H + 11 } else { 10 };
    let (top, bottom) = match opts().status_pos.ay {
        0 => (band, 10),
        2 => (10, band),
        _ => (10, 10),
    };
    (0, top, fb.width(), fb.height().saturating_sub(top + bottom))
}

fn dim_color(c: (u8, u8, u8), pct: u32) -> (u8, u8, u8) {
    let f = |v: u8| ((v as u32 * pct) / 100) as u8;
    (f(c.0), f(c.1), f(c.2))
}

/// Largest scale (<= `max_scale`, at least 1) so `text` fits in `max_width` px.
fn fit_scale(text: &str, max_scale: u32, max_width: u32) -> u32 {
    let mut scale = max_scale.max(1);
    while scale > 1 && Framebuffer::text_width(text, scale) > max_width {
        scale -= 1;
    }
    scale
}

fn draw_bars(fb: &mut Framebuffer, heights: &[f32], color_mode: ColorMode) {
    let o = opts();
    let rect = content_rect(fb);
    let sw = rect.2 * o.spectrum_w.clamp(1, 100) / 100;
    let sh = rect.3 * o.spectrum_h.clamp(1, 100) / 100;
    let (area_left, area_top) = o.spectrum_pos.place(rect, (sw, sh), (0, 0));
    let area_height = sh;

    let gap = 3u32;
    let total_gap = gap * (heights.len() as u32 + 1);
    let bar_width = (sw.saturating_sub(total_gap)) / heights.len() as u32;

    let mut x = area_left + gap;
    for &h in heights {
        let bar_h = (area_height as f32 * h).round() as u32;
        let y = area_top + (area_height - bar_h);

        let (r, g, b) = level_color(h, color_mode);
        fb.fill_rect(x, y, bar_width, bar_h, r, g, b);

        x += bar_width + gap;
    }
}

/// Bar color based on the level (0.0-1.0) & `color_mode`:
/// - `Default`: green (quiet) -> yellow -> red (loud) gradient, as originally.
/// - `Custom(r, g, b)`: a single fixed color, but its brightness is still scaled
///   with the level (with a lower bound so the bar is never fully dark)
///   so the EQ's visual dynamics aren't lost even with just one color.
fn level_color(level: f32, color_mode: ColorMode) -> (u8, u8, u8) {
    let level = level.clamp(0.0, 1.0);
    match color_mode {
        ColorMode::Default => {
            if level < 0.6 {
                let t = level / 0.6;
                (
                    (0x20 as f32 + t * (0xE0 - 0x20) as f32) as u8,
                    0xE0,
                    0x30,
                )
            } else {
                let t = (level - 0.6) / 0.4;
                (0xE0, (0xE0 as f32 * (1.0 - t)) as u8, 0x30)
            }
        }
        ColorMode::Custom(r, g, b) => {
            const MIN_BRIGHTNESS: f32 = 0.25;
            let factor = MIN_BRIGHTNESS + (1.0 - MIN_BRIGHTNESS) * level;
            (
                (r as f32 * factor) as u8,
                (g as f32 * factor) as u8,
                (b as f32 * factor) as u8,
            )
        }
    }
}

/// Solid "accent" color used for the idle clock — unlike `level_color` which
/// scales with the bar level, this clock is always shown at full brightness (not
/// dimmed) so it's clearly readable from a distance when the screen is "empty".
fn accent_color(color_mode: ColorMode) -> (u8, u8, u8) {
    match color_mode {
        ColorMode::Default => (0xE0, 0xE0, 0xE0),
        ColorMode::Custom(r, g, b) => (r, g, b),
    }
}

/// Dashboard grid: 4 boxes (FPS, GPU, CPU, RAM) side by side — replaces the EQ bar/clock
/// when `gaming_mode` is active (GPU usage > 50%, see the main loop). The small info
/// line (`draw_status_lines`) is still shown separately above as usual.
///
/// Each box: small label on top, large number in the middle, small detail at the
/// bottom. Chosen from 3 proposed display options (large number+gauge,
/// history graph, dashboard grid) — the user picked the grid.
fn draw_game_dashboard(
    fb: &mut Framebuffer,
    sys: &System,
    gpu_percent: Option<f32>,
    gpu_data: &gpu_amd::GpuAmdData,
    cpu_temp: Option<f32>,
    cpu_power: Option<f32>,
    cpu_mhz: Option<u32>,
    color_mode: ColorMode,
) {
    // Same content area as the spectrum/clock, so all modes
    // occupy the same space (they don't touch the info block).
    let (area_left, area_top, width, area_height) = content_rect(fb);

    let side_margin = 20u32;
    let gap = 16u32;
    let panel_count = 4u32;
    let usable_width = width.saturating_sub(side_margin * 2);
    let portrait = is_portrait(fb);
    // Landscape: 4 boxes side by side. Portrait: 4 boxes stacked vertically.
    let (panel_width, panel_height) = if portrait {
        (
            usable_width,
            area_height.saturating_sub(gap * (panel_count - 1)) / panel_count,
        )
    } else {
        (
            usable_width.saturating_sub(gap * (panel_count - 1)) / panel_count,
            area_height,
        )
    };

    let border = accent_color(color_mode);
    let panel_bg = panel_fill();
    let value_color = opts().text_color.unwrap_or((0xF0u8, 0xF0u8, 0xF0u8));
    let label_color = opts().text_color.map_or((0xA0u8, 0xA0u8, 0xA8u8), |c| dim_color(c, 70));
    let border_thickness = 2u32;

    // Each panel's data is prepared first (as strings), then drawn in a single loop
    // below so the layout of all 4 boxes stays consistent (no code duplication).
    struct Panel {
        label: &'static str,
        value: String,
        detail: String,
    }

    let fps_value = gpu_data.fps.map_or_else(|| "--".to_string(), |f| f.to_string());
    // If GPU usage is high but fps is empty, the game is most likely
    // borderless windowed (not exclusive fullscreen) — see the note in
    // gpu_amd.rs. Show this hint instead of just a confusing plain "--".
    let fps_detail = match gpu_data.fps {
        Some(f) if f > 0 => format!("{:.1}MS", 1000.0 / f as f32),
        _ => match fps_etw::status() {
            fps_etw::Status::NeedsAdmin => i18n::t().need_admin.to_string(),
            _ => "-".to_string(),
        },
    };

    let gpu_value = gpu_percent.map_or_else(|| "N/A".to_string(), |p| format!("{p:.0}%"));
    let gpu_detail = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = gpu_data.temp_edge_c { parts.push(format!("{t}C")); }
        if let Some(w) = gpu_data.power_w { parts.push(format!("{w}W")); }
        if let Some(r) = gpu_data.fan_rpm { parts.push(format!("{r}rpm")); }
        if parts.is_empty() { "N/A".to_string() } else { parts.join(" ") }
    };

    let cpu_pct = sys.global_cpu_info().cpu_usage();
    let cpu_value = format!("{cpu_pct:.0}%");
    // CPU detail: real-time frequency (GHz/MHz) + temperature + power — only
    // show the fields that are available (e.g. if the temperature driver isn't
    // present, just frequency + watts).
    let cpu_detail = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(m) = cpu_mhz {
            parts.push(format_freq_mhz(m));
        }
        if let Some(t) = cpu_temp { parts.push(format!("{t:.0}C")); }
        if let Some(w) = cpu_power { parts.push(format!("{w:.0}W")); }
        if parts.is_empty() { "N/A".to_string() } else { parts.join(" ") }
    };

    let used_mb = sys.used_memory() / 1024 / 1024;
    let total_mb = sys.total_memory() / 1024 / 1024;
    let ram_pct = if total_mb > 0 { (used_mb as f32 / total_mb as f32) * 100.0 } else { 0.0 };
    let ram_value = format!("{ram_pct:.0}%");
    let ram_detail = format!("{used_mb}/{total_mb}MB");

    let panels = [
        Panel { label: "FPS", value: fps_value, detail: fps_detail },
        Panel { label: "GPU", value: gpu_value, detail: gpu_detail },
        Panel { label: "CPU", value: cpu_value, detail: cpu_detail },
        Panel { label: "RAM", value: ram_value, detail: ram_detail },
    ];

    // A scale of 18 was chosen so the longest realistic 4-character string
    // that can appear here ("100%") still fits within 1 box's width (~458px at
    // 1920x462 resolution): 4 char * 6 * 18 = 432px, still leaving some margin.
    let value_scale = 18u32;
    let label_scale = 3u32;
    let detail_scale = 4u32;
    let padding = 14u32;

    for (i, panel) in panels.iter().enumerate() {
        let i = i as u32;
        let (x, area_top, area_height) = if portrait {
            (area_left + side_margin, area_top + i * (panel_height + gap), panel_height)
        } else {
            (area_left + side_margin + i * (panel_width + gap), area_top, panel_height)
        };
        // Text scale is automatically reduced if it doesn't fit the box width.
        let inner_w = panel_width.saturating_sub(border_thickness * 2 + 8);
        let value_scale = fit_scale(&panel.value, value_scale, inner_w);
        let detail_scale = fit_scale(&panel.detail, detail_scale, inner_w);
        let label_height = Framebuffer::text_height(label_scale);
        let detail_height = Framebuffer::text_height(detail_scale);
        let value_height = Framebuffer::text_height(value_scale);

        // Border (outer box) then a slightly smaller fill inside it —
        // an "outline" effect without needing a separate line-drawing function.
        fb.fill_rect(x, area_top, panel_width, area_height, border.0, border.1, border.2);
        fb.fill_rect(
            x + border_thickness,
            area_top + border_thickness,
            panel_width.saturating_sub(border_thickness * 2),
            area_height.saturating_sub(border_thickness * 2),
            panel_bg.0, panel_bg.1, panel_bg.2,
        );

        let label_width = Framebuffer::text_width(panel.label, label_scale);
        let label_x = x + panel_width.saturating_sub(label_width) / 2;
        let label_y = area_top + padding;
        fb.draw_text(label_x, label_y, panel.label, label_color.0, label_color.1, label_color.2, label_scale);

        let detail_width = Framebuffer::text_width(&panel.detail, detail_scale);
        let detail_x = x + panel_width.saturating_sub(detail_width) / 2;
        let detail_y = area_top + area_height.saturating_sub(detail_height + padding);
        fb.draw_text(detail_x, detail_y, &panel.detail, label_color.0, label_color.1, label_color.2, detail_scale);

        // The large number is positioned right in the middle of the EMPTY space between
        // the label (top) and the detail (bottom) — not the center of the whole box — so it
        // doesn't feel "pushed down" when the label/detail take up noticeable space.
        let value_width = Framebuffer::text_width(&panel.value, value_scale);
        let value_x = x + panel_width.saturating_sub(value_width) / 2;
        let middle_top = label_y + label_height;
        let middle_bottom = detail_y;
        let middle_space = middle_bottom.saturating_sub(middle_top);
        let value_y = middle_top + middle_space.saturating_sub(value_height) / 2;
        fb.draw_text(value_x, value_y, &panel.value, value_color.0, value_color.1, value_color.2, value_scale);

    }
}

/// Draws the large digital clock in the middle of the area normally used by the EQ bar,
/// called in place of `draw_bars` while idle (an empty EQ bar
/// isn't useful to keep drawing). The small info line (`draw_status_lines`) is
/// still drawn separately as usual, unaffected by this function.
fn draw_idle_clock(fb: &mut Framebuffer, color_mode: ColorMode) {
    let o = opts();
    let rect = content_rect(fb);
    let width = rect.2;

    let (r, g, b) = o.clock_color.unwrap_or_else(|| accent_color(color_mode));
    let now = Local::now();

    // Each line is resized to fit within the canvas width.
    let max_w = width.saturating_sub(40);
    let time_str = now.format("%H:%M:%S").to_string();
    let mut lines: Vec<(String, u32)> = vec![(time_str, o.clock_time_size)];
    if o.show.clock_date {
        if is_portrait(fb) {
            lines.push((i18n::weekday(&now), o.clock_date_size));
            lines.push((i18n::day_month_year(&now), o.clock_date_size));
        } else {
            lines.push((i18n::date_long(&now), o.clock_date_size));
        }
    }
    let lines: Vec<(String, u32)> = lines
        .into_iter()
        .map(|(t, max)| {
            let sc = fit_scale(&t, max, max_w);
            (t, sc)
        })
        .collect();

    let gap = 20u32;
    let block_height: u32 = lines
        .iter()
        .map(|(_, sc)| Framebuffer::text_height(*sc))
        .sum::<u32>()
        + gap * (lines.len() as u32 - 1);
    let block_width = lines.iter().map(|(t, sc)| Framebuffer::text_width(t, *sc)).max().unwrap_or(0);
    let (_, mut y) = o.clock_pos.place(rect, (block_width, block_height), (0, 10));
    let (bx, _) = o.clock_pos.place(rect, (block_width, 0), (20, 0));
    draw_backdrop_if(fb, o.clock_backdrop.unwrap_or(o.text_backdrop), 12, bx, y, block_width, block_height);
    for (text, sc) in &lines {
        let w = Framebuffer::text_width(text, *sc);
        // Each line is aligned according to the horizontal anchor.
        let (x, _) = o.clock_pos.place(rect, (w, 0), (20, 0));
        fb.draw_text(x, y, text, r, g, b, *sc);
        y += Framebuffer::text_height(*sc) + gap;
    }
}

/// Info block (CPU/GPU/RAM/network/disk/volume/track/time): elements are
/// chosen with `show`/`hide`, the color with `text_color`, the position with
/// `status_position`. `Lines` style = 3 long lines (landscape); `List` =
/// short vertical list (portrait).
///
/// Fields with no data (e.g. GPU not initialized) remain visible as
/// "N/A" / "-", so the lines don't change position.
#[allow(clippy::too_many_arguments)]
fn draw_status_lines(
    fb: &mut Framebuffer,
    sys: &System,
    gpu_percent: Option<f32>,
    gpu_data: &gpu_amd::GpuAmdData,
    net_kb: (f64, f64),
    disk_mb: (f64, f64),
    volume: Option<(f32, bool)>,
    cpu_temp: Option<f32>,
    cpu_power: Option<f32>,
    cpu_mhz: Option<u32>,
    now_playing: Option<&str>,
    weather: Option<&weather::WeatherSnapshot>,
    marquee: &mut Marquee,
) {
    let o = opts();
    let sh = &o.show;
    let n_lines = status_line_count(fb);
    if n_lines == 0 {
        return;
    }
    let scale = STATUS_TEXT_SCALE;
    let color = o.text_color.unwrap_or((0xE0, 0xE0, 0xE0));
    let tr = i18n::t();
    let style = status_style(fb);
    let now = Local::now();

    let cpu = sys.global_cpu_info().cpu_usage();
    let used_mb = sys.used_memory() / 1024 / 1024;
    let total_mb = sys.total_memory() / 1024 / 1024;
    let uptime_str = format_uptime(System::uptime());
    let time_str = now.format("%H:%M:%S").to_string();
    let date_str = now.format("%Y-%m-%d").to_string();

    let gpu_str = gpu_percent.map_or_else(|| "N/A".to_string(), |p| format!("{p:.0}%"));
    // GPU sensors (ADL PMLog): only the supported fields. "C" and not "°C": ASCII font.
    let gpu_hw = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = gpu_data.temp_edge_c.filter(|_| sh.gpu_temp) { parts.push(format!("{t}C")); }
        if let Some(w) = gpu_data.power_w.filter(|_| sh.gpu_power) { parts.push(format!("{w}W")); }
        if let Some(r) = gpu_data.fan_rpm.filter(|_| sh.gpu_fan) { parts.push(format!("{r}rpm")); }
        if let Some(c) = gpu_data.clock_mhz.filter(|_| sh.gpu_clock) { parts.push(format_freq_mhz(c.max(0) as u32)); }
        if let Some(f) = gpu_data.fps.filter(|_| sh.gpu_fps) { parts.push(format!("{f}fps")); }
        parts.join(" ")
    };
    let cpu_hw = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = cpu_temp.filter(|_| sh.cpu_temp) { parts.push(format!("{t:.0}C")); }
        if let Some(w) = cpu_power.filter(|_| sh.cpu_power) { parts.push(format!("{w:.0}W")); }
        if parts.is_empty() && (sh.cpu_temp || sh.cpu_power) {
            "N/A".to_string()
        } else {
            parts.join(" ")
        }
    };
    let cpu_freq = if sh.cpu_freq {
        cpu_mhz.map_or_else(|| "N/A".to_string(), format_freq_mhz)
    } else {
        String::new()
    };
    let (net_down, net_up) = net_kb;
    let (disk_read, disk_write) = disk_mb;
    let volume_str = match volume {
        Some((_, true)) => tr.mute.to_string(),
        Some((pct, false)) => format!("{pct:.0}%"),
        None => "N/A".to_string(),
    };
    let song_str = now_playing.unwrap_or("-");

    let mem_p = format!("{} {}", tr.mem, fmt_mem(used_mb, total_mb, o.mem_gb));
    let nd = fmt_net(net_down, o.net_unit);
    let nu = fmt_net(net_up, o.net_unit);
    let vol_p = format!("VOL {volume_str}");
    let weather_str = match weather {
        Some(w) => {
            let unit = if o.weather_fahrenheit { "F" } else { "C" };
            format!(
                "{} {:.0}{unit} {}",
                tr.weather,
                w.temp_in(o.weather_fahrenheit),
                weather_icon::condition_name(w.icon)
            )
        }
        None => format!("{} N/A", tr.weather),
    };

    if style == StatusStyle::Items {
        draw_status_items(
            fb, &o, &[
                format!("CPU {cpu:.0}%{}{}{}", if cpu_freq.is_empty() { "" } else { " " }, cpu_freq,
                    if cpu_hw.is_empty() { String::new() } else { format!(" {cpu_hw}") }),
                format!("GPU {gpu_str}{}", if gpu_hw.is_empty() { String::new() } else { format!(" {gpu_hw}") }),
                format!("{} {uptime_str}", tr.uptime),
                time_str.clone(),
                date_str.clone(),
                mem_p.clone(),
                format!("{} {} {nd} {} {nu}", tr.net, tr.net_down, tr.net_up),
                format!("{} {} {disk_read:.1}MB/S {} {disk_write:.1}MB/S", tr.disk, tr.disk_read, tr.disk_write),
                vol_p.clone(),
                if o.nowplaying_label { format!("{}: ", tr.now_playing) } else { String::new() },
                weather_str,
            ],
            song_str, marquee,
        );
        return;
    }

    // "Normal" text lines + (optionally) the track line.
    let mut rows: Vec<String> = Vec::new();
    let mut np_prefix: Option<String> = None; // Lines: prefix before the title
    let mut np_label: Option<String> = None; // List: label, title on the next line

    match style {
        StatusStyle::Items => {}
        StatusStyle::Lines | StatusStyle::Auto => {
            let gpu_full = if gpu_hw.is_empty() { gpu_str.clone() } else { format!("{gpu_str} {gpu_hw}") };
            let l1: Vec<String> = [
                (sh.cpu, format!("CPU {cpu:.0}% {cpu_freq} {cpu_hw}")),
                (sh.gpu, format!("GPU {gpu_full}")),
                (sh.uptime, format!("{} {uptime_str}", tr.uptime)),
                (sh.time, time_str.clone()),
                (sh.date, date_str.clone()),
            ]
            .into_iter()
            .filter_map(|(on, t)| on.then_some(t))
            .collect();
            if !l1.is_empty() {
                rows.push(l1.join("  "));
            }
            let l2: Vec<String> = [
                (sh.mem, mem_p.clone()),
                (
                    sh.net,
                    format!("{} {} {nd} {} {nu}", tr.net, tr.net_down, tr.net_up),
                ),
                (
                    sh.disk,
                    format!(
                        "{} {} {disk_read:.1}MB/S {} {disk_write:.1}MB/S",
                        tr.disk, tr.disk_read, tr.disk_write
                    ),
                ),
            ]
            .into_iter()
            .filter_map(|(on, t)| on.then_some(t))
            .collect();
            if !l2.is_empty() {
                rows.push(l2.join("  "));
            }
            if sh.nowplaying {
                let prefix = if sh.volume { format!("{vol_p}  {}: ", tr.now_playing) } else { format!("{}: ", tr.now_playing) };
                np_prefix = Some(prefix);
            } else if sh.volume {
                rows.push(vol_p.clone());
            }
        }
        StatusStyle::List => {
            match (sh.time, sh.date) {
                (true, true) => rows.push(format!("{time_str}  {date_str}")),
                (true, false) => rows.push(time_str.clone()),
                (false, true) => rows.push(date_str.clone()),
                _ => {}
            }
            if sh.uptime {
                rows.push(format!("{} {uptime_str}", tr.uptime));
            }
            if sh.cpu {
                rows.push(format!("CPU {cpu:.0}% {cpu_freq}"));
                rows.push(format!("    {cpu_hw}"));
            }
            if sh.gpu {
                rows.push(format!("GPU {gpu_str}"));
                rows.push(format!("    {gpu_hw}"));
            }
            if sh.mem {
                rows.push(mem_p.clone());
            }
            if sh.net {
                rows.push(format!("{} {} {nd} {} {nu}", tr.net, tr.net_down, tr.net_up));
            }
            if sh.disk {
                rows.push(format!("{} {} {disk_read:.1} {} {disk_write:.1}MB/S", tr.disk, tr.disk_read, tr.disk_write));
            }
            if sh.volume {
                rows.push(vol_p.clone());
            }
            if sh.nowplaying {
                np_label = Some(format!("{}:", tr.now_playing));
            }
        }
    }

    // ---- block geometry
    let canvas_w = fb.width();
    let max_w = canvas_w.saturating_sub(40);
    let widest_row = rows.iter().map(|r| Framebuffer::text_width(r, scale)).max().unwrap_or(0);
    let mut block_w = widest_row;
    let mut scrolling = false;
    let mut title_window = 0u32; // title window width
    let mut prefix_w = 0u32;
    if let Some(prefix) = &np_prefix {
        prefix_w = Framebuffer::text_width(prefix, scale);
        title_window = max_w.saturating_sub(prefix_w);
        scrolling = title_window > 0 && marquee.tick(song_str, title_window);
        let title_w = if scrolling { title_window } else { Framebuffer::text_width(song_str, scale) };
        block_w = block_w.max(prefix_w + title_w);
    } else if let Some(label) = &np_label {
        // List: the title uses the block's full width (minimum ~23 characters).
        let min_w = max_w.min(23 * (5 + 1) * scale);
        block_w = block_w.max(Framebuffer::text_width(label, scale)).max(min_w);
        title_window = block_w;
        scrolling = title_window > 0 && marquee.tick(song_str, title_window);
    }
    let block_w = block_w.min(max_w.max(1));
    let total_lines = rows.len() as u32 + np_prefix.is_some() as u32 + 2 * np_label.is_some() as u32;
    debug_assert_eq!(total_lines, n_lines);
    let block_h = total_lines * STATUS_LINE_H - 6;
    let (bx, by) = o.status_pos.place((0, 0, canvas_w, fb.height()), (block_w, block_h), (20, 8));
    draw_backdrop(fb, bx, by, block_w, block_h);

    // ---- drawing
    let mut y = by;
    for row in &rows {
        fb.draw_text(bx, y, row, color.0, color.1, color.2, scale);
        y += STATUS_LINE_H;
    }
    let draw_title = |fb: &mut Framebuffer, marquee: &Marquee, x0: u32, y: u32, window: u32| {
        if window == 0 {
            return;
        }
        let x1 = x0 + window;
        if scrolling {
            // Two copies side by side: when the first one exits, the second picks up right after.
            let loop_text = format!("{song_str}{MARQUEE_GAP}");
            let loop_width = Framebuffer::text_width(&loop_text, scale) as i64;
            let base_x = x0 as i64 - marquee.offset_px as i64;
            fb.draw_text_clipped(base_x, y, &loop_text, color.0, color.1, color.2, scale, x0, x1);
            fb.draw_text_clipped(base_x + loop_width, y, &loop_text, color.0, color.1, color.2, scale, x0, x1);
        } else {
            fb.draw_text(x0, y, song_str, color.0, color.1, color.2, scale);
        }
    };
    if let Some(prefix) = &np_prefix {
        fb.draw_text(bx, y, prefix, color.0, color.1, color.2, scale);
        draw_title(fb, marquee, bx + prefix_w, y, title_window);
    } else if let Some(label) = &np_label {
        fb.draw_text(bx, y, label, color.0, color.1, color.2, scale);
        y += STATUS_LINE_H;
        draw_title(fb, marquee, bx, y, title_window);
    }
}

/// Items mode: each item has its own size, position and color. Items with the
/// same anchor are stacked into one block. `texts` is in `ITEM_NAMES` order; for the
/// track it only contains the prefix (the title scrolls next to it).
fn draw_status_items(fb: &mut Framebuffer, o: &UiOptions, texts: &[String], song: &str, marquee: &mut Marquee) {
    let canvas = (0, 0, fb.width(), fb.height());
    let max_w = fb.width().saturating_sub(40);
    let default_color = o.text_color.unwrap_or((0xE0, 0xE0, 0xE0));
    // Groups by anchor, in order of appearance.
    let mut groups: Vec<(Anchor, Vec<usize>)> = Vec::new();
    for i in enabled_items(&o.show) {
        let a = item_anchor(o, i);
        match groups.iter_mut().find(|g| g.0 == a) {
            Some(g) => g.1.push(i),
            None => groups.push((a, vec![i])),
        }
    }
    // Width (excluding the track) of each group: needed so the track doesn't overlap its neighbors.
    let group_w: Vec<u32> = groups
        .iter()
        .map(|(_, m)| {
            m.iter().filter(|&&i| i != 9).map(|&i| Framebuffer::text_width(&texts[i], item_scale(o, i))).max().unwrap_or(0)
        })
        .collect();
    let width_of = |ax: u8, ay: u8| -> u32 {
        groups.iter().position(|g| g.0.ax == ax && g.0.ay == ay).map_or(0, |k| group_w[k])
    };
    let gap = 30u32;
    for (anchor, members) in groups.clone() {
        let cw = fb.width();
        let (l, c, r) = (width_of(0, anchor.ay), width_of(1, anchor.ay), width_of(2, anchor.ay));
        let mut avail = max_w;
        match anchor.ax {
            1 => {
                let side = l.max(r);
                if side > 0 {
                    avail = avail.min(cw.saturating_sub(2 * (side + 20 + gap)));
                }
            }
            0 => {
                if c > 0 {
                    avail = avail.min(((cw - c.min(cw)) / 2).saturating_sub(20 + gap));
                } else if r > 0 {
                    avail = avail.min(cw.saturating_sub(r + 40 + gap));
                }
            }
            _ => {
                if c > 0 {
                    avail = avail.min(((cw - c.min(cw)) / 2).saturating_sub(20 + gap));
                } else if l > 0 {
                    avail = avail.min(cw.saturating_sub(l + 40 + gap));
                }
            }
        }
        avail = avail.min(max_w * o.nowplaying_width / 100);
        let mut widths = Vec::new();
        let mut title_windows = Vec::new();
        for &i in &members {
            let sc = item_scale(o, i);
            let mut w = Framebuffer::text_width(&texts[i], sc);
            let mut win = 0;
            if i == 9 {
                marquee.scale = sc;
                win = avail.saturating_sub(w);
                let scrolling = win > 0 && marquee.tick(song, win);
                w += if scrolling { win } else { Framebuffer::text_width(song, sc) };
            }
            widths.push(w.min(max_w.max(1)));
            title_windows.push(win);
        }
        let block_w = widths.iter().copied().max().unwrap_or(0);
        let block_h: u32 = members.iter().map(|&i| item_line_h(item_scale(o, i))).sum::<u32>() - 6;
        let (bx, by) = anchor.place(canvas, (block_w, block_h), (20, 8));
        let mut y = by;
        for (k, &i) in members.iter().enumerate() {
            let sc = item_scale(o, i);
            let c = o.items[i].color.unwrap_or(default_color);
            // Horizontal alignment according to the group's anchor.
            let x = match anchor.ax {
                0 => bx,
                1 => bx + (block_w - widths[k].min(block_w)) / 2,
                _ => bx + (block_w - widths[k].min(block_w)),
            };
            let line_w = widths[k];
            draw_backdrop_if(fb, o.items[i].backdrop.unwrap_or(o.text_backdrop), 8, x, y, line_w, Framebuffer::text_height(sc));
            fb.draw_text(x, y, &texts[i], c.0, c.1, c.2, sc);
            if i == 9 && title_windows[k] > 0 {
                let x0 = x + Framebuffer::text_width(&texts[i], sc);
                let x1 = x0 + title_windows[k];
                if Framebuffer::text_width(song, sc) > title_windows[k] {
                    let loop_text = format!("{song}{MARQUEE_GAP}");
                    let lw = Framebuffer::text_width(&loop_text, sc) as i64;
                    let base = x0 as i64 - marquee.offset_px as i64;
                    fb.draw_text_clipped(base, y, &loop_text, c.0, c.1, c.2, sc, x0, x1);
                    fb.draw_text_clipped(base + lw, y, &loop_text, c.0, c.1, c.2, sc, x0, x1);
                } else {
                    fb.draw_text(x0, y, song, c.0, c.1, c.2, sc);
                }
            }
            y += item_line_h(sc);
        }
    }
}

/// Format the CPU frequency: >= 1000 MHz is shown as GHz with 1 decimal
/// (e.g. "4.9GHz"), below that it stays in MHz ("950MHz"). ASCII bitmap font
/// (avoiding the ° character / any non-ASCII character).
fn format_freq_mhz(mhz: u32) -> String {
    if mhz >= 1000 {
        format!("{:.1}GHz", mhz as f32 / 1000.0)
    } else {
        format!("{mhz}MHz")
    }
}

/// Format the system uptime (seconds since boot) as `"HH:MM:SS"`, or
/// `"NDHH:MM:SS"` (e.g. `"3D02:15:07"`) if more than 1 day has passed.
fn format_uptime(total_secs: u64) -> String {
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if days > 0 {
        format!("{}D{:02}:{:02}:{:02}", days, hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn sample(fb: &mut Framebuffer) {
        let sys = System::new_all();
        let gpu = gpu_amd::GpuAmdData::default();
        let mut marquee = Marquee::new();
        fb.clear(0x08, 0x08, 0x10);
        draw_game_dashboard(fb, &sys, Some(87.0), &gpu, Some(61.0), Some(88.0), Some(4900), ColorMode::Default);
        draw_status_lines(
            fb, &sys, Some(87.0), &gpu, (123.0, 45.0), (1.5, 0.2), Some((40.0, false)),
            Some(61.0), Some(88.0), Some(4900), Some("Artist - A very long song title for the marquee"),
            None,
            &mut marquee,
        );
    }

    /// No panics and correct dimensions in both orientations; with
    /// TROFEO_DUMP_DIR set, also saves PNGs for visual inspection.
    #[test]
    fn draws_in_both_orientations() {
        for (name, o) in [("landscape", Orientation::Landscape), ("portrait", Orientation::Portrait)] {
            let mut fb = Framebuffer::new(o.canvas());
            sample(&mut fb);
            let bars = vec![0.6f32; NUM_BARS];
            let mut fb2 = Framebuffer::new(o.canvas());
            fb2.clear(0x08, 0x08, 0x10);
            draw_bars(&mut fb2, &bars, ColorMode::Default);
            let mut fb3 = Framebuffer::new(o.canvas());
            fb3.clear(0x08, 0x08, 0x10);
            draw_idle_clock(&mut fb3, ColorMode::Default);
            let panel = fb.rotated(o.output_rotation(false));
            assert_eq!((panel.width(), panel.height()), (1920, 462));
            if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                for (tag, f) in [("game", &fb), ("bars", &fb2), ("clock", &fb3)] {
                    std::fs::write(
                        format!("{dir}/{name}_{tag}.ppm"),
                        [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
                    ).unwrap();
                }
            }
        }
    }

    /// All preset layouts, landscape and portrait: no panics; with
    /// TROFEO_DUMP_DIR, saves PNGs.
    #[test]
    fn preset_layouts_render() {
        let data = layouts::WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: Some("Artist - A very long song title that has to scroll across the panel".into()),
            weather: Some(weather::WeatherSnapshot {
                temp_c: 21.0,
                humidity: Some(55),
                code: 61,
                icon: weather_icon::WeatherIcon::Rain,
                city: Some("Milano".into()),
            }),
        };
        for def in layouts::LAYOUTS {
            for (tag, o) in [("l", Orientation::Landscape), ("p", Orientation::Portrait)] {
                let mut fb = Framebuffer::new(o.canvas());
                fb.clear(0x08, 0x08, 0x10);
                let mut m = Marquee::new();
                layouts::draw_layout(&mut fb, def, &data, ColorMode::Default, &mut m);
                if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                    std::fs::write(
                        format!("{dir}/layout_{}_{tag}.ppm", def.name),
                        [format!("P6\n{} {}\n255\n", fb.width(), fb.height()).into_bytes(), fb.as_bytes().to_vec()].concat(),
                    ).unwrap();
                }
            }
        }
    }

    /// Semi-transparent panels over a background: compositing with panel_opacity.
    #[test]
    fn panel_opacity_shows_background() {
        let data = layouts::WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: None,
            weather: None,
        };
        let canvas = Orientation::Landscape.canvas();
        let mut out = Vec::new();
        for op in [100u8, 50, 15] {
            TEST_OPTS.with(|t| *t.borrow_mut() = Some(UiOptions { panel_opacity: op, show: Show::all(false), ..UiOptions::default() }));
            let mut ui = Framebuffer::new(canvas);
            ui.clear(1, 0, 2);
            let mut m = Marquee::new();
            layouts::draw_layout(&mut ui, layouts::find("overview").unwrap(), &data, ColorMode::Default, &mut m);
            let mut fb = Framebuffer::new(canvas);
            for (i, p) in fb.as_bytes_mut().chunks_exact_mut(3).enumerate() {
                let x = i as u32 % canvas.width;
                let y = i as u32 / canvas.width;
                p.copy_from_slice(&[(x / 8) as u8, (y / 2) as u8, 160]);
            }
            let panel = (op < 100).then_some((PANEL_KEY, PANEL_COLOR, op));
            fb.blit_ui(&ui, 0, 0, (1, 0, 2), panel);
            out.push(fb);
        }
        TEST_OPTS.with(|t| *t.borrow_mut() = None);
        if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
            for (i, f) in out.iter().enumerate() {
                std::fs::write(
                    format!("{dir}/opacity_{i}.ppm"),
                    [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
    }

    #[test]
    fn item_config_parses() {
        let txt = "status_style = items\ncpu_size = 5\ncpu_color = #FFC800\ncpu_position = top-left\ngpu_size = 4\ngpu_position = top-right\nram_size = 2\nram_position = bottom-left\nnowplaying_size = 3\nnowplaying_position = bottom\ntime_size = 6\ntime_position = bottom-right\nhide = uptime, date, net, disk, volume, cpu_freq, gpu_fan\nclock_time_size = 12\n";
        let f = ConfigFile::parse(txt).unwrap();
        let o = parse_ui_options(&f).unwrap();
        assert_eq!(o.items[0].size, Some(5));
        assert_eq!(o.items[0].color, Some((255, 200, 0)));
        assert_eq!(o.items[5].size, Some(2));
        assert_eq!(o.items[3].size, Some(6));
        assert!(!o.show.cpu_freq && o.show.cpu_temp);
        assert_eq!(o.clock_time_size, 12);
        let ob = parse_ui_options(&ConfigFile::parse("text_backdrop = true\ncpu_backdrop = false\nclock_backdrop = false\n").unwrap()).unwrap();
        assert_eq!(ob.items[0].backdrop, Some(false));
        assert_eq!(ob.items[1].backdrop, None);
        assert_eq!(ob.clock_backdrop, Some(false));
        // Without `layout`: items with a style => default2. With `layout = default`: the classic screen.
        assert_eq!(o.layouts.len(), 1);
        assert_eq!(o.layouts[0].0.name, "default2");
        let o = parse_ui_options(&ConfigFile::parse(&format!("{txt}layout = default\n")).unwrap()).unwrap();
        assert!(o.layouts.is_empty());
        let o = parse_ui_options(&ConfigFile::parse(&format!("{txt}layout = default, default2\n")).unwrap()).unwrap();
        assert_eq!(o.layouts.len(), 2);
    }

    /// Per-layout rotation duration (`layout = a:30, b:5`): each entry can carry
    /// its own seconds; one left bare falls back to `layout_interval`.
    #[test]
    fn layout_seconds_parse_per_entry() {
        let o = parse_ui_options(&ConfigFile::parse("layout = default:30, weather:5\n").unwrap()).unwrap();
        assert_eq!(o.layouts.len(), 2);
        assert_eq!(o.layouts[0].0.name, "default");
        assert_eq!(o.layouts[0].1, 30);
        assert_eq!(o.layouts[1].0.name, "weather");
        assert_eq!(o.layouts[1].1, 5);

        // No ":seconds" at all => every entry uses layout_interval (default 15).
        let o = parse_ui_options(&ConfigFile::parse("layout = cpu, gpu\n").unwrap()).unwrap();
        assert_eq!(o.layouts[0].1, 15);
        assert_eq!(o.layouts[1].1, 15);

        // A bare entry mixed with explicit ones falls back to a custom layout_interval.
        let o = parse_ui_options(&ConfigFile::parse("layout = cpu:20, gpu\nlayout_interval = 40\n").unwrap()).unwrap();
        assert_eq!(o.layouts[0].1, 20);
        assert_eq!(o.layouts[1].1, 40);

        // Out-of-range and non-numeric per-entry seconds are rejected.
        assert!(parse_ui_options(&ConfigFile::parse("layout = cpu:1\n").unwrap()).is_err());
        assert!(parse_ui_options(&ConfigFile::parse("layout = cpu:9999\n").unwrap()).is_err());
        assert!(parse_ui_options(&ConfigFile::parse("layout = cpu:abc\n").unwrap()).is_err());
    }

    /// The main loop picks the active entry by walking cumulative durations,
    /// so a 30s layout stays on screen for 30s straight and a 5s one for 5s —
    /// not alternating every 5s (see the doc comment above the real loop code).
    /// This mirrors that selection logic against `layout = default:30, weather:5`.
    #[test]
    fn layout_rotation_respects_per_entry_seconds() {
        let o = parse_ui_options(&ConfigFile::parse("layout = default:30, weather:5\n").unwrap()).unwrap();
        let pick = |elapsed_secs: u64| -> &'static str {
            let total: u64 = o.layouts.iter().map(|(_, s)| *s as u64).sum::<u64>().max(1);
            let t = elapsed_secs % total;
            let mut acc = 0u64;
            for (def, secs) in &o.layouts {
                acc += *secs as u64;
                if t < acc {
                    return def.name;
                }
            }
            o.layouts.last().unwrap().0.name
        };
        // First 30s: "default". Next 5s (30..35): "weather". Then the 35s cycle repeats.
        for t in [0, 1, 15, 29] {
            assert_eq!(pick(t), "default", "t={t}");
        }
        for t in [30, 32, 34] {
            assert_eq!(pick(t), "weather", "t={t}");
        }
        assert_eq!(pick(35), "default"); // cycle wraps back
        assert_eq!(pick(35 + 30), "weather");
    }

    #[test]
    fn user_config_2_parses() {
        let txt = "cpu_size = 5\n#cpu_color = #FFC800\ncpu_position = top-left\n\ngpu_size = 5\ngpu_position = top-right\n\nram_size = 5\nram_position = bottom-left\n\nnowplaying_size = 5\nnowplaying_position = bottom\n\nnet_size = 5\nnet_position = bottom-right\n\nhide = uptime, date, disk, volume, cpu_freq, gpu_fan, time\n\nclock_time_size = 24\n";
        let o = parse_ui_options(&ConfigFile::parse(txt).unwrap()).unwrap();
        assert_eq!(o.clock_time_size, 24);
        assert_eq!(o.layouts[0].0.name, "default2");
    }

    #[test]
    fn units_format() {
        assert_eq!(fmt_net(2300.0, NetUnit::Kb), "2300KB/S");
        assert_eq!(fmt_net(2048.0, NetUnit::Mb), "2.0MB/S");
        assert_eq!(fmt_net(500.0, NetUnit::Auto), "500KB/S");
        assert_eq!(fmt_net(3072.0, NetUnit::Auto), "3.0MB/S");
        assert_eq!(fmt_mem(18432, 32768, true), "18.0/32GB");
        assert_eq!(fmt_mem(18432, 32768, false), "18432/32768MB");
    }

    #[test]
    fn text_backdrop_draws_panel_behind_clock() {
        let canvas = Orientation::Landscape.canvas();
        let count = |on: bool| {
            TEST_OPTS.with(|t| *t.borrow_mut() = Some(UiOptions {
                text_backdrop: on, panel_opacity: 50, show: Show::default(), ..UiOptions::default()
            }));
            let mut ui = Framebuffer::new(canvas);
            ui.clear(1, 0, 2);
            draw_idle_clock(&mut ui, ColorMode::Default);
            ui.as_bytes().chunks_exact(3).filter(|p| **p == [2, 0, 1]).count()
        };
        let (off, on) = (count(false), count(true));
        TEST_OPTS.with(|t| *t.borrow_mut() = None);
        assert_eq!(off, 0);
        assert!(on > 10_000, "{on}");
    }

    /// Layout scenarios: no panics, and with TROFEO_DUMP_DIR, saves PNGs.
    #[test]
    fn layout_scenarios() {
        let scenarios: Vec<(&str, Orientation, UiOptions, bool)> = vec![
            ("s1_bottomleft_list_redtext", Orientation::Landscape, UiOptions {
                status_pos: Anchor::parse("bottom-left").unwrap(),
                status_style: StatusStyle::List,
                text_color: Some((255, 60, 60)),
                clock_pos: Anchor::parse("top-right").unwrap(),
                clock_color: Some((80, 200, 255)),
                show: { let mut s = Show::default(); s.clock_date = false; s.disk = false; s },
                ..UiOptions::default()
            }, true),
            ("s2_spectrum_small_topright", Orientation::Landscape, UiOptions {
                spectrum_pos: Anchor::parse("top-right").unwrap(),
                spectrum_w: 40,
                spectrum_h: 50,
                status_pos: Anchor::parse("center-left").unwrap(),
                status_style: StatusStyle::List,
                ..UiOptions::default()
            }, false),
            ("s3_only_cpu_time_bottom", Orientation::Landscape, UiOptions {
                show: { let mut s = Show::all(false); s.cpu = true; s.time = true; s.nowplaying = true; s.spectrum = true; s },
                status_pos: Anchor::parse("bottom-right").unwrap(),
                ..UiOptions::default()
            }, false),
            ("s5_items_sizes", Orientation::Landscape, {
                let mut it = [ItemStyle::default(); 11];
                it[0] = ItemStyle { size: Some(5), pos: Some(Anchor::parse("top-left").unwrap()), color: Some((255, 200, 0)), backdrop: None };
                it[5] = ItemStyle { size: Some(2), pos: Some(Anchor::parse("bottom-left").unwrap()), color: None, backdrop: None };
                it[1] = ItemStyle { size: Some(4), pos: Some(Anchor::parse("top-right").unwrap()), color: None, backdrop: None };
                it[3] = ItemStyle { size: Some(6), pos: Some(Anchor::parse("bottom-right").unwrap()), color: None, backdrop: None };
                it[9] = ItemStyle { size: Some(3), pos: Some(Anchor::parse("bottom").unwrap()), color: None, backdrop: None };
                let mut sh = Show::default();
                sh.cpu_freq = false; sh.gpu_fan = false; sh.date = false; sh.uptime = false; sh.net = false; sh.disk = false; sh.volume = false;
                UiOptions { items: it, status_style: StatusStyle::Items, show: sh, clock_time_size: 12, ..UiOptions::default() }
            }, true),
            ("s6_user_config", Orientation::Landscape, {
                let mut it = [ItemStyle::default(); 11];
                let mk = |sz: u32, pos: &str| ItemStyle { size: Some(sz), pos: Some(Anchor::parse(pos).unwrap()), color: None, backdrop: None };
                it[0] = mk(5, "top-left"); it[1] = mk(5, "top-right"); it[5] = mk(3, "bottom-left");
                it[9] = mk(3, "bottom"); it[6] = mk(3, "bottom-right");
                let mut sh = Show::default();
                sh.uptime = false; sh.date = false; sh.disk = false; sh.volume = false; sh.cpu_freq = false; sh.gpu_fan = false; sh.time = false;
                UiOptions { items: it, show: sh, status_style: StatusStyle::Items, nowplaying_label: false, clock_time_size: 24, ..UiOptions::default() }
            }, true),
            ("s4_portrait_bottom", Orientation::Portrait, UiOptions {
                status_pos: Anchor::parse("bottom").unwrap(),
                clock_pos: Anchor::parse("top-left").unwrap(),
                ..UiOptions::default()
            }, true),
            ("s7_weather_item", Orientation::Landscape, {
                let mut it = [ItemStyle::default(); 11];
                it[0] = ItemStyle { size: Some(4), pos: Some(Anchor::parse("top-left").unwrap()), color: None, backdrop: None };
                it[3] = ItemStyle { size: Some(4), pos: Some(Anchor::parse("top-right").unwrap()), color: None, backdrop: None };
                it[10] = ItemStyle { size: Some(5), pos: Some(Anchor::parse("bottom-right").unwrap()), color: Some((80, 170, 255)), backdrop: Some(true) };
                let mut sh = Show::all(false);
                sh.cpu = true; sh.time = true; sh.weather = true; sh.clock = true; sh.clock_date = true;
                UiOptions { items: it, status_style: StatusStyle::Items, show: sh, clock_time_size: 20, ..UiOptions::default() }
            }, true),
        ];
        let weather_sample = weather::WeatherSnapshot {
            temp_c: 18.0,
            humidity: Some(64),
            code: 2,
            icon: weather_icon::WeatherIcon::PartlyCloudy,
            city: Some("Milano".into()),
        };
        let sys = System::new_all();
        let gpu = gpu_amd::GpuAmdData::default();
        for (name, orient, o, idle) in scenarios {
            TEST_OPTS.with(|t| *t.borrow_mut() = Some(o));
            let mut fb = Framebuffer::new(orient.canvas());
            fb.clear(0x08, 0x08, 0x10);
            if idle {
                draw_idle_clock(&mut fb, ColorMode::Default);
            } else {
                draw_bars(&mut fb, &vec![0.7f32; NUM_BARS], ColorMode::Default);
            }
            let mut marquee = Marquee::new();
            draw_status_lines(
                &mut fb, &sys, Some(87.0), &gpu, (123.0, 45.0), (1.5, 0.2), Some((40.0, false)),
                Some(61.0), Some(88.0), Some(4900), Some("Artist - A very long song title for the marquee bar"),
                Some(&weather_sample),
                &mut marquee,
            );
            if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                std::fs::write(
                    format!("{dir}/{name}.ppm"),
                    [format!("P6\n{} {}\n255\n", fb.width(), fb.height()).into_bytes(), fb.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
        TEST_OPTS.with(|t| *t.borrow_mut() = None);
    }

    /// Italian + margins + background: compositing without panics; with
    /// TROFEO_DUMP_DIR, saves the result.
    #[test]
    fn italian_with_margin_and_background() {
        i18n::set_language(Lang::It);
        let canvas = Orientation::Landscape.canvas();
        let margins = Margins { top: 40, bottom: 10, left: 30, right: 30 };
        let (iw, ih) = margins.inner(canvas.width, canvas.height).unwrap();
        let mut ui = Framebuffer::new(trofeo_lcd::Resolution::new(iw, ih));
        ui.clear(1, 0, 2);
        let sys = System::new_all();
        let gpu = gpu_amd::GpuAmdData::default();
        let mut marquee = Marquee::new();
        draw_bars(&mut ui, &vec![0.5f32; NUM_BARS], ColorMode::Default);
        draw_status_lines(
            &mut ui, &sys, Some(87.0), &gpu, (123.0, 45.0), (1.5, 0.2), Some((40.0, false)),
            Some(61.0), Some(88.0), Some(4900), Some("Perche' e' cosi' - Caffe"), None, &mut marquee,
        );
        let mut fb = Framebuffer::new(canvas);
        let mut px = vec![0u8; (canvas.width * canvas.height * 3) as usize];
        for (i, p) in px.chunks_exact_mut(3).enumerate() {
            let x = (i as u32 % canvas.width) as u8;
            p.copy_from_slice(&[x / 3, 40, 90]);
        }
        fb.as_bytes_mut().copy_from_slice(&px);
        fb.blit_keyed(&ui, margins.left, margins.top, (1, 0, 2));
        i18n::set_language(Lang::En);
        if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
            std::fs::write(
                format!("{dir}/it_margin_bg.ppm"),
                [format!("P6\n{} {}\n255\n", fb.width(), fb.height()).into_bytes(), fb.as_bytes().to_vec()].concat(),
            ).unwrap();
        }
    }
}
