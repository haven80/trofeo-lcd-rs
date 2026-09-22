//! System audio capture (loopback — what comes out of the speakers, NOT the
//! microphone) used as the audio data source for the visualizer.
//!
//! - On **Windows**: uses WASAPI loopback via the `wasapi` crate — grabs the
//!   default *render* (output) device, then requests a client with
//!   `Capture` direction so that WASAPI automatically enables loopback mode
//!   (see `examples/record.rs` in the `wasapi` crate: "Use `Direction::Render`
//!   for loopback mode (for capturing from a playback device)").
//! - On **Linux**: uses PulseAudio/PipeWire via `libpulse-simple-binding`
//!   — connects to the special device `@DEFAULT_MONITOR@` (the monitor
//!   source of the current default sink). This special name is resolved by
//!   the PulseAudio/PipeWire-pulse SERVER itself (the same as used by
//!   `parec`/`pacat -r`), so it automatically follows along when the
//!   default output device changes, and works equally well on native
//!   PipeWire systems (via the `pipewire-pulse` compatibility layer that is
//!   now standard on modern distros) and on genuine PulseAudio.
//! - On **other OSes** (macOS/BSD, kept so this code can still be
//!   compile-checked there): a synthetic source — not real audio.
//!
//! All three paths fill the same `SAMPLE_RATE` Hz mono ring buffer, so
//! `main.rs` doesn't need to know which OS is in use.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Internal sample rate used by the visualizer (capture is resampled /
/// requested at this rate).
pub const SAMPLE_RATE: u32 = 44_100;

/// Maximum number of mono samples stored in the buffer (roughly 0.5 seconds).
const RING_CAPACITY: usize = SAMPLE_RATE as usize / 2;

pub type SharedRing = Arc<Mutex<VecDeque<f32>>>;

fn push_mono_samples(ring: &SharedRing, samples: impl Iterator<Item = f32>) {
    let mut buf = ring.lock().expect("audio ring mutex poisoned");
    for s in samples {
        if buf.len() >= RING_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(s);
    }
}

/// Grab the latest `n` mono samples from the ring buffer, zero-padded at
/// the front if there aren't enough yet (e.g. just started).
pub fn take_latest(ring: &SharedRing, n: usize) -> Vec<f32> {
    let buf = ring.lock().expect("audio ring mutex poisoned");
    let have = buf.len();
    let mut out = vec![0f32; n];
    if have == 0 {
        return out;
    }
    let take = have.min(n);
    // Take the `take` most recent samples (from the back), placing them at the end of the output buffer.
    let skip = have - take;
    for (i, s) in buf.iter().skip(skip).enumerate() {
        out[n - take + i] = *s;
    }
    out
}

/// Start the audio capture thread in the background, returning a handle to
/// the ring buffer that keeps getting filled.
pub fn spawn_capture() -> anyhow::Result<SharedRing> {
    let ring: SharedRing = Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY)));

    #[cfg(windows)]
    {
        let ring_clone = ring.clone();
        std::thread::Builder::new()
            .name("audio-capture-wasapi".into())
            .spawn(move || {
                if let Err(e) = windows_loopback::run(ring_clone) {
                    eprintln!("Audio capture (WASAPI loopback) stopped: {e:#}");
                }
            })?;
    }

    #[cfg(target_os = "linux")]
    {
        let ring_clone = ring.clone();
        std::thread::Builder::new()
            .name("audio-capture-pulse".into())
            .spawn(move || {
                if let Err(e) = linux_loopback::run(ring_clone) {
                    eprintln!(
                        "Audio capture (PulseAudio/PipeWire loopback) failed: {e:#}\n\
                         Make sure PulseAudio or PipeWire (with the pipewire-pulse package) \
                         is running. The EQ bar will stay completely silent (no fallback to fake data)."
                    );
                }
            })?;
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let ring_clone = ring.clone();
        std::thread::Builder::new()
            .name("audio-capture-fallback".into())
            .spawn(move || fallback::run(ring_clone))?;
    }

    Ok(ring)
}

#[cfg(windows)]
mod windows_loopback {
    use super::{push_mono_samples, SharedRing, SAMPLE_RATE};
    use std::time::{Duration, Instant};
    use wasapi::*;

