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
//!   Vulkan / OpenGL games usually don't generate these events and are left
//!   without FPS (some drivers do route them through DXGI).
//! - A Present can raise the event more than once (a runtime calling `Present` from inside
//!   `Present1`, a layer or an overlay in between): like PresentMon, only the outermost call
//!   of a thread counts (see [`PresentFilter`]).
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

/// Where the FPS comes from (`fps_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FpsSource {
    /// The Presents seen through ETW (what PresentMon counts) when there are any, else the
    /// Radeon driver's own figure.
    #[default]
    Auto,
    /// Only the ETW Presents.
    Etw,
    /// Only the Radeon driver's figure (exclusive fullscreen games).
    Driver,
}

impl FpsSource {
    pub const NAMES: &'static str = "auto | etw | driver";

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "automatic" | "automatico" => Some(FpsSource::Auto),
            "etw" | "presents" | "presentmon" => Some(FpsSource::Etw),
            "driver" | "adl" | "radeon" | "amd" => Some(FpsSource::Driver),
            _ => None,
        }
    }
}

/// The FPS to show: `driver` is the Radeon driver's figure, `etw` the one counted from the Presents.
///
/// `auto` takes the Presents, with one safety net: if they read well over the driver's own figure
/// (a test on an RX 6700 XT counted 2.1 Presents per displayed frame, with every game), the extra
/// ones are not frames and the driver's figure, which counts what is displayed, is used.
pub fn pick_fps(source: FpsSource, driver: Option<i32>, etw: Option<u32>) -> Option<i32> {
    let etw = etw.map(|f| f as i32);
    match source {
        FpsSource::Auto => match (etw, driver) {
            (Some(e), Some(d)) if d > 0 && e as f32 > d as f32 * 1.6 => Some(d),
            (e, d) => e.or(d),
        },
        FpsSource::Etw => etw,
        FpsSource::Driver => driver,
    }
}

/// The Radeon driver's FPS, steadied: its readings jump now and then (an earlier test showed
/// 134, 134, 19634, 257, 221, 134), so the figure is the median of the last few and absurd values are
/// ignored.
#[derive(Default, Debug)]
pub struct DriverFps {
    recent: std::collections::VecDeque<i32>,
}

impl DriverFps {
    const KEEP: usize = 5;
    const MAX: i32 = 1500;

    pub fn push(&mut self, v: Option<i32>) -> Option<i32> {
        match v {
            None => self.recent.clear(),
            Some(f) if (0..=Self::MAX).contains(&f) => {
                self.recent.push_back(f);
                if self.recent.len() > Self::KEEP {
                    self.recent.pop_front();
                }
            }
            Some(_) => {}
        }
        let mut s: Vec<i32> = self.recent.iter().copied().collect();
        s.sort_unstable();
        s.get(s.len() / 2).copied().filter(|f| *f > 0)
    }
}

/// One `Present_Start` (`start`) or `Present_Stop` event of the DXGI provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct PresentEvent {
    pub ts: i64,
    pub pid: u32,
    pub tid: u32,
    pub start: bool,
    /// A `Present_Start` made with `DXGI_PRESENT_TEST`: it asks whether the window is visible and
    /// shows nothing, so it is not a frame (PresentMon ignores these too).
    pub test: bool,
}

