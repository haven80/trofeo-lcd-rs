//! Reads what Steam keeps on this computer: the games that are installed, their covers and
//! how long they were played. Nothing is downloaded and no account is touched; everything
//! comes from the files of the Steam client:
//!
//! * `steamapps/libraryfolders.vdf` - the folders where games are installed;
//! * `steamapps/appmanifest_<appid>.acf` - one file per installed game (name, folder, last play);
//! * `appcache/librarycache/<appid>/...` - the covers the client has already downloaded;
//! * `userdata/<id>/config/localconfig.vdf` - the play time, when the client keeps it there.
//!
//! Only INSTALLED games are listed (a game that is owned but not installed has no manifest
//! on disk). The covers are the ones the Steam client has cached: a game whose cover the
//! client never showed has none and gets a drawn card with its name instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------------- VDF

/// A value of Valve's text format (`"key" "value"` and `"key" { ... }`).
#[derive(Clone, Debug, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// The child `key` (case-insensitive) of an object.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(v) => v.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn children(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(v) => v,
            Vdf::Str(_) => &[],
        }
    }

    /// Follows a path of keys: `get_path(&["Software", "Valve", "Steam"])`.
    pub fn get_path(&self, path: &[&str]) -> Option<&Vdf> {
        path.iter().try_fold(self, |v, k| v.get(k))
    }
}

/// Parses a VDF text. The top level is returned as an object (a file is a list of keys).
/// Anything it cannot read ends the parse early instead of failing: the caller gets what
/// was understood so far.
pub fn parse_vdf(text: &str) -> Vdf {
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0usize;
    Vdf::Obj(parse_obj(&chars, &mut pos, 0))
}

fn skip_blank(c: &[char], pos: &mut usize) {
    loop {
        while *pos < c.len() && c[*pos].is_whitespace() {
            *pos += 1;
        }
        // `// comment` up to the end of the line
        if *pos + 1 < c.len() && c[*pos] == '/' && c[*pos + 1] == '/' {
            while *pos < c.len() && c[*pos] != '\n' {
                *pos += 1;
            }
            continue;
        }
        break;
    }
}

fn parse_token(c: &[char], pos: &mut usize) -> Option<String> {
    skip_blank(c, pos);
    if *pos >= c.len() {
        return None;
    }
    if c[*pos] == '"' {
        *pos += 1;
        let mut s = String::new();
        while *pos < c.len() && c[*pos] != '"' {
            if c[*pos] == '\\' && *pos + 1 < c.len() {
                *pos += 1;
                s.push(match c[*pos] {
                    'n' => '\n',
                    't' => '\t',
                    other => other, // \\ and \" and anything else: the character itself
                });
            } else {
                s.push(c[*pos]);
            }
            *pos += 1;
        }
        *pos += 1; // closing quote
        Some(s)
    } else if c[*pos] == '{' || c[*pos] == '}' {
        None
    } else {
        // unquoted word
        let start = *pos;
        while *pos < c.len() && !c[*pos].is_whitespace() && c[*pos] != '{' && c[*pos] != '}' && c[*pos] != '"' {
            *pos += 1;
        }
        Some(c[start..*pos].iter().collect())
    }
}

fn parse_obj(c: &[char], pos: &mut usize, depth: u32) -> Vec<(String, Vdf)> {
    let mut out = Vec::new();
    if depth > 24 {
        return out;
    }
    loop {
        skip_blank(c, pos);
        if *pos >= c.len() {
            return out;
        }
        if c[*pos] == '}' {
            *pos += 1;
            return out;
        }
        let Some(key) = parse_token(c, pos) else {
            // a stray brace: skip it so the parse always moves forward
            *pos += 1;
            continue;
        };
        skip_blank(c, pos);
        if *pos < c.len() && c[*pos] == '{' {
            *pos += 1;
            out.push((key, Vdf::Obj(parse_obj(c, pos, depth + 1))));
        } else if let Some(v) = parse_token(c, pos) {
            out.push((key, Vdf::Str(v)));
        } else {
            return out;
        }
    }
}

// ------------------------------------------------------------------- the library

