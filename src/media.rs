//! System master volume and the title of the media/song currently playing.
//!
//! - **Windows**: COM `IAudioEndpointVolume` (volume) + WinRT System Media
//!   Transport Controls (song title — the same official API used by
//!   Windows 11's built-in "now playing" widget).
//! - **Linux**: `pactl` (volume — compatible with PulseAudio AND
//!   PipeWire+pipewire-pulse) + `playerctl` (song title via MPRIS — the
//!   standard D-Bus API supported by nearly every Linux media player:
//!   Spotify, VLC, Firefox/Chrome, etc). Both are called as external
//!   processes (not native bindings) — much simpler & more robust than a
//!   full D-Bus/PulseAudio client implementation, with the trade-off that
//!   both tools need to be installed (very commonly present on modern Linux
//!   desktops; if missing, an install message is printed once to stderr).
//!
//! The song title query runs on a SEPARATE THREAD (similar to the audio
//! capture in `audio.rs`), rather than directly in the render loop, because
//! its calls (WinRT async / spawning a process) can occasionally take a
//! while — so the LCD frame rate doesn't stutter if that call is slow.

use std::sync::{Arc, Mutex};
use std::time::Instant;

pub type SharedNowPlaying = Arc<Mutex<Option<String>>>;

/// Album art, already decoded: a square RGB8 image (at most 640x640).
pub struct CoverArt {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Where the track is, as of `sampled_at` (the player only reports it now and
/// then, so the screen extrapolates between samples while `playing`).
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    pub sampled_at: Instant,
}

impl Timeline {
    /// Current position in ms, clamped to the track length.
    pub fn position_now_ms(&self) -> u64 {
        let extra = if self.playing { self.sampled_at.elapsed().as_millis() as u64 } else { 0 };
        (self.position_ms + extra).min(self.duration_ms)
    }
}

/// Everything the "music screen" needs about the current track.
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: Option<Arc<CoverArt>>,
    pub timeline: Option<Timeline>,
}

/// Latest track info (replaced as a whole every poll; the cover `Arc` stays the
/// same until the track changes, so `Arc::ptr_eq` detects a new cover).
pub type SharedTrack = Arc<Mutex<Option<Arc<TrackInfo>>>>;

/// The classic one-line form used by the status lines: "TITLE - ARTIST".
pub fn now_playing_string(title: &str, artist: &str) -> String {
    if artist.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title} - {artist}")
    }
}

/// How many polls (1 s apart) to keep asking for a cover that hasn't arrived
/// yet — players often publish the art a moment after the title.
const COVER_RETRIES: u32 = 6;

/// Decode album-art bytes (JPEG/PNG/GIF/BMP) into a square RGB image: the
/// centre is cropped to a square and large images are shrunk to 640 px.
pub fn decode_cover(bytes: &[u8]) -> Option<CoverArt> {
    let img = image::load_from_memory(bytes).ok()?.to_rgb8();
    let (w, h) = img.dimensions();
    if w < 2 || h < 2 {
        return None;
    }
    let side = w.min(h);
    let sq = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let out = if side > 640 {
        image::imageops::resize(&sq, 640, 640, image::imageops::FilterType::Triangle)
    } else {
        sq
    };
    Some(CoverArt { width: out.width(), height: out.height(), rgb: out.into_raw() })
}

/// One `playerctl metadata` reading (Linux/MPRIS), split into fields.
#[allow(dead_code)] // only the Linux watcher uses it (tests run everywhere)
#[derive(Debug, PartialEq)]
struct PlayerMeta {
    title: String,
    artist: String,
    album: String,
    art_url: String,
    position_ms: Option<u64>,
    length_ms: Option<u64>,
    playing: bool,
}

/// The `--format` string matching [`parse_playerctl`] (`\x1f` separates fields:
/// it practically never appears in real titles).
#[allow(dead_code)]
const PLAYERCTL_FORMAT: &str = "{{title}}\u{1f}{{artist}}\u{1f}{{album}}\u{1f}{{mpris:artUrl}}\u{1f}{{position}}\u{1f}{{mpris:length}}\u{1f}{{status}}";