/// Counts every frame once, however many times the DXGI provider reports it.
///
/// A single `Present` can raise `Present_Start` twice in a row on one thread (the runtime calls
/// the inner `Present` from the outer one; a layer, an overlay or a driver can add one more), and
/// every one of them was counted, which reads exactly twice the real rate. PresentMon avoids it by
/// following the calls of each thread: a `Present_Start` that arrives while the thread is still
/// inside an earlier one is the same frame. Same here, with a depth per thread.
///
/// The events of a batch are not always in time order, so they are held for a quarter of a second
/// and handled sorted by their timestamps. If no `Present_Stop` has ever been seen (a system that
/// does not emit them) every start counts, as it did before.
#[derive(Default, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct PresentFilter {
    pending: Vec<PresentEvent>,
    newest: i64,
    /// Per thread: how many Presents it is inside, and the time of its last event.
    threads: std::collections::HashMap<u32, (u32, i64)>,
    stops_seen: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl PresentFilter {
    /// Takes an event; returns the frames (process, time) that are now known to be real.
    pub fn feed(&mut self, e: PresentEvent, freq: i64) -> Vec<(u32, i64)> {
        self.newest = self.newest.max(e.ts);
        // known as soon as it arrives, so the very first frame is already judged with it
        if !e.start {
            self.stops_seen = true;
        }
        let at = self.pending.partition_point(|x| x.ts <= e.ts);
        self.pending.insert(at, e);
        let limit = self.newest - freq / 4;
        let n = self.pending.partition_point(|x| x.ts <= limit);
        let ready: Vec<PresentEvent> = self.pending.drain(..n).collect();
        let mut out = Vec::new();
        for ev in ready {
            if self.step(ev, freq) {
                out.push((ev.pid, ev.ts));
            }
        }
        out
    }

    /// Whether this event is a new frame.
    fn step(&mut self, e: PresentEvent, freq: i64) -> bool {
        let st = self.threads.entry(e.tid).or_insert((0, e.ts));
        // a Stop that never came (a thread that ended inside a Present): do not stay "inside" forever
        if st.0 > 0 && e.ts - st.1 > freq {
            st.0 = 0;
        }
        st.1 = e.ts;
        if e.start {
            let counts = !e.test && (st.0 == 0 || !self.stops_seen);
            st.0 += 1;
            counts
        } else {
            st.0 = st.0.saturating_sub(1);
            false
        }
    }
}

/// What the DXGI provider reported about one process (`--fps-diag`).
#[derive(Default, Clone, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct DiagRow {
    /// Event id -> how many (42 = Present_Start, 43 = Present_Stop).
    pub ids: std::collections::BTreeMap<u16, u32>,
    pub threads: std::collections::BTreeSet<u32>,
    pub chains: std::collections::BTreeSet<u64>,
    pub raw_starts: u32,
    /// Frames left after the test presents and the nested calls were removed.
    pub counted: u32,
    /// Present_Start events made with `DXGI_PRESENT_TEST`.
    pub tests: u32,
    /// `Flags` argument of Present -> how many.
    pub flags: std::collections::BTreeMap<u32, u32>,
    /// `SyncInterval` argument -> how many.
    pub syncs: std::collections::BTreeMap<u32, u32>,
    /// Thread -> Present_Starts it made.
    pub thread_starts: std::collections::BTreeMap<u32, u32>,
}

#[cfg(windows)]
mod imp {
    use super::{DiagRow, PresentEvent, PresentFilter, PresentWindow, Status};
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
    const DXGI_PRESENT_STOP: u16 = 43;
    /// `DXGI_PRESENT_TEST`: the Present only checks the window, it draws nothing.
    const DXGI_PRESENT_TEST: u32 = 1;

