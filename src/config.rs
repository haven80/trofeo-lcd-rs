//! Simple `key = value` config file (a TOML subset, no extra
//! dependencies). Blank lines and `# ...` comments are ignored; values may be
//! quoted. Example:
//!
//! ```text
//! # trofeo.conf
//! orientation = portrait   # landscape | portrait
//! flip = false             # true = an extra 180° rotation
//! ```
//!
//! Priority order: defaults < config file < command-line arguments.
//!
//! Search locations (if `--config <FILE>` is not given): `trofeo.conf`
//! in the same folder as the program, then in the current working folder.

use crate::{Orientation, Panel, Resolution};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_FILE_NAME: &str = "trofeo.conf";
/// Accepted file names, in priority order.
pub const FILE_NAMES: [&str; 2] = ["trofeo.conf", "trofeo.config"];

#[derive(Debug, Default, Clone)]
pub struct ConfigFile {
    values: HashMap<String, String>,
    /// The file that was read (None = no file, using defaults).
    pub path: Option<PathBuf>,
}

/// Strip an end-of-line `# comment` from a value, but keep `#RRGGBB` colors:
/// a `#` followed by a hex digit is a color when it STARTS the value or follows
/// a comma (so lists like `spectrum_gradient = #FF0080, #00FFFF` survive);
/// any other `#` starts a comment.
fn strip_value_comment(v: &str) -> String {
    // a quoted value ("Fan #2") is taken whole: a `#` inside the quotes is not a comment
    if let Some(q) = v.chars().next().filter(|c| matches!(c, '"' | '\'')) {
        if let Some(end) = v[1..].find(q) {
            return v[1..1 + end].to_string();
        }
    }
    let mut out = String::new();
    let mut prev_sig: Option<char> = None; // last non-whitespace char kept so far
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '#' {
            let color_ctx = prev_sig.is_none() || prev_sig == Some(',');
            if !(color_ctx && chars.peek().is_some_and(|n| n.is_ascii_hexdigit())) {
                break;
            }
        }
        if !c.is_whitespace() {
            prev_sig = Some(c);
        }
        out.push(c);
    }
    out.trim().to_string()
}

impl ConfigFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut values = HashMap::new();
        // Windows Notepad can save with a UTF-8 BOM at the start.
        let text = text.trim_start_matches('\u{feff}');
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: missing '=' ({raw:?})", n + 1))?;
            let v = strip_value_comment(v.trim());
            let v = v.trim_matches(|c| c == '"' || c == '\'');
            values.insert(k.trim().to_ascii_lowercase(), v.to_string());
        }
        Ok(Self { values, path: None })
    }

    /// Read from `explicit` (error if missing) or search the standard
    /// locations (no file found = empty config, not an error).
    pub fn load(explicit: Option<&Path>) -> Result<Self, String> {
        let candidates: Vec<PathBuf> = match explicit {
            Some(p) => vec![p.to_path_buf()],
            None => {
                let mut v = Vec::new();
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(dir) = exe.parent() {
                        for n in FILE_NAMES {
                            v.push(dir.join(n));
                        }
                    }
                }
                for n in FILE_NAMES {
                    v.push(PathBuf::from(n));
                }
                v
            }
        };
        for path in candidates {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let mut cfg = Self::parse(&text)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    cfg.path = Some(path);
                    return Ok(cfg);
                }
                Err(e) if explicit.is_some() => {
                    return Err(format!("unable to read {}: {e}", path.display()));
                }
                Err(_) => continue,
            }
        }
        Ok(Self::default())
    }

    /// Path of the file `load` would read right now (for hot reload).
    pub fn find_path(explicit: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = explicit {
            return Some(p.to_path_buf());
        }
        let mut v = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                for n in FILE_NAMES {
                    v.push(dir.join(n));
                }
            }
        }
        for n in FILE_NAMES {
            v.push(PathBuf::from(n));
        }
        v.into_iter().find(|p| p.is_file())
    }

    /// Set/overwrite a key (used for command-line arguments).
    pub fn set(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_ascii_lowercase(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn get_bool(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key).map(|v| v.to_ascii_lowercase()) {
            None => Ok(None),
            Some(v) if matches!(v.as_str(), "true" | "1" | "yes" | "on") => Ok(Some(true)),
            Some(v) if matches!(v.as_str(), "false" | "0" | "no" | "off") => Ok(Some(false)),
            Some(v) => Err(format!("{key}: '{v}' is not true/false")),
        }
    }

    /// Size of the panel's picture: `panel_size = 1920x400` wins over a model name
    /// (`panel = trofeo-11.3`); neither = the Trofeo Vision 9.16 (1920x462).
    pub fn panel_resolution(&self) -> Result<Resolution, String> {
        if self.panel_is_auto() {
            if let Some(r) = crate::detected_resolution() {
                return Ok(r);
            }
            return Ok(crate::detected_panel().unwrap_or_default().resolution());
        }
        panel_from(self.get("panel"), self.get("panel_size"))
    }

    /// Neither `panel` nor `panel_size` is set: the model found at the handshake decides.
    pub fn panel_is_auto(&self) -> bool {
        self.get("panel").is_none() && self.get("panel_size").is_none()
    }

    pub fn orientation(&self) -> Result<Option<Orientation>, String> {
        match self.get("orientation") {
            None => Ok(None),
            Some(v) => Orientation::parse(v)
                .map(Some)
                .ok_or_else(|| format!("orientation: '{v}' is not valid (landscape | portrait)")),
        }
    }
}

