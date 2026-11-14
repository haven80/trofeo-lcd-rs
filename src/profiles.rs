//! Profiles (2.21.0): named sets of settings that are laid over the configuration file while
//! they are active, and that can be switched at any moment without touching the file.
//!
//! - A profile is the file `profiles/<name>.profile` next to the configuration file: plain
//!   `key = value` lines. What it says **wins over the configuration file** (and over the theme
//!   defaults); the command line still wins over everything.
//! - Which profile is active, in this order: the one a **rule** picks for the program in the
//!   foreground (`profile_rules = mygame.exe=gaming`), else the one chosen by hand (the tray
//!   menu, the hotkey `profile_key`, the settings editor: written in `profiles/active`), else
//!   `profile = name` of the configuration file, else none.
//! - Only settings that apply live can be in a profile (not the ones marked "needs a restart"),
//!   so that switching never needs one.
//!
//! Nothing here runs unless a `profiles` folder with profiles exists or `profile*` keys are set.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::conf_edit::ConfDoc;
use crate::schema::{check_value, fields};

pub const FOLDER: &str = "profiles";
pub const EXT: &str = "profile";
/// The file (in the folder) that remembers the profile chosen by hand: a name, or `none`.
pub const ACTIVE_FILE: &str = "active";
/// After leaving the program a rule is about, the profile goes back only after this long
/// (so a quick Alt-Tab does not make the screen flip).
pub const RULE_HOLD: Duration = Duration::from_secs(5);

static AUTO: Mutex<Option<String>> = Mutex::new(None);

pub fn folder_of(config: &Path) -> Option<PathBuf> {
    config.parent().map(|p| p.join(FOLDER))
}

/// The profile a rule picked for the program in the foreground (set by the main loop).
pub fn set_auto(name: Option<String>) {
    *AUTO.lock().unwrap_or_else(|e| e.into_inner()) = name;
}

pub fn auto() -> Option<String> {
    AUTO.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A name: lowercase letters, digits, `-` and `_`, up to 40, and not `none`.
pub fn valid_name(name: &str) -> Result<(), &'static str> {
    let n = name.trim();
    if n.is_empty() || n.len() > 40 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err("use lowercase letters, digits, - and _ (up to 40)");
    }
    if n == "none" {
        return Err("'none' means no profile");
    }
    Ok(())
}

pub fn path_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{EXT}", name.trim()))
}

/// The profiles in `dir`, sorted.
pub fn list(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some(EXT) {
                return None;
            }
            p.file_stem()?.to_str().map(str::to_string)
        })
        .filter(|n| valid_name(n).is_ok())
        .collect();
    out.sort();
    out
}

/// A setting a profile may carry: a known one that applies live, with a valid value (and not
/// one of the `profile*` keys, which say which profile there is).
fn check_setting(all: &[crate::schema::Field], key: &str, value: &str) -> Result<(), String> {
    let f = all.iter().find(|f| f.key == key).ok_or_else(|| format!("'{key}' is not a setting of this version"))?;
    if key.starts_with("profile") {
        return Err(format!("'{key}' cannot be in a profile"));
    }
    if f.restart {
        return Err(format!("'{key}' needs a restart of the program, so it cannot be switched with a profile"));
    }
    if !check_value(f, value) {
        return Err(format!("{key}: '{value}' is not valid"));
    }
    Ok(())
}

/// The settings of the profile `name` in `dir`.
pub fn load(dir: &Path, name: &str) -> Result<Vec<(String, String)>, String> {
    valid_name(name).map_err(|e| format!("profile '{name}': {e}"))?;
    let path = path_of(dir, name);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("profile '{name}': cannot read {}: {e}", path.display()))?;
    let doc = ConfDoc::parse(text.trim_start_matches('\u{feff}'));
    let all = fields();
    let mut out = Vec::new();
    for k in doc.keys() {
        let v = doc.get(&k).unwrap_or_default();
        check_setting(&all, &k, &v).map_err(|e| format!("{}: {e}", path.display()))?;
        out.push((k, v));
    }
    Ok(out)
}

