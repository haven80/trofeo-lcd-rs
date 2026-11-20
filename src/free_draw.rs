//! Draws a free layout (see `trofeo_lcd::free`) on the screen: the elements of the file
//! `layouts/<name>.layout`, in order, over the background. The file is looked at again once a
//! second, so a change made in the visual builder (or by hand) shows at once.

use super::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use trofeo_lcd::free::{self, Element, Free, Kind};

mod spectrum_el;

/// The screens of the free layouts: the files read so far and the images loaded for them.
pub struct FreeScreen {
    files: HashMap<String, Cached>,
    images: HashMap<(String, u32, u32), Option<Arc<Picture>>>,
}

struct Cached {
    stamp: Option<(SystemTime, u64)>,
    checked: Instant,
    layout: Result<Arc<Free>, String>,
}

struct Picture {
    w: u32,
    h: u32,
    /// RGBA, straight alpha.
    rgba: Vec<u8>,
}

impl FreeScreen {
    pub fn new() -> FreeScreen {
        FreeScreen { files: HashMap::new(), images: HashMap::new() }
    }

    /// The layout `name`, read again when its file changed (looked at once a second).
    fn layout(&mut self, name: &str, portrait: bool) -> Result<Arc<Free>, String> {
        let dir = free::dir();
        let file = dir.as_ref().map(|d| free::path_of(d, name));
        // a built-in design, unless a file of yours has its name
        if free::builtin(name).is_some() && !file.as_ref().is_some_and(|p| p.is_file()) {
            let key = format!("\0{name}\0{portrait}");
            if let Some(c) = self.files.get(&key) {
                return c.layout.clone();
            }
            let layout = free::load_builtin_for(name, portrait).map(Arc::new);
            self.files.insert(key, Cached { stamp: None, checked: Instant::now(), layout: layout.clone() });
            return layout;
        }
        let dir = dir.ok_or_else(|| "no layouts folder (no configuration file)".to_string())?;
        let path = free::path_of(&dir, name);
        if let Some(c) = self.files.get(name) {
            if c.checked.elapsed() < Duration::from_secs(1) {
                return c.layout.clone();
            }
        }
        let stamp = std::fs::metadata(&path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
        if let Some(c) = self.files.get_mut(name) {
            c.checked = Instant::now();
            if c.stamp == stamp && stamp.is_some() {
                return c.layout.clone();
            }
        }
        let layout = free::load(&dir, name).map(Arc::new);
        if let Err(e) = &layout {
            eprintln!("Free layout not drawn: {e}");
        }
        self.files.insert(name.to_string(), Cached { stamp, checked: Instant::now(), layout: layout.clone() });
        layout
    }

    fn picture(&mut self, file: &str, w: u32, h: u32) -> Option<Arc<Picture>> {
        let key = (file.to_string(), w, h);
        if let Some(p) = self.images.get(&key) {
            return p.clone();
        }
        let mut path = std::path::PathBuf::from(file);
        if path.is_relative() {
            if let Some(base) = free::dir().and_then(|d| d.parent().map(std::path::Path::to_path_buf)) {
                path = base.join(path);
            }
        }
        let pic = image::open(&path).ok().map(|img| {
            let img = img.resize(w.max(1), h.max(1), image::imageops::FilterType::Triangle).to_rgba8();
            Arc::new(Picture { w: img.width(), h: img.height(), rgba: img.into_raw() })
        });
        if self.images.len() > 64 {
            self.images.clear();
        }
        self.images.insert(key, pic.clone());
        pic
    }

    /// Draw the layout `name` (the part of the screen `area`).
    pub fn draw(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32), name: &str, wd: &layouts::WidgetData, color_mode: ColorMode) {
        let layout = match self.layout(name, area.3 > area.2) {
            Ok(l) => l,
            Err(e) => {
                let scale = fit_scale(&e, 3, area.2.saturating_sub(20));
                fb.draw_text(area.0 + 10, area.1 + 10, &e, 0xFF, 0x60, 0x60, scale);
                return;
            }
        };
        let text_color = opts().text_color.unwrap_or((0xF0, 0xF0, 0xF0));
        for e in &layout.elements {
            self.element(fb, area, e, wd, color_mode, text_color);
        }
    }

