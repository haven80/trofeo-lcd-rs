//! Preset layouts: large panels (CPU, GPU, temperatures, RAM, network,
//! disk, track, clock, FPS) in place of the large clock. Chosen with
//! `layout = name` in trofeo.conf (or `--layout name`).
//!
//! Each layout is a grid: rows of (widget, width in columns). In
//! portrait mode, widgets are stacked vertically (1 column, 2 if there are > 6).

use super::*;
use crate::weather_icon::{self, WeatherIcon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Widget {
    Cpu,
    CpuTemp,
    Gpu,
    GpuTemp,
    Ram,
    Net,
    Disk,
    Fps,
    Clock,
    Music,
    Weather,
    News,
}

use Widget::*;

#[derive(Debug)]
pub struct LayoutDef {
    pub name: &'static str,
    pub description: &'static str,
    /// Standard screen (bars/clock/dashboard + status lines), not a panel grid.
    pub standard: bool,
    pub cols: u32,
    /// Rows of (widget, column span).
    pub rows: &'static [&'static [(Widget, u32)]],
}

pub const LAYOUTS: &[LayoutDef] = &[
    LayoutDef {
        name: "default",
        description: "Standard screen: status lines on top, spectrum with music, large clock when idle",
        standard: true,
        cols: 1,
        rows: &[],
    },
    LayoutDef {
        name: "default2",
        description: "Like default, but each item has its own size/position/color (cpu_size, ram_position, ...)",
        standard: true,
        cols: 1,
        rows: &[],
    },
    LayoutDef {
        name: "cpu-gpu",
        description: "CPU and GPU side by side: large usage, temperature and watts below",
        standard: false,
        cols: 2,
        rows: &[&[(Cpu, 1), (Gpu, 1)]],
    },
    LayoutDef {
        name: "temps",
        description: "CPU and GPU temperatures, large, with a bar",
        standard: false,
        cols: 2,
        rows: &[&[(CpuTemp, 1), (GpuTemp, 1)]],
    },
    LayoutDef {
        name: "overview",
        description: "CPU, GPU, RAM and clock",
        standard: false,
        cols: 4,
        rows: &[&[(Cpu, 1), (Gpu, 1), (Ram, 1), (Clock, 1)]],
    },
    LayoutDef {
        name: "grid6",
        description: "3x2 grid: CPU/GPU usage and temperature, RAM, clock",
        standard: false,
        cols: 3,
        rows: &[&[(Cpu, 1), (Gpu, 1), (Ram, 1)], &[(CpuTemp, 1), (GpuTemp, 1), (Clock, 1)]],
    },
    LayoutDef {
        name: "clock-center",
        description: "Clock in the center, CPU on the left and GPU on the right",
        standard: false,
        cols: 4,
        rows: &[&[(Cpu, 1), (Clock, 2), (Gpu, 1)]],
    },
    LayoutDef {
        name: "io",
        description: "Network, disk and RAM",
        standard: false,
        cols: 3,
        rows: &[&[(Net, 1), (Disk, 1), (Ram, 1)]],
    },
    LayoutDef {
        name: "music",
        description: "Currently playing track, large, + clock",
        standard: false,
        cols: 3,
        rows: &[&[(Music, 2), (Clock, 1)]],
    },
    LayoutDef {
        name: "gaming",
        description: "FPS, GPU, GPU temperature and CPU",
        standard: false,
        cols: 4,
        rows: &[&[(Fps, 1), (Gpu, 1), (GpuTemp, 1), (Cpu, 1)]],
    },
    LayoutDef {
        name: "cpu",
        description: "CPU only, large: usage, temperature, frequency, watts",
        standard: false,
        cols: 1,
        rows: &[&[(Cpu, 1)]],
    },
    LayoutDef {
        name: "gpu",
        description: "GPU only, large: usage, temperature, watts",
        standard: false,
        cols: 1,
        rows: &[&[(Gpu, 1)]],
    },
    LayoutDef {
        name: "weather",
        description: "Current weather, large: temperature, condition icon and city",
        standard: false,
        cols: 1,
        rows: &[&[(Weather, 1)]],
    },
    LayoutDef {
        name: "news",
        description: "Latest headlines from ticker_source, scrolling",
        standard: false,
        cols: 1,
        rows: &[&[(News, 1)]],
    },
];

