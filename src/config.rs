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

use crate::Orientation;
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
            // End-of-line comment: `#` ... ; but a value that STARTS with `#` is a color (#RRGGBB).
            let v = v.trim();
            let v = if let Some(rest) = v.strip_prefix('#') {
                let tok = rest.split_whitespace().next().unwrap_or("");
                format!("#{tok}")
            } else {
                v.split('#').next().unwrap_or("").trim().to_string()
            };
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

    pub fn orientation(&self) -> Result<Option<Orientation>, String> {
        match self.get("orientation") {
            None => Ok(None),
            Some(v) => Orientation::parse(v)
                .map(Some)
                .ok_or_else(|| format!("orientation: '{v}' is not valid (landscape | portrait)")),
        }
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
    fn hex_colors_are_not_comments() {
        let c = ConfigFile::parse("cpu_color = #FFC800\ngpu_color = #00FF00  # verde\nx = 5 # c\n# y = 1\n").unwrap();
        assert_eq!(c.get("cpu_color"), Some("#FFC800"));
        assert_eq!(c.get("gpu_color"), Some("#00FF00"));
        assert_eq!(c.get("x"), Some("5"));
        assert_eq!(c.get("y"), None);
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
