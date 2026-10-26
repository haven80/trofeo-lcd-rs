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
mod lhm;
mod gpu;
mod gpu_amd;
mod media;
mod music_screen;
mod netdisk;
mod openrgb_sync;
mod pawnio;
mod effect;
mod pixel_game;
mod spectrum;
mod frame;
mod tiles;
mod ticker;
mod weather;
mod weather_icon;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;
use sysinfo::System;
use trofeo_lcd::background::Background;
use trofeo_lcd::config::{ConfigFile, Margins};
use trofeo_lcd::i18n::Lang;
use trofeo_lcd::layout::{Anchor, BgLayout, Fit, Show};
use trofeo_lcd::i18n;
use trofeo_lcd::{Framebuffer, LyLcd, Orientation};
use trofeo_lcd::{hotkey, png_save, smooth_font};

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
    /// Size of the panel's picture (landscape): `panel` / `panel_size`.
    panel: trofeo_lcd::Resolution,
    /// Neither `panel` nor `panel_size` set: the model recognised at the handshake decides.
    panel_auto: bool,
    /// Extra 180° rotation (`--flip` / `flip = true`), if the result comes out upside down.
    flip: bool,
    /// HID screens (Trofeo Vision 6.86): how the panel is mounted. `None` = from the handshake.
    hid_mount: Option<trofeo_lcd::Rotation>,
    /// HID screens: write the panel's own size in the frame header (`hid_header = panel`).
    hid_header_panel: bool,
    /// Margins (px) to keep the interface away from the screen edges.
    margins: Margins,
    language: Lang,
    /// Background: an image (jpg/png/bmp), an animated gif, or a video (requires ffmpeg).
    background: Option<std::path::PathBuf>,
    /// How much to darken the background, 0-100 (for better text readability).
    background_dim: u8,
    /// A solid background color (`background_color = black | white | pink | #1B2A4A ...`),
    /// used when there is no image and no animated effect.
    background_color: Option<(u8, u8, u8)>,
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
    /// Ticker source(s) (`ticker_source = ...` in trofeo.conf, comma-separated:
    /// RSS/Atom feed URLs, plain-text URLs, or local file paths — see
    /// `ticker.rs`). `None` = ticker disabled. Changing it requires a restart
    /// (it re-spawns the background fetch thread), like `weather_city`.
    ticker_source: Option<String>,
    /// How often to re-fetch the ticker sources, in minutes. Restart-required.
    ticker_refresh_min: u32,
    /// Background position/fit.
    bg_layout: BgLayout,
    ui: UiOptions,
    /// `--preview-png`: draw ONE frame with sample data into a PNG and exit (used by the
    /// settings editor `trofeo_config` for its live preview; nothing is sent to the LCD).
    preview: Option<PreviewReq>,
}

