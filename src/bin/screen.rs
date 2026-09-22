//! Second Monitor program for the Thermalright Trofeo Vision 9.16 LCD.
//!
//! Captures the Windows desktop screen or a virtual monitor (e.g. a Virtual
//! Display Driver) in real time via the DXGI Desktop Duplication API, then
//! sends it to the Trofeo LCD screen via USB bulk transfer (LY protocol).

use std::time::{Duration, Instant};
use anyhow::{bail, Result};
use trofeo_lcd::dxgi_capture::{self, CaptureResult, DxgiSession};
use trofeo_lcd::png_save;
use trofeo_lcd::config::{ConfigFile, Margins};
use trofeo_lcd::{hotkey, Framebuffer, LyLcd, Orientation};

const DEFAULT_ACTIVE_FPS: f32 = 30.0;
const DEFAULT_IDLE_FPS: f32 = 10.0;
const DEFAULT_JPEG_QUALITY: u8 = 75;
/// Minimum interval between two consecutive screenshots (ms) — prevents
/// saving dozens of files while the hotkey is held down.
const SNAP_MIN_INTERVAL: Duration = Duration::from_millis(500);

struct Config {
    display_index: Option<usize>,
    active_fps: f32,
    idle_fps: f32,
    quality: u8,
    /// Orientation of the captured canvas (portrait = display mounted upright).
    orientation: Orientation,
    /// Extra 180° rotation (upside-down screen).
    flip: bool,
    margins: Margins,
    brightness: u8,
    hide_console: bool,
    list_only: bool,
    /// (key virtual-key code, original label from the argument) — `None` = disabled.
    screenshot_key: Option<(u32, String)>,
}

fn print_help() {
    println!(
        "trofeo_screen — Second Monitor Streamer for the Thermalright Trofeo Vision 9.16 LCD\n\n\
        USAGE:\n\
        \x20 trofeo_screen [OPTIONS]\n\n\
        OPTIONS:\n\
        \x20 -l, --list-displays       List all detected monitors then exit\n\
        \x20 -d, --display <INDEX>     Index of the monitor to stream to the LCD\n\
        \x20                           (default: automatically pick the 1920x462 monitor,\n\
        \x20                            or the secondary monitor)\n\
        \x20     --fps <N>             Target FPS when the screen is changing (default: {DEFAULT_ACTIVE_FPS})\n\
        \x20     --idle-fps <N>        Target polling FPS when the screen is idle (default: {DEFAULT_IDLE_FPS})\n\
        \x20 -q, --quality <1-100>     JPEG compression quality (default: {DEFAULT_JPEG_QUALITY})\n\
        \x20     --orientation <O>     'landscape' (default) or 'portrait' (display mounted upright;\n\
        \x20                           virtual monitor must be 462x1920)\n\
        \x20 -r, --rotate, --flip      Rotate an extra 180 degrees (if the screen is upside down)\n\
        \x20     --margin <PX>         Inset the image from the edges (all sides)\n\
        \x20     --margin-top/-bottom/-left/-right <PX>  margin for a single side\n\
        \x20     --brightness <0-100>  Panel brightness (default: 100)\n\
        \x20     --config <FILE>       Config file (default: trofeo.conf in the program/working folder)\n\
        \x20     --hide-console        Hide the console window on Windows (handy for autorun)\n\
        \x20 -k, --screenshot-key <KEY>  Global hotkey to save an LCD frame\n\
        \x20                           screenshot as PNG to the Desktop (f1-f12,\n\
        \x20                           or printscreen). Default: DISABLED.\n\
        \x20 -h, --help                Show this help\n"
    );
}

