//! FPS of the foreground game, for ANY GPU (NVIDIA/AMD/Intel), via ETW:
//! it listens for DXGI `Present` events (provider Microsoft-Windows-DXGI,
//! event 42 = Present_Start) — the same source PresentMon uses — and counts
//! Presents per second for the process that owns the foreground window.
//! Each Present is dated with the event's OWN timestamp (QPC), not with the
//! moment the callback runs: ETW delivers real-time events in batches of
//! about a second, and dating them on arrival doubled the reading (2.9.4).
//!
//! Honest notes:
//! - The program needs to run **as administrator** (real-time ETW).
//! - Covers DirectX 9/10/11/12 (anything that goes through DXGI). Native
//!   Vulkan / OpenGL games don't generate these events and will be left
//!   without FPS.
//! - Windows only; on other systems `start()` returns an error.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotStarted,
    Running,
    /// ETW couldn't be started (usually: missing administrator access).
    NeedsAdmin,
    Failed,
}

/// The Presents of one process, by the time they HAPPENED (the timestamp of the event, in ticks of
/// a clock that runs `freq` ticks a second), not by the time the program got to hear of them.
///
/// Windows hands the events of a real-time trace over in batches, about once a second. Dating
/// them with the clock of the moment of delivery piled a whole second of Presents on one
/// instant, and a "last second" window that holds two batches reads up to twice the real FPS
/// (right at the start there is only one batch, so the first reading was right and then it doubled).
#[derive(Default, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct PresentWindow {
    ts: std::collections::VecDeque<i64>,
    newest: i64,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl PresentWindow {
    /// How long after its last Present a process still has an FPS reading, in seconds. Batches
    /// of events arrive up to about a second late, so it is a bit more than that.
    const STALE_SECS: i64 = 3;

    pub fn push(&mut self, t: i64, freq: i64) {
        self.ts.push_back(t);
        self.newest = self.newest.max(t);
        // keep the second before the newest event
        while self.ts.front().is_some_and(|f| self.newest - *f > freq) {
            self.ts.pop_front();
        }
    }

    /// Presents per second over the last second of events; `None` when there are too few
    /// events, or the last one is old (the game stopped presenting, or the window is not shown).
    pub fn fps(&self, now: i64, freq: i64) -> Option<u32> {
        if freq <= 0 || self.ts.is_empty() || now - self.newest > freq * Self::STALE_SECS {
            return None;
        }
        let from = self.newest - freq;
        let mut n = 0u32;
        let mut first = self.newest;
        for &t in self.ts.iter().filter(|t| **t >= from) {
            n += 1;
            first = first.min(t);
        }
        let span = self.newest - first;
        if n < 5 || span <= 0 {
            return None;
        }
        // n events = n - 1 intervals between the first and the last
        Some(((n - 1) as f64 * freq as f64 / span as f64).round() as u32)
    }
}

