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

pub type SharedNowPlaying = Arc<Mutex<Option<String>>>;

#[cfg(windows)]
mod imp {
    use super::SharedNowPlaying;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
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

    /// Start a background thread that polls the active song/media title once per second.
    pub fn spawn_now_playing_watcher() -> anyhow::Result<SharedNowPlaying> {
        let shared: SharedNowPlaying = Arc::new(Mutex::new(None));
        let shared_clone = shared.clone();

        std::thread::Builder::new()
            .name("now-playing-smtc".into())
            .spawn(move || {
                // COM/WinRT needs to be initialized per-thread.
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                loop {
                    let title = query_now_playing().ok().flatten();
                    if let Ok(mut guard) = shared_clone.lock() {
                        *guard = title;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            })?;

        Ok(shared)
    }

    fn query_now_playing() -> anyhow::Result<Option<String>> {
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
        if artist.trim().is_empty() {
            Ok(Some(title))
        } else {
            Ok(Some(format!("{title} - {artist}")))
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::SharedNowPlaying;
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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

    /// Start a background thread that polls the active song/media title once
    /// per second via `playerctl` (the most commonly used MPRIS command-line
    /// client on Linux — works with any player that supports the MPRIS
    /// standard: Spotify, VLC, a browser tab playing audio, etc).
    pub fn spawn_now_playing_watcher() -> anyhow::Result<SharedNowPlaying> {
        let shared: SharedNowPlaying = Arc::new(Mutex::new(None));
        let shared_clone = shared.clone();

        std::thread::Builder::new()
            .name("now-playing-mpris".into())
            .spawn(move || {
                let mut warned_missing = false;
                loop {
                    match query_now_playing() {
                        Ok(title) => {
                            if let Ok(mut guard) = shared_clone.lock() {
                                *guard = title;
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

        Ok(shared)
    }

    /// `\x1f` (ASCII unit separator) is used to separate title/artist so that
    /// only ONE `playerctl` call is needed per poll (not two) — this
    /// character is practically never going to appear in a real song title.
    fn query_now_playing() -> anyhow::Result<Option<String>> {
        let output = Command::new("playerctl")
            .args(["metadata", "--format", "{{title}}\u{1f}{{artist}}"])
            .output()
            .map_err(|e| anyhow::anyhow!("playerctl not found: {e}"))?;

        if !output.status.success() {
            // Normal & common: there's simply no active MPRIS player at
            // all right now — not an error.
            return Ok(None);
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let mut parts = text.trim_end().splitn(2, '\u{1f}');
        let title = parts.next().unwrap_or("").trim();
        let artist = parts.next().unwrap_or("").trim();

        if title.is_empty() {
            return Ok(None);
        }
        if artist.is_empty() {
            Ok(Some(title.to_string()))
        } else {
            Ok(Some(format!("{title} - {artist}")))
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::SharedNowPlaying;
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

    pub fn spawn_now_playing_watcher() -> anyhow::Result<SharedNowPlaying> {
        Ok(Arc::new(Mutex::new(None)))
    }
}

pub use imp::{spawn_now_playing_watcher, AudioMonitor};
