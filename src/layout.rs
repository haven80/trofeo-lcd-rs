//! Positioning (9 anchor points), visible elements, and background fit
//! modes. Shared by `trofeo_lcd` and `background`.

/// Anchor in a 3x3 grid. `ax`/`ay`: 0 = left/top, 1 = center, 2 = right/bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub ax: u8,
    pub ay: u8,
}

impl Anchor {
    pub const CENTER: Anchor = Anchor { ax: 1, ay: 1 };
    pub const TOP_LEFT: Anchor = Anchor { ax: 0, ay: 0 };

    /// Accepts: center, center-left, center-right, top, top-left, top-right,
    /// bottom, bottom-left, bottom-right (also in Italian: centro, alto,
    /// basso, sinistra, destra; separators '-', '_' or space).
    pub fn parse(s: &str) -> Option<Anchor> {
        let lower = s.trim().to_ascii_lowercase();
        let mut a = Anchor::CENTER;
        let mut any = false;
        for tok in lower.split(|c: char| c == '-' || c == '_' || c == ' ').filter(|t| !t.is_empty()) {
            any = true;
            match tok {
                "center" | "centre" | "centro" | "centrato" => {}
                "top" | "alto" | "su" => a.ay = 0,
                "bottom" | "basso" | "giu" => a.ay = 2,
                "left" | "sinistra" | "sx" => a.ax = 0,
                "right" | "destra" | "dx" => a.ax = 2,
                _ => return None,
            }
        }
        any.then_some(a)
    }

    /// Fraction 0.0 / 0.5 / 1.0 per axis.
    pub fn fx(self) -> f32 {
        self.ax as f32 / 2.0
    }
    pub fn fy(self) -> f32 {
        self.ay as f32 / 2.0
    }

    /// Top-left corner of an element `(w, h)` anchored inside the
    /// rectangle `(rx, ry, rw, rh)`, with `pad` px from the anchor's edges.
    pub fn place(self, rect: (u32, u32, u32, u32), size: (u32, u32), pad: (u32, u32)) -> (u32, u32) {
        let (rx, ry, rw, rh) = rect;
        let x = match self.ax {
            0 => rx + pad.0,
            1 => rx + rw.saturating_sub(size.0) / 2,
            _ => rx + rw.saturating_sub(size.0 + pad.0),
        };
        let y = match self.ay {
            0 => ry + pad.1,
            1 => ry + rh.saturating_sub(size.1) / 2,
            _ => ry + rh.saturating_sub(size.1 + pad.1),
        };
        (x, y)
    }
}

/// Elements that can be shown/hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Show {
    pub cpu: bool,
    pub gpu: bool,
    pub uptime: bool,
    pub time: bool,
    pub date: bool,
    pub mem: bool,
    pub net: bool,
    pub disk: bool,
    pub volume: bool,
    pub nowplaying: bool,
    /// Current weather (temperature + condition icon).
    pub weather: bool,
    /// Audio spectrum (EQ bars).
    pub spectrum: bool,
    /// Scrolling ticker line (news headlines or any custom feed, see `ticker.rs`).
    pub ticker: bool,
    /// Large clock in the foreground.
    pub clock: bool,
    /// Date under the large clock.
    pub clock_date: bool,
    /// FPS/GPU/CPU/RAM dashboard while gaming.
    pub dashboard: bool,
    // Details inside the CPU/GPU lines (always on unless `hide`).
    pub cpu_freq: bool,
    pub cpu_temp: bool,
    pub cpu_power: bool,
    pub gpu_temp: bool,
    pub gpu_power: bool,
    pub gpu_fan: bool,
    pub gpu_clock: bool,
    pub gpu_fps: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self::all(true)
    }
}

impl Show {
    pub fn all(v: bool) -> Self {
        Show {
            cpu: v, gpu: v, uptime: v, time: v, date: v, mem: v, net: v, disk: v,
            volume: v, nowplaying: v, weather: v, spectrum: v, ticker: v, clock: v, clock_date: v, dashboard: v,
            cpu_freq: true, cpu_temp: true, cpu_power: true,
            gpu_temp: true, gpu_power: true, gpu_fan: true, gpu_clock: true, gpu_fps: true,
        }
    }

    pub const NAMES: &'static str =
        "cpu, gpu, uptime, time, date, mem, net, disk, volume, nowplaying, weather, spectrum, ticker, clock, clock_date, dashboard, cpu_freq, cpu_temp, cpu_power, gpu_temp, gpu_power, gpu_fan, gpu_clock, gpu_fps";