/// What the settings editor asks `--preview-png` to draw.
#[derive(Clone, Debug, PartialEq)]
struct PreviewReq {
    path: std::path::PathBuf,
    /// Time (seconds) for the animated backgrounds.
    secs: f64,
    state: PreviewState,
    /// Which entry of the `layout` list to show (0 = the first).
    layout: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewState {
    /// Nothing is playing: the big clock / the panel layout.
    Idle,
    /// Music is playing: the spectrum.
    Playing,
    /// The now-playing card (only when `music_screen = true`).
    Music,
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
         \x20\x20                          considered silent (default: {DEFAULT_SILENCE_THRESHOLD}).\n\
         \x20\x20                          Also settable as silence_threshold in trofeo.conf\n\
         \x20\x20                          (applies live, no restart) — lower it if the EQ bars\n\
         \x20\x20                          don't react at low volume (loopback capture reflects\n\
         \x20\x20                          the system/app volume level).\n\
         \x20\x20--silence-timeout-ms <N>  How long it must stay silent before dropping to\n\
         \x20\x20                          idle-fps, in milliseconds (default: {DEFAULT_SILENCE_TIMEOUT_MS}).\n\
         \x20\x20                          Also settable as silence_timeout_ms in trofeo.conf.\n\
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
         \x20\x20--lcd-test               Trofeo Vision 6.86 (0416:5302): shows 8 test pictures to find\n\
         \x20\x20                          the right hid_mount / hid_header, then exits\n\
         \x20\x20--hid-mount <D>          6.86: auto | 0 | 90 | 180 | 270 (degrees); --hid-header raster|panel\n\
         \x20\x20--orientation <O>       'landscape' (default) or 'portrait' (screen mounted upright)\n\
         \x20\x20--panel <MODEL>         trofeo-9.16 (default, 1920x462) or trofeo-11.3 (1920x400)\n\
         \x20\x20--panel-size <WxH>      Picture size in pixels for any other panel (e.g. 1920x440)\n\
         \x20\x20--flip                   Extra 180° rotation (if the display comes out upside down)\n\
         \x20\x20--margin <PX>            Keep the interface away from the edges (all sides);\n\
         \x20\x20--margin-top/-bottom/-left/-right <PX>  margin for a single side\n\
         \x20\x20--language <en|it|de|es|fr|pt>  Interface language (default: en)\n\
         \x20\x20--background <FILE>      Background: jpg/png/bmp, animated gif, or video\n\
         \x20\x20                          (videos require ffmpeg)\n\
         \x20\x20--background-color <C>   Solid background color (name or #RRGGBB), when there is no image/effect\n\
         \x20\x20--background-dim <0-100> Darkens the background (default: 40)\n\
         \x20\x20--ffmpeg <PATH>      Path to ffmpeg (default: next to the exe/PATH)\n\
         \x20\x20--lhm-list              Lists what LibreHardwareMonitor reports (fans, temperatures)\n\
         \x20\x20                          and which sensor is used for each role, then exits.\n\
         \x20\x20--lhm <true|false>      Read fan speeds etc. from LibreHardwareMonitor's web\n\
         \x20\x20                          server (Options > Remote Web Server > Run). Also\n\
         \x20\x20                          --lhm-url, --lhm-interval, --lhm-cpu-fan, --lhm-case-fan,\n\
         \x20\x20                          --lhm-pump, --lhm-mb-temp (part of a sensor's name).\n\
         \x20\x20--deepcool-source <S>    DeepCool case display section: cpu | gpu | alternate (in turn)\n\
         \x20\x20--deepcool-mode <M>      CPU section of the CH170/CH270/CH690 display: freq | fan |\n\
         \x20\x20                          alternate (fan = RPM from LibreHardwareMonitor);\n\
         \x20\x20                          --deepcool-fan cpu|case|pump|gpu, --deepcool-alternate <s>.\n\
         \x20\x20--weather-city <NAME>    City for the weather module (default: auto-detect\n\
         \x20\x20                          from the machine's public IP address)\n\
         \x20\x20ticker_source <URL/FILE> (config-file only) News ticker source(s), comma-\n\
         \x20\x20                          separated: RSS/Atom feed URL, plain-text URL, or a\n\
         \x20\x20                          local file (one item per line). Shown as a scrolling\n\
         \x20\x20                          line (show/hide = ticker) and/or --layout news.\n\
         \x20\x20                          Requires a restart, like --weather-city.\n\
         \x20\x20ticker_refresh_min <N>   (config-file only) How often to re-fetch, in minutes\n\
         \x20\x20                          (default: 10). Requires a restart.\n\
         \x20\x20--ticker-position <P>    Ticker line position (default: bottom)\n\
         \x20\x20--ticker-size <1-30>     Ticker line text size (default: 3)\n\
         \x20\x20--ticker-color <COLOR>   Ticker line color\n\
         \x20\x20--show <LIST>           Show ONLY these elements (comma-separated):\n\
         \x20\x20                          cpu, gpu, uptime, time, date, mem, net, disk, volume,\n\
         \x20\x20                          nowplaying, weather, ticker, spectrum, clock, clock_date, dashboard\n\
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
         \x20\x20--layout-hold-on-music <true|false>  While music plays, stay on the first layout (no rotation)\n\
         \x20\x20--music-screen <true|false>  While music plays, show a now-playing card (cover, title, progress, spectrum)\n\
         \x20\x20--music-progress <true|false>  Music screen: progress bar and times (default: true)\n\
         \x20\x20--music-spectrum-color <cover|main|COLOR|PRESET>  Music screen spectrum color (default: cover)\n\
         \x20\x20--font <pixel|smooth>  Font of all the text: the 5x7 pixel font (default) or smooth anti-aliased Poppins\n\
         \x20\x20--font-weight <auto|light|regular|medium>  Weight of the smooth font (default: by text size)\n\
         \x20\x20--pixel-game <true|false>  Pixel-art mini game under the clock on the default screen (default: true)\n\
         \x20\x20--pixel-game-hero <knight|cat|both>, --pixel-game-height <20-70>, --pixel-game-hud <true|false>\n\
         \x20\x20--background-effect <plasma|aurora|lava|waves|matrix|rain|aquarium|network|globe|none>  Animated background instead of an image (default: none)\n\
         \x20\x20--effect-palette <P>    pastel | fluo | neon | candy | sunset | ocean | aurora | fire | forest | ice | mono | rainbow |\n         \x20\x20                          matrix | amber | violet | crimson | snow (one-colour, for matrix/rain) |\n         \x20\x20                          lagoon | deep | tropical (for aquarium) | all\n\
         \x20\x20--effect-colors <C>     Own colors: 2+ colors, e.g. #FF0080,#00FFFF (used when --effect-palette is not set)\n\
         \x20\x20--effect-speed <0-100>, --effect-scale <25-400>, --effect-brightness <0-100>, --effect-fps <1-30>, --effect-level <0-100> (aquarium: how high the water is; network / globe: how many nodes)\n\
         \x20\x20--effect-theme <dark|light>  matrix / rain / aquarium / network / globe on a dark (default) or a white background\n\
         \x20\x20                          (defaults: 30, 100, 40, 8); --effect-palette-interval <SEC> rotates several palettes\n\
         \x20\x20--music-stats <list>      Music screen footer: cpu, gpu, ram, net, disk, time, uptime, all or none (default: cpu,gpu,ram)\n\
         \x20\x20--music-bg <gradient|blur|color|solid|none>  Music screen background (default: gradient)\n\
         \x20\x20--music-bg-brightness / --music-bg-blur <0-100>  Background brightness (40) / blur amount (70)\n\
         \x20\x20--music-bg-fit <center|stretch>, --music-bg-color <color>  Blur fit; color for 'solid'\n\
         \x20\x20--spectrum-style <S>     bars | led | peaks | area | mirror | all (several, comma-separated, rotate)\n\
         \x20\x20--spectrum-palette <P>   default | rainbow | fire | ocean | sunset | neon | ice | matrix | purple | custom | all\n\
         \x20\x20--spectrum-gradient <C>  Custom palette: 2+ colors, e.g. #FF0080,#00FFFF\n\
         \x20\x20--spectrum-style-interval / --spectrum-palette-interval <SEC>  Rotation period (default 30 / 20)\n\
         \x20\x20--spectrum-rainbow-speed <DEG/S>  Hue scroll of the rainbow palette (default 30, 0 = static)\n\
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

/// Common color names that can be used directly without knowing the hex/RGB code
/// (English, Italian and Indonesian aliases for the basic ones; spaces, `-` and `_` are
/// ignored: `dark green` = `darkgreen`).
fn named_color(name: &str) -> Option<(u8, u8, u8)> {
    let key: String = name.to_ascii_lowercase().chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect();
    Some(match key.as_str() {
        "black" | "nero" | "hitam" => (0x00, 0x00, 0x00),
        "white" | "putih" | "bianco" => (0xFF, 0xFF, 0xFF),
        "gray" | "grey" | "grigio" | "abu" => (0x80, 0x80, 0x80),
        "lightgray" | "lightgrey" | "grigiochiaro" => (0xD3, 0xD3, 0xD3),
        "darkgray" | "darkgrey" | "grigioscuro" => (0x40, 0x40, 0x40),
        "silver" | "argento" => (0xC0, 0xC0, 0xC0),
        "red" | "merah" | "rosso" => (0xFF, 0x00, 0x00),
        "darkred" | "maroon" | "bordeaux" => (0x80, 0x00, 0x00),
        "green" | "hijau" | "verde" | "lime" => (0x00, 0xFF, 0x00),
        "darkgreen" | "verdescuro" => (0x00, 0x64, 0x00),
        "lightgreen" | "verdechiaro" => (0x90, 0xEE, 0x90),
        "olive" | "oliva" => (0x80, 0x80, 0x00),
        "blue" | "biru" | "blu" => (0x00, 0x00, 0xFF),
        "navy" | "darkblue" | "bluscuro" | "blunotte" => (0x00, 0x00, 0x80),
        "lightblue" | "bluchiaro" => (0xAD, 0xD8, 0xE6),
        "skyblue" | "celeste" => (0x87, 0xCE, 0xEB),
        "yellow" | "kuning" | "giallo" => (0xFF, 0xFF, 0x00),
        "gold" | "oro" => (0xFF, 0xD7, 0x00),
        "cyan" | "azzurro" | "ciano" | "aqua" => (0x00, 0xFF, 0xFF),
        "teal" | "verdeacqua" => (0x00, 0x80, 0x80),
        "turquoise" | "turchese" => (0x40, 0xE0, 0xD0),
        "magenta" | "fucsia" | "fuchsia" => (0xFF, 0x00, 0xFF),
        "pink" | "rosa" => (0xFF, 0xC0, 0xCB),
        "hotpink" | "rosafluo" => (0xFF, 0x69, 0xB4),
        "salmon" | "salmone" => (0xFA, 0x80, 0x72),
        "coral" | "corallo" => (0xFF, 0x7F, 0x50),
        "orange" | "oranye" | "arancione" => (0xFF, 0xA5, 0x00),
        "brown" | "marrone" => (0x8B, 0x45, 0x13),
        "beige" => (0xF5, 0xF5, 0xDC),
        "purple" | "ungu" | "viola" => (0x80, 0x00, 0x80),
        "violet" | "violetto" => (0xEE, 0x82, 0xEE),
        "indigo" => (0x4B, 0x00, 0x82),
        "lavender" | "lavanda" => (0xE6, 0xE6, 0xFA),
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
    let (mut preview_path, mut preview_secs, mut preview_state, mut preview_layout) =
        (None::<std::path::PathBuf>, 0.0f64, PreviewState::Idle, 0usize);

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
                    .ok_or_else(|| anyhow::anyhow!("--language requires one of: {}", Lang::NAMES))?;
                language_cli = Some(Lang::parse(&raw).ok_or_else(|| {
                    anyhow::anyhow!("--language: '{raw}' not valid ({})", Lang::NAMES)
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
            "--preview-png" => {
                preview_path = Some(
                    args.next().ok_or_else(|| anyhow::anyhow!("--preview-png requires a file path"))?.into(),
                );
            }
            "--preview-time" => preview_secs = next_f32(&mut args, "--preview-time")? as f64,
            "--preview-layout" => preview_layout = next_u64(&mut args, "--preview-layout")? as usize,
            "--preview-state" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--preview-state requires idle | playing | music"))?;
                preview_state = match raw.to_ascii_lowercase().as_str() {
                    "idle" => PreviewState::Idle,
                    "playing" | "spectrum" => PreviewState::Playing,
                    "music" => PreviewState::Music,
                    _ => anyhow::bail!("--preview-state: '{raw}' not valid (idle | playing | music)"),
                };
            }
            "--lhm-list" => {
                // Handled in `main()` after parsing (needs the lhm_* settings).
            }
            "--lcd-test" => {
                // Handled in `main()` after the handshake.
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
    let panel = file.panel_resolution().map_err(|e| anyhow::anyhow!(e))?;
    let panel_auto = file.panel_is_auto();
    let flip = match flip_cli {
        Some(f) => f,
        None => file.get_bool("flip").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false),
    };
    let hid_mount = match file.get("hid_mount").map(|v| v.trim().to_ascii_lowercase()) {
        None => None,
        Some(v) if v == "auto" || v.is_empty() => None,
        Some(v) => Some(
            v.parse::<u32>()
                .ok()
                .and_then(trofeo_lcd::Rotation::from_degrees)
                .ok_or_else(|| anyhow::anyhow!("hid_mount: '{v}' not valid (auto | 0 | 90 | 180 | 270)"))?,
        ),
    };
    let hid_header_panel = match file.get("hid_header").map(|v| v.trim().to_ascii_lowercase()) {
        None => false,
        Some(v) if v == "raster" || v == "picture" => false,
        Some(v) if v == "panel" => true,
        Some(v) => anyhow::bail!("hid_header: '{v}' not valid (raster | panel)"),
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
                .ok_or_else(|| anyhow::anyhow!("language: '{v}' not valid ({})", Lang::NAMES))?,
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
    let background_color = parse_color_opt(&file, "background_color")?;
    let ffmpeg = ffmpeg_cli.or_else(|| file.get("ffmpeg").map(str::to_string));
    let weather_city = weather_city_cli.or_else(|| file.get("weather_city").map(str::to_string));
    let ticker_source = file.get("ticker_source").map(str::to_string);
    let ticker_refresh_min = file.get_u32("ticker_refresh_min").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(10);
    if !(1..=1440).contains(&ticker_refresh_min) {
        anyhow::bail!("ticker_refresh_min: must be between 1 and 1440 (minutes)");
    }

    Ok(Config {
        config_explicit: config_path.clone(),
        weather_city,
        ticker_source,
        ticker_refresh_min,
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
        panel,
        panel_auto,
        flip,
        hid_mount,
        hid_header_panel,
        margins,
        language,
        background,
        background_dim: background_dim as u8,
        background_color,
        ffmpeg,
        brightness: brightness as u8,
        fps_monitor,
        bg_layout,
        ui,
        preview: preview_path
            .map(|path| PreviewReq { path, secs: preview_secs, state: preview_state, layout: preview_layout }),
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
    "background_color", "background_fit", "background_position", "background_offset_x", "background_offset_y",
    "brightness", "deepcool", "fps_monitor", "layout", "layout_spectrum",
    "layout_hold_on_music", "layout_interval", "panel_opacity", "text_backdrop",
    "spectrum_style", "spectrum_style_interval", "spectrum_palette",
    "spectrum_palette_interval", "spectrum_gradient", "spectrum_rainbow_speed",
    "music_screen", "music_blur", "music_progress", "music_bg", "music_bg_brightness",
    "music_bg_blur", "music_bg_fit", "music_bg_color", "music_stats", "music_spectrum_color",
    "pixel_game", "pixel_game_hero", "pixel_game_height", "pixel_game_hud",
    "background_effect", "effect_palette", "effect_colors", "effect_palette_interval",
    "effect_speed", "effect_scale", "effect_brightness", "effect_fps", "effect_level", "effect_theme",
    "panel", "panel_size", "font", "font_weight",
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

/// Formats a CPU/GPU/RAM usage percentage with the configured number of
/// decimal places (`percent_decimals`, 0 by default — same "37%" as before
/// this option existed; 1 gives "37.4%"). Used everywhere a usage
/// percentage is shown, so all three stay consistent with each other.
fn fmt_pct(v: f32) -> String {
    format!("{:.*}%", opts().percent_decimals as usize, v)
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
    OVERRIDE_KEYS.contains(&key) || item_key(key).is_some() || key == "net_unit" || key == "mem_unit" || key == "ram_unit" || key == "nowplaying_width" || key == "nowplaying_label" || key == "clock_backdrop" || key == "clock_time_size" || key == "clock_date_size" || key == "weather_unit" || key == "ticker_position" || key == "ticker_size" || key == "ticker_color" || key == "ticker_backdrop" || key == "ticker_width" || key == "percent_decimals" || key == "font" || key == "font_weight" || key == "tiles" || key.starts_with("tiles_") || key.starts_with("frame_") || key == "lhm" || key.starts_with("lhm_") || key.starts_with("dash_") || key == "deck" || key == "deck_gauge" || key == "cards" || key.starts_with("cards_") || key == "orbit" || key.starts_with("orbit_") || key.starts_with("hid_") || key == "deepcool_source" || key == "deepcool_mode" || key == "deepcool_fan" || key == "deepcool_alternate" || (key.starts_with("tile_") && key.ends_with("_color"))
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

/// Which entry of `layout = a:30, b:5` is on screen `elapsed_secs` after the
/// rotation started: walks the cumulative durations modulo the cycle length.
/// With `hold_first` (music playing + `layout_hold_on_music`) it is always the
/// first entry.
fn rotation_index(layouts: &[(&'static layouts::LayoutDef, u32)], elapsed_secs: u64, hold_first: bool) -> usize {
    if hold_first || layouts.is_empty() {
        return 0;
    }
    let total: u64 = layouts.iter().map(|(_, secs)| *secs as u64).sum::<u64>().max(1);
    let t = elapsed_secs % total;
    let mut acc = 0u64;
    for (i, (_, secs)) in layouts.iter().enumerate() {
        acc += *secs as u64;
        if t < acc {
            return i;
        }
    }
    layouts.len() - 1
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
        font: match file.get("font").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("pixel") | Some("") => trofeo_lcd::TextFont::Pixel,
            Some("smooth") | Some("poppins") | Some("morbido") => trofeo_lcd::TextFont::Smooth,
            Some(v) => anyhow::bail!("font: '{v}' not valid (pixel | smooth)"),
        },
        font_weight: match file.get("font_weight").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("auto") | Some("") => None,
            Some("light") | Some("leggero") => Some(smooth_font::Weight::Light),
            Some("regular") | Some("normal") | Some("normale") => Some(smooth_font::Weight::Regular),
            Some("medium") | Some("medio") => Some(smooth_font::Weight::Medium),
            Some(v) => anyhow::bail!("font_weight: '{v}' not valid (auto | light | regular | medium)"),
        },
        percent_decimals: match file.get_u32("percent_decimals").map_err(|e| anyhow::anyhow!(e))? {
            None => 0,
            Some(0) => 0,
            Some(1) => 1,
            Some(v) => anyhow::bail!("percent_decimals: must be 0 or 1 (got {v})"),
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
        ticker_pos: parse_anchor_opt(file, "ticker_position", Anchor { ax: 1, ay: 2 })?,
        ticker_size: parse_size_opt(file, "ticker_size", 3)?,
        ticker_color: parse_color_opt(file, "ticker_color")?,
        ticker_backdrop: file.get_bool("ticker_backdrop").map_err(|e| anyhow::anyhow!(e))?,
        ticker_width: parse_percent(file, "ticker_width")?,
        layout_hold_on_music: file.get_bool("layout_hold_on_music").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false),
        spectrum_styles: parse_spectrum_styles(file)?,
        spectrum_style_interval: parse_interval(file, "spectrum_style_interval", 30)?,
        spectrum_palettes: parse_spectrum_palettes(file)?,
        spectrum_palette_interval: parse_interval(file, "spectrum_palette_interval", 20)?,
        spectrum_rainbow_speed: match file.get_u32("spectrum_rainbow_speed").map_err(|e| anyhow::anyhow!(e))? {
            None => 30,
            Some(v) if v <= 360 => v,
            Some(v) => anyhow::bail!("spectrum_rainbow_speed: must be between 0 and 360 (got {v})"),
        },
        music_screen: file.get_bool("music_screen").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false),
        music_bg: parse_music_bg(file)?,
        music_stats: parse_music_stats(file)?,
        music_spectrum: parse_music_spectrum(file)?,
        pixel_game: file.get_bool("pixel_game").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(true),
        pixel_game_hero: match file.get("pixel_game_hero") {
            None => pixel_game::HeroChoice::Both,
            Some(v) => pixel_game::HeroChoice::parse(v)
                .ok_or_else(|| anyhow::anyhow!("pixel_game_hero: '{v}' not valid ({})", pixel_game::HeroChoice::NAMES))?,
        },
        pixel_game_height: match file.get_u32("pixel_game_height").map_err(|e| anyhow::anyhow!(e))? {
            None => 46,
            Some(v) if (20..=70).contains(&v) => v,
            Some(v) => anyhow::bail!("pixel_game_height: must be between 20 and 70 (got {v})"),
        },
        pixel_game_hud: file.get_bool("pixel_game_hud").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(true),
        music_stats_set: file.get("music_stats").is_some(),
        music_progress: file.get_bool("music_progress").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(true),
        effect: parse_effect(file)?,
        tiles: tiles::parse(file)?,
        frame: frame::parse(file)?,
        lhm: lhm::parse(file)?,
        deepcool: deepcool::parse(file)?,
        dash: tiles::dash::parse(file)?,
    })
}

/// `background_effect` + `effect_*` keys -> the animated background settings.
fn parse_effect(file: &ConfigFile) -> anyhow::Result<effect::Settings> {
    let mut e = effect::Settings::default();
    if let Some(v) = file.get("background_effect") {
        e.style = match v.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "no" | "false" | "nessuno" | "" => None,
            other => Some(
                effect::Style::parse(other)
                    .ok_or_else(|| anyhow::anyhow!("background_effect: '{v}' not valid ({} | none)", effect::Style::NAMES))?,
            ),
        };
    }
    // Own colors (`effect_colors`) form the `custom` palette, and are used on their own
    // when `effect_palette` is not set.
    let custom = match file.get("effect_colors") {
        None => None,
        Some(list) => {
            let mut stops = Vec::new();
            for raw in list.split(',') {
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                match parse_color(raw).map_err(|e| anyhow::anyhow!("effect_colors: {e}"))? {
                    ColorMode::Custom(r, g, b) => stops.push((r, g, b)),
                    ColorMode::Default => anyhow::bail!("effect_colors: 'default' is not a color"),
                }
            }
            if stops.len() < 2 {
                anyhow::bail!("effect_colors: needs at least 2 colors, e.g. #FF0080, #00FFFF");
            }
            Some(effect::Palette::custom(stops))
        }
    };
    match file.get("effect_palette") {
        None => {
            if let Some(c) = custom {
                e.palettes = vec![c];
            }
        }
        Some(list) => {
            let mut out: Vec<effect::Palette> = Vec::new();
            for raw in list.split(',') {
                let name = raw.trim();
                if name.is_empty() {
                    continue;
                }
                if name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("rotate") {
                    out.extend(effect::Palette::presets());
                } else if name.eq_ignore_ascii_case("custom") {
                    out.push(custom.clone().ok_or_else(|| {
                        anyhow::anyhow!("effect_palette: 'custom' needs effect_colors = #RRGGBB, #RRGGBB")
                    })?);
                } else {
                    out.push(effect::Palette::parse(name).ok_or_else(|| {
                        anyhow::anyhow!("effect_palette: '{name}' not valid ({} | custom | all)", effect::Palette::NAMES)
                    })?);
                }
            }
            out.dedup();
            if !out.is_empty() {
                e.palettes = out;
            }
        }
    }
    // The rain styles look right in green and the aquarium in turquoise: those are the
    // defaults unless colors are chosen.
    if file.get("effect_palette").is_none() && file.get("effect_colors").is_none() {
        match e.style {
            Some(effect::Style::Matrix | effect::Style::Rain) => {
                e.palettes = vec![effect::Palette::parse("matrix").expect("preset")];
            }
            Some(effect::Style::Aquarium) => {
                e.palettes = vec![effect::Palette::parse("lagoon").expect("preset")];
            }
            Some(effect::Style::Network | effect::Style::Globe) => {
                e.palettes = vec![effect::Palette::parse("ice").expect("preset")];
            }
            _ => {}
        }
    }
    let ranged = |key: &str, default: u32, lo: u32, hi: u32| -> anyhow::Result<u32> {
        match file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            None => Ok(default),
            Some(v) if (lo..=hi).contains(&v) => Ok(v),
            Some(v) => anyhow::bail!("{key}: must be between {lo} and {hi} (got {v})"),
        }
    };
    e.speed = ranged("effect_speed", e.speed, 0, 100)?;
    e.scale = ranged("effect_scale", e.scale, 25, 400)?;
    e.light = match file.get("effect_theme").map(|v| v.trim().to_ascii_lowercase()) {
        None => false,
        Some(v) => match v.as_str() {
            "dark" | "scuro" | "black" | "nero" | "" => false,
            "light" | "chiaro" | "white" | "bianco" => true,
            _ => anyhow::bail!("effect_theme: '{v}' not valid (dark | light)"),
        },
    };
    // water needs more light than the other styles to look like water, and the light
    // variants need strong enough colors to stand out on white
    let default_brightness = match e.style {
        Some(effect::Style::Aquarium) => 65,
        Some(effect::Style::Matrix | effect::Style::Rain) if e.light => 65,
        Some(effect::Style::Network | effect::Style::Globe) => 70,
        _ => e.brightness,
    };
    e.brightness = ranged("effect_brightness", default_brightness, 0, 100)?;
    e.fps = ranged("effect_fps", e.fps, 1, 30)?;
    e.level = ranged("effect_level", e.level, 0, 100)?;
    e.palette_interval = ranged("effect_palette_interval", e.palette_interval, 5, 3600)?;
    Ok(e)
}

/// `music_spectrum_color = cover | main | #RRGGBB | #RRGGBB, #RRGGBB | fire, ocean | all`:
/// the spectrum's colors on the music screen. A single color is used flat, two or more
/// make a gradient (quiet -> loud), preset names (several rotate) use the presets.
/// `purple` is both a preset and a color name: the preset wins (use #800080 for the color).
fn parse_music_spectrum(file: &ConfigFile) -> anyhow::Result<music_screen::SpectrumColor> {
    use music_screen::SpectrumColor;
    let Some(list) = file.get("music_spectrum_color") else { return Ok(SpectrumColor::Cover) };
    let toks: Vec<&str> = list.split(',').map(str::trim).filter(|t| !t.is_empty()).collect();
    match toks.as_slice() {
        [] => return Ok(SpectrumColor::Cover),
        [t] if t.eq_ignore_ascii_case("cover") || t.eq_ignore_ascii_case("copertina") => return Ok(SpectrumColor::Cover),
        [t] if t.eq_ignore_ascii_case("main") || t.eq_ignore_ascii_case("same") => return Ok(SpectrumColor::Main),
        _ => {}
    }
    let is_preset = |t: &str| t.eq_ignore_ascii_case("all") || t.eq_ignore_ascii_case("rotate") || spectrum::Palette::parse(t).is_some();
    if toks.iter().all(|t| is_preset(t)) {
        let mut out = Vec::new();
        for t in &toks {
            if t.eq_ignore_ascii_case("all") || t.eq_ignore_ascii_case("rotate") {
                out.extend(spectrum::Palette::presets());
            } else if let Some(p) = spectrum::Palette::parse(t) {
                out.push(p);
            }
        }
        out.dedup();
        return Ok(SpectrumColor::Palettes(out));
    }
    let mut stops = Vec::new();
    for t in &toks {
        match parse_color(t) {
            Ok(ColorMode::Custom(r, g, b)) => stops.push((r, g, b)),
            _ => anyhow::bail!(
                "music_spectrum_color: '{t}' is not valid (cover | main | #RRGGBB | color names | {} | all)",
                spectrum::Palette::NAMES
            ),
        }
    }
    if stops.len() == 1 {
        stops.push(stops[0]); // one color = flat
    }
    Ok(SpectrumColor::Palettes(vec![spectrum::Palette::Custom(stops)]))
}

/// `music_stats = cpu, gpu, ram` / `all` / `none`: the small info line at the bottom
/// of the music screen (order is kept; unset = cpu, gpu, ram).
fn parse_music_stats(file: &ConfigFile) -> anyhow::Result<Vec<music_screen::StatItem>> {
    use music_screen::StatItem;
    let Some(list) = file.get("music_stats") else { return Ok(Vec::new()) };
    let mut out: Vec<StatItem> = Vec::new();
    for raw in list.split(',') {
        let name = raw.trim();
        if name.is_empty() || name.eq_ignore_ascii_case("none") || name.eq_ignore_ascii_case("off") {
            continue;
        }
        if name.eq_ignore_ascii_case("all") {
            for it in StatItem::ALL {
                if !out.contains(&it) {
                    out.push(it);
                }
            }
            continue;
        }
        let it = StatItem::parse(name)
            .ok_or_else(|| anyhow::anyhow!("music_stats: '{name}' not valid ({})", StatItem::NAMES))?;
        if !out.contains(&it) {
            out.push(it);
        }
    }
    Ok(out)
}

/// `music_bg*` keys -> the music screen background. The old `music_blur = false`
/// (no background of its own) still works when `music_bg` is not set.
fn parse_music_bg(file: &ConfigFile) -> anyhow::Result<music_screen::Bg> {
    use music_screen::{Bg, BgFit, BgMode};
    let mut bg = Bg::default();
    if let Some(v) = file.get("music_bg") {
        bg.mode = BgMode::parse(v).ok_or_else(|| anyhow::anyhow!("music_bg: '{v}' not valid ({})", BgMode::NAMES))?;
    } else if file.get_bool("music_blur").map_err(|e| anyhow::anyhow!(e))? == Some(false) {
        bg.mode = BgMode::None;
    }
    let pct = |key: &str, default: u32| -> anyhow::Result<u32> {
        let v = file.get_u32(key).map_err(|e| anyhow::anyhow!(e))?.unwrap_or(default);
        if v > 100 {
            anyhow::bail!("{key}: must be between 0 and 100 (got {v})");
        }
        Ok(v)
    };
    bg.brightness = pct("music_bg_brightness", bg.brightness)?;
    bg.blur = pct("music_bg_blur", bg.blur)?;
    if let Some(v) = file.get("music_bg_fit") {
        bg.fit = BgFit::parse(v).ok_or_else(|| anyhow::anyhow!("music_bg_fit: '{v}' not valid (center | stretch)"))?;
    }
    if let Some(c) = parse_color_opt(file, "music_bg_color")? {
        bg.color = c;
    }
    Ok(bg)
}

/// The music screen shows while it is enabled, the player reports a track,
/// sound is actually playing and the PC isn't busy gaming (the game dashboard wins).
fn music_screen_active(enabled: bool, has_track: bool, gaming: bool, sound_recent: bool) -> bool {
    enabled && has_track && !gaming && sound_recent
}

/// A rotation interval in seconds (2-3600), same bounds as `layout_interval`.
fn parse_interval(file: &ConfigFile, key: &str, default: u32) -> anyhow::Result<u32> {
    let v = file.get_u32(key).map_err(|e| anyhow::anyhow!(e))?.unwrap_or(default);
    if !(2..=3600).contains(&v) {
        anyhow::bail!("{key}: must be between 2 and 3600 seconds");
    }
    Ok(v)
}

/// `spectrum_style = led` / `led, area` / `all` (alias `rotate`): one entry is
/// fixed, several rotate. Unset or `bars` = the classic look (empty list).
fn parse_spectrum_styles(file: &ConfigFile) -> anyhow::Result<Vec<spectrum::Style>> {
    let Some(list) = file.get("spectrum_style") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for raw in list.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("rotate") {
            out.extend(spectrum::Style::ALL);
            continue;
        }
        let st = spectrum::Style::parse(name).ok_or_else(|| {
            anyhow::anyhow!("spectrum_style: '{name}' not valid ({} | all)", spectrum::Style::NAMES)
        })?;
        out.push(st);
    }
    out.dedup();
    if out == [spectrum::Style::Bars] {
        out.clear();
    }
    Ok(out)
}

/// `spectrum_palette = fire` / `fire, ocean` / `all` (alias `rotate`), plus an
/// optional `spectrum_gradient = #FF0080, #00FFFF` defining the `custom`
/// palette (and selecting it on its own when `spectrum_palette` is unset).
fn parse_spectrum_palettes(file: &ConfigFile) -> anyhow::Result<Vec<spectrum::Palette>> {
    let custom = match file.get("spectrum_gradient") {
        None => None,
        Some(list) => {
            let mut stops = Vec::new();
            for raw in list.split(',') {
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                match parse_color(raw).map_err(|e| anyhow::anyhow!("spectrum_gradient: {e}"))? {
                    ColorMode::Custom(r, g, b) => stops.push((r, g, b)),
                    ColorMode::Default => anyhow::bail!("spectrum_gradient: 'default' is not a color"),
                }
            }
            if stops.len() < 2 {
                anyhow::bail!("spectrum_gradient: needs at least 2 colors, e.g. #FF0080, #00FFFF");
            }
            Some(stops)
        }
    };
    let Some(list) = file.get("spectrum_palette") else {
        return Ok(custom.map(|c| vec![spectrum::Palette::Custom(c)]).unwrap_or_default());
    };
    let mut out = Vec::new();
    for raw in list.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("rotate") {
            out.extend(spectrum::Palette::presets());
            continue;
        }
        if name.eq_ignore_ascii_case("custom") {
            let c = custom.clone().ok_or_else(|| {
                anyhow::anyhow!("spectrum_palette: 'custom' needs spectrum_gradient = #RRGGBB, #RRGGBB")
            })?;
            out.push(spectrum::Palette::Custom(c));
            continue;
        }
        out.push(spectrum::Palette::parse(name).ok_or_else(|| {
            anyhow::anyhow!("spectrum_palette: '{name}' not valid ({} | custom | all)", spectrum::Palette::NAMES)
        })?);
    }
    out.dedup();
    Ok(out)
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
    let resolution = config.orientation.canvas_for(config.panel);
    let (ui_w, ui_h) = config
        .margins
        .inner(resolution.width, resolution.height)
        .map_err(|e| anyhow::anyhow!(e))?;
    let ui_resolution = trofeo_lcd::Resolution::new(ui_w, ui_h);
    // An animated effect (plasma, aurora, ...) takes the place of an image/video background.
    let effect_on = config.ui.effect.style.is_some();
    if effect_on && config.background.is_some() {
        println!("Background effect is on: the background image/video is not used.");
    }
    let background: Option<Background> = match config.background.as_ref().filter(|_| !effect_on) {
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
    } else if effect_on && config.ui.effect.is_moving() {
        // The effect drifts slowly: its own (low) frame rate is enough.
        config.idle_fps.max(config.ui.effect.fps as f32)
    } else {
        config.idle_fps
    };
    let composite = background.is_some() || effect_on || config.background_color.is_some() || !config.margins.is_zero() || config.ui.panel_opacity < 100;
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

fn hex_head(b: &[u8], n: usize) -> String {
    b.iter().take(n).map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

/// `--lcd-test` (HID screens): shows a test picture with its corners named, for every
/// combination of `hid_mount` and `hid_header`, five seconds each. The one that looks right
/// (text readable, TOP-LEFT in the top left corner) gives the values for trofeo.conf.
fn run_lcd_test(lcd: &LyLcd, hs: &mut trofeo_lcd::Handshake, panel: trofeo_lcd::Resolution) -> anyhow::Result<()> {
    use trofeo_lcd::Rotation;
    println!();
    println!("LCD test: every 5 seconds the picture changes. Note the number of the one that is upright");
    println!("(readable text, 'TOP LEFT' in the top left corner, the arrow pointing right).");
    let mounts = [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270];
    let mut n = 0;
    for header_panel in [false, true] {
        for m in mounts {
            n += 1;
            hs.mount = m;
            hs.header_panel_size = header_panel;
            hs.rotation = Rotation::R0;
            hs.brightness = 100;
            let label = format!("{n}/8  hid_mount = {}  hid_header = {}", m.degrees(), if header_panel { "panel" } else { "raster" });
            println!("  {label}");
            let mut fb = Framebuffer::new(panel);
            let (w, h) = (panel.width, panel.height);
            fb.clear(16, 20, 40);
            fb.fill_rect(0, 0, w, 6, 255, 200, 0);
            fb.fill_rect(0, h - 6, w, 6, 0, 200, 255);
            fb.fill_rect(0, 0, 6, h, 255, 60, 60);
            fb.fill_rect(w - 6, 0, 6, h, 60, 255, 60);
            fb.fill_rect(10, 10, 80, 80, 255, 60, 60);
            fb.draw_text(110, 24, "TOP LEFT", 255, 255, 255, 5);
            let bw = Framebuffer::text_width("BOTTOM RIGHT", 5);
            fb.draw_text(w.saturating_sub(bw + 24), h.saturating_sub(70), "BOTTOM RIGHT", 255, 255, 255, 5);
            fb.draw_text(w / 2 - 220, h / 2 - 40, "-----> RIGHT", 255, 255, 0, 6);
            fb.draw_text(w / 2 - 300, h / 2 + 40, &label, 200, 200, 200, 3);
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(5) {
                lcd.send_framebuffer(hs, &fb, 80)?;
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    println!("Done. Put the numbers of the right picture in trofeo.conf, e.g.:  hid_mount = 90   hid_header = raster");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let diag_mode = std::env::args().any(|a| a == "--diag");
    let lcd_test = std::env::args().any(|a| a == "--lcd-test");
    let lhm_list = std::env::args().any(|a| a == "--lhm-list");
    let mut config = parse_args()?;
    if lhm_list {
        let ok = lhm::diag(&config.ui.lhm.url, &config.ui.lhm.filters);
        std::process::exit(if ok { 0 } else { 1 });
    }
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

    if let Some(req) = config.preview.clone() {
        return run_preview(&config, &req);
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
    let hid_default_mount = hs.mount;
    if lcd.variant() == trofeo_lcd::Variant::Hid2 {
        println!("HID screen (0416:5302): handshake reply {}", hex_head(&hs.raw_response, 40));
        match hs.native {
            Some(r) => println!("  model PM={} SUB={} -> picture {}x{}", hs.pm, hs.sub, r.width, r.height),
            None => println!("  model PM={} SUB={} is not in the table: using the panel size of the configuration (panel_size)", hs.pm, hs.sub),
        }
        if trofeo_lcd::hid2::needs_rgb565(hs.pm) {
            anyhow::bail!("this HID screen (PM={}) takes RGB565 pictures, which this program does not send yet. Please report the model.", hs.pm);
        }
        hs.mount = config.hid_mount.unwrap_or(hid_default_mount);
        hs.header_panel_size = config.hid_header_panel;
        println!("  mount {}° (hid_mount = {}), frame header size: {}",
            hs.mount.degrees(),
            config.hid_mount.map_or("auto".to_string(), |r| r.degrees().to_string()),
            if hs.header_panel_size { "panel" } else { "picture" });
        trofeo_lcd::set_detected_panel(None);
        trofeo_lcd::set_detected_resolution(hs.native);
        if config.panel_auto {
            if let Some(r) = hs.native {
                config.panel = r;
            }
        }
        if lcd_test {
            return run_lcd_test(&lcd, &mut hs, config.panel);
        }
    } else if lcd_test {
        anyhow::bail!("--lcd-test is for the HID screens (Trofeo Vision 6.86, USB 0416:5302)");
    }
    trofeo_lcd::set_detected_panel(trofeo_lcd::panel_from_handshake(&hs.raw_response));
    if config.panel_auto {
        if let Some(p) = trofeo_lcd::detected_panel() {
            config.panel = p.resolution();
            println!("Panel recognised from the handshake: Trofeo Vision 11.3\" ({}x{})", config.panel.width, config.panel.height);
        }
    }
    println!(
        "Orientation: {:?} (canvas {}x{}, panel {}x{}, rotation {}°)",
        config.orientation,
        config.orientation.canvas_for(config.panel).width,
        config.orientation.canvas_for(config.panel).height,
        config.panel.width,
        config.panel.height,
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

    // Ticker (news headlines or any custom feed, see ticker.rs): same
    // background-thread pattern as weather, refreshed every `ticker_refresh_min`.
    let ticker_sources: Vec<String> = config
        .ticker_source
        .as_deref()
        .map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    let ticker_monitor =
        ticker::TickerMonitor::spawn(ticker_sources, Duration::from_secs(config.ticker_refresh_min as u64 * 60));
    let mut latest_ticker: Vec<String> = Vec::new();
    let mut ticker_marquee = Marquee::new();

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
    let (now_playing, track_shared) = media::spawn_track_watcher()?;
    let mut music = music_screen::MusicScreen::new();
    let mut game = pixel_game::PixelGame::new();
    let mut bg_effect = effect::Effect::new();
    let mut now_playing_marquee = Marquee::new();

    let mut latest_gpu_percent: Option<f32> = None;
    let mut latest_net_kb = (0.0f64, 0.0f64); // (down, up)
    let mut latest_disk_mb = (0.0f64, 0.0f64); // (read, write)
    let mut latest_volume: Option<(f32, bool)> = None; // (percent, muted)
    let mut latest_cpu_temp: Option<f32> = None; // °C
    let mut latest_cpu_power: Option<f32> = None; // Watts
    let mut latest_cpu_mhz: Option<u32> = None; // real-time frequency
    // LibreHardwareMonitor: fan speeds etc. The client is restarted if its URL / interval change.
    let mut lhm_link = lhm::Link::new();

    i18n::set_language(config.language);
    set_ui_options(config.ui.clone());
    let mut ui_layouts = config.ui.layouts.clone();
    let mut layouts_started = Instant::now();
    let mut layout_index = usize::MAX;
    let mut layout_marquee = Marquee::new();
    let mut tiles_screen = tiles::TilesScreen::new();
    let mut frame_screen = frame::FrameScreen::new();
    let mut dash_screen = tiles::dash::DashScreen::new();
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
    let mut peak_tracker = spectrum::PeakTracker::new(NUM_BARS);
    let mut last_peak_update = Instant::now();
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
                            || new_cfg.weather_city != config.weather_city
                            || new_cfg.ticker_source != config.ticker_source
                            || new_cfg.ticker_refresh_min != config.ticker_refresh_min;
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
                                if lcd.variant() == trofeo_lcd::Variant::Hid2 {
                                    hs.mount = new_cfg.hid_mount.unwrap_or(hid_default_mount);
                                    hs.header_panel_size = new_cfg.hid_header_panel;
                                }
                                println!("Configuration reloaded.");
                                report_config_error("");
                                if restart_needed {
                                    println!("(deepcool, fps_monitor, openrgb, hide_console, screenshot_key, weather_city, ticker_source, ticker_source and ticker_refresh_min require a restart)");
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
            // Optional: while music plays, stay on the first layout (and restart the
            // rotation from it once the music stops).
            let hold = opts().layout_hold_on_music && last_sound.elapsed() < config.silence_timeout;
            if hold {
                layouts_started = Instant::now();
            }
            let idx = rotation_index(&ui_layouts, layouts_started.elapsed().as_secs(), hold);
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
        // Music screen: a track is playing (sound + player info) and we're not gaming.
        let track = track_shared.lock().ok().and_then(|g| g.clone());
        let music_screen_on = music_screen_active(
            opts().music_screen,
            track.is_some(),
            latest_gpu_percent.is_some_and(|p| p > 50.0),
            last_sound.elapsed() < config.silence_timeout,
        );
        let is_idle = !music_screen_on && (!ui_show.spectrum || last_sound.elapsed() >= config.silence_timeout);
        // The pixel game lives on the default screen only (not on a layout panel, the
        // music screen or the gaming dashboard) and needs the clock to be shown.
        let gaming_now = latest_gpu_percent.is_some_and(|p| p > 50.0);
        let game_wanted = opts().pixel_game
            && ui_show.clock
            && !music_screen_on
            && !(gaming_now && ui_show.dashboard)
            && ui_layouts
                .get(layout_index.min(ui_layouts.len().saturating_sub(1)))
                .map_or(true, |(def, _)| def.standard);
        GAME_STRIP.store(game_wanted, std::sync::atomic::Ordering::Relaxed);
        let game_on = game_wanted && default_areas(&fb).1.is_some();
        let target_fps = if is_idle && game_on {
            // An animation needs a smooth frame rate even when nothing else moves.
            idle_fps_effective.max(config.active_fps)
        } else if is_idle {
            idle_fps_effective
        } else {
            config.active_fps
        };
        // The tiles screen (see `tiles.rs`) with an animated background of its own needs a
        // steady frame rate even while idle.
        let tiles_wanted = is_idle
            && active_def.is_some_and(|d| d.name == "tiles")
            && !(gaming_now && ui_show.dashboard);
        // The digital frame (see `frame.rs`): a few frames a second while nothing moves, a
        // smooth rate while a photo is changing.
        let frame_wanted = is_idle
            && active_def.is_some_and(|d| d.name == "frame")
            && !(gaming_now && ui_show.dashboard);
        // `deck`, `cards`, `orbit` (see `tiles/dash.rs`): the graphs move once a second.
        let dash_style = active_def.and_then(|d| tiles::dash::Style::from_layout(d.name)).filter(|_| is_idle && !(gaming_now && ui_show.dashboard));
        let target_fps = match dash_style {
            Some(_) => target_fps.max(tiles::dash::DashScreen::wanted_fps(&opts().dash, config.ui.effect.fps as f32, config.active_fps)),
            None => target_fps,
        };
        let target_fps = if frame_wanted { target_fps.max(frame_screen.wanted_fps()) } else { target_fps };
        let target_fps = match (tiles_wanted, opts().tiles.bg) {
            (true, tiles::BgMode::Effect) => target_fps.max(config.ui.effect.fps as f32),
            (true, tiles::BgMode::Image) => target_fps.max(config.active_fps),
            // colour presets drifting through the tiles: a few frames a second are plenty
            (true, _) if !opts().tiles.fill_palettes.is_empty() => target_fps.max(5.0),
            _ => target_fps,
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
        let peak_now = Instant::now();
        peak_tracker.update(&bar_heights, (peak_now - last_peak_update).as_secs_f32().min(0.25));
        last_peak_update = peak_now;

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

            // LibreHardwareMonitor (fans, board / disk temperature; fills CPU temp / power
            // that the built-in sensor cannot read).
            let mut lhm_now = lhm_link.update(&opts().lhm);
            if latest_cpu_temp.is_none() {
                latest_cpu_temp = lhm_now.cpu_temp;
            }
            if latest_cpu_power.is_none() {
                latest_cpu_power = lhm_now.cpu_power;
            }

            // Weather: just reads whatever the background thread last fetched
            // (non-blocking); stays `None` until the first fetch succeeds.
            latest_weather = weather_monitor.sample();
            latest_ticker = ticker_monitor.sample();

            // AMD GPU sensor: Edge temperature, ASIC power, fan RPM via ADL PMLog.
            latest_gpu_data = gpu_amd.sample();
            // NVIDIA takes priority: on a PC with an AMD iGPU, ADL would report the
            // iGPU's power (with no temperature) and would hide the real NVIDIA card.
            if let Some(n) = &nvml {
                let (t, w, c) = n.sample();
                if t.is_some() || w.is_some() {
                    latest_gpu_data.temp_hotspot_c = None; // the AMD value would be the iGPU's
                    latest_gpu_data.temp_edge_c = t;
                    latest_gpu_data.power_w = w;
                    latest_gpu_data.clock_mhz = c;
                }
            }
            // The hot spot: the driver's own reading first, LibreHardwareMonitor's otherwise.
            if latest_gpu_data.temp_hotspot_c.is_none() {
                latest_gpu_data.temp_hotspot_c = lhm_now.gpu_hotspot.map(|t| t.round() as i32);
            }
            // The GPU fan: the driver's own reading first, LibreHardwareMonitor's otherwise.
            match latest_gpu_data.fan_rpm {
                Some(r) => lhm_now.gpu_fan = Some(r.max(0) as u32),
                None => latest_gpu_data.fan_rpm = lhm_now.gpu_fan.map(|r| r as i32),
            }
            lhm::set_current(lhm_now);
            deepcool::set_gpu(deepcool::GpuSample {
                temp_c: latest_gpu_data.temp_edge_c,
                power_w: latest_gpu_data.power_w,
                usage_pct: latest_gpu_percent,
                clock_mhz: latest_gpu_data.clock_mhz,
            });
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

        // Data of the panel layouts (also used by the tiles screen, which paints on `fb`).
        let widget_data = (is_idle && active_def.is_some() && !(gaming_mode && ui_show.dashboard)).then(|| layouts::WidgetData {
            cpu_pct: sys.global_cpu_info().cpu_usage(),
            cpu_temp: latest_cpu_temp,
            cpu_power: latest_cpu_power,
            cpu_mhz: latest_cpu_mhz,
            gpu_pct: latest_gpu_percent,
            gpu_temp: latest_gpu_data.temp_edge_c,
            gpu_hotspot: latest_gpu_data.temp_hotspot_c,
            gpu_power: latest_gpu_data.power_w,
            fps: latest_gpu_data.fps,
            used_mb: sys.used_memory() / 1024 / 1024,
            total_mb: sys.total_memory() / 1024 / 1024,
            net_kb: latest_net_kb,
            disk_mb: latest_disk_mb,
            now_playing: now_playing_title.clone(),
            weather: latest_weather.clone(),
            ticker: latest_ticker.clone(),
        });
        let tiles_now = tiles_wanted && widget_data.is_some();
        let frame_now = frame_wanted && widget_data.is_some();
        let dash_now = dash_style.filter(|_| widget_data.is_some());
        // Base: a background (image/gif/video) or a solid color; the interface goes
        // on the `ui` buffer (transparent) when there are margins or a background.
        if composite {
            match (background.as_mut(), config.ui.effect.style) {
                (Some(bg), _) => bg.render_into(&mut fb),
                (None, Some(style)) => bg_effect.render(&mut fb, &config.ui.effect, style, config.ui.effect.brightness),
                (None, None) => {
                    let c = config.background_color.unwrap_or((0x08, 0x08, 0x10));
                    fb.clear(c.0, c.1, c.2)
                }
            }
            ui.clear(UI_KEY.0, UI_KEY.1, UI_KEY.2);
        } else {
            fb.clear(0x08, 0x08, 0x10);
        }
        // The tiles screen blends its tiles into `fb` itself (they are semi-transparent with
        // rounded, anti-aliased corners, which the keyed `ui` buffer cannot express).
        if let (true, Some(wd)) = (tiles_now, widget_data.as_ref()) {
            let area = if composite {
                (config.margins.left, config.margins.top, ui.width(), ui.height())
            } else {
                (0, 0, fb.width(), fb.height())
            };
            let tiles_env = tiles::Env { fps: config.active_fps, ffmpeg: config.ffmpeg.as_deref(), effect: &config.ui.effect, secs: None };
            tiles_screen.draw(&mut fb, area, wd, color_mode, &opts().tiles, &tiles_env);
        }
        if let (Some(style), Some(wd)) = (dash_now, widget_data.as_ref()) {
            let area = if composite {
                (config.margins.left, config.margins.top, ui.width(), ui.height())
            } else {
                (0, 0, fb.width(), fb.height())
            };
            let env = tiles::Env { fps: config.active_fps, ffmpeg: config.ffmpeg.as_deref(), effect: &config.ui.effect, secs: None };
            let o = opts();
            dash_screen.draw(&mut fb, area, style, wd, color_mode, &o.dash, &o.tiles, &env);
        }
        if let (true, Some(wd)) = (frame_now, widget_data.as_ref()) {
            let area = if composite {
                (config.margins.left, config.margins.top, ui.width(), ui.height())
            } else {
                (0, 0, fb.width(), fb.height())
            };
            let env = tiles::Env { fps: config.active_fps, ffmpeg: config.ffmpeg.as_deref(), effect: &config.ui.effect, secs: None };
            frame_screen.draw(&mut fb, area, wd, color_mode, &opts().frame, &opts().tiles, &env);
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
        } else if music_screen_on {
            if let Some(t) = &track {
                music.stats = music_screen::Stats {
                    cpu: Some(sys.global_cpu_info().cpu_usage()),
                    cpu_temp: latest_cpu_temp,
                    gpu: latest_gpu_percent,
                    gpu_temp: latest_gpu_data.temp_edge_c,
                    ram_mb: Some((sys.used_memory() / 1024 / 1024, sys.total_memory() / 1024 / 1024)),
                    net_kb: Some(latest_net_kb),
                    disk_mb: Some(latest_disk_mb),
                    time: Local::now().format("%H:%M:%S").to_string(),
                    uptime: format_uptime(System::uptime()),
                };
                music.draw(target, t, &bar_heights, peak_tracker.peaks(), &opts());
            }
        } else if is_idle {
            if let (Some(def), Some(wd)) = (active_def, widget_data.as_ref()) {
                layouts::draw_layout(target, def, wd, color_mode, &mut layout_marquee);
            } else if ui_show.clock {
                draw_idle_clock(target, color_mode);
            }
        } else {
            draw_bars(target, &bar_heights, peak_tracker.peaks(), color_mode);
        }
        // The pixel game strip under the clock / spectrum.
        if game_on && !(gaming_mode && ui_show.dashboard) {
            if let Some(rect) = default_areas(target).1 {
                let now_t = Local::now();
                let low = bar_heights.len().min(10).max(1);
                let inputs = pixel_game::Inputs {
                    cpu: sys.global_cpu_info().cpu_usage(),
                    net_kb: latest_net_kb.0 + latest_net_kb.1,
                    disk_mb: latest_disk_mb.0 + latest_disk_mb.1,
                    music: last_sound.elapsed() < config.silence_timeout,
                    bass: bar_heights.iter().take(low).sum::<f32>() / low as f32,
                    weather: latest_weather.as_ref().map(|w| w.icon),
                    hour: now_t.hour(),
                    minute: now_t.minute(),
                    second: now_t.second(),
                    unix_secs: now_t.timestamp().max(0) as u64,
                };
                let o = opts();
                game.frame(target, rect, &inputs, o.pixel_game_hero, o.pixel_game_hud);
            }
        }
        // The music screen is a full-panel card of its own: no status lines/ticker on top.
        if !music_screen_on {
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
            if ui_show.ticker {
                draw_ticker_bar(target, &latest_ticker, &mut ticker_marquee);
            }
        }
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

/// Draws ONE frame of the interface with made-up sensor values into a PNG and returns — the
/// settings editor (`trofeo_config`) shows it as its live preview, so what it displays is
/// exactly what the program draws. It follows the same order as the main loop: background,
/// tiles, the screen itself (layout / clock / spectrum / music card), the pixel game, the
/// status lines and the ticker. Nothing is sent to the LCD and no sensor or audio device is opened.
fn run_preview(config: &Config, req: &PreviewReq) -> anyhow::Result<()> {
    let fb = render_preview(config, req)?;
    std::fs::write(&req.path, png_save::encode(&fb))
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", req.path.display()))?;
    Ok(())
}

/// The sample data of the preview (the program is not reading any sensor here).
fn preview_data() -> (layouts::WidgetData, gpu_amd::GpuAmdData, weather::WeatherSnapshot) {
    use weather_icon::WeatherIcon;
    let weather = weather::WeatherSnapshot {
        temp_c: 21.0,
        humidity: Some(58),
        code: 2,
        icon: WeatherIcon::PartlyCloudy,
        city: Some("Milano".to_string()),
        forecast: Vec::new(),
    };
    let gpu = gpu_amd::GpuAmdData {
        temp_edge_c: Some(58),
        power_w: Some(142),
        fan_rpm: Some(1480),
        fps: Some(144),
        ..Default::default()
    };
    let wd = layouts::WidgetData {
        cpu_pct: 37.0,
        cpu_temp: Some(61.0),
        cpu_power: Some(48.0),
        cpu_mhz: Some(4350),
        gpu_pct: Some(46.0),
        gpu_temp: gpu.temp_edge_c,
        gpu_hotspot: gpu.temp_hotspot_c.or(gpu.temp_edge_c.map(|t| t + 12)),
        gpu_power: gpu.power_w,
        fps: gpu.fps,
        used_mb: 11_800,
        total_mb: 32_768,
        net_kb: (850.0, 120.0),
        disk_mb: (14.0, 3.0),
        now_playing: Some("Artist - Song title".to_string()),
        weather: Some(weather.clone()),
        ticker: vec!["Sample headline: this is how the ticker looks".into(), "Second headline of the preview".into()],
    };
    (wd, gpu, weather)
}

fn render_preview(config: &Config, req: &PreviewReq) -> anyhow::Result<Framebuffer> {
    use std::sync::atomic::Ordering::Relaxed;
    i18n::set_language(config.language);
    set_ui_options(config.ui.clone());
    let Display { background, composite, mut fb, mut ui, .. } = build_display(config)?;
    let mut background = background;
    let (wd, gpu_data, weather) = preview_data();
    lhm::set_current(lhm::Readings::sample());
    let o = opts();
    let ui_show = o.show;

    // Which screen: the layout list entry asked for, or the standard one.
    let layouts_list = &config.ui.layouts;
    let idx = req.layout.min(layouts_list.len().saturating_sub(1));
    let active_def = layouts_list.get(idx).map(|(d, _)| *d).filter(|d| !d.standard);
    ACTIVE_PANEL.store(active_def.is_some(), Relaxed);
    ACTIVE_ITEMS.store(layouts_list.get(idx).is_some_and(|(d, _)| d.name == "default2"), Relaxed);
    let music_on = req.state == PreviewState::Music && o.music_screen;
    let is_idle = !music_on && (req.state != PreviewState::Playing || !ui_show.spectrum);
    let game_wanted = o.pixel_game && ui_show.clock && !music_on && layouts_list.get(idx).map_or(true, |(d, _)| d.standard);
    GAME_STRIP.store(game_wanted, Relaxed);
    let game_on = game_wanted && default_areas(&fb).1.is_some();
    let tiles_now = is_idle && active_def.is_some_and(|d| d.name == "tiles");
    let frame_now = is_idle && active_def.is_some_and(|d| d.name == "frame");
    let dash_now = active_def.and_then(|d| tiles::dash::Style::from_layout(d.name)).filter(|_| is_idle);
    let color_mode = config.color_mode;

    let mut sys = System::new_all();
    sys.refresh_cpu();
    std::thread::sleep(Duration::from_millis(220));
    sys.refresh_cpu();
    sys.refresh_memory();

    // Base: background, then the interface on the keyed buffer (as in the main loop).
    if composite {
        match (background.as_mut(), config.ui.effect.style) {
            (Some(bg), _) => bg.render_into(&mut fb),
            (None, Some(style)) => {
                effect::Effect::new().render_at(&mut fb, &config.ui.effect, style, config.ui.effect.brightness, req.secs)
            }
            (None, None) => {
                let c = config.background_color.unwrap_or((0x08, 0x08, 0x10));
                fb.clear(c.0, c.1, c.2)
            }
        }
        ui.clear(UI_KEY.0, UI_KEY.1, UI_KEY.2);
    } else {
        fb.clear(0x08, 0x08, 0x10);
    }
    if tiles_now {
        let area = if composite {
            (config.margins.left, config.margins.top, ui.width(), ui.height())
        } else {
            (0, 0, fb.width(), fb.height())
        };
        let env = tiles::Env { fps: config.active_fps, ffmpeg: config.ffmpeg.as_deref(), effect: &config.ui.effect, secs: Some(req.secs) };
        tiles::TilesScreen::new().draw(&mut fb, area, &wd, color_mode, &o.tiles, &env);
    }
    if let Some(style) = dash_now {
        let area = if composite {
            (config.margins.left, config.margins.top, ui.width(), ui.height())
        } else {
            (0, 0, fb.width(), fb.height())
        };
        let env = tiles::Env { fps: config.active_fps, ffmpeg: config.ffmpeg.as_deref(), effect: &config.ui.effect, secs: Some(req.secs) };
        tiles::dash::DashScreen::new().draw(&mut fb, area, style, &wd, color_mode, &o.dash, &o.tiles, &env);
    }
    if frame_now {
        let area = if composite {
            (config.margins.left, config.margins.top, ui.width(), ui.height())
        } else {
            (0, 0, fb.width(), fb.height())
        };
        let env = tiles::Env { fps: config.active_fps, ffmpeg: config.ffmpeg.as_deref(), effect: &config.ui.effect, secs: Some(req.secs) };
        let mut screen = frame::FrameScreen::with_seed(7);
        screen.still = true;
        screen.draw_settled(&mut fb, area, &wd, color_mode, &o.frame, &o.tiles, &env);
    }
    let target: &mut Framebuffer = if composite { &mut ui } else { &mut fb };

    // Spectrum of the sample music: a few moving humps.
    let bars: Vec<f32> = (0..NUM_BARS)
        .map(|i| {
            let x = i as f32 / NUM_BARS as f32;
            let a = ((x * 9.0 + req.secs as f32 * 1.3).sin() * 0.5 + 0.5) * (1.0 - x * 0.55);
            (0.12 + 0.75 * a).clamp(0.0, 1.0)
        })
        .collect();
    let mut peaks = spectrum::PeakTracker::new(NUM_BARS);
    peaks.update(&bars, 0.1);

    if music_on {
        let mut music = music_screen::MusicScreen::new();
        music.stats = music_screen::Stats {
            cpu: Some(wd.cpu_pct),
            cpu_temp: wd.cpu_temp,
            gpu: wd.gpu_pct,
            gpu_temp: wd.gpu_temp,
            ram_mb: Some((wd.used_mb, wd.total_mb)),
            net_kb: Some(wd.net_kb),
            disk_mb: Some(wd.disk_mb),
            time: Local::now().format("%H:%M:%S").to_string(),
            uptime: format_uptime(System::uptime()),
        };
        let track = media::TrackInfo {
            title: "Song title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            cover: None,
            timeline: Some(media::Timeline {
                position_ms: 83_000,
                duration_ms: 215_000,
                playing: true,
                sampled_at: Instant::now(),
            }),
        };
        music.draw(target, &track, &bars, peaks.peaks(), &o);
    } else if is_idle {
        if let Some(def) = active_def {
            layouts::draw_layout(target, def, &wd, color_mode, &mut Marquee::new());
        } else if ui_show.clock {
            draw_idle_clock(target, color_mode);
        }
    } else {
        draw_bars(target, &bars, peaks.peaks(), color_mode);
    }
    if game_on {
        if let Some(rect) = default_areas(target).1 {
            let now = Local::now();
            let inputs = pixel_game::Inputs {
                cpu: wd.cpu_pct,
                net_kb: 970.0,
                disk_mb: 17.0,
                music: req.state != PreviewState::Idle,
                bass: 0.4,
                weather: Some(weather.icon),
                hour: now.hour(),
                minute: now.minute(),
                second: now.second(),
                unix_secs: now.timestamp().max(0) as u64,
            };
            pixel_game::PixelGame::new().frame(target, rect, &inputs, o.pixel_game_hero, o.pixel_game_hud);
        }
    }
    if !music_on {
        draw_status_lines(
            target,
            &sys,
            wd.gpu_pct,
            &gpu_data,
            wd.net_kb,
            wd.disk_mb,
            Some((65.0, false)),
            wd.cpu_temp,
            wd.cpu_power,
            wd.cpu_mhz,
            wd.now_playing.as_deref(),
            Some(&weather),
            &mut Marquee::new(),
        );
        // (without a `ticker_source` the program has no headlines, so nothing is drawn)
        if ui_show.ticker && config.ticker_source.is_some() {
            draw_ticker_bar(target, &wd.ticker, &mut Marquee::new());
        }
    }
    if composite {
        let panel = (config.ui.panel_opacity < 100).then_some((PANEL_KEY, PANEL_COLOR, config.ui.panel_opacity));
        fb.blit_ui(&ui, config.margins.left, config.margins.top, UI_KEY, panel);
    }
    Ok(fb)
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
    /// Scrolling ticker line position/size/color/panel (independent from the
    /// dedicated "news" panel, which uses the normal layout system).
    ticker_pos: Anchor,
    ticker_size: u32,
    ticker_color: Option<(u8, u8, u8)>,
    ticker_backdrop: Option<bool>,
    /// Maximum width (% of the canvas) of the ticker line.
    ticker_width: u32,
    /// Decimal places shown on CPU/GPU/RAM usage percentages (`37%` vs
    /// `37.4%`). 0 (default) keeps the original whole-number look; 1 adds
    /// one decimal. Deliberately capped at 1 — the LCD panel is small, and
    /// a second decimal digit (`37.42%`) reads as noise rather than useful
    /// precision at this size.
    percent_decimals: u8,
    /// The font of every text of the program: the 5x7 pixel font (default) or anti-aliased Poppins.
    font: trofeo_lcd::TextFont,
    /// Weight of the smooth font (`None` = by text size).
    font_weight: Option<smooth_font::Weight>,
    /// While music is playing, freeze the layout rotation on the FIRST layout
    /// of the list (the one that normally carries the spectrum) instead of
    /// letting the rotation walk through panels the spectrum would cover.
    /// Rotation restarts from the first layout once the music stops.
    layout_hold_on_music: bool,
    /// Spectrum drawing styles (empty = classic filled bars). More than one
    /// entry rotates every `spectrum_style_interval` seconds.
    spectrum_styles: Vec<spectrum::Style>,
    spectrum_style_interval: u32,
    /// Spectrum color presets (empty = the legacy `color` behaviour). More
    /// than one entry rotates (cross-faded) every `spectrum_palette_interval` s.
    spectrum_palettes: Vec<spectrum::Palette>,
    spectrum_palette_interval: u32,
    /// Hue scroll speed of the `rainbow` palette, degrees per second (0 = static).
    spectrum_rainbow_speed: u32,
    /// While a track plays, replace the whole screen with the now-playing card
    /// (cover, artist/title, progress, spectrum) — see `music_screen.rs`.
    music_screen: bool,
    /// Music screen background: mode (gradient / blur / color / solid / none),
    /// brightness, blur amount, fit and the solid color.
    music_bg: music_screen::Bg,
    /// Music screen footer: which system stats to show in small text at the bottom.
    /// `music_stats_set` tells "not configured" (default cpu, gpu, ram) from "none".
    music_stats: Vec<music_screen::StatItem>,
    music_stats_set: bool,
    /// Music screen spectrum color: from the cover, as on the main screen, or its own.
    music_spectrum: music_screen::SpectrumColor,
    /// Pixel-art mini game under the clock on the default screen (see `pixel_game.rs`).
    pixel_game: bool,
    pixel_game_hero: pixel_game::HeroChoice,
    /// Share of the content area the game strip takes, in percent.
    pixel_game_height: u32,
    /// Show the coin counter in a corner of the game.
    pixel_game_hud: bool,
    /// Music screen: show the progress bar and times (when the player reports them).
    music_progress: bool,
    /// Animated background effect (plasma, aurora, lava, waves) in place of an image,
    /// and on the music screen when `music_bg = effect` (see `effect.rs`).
    effect: effect::Settings,
    /// The `tiles` layout: which tiles, font, colors and background (see `tiles.rs`).
    tiles: tiles::Settings,
    /// The `frame` layout: the digital picture frame (see `frame.rs`).
    frame: frame::Settings,
    /// LibreHardwareMonitor (fan speeds and other sensors, see `lhm.rs`).
    lhm: lhm::Settings,
    /// What the DeepCool case display shows (frequency / fan RPM), see `deepcool/mod.rs`.
    deepcool: deepcool::Settings,
    /// The `deck`, `cards` and `orbit` screens (see `tiles/dash.rs`).
    dash: tiles::dash::Settings,
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
            ticker_pos: Anchor { ax: 1, ay: 2 },
            ticker_size: 3,
            ticker_color: None,
            ticker_backdrop: None,
            ticker_width: 100,
            percent_decimals: 0,
            font: trofeo_lcd::TextFont::Pixel,
            font_weight: None,
            layout_hold_on_music: false,
            spectrum_styles: Vec::new(),
            spectrum_style_interval: 30,
            spectrum_palettes: Vec::new(),
            spectrum_palette_interval: 20,
            spectrum_rainbow_speed: 30,
            music_screen: false,
            music_bg: music_screen::Bg::default(),
            music_stats: Vec::new(),
            music_stats_set: false,
            music_spectrum: music_screen::SpectrumColor::Cover,
            pixel_game: true,
            pixel_game_hero: pixel_game::HeroChoice::Both,
            pixel_game_height: 46,
            pixel_game_hud: true,
            music_progress: true,
            effect: effect::Settings::default(),
            tiles: tiles::Settings::default(),
            frame: frame::Settings::default(),
            lhm: lhm::Settings::default(),
            deepcool: deepcool::Settings::default(),
            dash: tiles::dash::Settings::default(),
        }
    }
}

static UI_OPTS: std::sync::RwLock<Option<UiOptions>> = std::sync::RwLock::new(None);

fn set_ui_options(o: UiOptions) {
    trofeo_lcd::set_text_style(o.font, o.font_weight);
    if let Ok(mut g) = UI_OPTS.write() {
        *g = Some(o);
    }
}

#[cfg(test)]
thread_local! {
    static TEST_OPTS: std::cell::RefCell<Option<UiOptions>> = const { std::cell::RefCell::new(None) };
    /// Per-test stand-in for `GAME_STRIP` (tests run in parallel threads).
    static TEST_GAME_STRIP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// true while a panel layout (not `default`) is active: `opts().show` uses `layout_show`.
/// true while the `default2` layout (items with their own style) is active.
static ACTIVE_ITEMS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static ACTIVE_PANEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// true while the pixel-game strip shares the default screen with the clock / spectrum.
static GAME_STRIP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Height in px of the pixel-game strip in a content area `content_h` tall, taking `pct`
/// percent of it (at most 260 px). 0 = not enough room to be worth drawing.
fn game_strip_height(content_h: u32, pct: u32) -> u32 {
    let gh = (content_h * pct.clamp(10, 80) / 100).min(260);
    if gh < 84 || content_h.saturating_sub(gh) < 70 {
        0
    } else {
        gh
    }
}

/// Is the game strip part of the default screen right now?
fn game_strip_on() -> bool {
    #[cfg(test)]
    return TEST_GAME_STRIP.with(|t| t.get());
    #[cfg(not(test))]
    GAME_STRIP.load(std::sync::atomic::Ordering::Relaxed)
}

/// The default screen's content area, split in the part for the clock / spectrum and
/// (while the pixel game is on) the game strip below it, just above the bottom info line.
fn default_areas(fb: &Framebuffer) -> ((u32, u32, u32, u32), Option<(u32, u32, u32, u32)>) {
    let c = content_rect(fb);
    if !game_strip_on() {
        return (c, None);
    }
    let gh = game_strip_height(c.3, opts().pixel_game_height);
    if gh == 0 {
        return (c, None);
    }
    const GAP: u32 = 6;
    const INSET: u32 = 10;
    let main = (c.0, c.1, c.2, c.3 - gh - GAP);
    let game = (c.0 + INSET, c.1 + c.3 - gh, c.2.saturating_sub(2 * INSET), gh);
    (main, Some(game))
}

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

/// Extra space between status lines in the smooth font: the tails of g, p, y and the comma.
fn smooth_extra(scale: u32) -> u32 {
    if trofeo_lcd::text_style().0 == trofeo_lcd::TextFont::Smooth { scale } else { 0 }
}

fn status_line_h() -> u32 {
    STATUS_TEXT_SCALE * 7 + 6 + smooth_extra(STATUS_TEXT_SCALE)
}

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
    scale * 7 + 6 + smooth_extra(scale)
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
    let band = if n > 0 { 8 + n * status_line_h() + 11 } else { 10 };
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

fn draw_bars(fb: &mut Framebuffer, heights: &[f32], peaks: &[f32], color_mode: ColorMode) {
    let o = opts();
    let rect = default_areas(fb).0;
    let sw = rect.2 * o.spectrum_w.clamp(1, 100) / 100;
    let sh = rect.3 * o.spectrum_h.clamp(1, 100) / 100;
    let (area_left, area_top) = o.spectrum_pos.place(rect, (sw, sh), (0, 0));
    let area = (area_left, area_top, sw, sh);

    let time = spectrum::now_secs();
    let style = spectrum::pick_style(&o.spectrum_styles, o.spectrum_style_interval, time);
    if o.spectrum_palettes.is_empty() {
        // No palette configured: the legacy `color` behaviour (default gradient / one color).
        spectrum::draw(fb, style, area, heights, peaks, &|level, _pos| level_color(level, color_mode));
    } else {
        let colors = spectrum::Colors {
            palettes: &o.spectrum_palettes,
            interval: o.spectrum_palette_interval as f32,
            rainbow_speed: o.spectrum_rainbow_speed as f32,
            time,
        };
        spectrum::draw(fb, style, area, heights, peaks, &|level, pos| colors.at(level, pos));
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
        ColorMode::Default => spectrum::classic(level),
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

    let gpu_value = gpu_percent.map_or_else(|| "N/A".to_string(), fmt_pct);
    let gpu_detail = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = gpu_data.temp_edge_c { parts.push(format!("{t}C")); }
        if let Some(w) = gpu_data.power_w { parts.push(format!("{w}W")); }
        if let Some(r) = gpu_data.fan_rpm { parts.push(format!("{r}rpm")); }
        if parts.is_empty() { "N/A".to_string() } else { parts.join(" ") }
    };

    let cpu_pct = sys.global_cpu_info().cpu_usage();
    let cpu_value = fmt_pct(cpu_pct);
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
    let ram_value = fmt_pct(ram_pct);
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
    let rect = default_areas(fb).0;
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
    let mut lines: Vec<(String, u32)> = lines
        .into_iter()
        .map(|(t, max)| {
            let sc = fit_scale(&t, max, max_w);
            (t, sc)
        })
        .collect();

    // With the pixel game below the clock there is less room: shrink the whole block
    // (keeping the time : date proportion) until it fits the area's height.
    let gap = if game_strip_on() { 10u32 } else { 20u32 };
    let total_h = |ls: &[(String, u32)]| -> u32 {
        ls.iter().map(|(_, sc)| Framebuffer::text_height(*sc)).sum::<u32>() + gap * (ls.len() as u32 - 1)
    };
    if total_h(&lines) + 8 > rect.3 {
        let full = lines.clone();
        let t0 = full[0].1.max(1);
        for ts in (1..t0).rev() {
            // The date keeps its proportion to the time (but stays readable: at least 2).
            for (i, (_, sc)) in lines.iter_mut().enumerate() {
                *sc = if i == 0 { ts } else { (full[i].1 * ts / t0).max(2).min(full[i].1) };
            }
            if total_h(&lines) + 8 <= rect.3 {
                break;
            }
        }
    }

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

/// Always-on-top scrolling "news ticker" line (a chyron, like a TV news
/// channel): drawn AFTER everything else, on top of the clock/bars/panel
/// currently on screen, whenever `ticker`/`ticker_source` items are
/// available and `show.ticker` (or `layout_show.ticker` while a panel is
/// active — see `opts()`) is on. Independent from the dedicated "news"
/// preset layout, which instead takes over the whole content area.
fn draw_ticker_bar(fb: &mut Framebuffer, items: &[String], marquee: &mut Marquee) {
    if items.is_empty() {
        return;
    }
    let o = opts();
    let text = items.join("     \u{2022}     ");
    let scale = o.ticker_size;
    let color = o.ticker_color.unwrap_or(o.text_color.unwrap_or((0xE0, 0xE0, 0xE0)));
    let canvas_w = fb.width();
    let max_w = canvas_w.saturating_sub(40).min(canvas_w * o.ticker_width / 100).max(1);
    let line_h = Framebuffer::text_height(scale) + smooth_extra(scale);
    marquee.scale = scale;
    let (x0, y) = o.ticker_pos.place((0, 0, canvas_w, fb.height()), (max_w, line_h), (20, 8));
    draw_backdrop_if(fb, o.ticker_backdrop.unwrap_or(o.text_backdrop), 10, x0, y, max_w, line_h);
    let full_w = Framebuffer::text_width(&text, scale);
    if full_w <= max_w || !marquee.tick(&text, max_w) {
        fb.draw_text(x0, y, &text, color.0, color.1, color.2, scale);
        return;
    }
    // Two copies side by side: when the first one exits, the second picks up
    // right after (same technique as the scrolling "now playing" title).
    let loop_text = format!("{text}{MARQUEE_GAP}");
    let loop_width = Framebuffer::text_width(&loop_text, scale) as i64;
    let x1 = x0 + max_w;
    let base_x = x0 as i64 - marquee.offset_px as i64;
    fb.draw_text_clipped(base_x, y, &loop_text, color.0, color.1, color.2, scale, x0, x1);
    fb.draw_text_clipped(base_x + loop_width, y, &loop_text, color.0, color.1, color.2, scale, x0, x1);
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

    let gpu_str = gpu_percent.map_or_else(|| "N/A".to_string(), fmt_pct);
    let cpu_str = fmt_pct(cpu);
    // GPU sensors (ADL PMLog): only the supported fields. "C" and not "°C": ASCII font.
    let gpu_hw = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = gpu_data.temp_edge_c.filter(|_| sh.gpu_temp) { parts.push(format!("{t}C")); }
        if let Some(t) = gpu_data.temp_hotspot_c.filter(|_| sh.gpu_hotspot) { parts.push(format!("hot {t}C")); }
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
        let lhm = lhm::current();
        let na = parts.is_empty() && (sh.cpu_temp || sh.cpu_power);
        // fan speeds (LibreHardwareMonitor): only the ones that exist
        let mut fans: Vec<String> = Vec::new();
        if let Some(r) = lhm.cpu_fan.filter(|_| sh.cpu_fan) { fans.push(format!("fan {r}rpm")); }
        if let Some(r) = lhm.case_fan.filter(|_| sh.case_fan) { fans.push(format!("case {r}rpm")); }
        if let Some(r) = lhm.pump.filter(|_| sh.pump) { fans.push(format!("pump {r}rpm")); }
        if na && fans.is_empty() {
            "N/A".to_string()
        } else {
            parts.extend(fans);
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
                format!("CPU {cpu_str}{}{}{}", if cpu_freq.is_empty() { "" } else { " " }, cpu_freq,
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
                (sh.cpu, format!("CPU {cpu_str} {cpu_freq} {cpu_hw}")),
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
                rows.push(format!("CPU {cpu_str} {cpu_freq}"));
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
    let block_h = total_lines * status_line_h() - 6;
    let (bx, by) = o.status_pos.place((0, 0, canvas_w, fb.height()), (block_w, block_h), (20, 8));
    draw_backdrop(fb, bx, by, block_w, block_h);

    // ---- drawing
    let mut y = by;
    for row in &rows {
        fb.draw_text(bx, y, row, color.0, color.1, color.2, scale);
        y += status_line_h();
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
        y += status_line_h();
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

    /// A representative 7-day forecast, varied enough (different weekdays,
    /// icons, temperature spreads, and one day with no rain-probability data)
    /// to exercise the "weather" panel's forecast strip in tests/screenshots.
    fn sample_forecast() -> Vec<weather::DailyForecast> {
        use chrono::Weekday::*;
        use weather_icon::WeatherIcon::*;
        [
            (Fri, Rain, 19.0, 12.0, Some(70)),
            (Sat, PartlyCloudy, 22.0, 13.0, Some(20)),
            (Sun, Clear, 25.0, 15.0, Some(0)),
            (Mon, Clear, 26.0, 16.0, None),
            (Tue, Cloudy, 21.0, 14.0, Some(30)),
            (Wed, Thunder, 18.0, 11.0, Some(80)),
            (Thu, PartlyCloudy, 20.0, 12.0, Some(10)),
        ]
        .into_iter()
        .map(|(weekday, icon, hi, lo, pop)| weather::DailyForecast {
            weekday,
            icon,
            temp_max_c: hi,
            temp_min_c: lo,
            precip_prob: pop,
        })
        .collect()
    }

    /// Regression test for the "news panel doesn't scroll" bug: `draw_layout`
    /// must pass the marquee to ANY widget with `scroll: true`, not just
    /// `Music` by name (that hardcoded check silently left `News` static).
    #[test]
    fn news_panel_scrolls() {
        let data = layouts::WidgetData {
            ticker: vec![
                "This is a deliberately long headline meant to be much wider than the panel so it must scroll".to_string(),
            ],
            ..layouts::WidgetData::default()
        };
        let canvas = Orientation::Landscape.canvas();
        let mut fb = Framebuffer::new(canvas);
        let mut m = Marquee::new();
        // First frame: primes the marquee with the text.
        layouts::draw_layout(&mut fb, layouts::find("news").unwrap(), &data, ColorMode::Default, &mut m);
        let offset_after_first = m.offset_px;
        // Let real wall-clock time pass, then draw again: the offset must
        // advance well beyond first-call jitter (at MARQUEE_SPEED_PX_S=45px/s,
        // 150ms is ~6.75px — assert a lenient but unambiguous threshold).
        std::thread::sleep(Duration::from_millis(150));
        layouts::draw_layout(&mut fb, layouts::find("news").unwrap(), &data, ColorMode::Default, &mut m);
        assert!(
            m.offset_px - offset_after_first > 1.0,
            "news panel's marquee did not advance — it would appear static (before: {offset_after_first}, after: {})",
            m.offset_px
        );
    }

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
            draw_bars(&mut fb2, &bars, &bars, ColorMode::Default);
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

    /// The Trofeo Vision 11.3 (1920x400): every screen draws without panics in both
    /// orientations, the panel output has the right size and the game strip still fits.
    #[test]
    fn draws_on_the_11_3_panel() {
        let panel = trofeo_lcd::TROFEO_VISION_11_3;
        let data = layouts::WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_hotspot: None, gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: Some("Artist - A very long song title".into()),
            weather: Some(weather::WeatherSnapshot {
                temp_c: 21.0, humidity: Some(55), code: 61,
                icon: weather_icon::WeatherIcon::Rain, city: Some("Milano".into()), forecast: sample_forecast(),
            }),
            ticker: vec!["First headline".into()],
        };
        for (tag, o) in [("l", Orientation::Landscape), ("p", Orientation::Portrait)] {
            let canvas = o.canvas_for(panel);
            let (cw, ch) = if tag == "l" { (1920, 400) } else { (400, 1920) };
            assert_eq!((canvas.width, canvas.height), (cw, ch));
            let mut fb = Framebuffer::new(canvas);
            sample(&mut fb);
            let out = fb.rotated(o.output_rotation(false));
            assert_eq!((out.width(), out.height()), (1920, 400));
            for def in layouts::LAYOUTS {
                let mut f = Framebuffer::new(canvas);
                f.clear(0x08, 0x08, 0x10);
                let mut m = Marquee::new();
                layouts::draw_layout(&mut f, def, &data, ColorMode::Default, &mut m);
                if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                    std::fs::write(
                        format!("{dir}/p113_layout_{}_{tag}.ppm", def.name),
                        [format!("P6\n{} {}\n255\n", f.width(), f.height()).into_bytes(), f.as_bytes().to_vec()].concat(),
                    ).unwrap();
                }
            }
            // The default screen with the pixel game strip.
            let sys = System::new_all();
            let gpu = gpu_amd::GpuAmdData::default();
            let mut o2 = parse_ui_options(&ConfigFile::parse("").unwrap()).unwrap();
            o2.status_style = StatusStyle::Lines;
            TEST_OPTS.with(|t| *t.borrow_mut() = Some(o2));
            TEST_GAME_STRIP.with(|t| t.set(true));
            let mut g = Framebuffer::new(canvas);
            g.clear(0, 0, 0);
            let (main_rect, game_rect) = default_areas(&g);
            if tag == "l" {
                let gr = game_rect.expect("the game strip should fit on 1920x400");
                assert!(gr.3 >= 84 && main_rect.3 >= 70, "{main_rect:?} {gr:?}");
            }
            draw_idle_clock(&mut g, ColorMode::Default);
            if let Some(gr) = game_rect {
                let mut pg = pixel_game::PixelGame::new();
                pg.frame(&mut g, gr, &pixel_game::Inputs::default(), pixel_game::HeroChoice::Both, true);
            }
            let mut marquee = Marquee::new();
            draw_status_lines(
                &mut g, &sys, Some(37.0), &gpu, (123.0, 45.0), (1.5, 0.2), Some((40.0, false)),
                Some(61.0), Some(88.0), Some(4900), Some("Artist - Song"), None, &mut marquee,
            );
            if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                std::fs::write(
                    format!("{dir}/p113_game_{tag}.ppm"),
                    [format!("P6\n{} {}\n255\n", g.width(), g.height()).into_bytes(), g.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
            TEST_GAME_STRIP.with(|t| t.set(false));
            TEST_OPTS.with(|t| *t.borrow_mut() = None);
        }
    }

    /// All preset layouts, landscape and portrait: no panics; with
    /// TROFEO_DUMP_DIR, saves PNGs.
    #[test]
    fn preset_layouts_render() {
        let data = layouts::WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_hotspot: None, gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: Some("Artist - A very long song title that has to scroll across the panel".into()),
            weather: Some(weather::WeatherSnapshot {
                temp_c: 21.0,
                humidity: Some(55),
                code: 61,
                icon: weather_icon::WeatherIcon::Rain,
                city: Some("Milano".into()),
                forecast: sample_forecast(),
            }),
            ticker: vec!["First headline".into(), "Second headline, a bit longer".into()],
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

    /// Same preset layouts, but composited over a real background image
    /// (`img/sample_background.jpg`) with `panel_opacity = 60`, the way a
    /// user sees them with `background = ...` set: no panics; with
    /// TROFEO_DUMP_DIR, saves PNGs. Skipped if the sample image isn't
    /// present in this checkout (e.g. a source archive without `img/`).
    #[test]
    fn preset_layouts_over_background_render() {
        let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/img/sample_background.jpg"));
        if !path.exists() {
            return;
        }
        let data = layouts::WidgetData {
            cpu_pct: 42.0, cpu_temp: Some(61.0), cpu_power: Some(82.0), cpu_mhz: Some(4700),
            gpu_pct: Some(76.0), gpu_temp: Some(68), gpu_hotspot: None, gpu_power: Some(210), fps: Some(120),
            used_mb: 16500, total_mb: 32768, net_kb: (1800.0, 95.0), disk_mb: (22.4, 3.1),
            now_playing: Some("Daft Punk - Harder, Better, Faster, Stronger".into()),
            weather: Some(weather::WeatherSnapshot {
                temp_c: 19.0,
                humidity: Some(58),
                code: 2,
                icon: weather_icon::WeatherIcon::PartlyCloudy,
                city: Some("Milano".into()),
                forecast: sample_forecast(),
            }),
            ticker: vec![
                "Local team wins championship after dramatic overtime finish".into(),
                "New park opens downtown with bike paths and a dog area".into(),
            ],
        };
        TEST_OPTS.with(|t| *t.borrow_mut() = Some(UiOptions { panel_opacity: 60, show: Show::all(false), ..UiOptions::default() }));
        let canvas = Orientation::Landscape.canvas();
        for def in layouts::LAYOUTS {
            let bg = Background::load(path, canvas, 40, 30.0, None, &BgLayout::default()).unwrap();
            let mut bg_fb = match &bg {
                trofeo_lcd::background::Background::Static(f) => {
                    Framebuffer::from_rgb(f.width(), f.height(), f.as_bytes().to_vec()).unwrap()
                }
                _ => unreachable!("a static image always loads as Background::Static"),
            };
            let mut ui = Framebuffer::new(canvas);
            ui.clear(UI_KEY.0, UI_KEY.1, UI_KEY.2);
            let mut m = Marquee::new();
            layouts::draw_layout(&mut ui, def, &data, ColorMode::Default, &mut m);
            bg_fb.blit_ui(&ui, 0, 0, UI_KEY, Some((PANEL_KEY, PANEL_COLOR, 60)));
            if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                std::fs::write(
                    format!("{dir}/bg_layout_{}.ppm", def.name),
                    [format!("P6\n{} {}\n255\n", bg_fb.width(), bg_fb.height()).into_bytes(), bg_fb.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
        TEST_OPTS.with(|t| *t.borrow_mut() = None);
    }

    /// Semi-transparent panels over a background: compositing with panel_opacity.
    #[test]
    fn panel_opacity_shows_background() {
        let data = layouts::WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_hotspot: None, gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: None,
            weather: None,
            ticker: Vec::new(),
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

    /// `percent_decimals` (CPU/GPU/RAM usage percentages): 0 is the default
    /// (unchanged "37%" look), 1 adds a decimal ("37.4%"), anything else is
    /// rejected rather than silently clamped.
    #[test]
    fn percent_decimals_parses_and_validates() {
        let o = parse_ui_options(&ConfigFile::parse("").unwrap()).unwrap();
        assert_eq!(o.percent_decimals, 0);
        let o = parse_ui_options(&ConfigFile::parse("percent_decimals = 0\n").unwrap()).unwrap();
        assert_eq!(o.percent_decimals, 0);
        let o = parse_ui_options(&ConfigFile::parse("percent_decimals = 1\n").unwrap()).unwrap();
        assert_eq!(o.percent_decimals, 1);
        assert!(parse_ui_options(&ConfigFile::parse("percent_decimals = 2\n").unwrap()).is_err());
        assert!(parse_ui_options(&ConfigFile::parse("percent_decimals = -1\n").unwrap()).is_err());
    }

    /// `fmt_pct` is what every CPU/GPU/RAM usage display goes through: it must
    /// round (not truncate) and respect `percent_decimals` from `opts()`.
    #[test]
    fn fmt_pct_respects_percent_decimals() {
        TEST_OPTS.with(|t| *t.borrow_mut() = Some(UiOptions { percent_decimals: 0, ..UiOptions::default() }));
        assert_eq!(fmt_pct(37.4), "37%");
        assert_eq!(fmt_pct(37.6), "38%");

        TEST_OPTS.with(|t| *t.borrow_mut() = Some(UiOptions { percent_decimals: 1, ..UiOptions::default() }));
        assert_eq!(fmt_pct(37.44), "37.4%");
        assert_eq!(fmt_pct(37.46), "37.5%");

        TEST_OPTS.with(|t| *t.borrow_mut() = None);
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
    /// Checks `rotation_index` against `layout = default:30, weather:5`.
    #[test]
    fn layout_rotation_respects_per_entry_seconds() {
        let o = parse_ui_options(&ConfigFile::parse("layout = default:30, weather:5\n").unwrap()).unwrap();
        let pick = |elapsed_secs: u64| -> &'static str { o.layouts[rotation_index(&o.layouts, elapsed_secs, false)].0.name };
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

    /// `layout_hold_on_music`: while music plays the rotation is pinned to the
    /// first layout, whatever the elapsed time.
    #[test]
    fn rotation_is_pinned_to_the_first_layout_while_holding() {
        let o = parse_ui_options(&ConfigFile::parse("layout = default:30, weather:5, cpu:5\n").unwrap()).unwrap();
        for t in [0u64, 31, 36, 39, 40, 1000] {
            assert_eq!(rotation_index(&o.layouts, t, true), 0, "t={t}");
        }
        // Not holding: the normal rotation, unchanged.
        assert_eq!(rotation_index(&o.layouts, 31, false), 1);
        assert_eq!(rotation_index(&o.layouts, 36, false), 2);
        assert_eq!(rotation_index(&[], 5, false), 0);
    }

    #[test]
    fn music_screen_only_shows_for_a_playing_track_when_enabled_and_not_gaming() {
        assert!(music_screen_active(true, true, false, true));
        assert!(!music_screen_active(false, true, false, true)); // option off
        assert!(!music_screen_active(true, false, false, true)); // player reports nothing
        assert!(!music_screen_active(true, true, true, true)); // gaming dashboard wins
        assert!(!music_screen_active(true, true, false, false)); // silence / paused
    }

    #[test]
    fn music_screen_options_parse() {
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        let d = parse("").unwrap();
        assert!(!d.music_screen && d.music_progress); // off by default
        assert_eq!(d.music_bg, music_screen::Bg::default()); // nice look by default
        let o = parse("music_screen = true\nmusic_progress = no\n").unwrap();
        assert!(o.music_screen && !o.music_progress);
        assert!(parse("music_screen = sometimes\n").is_err());
    }

    #[test]
    fn music_spectrum_color_option_parses() {
        use music_screen::SpectrumColor::*;
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        assert_eq!(parse("").unwrap().music_spectrum, Cover);
        assert_eq!(parse("music_spectrum_color = cover\n").unwrap().music_spectrum, Cover);
        assert_eq!(parse("music_spectrum_color = Main\n").unwrap().music_spectrum, Main);
        // One color = flat; two or more = a gradient, quiet -> loud.
        assert_eq!(
            parse("music_spectrum_color = #0050FF\n").unwrap().music_spectrum,
            Palettes(vec![spectrum::Palette::Custom(vec![(0, 0x50, 255), (0, 0x50, 255)])])
        );
        assert_eq!(
            parse("music_spectrum_color = red, #FFFF00\n").unwrap().music_spectrum,
            Palettes(vec![spectrum::Palette::Custom(vec![(255, 0, 0), (255, 255, 0)])])
        );
        // Preset names (several rotate), independent of the main spectrum_palette.
        let o = parse("spectrum_palette = ocean\nmusic_spectrum_color = fire, neon\n").unwrap();
        assert_eq!(o.music_spectrum, Palettes(vec![spectrum::Palette::Fire, spectrum::Palette::Neon]));
        assert_eq!(o.spectrum_palettes, vec![spectrum::Palette::Ocean]);
        match parse("music_spectrum_color = all\n").unwrap().music_spectrum {
            Palettes(p) => assert!(p.len() >= 7),
            other => panic!("{other:?}"),
        }
        assert!(parse("music_spectrum_color = rainbowish\n").is_err());
        assert!(parse("music_spectrum_color = fire, #FF0000\n").is_err()); // presets and colors do not mix
    }

    /// The game strip sits between the clock and the info lines, on the classic default
    /// screen and on the per-item "default2" one, and nothing is drawn on top of it.
    #[test]
    fn pixel_game_strip_shares_the_default_and_default2_screens() {
        let sys = System::new_all();
        let gpu = gpu_amd::GpuAmdData::default();
        let configs = [
            ("classic", ""),
            ("items_top", "layout = default2\ncpu_size = 4\ncpu_position = top-left\nram_size = 3\nram_position = top-right\ngpu_size = 4\ngpu_position = top\n"),
            ("items_bottom", "layout = default2\ncpu_size = 3\ncpu_position = bottom-left\nram_size = 3\nram_position = bottom-right\nnowplaying_position = bottom\n"),
            ("status_bottom", "status_position = bottom-left\n"),
        ];
        for (name, txt) in configs {
            let mut o = parse_ui_options(&ConfigFile::parse(txt).unwrap()).unwrap();
            assert!(o.pixel_game, "on by default");
            if name.starts_with("items") {
                // What the main loop does while the "default2" layout is on screen.
                o.status_style = StatusStyle::Items;
            }
            TEST_OPTS.with(|t| *t.borrow_mut() = Some(o));
            TEST_GAME_STRIP.with(|t| t.set(true));
            let mut fb = Framebuffer::new(Orientation::Landscape.canvas());
            fb.clear(0, 0, 0);
            let (main_rect, game_rect) = default_areas(&fb);
            let game_rect = game_rect.unwrap_or_else(|| panic!("{name}: no room for the game"));
            let content = content_rect(&fb);
            // The strip is inside the content area (so clear of the info bands), below the clock area.
            assert!(game_rect.1 >= main_rect.1 + main_rect.3, "{name}: strip must be under the clock area");
            assert!(game_rect.1 + game_rect.3 <= content.1 + content.3, "{name}: strip leaks into the info band");
            assert!(game_rect.3 >= 84 && main_rect.3 >= 70, "{name}: {main_rect:?} {game_rect:?}");
            draw_idle_clock(&mut fb, ColorMode::Default);
            // The clock must stay out of the strip.
            let lit = |r: (u32, u32, u32, u32)| {
                (r.1..r.1 + r.3).any(|y| (r.0..r.0 + r.2).any(|x| fb.as_bytes()[((y * fb.width() + x) * 3) as usize..][..3] != [0, 0, 0]))
            };
            assert!(!lit(game_rect), "{name}: the clock spilled into the game strip");
            assert!(lit(main_rect), "{name}: the clock should still be drawn");
            let mut pg = pixel_game::PixelGame::new();
            pg.frame(&mut fb, game_rect, &pixel_game::Inputs::default(), pixel_game::HeroChoice::Both, true);
            let mut marquee = Marquee::new();
            draw_status_lines(
                &mut fb, &sys, Some(37.0), &gpu, (123.0, 45.0), (1.5, 0.2), Some((40.0, false)),
                Some(61.0), Some(88.0), Some(4900), Some("Artist - Song"), None, &mut marquee,
            );
            if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
                std::fs::write(
                    format!("{dir}/game_{name}.ppm"),
                    [format!("P6\n{} {}\n255\n", fb.width(), fb.height()).into_bytes(), fb.as_bytes().to_vec()].concat(),
                ).unwrap();
            }
        }
        // Switched off (or no room): the old full-height clock area.
        TEST_GAME_STRIP.with(|t| t.set(false));
        let fb = Framebuffer::new(Orientation::Landscape.canvas());
        assert_eq!(default_areas(&fb), (content_rect(&fb), None));
        assert_eq!(game_strip_height(120, 46), 0); // too short a content area
        assert!(game_strip_height(352, 46) >= 84 && game_strip_height(1700, 46) <= 260);
        TEST_OPTS.with(|t| *t.borrow_mut() = None);
    }

    #[test]
    fn color_names_and_solid_background() {
        assert_eq!(named_color("white"), Some((255, 255, 255)));
        assert_eq!(named_color("Dark Green"), Some((0, 100, 0)));
        assert_eq!(named_color("sky-blue"), Some((0x87, 0xCE, 0xEB)));
        assert_eq!(named_color("rosa"), Some((0xFF, 0xC0, 0xCB)));
        assert_eq!(named_color("fucsia"), Some((255, 0, 255)));
        assert_eq!(named_color("nonsense"), None);
        let bg = |t: &str| parse_color_opt(&ConfigFile::parse(t).unwrap(), "background_color");
        assert_eq!(bg("").unwrap(), None);
        assert_eq!(bg("background_color = pink\n").unwrap(), Some((255, 192, 203)));
        assert_eq!(bg("background_color = #1B2A4A\n").unwrap(), Some((0x1B, 0x2A, 0x4A)));
        assert_eq!(bg("background_color = 10,20,30\n").unwrap(), Some((10, 20, 30)));
        assert!(bg("background_color = notacolor\n").is_err());
        assert!(is_override_key("background_color"));
        // matrix/rain default to the green palette unless colors are chosen
        let e = parse_effect(&ConfigFile::parse("background_effect = matrix\n").unwrap()).unwrap();
        assert_eq!(e.style, Some(effect::Style::Matrix));
        assert_eq!(e.palettes, vec![effect::Palette::parse("matrix").unwrap()]);
        let e = parse_effect(&ConfigFile::parse("background_effect = rain\neffect_palette = amber\n").unwrap()).unwrap();
        assert_eq!(e.palettes, vec![effect::Palette::parse("amber").unwrap()]);
    }

    #[test]
    fn aquarium_options() {
        let e = |t: &str| parse_effect(&ConfigFile::parse(t).unwrap());
        let a = e("background_effect = aquarium\n").unwrap();
        assert_eq!(a.style, Some(effect::Style::Aquarium));
        assert_eq!(a.palettes, vec![effect::Palette::parse("lagoon").unwrap()]);
        assert_eq!((a.level, a.brightness), (50, 65));
        let b = e("background_effect = acquario\neffect_palette = deep\neffect_level = 70\neffect_brightness = 30\n").unwrap();
        assert_eq!(b.palettes, vec![effect::Palette::parse("deep").unwrap()]);
        assert_eq!((b.level, b.brightness), (70, 30));
        // the other styles keep their own default brightness
        assert_eq!(e("background_effect = plasma\n").unwrap().brightness, 40);
        assert!(e("effect_level = 101\n").is_err());
        // the light (white) variant
        let l = e("background_effect = aquarium\neffect_theme = light\n").unwrap();
        assert!(l.light && l.brightness == 65);
        assert!(!a.light);
        assert!(e("background_effect = matrix\neffect_theme = chiaro\n").unwrap().light);
        assert_eq!(e("background_effect = matrix\neffect_theme = light\n").unwrap().brightness, 65);
        assert_eq!(e("background_effect = matrix\n").unwrap().brightness, 40);
        assert!(!e("effect_theme = dark\n").unwrap().light);
        assert!(e("effect_theme = pink\n").is_err());
        assert!(is_override_key("effect_theme"));
        assert!(is_override_key("effect_level"));
    }

    #[test]
    fn font_options() {
        let o = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        let d = o("").unwrap();
        assert_eq!((d.font, d.font_weight), (trofeo_lcd::TextFont::Pixel, None));
        let s = o("font = smooth\nfont_weight = light\n").unwrap();
        assert_eq!((s.font, s.font_weight), (trofeo_lcd::TextFont::Smooth, Some(smooth_font::Weight::Light)));
        assert_eq!(o("font = Poppins\nfont_weight = auto\n").unwrap().font, trofeo_lcd::TextFont::Smooth);
        assert!(o("font = comic\n").is_err());
        assert!(o("font_weight = bold\n").is_err());
        assert!(is_override_key("font") && is_override_key("font_weight"));
    }

    #[test]
    fn network_and_globe_options() {
        let e = |t: &str| parse_effect(&ConfigFile::parse(t).unwrap());
        for (name, style) in [("network", effect::Style::Network), ("rete", effect::Style::Network), ("globe", effect::Style::Globe), ("sfera", effect::Style::Globe)] {
            let a = e(&format!("background_effect = {name}\n")).unwrap();
            assert_eq!(a.style, Some(style));
            assert_eq!(a.palettes, vec![effect::Palette::parse("ice").unwrap()]);
            assert_eq!((a.brightness, a.level, a.light), (70, 50, false));
            let l = e(&format!("background_effect = {name}\neffect_theme = light\neffect_palette = crimson\neffect_level = 80\n")).unwrap();
            assert!(l.light);
            assert_eq!((l.palettes, l.level), (vec![effect::Palette::parse("crimson").unwrap()], 80));
        }
    }

    /// Whether the program accepts `text` as a configuration (every parser that reads the file).
    fn program_accepts(text: &str) -> Result<(), String> {
        let file = ConfigFile::parse(text)?;
        parse_ui_options(&file).map_err(|e| e.to_string())?;
        file.margins()?.inner(1920, 462)?;
        file.margins()?.inner(462, 1920)?;
        file.panel_resolution()?;
        file.orientation()?;
        parse_bg_layout(&file).map_err(|e| e.to_string())?;
        if let Some(l) = file.get("language") {
            Lang::parse(l).ok_or_else(|| format!("language {l}"))?;
        }
        Ok(())
    }

    #[test]
    fn schema_values_are_accepted_by_the_program() {
        use trofeo_lcd::schema::{fields, Kind};
        let mut tried = 0;
        for f in fields() {
            // keys the program only reads on the command-line path (parse_args) are range-checked there
            let key = f.key.as_str();
            let mut values: Vec<String> = Vec::new();
            match &f.kind {
                Kind::Bool => values.extend(["true".into(), "false".into()]),
                Kind::Choice(o) => values.extend(o.iter().map(|s| s.to_string())),
                Kind::Multi(o) => {
                    values.extend(o.iter().map(|s| s.to_string()));
                    // two at once (not the stand-alone ones)
                    let plain: Vec<&&str> = o.iter().filter(|s| **s != "all" && **s != "none").collect();
                    if plain.len() >= 2 {
                        values.push(format!("{}, {}", plain[0], plain[1]));
                    }
                }
                Kind::Int { min, max } => values.extend([min.to_string(), ((min + max) / 2).to_string(), max.to_string()]),
                Kind::Float { min, max } => values.extend([format!("{min}"), format!("{max}")]),
                Kind::Color => values.extend(["#112233".into(), "red".into(), "10,20,30".into()]),
                Kind::ColorList if matches!(key, "effect_colors" | "spectrum_gradient" | "tiles_colors") => values.push("#112233, #445566".into()),
                Kind::ColorList => values.extend(["#112233".into(), "#112233, #445566".into()]),
                Kind::BarColor => values.extend(["auto".into(), "#112233".into(), "#112233, #445566".into()]),
                Kind::Text | Kind::Path | Kind::Folder | Kind::Layout | Kind::Tiles | Kind::TileColumn => {}
                Kind::Panels { columns } => {
                    if *columns {
                        values.extend(["cpu".into(), "clock | cpu, gpu | music, net, disk".into()]);
                    } else {
                        values.extend(["cpu".into(), "cpu, gpu, ram, fans".into()]);
                    }
                }
            }
            for v in values {
                // the settings some values need next to them
                let extra = match (key, v.as_str()) {
                    ("tiles_bg", "image") => "tiles_bg_image = picture.png\n",
                    ("dash_bg", "image") => "dash_bg_image = picture.png\n",
                    ("effect_palette", "custom") => "effect_colors = #112233, #445566\n",
                    ("tiles_palette", "custom") => "tiles_colors = #112233, #445566\n",
                    ("spectrum_palette", "custom") => "spectrum_gradient = #112233, #445566\n",
                    _ => "",
                };
                // a single color for keys that take a list is fine, but "default" means nothing there
                let text = format!("{extra}{key} = {v}\n");
                if let Err(e) = program_accepts(&text) {
                    panic!("the editor offers `{key} = {v}` but the program refuses it: {e}");
                }
                tried += 1;
            }
        }
        assert!(tried > 400, "only {tried} values were tried");
    }

    #[test]
    fn schema_names_match_the_program() {
        use trofeo_lcd::schema::*;
        // layouts
        let names: Vec<&str> = layouts::LAYOUTS.iter().map(|l| l.name).collect();
        assert_eq!(LAYOUT_NAMES.iter().map(|(n, _, _)| *n).collect::<Vec<_>>(), names);
        // tiles
        assert_eq!(TILE_KINDS.iter().map(|(n, _, _)| *n).collect::<Vec<_>>(), tiles::Kind::ALL.iter().map(|k| k.key()).collect::<Vec<_>>());
        // colors
        for (n, c) in COLOR_NAMES {
            assert_eq!(named_color(n), Some(*c), "{n}");
        }
        // items of the info block
        for (k, _, _) in ITEMS {
            assert!(item_key(&format!("{k}_size")).is_some(), "{k}");
        }
        // effects and their colors
        for e in EFFECTS.iter().filter(|e| **e != "none") {
            assert!(effect::Style::parse(e).is_some(), "{e}");
        }
        for p in EFFECT_PALETTES.iter().filter(|p| !["custom", "all"].contains(p)) {
            assert!(effect::Palette::parse(p).is_some(), "{p}");
        }
        // every key of the editor is one the program reads from the config file
        let from_cli_or_file = |k: &str| is_override_key(k) || ["orientation", "flip", "panel_size", "background", "background_dim", "ffmpeg", "language",
            "hid_mount", "hid_header", "color", "silence_threshold", "silence_timeout_ms", "weather_city", "ticker_source", "ticker_refresh_min", "margin", "margin_top", "margin_bottom", "margin_left", "margin_right"].contains(&k);
        for f in fields() {
            assert!(from_cli_or_file(&f.key), "{} is not a key of the program", f.key);
        }
    }

    #[test]
    fn schema_generated_lists_are_accepted() {
        // the strings the editor's layout and tiles widgets write
        for t in [
            "layout = default",
            "layout = default:30, weather:5",
            "layout = tiles:20, cpu-gpu, news",
            "layout = frame:60, tiles:20",
            "frame_left = clock, ram:ring",
            "frame_right = none",
            "frame_folder = C:\\Users\\Me\\Pictures\\Family trip",
            "tiles = clock:tall, cpu_temp, gpu_temp | ram:ring, cpu:ring, gpu:ring",
            "tiles = clock",
            "tiles = cpu:ring | gpu:ring | ram | net",
        ] {
            program_accepts(&format!("{t}\n")).unwrap_or_else(|e| panic!("{t}: {e}"));
        }
    }

    #[test]
    fn tiles_options_come_from_the_config_and_the_command_line() {
        let o = parse_ui_options(&ConfigFile::parse(
            "layout = tiles:20, weather:5\ntiles = clock, cpu:ring, gpu_temp\ntiles_font = pixel\ntiles_bar_color = #00C8FF, #FF0080\ntile_cpu_color = #FFC800\ntiles_bg = gradient\ntiles_bg_color = #102030, #302010\n",
        ).unwrap()).unwrap();
        assert_eq!(o.layouts.len(), 2);
        assert_eq!(o.layouts[0].0.name, "tiles");
        assert!(!o.layouts[0].0.standard && o.layouts[0].0.rows.is_empty());
        assert_eq!(o.tiles.rows[0].len(), 3);
        assert_eq!(o.tiles.font, tiles::FontStyle::Pixel);
        assert_eq!(o.tiles.bg, tiles::BgMode::Gradient);
        assert_eq!(o.tiles.bar_color(tiles::Kind::Cpu), &tiles::BarColor::Colors(vec![(255, 200, 0)]));
        assert_eq!(o.tiles.bar_color(tiles::Kind::Ram), &tiles::BarColor::Colors(vec![(0, 200, 255), (255, 0, 128)]));
        // not configured: the defaults (smooth font, global background, bars chosen by level)
        let d = parse_ui_options(&ConfigFile::parse("").unwrap()).unwrap().tiles;
        assert_eq!((d.font, d.bg, d.bar.clone()), (tiles::FontStyle::Smooth, tiles::BgMode::Global, tiles::BarColor::Auto));
        // keys that are also command-line flags
        for k in ["tiles", "tiles_font", "tiles_bg", "tiles_bar_color", "tile_cpu_color", "tile_gpu_temp_color"] {
            assert!(is_override_key(k), "{k}");
        }
        // a mistake is reported (the config reload then keeps the previous settings)
        for bad in ["tiles = cpu, bogus", "tiles_font = comic", "tile_ram_color = notacolor", "tiles_bg = image"] {
            assert!(parse_ui_options(&ConfigFile::parse(bad).unwrap()).is_err(), "{bad}");
        }
    }

    #[test]
    fn pixel_game_options_parse() {
        use pixel_game::HeroChoice::*;
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        let d = parse("").unwrap();
        assert!(d.pixel_game && d.pixel_game_hud && d.pixel_game_hero == Both && d.pixel_game_height == 46);
        let o = parse("pixel_game = false\npixel_game_hero = cat\npixel_game_height = 30\npixel_game_hud = no\n").unwrap();
        assert!(!o.pixel_game && !o.pixel_game_hud && o.pixel_game_hero == Cat && o.pixel_game_height == 30);
        assert_eq!(parse("pixel_game_hero = KNIGHT\n").unwrap().pixel_game_hero, Knight);
        assert!(parse("pixel_game_hero = dragon\n").is_err());
        assert!(parse("pixel_game_height = 10\n").is_err());
        assert!(parse("pixel_game_height = 90\n").is_err());
        assert!(parse("pixel_game = maybe\n").is_err());
    }

    #[test]
    fn music_stats_option_parses() {
        use music_screen::StatItem::*;
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        let d = parse("").unwrap();
        assert!(!d.music_stats_set && d.music_stats.is_empty()); // unset = built-in default (cpu, gpu, ram)
        let o = parse("music_stats = net, cpu, net\n").unwrap();
        assert!(o.music_stats_set && o.music_stats == [Net, Cpu]); // order kept, duplicates dropped
        assert_eq!(parse("music_stats = all\n").unwrap().music_stats.len(), 7);
        let n = parse("music_stats = none\n").unwrap();
        assert!(n.music_stats_set && n.music_stats.is_empty()); // explicitly nothing
        assert!(parse("music_stats = cpu, fans\n").is_err());
    }

    #[test]
    fn music_background_options_parse() {
        use music_screen::{Bg, BgFit, BgMode};
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        assert_eq!(Bg::default().mode, BgMode::Gradient);
        let o = parse("music_bg = blur\nmusic_bg_brightness = 55\nmusic_bg_blur = 20\nmusic_bg_fit = stretch\n").unwrap();
        assert_eq!(o.music_bg, Bg { mode: BgMode::Blur, brightness: 55, blur: 20, fit: BgFit::Stretch, ..Bg::default() });
        let o = parse("music_bg = solid\nmusic_bg_color = #102030\n").unwrap();
        assert_eq!((o.music_bg.mode, o.music_bg.color), (BgMode::Solid, (0x10, 0x20, 0x30)));
        assert_eq!(parse("music_bg = none\n").unwrap().music_bg.mode, BgMode::None);
        assert_eq!(parse("music_bg = Color\n").unwrap().music_bg.mode, BgMode::Color);
        // The old switch still works when music_bg is not set, and music_bg wins when both are.
        assert_eq!(parse("music_blur = false\n").unwrap().music_bg.mode, BgMode::None);
        assert_eq!(parse("music_blur = true\n").unwrap().music_bg.mode, BgMode::Gradient);
        assert_eq!(parse("music_blur = false\nmusic_bg = blur\n").unwrap().music_bg.mode, BgMode::Blur);
        // Mistakes are reported, not ignored.
        assert!(parse("music_bg = rainbow\n").is_err());
        assert!(parse("music_bg_brightness = 101\n").is_err());
        assert!(parse("music_bg_blur = -1\n").is_err());
        assert!(parse("music_bg_fit = zoom\n").is_err());
        assert!(parse("music_bg_color = notacolor\n").is_err());
    }

    #[test]
    fn effect_options_parse() {
        use effect::{Palette, Style};
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        // Off by default; the defaults are slow, dim and light on the CPU.
        let d = parse("").unwrap().effect;
        assert_eq!(d.style, None);
        assert_eq!((d.speed, d.scale, d.brightness, d.fps), (30, 100, 40, 8));
        assert_eq!(d.palettes.len(), 1);
        let o = parse("background_effect = lava\neffect_palette = pastel\neffect_speed = 12\neffect_scale = 250\neffect_brightness = 55\neffect_fps = 5\n").unwrap().effect;
        assert_eq!(o.style, Some(Style::Lava));
        assert_eq!(o.palettes, vec![Palette::parse("pastel").unwrap()]);
        assert_eq!((o.speed, o.scale, o.brightness, o.fps), (12, 250, 55, 5));
        assert_eq!(parse("background_effect = SILK\n").unwrap().effect.style, Some(Style::Waves));
        assert_eq!(parse("background_effect = none\n").unwrap().effect.style, None);
        // Several palettes rotate; `all` is every preset.
        let r = parse("effect_palette = fluo, ocean\neffect_palette_interval = 30\n").unwrap().effect;
        assert_eq!((r.palettes.len(), r.palette_interval), (2, 30));
        assert_eq!(parse("effect_palette = all\n").unwrap().effect.palettes, Palette::presets());
        // Own colors: on their own they are the palette; `custom` places them in a list.
        let c = parse("effect_colors = #FF0080, #00FFC8\n").unwrap().effect;
        assert_eq!(c.palettes, vec![Palette::custom(vec![(255, 0, 128), (0, 255, 200)])]);
        let m = parse("effect_colors = #FF0080, #00FFFF\neffect_palette = pastel, custom\n").unwrap().effect;
        assert_eq!(m.palettes.len(), 2);
        // Mistakes are reported, not ignored.
        assert!(parse("background_effect = sparkles\n").is_err());
        assert!(parse("effect_palette = beige\n").is_err());
        assert!(parse("effect_palette = custom\n").is_err());
        assert!(parse("effect_colors = #FF0080\n").is_err());
        assert!(parse("effect_colors = #FF0080, nope\n").is_err());
        assert!(parse("effect_speed = 101\n").is_err());
        assert!(parse("effect_scale = 10\n").is_err());
        assert!(parse("effect_scale = 401\n").is_err());
        assert!(parse("effect_brightness = 101\n").is_err());
        assert!(parse("effect_fps = 0\n").is_err());
        assert!(parse("effect_fps = 31\n").is_err());
        assert!(parse("effect_palette_interval = 4\n").is_err());
        assert!(parse("effect_palette_interval = 3601\n").is_err());
        // They can be set from the command line too.
        for k in ["background_effect", "effect_palette", "effect_colors", "effect_palette_interval", "effect_speed", "effect_scale", "effect_brightness", "effect_fps"] {
            assert!(is_override_key(k), "{k}");
        }
    }

    /// The idle clock, the pixel game and the status lines drawn over the animated
    /// background through the same compositing the main loop uses.
    #[test]
    fn effect_background_sits_under_the_interface() {
        let canvas = Orientation::Landscape.canvas();
        let mut o = UiOptions { show: Show::all(true), ..UiOptions::default() };
        o.effect.style = Some(effect::Style::Plasma);
        TEST_OPTS.with(|t| *t.borrow_mut() = Some(o.clone()));
        TEST_GAME_STRIP.with(|g| g.set(true));
        let mut fb = Framebuffer::new(canvas);
        let mut ui = Framebuffer::new(canvas);
        let mut fx = effect::Effect::new();
        fx.render_at(&mut fb, &o.effect, effect::Style::Plasma, o.effect.brightness, 3.0);
        let backdrop = fb.as_bytes().to_vec();
        ui.clear(UI_KEY.0, UI_KEY.1, UI_KEY.2);
        draw_idle_clock(&mut ui, ColorMode::Default);
        fb.blit_ui(&ui, 0, 0, UI_KEY, None);
        TEST_GAME_STRIP.with(|g| g.set(false));
        TEST_OPTS.with(|t| *t.borrow_mut() = None);
        let changed = fb.as_bytes().chunks_exact(3).zip(backdrop.chunks_exact(3)).filter(|(a, b)| a != b).count();
        let total = (canvas.width * canvas.height) as usize;
        // The clock covers a small part of the screen; the rest is the untouched effect.
        assert!(changed > 500, "the clock was not drawn ({changed})");
        assert!(changed < total / 4, "the clock hid the background ({changed} of {total})");
        // And no pixel is left at the transparent key.
        assert!(fb.as_bytes().chunks_exact(3).all(|p| p != [UI_KEY.0, UI_KEY.1, UI_KEY.2]));
        if let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") {
            let _ = std::fs::write(
                format!("{dir}/effect_clock.ppm"),
                [format!("P6\n{} {}\n255\n", fb.width(), fb.height()).into_bytes(), fb.as_bytes().to_vec()].concat(),
            );
        }
    }

    #[test]
    fn layout_hold_on_music_option_parses() {
        let off = parse_ui_options(&ConfigFile::parse("layout = default, weather\n").unwrap()).unwrap();
        assert!(!off.layout_hold_on_music);
        let on = parse_ui_options(&ConfigFile::parse("layout = default, weather\nlayout_hold_on_music = true\n").unwrap()).unwrap();
        assert!(on.layout_hold_on_music);
        assert!(parse_ui_options(&ConfigFile::parse("layout_hold_on_music = maybe\n").unwrap()).is_err());
    }

    #[test]
    fn spectrum_options_parse() {
        let parse = |t: &str| parse_ui_options(&ConfigFile::parse(t).unwrap());
        // Defaults: classic look, nothing configured.
        let d = parse("").unwrap();
        assert!(d.spectrum_styles.is_empty() && d.spectrum_palettes.is_empty());
        assert_eq!((d.spectrum_style_interval, d.spectrum_palette_interval, d.spectrum_rainbow_speed), (30, 20, 30));
        // Styles: single, list, all/rotate, bars alone = classic.
        assert_eq!(parse("spectrum_style = led\n").unwrap().spectrum_styles, vec![spectrum::Style::Led]);
        assert_eq!(parse("spectrum_style = led, area\n").unwrap().spectrum_styles, vec![spectrum::Style::Led, spectrum::Style::Area]);
        assert_eq!(parse("spectrum_style = all\n").unwrap().spectrum_styles.len(), 5);
        assert_eq!(parse("spectrum_style = rotate\n").unwrap().spectrum_styles.len(), 5);
        assert!(parse("spectrum_style = bars\n").unwrap().spectrum_styles.is_empty());
        assert!(parse("spectrum_style = disco\n").is_err());
        // Palettes.
        assert_eq!(parse("spectrum_palette = fire\n").unwrap().spectrum_palettes, vec![spectrum::Palette::Fire]);
        assert_eq!(parse("spectrum_palette = fire, ocean\n").unwrap().spectrum_palettes.len(), 2);
        assert_eq!(parse("spectrum_palette = all\n").unwrap().spectrum_palettes.len(), 8);
        assert_eq!(parse("spectrum_palette = default\n").unwrap().spectrum_palettes, vec![spectrum::Palette::Default]);
        assert!(parse("spectrum_palette = lava\n").is_err());
        // Custom gradient: alone it selects itself; `custom` needs it.
        let g = parse("spectrum_gradient = #FF0080, #00FFFF\n").unwrap();
        assert_eq!(g.spectrum_palettes, vec![spectrum::Palette::Custom(vec![(255, 0, 128), (0, 255, 255)])]);
        let mix = parse("spectrum_palette = fire, custom\nspectrum_gradient = red, blue\n").unwrap();
        assert_eq!(mix.spectrum_palettes.len(), 2);
        assert!(parse("spectrum_palette = custom\n").is_err());
        assert!(parse("spectrum_gradient = #FF0080\n").is_err()); // needs 2+
        assert!(parse("spectrum_gradient = #FF0080, nonsense\n").is_err());
        // Intervals and rainbow speed are range-checked.
        assert_eq!(parse("spectrum_style_interval = 5\nspectrum_palette_interval = 60\nspectrum_rainbow_speed = 0\n").unwrap().spectrum_style_interval, 5);
        assert!(parse("spectrum_style_interval = 1\n").is_err());
        assert!(parse("spectrum_palette_interval = 9999\n").is_err());
        assert!(parse("spectrum_rainbow_speed = 361\n").is_err());
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
            forecast: sample_forecast(),
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
                draw_bars(&mut fb, &vec![0.7f32; NUM_BARS], &vec![0.7f32; NUM_BARS], ColorMode::Default);
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
        draw_bars(&mut ui, &vec![0.5f32; NUM_BARS], &vec![0.5f32; NUM_BARS], ColorMode::Default);
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
