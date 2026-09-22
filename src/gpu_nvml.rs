//! NVIDIA GPU temperature and power. Two paths, in order:
//! 1. **NVML** (the GeForce driver's `nvml.dll`: System32, NVSMI or DriverStore),
//!    loaded dynamically;
//! 2. **nvidia-smi** (`nvidia-smi.exe`, always installed with the driver), launched
//!    in the background without a window every ~1.5 s.
//!
//! `NvSensor::new()` picks automatically and reports in `describe()` what it's
//! using (or why NVML didn't start). Without an NVIDIA driver it returns `None`.
//! Windows only.

#[cfg(windows)]
mod imp {
    use std::ffi::{c_void, CString};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    type InitFn = unsafe extern "C" fn() -> i32;
    type HandleFn = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
    type TempFn = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
    type PowerFn = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;
    type ClockFn = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
    /// (temperature °C, power W, core clock MHz)
    pub type Reading = (Option<i32>, Option<i32>, Option<i32>);
    /// `nvmlTemperature_t` (newer NVML / RTX 50): { version, sensorType, temperature }.
    #[repr(C)]
    struct TempV {
        version: u32,
        sensor_type: u32,
        temperature: i32,
    }
    type TempVFn = unsafe extern "C" fn(*mut c_void, *mut TempV) -> i32;
    /// NVML_STRUCT_VERSION(Temperature, 1) = sizeof | (1 << 24)
    const TEMP_V1: u32 = 12 | (1 << 24);

    struct Nvml {
        clock: Option<ClockFn>,
        dev: *mut c_void,
        temp: TempFn,
        temp_v: Option<TempVFn>,
        power: PowerFn,
    }