    fn element(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32), e: &Element, wd: &layouts::WidgetData, color_mode: ColorMode, text_color: Rgb) {
        let (ax, ay, aw, ah) = (area.0 as f32, area.1 as f32, area.2 as f32, area.3 as f32);
        let (w_px, h_px) = ((e.w / 100.0 * aw).round(), (e.h / 100.0 * ah).round());
        match e.kind {
            Kind::Text => {
                let text = resolve(&e.text, wd);
                if text.is_empty() {
                    return;
                }
                // without a width of its own, a text is made smaller to stay on the screen
                let x_px = e.x / 100.0 * aw;
                let room = match e.anchor.fractions().0 {
                    f if f < 0.25 => aw - x_px,
                    f if f > 0.75 => x_px,
                    _ => 2.0 * x_px.min(aw - x_px),
                };
                let limit = if e.w > 0.0 { w_px as u32 } else { room.max(8.0) as u32 };
                let _font = FontGuard::new(e);
                let scale = fit_scale(&text, e.size, limit.max(1));
                let (tw, th) = (Framebuffer::text_width(&text, scale) as f32, Framebuffer::text_height(scale) as f32);
                let (px, py) = free::place(e, aw, ah, tw, th);
                let c = e.color.unwrap_or(text_color);
                put_text(fb, area, ax + px, ay + py, &text, c, scale);
            }
            Kind::Rect => {
                let (px, py) = free::place(e, aw, ah, w_px, h_px);
                let c = e.color.unwrap_or((0x20, 0x20, 0x2C));
                blend_rect(fb, area, ax + px, ay + py, w_px, h_px, c, e.opacity as f32 / 100.0);
            }
            Kind::Bar => {
                let (px, py) = free::place(e, aw, ah, w_px, h_px);
                let (x, y) = (ax + px, ay + py);
                blend_rect(fb, area, x, y, w_px, h_px, e.back, 1.0);
                if let Some((frac, v)) = metric_fraction(e, wd) {
                    let c = bar_color(e, frac, v, color_mode);
                    let fill_w = (w_px * frac).round();
                    match gradient_of(e, v) {
                        Some((c2, phase)) if fill_w >= 1.0 => {
                            // the filled part is painted in strips, each in its own colour
                            let step = 2.0_f32;
                            let mut at = 0.0_f32;
                            while at < fill_w {
                                let sw = step.min(fill_w - at);
                                let t = (at + sw / 2.0) / fill_w;
                                blend_rect(fb, area, x + at, y, sw + 0.5, h_px, gradient_at(e.color.unwrap_or(c), c2, t, phase), 1.0);
                                at += step;
                            }
                        }
                        _ => blend_rect(fb, area, x, y, fill_w, h_px, c, 1.0),
                    }
                }
            }
            Kind::Ring => {
                let d = w_px.min(h_px);
                if d < 4.0 {
                    return;
                }
                let (px, py) = free::place(e, aw, ah, w_px, h_px);
                let (cx, cy) = (ax + px + w_px / 2.0, ay + py + h_px / 2.0);
                let fraction = metric_fraction(e, wd);
                let fill = fraction.map(|(f, v)| bar_color(e, f, v, color_mode)).unwrap_or(e.back);
                let grad = fraction.and_then(|(_, v)| gradient_of(e, v)).map(|(c2, phase)| (e.color.unwrap_or(fill), c2, phase));
                draw_ring(fb, area, (cx, cy), d / 2.0, e.thick / 100.0, fraction.map_or(0.0, |f| f.0), fill, e.back, grad);
                if !e.text.is_empty() {
                    let text = resolve(&e.text, wd);
                    let _font = FontGuard::new(e);
                    let inner = d / 2.0 * (1.0 - e.thick / 100.0);
                    let scale = fit_scale(&text, 40, (inner * 1.7) as u32);
                    let (tw, th) = (Framebuffer::text_width(&text, scale) as f32, Framebuffer::text_height(scale) as f32);
                    put_text(fb, area, cx - tw / 2.0, cy - th / 2.0, &text, e.color.unwrap_or(text_color), scale);
                }
            }
            Kind::Spectrum => {
                let feed = spectrum::feed();
                let (vals, peaks) = spectrum_el::bars_of(e, feed.as_ref());
                spectrum_el::draw(fb, area, e, &vals, &peaks);
            }
            Kind::Image => {
                if w_px < 1.0 || h_px < 1.0 {
                    return;
                }
                if let Some(pic) = self.picture(&e.file, w_px as u32, h_px as u32) {
                    let (px, py) = free::place(e, aw, ah, pic.w as f32, pic.h as f32);
                    blit(fb, area, ax + px, ay + py, &pic, e.opacity as f32 / 100.0);
                }
            }
        }
    }
}

