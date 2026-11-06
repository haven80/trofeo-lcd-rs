//! What the program knows about the game that is running: its name, where it comes from
//! (Steam, Epic, GOG, Xbox), how long this session lasts, the size of its window, the memory it
//! uses and the FPS of the session (average, lowest, highest).
//!
//! The game is the program that owns the foreground window while the gaming screen is on. Its name
//! comes, in this order, from:
//!
//! 1. Steam: if the .exe sits inside the folder of an installed Steam game, the name, the AppID,
//!    the cover and the total play time are the ones Steam has on disk (see `steam.rs`);
//! 2. the folder the game is installed in, for Epic Games / GOG;
//! 3. the title of the window;
//! 4. the name of the .exe.

use crate::foreground::WindowInfo;
use crate::steam::{Cover, Library};
use std::path::Path;
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};

/// Where the game was bought / is launched from, as far as its folder tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Steam,
    Epic,
    Gog,
    Xbox,
    Other,
}

impl Store {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Store::Steam => Some("Steam"),
            Store::Epic => Some("Epic Games"),
            Store::Gog => Some("GOG"),
            Store::Xbox => Some("Xbox"),
            Store::Other => None,
        }
    }
}

/// FPS of the whole session of one game.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SessionFps {
    sum: u64,
    n: u32,
    pub min: Option<i32>,
    pub max: Option<i32>,
}

impl SessionFps {
    pub fn push(&mut self, fps: i32) {
        // 0 is "nothing drawn" (a loading screen, a pause): it says nothing about the game's speed
        if fps <= 0 {
            return;
        }
        self.sum += fps as u64;
        self.n += 1;
        self.min = Some(self.min.map_or(fps, |m| m.min(fps)));
        self.max = Some(self.max.map_or(fps, |m| m.max(fps)));
    }

