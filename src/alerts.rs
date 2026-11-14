//! Threshold alerts (2.19.0): when a reading stays above its limit for a few seconds, the
//! screen gets a pulsing border and a bar with the value. Everything is off by default
//! (every limit is 0) and costs nothing while off: no state is touched and nothing is drawn.

use std::time::{Duration, Instant};

use crate::Framebuffer;

/// How far below the limit a reading must fall before the alert clears (avoids flicker).
pub const HYSTERESIS: f32 = 2.0;
pub const MAX_DELAY_SECS: u32 = 600;

/// What to watch. A limit of 0 means "off".
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub cpu_temp: u32,
    pub gpu_temp: u32,
    pub cpu_load: u32,
    pub gpu_load: u32,
    pub ram: u32,
    /// Seconds a reading must stay over its limit before the alert shows.
    pub delay_secs: u32,
    pub color: (u8, u8, u8),
}

impl Default for Rules {
    fn default() -> Rules {
        Rules { cpu_temp: 0, gpu_temp: 0, cpu_load: 0, gpu_load: 0, ram: 0, delay_secs: 3, color: (255, 40, 40) }
    }
}

impl Rules {
    pub fn any(&self) -> bool {
        self.limits().iter().any(|&l| l > 0)
    }

    fn limits(&self) -> [u32; 5] {
        [self.cpu_temp, self.gpu_temp, self.cpu_load, self.gpu_load, self.ram]
    }
}

/// The latest readings (None = not available on this PC).
#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub cpu_load: Option<f32>,
    pub gpu_load: Option<f32>,
    pub ram: Option<f32>,
}

impl Reading {
    fn values(&self) -> [Option<f32>; 5] {
        [self.cpu_temp, self.gpu_temp, self.cpu_load, self.gpu_load, self.ram]
    }
}

const LABELS: [&str; 5] = ["CPU TEMP", "GPU TEMP", "CPU LOAD", "GPU LOAD", "RAM"];

/// An alert that is showing now.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub text: String,
}

#[derive(Default)]
pub struct State {
    over_since: [Option<Instant>; 5],
    active: [bool; 5],
}

impl State {
    pub fn new() -> State {
        State::default()
    }

    /// Feed the latest readings; returns the alert to show (the first active one), if any.
    pub fn update(&mut self, rules: &Rules, now: Instant, reading: &Reading) -> Option<Active> {
        if !rules.any() {
            *self = State::default();
            return None;
        }
        let delay = Duration::from_secs(rules.delay_secs as u64);
        let limits = rules.limits();
        let values = reading.values();
        let mut shown: Option<Active> = None;
        for i in 0..5 {
            let (limit, value) = (limits[i] as f32, values[i]);
            let over = match value {
                Some(v) if limits[i] > 0 => v > limit || (self.active[i] && v > limit - HYSTERESIS),
                _ => false,
            };
            if !over {
                self.over_since[i] = None;
                self.active[i] = false;
                continue;
            }
            let since = *self.over_since[i].get_or_insert(now);
            if now.duration_since(since) >= delay {
                self.active[i] = true;
            }
            if self.active[i] && shown.is_none() {
                let v = value.unwrap_or(0.0);
                let unit = if i < 2 { "C" } else { "%" };
                shown = Some(Active { text: format!("{} {:.0}{}", LABELS[i], v, unit) });
            }
        }
        shown
    }
}

/// Pulse between 0.25 and 1.0, one beat per second.
pub fn pulse(ms: u128) -> f32 {
    let phase = (ms % 1000) as f32 / 1000.0;
    0.625 + 0.375 * (phase * std::f32::consts::TAU).sin()
}

fn blend_px(px: &mut [u8], c: (u8, u8, u8), a: f32) {
    for (d, s) in px.iter_mut().zip([c.0, c.1, c.2]) {
        *d = (*d as f32 + (s as f32 - *d as f32) * a).round().clamp(0.0, 255.0) as u8;
    }
}