    fn candidate_paths() -> Vec<String> {
        let mut v = vec![
            "nvml.dll".to_string(),
            "C:\\Windows\\System32\\nvml.dll".to_string(),
            "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll".to_string(),
        ];
        // DriverStore: C:\Windows\System32\DriverStore\FileRepository\nv*\nvml.dll
        if let Ok(rd) = std::fs::read_dir("C:\\Windows\\System32\\DriverStore\\FileRepository") {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with("nv") {
                    let p = e.path().join("nvml.dll");
                    if p.is_file() {
                        v.push(p.to_string_lossy().into_owned());
                    }
                }
            }
        }
        v
    }

    impl Nvml {
        fn new() -> Result<Self, String> {
            unsafe {
                let mut module = None;
                for path in candidate_paths() {
                    let c = CString::new(path).map_err(|e| e.to_string())?;
                    if let Ok(m) = LoadLibraryA(PCSTR(c.as_ptr() as *const u8)) {
                        module = Some(m);
                        break;
                    }
                }
                let module = module.ok_or("nvml.dll not found")?;
                let sym = |name: &str| {
                    let c = CString::new(name).ok()?;
                    GetProcAddress(module, PCSTR(c.as_ptr() as *const u8))
                };
                let init: InitFn = std::mem::transmute(sym("nvmlInit_v2").ok_or("nvmlInit_v2 missing")?);
                let get_handle: HandleFn = std::mem::transmute(
                    sym("nvmlDeviceGetHandleByIndex_v2").ok_or("nvmlDeviceGetHandleByIndex_v2 missing")?,
                );
                let temp: TempFn =
                    std::mem::transmute(sym("nvmlDeviceGetTemperature").ok_or("nvmlDeviceGetTemperature missing")?);
                let power: PowerFn =
                    std::mem::transmute(sym("nvmlDeviceGetPowerUsage").ok_or("nvmlDeviceGetPowerUsage missing")?);
                let temp_v: Option<TempVFn> =
                    sym("nvmlDeviceGetTemperatureV").map(|f| std::mem::transmute(f));
                let clock: Option<ClockFn> = sym("nvmlDeviceGetClockInfo").map(|f| std::mem::transmute(f));
                let rc = init();
                if rc != 0 {
                    return Err(format!("nvmlInit error {rc}"));
                }
                let mut dev: *mut c_void = std::ptr::null_mut();
                let rc = get_handle(0, &mut dev);
                if rc != 0 || dev.is_null() {
                    return Err(format!("nvmlDeviceGetHandleByIndex error {rc}"));
                }
                let n = Nvml { clock, dev, temp, temp_v, power };
                // Real test: if even the first reading fails, better to fall back to nvidia-smi.
                if n.sample().0.is_none() {
                    return Err("NVML did not return a temperature".into());
                }
                Ok(n)
            }
        }

        fn sample(&self) -> Reading {
            unsafe {
                let mut t = 0u32;
                let mut p = 0u32;
                let mut temp = ((self.temp)(self.dev, 0, &mut t) == 0).then_some(t as i32);
                if temp.is_none() {
                    // RTX 50 / recent NVML: the classic function doesn't respond, the "V" version is needed.
                    if let Some(f) = self.temp_v {
                        let mut tv = TempV { version: TEMP_V1, sensor_type: 0, temperature: 0 };
                        if f(self.dev, &mut tv) == 0 {
                            temp = Some(tv.temperature);
                        }
                    }
                }
                let power = ((self.power)(self.dev, &mut p) == 0).then_some((p / 1000) as i32);
                let mut c = 0u32;
                let clock = self
                    .clock
                    .and_then(|f| (f(self.dev, 0, &mut c) == 0).then_some(c as i32));
                (temp, power, clock)
            }
        }
    }

    type Shared = Arc<Mutex<Reading>>;

    fn smi_path() -> PathBuf {
        for p in [
            "C:\\Windows\\System32\\nvidia-smi.exe",
            "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvidia-smi.exe",
        ] {
            if std::path::Path::new(p).is_file() {
                return PathBuf::from(p);
            }
        }
        PathBuf::from("nvidia-smi")
    }

    fn run_smi() -> Option<Reading> {
        let out = Command::new(smi_path())
            .args(["--query-gpu=temperature.gpu,power.draw,clocks.gr", "--format=csv,noheader,nounits", "-i", "0"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut it = text.lines().next()?.split(',');
        let temp = it.next()?.trim().parse::<f32>().ok().map(|v| v.round() as i32);
        let power = it.next().and_then(|v| v.trim().parse::<f32>().ok()).map(|v| v.round() as i32);
        let clock = it.next().and_then(|v| v.trim().parse::<f32>().ok()).map(|v| v.round() as i32);
        Some((temp, power, clock))
    }

    /// Text report for `--diag`: what NVML sees and what nvidia-smi returns.
    pub fn diag() -> String {
        let mut out = String::new();
        match Nvml::new() {
            Ok(n) => {
                let (t, w, c) = n.sample();
                out.push_str(&format!("NVML: OK  temperature={t:?} power={w:?} clock={c:?}\n"));
            }
            Err(e) => out.push_str(&format!("NVML: FAILED ({e})\n")),
        }
        let o = Command::new(smi_path())
            .args(["--query-gpu=name,temperature.gpu,power.draw", "--format=csv,noheader", "-i", "0"])
            .stdin(Stdio::null())
            .creation_flags(0x0800_0000)
            .output();
        match o {
            Ok(o) => out.push_str(&format!(
                "nvidia-smi: code={:?} out='{}' err='{}'\n",
                o.status.code(),
                String::from_utf8_lossy(&o.stdout).trim(),
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => out.push_str(&format!("nvidia-smi: could not launch ({e})\n")),
        }
        out
    }

    enum Source {
        Nvml(Nvml),
        Smi(Shared),
    }

    pub struct NvSensor {
        src: Source,
        info: String,
    }

    impl NvSensor {
        pub fn new() -> Option<Self> {
            match Nvml::new() {
                Ok(n) => Some(NvSensor { src: Source::Nvml(n), info: "NVML".into() }),
                Err(why) => {
                    // Fallback: nvidia-smi. If it doesn't work even once, no sensor at all.
                    let first = run_smi()?;
                    let shared: Shared = Arc::new(Mutex::new(first));
                    let s2 = shared.clone();
                    std::thread::spawn(move || loop {
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                        if let Some(v) = run_smi() {
                            if let Ok(mut g) = s2.lock() {
                                *g = v;
                            }
                        }
                    });
                    Some(NvSensor { src: Source::Smi(shared), info: format!("nvidia-smi (NVML: {why})") })
                }
            }
        }

        pub fn describe(&self) -> &str {
            &self.info
        }

        /// (temperature °C, power W) of the first NVIDIA GPU.
        pub fn sample(&self) -> Reading {
            match &self.src {
                Source::Nvml(n) => n.sample(),
                Source::Smi(s) => s.lock().map(|g| *g).unwrap_or((None, None, None)),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn diag() -> String {
        "Windows only\n".into()
    }
    pub struct NvSensor;
    impl NvSensor {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn describe(&self) -> &str {
            ""
        }
        pub fn sample(&self) -> (Option<i32>, Option<i32>, Option<i32>) {
            (None, None, None)
        }
    }
}

pub use imp::{diag, NvSensor};