    /// Outer reconnect loop: WASAPI ties a client to a specific device at
    /// `initialize_client` time. If the *default* render device changes
    /// afterwards (headphones plugged in/out, a Bluetooth device connecting,
    /// Windows restarting the audio service, exclusive-mode contention,
    /// etc.), that client is permanently invalidated — every further read
    /// fails forever, even though nothing is wrong with the machine. Without
    /// this outer loop, that meant the EQ bars went silent until the whole
    /// program was restarted. `run_once` gives up (returns `Err`) once reads
    /// have been failing continuously for a few seconds, and this loop just
    /// re-enumerates the (possibly new) default device and starts over —
    /// the same recovery a manual restart gave, but automatic.
    pub fn run(ring: SharedRing) -> anyhow::Result<()> {
        initialize_mta().ok()?;
        loop {
            if let Err(e) = run_once(&ring) {
                eprintln!("WASAPI loopback capture lost ({e:#}); reconnecting to the current default audio device...");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn run_once(ring: &SharedRing) -> anyhow::Result<()> {
        let enumerator = DeviceEnumerator::new()?;
        // IMPORTANT: grab the Render (output/speaker) device here, NOT Capture —
        // this is what makes WASAPI treat it as a loopback request.
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut audio_client = device.get_iaudioclient()?;

        let desired_format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 2, None);
        let (_def_time, min_time) = audio_client.get_device_period()?;

        // Deliberately using POLLING instead of the event (EventsShared) mode:
        // on many drivers, the WASAPI loopback event NEVER gets signaled while
        // no audio is actually playing (idle/silent device) — this is a
        // documented WASAPI limitation, not a device bug. Waiting on the event
        // in that state would always time out even though the capture itself
        // is healthy. Polling doesn't have this problem: when the device is
        // silent, `read_from_device_to_deque` simply returns 0 new bytes and
        // the EQ bar just stays low.
        let mode = StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: min_time,
        };

        // The direction requested from the client is Capture, even though the
        // device is Render — this combination is what triggers
        // AUDCLNT_STREAMFLAGS_LOOPBACK inside the crate.
        audio_client.initialize_client(&desired_format, &Direction::Capture, &mode)?;

        let capture_client = audio_client.get_audiocaptureclient()?;
        let blockalign = desired_format.get_blockalign() as usize; // bytes per frame (2 ch x 4 byte float)
        let channels = 2usize;

        let mut byte_queue: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
        audio_client.start_stream()?;

        // Check the buffer roughly 2x more often than the device period
        // (min_time is in WASAPI's 100ns/"hns" units -> divide by 10 to get
        // microseconds), so nothing is missed/dropped without needing an
        // event handle at all.
        let period_micros = (min_time as u64 / 10).max(1);
        let poll_interval = Duration::from_micros((period_micros / 2).max(2_000));

        // Reused every iteration (`clear()`, not reallocated) to hold all the
        // mono samples from one polling pass BEFORE pushing them into the
        // shared ring buffer. Previously `push_mono_samples` (mutex lock) was
        // called for EACH single audio sample (up to 44100x/sec) — now it's
        // called once per poll, which is much cheaper on the CPU (repeated
        // mutex lock/unlock isn't free even when uncontended).
        let mut mono_batch: Vec<f32> = Vec::with_capacity(256);

        // How long reads have been failing *continuously*. A brief hiccup
        // (a frame or two) is normal and stays silent about it; but once the
        // client has been erroring for a few seconds straight, it's not
        // coming back on its own (typically `AUDCLNT_E_DEVICE_INVALIDATED`
        // after a default-device change) — bail out so the outer loop in
        // `run` re-enumerates the device and reconnects from scratch.
        const GIVE_UP_AFTER: Duration = Duration::from_secs(5);
        let mut failing_since: Option<Instant> = None;

        loop {
            // Transient errors (e.g. device briefly changing) don't
            // immediately kill the capture thread — they're logged and
            // retried, but only for a bounded amount of time (see
            // `GIVE_UP_AFTER` above): a permanently invalidated client would
            // otherwise retry forever and the EQ bars would stay silent
            // until the whole program was restarted.
            if let Err(e) = capture_client.read_from_device_to_deque(&mut byte_queue) {
                let since = *failing_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= GIVE_UP_AFTER {
                    anyhow::bail!("no successful read in over {GIVE_UP_AFTER:?} (last error: {e})");
                }
                eprintln!("WASAPI audio read temporarily failed: {e}");
                std::thread::sleep(poll_interval);
                continue;
            }
            failing_since = None;

            // Convert interleaved stereo float32 bytes -> mono f32 samples,
            // collecting them first into a local buffer (without locking the
            // mutex at all yet).
            mono_batch.clear();
            while byte_queue.len() >= blockalign {
                let mut frame_bytes = [0u8; 32]; // enough for a few float32 channels
                let frame_len = blockalign.min(frame_bytes.len());
                for b in frame_bytes.iter_mut().take(frame_len) {
                    *b = byte_queue.pop_front().unwrap();
                }
                let bytes_per_sample = 4usize;
                let mut sum = 0f32;
                for ch in 0..channels {
                    let start = ch * bytes_per_sample;
                    if start + 4 <= frame_len {
                        let v = f32::from_le_bytes([
                            frame_bytes[start],
                            frame_bytes[start + 1],
                            frame_bytes[start + 2],
                            frame_bytes[start + 3],
                        ]);
                        sum += v;
                    }
                }
                mono_batch.push(sum / channels as f32);
            }

            // Lock the mutex ONCE for this whole batch from this poll.
            if !mono_batch.is_empty() {
                push_mono_samples(ring, mono_batch.iter().copied());
            }

            std::thread::sleep(poll_interval);
        }
    }
}

/// Loopback via PulseAudio/PipeWire (`libpulse-simple-binding`). The device
/// `"@DEFAULT_MONITOR@"` is a special name resolved by the SERVER (not the
/// client) into the monitor source of the current default sink — exactly
/// what's used by `parec`/`pacat -r` from `pulseaudio-utils`. This is what
/// makes it "loopback": we capture from the OUTPUT (speaker), not the input
/// (microphone).
#[cfg(target_os = "linux")]
mod linux_loopback {
    use super::{push_mono_samples, SharedRing, SAMPLE_RATE};
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    use libpulse_simple_binding::Simple;