    static PRESENTS: OnceLock<Mutex<HashMap<u32, PresentWindow>>> = OnceLock::new();
    static FILTER: OnceLock<Mutex<PresentFilter>> = OnceLock::new();
    static DIAG: OnceLock<Mutex<Option<HashMap<u32, DiagRow>>>> = OnceLock::new();
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
        let r = &*rec;
        let h = &r.EventHeader;
        if h.ProviderId != DXGI_PROVIDER {
            return;
        }
        let id = h.EventDescriptor.Id;
        // The arguments of a Present_Start: the swap chain (a pointer), Flags, SyncInterval.
        let (mut chain, mut flags, mut sync) = (0u64, 0u32, 0u32);
        if id == DXGI_PRESENT_START && !r.UserData.is_null() {
            let ptr = if h.Flags & 0x20 != 0 { 4usize } else { 8usize }; // EVENT_HEADER_FLAG_32_BIT_HEADER
            let len = r.UserDataLength as usize;
            let data = std::slice::from_raw_parts(r.UserData as *const u8, len);
            if len >= ptr {
                let mut b = [0u8; 8];
                b[..ptr].copy_from_slice(&data[..ptr]);
                chain = u64::from_le_bytes(b);
            }
            if len >= ptr + 4 {
                flags = u32::from_le_bytes([data[ptr], data[ptr + 1], data[ptr + 2], data[ptr + 3]]);
            }
            if len >= ptr + 8 {
                sync = u32::from_le_bytes([data[ptr + 4], data[ptr + 5], data[ptr + 6], data[ptr + 7]]);
            }
        }
        let test = id == DXGI_PRESENT_START && flags & DXGI_PRESENT_TEST != 0;
        // for `--fps-diag`: everything the provider says, by process
        if let Some(Ok(mut d)) = DIAG.get().map(|m| m.lock()) {
            if let Some(map) = d.as_mut() {
                let row = map.entry(h.ProcessId).or_default();
                *row.ids.entry(id).or_insert(0) += 1;
                row.threads.insert(h.ThreadId);
                if id == DXGI_PRESENT_START {
                    row.raw_starts += 1;
                    row.chains.insert(chain);
                    *row.flags.entry(flags).or_insert(0) += 1;
                    *row.syncs.entry(sync).or_insert(0) += 1;
                    *row.thread_starts.entry(h.ThreadId).or_insert(0) += 1;
                    if test {
                        row.tests += 1;
                    }
                }
            }
        }
        if id != DXGI_PRESENT_START && id != DXGI_PRESENT_STOP {
            return;
        }
        // the time the Present happened (the trace is set up with the performance counter as its
        // clock), not the time this callback runs: the events come in batches
        let ev = PresentEvent { ts: h.TimeStamp, pid: h.ProcessId, tid: h.ThreadId, start: id == DXGI_PRESENT_START, test };
        let frames = match FILTER.get_or_init(Default::default).lock() {
            Ok(mut f) => f.feed(ev, freq()),
            Err(_) => return,
        };
        if frames.is_empty() {
            return;
        }
        if let Ok(mut map) = presents().lock() {
            for (pid, ts) in &frames {
                map.entry(*pid).or_default().push(*ts, freq());
            }
        }
        if let Some(Ok(mut d)) = DIAG.get().map(|m| m.lock()) {
            if let Some(map) = d.as_mut() {
                for (pid, _) in &frames {
                    map.entry(*pid).or_default().counted += 1;
                }
            }
        }
    }

    /// Starts collecting what the provider reports (for `--fps-diag`).
    pub fn diag_begin() {
        *DIAG.get_or_init(Default::default).lock().unwrap() = Some(HashMap::new());
    }

    pub fn diag_snapshot() -> Vec<(u32, DiagRow)> {
        match DIAG.get().and_then(|m| m.lock().ok()) {
            Some(d) => d.as_ref().map(|m| m.iter().map(|(k, v)| (*k, v.clone())).collect()).unwrap_or_default(),
            None => Vec::new(),
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
    pub fn diag_begin() {}
    pub fn diag_snapshot() -> Vec<(u32, super::DiagRow)> {
        Vec::new()
    }
}

pub use imp::{diag_begin, diag_snapshot, fps_for_pid, start, status};

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

    use super::{pick_fps, FpsSource, PresentEvent, PresentFilter};

    fn ev(ts: i64, tid: u32, start: bool) -> PresentEvent {
        PresentEvent { ts, pid: 7, tid, start, test: false }
    }

    /// Runs events through a filter, then flushes it with a late event; returns the frames' times.
    fn frames_of(events: Vec<PresentEvent>) -> Vec<i64> {
        let mut f = PresentFilter::default();
        let mut out = Vec::new();
        for e in events {
            out.extend(f.feed(e, FREQ).into_iter().map(|(_, t)| t));
        }
        // an event far in the future releases everything held before it
        out.extend(f.feed(ev(1_000 * FREQ, 999, true), FREQ).into_iter().map(|(_, t)| t));
        out
    }

    #[test]
    fn a_present_that_reports_twice_counts_once() {
        // 100 frames at 100 fps; each: start, nested start, nested stop, stop
        let mut v = Vec::new();
        for i in 0..100i64 {
            let t = i * FREQ / 100;
            v.extend([ev(t, 1, true), ev(t + 10, 1, true), ev(t + 500, 1, false), ev(t + 900, 1, false)]);
        }
        let frames = frames_of(v);
        assert_eq!(frames.len(), 100);
        let mut w = PresentWindow::default();
        for t in frames {
            w.push(t, FREQ);
        }
        assert_eq!(w.fps(FREQ, FREQ), Some(100));
    }

    #[test]
    fn plain_frames_and_two_threads_all_count() {
        let mut v = Vec::new();
        for i in 0..60i64 {
            let t = i * FREQ / 60;
            v.extend([ev(t, 1, true), ev(t + 100, 1, false)]);
        }
        // a second thread presenting on its own (two swap chains): its frames count too
        for i in 0..30i64 {
            let t = i * FREQ / 30 + 5;
            v.extend([ev(t, 2, true), ev(t + 100, 2, false)]);
        }
        v.sort_by_key(|e| e.ts);
        assert_eq!(frames_of(v).len(), 90);
    }

    #[test]
    fn events_out_of_order_are_put_in_order_first() {
        // a Stop that reaches the program before its own Start, and a nested Start after it
        let v = vec![ev(100, 1, true), ev(400, 1, false), ev(FREQ, 1, true), ev(FREQ + 5, 1, true), ev(FREQ + 300, 1, false), ev(FREQ + 400, 1, false)];
        let mut shuffled = v.clone();
        shuffled.swap(0, 1);
        shuffled.swap(4, 5);
        assert_eq!(frames_of(v).len(), 2);
        assert_eq!(frames_of(shuffled).len(), 2);
    }

    #[test]
    fn without_any_stop_every_start_counts_as_before() {
        let v: Vec<PresentEvent> = (0..50i64).map(|i| ev(i * FREQ / 50, 1, true)).collect();
        assert_eq!(frames_of(v).len(), 50);
    }

    #[test]
    fn a_lost_stop_does_not_silence_the_thread_for_good() {
        // frame 1 never gets its Stop; a stop has been seen elsewhere, so nesting is on
        let mut v = vec![ev(0, 2, true), ev(10, 2, false), ev(100, 1, true)];
        // one second later the thread presents again, with Stops
        for i in 0..20i64 {
            let t = 3 * FREQ + i * FREQ / 20;
            v.extend([ev(t, 1, true), ev(t + 50, 1, false)]);
        }
        let n = frames_of(v).len();
        assert_eq!(n, 22, "the lost-stop frame, the other thread's frame and the 20 later ones");
    }

    #[test]
    fn the_source_decides_which_figure_is_shown() {
        assert_eq!(pick_fps(FpsSource::Auto, Some(288), Some(144)), Some(144));
        assert_eq!(pick_fps(FpsSource::Auto, Some(90), None), Some(90));
        // the Presents read double the driver's figure: the driver's is the real one
        assert_eq!(pick_fps(FpsSource::Auto, Some(134), Some(265)), Some(134));
        // frame generation: the driver shows more than the game presents; keep the Presents
        assert_eq!(pick_fps(FpsSource::Auto, Some(240), Some(120)), Some(120));
        // close enough: the Presents
        assert_eq!(pick_fps(FpsSource::Auto, Some(134), Some(125)), Some(125));
        assert_eq!(pick_fps(FpsSource::Etw, Some(134), Some(265)), Some(265));
        assert_eq!(pick_fps(FpsSource::Etw, Some(288), None), None);
        assert_eq!(pick_fps(FpsSource::Driver, Some(288), Some(144)), Some(288));
        assert_eq!(FpsSource::parse("ETW"), Some(FpsSource::Etw));
        assert_eq!(FpsSource::parse("adl"), Some(FpsSource::Driver));
        assert_eq!(FpsSource::parse("x"), None);
    }

    use super::DriverFps;

    #[test]
    fn test_presents_are_not_frames_but_still_count_as_a_call_in_progress() {
        // every frame: a Present with the TEST flag (checks the window), then the real one
        let mut v = Vec::new();
        for i in 0..60i64 {
            let t = i * FREQ / 60;
            v.extend([
                PresentEvent { ts: t, pid: 7, tid: 1, start: true, test: true },
                ev(t + 10, 1, false),
                ev(t + 20, 1, true),
                ev(t + 300, 1, false),
            ]);
        }
        assert_eq!(frames_of(v).len(), 60);
        // a test Present that wraps the real one (nested): still one frame
        let v = vec![
            PresentEvent { ts: 0, pid: 7, tid: 1, start: true, test: true },
            ev(5, 1, true),
            ev(50, 1, false),
            ev(60, 1, false),
        ];
        assert_eq!(frames_of(v).len(), 0, "the outer call is a test, the inner one is nested in it");
    }

    #[test]
    fn the_driver_figure_ignores_jumps() {
        let mut d = DriverFps::default();
        let mut shown = Vec::new();
        for v in [134, 135, 19634, 134, 257, 134, 135, 134] {
            shown.push(d.push(Some(v)).unwrap());
        }
        assert!(shown.iter().skip(2).all(|f| (134..=135).contains(f)), "{shown:?}");
        assert_eq!(d.push(None), None);
        assert_eq!(d.push(Some(0)), None, "zero means no game");
    }
}