pub fn find(name: &str) -> Option<&'static LayoutDef> {
    let n = name.trim().to_ascii_lowercase();
    LAYOUTS.iter().find(|l| l.name == n)
}

pub fn names() -> String {
    LAYOUTS.iter().map(|l| l.name).collect::<Vec<_>>().join(", ")
}

/// Point-in-time data for the widgets.
#[derive(Default, Clone)]
pub struct WidgetData {
    pub cpu_pct: f32,
    pub cpu_temp: Option<f32>,
    pub cpu_power: Option<f32>,
    pub cpu_mhz: Option<u32>,
    pub gpu_pct: Option<f32>,
    pub gpu_temp: Option<i32>,
    pub gpu_power: Option<i32>,
    pub fps: Option<i32>,
    pub used_mb: u64,
    pub total_mb: u64,
    pub net_kb: (f64, f64),
    pub disk_mb: (f64, f64),
    pub now_playing: Option<String>,
    pub weather: Option<crate::weather::WeatherSnapshot>,
    /// Latest headlines/items from `ticker_source` (see `ticker.rs`), in the
    /// order they were fetched.
    pub ticker: Vec<String>,
}

struct View {
    label: String,
    value: String,
    detail: String,
    /// Gauge bar 0..1 under the value.
    gauge: Option<f32>,
    /// Value color (if None: text color).
    value_color: Option<(u8, u8, u8)>,
    /// The value is a long title that should scroll (track name).
    scroll: bool,
    /// A small condition icon drawn above the value (weather panels).
    icon: Option<(WeatherIcon, (u8, u8, u8))>,
    /// Up to 7 days, precomputed for display (weather panel only; empty for
    /// every other widget).
    forecast: Vec<ForecastDayView>,
}

/// One precomputed forecast-strip column (weather panel only). Formatting
/// (unit conversion, localized day abbreviation) happens once here in
/// `build_view`, so `draw_panel` just draws strings/an icon — same split as
/// the rest of `View`.
struct ForecastDayView {
    day_label: &'static str,
    icon: WeatherIcon,
    icon_color: (u8, u8, u8),
    /// "24/15" (already unit-converted, no degree symbol — kept compact so
    /// it fits a narrow column at small scale).
    temps: String,
    /// "70%", or empty when the model didn't provide a probability for that day.
    pop: String,
}

fn fmt_rate(kb: f64) -> String {
    if kb >= 1024.0 {
        format!("{:.1}MB/S", kb / 1024.0)
    } else {
        format!("{kb:.0}KB/S")
    }
}

fn na() -> String {
    "N/A".to_string()
}

fn temp_view(label: &str, t: Option<f32>, detail: String, color_mode: ColorMode) -> View {
    match t {
        Some(t) => View {
            label: label.into(),
            value: format!("{t:.0}C"),
            detail,
            gauge: Some((t / 100.0).clamp(0.0, 1.0)),
            value_color: Some(level_color((t / 100.0).clamp(0.0, 1.0).max(0.2), color_mode)),
            scroll: false,
            icon: None,
            forecast: Vec::new(),
        },
        None => View { label: label.into(), value: "--".into(), detail: na(), gauge: None, value_color: None, scroll: false, icon: None, forecast: Vec::new() },
    }
}