type Rgb = (u8, u8, u8);

/// A text with its `{...}` filled in.
fn resolve(text: &str, wd: &layouts::WidgetData) -> String {
    free::substitute(text, &|name| placeholder(name, wd))
}

fn gb(mb: u64) -> String {
    format!("{:.1}GB", mb as f64 / 1024.0)
}

fn rate(kb: f64) -> String {
    if kb >= 1024.0 { format!("{:.1}MB/S", kb / 1024.0) } else { format!("{kb:.0}KB/S") }
}

fn placeholder(name: &str, wd: &layouts::WidgetData) -> String {
    let dash = || "--".to_string();
    let now = Local::now();
    let ram_pct = wd.used_mb as f32 * 100.0 / wd.total_mb.max(1) as f32;
    match name {
        "cpu" => fmt_pct(wd.cpu_pct),
        "gpu" => wd.gpu_pct.map(fmt_pct).unwrap_or_else(dash),
        "ram" => fmt_pct(ram_pct),
        "cpu_temp" => wd.cpu_temp.map(|t| format!("{t:.0}C")).unwrap_or_else(dash),
        "gpu_temp" => wd.gpu_temp.map(|t| format!("{t}C")).unwrap_or_else(dash),
        "gpu_hotspot" => wd.gpu_hotspot.map(|t| format!("{t}C")).unwrap_or_else(dash),
        "cpu_power" => wd.cpu_power.map(|p| format!("{p:.0}W")).unwrap_or_else(dash),
        "gpu_power" => wd.gpu_power.map(|p| format!("{p}W")).unwrap_or_else(dash),
        "cpu_mhz" => wd.cpu_mhz.map(|m| m.to_string()).unwrap_or_else(dash),
        "cpu_freq" => wd.cpu_mhz.map(format_freq_mhz).unwrap_or_else(dash),
        "fps" => wd.fps.map(|f| f.to_string()).unwrap_or_else(dash),
        "ram_used" => gb(wd.used_mb),
        "ram_free" => gb(wd.total_mb.saturating_sub(wd.used_mb)),
        "ram_total" => gb(wd.total_mb),
        "net_down" => rate(wd.net_kb.0),
        "net_up" => rate(wd.net_kb.1),
        "disk_read" => format!("{:.1}MB/S", wd.disk_mb.0),
        "disk_write" => format!("{:.1}MB/S", wd.disk_mb.1),
        "song" => wd.now_playing.clone().unwrap_or_default(),
        "time" => fmt_clock(&now, true),
        "time_short" => fmt_clock(&now, false),
        "date" => i18n::date_long(&now),
        "weather_temp" => wd
            .weather
            .as_ref()
            .map(|w| if opts().weather_fahrenheit { format!("{:.0}F", w.temp_c * 9.0 / 5.0 + 32.0) } else { format!("{:.0}C", w.temp_c) })
            .unwrap_or_else(dash),
        "weather_city" => wd.weather.as_ref().and_then(|w| w.city.clone()).unwrap_or_default(),
        _ => dash(),
    }
}