/// Which shape a cover picture has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverKind {
    /// The tall box art (600x900).
    Portrait,
    /// A wide banner (the "header" of the store page).
    Wide,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cover {
    pub path: PathBuf,
    pub kind: CoverKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SteamGame {
    pub appid: u32,
    pub name: String,
    /// The folder the game is installed in.
    pub dir: PathBuf,
    /// Unix time the game was last played (0 = never).
    pub last_played: u64,
    /// Minutes played in total, when the client keeps it in `localconfig.vdf`.
    pub playtime_min: Option<u32>,
    pub cover: Option<Cover>,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    #[allow(dead_code)]
    pub root: PathBuf,
    /// Most recently played first, then by name.
    pub games: Vec<SteamGame>,
}

/// Tools and runtimes Steam installs as "games": they are not shown.
fn is_tool(appid: u32, name: &str) -> bool {
    appid == 228980
        || name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name.starts_with("Steamworks Common")
        || name.starts_with("SteamVR")
        || name.starts_with("Steam Controller Configs")
}

/// Where Steam is installed: the path given in the config, the one the Steam client writes in the
/// registry (Windows), or the usual folders.
pub fn find_steam(custom: Option<&Path>) -> Option<PathBuf> {
    let looks_ok = |p: &Path| p.join("steamapps").is_dir();
    if let Some(c) = custom {
        return looks_ok(c).then(|| c.to_path_buf());
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(p) = registry_steam_path() {
            candidates.push(PathBuf::from(p));
        }
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(pf) = std::env::var_os(var) {
                candidates.push(PathBuf::from(pf).join("Steam"));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".steam/steam"));
        candidates.push(home.join(".local/share/Steam"));
        candidates.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    candidates.into_iter().find(|p| looks_ok(p))
}

#[cfg(windows)]
fn registry_steam_path() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let subkey: Vec<u16> = "Software\\Valve\\Steam".encode_utf16().chain(std::iter::once(0)).collect();
    let value: Vec<u16> = "SteamPath".encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the UTF-16 strings are valid and `buf`/`size` are local variables.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (size as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The folders that hold games: Steam's own and the ones in `libraryfolders.vdf`.
fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) else {
        return out;
    };
    let vdf = parse_vdf(&text);
    if let Some(lf) = vdf.get("libraryfolders") {
        for (key, v) in lf.children() {
            if !key.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            // new format: "0" { "path" "D:\\Games" ... }; old format: "1" "D:\\Games"
            let p = v.get("path").and_then(Vdf::str).or_else(|| v.str());
            if let Some(p) = p {
                let p = PathBuf::from(p);
                if !out.iter().any(|o| same_path(o, &p)) {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}

/// The cover the client has cached for `appid`: the tall box art when there is one, else a banner.
/// The files live in `appcache/librarycache/<appid>/` (newer clients add a folder with a hash
/// name inside it) or, in older clients, directly in `librarycache` as `<appid>_<name>.jpg`.
pub fn find_cover(root: &Path, appid: u32) -> Option<Cover> {
    let cache = root.join("appcache/librarycache");
    let portrait = ["library_600x900.jpg", "library_600x900.png", "library_600x900_2x.jpg"];
    let wide = ["header.jpg", "library_header.jpg", "capsule_616x353.jpg", "library_hero.jpg", "header.png"];
    let dir = cache.join(appid.to_string());
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut stack = vec![(dir, 0u32)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 2 {
                    stack.push((p, depth + 1));
                }
            } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                files.push((p.clone(), n.to_ascii_lowercase()));
            }
        }
    }
    let pick = |names: &[&str]| -> Option<PathBuf> {
        for n in names {
            if let Some((p, _)) = files.iter().find(|(_, f)| f == n) {
                return Some(p.clone());
            }
        }
        None
    };
    if let Some(p) = pick(&portrait) {
        return Some(Cover { path: p, kind: CoverKind::Portrait });
    }
    // the flat layout of older clients
    for n in &portrait[..2] {
        let p = cache.join(format!("{appid}_{n}"));
        if p.is_file() {
            return Some(Cover { path: p, kind: CoverKind::Portrait });
        }
    }
    if let Some(p) = pick(&wide) {
        return Some(Cover { path: p, kind: CoverKind::Wide });
    }
    for n in ["header.jpg", "library_header.jpg"] {
        let p = cache.join(format!("{appid}_{n}"));
        if p.is_file() {
            return Some(Cover { path: p, kind: CoverKind::Wide });
        }
    }
    None
}

/// Minutes played per game from the most recently used `userdata/<id>/config/localconfig.vdf`.
fn playtimes(root: &Path) -> HashMap<u32, u32> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    if let Ok(rd) = std::fs::read_dir(root.join("userdata")) {
        for e in rd.flatten() {
            let f = e.path().join("config/localconfig.vdf");
            if let Ok(m) = std::fs::metadata(&f).and_then(|m| m.modified()) {
                if best.as_ref().map_or(true, |(t, _)| m > *t) {
                    best = Some((m, f));
                }
            }
        }
    }
    let mut out = HashMap::new();
    let Some((_, file)) = best else { return out };
    let Ok(text) = std::fs::read_to_string(file) else { return out };
    let vdf = parse_vdf(&text);
    let apps = vdf
        .get("UserLocalConfigStore")
        .and_then(|v| v.get_path(&["Software", "Valve", "Steam", "apps"]))
        .or_else(|| vdf.get_path(&["Software", "Valve", "Steam", "apps"]));
    if let Some(apps) = apps {
        for (id, v) in apps.children() {
            if let (Ok(id), Some(t)) = (id.parse::<u32>(), v.get("Playtime").and_then(Vdf::str).and_then(|t| t.trim().parse::<u32>().ok())) {
                out.insert(id, t);
            }
        }
    }
    out
}

