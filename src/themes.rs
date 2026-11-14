//! Themes: `theme = ocean` gives a whole look (background, colours, panel opacity) in one
//! line. A theme only supplies **defaults**: every setting written in the configuration file
//! (or on the command line) wins over it, so a theme is a starting point to adjust.

/// One theme: a name, a label and the settings it provides.
pub struct Theme {
    pub name: &'static str,
    pub en: &'static str,
    pub it: &'static str,
    /// `(key, value)`; only settings of the look (checked by the tests).
    pub settings: &'static [(&'static str, &'static str)],
}

pub const THEMES: &[Theme] = &[
    Theme {
        name: "midnight",
        en: "Midnight: deep blue and violet light",
        it: "Mezzanotte: luce blu profondo e viola",
        settings: &[
            ("background_effect", "aurora"),
            ("effect_palette", "midnight"),
            ("text_color", "#C9D4FF"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#3A5BFF, #8B5CF6"),
            ("dash_colors", "#6C8CFF, #9B6BFF, #4FD1FF"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "ocean",
        en: "Ocean: waves in blue and teal",
        it: "Oceano: onde blu e verde acqua",
        settings: &[
            ("background_effect", "waves"),
            ("effect_palette", "ocean"),
            ("text_color", "#D6F4FF"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#0EA5E9, #2DD4BF"),
            ("dash_colors", "#38BDF8, #2DD4BF, #818CF8"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "sunset",
        en: "Sunset: orange, pink and violet",
        it: "Tramonto: arancio, rosa e viola",
        settings: &[
            ("background_effect", "plasma"),
            ("effect_palette", "sunset"),
            ("text_color", "#FFE7D6"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#FF7A18, #FF3CAC"),
            ("dash_colors", "#FF9F43, #FF6FB1, #A76BFF"),
            ("panel_opacity", "50"),
        ],
    },
    Theme {
        name: "forest",
        en: "Forest: soft greens",
        it: "Foresta: verdi morbidi",
        settings: &[
            ("background_effect", "aurora"),
            ("effect_palette", "forest"),
            ("text_color", "#DFF7E3"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#34D399, #A3E635"),
            ("dash_colors", "#4ADE80, #A3E635, #2DD4BF"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "ember",
        en: "Ember: a lava lamp in red and amber",
        it: "Brace: una lava lamp rossa e ambra",
        settings: &[
            ("background_effect", "lava"),
            ("effect_palette", "ember"),
            ("text_color", "#FFE9CC"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#F59E0B, #EF4444"),
            ("dash_colors", "#FBBF24, #F97316, #EF4444"),
            ("panel_opacity", "50"),
        ],
    },
    Theme {
        name: "neon",
        en: "Neon: a network of magenta and cyan",
        it: "Neon: una rete di magenta e azzurro",
        settings: &[
            ("background_effect", "network"),
            ("effect_palette", "neon"),
            ("text_color", "#F2E9FF"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#FF2BD6, #22D3EE"),
            ("dash_colors", "#FF2BD6, #22D3EE, #A78BFA"),
            ("panel_opacity", "55"),
        ],
    },
    Theme {
        name: "matrix",
        en: "Matrix: green digital rain",
        it: "Matrix: pioggia digitale verde",
        settings: &[
            ("background_effect", "rain"),
            ("effect_palette", "matrix"),
            ("text_color", "#9BFFB0"),
            ("clock_color", "#D8FFE0"),
            ("spectrum_gradient", "#16A34A, #86EFAC"),
            ("dash_colors", "#22C55E, #86EFAC, #4ADE80"),
            ("panel_opacity", "60"),
        ],
    },
    Theme {
        name: "mono",
        en: "Mono: black and white, no animation",
        it: "Mono: bianco e nero, senza animazioni",
        settings: &[
            ("background_color", "#0B0B0D"),
            ("text_color", "#D0D0D4"),
            ("clock_color", "#FFFFFF"),
            ("spectrum_gradient", "#8A8A92, #F2F2F5"),
            ("dash_colors", "#E5E5EA, #A1A1AA, #71717A"),
            ("panel_opacity", "40"),
        ],
    },
];

/// The names a `theme` can have, for the messages and the editor.
pub fn names() -> Vec<&'static str> {
    THEMES.iter().map(|t| t.name).collect()
}

pub fn find(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.name.eq_ignore_ascii_case(name.trim()))
}

// ---------------------------------------------------------------------------------------------
// Your own themes: `themes/<name>.theme` next to the configuration file. A theme file is plain
// `key = value` lines of the look (as "Copy the settings" writes them), so it can be pasted in a
// message and saved by someone else under any name.
// ---------------------------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::conf_edit::ConfDoc;
use crate::schema::{check_value, fields, Field};

/// Extension of a theme file.
pub const EXT: &str = "theme";
/// Name of the folder (next to the configuration file).
pub const FOLDER: &str = "themes";
/// Environment variable that points to another themes folder (used by the editor's preview,
/// which runs the program on a temporary configuration file).
pub const ENV_DIR: &str = "TROFEO_THEMES_DIR";

static USER_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The themes folder of a configuration file.
pub fn folder_of(config: &Path) -> Option<PathBuf> {
    config.parent().map(|p| p.join(FOLDER))
}

/// Remember where the themes of the configuration file in use are.
pub fn set_user_dir(dir: Option<PathBuf>) {
    *USER_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// GitHub does not take `.theme` files as attachments, so the file to attach to a post is a
/// text file: `name.theme.txt`.
pub fn export_file_name(name: &str) -> String {
    format!("{}.{EXT}.txt", name.trim())
}

/// A theme name made from a file name (`Cool Look.theme.txt` -> `cool-look`), or None when
/// nothing usable is left or it is the name of a built-in theme.
pub fn name_from_file(file_name: &str) -> Option<String> {
    let lower = file_name.trim().to_ascii_lowercase();
    let base = lower.strip_suffix(".txt").unwrap_or(&lower);
    let base = base.strip_suffix(&format!(".{EXT}")).unwrap_or(base);
    let base = base.strip_suffix(".txt").unwrap_or(base);
    let mut name: String = base
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' { c } else { '-' })
        .collect();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    let mut name = name.trim_matches('-').to_string();
    name.truncate(40);
    let name = name.trim_matches('-').to_string();
    match valid_name(&name) {
        Ok(()) => Some(name),
        Err(_) => (!name.is_empty() && name.len() <= 36 && valid_name(&format!("{name}-2")).is_ok()).then(|| format!("{name}-2")),
    }
}

/// The folder with the user's themes, if one is known.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(ENV_DIR).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    USER_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A name for a theme of your own: lowercase letters, digits, `-` and `_`, up to 40, and not
/// the name of a built-in theme.
pub fn valid_name(name: &str) -> Result<(), &'static str> {
    let n = name.trim();
    if n.is_empty() || n.len() > 40 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err("use lowercase letters, digits, - and _ (up to 40)");
    }
    if n == "none" || find(n).is_some() {
        return Err("that is the name of a built-in theme");
    }
    Ok(())
}

pub fn path_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{EXT}", name.trim()))
}

/// The themes of your own that are in `dir`, sorted.
pub fn list_user(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some(EXT)).then(|| p.file_stem()?.to_str().map(str::to_string)).flatten()
        })
        .filter(|n| valid_name(n).is_ok())
        .collect();
    out.sort();
    out
}

/// Is this a setting a theme (of your own) may carry? The look, with a valid value, and not
/// `theme` itself.
fn check_setting(all: &[Field], key: &str, value: &str) -> Result<(), String> {
    let f = all.iter().find(|f| f.key == key).ok_or_else(|| format!("'{key}' is not a setting of this version"))?;
    if key == "theme" || !crate::share::shareable(f) {
        return Err(format!("'{key}' is not part of the look"));
    }
    if !check_value(f, value) {
        return Err(format!("{key}: '{value}' is not valid"));
    }
    Ok(())
}

/// The settings of the theme of your own `name` in `dir`.
pub fn load_user(dir: &Path, name: &str) -> Result<Vec<(String, String)>, String> {
    valid_name(name).map_err(|e| format!("theme '{name}': {e}"))?;
    let path = path_of(dir, name);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("theme '{name}': cannot read {}: {e}", path.display()))?;
    let doc = ConfDoc::parse(text.trim_start_matches('\u{feff}'));
    let all = fields();
    let mut out = Vec::new();
    for k in doc.keys() {
        let v = doc.get(&k).unwrap_or_default();
        check_setting(&all, &k, &v).map_err(|e| format!("{}: {e}", path.display()))?;
        out.push((k, v));
    }
    if out.is_empty() {
        return Err(format!("{}: no settings in it", path.display()));
    }
    Ok(out)
}

/// The text of a theme file: a header and the settings.
pub fn render(name: &str, settings: &[(String, String)]) -> String {
    let mut s = format!("# trofeo_lcd theme: {name}\n# Save as {FOLDER}/{name}.{EXT} next to your configuration file, then set  theme = {name}\n");
    for (k, v) in settings {
        s.push_str(&format!("{k} = {v}\n"));
    }
    s
}

/// Save a theme of your own in `dir` (created when needed). Only settings of the look are
/// accepted (the callers filter first; this checks again). Returns where it was written.
pub fn save_user(dir: &Path, name: &str, settings: &[(String, String)]) -> Result<PathBuf, String> {
    valid_name(name).map_err(|e| format!("'{}': {e}", name.trim()))?;
    let all = fields();
    for (k, v) in settings {
        check_setting(&all, k, v)?;
    }
    if settings.is_empty() {
        return Err("there is nothing to save: every setting of the look is at its default".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = path_of(dir, name);
    std::fs::write(&path, render(name.trim(), settings)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// The settings a theme (built-in, or of your own in `dir`) gives, or why it cannot be used.
pub fn settings_of(name: &str, dir: Option<&Path>) -> Result<Vec<(String, String)>, String> {
    let name = name.trim().to_ascii_lowercase();
    if let Some(t) = find(&name) {
        return Ok(t.settings.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
    }
    if valid_name(&name).is_ok() {
        if let Some(d) = dir {
            if path_of(d, &name).is_file() {
                return load_user(d, &name);
            }
        }
    }
    let mut all: Vec<String> = names().iter().map(|s| s.to_string()).collect();
    if let Some(d) = dir {
        all.extend(list_user(d));
    }
    Err(format!("theme: '{name}' not found (none | {})", all.join(" | ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("trofeo_themes_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn every_theme_sets_only_known_valid_settings_of_the_look() {
        let all = fields();
        for t in THEMES {
            assert!(!t.settings.is_empty() && !t.en.is_empty() && !t.it.is_empty(), "{}", t.name);
            for (k, v) in t.settings {
                let f = all.iter().find(|f| f.key == *k).unwrap_or_else(|| panic!("{}: unknown setting {k}", t.name));
                assert!(crate::share::shareable(f), "{}: {k} is not part of the look", t.name);
                assert!(check_value(f, v), "{}: {k} = {v} is not valid", t.name);
            }
            let mut keys: Vec<&str> = t.settings.iter().map(|(k, _)| *k).collect();
            keys.sort();
            let n = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), n, "{}: a setting twice", t.name);
        }
    }

    #[test]
    fn the_editor_offers_exactly_the_themes_there_are() {
        let mut offered: Vec<&str> = crate::schema::THEME_NAMES.iter().copied().filter(|n| *n != "none").collect();
        let mut real = names();
        offered.sort();
        real.sort();
        assert_eq!(offered, real);
    }

    #[test]
    fn a_theme_gives_defaults_and_the_file_wins() {
        use crate::config::ConfigFile;
        let f = ConfigFile::parse("theme = ocean\nclock_color = #FF0000\n").unwrap();
        assert_eq!(f.get("clock_color"), Some("#FF0000"), "the file wins");
        assert_eq!(f.get("background_effect"), Some("waves"), "the theme fills in the rest");
        let none = ConfigFile::parse("theme = none\n").unwrap();
        assert_eq!(none.get("background_effect"), None);
        assert!(ConfigFile::parse("theme = nope\n").is_err());
    }

    #[test]
    fn names_are_unique_and_found_in_any_case() {
        let mut n = names();
        n.sort();
        let len = n.len();
        n.dedup();
        assert_eq!(n.len(), len);
        assert_eq!(find(" Ocean ").map(|t| t.name), Some("ocean"));
        assert!(find("nope").is_none());
    }

    #[test]
    fn names_of_your_own_themes_are_checked() {
        assert!(valid_name("my-night_2").is_ok());
        for bad in ["", "My Theme", "a/b", "..", "ocean", "none", "Ocean", &"x".repeat(41)] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_theme_of_your_own_can_be_saved_listed_loaded_and_exported() {
        let dir = tmp("roundtrip");
        let s = pairs(&[("clock_color", "#FFD000"), ("panel_opacity", "30")]);
        let p = save_user(&dir, "my-gold", &s).unwrap();
        assert!(p.is_file());
        assert_eq!(list_user(&dir), vec!["my-gold".to_string()]);
        assert_eq!(load_user(&dir, "my-gold").unwrap(), s);
        // the exported text is what another person pastes and saves under any name
        let text = render("my-gold", &s);
        assert!(text.contains("theme = my-gold") && text.contains("clock_color = #FFD000"));
        assert_eq!(save_user(&dir, "copy", &pairs(&[("clock_color", "#FFD000")])).is_ok(), true);
        std::fs::write(path_of(&dir, "pasted"), text).unwrap();
        assert_eq!(load_user(&dir, "pasted").unwrap(), s, "the header is only comments");
        assert_eq!(list_user(&dir).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_look_is_saved_or_loaded() {
        let dir = tmp("look");
        for bad in [("background", "C:\\a.jpg"), ("weather_city", "Rome"), ("theme", "ocean"), ("not_a_key", "1"), ("clock_ampm", "maybe")] {
            assert!(save_user(&dir, "x", &pairs(&[bad])).is_err(), "{bad:?}");
        }
        assert!(save_user(&dir, "x", &[]).is_err(), "nothing to save");
        assert!(save_user(&dir, "ocean", &pairs(&[("clock_color", "#FFF")])).is_err(), "a built-in name");
        assert!(!dir.exists(), "nothing was written");
        // a file written by hand (or downloaded) is checked the same way
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path_of(&dir, "evil"), "clock_color = #FFFFFF\nbackground = C:\\Users\\me\\a.jpg\n").unwrap();
        assert!(load_user(&dir, "evil").unwrap_err().contains("not part of the look"));
        std::fs::write(path_of(&dir, "empty"), "# nothing\n").unwrap();
        assert!(load_user(&dir, "empty").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_of_finds_built_in_and_own_themes_and_explains_a_miss() {
        let dir = tmp("find");
        save_user(&dir, "mine", &pairs(&[("clock_color", "#00FF00")])).unwrap();
        assert!(settings_of("Ocean", None).unwrap().iter().any(|(k, _)| k == "background_effect"));
        assert_eq!(settings_of("mine", Some(&dir)).unwrap(), pairs(&[("clock_color", "#00FF00")]));
        let e = settings_of("nope", Some(&dir)).unwrap_err();
        assert!(e.contains("mine") && e.contains("ocean"), "{e}");
        assert!(settings_of("mine", None).is_err(), "no folder known");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_export_is_a_text_file_and_comes_back_under_a_name_from_the_file() {
        assert_eq!(export_file_name(" my-gold "), "my-gold.theme.txt");
        assert_eq!(name_from_file("my-gold.theme.txt").as_deref(), Some("my-gold"));
        assert_eq!(name_from_file("Cool Look (2).theme.txt").as_deref(), Some("cool-look-2"));
        assert_eq!(name_from_file("night.theme").as_deref(), Some("night"));
        assert_eq!(name_from_file("ocean.theme.txt").as_deref(), Some("ocean-2"), "a built-in name gets a suffix");
        assert_eq!(name_from_file("...txt"), None);
        assert_eq!(name_from_file("###.theme.txt"), None);
    }

    #[test]
    fn theme_equals_name_uses_the_file_and_the_config_still_wins() {
        use crate::config::ConfigFile;
        let dir = tmp("config");
        save_user(&dir, "mine", &pairs(&[("clock_color", "#00FF00"), ("panel_opacity", "20")])).unwrap();
        let mut f = ConfigFile::parse("theme = mine\npanel_opacity = 70\n").unwrap_or_else(|_| ConfigFile::parse("").unwrap());
        // (parse alone knows no folder: it is the explicit call that finds the file)
        let mut g = ConfigFile::parse("panel_opacity = 70\n").unwrap();
        g.set("theme", "mine");
        g.apply_theme_in(Some(&dir)).unwrap();
        assert_eq!(g.get("clock_color"), Some("#00FF00"));
        assert_eq!(g.get("panel_opacity"), Some("70"), "the file wins");
        f.set("theme", "ghost");
        assert!(f.apply_theme_in(Some(&dir)).unwrap_err().contains("mine"), "the message lists the themes of your own");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