/// The number a bar or ring shows.
fn metric_number(id: &str, wd: &layouts::WidgetData) -> Option<f32> {
    match id {
        "cpu" => Some(wd.cpu_pct),
        "gpu" => wd.gpu_pct,
        "ram" => Some(wd.used_mb as f32 * 100.0 / wd.total_mb.max(1) as f32),
        "cpu_temp" => wd.cpu_temp,
        "gpu_temp" => wd.gpu_temp.map(|t| t as f32),
        "gpu_hotspot" => wd.gpu_hotspot.map(|t| t as f32),
        "cpu_power" => wd.cpu_power,
        "gpu_power" => wd.gpu_power.map(|p| p as f32),
        "cpu_mhz" => wd.cpu_mhz.map(|m| m as f32),
        "fps" => wd.fps.map(|f| f as f32),
        _ => None,
    }
}

/// (0..1 along min..max, the number) — None when the reading is not available.
fn metric_fraction(e: &Element, wd: &layouts::WidgetData) -> Option<(f32, f32)> {
    let v = metric_number(&e.value, wd)?;
    let max = e.max.unwrap_or_else(|| free::metric_max(&e.value));
    Some((((v - e.min) / (max - e.min)).clamp(0.0, 1.0), v))
}

/// Seconds since the program started (or the time of a preview picture): the clock of the
/// moving gradients.
static PREVIEW_SECS: std::sync::Mutex<Option<f64>> = std::sync::Mutex::new(None);

pub fn set_preview_secs(secs: Option<f64>) {
    *PREVIEW_SECS.lock().unwrap_or_else(|e| e.into_inner()) = secs;
}

