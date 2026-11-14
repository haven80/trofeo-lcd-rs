//! "Share your design": the settings editor (`trofeo_config`) asks the running program for a
//! screenshot of what is on the screen, shows it, copies the settings that make up the look
//! (and nothing personal) and opens a prefilled page of the project on GitHub, where the user
//! drops the picture. No account data or token is ever stored in the program.
//!
//! The editor and the program are two processes, so they talk through a small file next to
//! the configuration file: the editor writes `trofeo_share.request` with the path of the PNG
//! it wants; the running program sees it within a second, saves the frame it is showing there
//! and deletes the request.

use std::io;
use std::path::{Path, PathBuf};

use crate::conf_edit::ConfDoc;
use crate::schema::{Field, Kind};

/// The request file, in the folder of the configuration file.
pub const REQUEST_FILE: &str = "trofeo_share.request";

/// The page where designs are posted (an issue form with the label `gallery`).
pub const GALLERY_NEW: &str = "https://github.com/haven80/trofeo-lcd-rs/issues/new?template=gallery.yml";
/// The gallery: the issues with the label `gallery`.
pub const GALLERY_LIST: &str = "https://github.com/haven80/trofeo-lcd-rs/issues?q=is%3Aissue+label%3Agallery";

pub fn request_path(dir: &Path) -> PathBuf {
    dir.join(REQUEST_FILE)
}

/// Folder where the editor keeps the pictures to share.
pub fn gallery_dir(conf_dir: &Path) -> PathBuf {
    conf_dir.join("gallery")
}

/// A file name for a new picture.
pub fn png_name() -> String {
    format!("design_{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S"))
}

/// Editor side: asks the running program to save its current frame as `png`.
pub fn write_request(dir: &Path, png: &Path) -> io::Result<()> {
    if let Some(p) = png.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(request_path(dir), png.to_string_lossy().as_bytes())
}

/// Program side: a request waiting in `dir`? Returns the PNG to write and removes the request.
/// Anything that is not a `.png` path is ignored.
pub fn take_request(dir: &Path) -> Option<PathBuf> {
    let file = request_path(dir);
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    let path = PathBuf::from(text.trim());
    let png = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
    (png && path.parent().is_some_and(|p| p.is_dir())).then_some(path)
}

/// Sections whose settings are part of how the screen looks.
const LOOK_SECTIONS: &[&str] = &[
    "background", "effect", "layouts", "gaming", "steam", "tiles", "dash", "frame", "show", "look", "spectrum", "items", "music", "game",
];

/// Is this setting part of the look, and safe to publish? Files and folders (they carry the
/// name of the user's account), the sensors, the weather city and the system settings are not.
pub fn shareable(f: &Field) -> bool {
    if matches!(f.kind, Kind::Path | Kind::Folder) {
        return false;
    }
    if f.key.contains("path") || f.key.contains("folder") || f.key.contains("city") || f.key.contains("url") {
        return false;
    }
    LOOK_SECTIONS.contains(&f.section) || f.key == "orientation"
}

/// The settings of the look that the user has set, as `key = value` lines.
pub fn settings_text(doc: &ConfDoc, fields: &[Field]) -> String {
    let mut out = String::new();
    for f in fields.iter().filter(|f| shareable(f) && doc.is_set(&f.key)) {
        if let Some(v) = doc.get(&f.key) {
            out.push_str(&format!("{} = {}\n", f.key, v));
        }
    }
    out
}