/// Parse the output of `playerctl metadata --format PLAYERCTL_FORMAT`.
/// `None` when there is no title. MPRIS times are in microseconds.
#[allow(dead_code)]
fn parse_playerctl(text: &str) -> Option<PlayerMeta> {
    let mut f = text.trim_end_matches(['\n', '\r']).splitn(7, '\u{1f}');
    let mut next = || f.next().unwrap_or("").trim().to_string();
    let (title, artist, album, art_url) = (next(), next(), next(), next());
    let us_to_ms = |s: String| s.parse::<u64>().ok().map(|v| v / 1000);
    let (position_ms, length_ms) = (us_to_ms(next()), us_to_ms(next()));
    let playing = next().eq_ignore_ascii_case("playing");
    if title.is_empty() {
        return None;
    }
    Some(PlayerMeta { title, artist, album, art_url, position_ms, length_ms, playing })
}

/// `file:///home/me/My%20Music/a.jpg` -> `/home/me/My Music/a.jpg`.
#[allow(dead_code)]
fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Remembers the cover of the current track between polls.
#[derive(Default)]
struct CoverCache {
    key: (String, String),
    cover: Option<Arc<CoverArt>>,
    tries: u32,
}

impl CoverCache {
    /// The cover for `(title, artist)`, calling `fetch` only when the track
    /// changed (or while a cover is still missing, up to `COVER_RETRIES`).
    fn get(&mut self, title: &str, artist: &str, fetch: impl FnOnce() -> Option<CoverArt>) -> Option<Arc<CoverArt>> {
        if self.key.0 != title || self.key.1 != artist {
            *self = CoverCache { key: (title.to_string(), artist.to_string()), cover: None, tries: 0 };
        }
        if self.cover.is_none() && self.tries < COVER_RETRIES {
            self.tries += 1;
            self.cover = fetch().map(Arc::new);
        }
        self.cover.clone()
    }
}

