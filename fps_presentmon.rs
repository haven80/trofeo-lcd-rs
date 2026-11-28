//! PresentMon command-line fallback for games whose DXGI events are not visible
//! to Trofeo's lightweight native ETW listener.
//!
//! The official PresentMon console executable is placed at tools/PresentMon.exe
//! by instalar_e_rodar.ps1. It is launched only for the current foreground PID,
//! and only used when the native ETW source has no valid reading.

#[cfg(windows)]
mod imp {
    use std::{
        collections::VecDeque,
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SAMPLE_WINDOW: Duration = Duration::from_millis(1200);
    const STALE_AFTER: Duration = Duration::from_secs(2);
    const RETRY_AFTER: Duration = Duration::from_secs(5);

    #[derive(Default)]
    struct Samples {
        values: VecDeque<(Instant, f64)>,
    }

    impl Samples {
        fn push(&mut self, ms_between: f64) {
            if !ms_between.is_finite() || !(0.2..=2000.0).contains(&ms_between) {
                return;
            }
            let now = Instant::now();
            self.values.push_back((now, ms_between));
            self.trim(now);
        }

        fn trim(&mut self, now: Instant) {
            while self.values.front().is_some_and(|(at, _)| now.duration_since(*at) > SAMPLE_WINDOW) {
                self.values.pop_front();
            }
            while self.values.len() > 2400 {
                self.values.pop_front();
            }
        }

        fn fps(&mut self) -> Option<u32> {
            let now = Instant::now();
            self.trim(now);
            let (newest, _) = self.values.back()?;
            if now.duration_since(*newest) > STALE_AFTER || self.values.len() < 3 {
                return None;
            }
            let mean_ms = self.values.iter().map(|(_, ms)| *ms).sum::<f64>() / self.values.len() as f64;
            let fps = (1000.0 / mean_ms).round() as u32;
            (1..=1000).contains(&fps).then_some(fps)
        }

        fn clear(&mut self) {
            self.values.clear();
        }
    }

    pub struct PresentMonMonitor {
        child: Option<Child>,
        pid: Option<u32>,
        samples: Arc<Mutex<Samples>>,
        last_attempt: Option<Instant>,
        executable: Option<PathBuf>,
    }

    impl Default for PresentMonMonitor {
        fn default() -> Self {
            Self {
                child: None,
                pid: None,
                samples: Arc::new(Mutex::new(Samples::default())),
                last_attempt: None,
                executable: None,
            }
        }
    }

    impl PresentMonMonitor {
        pub fn new() -> Self { Self::default() }

        fn find_executable() -> Option<PathBuf> {
            let mut candidates = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    for ancestor in dir.ancestors().take(4) {
                        candidates.push(ancestor.join("tools").join("PresentMon.exe"));
                    }
                    candidates.push(dir.join("PresentMon.exe"));
                }
            }
            if let Ok(dir) = std::env::current_dir() {
                candidates.push(dir.join("tools").join("PresentMon.exe"));
                candidates.push(dir.join("PresentMon.exe"));
            }
            candidates.into_iter().find(|p| p.is_file())
        }

        fn stop_child(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        fn start_for_pid(&mut self, pid: u32) {
            self.last_attempt = Some(Instant::now());
            let Some(path) = self.executable.clone().or_else(Self::find_executable) else {
                eprintln!("FPS PresentMon: tools\\PresentMon.exe nao encontrado. Execute instalar_e_rodar.bat com internet para baixar a ferramenta oficial.");
                return;
            };
            self.executable = Some(path.clone());

            let session = format!("TrofeoLCD_FPS_{pid}");
            let spawned = Command::new(&path)
                .arg("--process_id").arg(pid.to_string())
                .arg("--output_stdout")
                .arg("--no_console_stats")
                .arg("--exclude_dropped")
                .arg("--session_name").arg(&session)
                .arg("--stop_existing_session")
                .arg("--terminate_on_proc_exit")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();

            match spawned {
                Ok(mut child) => {
                    let Some(stdout) = child.stdout.take() else {
                        let _ = child.kill();
                        eprintln!("FPS PresentMon: nao foi possivel abrir a saida CSV.");
                        return;
                    };
                    if let Some(stderr) = child.stderr.take() {
                        std::thread::spawn(move || {
                            for line in BufReader::new(stderr).lines().flatten() {
                                eprintln!("PresentMon: {line}");
                            }
                        });
                    }
                    let samples = Arc::clone(&self.samples);
                    std::thread::spawn(move || {
                        let reader = BufReader::new(stdout);
                        let mut interval_col: Option<usize> = None;
                        for line in reader.lines().flatten() {
                            if interval_col.is_none() {
                                let headers: Vec<&str> = line.trim_start_matches('\u{feff}').split(',').collect();
                                if let Some(i) = headers.iter().position(|h| h.trim_matches('"').eq_ignore_ascii_case("MsBetweenPresents")) {
                                    interval_col = Some(i);
                                }
                                continue;
                            }
                            let Some(i) = interval_col else { continue };
                            let fields: Vec<&str> = line.split(',').collect();
                            let Some(raw) = fields.get(i) else { continue };
                            if let Ok(ms) = raw.trim().trim_matches('"').parse::<f64>() {
                                if let Ok(mut s) = samples.lock() {
                                    s.push(ms);
                                }
                            }
                        }
                    });
                    self.child = Some(child);
                    eprintln!("FPS PresentMon: captura alternativa iniciada para PID {pid}.");
                }
                Err(e) => {
                    eprintln!("FPS PresentMon: falha ao iniciar {}: {e}", path.display());
                }
            }
        }

        /// Returns a smoothed FPS value for the foreground PID. The child process
        /// is restarted only when the foreground PID changes or the capture exits.
        pub fn sample(&mut self, pid: u32) -> Option<u32> {
            if self.pid != Some(pid) {
                self.stop_child();
                self.pid = Some(pid);
                self.last_attempt = None;
                if let Ok(mut s) = self.samples.lock() { s.clear(); }
            }

            let exited = self.child.as_mut().and_then(|c| c.try_wait().ok().flatten()).is_some();
            if exited {
                self.child = None;
                self.last_attempt = Some(Instant::now());
                eprintln!("FPS PresentMon: processo de captura encerrou; nova tentativa em breve.");
            }

            if self.child.is_none()
                && self.last_attempt.is_none_or(|t| t.elapsed() >= RETRY_AFTER)
            {
                self.start_for_pid(pid);
            }

            self.samples.lock().ok()?.fps()
        }

        pub fn stop(&mut self) {
            self.stop_child();
            self.pid = None;
            self.last_attempt = None;
            if let Ok(mut s) = self.samples.lock() { s.clear(); }
        }
    }

    impl Drop for PresentMonMonitor {
        fn drop(&mut self) { self.stop_child(); }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct PresentMonMonitor;
    impl PresentMonMonitor {
        pub fn new() -> Self { Self }
        pub fn sample(&mut self, _pid: u32) -> Option<u32> { None }
        pub fn stop(&mut self) {}
    }
}

pub use imp::PresentMonMonitor;