/// A pulsing border around the picture and a bar at the top with the reading.
pub fn draw(fb: &mut Framebuffer, alert: &Active, color: (u8, u8, u8), ms: u128) {
    let (w, h) = (fb.width() as usize, fb.height() as usize);
    if w < 40 || h < 40 {
        return;
    }
    let a = pulse(ms);
    let t = (w.min(h) / 40).max(3);
    let px = fb.as_bytes_mut();
    for y in 0..h {
        let edge_y = y < t || y >= h - t;
        for x in 0..w {
            if edge_y || x < t || x >= w - t {
                let i = (y * w + x) * 3;
                blend_px(&mut px[i..i + 3], color, a);
            }
        }
    }
    // Bar with the text, near the top.
    let mut scale = 4u32;
    while scale > 1 && Framebuffer::text_width(&alert.text, scale) + 4 * t as u32 + 24 > w as u32 {
        scale -= 1;
    }
    let text_h = Framebuffer::text_height(scale);
    let bar_h = text_h + 20;
    let tw = Framebuffer::text_width(&alert.text, scale);
    let bar_w = (tw + 40).min(w as u32 - 2 * t as u32);
    let x0 = (w as u32 - bar_w) / 2;
    let y0 = t as u32 + 6;
    fb.fill_rect(x0, y0, bar_w, bar_h, 18, 20, 28);
    fb.fill_rect(x0, y0, bar_w, 3, color.0, color.1, color.2);
    fb.draw_text(x0 + (bar_w - tw.min(bar_w)) / 2, y0 + 10, &alert.text, 255, 255, 255, scale);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resolution;

    fn rules() -> Rules {
        Rules { cpu_temp: 85, ram: 90, delay_secs: 3, ..Rules::default() }
    }

    fn hot(t: f32) -> Reading {
        Reading { cpu_temp: Some(t), ..Reading::default() }
    }

    #[test]
    fn nothing_is_watched_by_default() {
        assert!(!Rules::default().any());
        let mut s = State::new();
        assert!(s.update(&Rules::default(), Instant::now(), &hot(120.0)).is_none());
    }

    #[test]
    fn it_waits_for_the_delay_then_shows_and_clears_with_hysteresis() {
        let (r, t0) = (rules(), Instant::now());
        let mut s = State::new();
        assert!(s.update(&r, t0, &hot(90.0)).is_none(), "just started");
        assert!(s.update(&r, t0 + Duration::from_secs(2), &hot(90.0)).is_none());
        let a = s.update(&r, t0 + Duration::from_secs(3), &hot(91.0)).expect("after the delay");
        assert_eq!(a.text, "CPU TEMP 91C");
        assert!(s.update(&r, t0 + Duration::from_secs(4), &hot(84.0)).is_some(), "inside the hysteresis");
        assert!(s.update(&r, t0 + Duration::from_secs(5), &hot(82.0)).is_none(), "cleared");
        assert!(s.update(&r, t0 + Duration::from_secs(6), &hot(90.0)).is_none(), "the delay starts again");
    }

    #[test]
    fn a_short_spike_is_ignored() {
        let (r, t0) = (rules(), Instant::now());
        let mut s = State::new();
        s.update(&r, t0, &hot(95.0));
        s.update(&r, t0 + Duration::from_secs(1), &hot(60.0));
        assert!(s.update(&r, t0 + Duration::from_secs(4), &hot(95.0)).is_none());
    }

    #[test]
    fn missing_readings_never_alert_and_units_are_right() {
        let (r, t0) = (rules(), Instant::now());
        let mut s = State::new();
        assert!(s.update(&r, t0, &Reading::default()).is_none());
        s.update(&r, t0, &Reading { ram: Some(95.0), ..Reading::default() });
        let a = s.update(&r, t0 + Duration::from_secs(3), &Reading { ram: Some(95.0), ..Reading::default() });
        assert_eq!(a.unwrap().text, "RAM 95%");
    }

    #[test]
    fn drawing_changes_the_border_and_stays_inside_the_picture() {
        let mut fb = Framebuffer::new(Resolution::new(480, 320));
        let before = fb.as_bytes_mut().to_vec();
        draw(&mut fb, &Active { text: "CPU TEMP 91C".into() }, (255, 40, 40), 250);
        assert_ne!(before, fb.as_bytes_mut().to_vec());
        assert!(pulse(0) >= 0.25 && pulse(250) <= 1.0 + f32::EPSILON);
        // Tiny pictures are left alone.
        let mut tiny = Framebuffer::new(Resolution::new(20, 20));
        draw(&mut tiny, &Active { text: "X".into() }, (255, 0, 0), 0);
    }
}