#[cfg(windows)]
mod imp {
    use super::{decode_cover, now_playing_string, CoverArt, CoverCache, SharedNowPlaying, SharedTrack, Timeline, TrackInfo};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionMediaProperties,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };
    use windows::Storage::Streams::DataReader;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    pub struct AudioMonitor {
        endpoint_volume: IAudioEndpointVolume,
    }

    impl AudioMonitor {
        pub fn new() -> anyhow::Result<Self> {
            unsafe {
                // May fail if COM was already initialized with a different
                // mode on this thread earlier (e.g. by another crate) — not
                // fatal.
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
                let endpoint_volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;

                Ok(Self { endpoint_volume })
            }
        }

        /// `(volume_percent 0-100, muted)`.
        pub fn sample(&self) -> anyhow::Result<(f32, bool)> {
            unsafe {
                let level = self.endpoint_volume.GetMasterVolumeLevelScalar()?;
                let muted = self.endpoint_volume.GetMute()?.as_bool();
                Ok((level * 100.0, muted))
            }
        }
    }

    /// Start a background thread that polls the active song/media once per second
    /// (title string for the status lines + full track info for the music screen).
    pub fn spawn_track_watcher() -> anyhow::Result<(SharedNowPlaying, SharedTrack)> {
        let shared: SharedNowPlaying = Arc::new(Mutex::new(None));
        let shared_clone = shared.clone();
        let track: SharedTrack = Arc::new(Mutex::new(None));
        let track_clone = track.clone();

        std::thread::Builder::new()
            .name("now-playing-smtc".into())
            .spawn(move || {
                // COM/WinRT needs to be initialized per-thread.
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                let mut cache = CoverCache::default();
                loop {
                    let info = query_track(&mut cache).ok().flatten();
                    if let Ok(mut guard) = shared_clone.lock() {
                        *guard = info.as_ref().map(|t| now_playing_string(&t.title, &t.artist));
                    }
                    if let Ok(mut guard) = track_clone.lock() {
                        *guard = info.map(Arc::new);
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            })?;

        Ok((shared, track))
    }

    fn query_track(cache: &mut CoverCache) -> anyhow::Result<Option<TrackInfo>> {
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
        let Ok(session) = manager.GetCurrentSession() else {
            return Ok(None);
        };
        let props = session.TryGetMediaPropertiesAsync()?.get()?;

        let title = props.Title()?.to_string();
        let artist = props.Artist()?.to_string();
        if title.trim().is_empty() {
            return Ok(None);
        }
        let album = props.AlbumTitle().map(|a| a.to_string()).unwrap_or_default();
        let cover = cache.get(&title, &artist, || read_thumbnail(&props).ok().flatten());
        let timeline = read_timeline(&session).ok().flatten();
        Ok(Some(TrackInfo { title, artist, album, cover, timeline }))
    }

    /// The album art the player published through the media session.
    fn read_thumbnail(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> anyhow::Result<Option<CoverArt>> {
        let stream = props.Thumbnail()?.OpenReadAsync()?.get()?;
        let size = stream.Size()?;
        if size == 0 || size > 8 * 1024 * 1024 {
            return Ok(None);
        }
        let reader = DataReader::CreateDataReader(&stream)?;
        reader.LoadAsync(size as u32)?.get()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf)?;
        Ok(decode_cover(&buf))
    }

    /// Track position/length. Players report `Position` as of `LastUpdatedTime`
    /// (Spotify only refreshes it on play/pause/seek), so while playing it is
    /// advanced by the time elapsed since then.
    fn read_timeline(session: &GlobalSystemMediaTransportControlsSession) -> anyhow::Result<Option<Timeline>> {
        // WinRT time is in 100 ns ticks; DateTime counts from 1601-01-01 UTC.
        const TICKS_1601_TO_1970: i64 = 116_444_736_000_000_000;
        let tl = session.GetTimelineProperties()?;
        let start = tl.StartTime()?.Duration;
        let duration = tl.EndTime()?.Duration - start;
        if duration <= 0 {
            return Ok(None);
        }
        let playing = session.GetPlaybackInfo()?.PlaybackStatus()?
            == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;
        let mut pos = (tl.Position()?.Duration - start).max(0);
        if playing {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| (d.as_nanos() / 100) as i64 + TICKS_1601_TO_1970)
                .unwrap_or(0);
            let updated = tl.LastUpdatedTime()?.UniversalTime;
            if now > updated {
                pos += now - updated;
            }
        }
        Ok(Some(Timeline {
            position_ms: (pos.min(duration) / 10_000) as u64,
            duration_ms: (duration / 10_000) as u64,
            playing,
            sampled_at: Instant::now(),
        }))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{
        decode_cover, file_url_to_path, now_playing_string, parse_playerctl, CoverArt, CoverCache, SharedNowPlaying,
        SharedTrack, Timeline, TrackInfo, PLAYERCTL_FORMAT,
    };
    use std::io::Read;
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// No need to keep any connection around — each `sample()` just spawns a
    /// short-lived `pactl` process (cheap, only called on each sysinfo
    /// refresh interval, not every frame).
    pub struct AudioMonitor;

    impl AudioMonitor {
        pub fn new() -> anyhow::Result<Self> {
            Ok(Self)
        }

        /// `(volume_percent 0-100, muted)` via `pactl`.
        pub fn sample(&self) -> anyhow::Result<(f32, bool)> {
            let volume = query_default_sink_volume().unwrap_or(0.0);
            let muted = query_default_sink_muted().unwrap_or(false);
            Ok((volume, muted))
        }
    }

    fn run_pactl(args: &[&str]) -> Option<String> {
        let output = Command::new("pactl").args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }

    /// Parse the output line from `pactl get-sink-volume @DEFAULT_SINK@`, e.g.:
    /// "Volume: front-left: 65536 / 100% / 0.00 dB, front-right: ..." —
    /// take the FIRST percentage number found (any channel is fine, good
    /// enough to display as a single "master volume" number).
    fn query_default_sink_volume() -> Option<f32> {
        let text = run_pactl(&["get-sink-volume", "@DEFAULT_SINK@"])?;
        text.split_whitespace()
            .find_map(|tok| tok.strip_suffix('%')?.parse::<f32>().ok())
    }

    /// Parse the output line from `pactl get-sink-mute @DEFAULT_SINK@`: "Mute: yes"/"Mute: no".
    fn query_default_sink_muted() -> Option<bool> {
        let text = run_pactl(&["get-sink-mute", "@DEFAULT_SINK@"])?.to_lowercase();
        if text.contains("yes") {
            Some(true)
        } else if text.contains("no") {
            Some(false)
        } else {
            None
        }
    }

    /// Start a background thread that polls the active song/media once per
    /// second via `playerctl` (the most commonly used MPRIS command-line
    /// client on Linux — works with any player that supports the MPRIS
    /// standard: Spotify, VLC, a browser tab playing audio, etc).
    pub fn spawn_track_watcher() -> anyhow::Result<(SharedNowPlaying, SharedTrack)> {
        let shared: SharedNowPlaying = Arc::new(Mutex::new(None));
        let shared_clone = shared.clone();
        let track: SharedTrack = Arc::new(Mutex::new(None));
        let track_clone = track.clone();

        std::thread::Builder::new()
            .name("now-playing-mpris".into())
            .spawn(move || {
                let mut warned_missing = false;
                let mut cache = CoverCache::default();
                loop {
                    match query_track(&mut cache) {
                        Ok(info) => {
                            if let Ok(mut guard) = shared_clone.lock() {
                                *guard = info.as_ref().map(|t| now_playing_string(&t.title, &t.artist));
                            }
                            if let Ok(mut guard) = track_clone.lock() {
                                *guard = info.map(Arc::new);
                            }
                        }
                        Err(_) if !warned_missing => {
                            eprintln!(
                                "WARNING: 'playerctl' not found — the song/media title \
                                 (NOW PLAYING) will always be empty.\n  Install: \
                                 sudo apt install playerctl   (Debian/Ubuntu)\n           \
                                 sudo pacman -S playerctl     (Arch)\n           \
                                 sudo dnf install playerctl   (Fedora)"
                            );
                            warned_missing = true;
                        }
                        Err(_) => {}
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            })?;

        Ok((shared, track))
    }

    fn query_track(cache: &mut CoverCache) -> anyhow::Result<Option<TrackInfo>> {
        let output = Command::new("playerctl")
            .args(["metadata", "--format", PLAYERCTL_FORMAT])
            .output()
            .map_err(|e| anyhow::anyhow!("playerctl not found: {e}"))?;

        if !output.status.success() {
            // Normal & common: there's simply no active MPRIS player at
            // all right now — not an error.
            return Ok(None);
        }
        let Some(m) = parse_playerctl(&String::from_utf8_lossy(&output.stdout)) else {
            return Ok(None);
        };
        let cover = cache.get(&m.title, &m.artist, || fetch_cover(&m.art_url));
        let timeline = match (m.position_ms, m.length_ms) {
            (Some(position_ms), Some(duration_ms)) if duration_ms > 0 => Some(Timeline {
                position_ms: position_ms.min(duration_ms),
                duration_ms,
                playing: m.playing,
                sampled_at: Instant::now(),
            }),
            _ => None,
        };
        Ok(Some(TrackInfo { title: m.title, artist: m.artist, album: m.album, cover, timeline }))
    }

    /// Load the cover from `mpris:artUrl`: a local `file://` path or an http(s) URL.
    fn fetch_cover(url: &str) -> Option<CoverArt> {
        if url.is_empty() {
            return None;
        }
        let bytes = if url.starts_with("file://") {
            std::fs::read(file_url_to_path(url)?).ok()?
        } else if url.starts_with("http://") || url.starts_with("https://") {
            let resp = ureq::get(url).timeout(Duration::from_secs(5)).call().ok()?;
            let mut buf = Vec::new();
            resp.into_reader().take(8 * 1024 * 1024).read_to_end(&mut buf).ok()?;
            buf
        } else {
            return None;
        };
        decode_cover(&bytes)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::{SharedNowPlaying, SharedTrack};
    use std::sync::{Arc, Mutex};

    pub struct AudioMonitor;

    impl AudioMonitor {
        pub fn new() -> anyhow::Result<Self> {
            Ok(Self)
        }

        pub fn sample(&self) -> anyhow::Result<(f32, bool)> {
            Ok((0.0, false))
        }
    }

    pub fn spawn_track_watcher() -> anyhow::Result<(SharedNowPlaying, SharedTrack)> {
        Ok((Arc::new(Mutex::new(None)), Arc::new(Mutex::new(None))))
    }
}

pub use imp::{spawn_track_watcher, AudioMonitor};

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn decode_cover_crops_to_a_square_and_caps_the_size() {
        let c = decode_cover(&png_bytes(300, 200)).unwrap();
        assert_eq!((c.width, c.height), (200, 200));
        assert_eq!(c.rgb.len(), 200 * 200 * 3);
        let big = decode_cover(&png_bytes(1500, 1200)).unwrap();
        assert_eq!((big.width, big.height), (640, 640));
        assert!(decode_cover(b"not an image").is_none());
        assert!(decode_cover(&[]).is_none());
    }

    #[test]
    fn cover_cache_fetches_once_per_track_and_retries_a_missing_cover_a_few_times() {
        let mut cache = CoverCache::default();
        let calls = std::cell::Cell::new(0);
        let png = png_bytes(8, 8);
        // Cover available at once: fetched exactly once however often we poll.
        for _ in 0..5 {
            let c = cache.get("A", "X", || {
                calls.set(calls.get() + 1);
                decode_cover(&png)
            });
            assert!(c.is_some());
        }
        assert_eq!(calls.get(), 1);
        // New track: fetched again.
        cache.get("B", "X", || {
            calls.set(calls.get() + 1);
            decode_cover(&png)
        });
        assert_eq!(calls.get(), 2);
        // A cover that never arrives is retried COVER_RETRIES times, then left alone.
        let mut cache = CoverCache::default();
        let misses = std::cell::Cell::new(0);
        for _ in 0..20 {
            assert!(cache.get("C", "Y", || {
                misses.set(misses.get() + 1);
                None
            })
            .is_none());
        }
        assert_eq!(misses.get(), COVER_RETRIES);
        // ... and it appearing late is picked up.
        let mut cache = CoverCache::default();
        assert!(cache.get("D", "Z", || None).is_none());
        assert!(cache.get("D", "Z", || decode_cover(&png)).is_some());
    }

    #[test]
    fn cover_arc_is_stable_between_polls_so_ptr_eq_detects_changes() {
        let mut cache = CoverCache::default();
        let png = png_bytes(8, 8);
        let a = cache.get("A", "X", || decode_cover(&png)).unwrap();
        let b = cache.get("A", "X", || decode_cover(&png)).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let c = cache.get("B", "X", || decode_cover(&png)).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
    }

    #[test]
    fn now_playing_string_keeps_the_classic_format() {
        assert_eq!(now_playing_string("Song", "Band"), "Song - Band");
        assert_eq!(now_playing_string("Song", "  "), "Song");
        assert_eq!(now_playing_string("Song", ""), "Song");
    }

    #[test]
    fn timeline_extrapolates_only_while_playing_and_clamps() {
        let t = Timeline { position_ms: 10_000, duration_ms: 200_000, playing: false, sampled_at: Instant::now() - std::time::Duration::from_secs(30) };
        assert_eq!(t.position_now_ms(), 10_000); // paused: frozen
        let t = Timeline { playing: true, ..t };
        let p = t.position_now_ms();
        assert!((39_900..=40_500).contains(&p), "{p}"); // 10 s + ~30 s elapsed
        let t = Timeline { position_ms: 199_000, duration_ms: 200_000, playing: true, sampled_at: Instant::now() - std::time::Duration::from_secs(60) };
        assert_eq!(t.position_now_ms(), 200_000); // never past the end
    }

    #[test]
    fn parse_playerctl_reads_all_fields() {
        let line = "Bohemian Rhapsody\u{1f}Queen\u{1f}A Night at the Opera\u{1f}https://i.scdn.co/image/abc\u{1f}83000000\u{1f}354000000\u{1f}Playing\n";
        let m = parse_playerctl(line).unwrap();
        assert_eq!(m.title, "Bohemian Rhapsody");
        assert_eq!(m.artist, "Queen");
        assert_eq!(m.album, "A Night at the Opera");
        assert_eq!(m.art_url, "https://i.scdn.co/image/abc");
        assert_eq!((m.position_ms, m.length_ms, m.playing), (Some(83_000), Some(354_000), true));
        // Paused, and a player that publishes less (no album/art/times).
        let m = parse_playerctl("T\u{1f}A\u{1f}\u{1f}\u{1f}\u{1f}\u{1f}Paused").unwrap();
        assert_eq!((m.album.as_str(), m.art_url.as_str(), m.position_ms, m.length_ms, m.playing), ("", "", None, None, false));
        // Too few fields / garbage numbers must not panic.
        let m = parse_playerctl("Only a title").unwrap();
        assert_eq!((m.title.as_str(), m.artist.as_str(), m.playing), ("Only a title", "", false));
        assert!(parse_playerctl("\u{1f}Artist only").is_none());
        assert!(parse_playerctl("").is_none());
        let m = parse_playerctl("T\u{1f}A\u{1f}B\u{1f}U\u{1f}abc\u{1f}-5\u{1f}Playing").unwrap();
        assert_eq!((m.position_ms, m.length_ms), (None, None));
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(file_url_to_path("file:///home/me/My%20Music/a%26b.jpg").as_deref(), Some("/home/me/My Music/a&b.jpg"));
        assert_eq!(file_url_to_path("file:///x/100%.jpg").as_deref(), Some("/x/100%.jpg")); // stray % kept
        assert_eq!(file_url_to_path("file:///x/trailing%2").as_deref(), Some("/x/trailing%2"));
        assert_eq!(file_url_to_path("https://x/y.jpg"), None);
    }
}