/// Panel size from a model name and/or an explicit size (the size wins).
pub fn panel_from(panel: Option<&str>, size: Option<&str>) -> Result<Resolution, String> {
    let model = match panel {
        None => Panel::default(),
        Some(v) => Panel::parse(v).ok_or_else(|| format!("panel: '{v}' is not valid ({})", Panel::NAMES))?,
    };
    match size {
        None => Ok(model.resolution()),
        Some(v) => crate::parse_resolution(v)
            .ok_or_else(|| format!("panel_size: '{v}' is not valid (WIDTHxHEIGHT, e.g. 1920x400)")),
    }
}

/// Margins (px, on the logical canvas) to keep the image away from the screen edges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Margins {
    pub top: u32,
    pub bottom: u32,
    pub left: u32,
    pub right: u32,
}

impl Margins {
    pub fn is_zero(&self) -> bool {
        *self == Margins::default()
    }

    /// Inner dimensions after the margins; error if too little space is left.
    pub fn inner(&self, w: u32, h: u32) -> Result<(u32, u32), String> {
        let iw = w.saturating_sub(self.left + self.right);
        let ih = h.saturating_sub(self.top + self.bottom);
        if iw < 100 || ih < 100 {
            return Err(format!(
                "margins too large: only {iw}x{ih} px left on a {w}x{h} canvas"
            ));
        }
        Ok((iw, ih))
    }
}

impl ConfigFile {
    pub fn get_f32(&self, key: &str) -> Result<Option<f32>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .parse::<f32>()
                .map(Some)
                .map_err(|_| format!("{key}: '{v}' is not a number")),
        }
    }    
    pub fn get_u32(&self, key: &str) -> Result<Option<u32>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .parse::<u32>()
                .map(Some)
                .map_err(|_| format!("{key}: '{v}' is not an integer >= 0")),
        }
    }

    /// `margin` (all sides) then `margin_top/bottom/left/right` per side.
    pub fn margins(&self) -> Result<Margins, String> {
        let all = self.get_u32("margin")?.unwrap_or(0);
        Ok(Margins {
            top: self.get_u32("margin_top")?.unwrap_or(all),
            bottom: self.get_u32("margin_bottom")?.unwrap_or(all),
            left: self.get_u32("margin_left")?.unwrap_or(all),
            right: self.get_u32("margin_right")?.unwrap_or(all),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_comes_from_the_model_or_an_explicit_size() {
        let r = |t: &str| ConfigFile::parse(t).unwrap().panel_resolution();
        assert_eq!(r("").unwrap(), Resolution::new(1920, 462));
        assert_eq!(r("panel = trofeo-11.3\n").unwrap(), Resolution::new(1920, 400));
        assert_eq!(r("panel = trofeo-9.16\n").unwrap(), Resolution::new(1920, 462));
        // An explicit size wins over the model.
        assert_eq!(r("panel = trofeo-11.3\npanel_size = 1920x440\n").unwrap(), Resolution::new(1920, 440));
        assert_eq!(r("panel_size = 1280x480 # other\n").unwrap(), Resolution::new(1280, 480));
        assert!(r("panel = nope\n").is_err());
        assert!(r("panel_size = big\n").is_err());
        assert_eq!(panel_from(Some("11.3"), None).unwrap(), Resolution::new(1920, 400));
        assert_eq!(panel_from(None, Some("1920x400")).unwrap(), Resolution::new(1920, 400));
    }

    #[test]
    fn hex_colors_are_not_comments() {
        let c = ConfigFile::parse("cpu_color = #FFC800\ngpu_color = #00FF00  # verde\nx = 5 # c\n# y = 1\n").unwrap();
        assert_eq!(c.get("cpu_color"), Some("#FFC800"));
        assert_eq!(c.get("gpu_color"), Some("#00FF00"));
        assert_eq!(c.get("x"), Some("5"));
        assert_eq!(c.get("y"), None);
    }

    #[test]
    fn hex_color_lists_survive_and_comments_still_strip() {
        let c = ConfigFile::parse(
            "g = #FF0080, #00FFFF\nh = #FF0080,#00FFFF,#112233   # three colors\nn = red, #00FFFF # tail\nk = #FF0080, # not a color\nq = 5#tail\n",
        )
        .unwrap();
        assert_eq!(c.get("g"), Some("#FF0080, #00FFFF"));
        assert_eq!(c.get("h"), Some("#FF0080,#00FFFF,#112233"));
        assert_eq!(c.get("n"), Some("red, #00FFFF"));
        assert_eq!(c.get("k"), Some("#FF0080,"));
        assert_eq!(c.get("q"), Some("5"));
    }

    #[test]
    fn parses_values_comments_and_quotes() {
        let c = ConfigFile::parse("# hi\norientation = \"Portrait\" # x\n\nflip=true\n").unwrap();
        assert_eq!(c.orientation().unwrap(), Some(Orientation::Portrait));
        assert_eq!(c.get_bool("flip").unwrap(), Some(true));
        assert_eq!(c.get_bool("missing").unwrap(), None);
    }

    #[test]
    fn tolerates_utf8_bom_and_crlf() {
        let c = ConfigFile::parse("\u{feff}# c\r\norientation = portrait\r\n").unwrap();
        assert_eq!(c.orientation().unwrap(), Some(Orientation::Portrait));
    }

    #[test]
    fn margins_all_and_per_side() {
        let c = ConfigFile::parse("margin = 10\nmargin_top = 40").unwrap();
        let m = c.margins().unwrap();
        assert_eq!((m.top, m.bottom, m.left, m.right), (40, 10, 10, 10));
        assert!(m.inner(1920, 462).is_ok());
        assert!(Margins { top: 300, bottom: 100, ..Default::default() }.inner(1920, 462).is_err());
    }

    #[test]
    fn rejects_bad_values() {
        assert!(ConfigFile::parse("orientation").is_err());
        assert!(ConfigFile::parse("orientation = diagonal").unwrap().orientation().is_err());
    }
}