fn parse_args() -> Result<Config> {
    let mut args = std::env::args().skip(1);
    let mut display_index = None;
    let mut active_fps = DEFAULT_ACTIVE_FPS;
    let mut idle_fps = DEFAULT_IDLE_FPS;
    let mut quality = DEFAULT_JPEG_QUALITY;
    let mut flip_cli: Option<bool> = None;
    let mut orientation_cli: Option<Orientation> = None;
    let mut config_path: Option<std::path::PathBuf> = None;
    let mut brightness_cli: Option<u32> = None;
    let mut margin_all: Option<u32> = None;
    let (mut m_top, mut m_bottom, mut m_left, mut m_right) = (None, None, None, None);
    let mut hide_console = false;
    let mut list_only = false;
    let mut screenshot_key: Option<(u32, String)> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-l" | "--list-displays" => {
                list_only = true;
            }
            "-d" | "--display" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--display needs a numeric index"))?;
                display_index = Some(raw.parse::<usize>()
                    .map_err(|_| anyhow::anyhow!("--display: '{raw}' is not a valid index number"))?);
            }
            "-k" | "--screenshot-key" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--screenshot-key needs a key name"))?;
                screenshot_key = Some((hotkey::parse_key_name(&raw)?, raw.trim().to_ascii_lowercase()));
            }
            "--fps" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--fps needs a numeric value"))?;
                active_fps = raw.parse::<f32>()
                    .map_err(|_| anyhow::anyhow!("--fps: '{raw}' is not a valid number"))?;
            }
            "--idle-fps" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--idle-fps needs a numeric value"))?;
                idle_fps = raw.parse::<f32>()
                    .map_err(|_| anyhow::anyhow!("--idle-fps: '{raw}' is not a valid number"))?;
            }
            "-q" | "--quality" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--quality needs a number 1-100"))?;
                quality = raw.parse::<u8>()
                    .map_err(|_| anyhow::anyhow!("--quality: '{raw}' is not a valid number 1-100"))?
                    .clamp(1, 100);
            }
            "-r" | "--rotate" | "--flip" => {
                flip_cli = Some(true);
            }
            "--orientation" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--orientation requires 'landscape' or 'portrait'"))?;
                orientation_cli = Some(Orientation::parse(&raw).ok_or_else(|| {
                    anyhow::anyhow!("--orientation: '{raw}' is invalid (landscape | portrait)")
                })?);
            }
            "--margin" | "--margin-top" | "--margin-bottom" | "--margin-left" | "--margin-right" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("{arg} requires a number of pixels"))?;
                let v: u32 = raw.parse().map_err(|_| anyhow::anyhow!("{arg}: '{raw}' is not a valid number"))?;
                match arg.as_str() {
                    "--margin" => margin_all = Some(v),
                    "--margin-top" => m_top = Some(v),
                    "--margin-bottom" => m_bottom = Some(v),
                    "--margin-left" => m_left = Some(v),
                    _ => m_right = Some(v),
                }
            }
            "--brightness" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--brightness requires a number 0-100"))?;
                brightness_cli = Some(raw.parse().map_err(|_| anyhow::anyhow!("--brightness: '{raw}' is invalid"))?);
            }
            "--config" => {
                let raw = args.next().ok_or_else(|| anyhow::anyhow!("--config requires a file path"))?;
                config_path = Some(raw.into());
            }
            "--hide-console" => {
                hide_console = true;
            }
            other => {
                bail!("Unrecognized argument: '{other}'. Run with --help for usage.");
            }
        }
    }

    if !(active_fps > 0.0) || !(idle_fps > 0.0) {
        bail!("--fps and --idle-fps must be numbers > 0");
    }

    // Priority: defaults < config file < command line.
    let file = ConfigFile::load(config_path.as_deref()).map_err(|e| anyhow::anyhow!(e))?;
    if let Some(p) = &file.path {
        println!("Config: {}", p.display());
    }
    let orientation = match orientation_cli {
        Some(o) => o,
        None => file.orientation().map_err(|e| anyhow::anyhow!(e))?.unwrap_or_default(),
    };
    let flip = match flip_cli {
        Some(f) => f,
        None => file.get_bool("flip").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(false),
    };

    let mut margins = file.margins().map_err(|e| anyhow::anyhow!(e))?;
    if let Some(v) = margin_all {
        margins = Margins { top: v, bottom: v, left: v, right: v };
    }
    margins.top = m_top.unwrap_or(margins.top);
    margins.bottom = m_bottom.unwrap_or(margins.bottom);
    margins.left = m_left.unwrap_or(margins.left);
    margins.right = m_right.unwrap_or(margins.right);

    let brightness = match brightness_cli {
        Some(v) => v,
        None => file.get_u32("brightness").map_err(|e| anyhow::anyhow!(e))?.unwrap_or(100),
    };
    if brightness > 100 {
        bail!("brightness must be between 0 and 100");
    }

    Ok(Config {
        display_index,
        active_fps,
        idle_fps,
        quality,
        orientation,
        flip,
        margins,
        brightness: brightness as u8,
        hide_console,
        list_only,
        screenshot_key,
    })
}

