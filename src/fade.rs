//! Fade between screens: when the screen on the display changes (the layout rotation moves on,
//! the music screen or the gaming screen starts or ends…) the old picture melts into the new
//! one instead of switching at once. `screen_fade_ms` is the length in milliseconds (0 = off);
//! `screen_fade_style` picks a crossfade or a fade through black.

use std::time::Instant;

/// How the old picture turns into the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    /// The old picture dissolves into the new one.
    #[default]
    Cross,
    /// The old picture fades out to black, then the new one fades in.
    Black,
}

impl Style {
    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cross" | "crossfade" => Some(Style::Cross),
            "black" | "dip" => Some(Style::Black),
            _ => None,
        }
    }
}

/// Longest fade accepted (ms).
pub const MAX_MS: u32 = 5000;

/// Smooth 0..=256 weight for a progress `t` in 0..=1 (smoothstep, so it starts and ends gently).
pub fn smooth(t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    ((t * t * (3.0 - 2.0 * t)) * 256.0).round() as u32
}

/// `to = from * (256 - w) + to * w` (all 0..=256 fixed point), byte by byte.
pub fn mix(from: &[u8], to: &mut [u8], w: u32) {
    let w = w.min(256);
    let iw = 256 - w;
    for (t, &f) in to.iter_mut().zip(from) {
        *t = ((f as u32 * iw + *t as u32 * w) >> 8) as u8;
    }
}

/// `buf = buf * w / 256` (towards black).
pub fn dim(buf: &mut [u8], w: u32) {
    let w = w.min(256);
    for b in buf.iter_mut() {
        *b = ((*b as u32 * w) >> 8) as u8;
    }
}

#[derive(Default)]
pub struct Fade {
    /// The last picture sent to the display (what the next fade starts from).
    last: Vec<u8>,
    /// The picture the running fade starts from.
    from: Vec<u8>,
    id: Option<u64>,
    running: Option<(Instant, u32, Style)>,
}

impl Fade {
    pub fn new() -> Fade {
        Fade::default()
    }

    /// A fade is in progress (the program draws at a steady pace meanwhile).
    pub fn active(&self, now: Instant) -> bool {
        self.running.is_some_and(|(t0, ms, _)| now.duration_since(t0).as_millis() < ms as u128)
    }

    /// Start of a frame: `id` identifies the screen about to be drawn. A different id than the
    /// previous frame starts a fade (if there is a previous picture and `ms > 0`).
    pub fn begin(&mut self, id: u64, ms: u32, style: Style, now: Instant) {
        if ms == 0 {
            self.last.clear();
            self.running = None;
            self.id = Some(id);
            return;
        }
        if self.id.is_some_and(|p| p != id) && !self.last.is_empty() {
            self.from.clear();
            self.from.extend_from_slice(&self.last);
            self.running = Some((now, ms.min(MAX_MS), style));
        }
        self.id = Some(id);
    }

    /// End of a frame (before the overlays): blends `fb` (RGB bytes) with the starting picture
    /// while a fade runs, and remembers what is about to be shown.
    pub fn finish(&mut self, fb: &mut [u8], ms: u32, now: Instant) {
        if ms == 0 {
            return;
        }
        if let Some((t0, total, style)) = self.running {
            let t = now.duration_since(t0).as_secs_f32() / (total as f32 / 1000.0);
            if t >= 1.0 || self.from.len() != fb.len() {
                self.running = None;
            } else {
                match style {
                    Style::Cross => mix(&self.from, fb, smooth(t)),
                    Style::Black if t < 0.5 => {
                        fb.copy_from_slice(&self.from);
                        dim(fb, 256 - smooth(t * 2.0));
                    }
                    Style::Black => dim(fb, smooth(t * 2.0 - 1.0)),
                }
            }
        }
        self.last.clear();
        self.last.extend_from_slice(fb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn smooth_goes_from_0_to_256() {
        assert_eq!(smooth(0.0), 0);
        assert_eq!(smooth(1.0), 256);
        assert_eq!(smooth(0.5), 128);
        assert!(smooth(0.25) < 64);
    }

    #[test]
    fn mix_blends_the_two_pictures() {
        let from = [200u8, 0, 100];
        let mut to = [0u8, 200, 100];
        mix(&from, &mut to, 128);
        assert_eq!(to, [100, 100, 100]);
        let mut a = [10u8, 20, 30];
        mix(&from, &mut a, 0);
        assert_eq!(a, from);
        let mut b = [10u8, 20, 30];
        mix(&from, &mut b, 256);
        assert_eq!(b, [10, 20, 30]);
    }

    #[test]
    fn nothing_happens_when_it_is_off() {
        let t0 = Instant::now();
        let mut f = Fade::new();
        let mut px = vec![9u8; 12];
        f.begin(1, 0, Style::Cross, t0);
        f.finish(&mut px, 0, t0);
        f.begin(2, 0, Style::Cross, t0);
        assert!(!f.active(t0));
        f.finish(&mut px, 0, t0);
        assert_eq!(px, vec![9u8; 12]);
    }

    #[test]
    fn a_screen_change_runs_a_crossfade_of_the_requested_length() {
        let t0 = Instant::now();
        let mut f = Fade::new();
        let mut a = vec![0u8; 6];
        f.begin(1, 1000, Style::Cross, t0);
        f.finish(&mut a, 1000, t0);
        assert!(!f.active(t0), "the first screen does not fade in from nothing");
        // the screen changes: white picture
        let t1 = t0 + Duration::from_millis(100);
        f.begin(2, 1000, Style::Cross, t1);
        assert!(f.active(t1));
        let mut b = vec![255u8; 6];
        f.finish(&mut b, 1000, t1);
        assert!(b.iter().all(|&v| v == 0), "starts from the old picture, got {b:?}");
        let mut c = vec![255u8; 6];
        f.begin(2, 1000, Style::Cross, t1 + Duration::from_millis(500));
        f.finish(&mut c, 1000, t1 + Duration::from_millis(500));
        assert!(c.iter().all(|&v| (120..=135).contains(&v)), "halfway is grey, got {c:?}");
        let end = t1 + Duration::from_millis(1000);
        assert!(!f.active(end));
        let mut d = vec![255u8; 6];
        f.finish(&mut d, 1000, end);
        assert!(d.iter().all(|&v| v == 255));
    }

    #[test]
    fn fading_through_black_goes_dark_in_the_middle() {
        let t0 = Instant::now();
        let mut f = Fade::new();
        let mut a = vec![200u8; 6];
        f.begin(1, 1000, Style::Black, t0);
        f.finish(&mut a, 1000, t0);
        f.begin(2, 1000, Style::Black, t0);
        let mut mid = vec![100u8; 6];
        f.finish(&mut mid, 1000, t0 + Duration::from_millis(500));
        assert!(mid.iter().all(|&v| v <= 1), "black in the middle, got {mid:?}");
        let mut q = vec![100u8; 6];
        f.finish(&mut q, 1000, t0 + Duration::from_millis(250));
        assert!(q.iter().all(|&v| v > 20 && v < 200));
    }

    #[test]
    fn styles_parse() {
        assert_eq!(Style::parse("cross"), Some(Style::Cross));
        assert_eq!(Style::parse("Black"), Some(Style::Black));
        assert_eq!(Style::parse("x"), None);
    }
}