fn build_view(w: Widget, d: &WidgetData, color_mode: ColorMode) -> View {
    let tr = i18n::t();
    let now = Local::now();
    match w {
        Cpu => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(m) = d.cpu_mhz { parts.push(format_freq_mhz(m)); }
            if let Some(t) = d.cpu_temp { parts.push(format!("{t:.0}C")); }
            if let Some(p) = d.cpu_power { parts.push(format!("{p:.0}W")); }
            View {
                label: "CPU".into(),
                value: format!("{:.0}%", d.cpu_pct),
                detail: if parts.is_empty() { na() } else { parts.join(" ") },
                gauge: Some((d.cpu_pct / 100.0).clamp(0.0, 1.0)),
                value_color: None,
                scroll: false,
                icon: None,
                forecast: Vec::new(),
            }
        }
        CpuTemp => {
            let mut detail = format!("{:.0}%", d.cpu_pct);
            if let Some(m) = d.cpu_mhz { detail = format!("{detail} {}", format_freq_mhz(m)); }
            if let Some(p) = d.cpu_power { detail = format!("{detail} {p:.0}W"); }
            temp_view("CPU TEMP", d.cpu_temp, detail, color_mode)
        }
        Gpu => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(t) = d.gpu_temp { parts.push(format!("{t}C")); }
            if let Some(p) = d.gpu_power { parts.push(format!("{p}W")); }
            View {
                label: "GPU".into(),
                value: d.gpu_pct.map_or_else(|| "--".into(), |p| format!("{p:.0}%")),
                detail: if parts.is_empty() { na() } else { parts.join(" ") },
                gauge: d.gpu_pct.map(|p| (p / 100.0).clamp(0.0, 1.0)),
                value_color: None,
                scroll: false,
                icon: None,
                forecast: Vec::new(),
            }
        }
        GpuTemp => {
            let mut detail = d.gpu_pct.map_or_else(String::new, |p| format!("{p:.0}%"));
            if let Some(p) = d.gpu_power { detail = format!("{detail} {p}W").trim().to_string(); }
            if detail.is_empty() { detail = na(); }
            temp_view("GPU TEMP", d.gpu_temp.map(|t| t as f32), detail, color_mode)
        }
        Ram => {
            let pct = if d.total_mb > 0 { d.used_mb as f32 * 100.0 / d.total_mb as f32 } else { 0.0 };
            View {
                label: tr.mem.to_string(),
                value: format!("{pct:.0}%"),
                detail: format!("{}/{}MB", d.used_mb, d.total_mb),
                gauge: Some((pct / 100.0).clamp(0.0, 1.0)),
                value_color: None,
                scroll: false,
                icon: None,
                forecast: Vec::new(),
            }
        }
        Net => View {
            label: format!("{} {}", tr.net, tr.net_down),
            value: fmt_rate(d.net_kb.0),
            detail: format!("{} {}", tr.net_up, fmt_rate(d.net_kb.1)),
            gauge: None,
            value_color: None,
            scroll: false,
            icon: None,
            forecast: Vec::new(),
        },
        Disk => View {
            label: format!("{} {}", tr.disk, tr.disk_read),
            value: format!("{:.1}MB/S", d.disk_mb.0),
            detail: format!("{} {:.1}MB/S", tr.disk_write, d.disk_mb.1),
            gauge: None,
            value_color: None,
            scroll: false,
            icon: None,
            forecast: Vec::new(),
        },
        Fps => View {
            label: "FPS".into(),
            value: d.fps.map_or_else(|| "--".into(), |f| f.to_string()),
            detail: match d.fps {
                Some(f) if f > 0 => format!("{:.1}MS", 1000.0 / f as f32),
                _ => "-".into(),
            },
            gauge: None,
            value_color: None,
            scroll: false,
            icon: None,
            forecast: Vec::new(),
        },
        Clock => View {
            label: i18n::weekday(&now),
            value: now.format("%H:%M:%S").to_string(),
            detail: i18n::day_month_year(&now),
            gauge: None,
            value_color: opts().clock_color.or_else(|| Some(accent_color(color_mode))),
            scroll: false,
            icon: None,
            forecast: Vec::new(),
        },
        Music => View {
            label: tr.now_playing.to_string(),
            value: d.now_playing.clone().unwrap_or_else(|| "-".into()),
            detail: String::new(),
            gauge: None,
            value_color: None,
            scroll: true,
            icon: None,
            forecast: Vec::new(),
        },
        News => View {
            label: tr.news.to_string(),
            value: if d.ticker.is_empty() { "-".into() } else { d.ticker.join("     \u{2022}     ") },
            detail: String::new(),
            gauge: None,
            value_color: None,
            scroll: true,
            icon: None,
            forecast: Vec::new(),
        },
        Weather => match &d.weather {
            Some(w) => {
                let fahrenheit = opts().weather_fahrenheit;
                let unit = if fahrenheit { "F" } else { "C" };
                let color = weather_icon::default_color(w.icon);
                let forecast = w
                    .forecast
                    .iter()
                    .map(|day| {
                        let day_color = weather_icon::default_color(day.icon);
                        ForecastDayView {
                            day_label: i18n::weekday_short(day.weekday),
                            icon: day.icon,
                            icon_color: day_color,
                            temps: format!("{:.0}/{:.0}", day.temp_max_in(fahrenheit), day.temp_min_in(fahrenheit)),
                            pop: day.precip_prob.map(|p| format!("{p}%")).unwrap_or_default(),
                        }
                    })
                    .collect();
                View {
                    label: w.city.clone().unwrap_or_else(|| tr.weather.to_string()),
                    value: format!("{:.0}{unit}", w.temp_in(fahrenheit)),
                    detail: format!(
                        "{}{}",
                        weather_icon::condition_name(w.icon),
                        w.humidity.map_or_else(String::new, |h| format!(" {h}%"))
                    ),
                    gauge: None,
                    value_color: Some(color),
                    scroll: false,
                    icon: Some((w.icon, color)),
                    forecast,
                }
            }
            None => View {
                label: tr.weather.to_string(),
                value: "--".into(),
                detail: na(),
                gauge: None,
                value_color: None,
                scroll: false,
                icon: None,
                forecast: Vec::new(),
            },
        },
    }
}