#[cfg(windows)]
mod imp {
    use super::{PresentWindow, Status};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    use windows::core::{GUID, PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
    use windows::Win32::System::Diagnostics::Etw::*;

    const SESSION_NAME: &str = "TrofeoLcdFpsSession";
    /// Microsoft-Windows-DXGI
    const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
    const DXGI_PRESENT_START: u16 = 42;

    static PRESENTS: OnceLock<Mutex<HashMap<u32, PresentWindow>>> = OnceLock::new();
    static STATUS: AtomicU8 = AtomicU8::new(0);
    static FREQ: OnceLock<i64> = OnceLock::new();

    fn presents() -> &'static Mutex<HashMap<u32, PresentWindow>> {
        PRESENTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Ticks per second of the clock of the trace (the performance counter, see `ClientContext`).
    fn freq() -> i64 {
        *FREQ.get_or_init(|| {
            let mut f = 0i64;
            unsafe {
                let _ = QueryPerformanceFrequency(&mut f);
            }
            f.max(1)
        })
    }

    fn qpc_now() -> i64 {
        let mut t = 0i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut t);
        }
        t
    }

    pub fn status() -> Status {
        match STATUS.load(Ordering::Relaxed) {
            1 => Status::Running,
            2 => Status::NeedsAdmin,
            3 => Status::Failed,
            _ => Status::NotStarted,
        }
    }

    unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
        if rec.is_null() {
            return;
        }
        let h = &(*rec).EventHeader;
        if h.ProviderId != DXGI_PROVIDER || h.EventDescriptor.Id != DXGI_PRESENT_START {
            return;
        }
        // the time the Present happened (the trace is set up with the performance counter as its
        // clock), not the time this callback runs: the events come in batches
        if let Ok(mut map) = presents().lock() {
            map.entry(h.ProcessId).or_default().push(h.TimeStamp, freq());
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// EVENT_TRACE_PROPERTIES buffer + session name, 8-byte aligned.
    fn make_props() -> Vec<u64> {
        let name_bytes = (SESSION_NAME.len() + 1) * 2;
        let total = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + name_bytes + 64;
        let mut buf = vec![0u64; total.div_ceil(8)];
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        unsafe {
            (*p).Wnode.BufferSize = (buf.len() * 8) as u32;
            (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            (*p).Wnode.ClientContext = 1; // QPC
            (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            (*p).LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        }
        buf
    }

    pub fn start() -> Result<(), String> {
        let name = wide(SESSION_NAME);
        let mut props = make_props();
        let mut handle = CONTROLTRACE_HANDLE::default();
        unsafe {
            let mut err = StartTraceW(
                &mut handle,
                PCWSTR(name.as_ptr()),
                props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
            );
            if err == ERROR_ALREADY_EXISTS {
                // A session left over from a previous run (crash): stop it and retry.
                let mut old = make_props();
                let _ = ControlTraceW(
                    CONTROLTRACE_HANDLE::default(),
                    PCWSTR(name.as_ptr()),
                    old.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                    EVENT_TRACE_CONTROL_STOP,
                );
                props = make_props();
                err = StartTraceW(
                    &mut handle,
                    PCWSTR(name.as_ptr()),
                    props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                );
            }
            if err.0 != 0 {
                STATUS.store(if err.0 == 5 { 2 } else { 3 }, Ordering::Relaxed);
                return Err(format!("StartTrace error {} (5 = administrator required)", err.0));
            }

            let err = EnableTraceEx2(
                handle,
                &DXGI_PROVIDER,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                5, // TRACE_LEVEL_VERBOSE
                u64::MAX,
                0,
                0,
                None,
            );
            if err.0 != 0 {
                STATUS.store(3, Ordering::Relaxed);
                return Err(format!("EnableTraceEx2 error {}", err.0));
            }
        }

        let logger = wide(SESSION_NAME);
        std::thread::spawn(move || unsafe {
            let mut logger = logger;
            let mut file: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            file.LoggerName = PWSTR(logger.as_mut_ptr());
            file.Anonymous1.ProcessTraceMode =
                PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
            file.Anonymous2.EventRecordCallback = Some(on_event);
            let th = OpenTraceW(&mut file);
            if th.Value == u64::MAX {
                STATUS.store(3, Ordering::Relaxed);
                eprintln!("FPS: OpenTrace failed");
                return;
            }
            let _ = ProcessTrace(&[th], None, None); // blocks while the session is alive
            STATUS.store(3, Ordering::Relaxed);
        });
        STATUS.store(1, Ordering::Relaxed);
        Ok(())
    }

    /// FPS (Presents/s over the last second of events) for process `pid`; `None` if too little data.
    pub fn fps_for_pid(pid: u32) -> Option<u32> {
        let map = presents().lock().ok()?;
        map.get(&pid)?.fps(qpc_now(), freq())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Status;
    pub fn status() -> Status {
        Status::NotStarted
    }
    pub fn start() -> Result<(), String> {
        Err("FPS via ETW is only available on Windows".into())
    }
    pub fn fps_for_pid(_pid: u32) -> Option<u32> {
        None
    }
}

pub use imp::{fps_for_pid, start, status};

#[cfg(test)]
mod tests {
    use super::PresentWindow;

    const FREQ: i64 = 10_000_000; // a typical QueryPerformanceFrequency

    /// `fps` events a second for `secs` seconds, as timestamps.
    fn events(fps: i64, secs: i64) -> Vec<i64> {
        (0..fps * secs).map(|i| i * FREQ / fps).collect()
    }

    #[test]
    fn the_fps_is_the_real_rate_whatever_the_batches_of_delivery() {
        for fps in [30i64, 60, 144, 240] {
            // the events reach the program in batches of 1 s; at every "now" after a batch the
            // reading must stay the real rate (it read up to twice as much before)
            for batch in 1..=5i64 {
                let mut w = PresentWindow::default();
                for t in events(fps, batch).into_iter() {
                    w.push(t, FREQ);
                }
                // right after the batch, a little later, and just before the next one
                for late in [0, FREQ / 4, FREQ / 2, FREQ * 9 / 10] {
                    let now = batch * FREQ + late;
                    let got = w.fps(now, FREQ).expect("a reading") as i64;
                    assert!((got - fps).abs() <= 1, "{fps} fps, batch {batch}, {late} late: read {got}");
                }
            }
        }
    }

    #[test]
    fn the_reading_goes_away_when_the_game_stops_and_needs_a_few_events() {
        let mut w = PresentWindow::default();
        for t in events(60, 2) {
            w.push(t, FREQ);
        }
        assert!(w.fps(2 * FREQ, FREQ).is_some());
        // nothing new for more than the stale time: no reading
        assert_eq!(w.fps(2 * FREQ + 4 * FREQ, FREQ), None);
        // too few events
        let mut few = PresentWindow::default();
        for t in events(60, 1).into_iter().take(4) {
            few.push(t, FREQ);
        }
        assert_eq!(few.fps(FREQ, FREQ), None);
        assert_eq!(PresentWindow::default().fps(0, FREQ), None);
    }

    #[test]
    fn a_changing_rate_is_followed_and_old_events_are_dropped() {
        let mut w = PresentWindow::default();
        for t in events(60, 3) {
            w.push(t, FREQ);
        }
        // then the game runs at 120 for three seconds
        let start = 3 * FREQ;
        for i in 0..360 {
            w.push(start + i * FREQ / 120, FREQ);
        }
        let got = w.fps(6 * FREQ, FREQ).unwrap() as i64;
        assert!((got - 120).abs() <= 1, "read {got}");
        assert!(w.ts.len() <= 125, "{} events kept", w.ts.len());
    }

    #[test]
    fn events_a_little_out_of_order_do_not_confuse_it() {
        let mut ev = events(100, 2);
        ev.swap(50, 51);
        ev.swap(120, 122);
        let mut w = PresentWindow::default();
        for t in ev {
            w.push(t, FREQ);
        }
        let got = w.fps(2 * FREQ, FREQ).unwrap() as i64;
        assert!((got - 100).abs() <= 1, "read {got}");
    }
}