    pub fn run(ring: SharedRing) -> anyhow::Result<()> {
        let spec = Spec {
            format: Format::FLOAT32NE,
            channels: 2,
            rate: SAMPLE_RATE,
        };
        if !spec.is_valid() {
            anyhow::bail!("Invalid PulseAudio sample spec (channel/rate/format)");
        }

        let simple = Simple::new(
            None,                      // default server
            "trofeo-lcd",              // application name
            Direction::Record,         // direction: record...
            Some("@DEFAULT_MONITOR@"), // ...from the default sink's monitor (loopback)
            "system audio (EQ visualizer)",
            &spec,
            None, // default channel map
            None, // default buffering attr
        )
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to connect to the PulseAudio/PipeWire server ({e}) — make sure \
                 PulseAudio is running, or if you're using PipeWire make sure the \
                 'pipewire-pulse' package is installed & active"
            )
        })?;

        // 4 bytes/sample (F32NE) x 2 channels = 8 bytes/frame. ~512 frames per
        // `read()` call — a small number chosen deliberately to keep the EQ
        // bar's update latency low (similar granularity to WASAPI on
        // Windows).
        const CHANNELS: usize = 2;
        const BYTES_PER_SAMPLE: usize = 4;
        const FRAME_BYTES: usize = BYTES_PER_SAMPLE * CHANNELS;
        let mut byte_buf = vec![0u8; FRAME_BYTES * 512];
        let mut mono_batch: Vec<f32> = Vec::with_capacity(512);

        loop {
            // `read()` blocks — the server handles pacing (same as
            // `parec`), so there's NO need for a manual sleep on each
            // iteration here.
            if let Err(e) = simple.read(&mut byte_buf) {
                eprintln!("PulseAudio/PipeWire audio read temporarily failed: {e}");
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }

            mono_batch.clear();
            for frame in byte_buf.chunks_exact(FRAME_BYTES) {
                let l = f32::from_ne_bytes([frame[0], frame[1], frame[2], frame[3]]);
                let r = f32::from_ne_bytes([frame[4], frame[5], frame[6], frame[7]]);
                mono_batch.push((l + r) * 0.5);
            }
            if !mono_batch.is_empty() {
                push_mono_samples(&ring, mono_batch.iter().copied());
            }
        }
    }
}

/// Synthetic source for OSes other than Windows & Linux, so this code can
/// still be compiled & run (without real audio) there. Linux has its own
/// real path, see `linux_loopback` above.
#[cfg(not(any(windows, target_os = "linux")))]
mod fallback {
    use super::{push_mono_samples, SharedRing, SAMPLE_RATE};
    use std::f32::consts::PI;
    use std::thread::sleep;
    use std::time::Duration;

    pub fn run(ring: SharedRing) {
        let mut phase = 0f32;
        let chunk = 512usize;
        let dt = chunk as f32 / SAMPLE_RATE as f32;
        loop {
            let samples = (0..chunk).map(|i| {
                let t = phase + i as f32 / SAMPLE_RATE as f32;
                // a mix of a few tones so the EQ bar isn't flat while testing.
                0.3 * (2.0 * PI * 220.0 * t).sin()
                    + 0.2 * (2.0 * PI * 880.0 * t).sin()
                    + 0.1 * (2.0 * PI * 3000.0 * t).sin()
            });
            push_mono_samples(&ring, samples);
            phase += dt;
            sleep(Duration::from_secs_f32(dt));
        }
    }
}