fn draw_panel(
    fb: &mut Framebuffer,
    rect: (u32, u32, u32, u32),
    v: &View,
    color_mode: ColorMode,
    marquee: Option<&mut Marquee>,
) {
    let (x, y, w, h) = rect;
    if w < 40 || h < 40 {
        return;
    }
    let border = accent_color(color_mode);
    let bt = 2u32;
    fb.fill_rect(x, y, w, h, border.0, border.1, border.2);
    let fill = panel_fill();
    fb.fill_rect(x + bt, y + bt, w - bt * 2, h - bt * 2, fill.0, fill.1, fill.2);

    let text = opts().text_color;
    let label_color = text.map_or((0xA0, 0xA0, 0xA8), |c| dim_color(c, 70));
    let value_color = v.value_color.or(text).unwrap_or((0xF0, 0xF0, 0xF0));
    let pad = if h < 200 { 8u32 } else { 14u32 };
    let inner_w = w.saturating_sub(bt * 2 + pad * 2);

    let label_scale = if h >= 240 { 4 } else if h >= 170 { 3 } else { 2 };
    let detail_scale_max = if h >= 240 { 5 } else if h >= 170 { 4 } else { 3 };
    let label_scale = fit_scale(&v.label, label_scale, inner_w);
    let label_h = Framebuffer::text_height(label_scale);
    let label_w = Framebuffer::text_width(&v.label, label_scale);
    let label_y = y + bt + pad;
    fb.draw_text(x + w.saturating_sub(label_w) / 2, label_y, &v.label, label_color.0, label_color.1, label_color.2, label_scale);

    // Detail at the bottom.
    let detail_scale = fit_scale(&v.detail, detail_scale_max, inner_w);
    let detail_h = if v.detail.is_empty() { 0 } else { Framebuffer::text_height(detail_scale) };
    let detail_y = y + h - bt - pad - detail_h;
    if detail_h > 0 {
        let dw = Framebuffer::text_width(&v.detail, detail_scale);
        fb.draw_text(x + w.saturating_sub(dw) / 2, detail_y, &v.detail, label_color.0, label_color.1, label_color.2, detail_scale);
    }

    // Gauge bar.
    let mut bottom_limit = if detail_h > 0 { detail_y.saturating_sub(10) } else { y + h - bt - pad };
    if let Some(frac) = v.gauge {
        let bar_h = 14u32;
        let bar_y = bottom_limit.saturating_sub(bar_h);
        let bar_x = x + bt + pad;
        fb.fill_rect(bar_x, bar_y, inner_w, bar_h, 0x30, 0x30, 0x3C);
        let fill = (inner_w as f32 * frac.clamp(0.0, 1.0)) as u32;
        let c = level_color(frac, color_mode);
        fb.fill_rect(bar_x, bar_y, fill, bar_h, c.0, c.1, c.2);
        bottom_limit = bar_y.saturating_sub(10);
    }

    // Value centered in the remaining space.
    let top_limit = label_y + label_h + 6;
    let mut mid_h = bottom_limit.saturating_sub(top_limit);

    // 7-day forecast strip (weather panel only): reserve a band at the
    // bottom of the middle area for it, shrinking what's left above for the
    // current conditions (icon + big temperature). `v.forecast` is always
    // empty for every other widget, so this never affects them.
    let forecast_rect = if v.forecast.is_empty() {
        None
    } else {
        let forecast_h = ((mid_h as f32 * 0.4) as u32).clamp(70, 170);
        if forecast_h + 60 <= mid_h {
            let fy = bottom_limit.saturating_sub(forecast_h);
            mid_h = mid_h.saturating_sub(forecast_h + 10);
            Some((x + bt + pad, fy, inner_w, forecast_h))
        } else {
            None
        }
    };

    if v.scroll {
        let scale = fit_scale("W", (mid_h / 7).clamp(1, 8), inner_w).max(2);
        let vh = Framebuffer::text_height(scale);
        let vy = top_limit + mid_h.saturating_sub(vh) / 2;
        let x0 = x + bt + pad;
        let x1 = x0 + inner_w;
        let natural = Framebuffer::text_width(&v.value, scale);
        let scrolling = marquee.as_ref().map_or(false, |_| natural > inner_w);
        if let (true, Some(m)) = (scrolling, marquee) {
            // Marquee measures at STATUS_TEXT_SCALE: we only use it for the advance.
            m.tick(&v.value, inner_w * STATUS_TEXT_SCALE / scale);
            let loop_text = format!("{}{}", v.value, MARQUEE_GAP);
            let loop_w = Framebuffer::text_width(&loop_text, scale) as i64;
            let off = (m.offset_px * scale as f32 / STATUS_TEXT_SCALE as f32) as i64 % loop_w.max(1);
            let base = x0 as i64 - off;
            fb.draw_text_clipped(base, vy, &loop_text, value_color.0, value_color.1, value_color.2, scale, x0, x1);
            fb.draw_text_clipped(base + loop_w, vy, &loop_text, value_color.0, value_color.1, value_color.2, scale, x0, x1);
        } else {
            let vx = x0 + inner_w.saturating_sub(natural) / 2;
            fb.draw_text(vx, vy, &v.value, value_color.0, value_color.1, value_color.2, scale);
        }
    } else {
        // A condition icon (weather panels) is drawn above the value, in its own
        // slice of the middle area; the value then centers in what's left.
        let mut value_top = top_limit;
        let mut value_h_avail = mid_h;
        if let Some((icon, (ir, ig, ib))) = v.icon {
            let icon_scale = ((mid_h / 3) / weather_icon::ICON_SIZE).clamp(1, 12);
            let icon_px = weather_icon::size(icon_scale);
            if icon_px + 20 < mid_h {
                let icon_x = x + w.saturating_sub(icon_px) / 2;
                weather_icon::draw(fb, icon_x, top_limit, icon_scale, icon, ir, ig, ib);
                value_top = top_limit + icon_px + 8;
                value_h_avail = mid_h.saturating_sub(icon_px + 8);
            }
        }
        let scale = fit_scale(&v.value, (value_h_avail / 7).max(1), inner_w);
        let vh = Framebuffer::text_height(scale);
        let vw = Framebuffer::text_width(&v.value, scale);
        let vy = value_top + value_h_avail.saturating_sub(vh) / 2;
        fb.draw_text(x + w.saturating_sub(vw) / 2, vy, &v.value, value_color.0, value_color.1, value_color.2, scale);
    }

    if let Some(rect) = forecast_rect {
        draw_forecast_strip(fb, rect, &v.forecast);
    }
}