#[cfg(windows)]
fn hide_console_window() -> Result<()> {
    use windows::Win32::System::Console::FreeConsole;
    unsafe { FreeConsole()? };
    Ok(())
}

fn show_displays() -> Result<()> {
    println!("Checking connected monitors...");
    let displays = dxgi_capture::list_displays()?;
    if displays.is_empty() {
        println!("No monitors detected!");
        return Ok(());
    }

    println!("\nDetected Monitors:");
    println!("{:-<75}", "");
    println!("{:<6} {:<24} {:<16} {:<12} {:<10}", "INDEX", "ADAPTER", "DEVICE", "RESOLUTION", "STATUS");
    println!("{:-<75}", "");

    for d in &displays {
        let res = format!("{}x{}", d.width, d.height);
        let status = if d.is_attached { "Active" } else { "Inactive" };
        let marker = if d.width == 1920 && d.height == 462 { " [MATCH 1920x462]" } else if d.width == 462 && d.height == 1920 { " [MATCH 462x1920 portrait]" } else { "" };
        println!(
            "{:<6} {:<24} {:<16} {:<12} {:<10}{}",
            d.index,
            d.adapter_name.chars().take(22).collect::<String>(),
            d.device_name,
            res,
            status,
            marker
        );
    }
    println!("{:-<75}\n", "");
    Ok(())
}

