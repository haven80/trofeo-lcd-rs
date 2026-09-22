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
        },
        None => View { label: label.into(), value: "--".into(), detail: na(), gauge: None, value_color: None, scroll: false, icon: None },
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
        },
        Disk => View {
            label: format!("{} {}", tr.disk, tr.disk_read),
            value: format!("{:.1}MB/S", d.disk_mb.0),
            detail: format!("{} {:.1}MB/S", tr.disk_write, d.disk_mb.1),
            gauge: None,
            value_color: None,
            scroll: false,
            icon: None,
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
        },
        Clock => View {
            label: i18n::weekday(&now),
            value: now.format("%H:%M:%S").to_string(),
            detail: i18n::day_month_year(&now),
            gauge: None,
            value_color: opts().clock_color.or_else(|| Some(accent_color(color_mode))),
            scroll: false,
            icon: None,
        },
        Music => View {
            label: tr.now_playing.to_string(),
            value: d.now_playing.clone().unwrap_or_else(|| "-".into()),
            detail: String::new(),
            gauge: None,
            value_color: None,
            scroll: true,
            icon: None,
        },
        Weather => match &d.weather {
            Some(w) => {
                let fahrenheit = opts().weather_fahrenheit;
                let unit = if fahrenheit { "F" } else { "C" };
                let color = weather_icon::default_color(w.icon);
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
    let mid_h = bottom_limit.saturating_sub(top_limit);
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
        let m = if w == Music { Some(&mut *marquee) } else { None };
        draw_panel(fb, rect, &view, color_mode, m);
    }
}