    fn slot(&mut self, name: &str) -> Option<&mut bool> {
        Some(match name {
            "cpu" => &mut self.cpu,
            "gpu" => &mut self.gpu,
            "uptime" | "attivo" => &mut self.uptime,
            "time" | "ora" | "orario" => &mut self.time,
            "date" | "data" => &mut self.date,
            "mem" | "ram" | "memoria" => &mut self.mem,
            "net" | "rete" | "network" => &mut self.net,
            "disk" | "disco" => &mut self.disk,
            "volume" | "vol" => &mut self.volume,
            "nowplaying" | "now_playing" | "brano" | "musica" | "media" => &mut self.nowplaying,
            "weather" | "meteo" => &mut self.weather,
            "spectrum" | "spettro" | "eq" => &mut self.spectrum,
            "ticker" | "news" | "notizie" => &mut self.ticker,
            "clock" | "orologio" => &mut self.clock,
            "clock_date" | "clockdate" | "data_orologio" => &mut self.clock_date,
            "dashboard" | "game" | "gioco" => &mut self.dashboard,
            "cpu_freq" | "cpu_mhz" => &mut self.cpu_freq,
            "cpu_temp" => &mut self.cpu_temp,
            "cpu_power" | "cpu_watt" => &mut self.cpu_power,
            "gpu_temp" => &mut self.gpu_temp,
            "gpu_power" | "gpu_watt" => &mut self.gpu_power,
            "gpu_fan" => &mut self.gpu_fan,
            "gpu_clock" | "gpu_mhz" => &mut self.gpu_clock,
            "gpu_fps" | "fps" => &mut self.gpu_fps,
            _ => return None,
        })
    }

    /// `list` = comma-separated list. With `only = true` it starts with everything
    /// off and turns on only the ones listed (`show`); with `false` it starts with
    /// everything on and turns off the ones listed (`hide`).
    pub fn apply_list(&mut self, list: &str, on: bool) -> Result<(), String> {
        for raw in list.split(',') {
            let name = raw.trim().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            match self.slot(&name) {
                Some(slot) => *slot = on,
                None => return Err(format!("unknown element '{name}' (valid: {})", Show::NAMES)),
            }
        }
        Ok(())
    }
}

/// How to fit the background to the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Fills the whole canvas while keeping the aspect ratio (crops the rest).
    #[default]
    Cover,
    /// Fills the whole canvas by stretching the image.
    Stretch,
    /// Whole image visible, with dark bars on the sides.
    Contain,
    /// Original size, no scaling.
    Original,
}

impl Fit {
    pub fn parse(s: &str) -> Option<Fit> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cover" | "riempi" | "fill" => Some(Fit::Cover),
            "stretch" | "stira" | "adatta" => Some(Fit::Stretch),
            "contain" | "fit" | "intera" => Some(Fit::Contain),
            "original" | "none" | "originale" | "center" => Some(Fit::Original),
            _ => None,
        }
    }
}

/// Full background layout.
#[derive(Debug, Clone, Copy)]
pub struct BgLayout {
    pub fit: Fit,
    pub anchor: Anchor,
    /// Extra offset in px (positive = right / down).
    pub offset: (i32, i32),
}

impl Default for BgLayout {
    fn default() -> Self {
        BgLayout { fit: Fit::Cover, anchor: Anchor::CENTER, offset: (0, 0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_parse_and_place() {
        assert_eq!(Anchor::parse("top-right"), Some(Anchor { ax: 2, ay: 0 }));
        assert_eq!(Anchor::parse("center-left"), Some(Anchor { ax: 0, ay: 1 }));
        assert_eq!(Anchor::parse("basso destra"), Some(Anchor { ax: 2, ay: 2 }));
        assert_eq!(Anchor::parse("top"), Some(Anchor { ax: 1, ay: 0 }));
        assert_eq!(Anchor::parse("diagonal"), None);
        let a = Anchor::parse("bottom-right").unwrap();
        assert_eq!(a.place((0, 0, 100, 50), (20, 10), (5, 5)), (75, 35));
        assert_eq!(Anchor::CENTER.place((0, 0, 100, 50), (20, 10), (5, 5)), (40, 20));
    }

    #[test]
    fn show_lists() {
        let mut s = Show::default();
        s.apply_list("spectrum, clock", false).unwrap();
        assert!(!s.spectrum && !s.clock && s.cpu);
        let mut s = Show::all(false);
        s.apply_list("cpu,rete", true).unwrap();
        assert!(s.cpu && s.net && !s.gpu);
        assert!(Show::default().apply_list("foo", true).is_err());
    }
}