/// Picks one (day_scale, temps_scale, pop_scale) triple shared by every
/// column of the forecast strip, each the largest that still fits
/// `col_inner` for the LONGEST string in that role across all 7 days (e.g. a
/// "-15/-22" temps string is longer than "24/15" and must not be allowed to
/// overlap into the next column) — a single narrow column (typical in
/// portrait mode, where the panel is only ~400px wide for 7 columns instead
/// of ~1850px in landscape) shrinks the text for every day, rather than only
/// the one day whose text happens to be long.
fn forecast_column_text_scales(days: &[ForecastDayView], col_inner: u32) -> (u32, u32, u32) {
    let day_scale = days.iter().map(|d| fit_scale(d.day_label, 2, col_inner)).min().unwrap_or(1);
    let temps_scale = days.iter().map(|d| fit_scale(&d.temps, 2, col_inner)).min().unwrap_or(1);
    let pop_scale = days
        .iter()
        .filter(|d| !d.pop.is_empty())
        .map(|d| fit_scale(&d.pop, 1, col_inner))
        .min()
        .unwrap_or(1);
    (day_scale, temps_scale, pop_scale)
}

/// Draws the 7-day forecast strip inside `rect`: one evenly-spaced column
/// per day, each with its day abbreviation, a small condition icon, the
/// high/low temperature, and (when known) the rain probability.
fn draw_forecast_strip(fb: &mut Framebuffer, rect: (u32, u32, u32, u32), days: &[ForecastDayView]) {
    let (x, y, w, h) = rect;
    if days.is_empty() || w < 40 || h < 40 {
        return;
    }
    let n = days.len() as u32;
    let col_w = w / n;
    let col_pad = 3u32;
    // Everything below is sized to fit inside ONE column, not just the
    // strip's height: in portrait mode the panel is much narrower (7 columns
    // in ~400px, not ~1850px), so without this a fixed scale would overlap
    // text from one day into the next. `fit_scale` picks the largest scale
    // that still fits `col_inner`, same technique used everywhere else text
    // has to fit a box; the smallest result across all 7 days is used for
    // all of them, so the row stays visually aligned instead of each column
    // being sized independently.
    let col_inner = col_w.saturating_sub(col_pad * 2).max(1);

    // A thin separator line marks this off as a distinct block from the
    // current-conditions area above it.
    fb.fill_rect(x, y, w, 1, 0x30, 0x30, 0x3C);

    let text = opts().text_color;
    let label_color = text.map_or((0xA0, 0xA0, 0xA8), |c| dim_color(c, 70));

    let (day_scale, temps_scale, pop_scale) = forecast_column_text_scales(days, col_inner);
    let day_h = Framebuffer::text_height(day_scale);
    let temps_h = Framebuffer::text_height(temps_scale);
    let pop_h = Framebuffer::text_height(pop_scale);
    let gap = 4u32;

    let icon_budget_h = h.saturating_sub(day_h + temps_h + pop_h + gap * 4 + 6);
    let icon_scale = (icon_budget_h / weather_icon::ICON_SIZE).clamp(1, 6).min((col_inner / weather_icon::ICON_SIZE).max(1));
    let icon_px = weather_icon::size(icon_scale);

    let content_h = day_h + gap + icon_px + gap + temps_h + gap + pop_h;
    let top = y + 6 + h.saturating_sub(6).saturating_sub(content_h) / 2;

    for (i, day) in days.iter().enumerate() {
        let cx = x + i as u32 * col_w;
        let mut cy = top;

        let dw = Framebuffer::text_width(day.day_label, day_scale);
        fb.draw_text(cx + col_w.saturating_sub(dw) / 2, cy, day.day_label, label_color.0, label_color.1, label_color.2, day_scale);
        cy += day_h + gap;

        let icon_x = cx + col_w.saturating_sub(icon_px) / 2;
        weather_icon::draw(fb, icon_x, cy, icon_scale, day.icon, day.icon_color.0, day.icon_color.1, day.icon_color.2);
        cy += icon_px + gap;

        let tw = Framebuffer::text_width(&day.temps, temps_scale);
        fb.draw_text(cx + col_w.saturating_sub(tw) / 2, cy, &day.temps, day.icon_color.0, day.icon_color.1, day.icon_color.2, temps_scale);
        cy += temps_h + gap;

        if !day.pop.is_empty() {
            let pw = Framebuffer::text_width(&day.pop, pop_scale);
            fb.draw_text(cx + col_w.saturating_sub(pw) / 2, cy, &day.pop, label_color.0, label_color.1, label_color.2, pop_scale);
        }
    }
}

