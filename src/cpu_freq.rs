//! Read the CPU frequency in **real time** (changing with load/boost).
//!
//! # Windows — PDH
//! The PDH counter `\Processor Information(_Total)\Processor Frequency` is
//! NOT reliable (its value is static on many Windows systems — verified on
//! this machine: stays at 3701 MHz even with all 12 cores fully loaded). The
//! one that actually moves is `\Processor Information(_Total)\% Processor
//! Performance`.
//!
//! The formula used (exactly the same one Task Manager uses for the
//! "Speed" column):
//! ```text
//! current frequency = base clock × (% Processor Performance / 100)
//! ```
//! The base clock is read from the registry `HKLM\HARDWARE\DESCRIPTION\System\
//! CentralProcessor\0\~MHz` (static, read once at init).
//!
//! # Linux — sysfs cpufreq
//! Average of `/sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq` (kHz)
//! across all cores. If the cpufreq driver isn't present (a machine without
//! a governor), falls back to the static value `cpuinfo_max_freq`/`/proc/cpuinfo
//! model name` so a number is still shown (the base clock), not N/A.

// ---------------------------------------------------------------------------
// Windows: PDH `% Processor Performance` × base clock from registry
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
        PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    };
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::PCWSTR;

    pub(super) struct Inner {
        query: isize,
        counter_perf: isize,
        base_mhz: u32,
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            // SAFETY: query is valid for the lifetime of `Inner`; closed only once.
            unsafe { let _ = PdhCloseQuery(self.query); }
        }
    }

    /// Read the base clock from the registry (`~MHz` = base frequency, DWORD).
    fn registry_base_mhz() -> Option<u32> {
        let subkey: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let value: Vec<u16> = "~MHz".encode_utf16().chain(std::iter::once(0)).collect();

        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the UTF-16 strings are valid; data/size point to local variables.
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut data as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        result.is_ok().then_some(data)
    }

    impl Inner {
        pub(super) fn new() -> Option<Self> {
            // SAFETY: PDH API calls; the handle is filled in by the DLL via pointer.
            unsafe {
                let mut query: isize = 0;
                let err = PdhOpenQueryW(None, 0, &mut query);
                if err != 0 {
                    eprintln!("WARNING: failed to open PDH query (0x{err:08X}), CPU frequency will be N/A.");
                    return None;
                }

                // Try the common counter name first, fall back to older variants.
                let mut counter_perf: isize = 0;
                let mut ok = false;
                for path in [
                    "\\Processor Information(_Total)\\% Processor Performance",
                    "\\Processor(_Total)\\% Processor Performance",
                ] {
                    let wide: Vec<u16> =
                        path.encode_utf16().chain(std::iter::once(0)).collect();
                    let err = PdhAddEnglishCounterW(query, PCWSTR(wide.as_ptr()), 0, &mut counter_perf);
                    if err == 0 {
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    eprintln!(
                        "WARNING: could not register the '% Processor Performance' counter, \
                         CPU frequency will be N/A."
                    );
                    PdhCloseQuery(query);
                    return None;
                }

                // Warm up with a few samples — the percentage counter needs an
                // initial sample (2 samples apart) before its first value is valid.
                for _ in 0..3 {
                    let _ = PdhCollectQueryData(query);
                    std::thread::sleep(Duration::from_millis(150));
                }

                let base_mhz = registry_base_mhz().unwrap_or(0);
                if base_mhz == 0 {
                    eprintln!(
                        "WARNING: CPU base clock could not be read from the registry, \
                         CPU frequency will be N/A."
                    );
                    PdhCloseQuery(query);
                    return None;
                }

                Some(Self { query, counter_perf, base_mhz })
            }
        }

        /// Current frequency in MHz = base × (%ProcessorPerformance / 100).
        pub(super) fn sample_mhz(&self) -> Option<u32> {
            // SAFETY: query/counter are valid for the lifetime of `self`.
            unsafe {
                let _ = PdhCollectQueryData(self.query);
                let mut value = PDH_FMT_COUNTERVALUE::default();
                if PdhGetFormattedCounterValue(self.counter_perf, PDH_FMT_DOUBLE, None, &mut value)
                    != 0
                {
                    return None;
                }
                // CStatus != 0 means there isn't valid data yet between 2 collects.
                if value.CStatus != 0 {
                    return None;
                }
                let perf = value.Anonymous.doubleValue.max(0.0);
                Some((self.base_mhz as f64 * perf / 100.0).round().clamp(0.0, f64::from(u32::MAX)) as u32)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: sysfs cpufreq
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod imp {
    use std::fs;

    pub(super) struct Inner {
        /// Base clock (kHz) as fallback when `scaling_cur_freq` isn't available.
        base_khz: u64,
    }

    /// Read the static base clock: prefer `cpuinfo_max_freq` (kHz), fall back
    /// to parsing `model name` in `/proc/cpuinfo` (e.g. "3.70GHz" -> 3_700_000 kHz).
    fn base_khz() -> Option<u64> {
        let via_sysfs =
            fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok());
        if let Some(khz) = via_sysfs {
            return Some(khz);
        }

        let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
        for line in cpuinfo.lines() {
            let line = line.trim();
            if !line.starts_with("model name") {
                continue;
            }
            if let Some(ghz_pos) = line.find("GHz") {
                let segment = &line[..ghz_pos];
                if let Some(last_space) = segment.rfind(' ') {
                    if let Ok(ghz) = segment[last_space + 1..].trim().parse::<f64>() {
                        return Some((ghz * 1_000_000.0).round() as u64);
                    }
                }
            }
        }
        None
    }

    /// Average `scaling_cur_freq` (kHz) across all cores (cpuN). SMT
    /// double-counting doesn't matter — the same convention is used by many
    /// other monitoring tools.
    fn read_scaling_khz(base: u64) -> u64 {
        let dirs = match fs::read_dir("/sys/devices/system/cpu") {
            Ok(d) => d,
            Err(_) => return base,
        };
        let mut total = 0u64;
        let mut n = 0u32;
        for entry in dirs.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("cpu") || !name[3..].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let path = entry.path().join("cpufreq/scaling_cur_freq");
            if let Some(khz) = fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
            {
                total += khz;
                n += 1;
            }
        }
        if n == 0 { base } else { total / u64::from(n) }
    }

    impl Inner {
        pub(super) fn new() -> Option<Self> {
            let base_khz = base_khz()?;
            if base_khz == 0 {
                return None;
            }
            Some(Self { base_khz })
        }

        /// Current frequency (MHz): average `scaling_cur_freq`, falling back to base.
        pub(super) fn sample_mhz(&self) -> Option<u32> {
            let khz = read_scaling_khz(self.base_khz);
            Some((khz / 1000).min(u64::from(u32::MAX)) as u32)
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(windows)]
struct Inner {
    imp: imp::Inner,
}
#[cfg(target_os = "linux")]
struct Inner {
    imp: imp::Inner,
}
#[cfg(not(any(windows, target_os = "linux")))]
struct Inner(());

/// Real-time CPU frequency monitor. On unsupported platforms, all methods
/// gracefully return `None` — the program keeps running, data shows "N/A".
pub struct CpuFreq {
    inner: Option<Inner>,
}

impl CpuFreq {
    /// Initialize the frequency reader. If it fails (driver/API not
    /// available), print a warning and continue (not exit).
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            return CpuFreq {
                inner: imp::Inner::new().map(|imp| Inner { imp }),
            };
        }
        #[cfg(target_os = "linux")]
        {
            return CpuFreq {
                inner: imp::Inner::new().map(|imp| Inner { imp }),
            };
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        CpuFreq { inner: None }
    }

    /// Current CPU frequency in MHz (real-time, following load/boost).
    /// `None` if there isn't valid data yet / unsupported.
    pub fn sample_mhz(&self) -> Option<u32> {
        #[cfg(windows)]
        {
            return self.inner.as_ref()?.imp.sample_mhz();
        }
        #[cfg(target_os = "linux")]
        {
            return self.inner.as_ref()?.imp.sample_mhz();
        }
        #[allow(unreachable_code)]
        None
    }
}