/// What an imported design would do.
#[derive(Debug, Default, PartialEq)]
pub struct Import {
    /// `(key, value)` to set.
    pub apply: Vec<(String, String)>,
    /// `(key, why)` left out.
    pub skipped: Vec<(String, &'static str)>,
}

/// Reads a design pasted as `key = value` lines (as "Copy the settings" writes them, also with
/// the lines of a code block around them) and sorts it: what is part of the look and has a
/// valid value is applied; the rest is left out with the reason.
pub fn parse_import(text: &str, fields: &[Field]) -> Import {
    let doc = ConfDoc::parse(text);
    let mut out = Import::default();
    for key in doc.keys() {
        let Some(value) = doc.get(&key) else { continue };
        match fields.iter().find(|f| f.key == key) {
            None => out.skipped.push((key, "not a setting of this version")),
            Some(f) if !shareable(f) => out.skipped.push((key, "not part of the look")),
            Some(f) if !crate::schema::check_value(f, &value) => out.skipped.push((key, "value not valid")),
            Some(_) => out.apply.push((key, value)),
        }
    }
    out
}

/// The prefilled page for a new gallery entry.
pub fn issue_url(version: &str) -> String {
    format!("{GALLERY_NEW}&title=%5BGallery%5D%20&version={version}")
}

/// Opens a web address or a folder with what the system uses for it.
pub fn open(target: &str) {
    use std::process::Command;
    let mut cmd;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if target.starts_with("http") {
            cmd = Command::new("rundll32");
            cmd.arg("url.dll,FileProtocolHandler").arg(target);
        } else {
            cmd = Command::new("explorer");
            cmd.arg(target);
        }
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(target_os = "macos")]
    {
        cmd = Command::new("open");
        cmd.arg(target);
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        cmd = Command::new("xdg-open");
        cmd.arg(target);
    }
    let _ = cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::fields;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("trofeo_share_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_request_is_written_taken_and_removed() {
        let d = tmp("req");
        let png = gallery_dir(&d).join("a.png");
        write_request(&d, &png).unwrap();
        assert!(request_path(&d).exists());
        assert_eq!(take_request(&d), Some(png));
        assert!(!request_path(&d).exists());
        assert_eq!(take_request(&d), None);
    }

    #[test]
    fn only_png_files_in_an_existing_folder_are_accepted() {
        let d = tmp("bad");
        std::fs::write(request_path(&d), d.join("x.exe").to_string_lossy().as_bytes()).unwrap();
        assert_eq!(take_request(&d), None);
        assert!(!request_path(&d).exists(), "an unusable request is removed too");
        std::fs::write(request_path(&d), d.join("nodir").join("x.png").to_string_lossy().as_bytes()).unwrap();
        assert_eq!(take_request(&d), None);
    }

    #[test]
    fn nothing_personal_is_shared() {
        let all = fields();
        for f in all.iter().filter(|f| shareable(f)) {
            assert!(!matches!(f.kind, Kind::Path | Kind::Folder), "{}", f.key);
        }
        for k in ["background", "frame_folder", "steam_path", "weather_city", "ticker_source", "lhm_url", "update_check", "hid_mount", "gaming_bg_image", "tiles_bg_image"] {
            if let Some(f) = all.iter().find(|f| f.key == k) {
                assert!(!shareable(f), "{k} must not be shared");
            }
        }
        for k in ["layout", "clock_format", "tiles", "music_layout", "background_effect"] {
            let f = all.iter().find(|f| f.key == k).unwrap_or_else(|| panic!("{k}"));
            assert!(shareable(f), "{k} is part of the look");
        }
    }

    #[test]
    fn the_text_has_only_the_look_settings_that_were_set() {
        let doc = ConfDoc::parse("weather_city = Rome\nbackground = C:\\Users\\me\\a.jpg\nclock_format = 12h\nlayout = deck, cards\nupdate_check = false\n");
        let t = settings_text(&doc, &fields());
        assert!(t.contains("clock_format = 12h") && t.contains("layout = deck, cards"), "{t}");
        assert!(!t.contains("Rome") && !t.contains("Users") && !t.contains("update_check"), "{t}");
    }

    #[test]
    fn the_page_is_the_gallery_form() {
        let u = issue_url("2.17.0");
        assert!(u.starts_with("https://github.com/haven80/trofeo-lcd-rs/issues/new?template=gallery.yml"));
        assert!(u.contains("version=2.17.0"));
        assert!(!u.contains(' '));
    }

    #[test]
    fn a_pasted_design_is_sorted_into_what_applies_and_what_does_not() {
        let text = "```ini\nclock_format = 12h\nlayout = deck, cards\nweather_city = Rome\nbackground = C:\\a.jpg\nclock_ampm = maybe\nnot_a_key = 1\n```\n";
        let imp = parse_import(text, &fields());
        let keys: Vec<&str> = imp.apply.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["clock_format", "layout"], "{imp:?}");
        let why = |k: &str| imp.skipped.iter().find(|(s, _)| s == k).map(|(_, w)| *w);
        assert_eq!(why("weather_city"), Some("not part of the look"));
        assert_eq!(why("background"), Some("not part of the look"));
        assert_eq!(why("clock_ampm"), Some("value not valid"));
        assert_eq!(why("not_a_key"), Some("not a setting of this version"));
        assert_eq!(parse_import("", &fields()), Import::default());
    }
}