/// Reads every installed game. It touches a few hundred small files at most: do not call it from
/// the drawing loop (see [`request`]).
pub fn load(root: &Path) -> Library {
    let times = playtimes(root);
    let mut games = Vec::new();
    for lib in library_folders(root) {
        let apps = lib.join("steamapps");
        let Ok(rd) = std::fs::read_dir(&apps) else { continue };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if !(fname.starts_with("appmanifest_") && fname.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            let vdf = parse_vdf(&text);
            let Some(st) = vdf.get("AppState") else { continue };
            let Some(appid) = st.get("appid").and_then(Vdf::str).and_then(|s| s.trim().parse::<u32>().ok()) else { continue };
            let name = st.get("name").and_then(Vdf::str).unwrap_or("").trim().to_string();
            if name.is_empty() || is_tool(appid, &name) {
                continue;
            }
            let installdir = st.get("installdir").and_then(Vdf::str).unwrap_or("");
            if games.iter().any(|g: &SteamGame| g.appid == appid) {
                continue;
            }
            games.push(SteamGame {
                appid,
                name,
                dir: apps.join("common").join(installdir),
                last_played: st.get("LastPlayed").and_then(Vdf::str).and_then(|s| s.trim().parse().ok()).unwrap_or(0),
                playtime_min: times.get(&appid).copied(),
                cover: find_cover(root, appid),
            });
        }
    }
    games.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Library { root: root.to_path_buf(), games }
}

impl Library {
    /// The game whose folder holds `exe`.
    pub fn game_for_exe(&self, exe: &Path) -> Option<&SteamGame> {
        let e = norm(exe);
        self.games
            .iter()
            .filter(|g| {
                let d = norm(&g.dir);
                !d.ends_with("/common") && e.starts_with(&format!("{d}/"))
            })
            .max_by_key(|g| g.dir.as_os_str().len())
    }
}

// -------------------------------------------------------------- shared, in background

struct State {
    lib: Option<Arc<Library>>,
    loaded: Option<Instant>,
    loading: bool,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { lib: None, loaded: None, loading: false }))
}

/// How often the games are read again (a game installed or played shows up after this).
const REFRESH: Duration = Duration::from_secs(300);

/// Starts reading the library in the background when it was never read or is old. Cheap to call
/// every frame.
pub fn request(custom: Option<&Path>) {
    {
        let Ok(mut st) = state().lock() else { return };
        if st.loading || st.loaded.is_some_and(|t| t.elapsed() < REFRESH) {
            return;
        }
        st.loading = true;
    }
    let custom = custom.map(Path::to_path_buf);
    std::thread::spawn(move || {
        let lib = find_steam(custom.as_deref()).map(|r| Arc::new(load(&r)));
        if let Ok(mut st) = state().lock() {
            if lib.is_some() || st.lib.is_none() {
                st.lib = lib;
            }
            st.loaded = Some(Instant::now());
            st.loading = false;
        }
    });
}

/// The library read so far (`None` until the first read ends, or when there is no Steam).
pub fn shared() -> Option<Arc<Library>> {
    state().lock().ok().and_then(|s| s.lib.clone())
}