    pub fn avg(&self) -> Option<i32> {
        (self.n > 0).then(|| (self.sum as f64 / self.n as f64).round() as i32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameInfo {
    pub pid: u32,
    pub name: String,
    pub exe_name: String,
    pub store: Store,
    pub appid: Option<u32>,
    /// Total minutes played according to Steam (before this session is counted).
    pub playtime_min: Option<u32>,
    pub cover: Option<Cover>,
    /// Size of the game's window (what it renders at, in a window or borderless).
    pub window: Option<(u32, u32)>,
    pub mem_mb: Option<u64>,
    pub fps: SessionFps,
    started: Instant,
    now: Instant,
}

impl GameInfo {
    /// How long this session has lasted.
    pub fn session(&self) -> Duration {
        self.now.saturating_duration_since(self.started)
    }

    /// "1:05:09" or "12:30".
    pub fn session_text(&self) -> String {
        clock_text(self.session().as_secs())
    }

    /// "62 h" / "3.5 h" / "40 min" of Steam's total play time.
    pub fn playtime_text(&self) -> Option<String> {
        self.playtime_min.map(playtime_text)
    }

    /// "2560x1440".
    pub fn resolution_text(&self) -> Option<String> {
        self.window.filter(|(w, h)| *w > 0 && *h > 0).map(|(w, h)| format!("{w}x{h}"))
    }

    /// Sample data for the previews and the editor.
    pub fn sample() -> GameInfo {
        let now = Instant::now();
        let mut fps = SessionFps::default();
        for f in [141, 144, 138, 120, 99, 144, 144, 150] {
            fps.push(f);
        }
        GameInfo {
            pid: 1,
            name: "Starfall Odyssey".into(),
            exe_name: "Starfall.exe".into(),
            store: Store::Steam,
            appid: Some(1145360),
            playtime_min: Some(3725),
            cover: None,
            window: Some((2560, 1440)),
            mem_mb: Some(4210),
            fps,
            started: now.checked_sub(Duration::from_secs(3725 + 9)).unwrap_or(now),
            now,
        }
    }
}

pub fn clock_text(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn playtime_text(min: u32) -> String {
    if min >= 600 {
        format!("{} h", min / 60)
    } else if min >= 60 {
        format!("{:.1} h", min as f32 / 60.0)
    } else {
        format!("{min} min")
    }
}

/// "eldenring" -> "Eldenring", "Hollow_Knight-win64" -> "Hollow Knight win64".
fn pretty_exe(stem: &str) -> String {
    let s: String = stem.chars().map(|c| if matches!(c, '_' | '-' | '.') { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cs = s.chars();
    match cs.next() {
        Some(c) if s.chars().all(|c| !c.is_uppercase()) => c.to_uppercase().collect::<String>() + cs.as_str(),
        _ => s,
    }
}

/// The folder right under `marker` in the path of the exe: `.../steamapps/common/Starfall Odyssey/x64/h.exe` -> "Starfall Odyssey".
fn folder_after(exe: &Path, markers: &[&str]) -> Option<String> {
    let comps: Vec<String> = exe.to_string_lossy().replace('\\', "/").split('/').map(str::to_string).collect();
    for (i, c) in comps.iter().enumerate() {
        let lc = c.to_lowercase();
        if markers.iter().any(|m| *m == lc) {
            // the exe itself is not a folder
            return comps.get(i + 1).filter(|n| i + 2 < comps.len() && !n.is_empty()).cloned();
        }
    }
    None
}

#[derive(Debug, PartialEq)]
struct Resolved {
    name: String,
    store: Store,
    appid: Option<u32>,
    playtime_min: Option<u32>,
    cover: Option<Cover>,
}

fn resolve(win: &WindowInfo, steam: Option<&Library>) -> Resolved {
    let stem = win.exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if let Some(g) = steam.and_then(|l| l.game_for_exe(&win.exe)) {
        return Resolved { name: g.name.clone(), store: Store::Steam, appid: Some(g.appid), playtime_min: g.playtime_min, cover: g.cover.clone() };
    }
    let lower = win.exe.to_string_lossy().replace('\\', "/").to_lowercase();
    let (store, folder) = if lower.contains("/steamapps/common/") {
        (Store::Steam, folder_after(&win.exe, &["common"]))
    } else if lower.contains("/epic games/") {
        (Store::Epic, folder_after(&win.exe, &["epic games"]))
    } else if lower.contains("/gog galaxy/games/") {
        (Store::Gog, folder_after(&win.exe, &["games"]))
    } else if lower.contains("/gog games/") {
        (Store::Gog, folder_after(&win.exe, &["gog games"]))
    } else if lower.contains("/xboxgames/") {
        (Store::Xbox, None)
    } else {
        (Store::Other, None)
    };
    let title = win.title.trim();
    let title_ok = title.chars().count() >= 2 && title.chars().count() <= 60 && !title.eq_ignore_ascii_case("program manager");
    // the folder of a store names the game better than a window title with extras in it
    let name = match (folder, title_ok) {
        (Some(f), _) if !f.is_empty() => f,
        (_, true) => title.to_string(),
        _ => pretty_exe(&stem),
    };
    Resolved { name, store, appid: None, playtime_min: None, cover: None }
}

/// Follows the game across the seconds of a session.
#[derive(Default)]
pub struct Tracker {
    current: Option<GameInfo>,
    mem_at: Option<Instant>,
}

impl Tracker {
    pub fn info(&self) -> Option<&GameInfo> {
        self.current.as_ref()
    }

    /// The gaming screen is off: the session is over.
    pub fn end(&mut self) {
        self.current = None;
        self.mem_at = None;
    }

    /// Call it every frame while gaming. `fg_fps` is the FPS read for the foreground program (it
    /// is what tells a game from the browser the player alt-tabbed to).
    pub fn update(&mut self, now: Instant, win: Option<WindowInfo>, sys: &mut System, steam: Option<&Library>, fg_fps: Option<i32>) {
        let own = std::process::id();
        let alive = |sys: &mut System, pid: u32| sys.refresh_process(Pid::from_u32(pid));
        // Which program is the game: the foreground one if it draws frames (or nothing is tracked
        // yet), else the one already tracked, as long as it still runs.
        let cur_pid = self.current.as_ref().map(|g| g.pid);
        let switch = match (&win, cur_pid) {
            (Some(w), Some(c)) if w.pid == c => false,
            (Some(w), None) => w.pid != own,
            (Some(w), Some(c)) => w.pid != own && (fg_fps.is_some_and(|f| f > 0) || !alive(sys, c)),
            (None, Some(c)) => !alive(sys, c),
            (None, None) => false,
        };
        if switch {
            self.current = None;
            if let Some(w) = &win {
                if w.pid != own {
                    let r = resolve(w, steam);
                    self.current = Some(GameInfo {
                        pid: w.pid,
                        name: r.name,
                        exe_name: w.exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        store: r.store,
                        appid: r.appid,
                        playtime_min: r.playtime_min,
                        cover: r.cover,
                        window: w.size,
                        mem_mb: None,
                        fps: SessionFps::default(),
                        started: now,
                        now,
                    });
                    self.mem_at = None;
                }
            }
        }
        let Some(g) = self.current.as_mut() else { return };
        g.now = now;
        if let Some(w) = win.as_ref().filter(|w| w.pid == g.pid) {
            g.window = w.size.or(g.window);
        }
        // the title of a window often shows the game's name only after it has loaded
        if g.store == Store::Other && g.appid.is_none() {
            if let Some(w) = win.as_ref().filter(|w| w.pid == g.pid) {
                let r = resolve(w, steam);
                if r.store != Store::Other || r.name != g.name {
                    g.name = r.name;
                    g.store = r.store;
                }
            }
        }
        if win.as_ref().is_some_and(|w| w.pid == g.pid) {
            if let Some(f) = fg_fps {
                g.fps.push(f);
            }
        }
        // memory: once every few seconds is plenty
        if self.mem_at.map_or(true, |t| now.saturating_duration_since(t) >= Duration::from_secs(3)) {
            if sys.refresh_process(Pid::from_u32(g.pid)) {
                g.mem_mb = sys.process(Pid::from_u32(g.pid)).map(|p| p.memory() / 1024 / 1024);
            }
            self.mem_at = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn win(exe: &str, title: &str) -> WindowInfo {
        WindowInfo { pid: 4242, exe: PathBuf::from(exe), title: title.into(), size: Some((2560, 1440)) }
    }

    #[test]
    fn the_name_comes_from_steam_the_store_folder_the_title_or_the_exe() {
        // a game of the Steam library
        let lib = Library {
            root: PathBuf::from("C:/Steam"),
            games: vec![crate::steam::SteamGame {
                appid: 1145360, name: "Starfall Odyssey".into(), dir: PathBuf::from("C:/Steam/steamapps/common/Starfall Odyssey"),
                last_played: 0, playtime_min: Some(3725), cover: None,
            }],
        };
        let r = resolve(&win("C:\\Steam\\steamapps\\common\\Starfall Odyssey\\x64\\Starfall.exe", "Starfall Odyssey II"), Some(&lib));
        assert_eq!((r.name.as_str(), r.store, r.appid, r.playtime_min), ("Starfall Odyssey", Store::Steam, Some(1145360), Some(3725)));
        // a Steam game in a library folder that is not known: the folder names it
        let r = resolve(&win("E:/Games/SteamLibrary/steamapps/common/Hollow Knight/hollow_knight.exe", ""), None);
        assert_eq!((r.name.as_str(), r.store), ("Hollow Knight", Store::Steam));
        let r = resolve(&win("C:/Program Files/Epic Games/Fortnite/FortniteGame/Binaries/Win64/FortniteClient-Win64-Shipping.exe", "Fortnite  "), None);
        assert_eq!((r.name.as_str(), r.store), ("Fortnite", Store::Epic));
        let r = resolve(&win("C:/GOG Galaxy/Games/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe", "Cyberpunk 2077 (C) 2020 by CD Projekt RED"), None);
        assert_eq!((r.name.as_str(), r.store), ("Cyberpunk 2077", Store::Gog));
        // anything else: the title of the window, then the exe
        let r = resolve(&win("D:/Games/Doom/doom.exe", "DOOM Eternal"), None);
        assert_eq!((r.name.as_str(), r.store), ("DOOM Eternal", Store::Other));
        let r = resolve(&win("D:/Games/Doom/doom_eternal-x64.exe", ""), None);
        assert_eq!(r.name, "Doom eternal x64");
        assert_eq!(resolve(&win("D:/x/Game.exe", "Program Manager"), None).name, "Game");
    }

    #[test]
    fn the_session_fps_ignores_zero_and_keeps_average_lowest_and_highest() {
        let mut f = SessionFps::default();
        assert_eq!(f.avg(), None);
        for v in [60, 0, 90, 120, 0] {
            f.push(v);
        }
        assert_eq!((f.avg(), f.min, f.max), (Some(90), Some(60), Some(120)));
    }

    #[test]
    fn times_are_written_short() {
        assert_eq!(clock_text(75), "01:15");
        assert_eq!(clock_text(3725), "1:02:05");
        assert_eq!(playtime_text(40), "40 min");
        assert_eq!(playtime_text(210), "3.5 h");
        assert_eq!(playtime_text(3725), "62 h");
    }

    #[test]
    fn the_tracker_follows_the_game_and_ignores_the_program_the_player_switches_to() {
        let mut sys = System::new();
        let t0 = Instant::now();
        let mut t = Tracker::default();
        let mut game = win("D:/Games/Doom/doom.exe", "DOOM Eternal");
        game.pid = std::process::id() + 1000; // not ours, and (almost surely) not running
        // nothing tracked yet: the foreground program becomes the game
        t.update(t0, Some(game.clone()), &mut sys, None, Some(144));
        assert_eq!(t.info().map(|g| g.name.as_str()), Some("DOOM Eternal"));
        t.update(t0 + Duration::from_secs(1), Some(game.clone()), &mut sys, None, Some(100));
        assert_eq!(t.info().and_then(|g| g.fps.avg()), Some(122));
        // alt-tab to a program that draws no frames: the game is still the game while it runs...
        // (here the pid does not exist, so it counts as closed and the browser would take over)
        let mut browser = win("C:/Program Files/Browser/browser.exe", "News");
        browser.pid = game.pid + 1;
        t.update(t0 + Duration::from_secs(2), Some(browser.clone()), &mut sys, None, None);
        assert_eq!(t.info().map(|g| g.exe_name.as_str()), Some("browser.exe"));
        // ...and one that does draw frames replaces it at once
        t.update(t0 + Duration::from_secs(3), Some(game.clone()), &mut sys, None, Some(90));
        assert_eq!(t.info().map(|g| g.name.as_str()), Some("DOOM Eternal"));
        t.end();
        assert!(t.info().is_none());
    }
}