/// Draws the layout inside `content_rect`.
pub fn draw_layout(
    fb: &mut Framebuffer,
    def: &LayoutDef,
    data: &WidgetData,
    color_mode: ColorMode,
    marquee: &mut Marquee,
) {
    if def.standard || def.rows.is_empty() {
        return;
    }
    let (rx, ry, rw, rh) = content_rect(fb);
    let side = 20u32;
    let gap = 16u32;
    let usable_w = rw.saturating_sub(side * 2);

    // (widget, x, y, w, h)
    let mut cells: Vec<(Widget, (u32, u32, u32, u32))> = Vec::new();
    if is_portrait(fb) {
        let widgets: Vec<Widget> = def.rows.iter().flat_map(|r| r.iter().map(|(w, _)| *w)).collect();
        let cols = if widgets.len() > 6 { 2 } else { 1 };
        let rows = (widgets.len() as u32).div_ceil(cols);
        let cw = usable_w.saturating_sub(gap * (cols - 1)) / cols;
        let ch = rh.saturating_sub(gap * (rows - 1)) / rows;
        for (i, w) in widgets.into_iter().enumerate() {
            let (c, r) = (i as u32 % cols, i as u32 / cols);
            cells.push((w, (rx + side + c * (cw + gap), ry + r * (ch + gap), cw, ch)));
        }
    } else {
        let nrows = def.rows.len() as u32;
        let unit = usable_w.saturating_sub(gap * (def.cols - 1)) / def.cols;
        let ch = rh.saturating_sub(gap * (nrows - 1)) / nrows;
        for (ri, row) in def.rows.iter().enumerate() {
            let mut col = 0u32;
            for (w, span) in row.iter() {
                let cw = unit * span + gap * (span - 1);
                cells.push((*w, (rx + side + col * (unit + gap), ry + ri as u32 * (ch + gap), cw, ch)));
                col += span;
            }
        }
    }

    for (w, rect) in cells {
        let view = build_view(w, data, color_mode);
        // Any widget whose View asks to scroll (currently Music, News) shares
        // the one marquee passed into `draw_layout` — keyed off `view.scroll`
        // rather than the widget name, so a future scrolling widget doesn't
        // silently render static text like News initially did.
        let m = if view.scroll { Some(&mut *marquee) } else { None };
        draw_panel(fb, rect, &view, color_mode, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(label: &'static str, temps: &str, pop: &str) -> ForecastDayView {
        ForecastDayView {
            day_label: label,
            icon: WeatherIcon::Clear,
            icon_color: (0xFF, 0xC8, 0x00),
            temps: temps.to_string(),
            pop: pop.to_string(),
        }
    }

    /// Regression test for a real bug: the forecast strip originally used a
    /// fixed text scale (2) regardless of how narrow each of the 7 columns
    /// was, so in portrait mode (~400px total / 7 columns =~ 56px each) the
    /// day name and hi/lo temperatures overlapped into the neighboring
    /// column instead of shrinking to fit. `forecast_column_text_scales`
    /// must pick a scale small enough that every day's text — including the
    /// LONGEST one, e.g. a two-digit-negative "-15/-22" — fits inside a
    /// single column, for any column width down to a narrow portrait one.
    #[test]
    fn forecast_text_scales_never_overlap_a_narrow_column() {
        let days = vec![
            day("FRI", "19/12", "70%"),
            day("SAT", "-15/-22", "20%"), // longest string in the group
            day("SUN", "25/15", ""),
        ];
        // 50px is roughly the real-world floor (a portrait-mode column is
        // ~57px before padding, ~51px inner — see `draw_forecast_strip`);
        // anything narrower can't fit a 7-character string like "-15/-22"
        // even at the minimum scale of 1 (`fit_scale` never goes below 1),
        // which is a hard limit of the glyph size, not a bug to test for.
        for col_inner in [50u32, 56, 90, 260] {
            let (day_scale, temps_scale, pop_scale) = forecast_column_text_scales(&days, col_inner);
            for d in &days {
                assert!(
                    Framebuffer::text_width(d.day_label, day_scale) <= col_inner,
                    "day label {:?} overflows a {col_inner}px column at scale {day_scale}",
                    d.day_label
                );
                assert!(
                    Framebuffer::text_width(&d.temps, temps_scale) <= col_inner,
                    "temps {:?} overflows a {col_inner}px column at scale {temps_scale}",
                    d.temps
                );
                if !d.pop.is_empty() {
                    assert!(
                        Framebuffer::text_width(&d.pop, pop_scale) <= col_inner,
                        "pop {:?} overflows a {col_inner}px column at scale {pop_scale}",
                        d.pop
                    );
                }
            }
        }
    }

    #[test]
    fn forecast_text_scales_default_to_one_with_no_days() {
        let (day_scale, temps_scale, pop_scale) = forecast_column_text_scales(&[], 200);
        assert_eq!((day_scale, temps_scale, pop_scale), (1, 1, 1));
    }
}