/// Reads at once (previews and tests); the result is also what [`shared`] returns afterwards.
pub fn load_now(custom: Option<&Path>) -> Option<Arc<Library>> {
    let lib = find_steam(custom).map(|r| Arc::new(load(&r)))?;
    if let Ok(mut st) = state().lock() {
        st.lib = Some(lib.clone());
        st.loaded = Some(Instant::now());
    }
    Some(lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vdf_parser_reads_nested_objects_comments_and_escapes() {
        let v = parse_vdf("\"AppState\"\n{\n\t\"appid\"\t\t\"730\"\n\t// a comment\n\t\"name\" \"Counter \\\"Strike\\\"\"\n\t\"UserConfig\" { \"language\" \"english\" }\n}\n");
        let st = v.get("appstate").unwrap();
        assert_eq!(st.get("appid").and_then(Vdf::str), Some("730"));
        assert_eq!(st.get("name").and_then(Vdf::str), Some("Counter \"Strike\""));
        assert_eq!(st.get_path(&["UserConfig", "language"]).and_then(Vdf::str), Some("english"));
        // broken input never panics and keeps what was understood
        let broken = parse_vdf("\"a\" { \"b\" \"1\" \"c\" { \"d\"");
        assert_eq!(broken.get_path(&["a", "b"]).and_then(Vdf::str), Some("1"));
        let _ = parse_vdf("}}} \"\" { { {");
    }

    fn fake_steam(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("trofeo_steam_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("steamapps");
        std::fs::create_dir_all(apps.join("common/Starfall Odyssey")).unwrap();
        std::fs::create_dir_all(apps.join("common/Moonlit Run")).unwrap();
        let second = root.join("lib2");
        std::fs::create_dir_all(second.join("steamapps/common/Tiny Orchard")).unwrap();
        std::fs::write(
            apps.join("libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", root.display().to_string().replace('\\', "\\\\"), second.display().to_string().replace('\\', "\\\\")),
        )
        .unwrap();
        let manifest = |id: u32, name: &str, dir: &str, last: u64| format!("\"AppState\"\n{{\n\t\"appid\"\t\"{id}\"\n\t\"name\"\t\"{name}\"\n\t\"installdir\"\t\"{dir}\"\n\t\"LastPlayed\"\t\"{last}\"\n}}\n");
        std::fs::write(apps.join("appmanifest_1145360.acf"), manifest(1145360, "Starfall Odyssey", "Starfall Odyssey", 1_700_000_500)).unwrap();
        std::fs::write(apps.join("appmanifest_504230.acf"), manifest(504230, "Moonlit Run", "Moonlit Run", 1_600_000_000)).unwrap();
        std::fs::write(apps.join("appmanifest_228980.acf"), manifest(228980, "Steamworks Common Redistributables", "Steamworks Shared", 0)).unwrap();
        std::fs::write(second.join("steamapps/appmanifest_413150.acf"), manifest(413150, "Tiny Orchard", "Tiny Orchard", 0)).unwrap();
        // covers: a hashed sub-folder (new clients), a wide banner only, a flat old-style file
        let c = root.join("appcache/librarycache/1145360/0a1b2c3d4e");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("library_600x900.jpg"), b"x").unwrap();
        let c = root.join("appcache/librarycache/504230");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("header.jpg"), b"x").unwrap();
        std::fs::write(root.join("appcache/librarycache/413150_library_600x900.jpg"), b"x").unwrap();
        // play time
        let cfg = root.join("userdata/12345/config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("localconfig.vdf"), "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1145360\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"Playtime\"\t\t\"3725\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n").unwrap();
        root
    }

    #[test]
    fn the_installed_games_covers_and_play_time_are_read_from_the_steam_folders() {
        let root = fake_steam("lib");
        assert_eq!(find_steam(Some(&root)), Some(root.clone()));
        assert_eq!(find_steam(Some(&root.join("nothing"))), None);
        let lib = load(&root);
        let names: Vec<_> = lib.games.iter().map(|g| g.name.as_str()).collect();
        // most recently played first; the redistributables are not games; a second library folder counts
        assert_eq!(names, vec!["Starfall Odyssey", "Moonlit Run", "Tiny Orchard"]);
        let hades = &lib.games[0];
        assert_eq!((hades.appid, hades.playtime_min), (1145360, Some(3725)));
        assert_eq!(hades.cover.as_ref().map(|c| c.kind), Some(CoverKind::Portrait));
        assert!(hades.cover.as_ref().unwrap().path.ends_with("library_600x900.jpg"));
        assert_eq!(lib.games[1].cover.as_ref().map(|c| c.kind), Some(CoverKind::Wide));
        assert_eq!(lib.games[2].cover.as_ref().map(|c| c.kind), Some(CoverKind::Portrait));
        assert_eq!(lib.games[2].playtime_min, None);
        // an exe inside the game's folder belongs to that game, in any spelling of the path
        let exe = root.join("steamapps/common/Starfall Odyssey/x64/Starfall.exe");
        assert_eq!(lib.game_for_exe(&exe).map(|g| g.appid), Some(1145360));
        let win = PathBuf::from(exe.to_string_lossy().replace('/', "\\").to_uppercase());
        assert_eq!(lib.game_for_exe(&win).map(|g| g.appid), Some(1145360));
        assert!(lib.game_for_exe(Path::new("C:/Windows/notepad.exe")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_without_steam_gives_an_empty_library() {
        let lib = load(Path::new("/definitely/not/here"));
        assert!(lib.games.is_empty());
    }
}