pub(crate) fn anim_secs() -> f64 {
    if let Some(s) = *PREVIEW_SECS.lock().unwrap_or_else(|e| e.into_inner()) {
        return s;
    }
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// The second colour of a bar or ring and where its gradient is now (0..1 of a loop; 0 when
/// the animation is off or the gradient is still: -1), unless the value is over `hot`.
fn gradient_of(e: &Element, v: f32) -> Option<(Rgb, f32)> {
    let c2 = e.color2?;
    if e.hot.is_some_and(|h| v >= h) {
        return None;
    }
    let phase = if e.flow > 0.0 && opts().bar_animation { ((anim_secs() / e.flow as f64).fract()) as f32 } else { -1.0 };
    Some((c2, phase))
}

/// The colour at `t` (0..1 along the filled part). Still (`phase` < 0): a straight run from
/// `c1` to `c2`. Moving (`phase` 0..1 of a loop): a wave of the two colours that travels along.
fn gradient_at(c1: Rgb, c2: Rgb, t: f32, phase: f32) -> Rgb {
    let k = if phase >= 0.0 { 0.5 - 0.5 * ((t - phase) * std::f32::consts::TAU).cos() } else { t.clamp(0.0, 1.0) };
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round().clamp(0.0, 255.0) as u8;
    (m(c1.0, c2.0), m(c1.1, c2.1), m(c1.2, c2.2))
}

/// Does the layout `name` have a moving gradient? Then the screen wants a steady frame rate.
impl FreeScreen {
    pub fn wanted_fps(&mut self, name: &str, portrait: bool) -> f32 {
        match self.layout(name, portrait) {
            Ok(l) if l.animated() => 20.0,
            _ => 0.0,
        }
    }
}

impl FreeScreen {
    /// Does the layout `name` have a spectrum in it? Then it follows the music.
    pub fn has_spectrum(&mut self, name: &str, portrait: bool) -> bool {
        self.layout(name, portrait).is_ok_and(|l| l.has_spectrum())
    }

    /// Does a spectrum in it listen to the left or the right channel only?
    pub fn has_stereo_spectrum(&mut self, name: &str, portrait: bool) -> bool {
        self.layout(name, portrait).is_ok_and(|l| l.has_stereo_spectrum())
    }
}

fn bar_color(e: &Element, frac: f32, v: f32, color_mode: ColorMode) -> Rgb {
    if e.hot.is_some_and(|h| v >= h) {
        return e.hot_color;
    }
    e.color.unwrap_or_else(|| level_color(frac.max(0.2), color_mode))
}

/// Text at a pixel position that may be partly outside the area (it is then not drawn
/// beyond the screen's edge).
/// The font of an element for as long as it lives: the text style is put back when it is dropped.
struct FontGuard((trofeo_lcd::TextFont, Option<trofeo_lcd::smooth_font::Weight>));

impl FontGuard {
    fn new(e: &Element) -> FontGuard {
        let before = trofeo_lcd::text_style();
        if e.font.is_some() || e.weight.is_some() {
            trofeo_lcd::set_text_style(e.font.unwrap_or(before.0), e.weight.or(before.1));
        }
        FontGuard(before)
    }
}

impl Drop for FontGuard {
    fn drop(&mut self) {
        trofeo_lcd::set_text_style(self.0 .0, self.0 .1);
    }
}

fn put_text(fb: &mut Framebuffer, area: (u32, u32, u32, u32), x: f32, y: f32, text: &str, c: Rgb, scale: u32) {
    let (x, y) = (x.round().max(0.0) as u32, y.round().max(0.0) as u32);
    if x >= fb.width() || y >= fb.height() || x >= area.0 + area.2 + 200 {
        return;
    }
    fb.draw_text(x, y, text, c.0, c.1, c.2, scale);
}

/// The part of a rectangle that is inside the area and the picture.
fn clip(fb: &Framebuffer, area: (u32, u32, u32, u32), x: f32, y: f32, w: f32, h: f32) -> Option<(u32, u32, u32, u32)> {
    let x0 = x.round().max(area.0 as f32).max(0.0);
    let y0 = y.round().max(area.1 as f32).max(0.0);
    let x1 = (x + w).round().min((area.0 + area.2) as f32).min(fb.width() as f32);
    let y1 = (y + h).round().min((area.1 + area.3) as f32).min(fb.height() as f32);
    (x1 > x0 && y1 > y0).then(|| (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
}

fn mix(px: &mut [u8], c: Rgb, a: f32) {
    for (d, s) in px.iter_mut().zip([c.0, c.1, c.2]) {
        *d = (*d as f32 + (s as f32 - *d as f32) * a).round().clamp(0.0, 255.0) as u8;
    }
}

fn blend_rect(fb: &mut Framebuffer, area: (u32, u32, u32, u32), x: f32, y: f32, w: f32, h: f32, c: Rgb, alpha: f32) {
    let Some((x0, y0, cw, ch)) = clip(fb, area, x, y, w, h) else { return };
    if alpha >= 0.999 {
        fb.fill_rect(x0, y0, cw, ch, c.0, c.1, c.2);
        return;
    }
    if alpha <= 0.0 {
        return;
    }
    let stride = fb.width() as usize;
    let px = fb.as_bytes_mut();
    for yy in y0..y0 + ch {
        for xx in x0..x0 + cw {
            let i = (yy as usize * stride + xx as usize) * 3;
            mix(&mut px[i..i + 3], c, alpha);
        }
    }
}

/// A ring: `frac` of it (clockwise from the top) in `fill`, the rest in `back`.
#[allow(clippy::too_many_arguments)]
fn draw_ring(fb: &mut Framebuffer, area: (u32, u32, u32, u32), center: (f32, f32), radius: f32, thick: f32, frac: f32, fill: Rgb, back: Rgb, grad: Option<(Rgb, Rgb, f32)>) {
    let inner = radius * (1.0 - thick.clamp(0.02, 1.0));
    let Some((x0, y0, cw, ch)) = clip(fb, area, center.0 - radius - 1.0, center.1 - radius - 1.0, radius * 2.0 + 2.0, radius * 2.0 + 2.0) else { return };
    let stride = fb.width() as usize;
    let px = fb.as_bytes_mut();
    for yy in y0..y0 + ch {
        for xx in x0..x0 + cw {
            let (dx, dy) = (xx as f32 + 0.5 - center.0, yy as f32 + 0.5 - center.1);
            let dist = dx.hypot(dy);
            let cover = (radius + 0.5 - dist).clamp(0.0, 1.0) * (dist - (inner - 0.5)).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let mut turn = dx.atan2(-dy) / std::f32::consts::TAU;
            if turn < 0.0 {
                turn += 1.0;
            }
            let c = if turn > frac {
                back
            } else if let (Some((c1, c2, phase)), true) = (grad, frac > 0.0) {
                gradient_at(c1, c2, turn / frac, phase)
            } else {
                fill
            };
            let i = (yy as usize * stride + xx as usize) * 3;
            mix(&mut px[i..i + 3], c, cover);
        }
    }
}

fn blit(fb: &mut Framebuffer, area: (u32, u32, u32, u32), x: f32, y: f32, pic: &Picture, opacity: f32) {
    let Some((x0, y0, cw, ch)) = clip(fb, area, x, y, pic.w as f32, pic.h as f32) else { return };
    let stride = fb.width() as usize;
    let (ox, oy) = (x.round() as i64, y.round() as i64);
    let px = fb.as_bytes_mut();
    for yy in y0..y0 + ch {
        for xx in x0..x0 + cw {
            let (sx, sy) = ((xx as i64 - ox) as usize, (yy as i64 - oy) as usize);
            if sx >= pic.w as usize || sy >= pic.h as usize {
                continue;
            }
            let s = (sy * pic.w as usize + sx) * 4;
            let a = pic.rgba[s + 3] as f32 / 255.0 * opacity;
            if a <= 0.0 {
                continue;
            }
            let i = (yy as usize * stride + xx as usize) * 3;
            mix(&mut px[i..i + 3], (pic.rgba[s], pic.rgba[s + 1], pic.rgba[s + 2]), a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> layouts::WidgetData {
        layouts::WidgetData {
            cpu_pct: 37.0,
            cpu_temp: Some(64.0),
            gpu_pct: Some(46.0),
            gpu_temp: Some(58),
            used_mb: 2048,
            total_mb: 8192,
            net_kb: (850.0, 2048.0),
            now_playing: Some("Song".into()),
            ..layouts::WidgetData::default()
        }
    }

    #[test]
    fn every_placeholder_has_a_value() {
        let d = data();
        for p in free::PLACEHOLDERS {
            assert_ne!(placeholder(p, &d), "--x", "{p}");
        }
        assert_eq!(placeholder("cpu_temp", &d), "64C");
        assert_eq!(placeholder("gpu_power", &d), "--", "not available");
        assert_eq!(placeholder("ram", &d), fmt_pct(25.0));
        assert_eq!(placeholder("ram_used", &d), "2.0GB");
        assert_eq!(placeholder("net_up", &d), "2.0MB/S");
        assert_eq!(resolve("CPU {cpu} {song}", &d), format!("CPU {} Song", fmt_pct(37.0)));
        for (m, _, _) in free::METRICS {
            // every metric the builder offers is one the program can show (None only when absent)
            let _ = metric_number(m, &layouts::WidgetData { cpu_temp: Some(1.0), gpu_pct: Some(1.0), gpu_temp: Some(1), gpu_hotspot: Some(1), cpu_power: Some(1.0), gpu_power: Some(1), cpu_mhz: Some(1), fps: Some(1), ..data() })
                .unwrap_or_else(|| panic!("{m} has no value"));
        }
    }

    #[test]
    fn a_bar_follows_its_range_and_turns_hot() {
        let d = data();
        let mut e = Element::new(Kind::Bar);
        e.value = "cpu_temp".into();
        e.min = 20.0;
        e.max = Some(100.0);
        e.hot = Some(60.0);
        let (f, v) = metric_fraction(&e, &d).unwrap();
        assert!((f - 0.55).abs() < 1e-6 && v == 64.0);
        assert_eq!(bar_color(&e, f, v, ColorMode::Default), e.hot_color);
        e.hot = Some(70.0);
        assert_ne!(bar_color(&e, f, v, ColorMode::Default), e.hot_color);
        e.value = "gpu_power".into();
        assert!(metric_fraction(&e, &d).is_none(), "no reading: no fill");
    }

    #[test]
    fn a_layout_is_drawn_on_the_screen_and_a_broken_one_says_so() {
        let dir = std::env::temp_dir().join(format!("trofeo_free_draw_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = free::parse("rect x=0 y=0 w=100 h=100 color=#102030\nbar x=10 y=50 w=50 h=10 value=cpu color=#FF0000\nring x=80 y=50 w=20 h=60 anchor=c value=gpu text=\"{gpu}\"\ntext x=50 y=10 anchor=c size=6 text=\"{time}\"\n").unwrap();
        free::save(&dir, "demo", &f).unwrap();
        std::fs::write(free::path_of(&dir, "broken"), "text x=\n").unwrap();
        free::set_dir(Some(dir.clone()));
        let mut screen = FreeScreen::new();
        let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(960, 240));
        screen.draw(&mut fb, (0, 0, 960, 240), "demo", &data(), ColorMode::Default);
        let mut px = |x: usize, y: usize| {
            let i = (y * 960 + x) * 3;
            let b = fb.as_bytes_mut();
            (b[i], b[i + 1], b[i + 2])
        };
        assert_eq!(px(5, 5), (0x10, 0x20, 0x30), "the rect");
        assert_eq!(px(100, 130), (0xFF, 0x00, 0x00), "the bar's fill (37% of 50% of the width = x 96..)");
        assert_ne!(px(500, 130), (0x10, 0x20, 0x30), "the track of the bar");
        let mut fb2 = Framebuffer::new(trofeo_lcd::Resolution::new(960, 240));
        screen.draw(&mut fb2, (0, 0, 960, 240), "broken", &data(), ColorMode::Default);
        assert!(fb2.as_bytes_mut().iter().any(|b| *b != 0), "the error is written on the screen");
        free::set_dir(None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gradient_runs_from_one_colour_to_the_other_and_a_moving_one_travels() {
        let (a, b) = ((0, 0, 0), (200, 100, 50));
        assert_eq!(gradient_at(a, b, 0.0, -1.0), a);
        assert_eq!(gradient_at(a, b, 1.0, -1.0), b);
        assert_eq!(gradient_at(a, b, 0.5, -1.0), (100, 50, 25));
        // moving: the wave is at its first colour where the phase is, and at the other half a loop away
        assert_eq!(gradient_at(a, b, 0.25, 0.25), a);
        assert_eq!(gradient_at(a, b, 0.75, 0.25), b);
        assert_ne!(gradient_at(a, b, 0.25, 0.0), gradient_at(a, b, 0.25, 0.25));
    }

    #[test]
    fn a_bar_with_a_gradient_is_painted_in_two_colours_and_flows_with_the_clock() {
        let f = free::parse("bar x=0 y=0 w=100 h=100 value=cpu max=37 color=#000000 color2=#FF0000 flow=4").unwrap();
        let e = &f.elements[0];
        let draw = |secs: f64| {
            set_preview_secs(Some(secs));
            let mut screen = FreeScreen::new();
            let mut fb = Framebuffer::new(trofeo_lcd::Resolution::new(400, 40));
            screen.element(&mut fb, (0, 0, 400, 40), e, &data(), ColorMode::Default, (255, 255, 255));
            (0..400).map(|x| fb.as_bytes_mut()[(20 * 400 + x) * 3]).collect::<Vec<u8>>()
        };
        let a = draw(0.0);
        assert!(a.iter().any(|r| *r > 200) && a.iter().any(|r| *r < 50), "both colours are there");
        assert_ne!(a, draw(1.0), "a second later the pattern has moved");
        assert_eq!(a, draw(4.0), "after a loop it is the same");
        set_preview_secs(None);
        assert!(free::parse("bar x=0 y=0 value=cpu color2=#FF0000 flow=4").unwrap().animated());
    }
}
