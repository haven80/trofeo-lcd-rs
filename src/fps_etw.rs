//! FPS of the foreground game, for ANY GPU (NVIDIA/AMD/Intel), via ETW:
//! it listens for DXGI `Present` events (provider Microsoft-Windows-DXGI,
//! event 42 = Present_Start) — the same source PresentMon uses — and counts
//! Presents per second for the process that owns the foreground window.
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

#[cfg(windows)]
mod imp {
    use super::Status;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use windows::core::{GUID, PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
    use windows::Win32::System::Diagnostics::Etw::*;

    const SESSION_NAME: &str = "TrofeoLcdFpsSession";
    /// Microsoft-Windows-DXGI
    const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
    const DXGI_PRESENT_START: u16 = 42;
    const WINDOW: Duration = Duration::from_millis(1000);

    static PRESENTS: OnceLock<Mutex<HashMap<u32, VecDeque<Instant>>>> = OnceLock::new();
    static STATUS: AtomicU8 = AtomicU8::new(0);

    fn presents() -> &'static Mutex<HashMap<u32, VecDeque<Instant>>> {
        PRESENTS.get_or_init(|| Mutex::new(HashMap::new()))
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
        let now = Instant::now();
        if let Ok(mut map) = presents().lock() {
            let q = map.entry(h.ProcessId).or_default();
            q.push_back(now);
            while q.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
                q.pop_front();
            }
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

    /// FPS (Presents/s in the last second) for process `pid`; `None` if too little data.
    pub fn fps_for_pid(pid: u32) -> Option<u32> {
        let now = Instant::now();
        let map = presents().lock().ok()?;
        let q = map.get(&pid)?;
        let n = q.iter().filter(|t| now.duration_since(**t) <= WINDOW).count() as u32;
        (n >= 5).then_some(n)
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