/// The text of a profile file.
pub fn render(name: &str, settings: &[(String, String)]) -> String {
    let mut s = format!("# trofeo_lcd profile: {name}\n# What is here wins over the configuration file while the profile is active.\n");
    for (k, v) in settings {
        s.push_str(&format!("{k} = {v}\n"));
    }
    s
}

/// Save a profile from `key = value` text (only settings that can be in a profile; the first
/// problem is returned with its key). Returns where it was written.
pub fn save_text(dir: &Path, name: &str, text: &str) -> Result<PathBuf, String> {
    valid_name(name).map_err(|e| format!("'{}': {e}", name.trim()))?;
    let doc = ConfDoc::parse(text);
    let all = fields();
    let mut pairs = Vec::new();
    for k in doc.keys() {
        let v = doc.get(&k).unwrap_or_default();
        check_setting(&all, &k, &v)?;
        pairs.push((k, v));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = path_of(dir, name);
    std::fs::write(&path, render(name.trim(), &pairs)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// The profile chosen by hand: `Some(name)`, `Some("none")` (chosen: no profile) or None (not chosen).
pub fn read_active(dir: &Path) -> Option<String> {
    let t = std::fs::read_to_string(dir.join(ACTIVE_FILE)).ok()?;
    let t = t.trim().to_ascii_lowercase();
    (!t.is_empty()).then_some(t)
}

/// Choose a profile by hand (`None` = no profile).
pub fn write_active(dir: &Path, name: Option<&str>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(ACTIVE_FILE), format!("{}\n", name.unwrap_or("none")))
}

/// Which profile applies now: the rule's, else the hand-chosen one, else `profile = name` of
/// the configuration file; `None` = no profile. Names that do not exist are not checked here.
pub fn selected(dir: Option<&Path>, from_file: Option<&str>) -> Option<String> {
    let clean = |s: &str| {
        let s = s.trim().to_ascii_lowercase();
        (!s.is_empty() && s != "none").then_some(s)
    };
    if let Some(a) = auto() {
        return clean(&a);
    }
    if let Some(a) = dir.and_then(read_active) {
        return clean(&a);
    }
    from_file.and_then(clean)
}

/// A short text that changes when the selection changes (the main loop reloads then).
pub fn signature(dir: Option<&Path>) -> String {
    format!("{:?}|{:?}", auto(), dir.and_then(read_active))
}

/// The next profile when going round with the hotkey: none -> first -> ... -> last -> none.
pub fn next(dir: &Path, current: Option<&str>) -> Option<String> {
    let all = list(dir);
    match current {
        None => all.first().cloned(),
        Some(c) => {
            let i = all.iter().position(|n| n == c)?;
            all.get(i + 1).cloned()
        }
    }
}

/// `profile_rules = mygame.exe=gaming, code.exe=work`: program name -> profile.
pub fn parse_rules(s: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for item in s.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let (exe, name) = item.rsplit_once('=').ok_or_else(|| format!("profile_rules: '{item}' is not program=profile"))?;
        let (exe, name) = (exe.trim().to_ascii_lowercase(), name.trim().to_ascii_lowercase());
        if exe.is_empty() {
            return Err(format!("profile_rules: '{item}' has no program name"));
        }
        valid_name(&name).map_err(|e| format!("profile_rules: '{name}': {e}"))?;
        out.push((exe, name));
    }
    Ok(out)
}

/// The profile a rule gives to the program `exe` (a path or a file name; case does not matter).
pub fn rule_match(rules: &[(String, String)], exe: &str) -> Option<String> {
    let file = exe.rsplit(['/', '\\']).next().unwrap_or(exe).to_ascii_lowercase();
    rules.iter().find(|(e, _)| *e == file).map(|(_, n)| n.clone())
}

/// What to do with the rules: the state of "which profile do the rules want now".
#[derive(Default)]
pub struct RuleState {
    current: Option<String>,
    left_at: Option<std::time::Instant>,
}

impl RuleState {
    /// `matched`: the profile a rule gives to the program in the foreground now. Returns the
    /// profile the rules want: the match at once, and the end of a match only after
    /// [`RULE_HOLD`].
    pub fn update(&mut self, matched: Option<String>, now: std::time::Instant) -> Option<String> {
        match matched {
            Some(m) => {
                self.current = Some(m);
                self.left_at = None;
            }
            None if self.current.is_some() => {
                let since = *self.left_at.get_or_insert(now);
                if now.duration_since(since) >= RULE_HOLD {
                    self.current = None;
                    self.left_at = None;
                }
            }
            None => {}
        }
        self.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "rule" choice is a global: the tests that read or change it take turns.
    static LOCK: Mutex<()> = Mutex::new(());

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("trofeo_profiles_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("work-2_a").is_ok());
        for bad in ["", "none", "Work", "a b", "../x", &"x".repeat(41)] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_profile_can_be_saved_listed_and_loaded() {
        let dir = tmp("save");
        save_text(&dir, "gaming", "layout = gaming\nbrightness = 100\n# a comment\n").unwrap();
        save_text(&dir, "work", "theme = mono\nbrightness = 60\n").unwrap();
        assert_eq!(list(&dir), vec!["gaming".to_string(), "work".to_string()]);
        let g = load(&dir, "gaming").unwrap();
        assert!(g.contains(&("brightness".to_string(), "100".to_string())) && g.len() == 2);
        assert!(render("x", &g).starts_with("# trofeo_lcd profile: x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_cannot_be_switched_live_is_refused() {
        let dir = tmp("refuse");
        for (text, why) in [
            ("deepcool = false\n", "restart"),
            ("profile = work\n", "cannot be in a profile"),
            ("profile_key = f9\n", "cannot be in a profile"),
            ("not_a_key = 1\n", "not a setting"),
            ("brightness = 500\n", "not valid"),
        ] {
            let e = save_text(&dir, "p", text).unwrap_err();
            assert!(e.contains(why), "{text:?} -> {e}");
        }
        assert!(!dir.exists(), "nothing was written");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path_of(&dir, "hand"), "deepcool = false\n").unwrap();
        assert!(load(&dir, "hand").is_err(), "a file written by hand is checked too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_selection_goes_rule_then_hand_then_file() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tmp("select");
        set_auto(None);
        assert_eq!(selected(Some(&dir), Some("work")).as_deref(), Some("work"), "from the file");
        write_active(&dir, Some("gaming")).unwrap();
        assert_eq!(selected(Some(&dir), Some("work")).as_deref(), Some("gaming"), "by hand wins over the file");
        write_active(&dir, None).unwrap();
        assert_eq!(selected(Some(&dir), Some("work")), None, "'none' by hand = no profile at all");
        set_auto(Some("movie".to_string()));
        assert_eq!(selected(Some(&dir), None).as_deref(), Some("movie"), "a rule wins over the hand");
        assert!(signature(Some(&dir)).contains("movie"));
        set_auto(None);
        assert_eq!(selected(None, Some(" None ")), None);
        assert_eq!(selected(None, None), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hotkey_goes_round_the_profiles_and_back_to_none() {
        let dir = tmp("next");
        assert_eq!(next(&dir, None), None, "no profiles");
        save_text(&dir, "a", "brightness = 10\n").unwrap();
        save_text(&dir, "b", "brightness = 20\n").unwrap();
        assert_eq!(next(&dir, None).as_deref(), Some("a"));
        assert_eq!(next(&dir, Some("a")).as_deref(), Some("b"));
        assert_eq!(next(&dir, Some("b")), None, "after the last: none");
        assert_eq!(next(&dir, Some("gone")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rules_are_parsed_and_match_a_program_by_its_file_name() {
        let r = parse_rules("MyGame.exe = gaming, code.exe=work ,").unwrap();
        assert_eq!(r, vec![("mygame.exe".to_string(), "gaming".to_string()), ("code.exe".to_string(), "work".to_string())]);
        assert_eq!(rule_match(&r, r"C:\Games\MYGAME.EXE").as_deref(), Some("gaming"));
        assert_eq!(rule_match(&r, "/usr/bin/code.exe").as_deref(), Some("work"));
        assert_eq!(rule_match(&r, "notepad.exe"), None);
        assert!(parse_rules("").unwrap().is_empty());
        for bad in ["mygame.exe", "=gaming", "a.exe=Bad Name", "a.exe=none"] {
            assert!(parse_rules(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_rule_applies_at_once_and_ends_only_after_the_hold() {
        let (mut s, t0) = (RuleState::default(), std::time::Instant::now());
        assert_eq!(s.update(None, t0), None);
        assert_eq!(s.update(Some("gaming".into()), t0).as_deref(), Some("gaming"));
        assert_eq!(s.update(None, t0 + Duration::from_secs(1)).as_deref(), Some("gaming"), "a quick Alt-Tab");
        assert_eq!(s.update(Some("gaming".into()), t0 + Duration::from_secs(2)).as_deref(), Some("gaming"));
        assert_eq!(s.update(None, t0 + Duration::from_secs(3)).as_deref(), Some("gaming"));
        assert_eq!(s.update(None, t0 + Duration::from_secs(9)), None, "gone for good");
        assert_eq!(s.update(Some("work".into()), t0 + Duration::from_secs(10)).as_deref(), Some("work"));
    }

    #[test]
    fn a_profile_wins_over_the_file_and_the_theme_fills_in_the_rest() {
        use crate::config::ConfigFile;
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_auto(None);
        let dir = tmp("config");
        save_text(&dir, "night", "brightness = 30\ntheme = mono\n").unwrap();
        // chosen by `profile = ` in the file
        let mut f = ConfigFile::parse_raw_for_tests("profile = night\nbrightness = 90\nclock_color = #FF0000\n");
        f.apply_profile_in(Some(&dir)).unwrap();
        assert_eq!(f.get("brightness"), Some("30"), "the profile wins over the file");
        assert_eq!(f.get("clock_color"), Some("#FF0000"), "the rest of the file stays");
        f.apply_theme_in(None).unwrap_or(());
        assert_eq!(f.profile.as_deref(), Some("night"));
        // the theme named by the profile gives its defaults (built-in: no folder needed)
        assert_eq!(f.get("theme"), Some("mono"));
        // by hand: wins over the file's `profile`; 'none' by hand: no profile
        save_text(&dir, "other", "brightness = 55\n").unwrap();
        write_active(&dir, Some("other")).unwrap();
        let mut g = ConfigFile::parse_raw_for_tests("profile = night\n");
        g.apply_profile_in(Some(&dir)).unwrap();
        assert_eq!(g.get("brightness"), Some("55"));
        write_active(&dir, None).unwrap();
        let mut h = ConfigFile::parse_raw_for_tests("profile = night\nbrightness = 90\n");
        h.apply_profile_in(Some(&dir)).unwrap();
        assert_eq!((h.get("brightness"), h.profile.clone()), (Some("90"), None));
        // a typo in `profile =` is an error; a chosen one that was deleted is ignored
        let mut bad = ConfigFile::parse_raw_for_tests("profile = nigth\n");
        std::fs::remove_file(dir.join(ACTIVE_FILE)).unwrap();
        assert!(bad.apply_profile_in(Some(&dir)).unwrap_err().contains("not found"));
        write_active(&dir, Some("deleted")).unwrap();
        let mut ok = ConfigFile::parse_raw_for_tests("brightness = 90\n");
        assert!(ok.apply_profile_in(Some(&dir)).is_ok() && ok.get("brightness") == Some("90"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