fn main() -> Result<()> {
    let config = parse_args()?;

    if config.list_only {
        return show_displays();
    }

    #[cfg(windows)]
    if config.hide_console {
        hide_console_window()?;
    }

    println!("============================================================");
    println!("  Trofeo Vision 9.16 — Second Monitor Streamer");
    println!("============================================================");

    // Open the USB connection to the Trofeo LCD screen
    println!("Looking for the Thermalright Trofeo Vision LCD device (0416:5408)...");
    let lcd = match LyLcd::open() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("FAILED: Could not open USB connection to the Trofeo LCD: {e}");
            eprintln!("Make sure the USB cable is plugged in and the WinUSB driver is installed (via Zadig).");
            bail!(e);
        }
    };

    let mut hs = lcd.handshake()?;
    hs.rotation = config.orientation.output_rotation(config.flip);
    hs.brightness = config.brightness;
    println!(
        "LCD Connected: {:?}, PM={} SUB={}, Orientation={:?}, Rotation={}°",
        lcd.variant(), hs.pm, hs.sub, config.orientation, hs.rotation.degrees()
    );

    // Screenshot hotkey (global, default DISABLED — only active when the
    // --screenshot-key argument is given).
    let mut snap_hotkey: Option<hotkey::Hotkey> = None;
    if let Some((vk, label)) = config.screenshot_key {
        match hotkey::register(vk) {
            Ok(h) => {
                println!(
                    "Screenshot hotkey: {} (global) — press to save the LCD frame as PNG to the Desktop",
                    label.to_uppercase()
                );
                snap_hotkey = Some(h);
            }
            Err(e) => eprintln!("WARNING: screenshot hotkey is not active: {e}"),
        }
    }

    // Initialize the DXGI screen capture session
    let mut session = match DxgiSession::new(config.display_index) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\nFAILED to initialize monitor capture: {e}");
            eprintln!("Run 'trofeo_screen --list-displays' to check the available monitors.");
            bail!(e);
        }
    };

    let (src_w, src_h) = session.src_resolution();
    let display_idx = session.display_index();
    println!(
        "Capturing Display [{display_idx}] (Resolution: {src_w}x{src_h}) -> Streaming to LCD (canvas {}x{}, 1920x462 panel)",
        config.orientation.canvas().width, config.orientation.canvas().height
    );
    println!(
        "Settings: Active FPS: {:.1} | Idle FPS: {:.1} | Quality: {}%",
        config.active_fps, config.idle_fps, config.quality
    );
    println!("Streaming started... Press Ctrl+C to stop.\n");

    let resolution = config.orientation.canvas();
    let mut fb = Framebuffer::new(resolution);
    // With margins, the capture goes into a smaller buffer, then gets pasted in the center.
    let (in_w, in_h) = config
        .margins
        .inner(resolution.width, resolution.height)
        .map_err(|e| anyhow::anyhow!(e))?;
    let use_margins = !config.margins.is_zero();
    let mut inner = Framebuffer::new(trofeo_lcd::Resolution::new(
        if use_margins { in_w } else { 1 },
        if use_margins { in_h } else { 1 },
    ));

    let active_frame_time = Duration::from_secs_f32(1.0 / config.active_fps);
    let idle_frame_time = Duration::from_secs_f32(1.0 / config.idle_fps);
    let mut last_snap = Instant::now() - SNAP_MIN_INTERVAL;

    loop {
        let frame_start = Instant::now();

        // Screenshot hotkey (global — keeps working even when the window
        // isn't focused). The saved frame is the last one that was displayed.
        if let Some(h) = &snap_hotkey {
            if hotkey::triggered(h.id) && last_snap.elapsed() >= SNAP_MIN_INTERVAL {
                match png_save::save(&fb, "trofeo_screen") {
                    Ok(p) => println!("Screenshot saved: {}", p.display()),
                    Err(e) => eprintln!("Failed to save screenshot: {e}"),
                }
                last_snap = Instant::now();
            }
        }

        // Capture the next frame from DXGI (100ms timeout)
        let capture_result = session.acquire_next_frame(100, if use_margins { &mut inner } else { &mut fb });

        match capture_result {
            Ok(CaptureResult::NewFrame) => {
                if use_margins {
                    fb.blit(&inner, config.margins.left, config.margins.top);
                }
                // Screen changed: send the new frame to the LCD
                if let Err(e) = lcd.send_framebuffer(&hs, &fb, config.quality) {
                    eprintln!("USB warning: Failed to send frame to LCD ({e}), retrying...");
                    std::thread::sleep(Duration::from_millis(200));
                }

                // Maintain the target active FPS
                let elapsed = frame_start.elapsed();
                if elapsed < active_frame_time {
                    std::thread::sleep(active_frame_time - elapsed);
                }
            }
            Ok(CaptureResult::Timeout) => {
                // Screen static/idle: nothing to send (saves USB bandwidth & CPU!)
                let elapsed = frame_start.elapsed();
                if elapsed < idle_frame_time {
                    std::thread::sleep(idle_frame_time - elapsed);
                }
            }
            Ok(CaptureResult::NeedsReinit) => {
                eprintln!("Monitor mode changed or access was lost. Reinitializing capture...");
                std::thread::sleep(Duration::from_millis(500));
                if let Ok(new_sess) = DxgiSession::new(Some(display_idx)) {
                    session = new_sess;
                    println!("Capture successfully reinitialized.");
                }
            }
            Err(e) => {
                eprintln!("Capture warning: {e}");
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}
