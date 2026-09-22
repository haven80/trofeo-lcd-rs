//! GPU usage (percent, total).
//!
//! - **Windows**: built-in performance counter ("GPU Engine", available since
//!   Windows 10 1803+ with WDDM 2.4+ drivers). HONEST NOTE: Windows doesn't
//!   have a single official "total GPU usage" number via PDH — what exists
//!   is per-"engine" utilization (3D, Copy, Video Decode, etc.) per process.
//!   Here we sum all instances of type `engtype_3d`, a common approach used
//!   by many third-party monitoring tools and usually the closest match to
//!   the "GPU %" number in Task Manager.
//! - **Linux**: the `amdgpu` kernel driver exposes this number DIRECTLY
//!   (already computed by the driver, not our estimate) via
//!   `/sys/class/drm/cardN/device/gpu_busy_percent` — just reading one text
//!   file, much simpler than the PDH path on Windows. The path is found once
//!   at `new()` (looking for the card with PCI vendor ID `0x1002` = AMD/ATI),
//!   then reused on every `sample()`.
//!
//! GPU temperature is DELIBERATELY NOT implemented here — see `gpu_amd.rs`
//! (ADL on Windows, hwmon on Linux) for that.

#[cfg(windows)]
mod imp {
    use windows::core::w;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
        PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    };

    pub struct GpuMonitor {
        // The PDH handle is represented as a raw `isize` in the `windows`
        // 0.58 crate (not separate `PDH_HQUERY`/`PDH_HCOUNTER` types).
        query: isize,
        counter: isize,
        // PDH rate/utilization counters need at least 2 samples before the
        // value is valid — the first sample is always discarded.
        primed: bool,
    }

    impl GpuMonitor {
        pub fn new() -> anyhow::Result<Self> {
            unsafe {
                let mut query: isize = 0;
                let status = PdhOpenQueryW(None, 0, &mut query);
                if status != 0 {
                    anyhow::bail!("PdhOpenQueryW failed (code {status:#x})");
                }

                let mut counter: isize = 0;
                // Wildcard "(*)" on the instance -> read as an array
                // containing ALL currently active GPU engine instances (per
                // process, per engine type).
                let path = w!(r"\GPU Engine(*)\Utilization Percentage");
                let status = PdhAddEnglishCounterW(query, path, 0, &mut counter);
                if status != 0 {
                    anyhow::bail!(
                        "PdhAddEnglishCounterW failed (code {status:#x}) — the OS/driver \
                         may not provide the 'GPU Engine' counter (needs Windows 10 1803+ & WDDM 2.4+)"
                    );
                }

                Ok(Self {
                    query,
                    counter,
                    primed: false,
                })
            }
        }

        /// Total GPU usage (0.0-100.0), or `Ok(0.0)` while no GPU 3D
        /// activity has been detected yet / first sample.
        pub fn sample(&mut self) -> anyhow::Result<f32> {
            unsafe {
                let status = PdhCollectQueryData(self.query);
                if status != 0 {
                    anyhow::bail!("PdhCollectQueryData failed (code {status:#x})");
                }

                if !self.primed {
                    self.primed = true;
                    return Ok(0.0);
                }

                let mut buffer_size: u32 = 0;
                let mut item_count: u32 = 0;
                // The first call is deliberately allowed to fail (buffer not
                // yet allocated), just to get the required buffer size via
                // `buffer_size`.
                let _ = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    None,
                );
                if buffer_size == 0 {
                    return Ok(0.0);
                }

                let mut buffer: Vec<u8> = vec![0u8; buffer_size as usize];
                let items_ptr = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                let status = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    Some(items_ptr),
                );
                if status != 0 {
                    anyhow::bail!("PdhGetFormattedCounterArrayW failed (code {status:#x})");
                }

                let items = std::slice::from_raw_parts(items_ptr, item_count as usize);
                let mut total = 0.0f64;
                for item in items {
                    if item.szName.is_null() {
                        continue;
                    }
                    let name = item.szName.to_string().unwrap_or_default();
                    if name.to_ascii_lowercase().contains("engtype_3d") {
                        total += item.FmtValue.Anonymous.doubleValue;
                    }
                }
                Ok(total.clamp(0.0, 100.0) as f32)
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::path::PathBuf;

    pub struct GpuMonitor {
        busy_path: Option<PathBuf>,
    }

    impl GpuMonitor {
        pub fn new() -> anyhow::Result<Self> {
            let busy_path = find_amd_gpu_busy_path();
            if busy_path.is_none() {
                eprintln!(
                    "WARNING: GPU usage unavailable — could not find \
                     /sys/class/drm/card*/device/gpu_busy_percent for an AMD GPU \
                     (amdgpu kernel driver). The info row will show N/A."
                );
            }
            Ok(Self { busy_path })
        }

        /// Total GPU usage (0.0-100.0) — already computed directly by the
        /// amdgpu kernel driver, just needs to be read.
        pub fn sample(&mut self) -> anyhow::Result<f32> {
            let Some(path) = &self.busy_path else {
                return Ok(0.0);
            };
            let text = fs::read_to_string(path).unwrap_or_default();
            let pct: f32 = text.trim().parse().unwrap_or(0.0);
            Ok(pct.clamp(0.0, 100.0))
        }
    }

    /// Find `/sys/class/drm/cardN/device/gpu_busy_percent` for the card with
    /// PCI vendor ID `0x1002` (AMD/ATI). Skips entries like
    /// "cardN-HDMI-A-1" (that's a display connector, not the GPU device itself).
    fn find_amd_gpu_busy_path() -> Option<PathBuf> {
        let entries = fs::read_dir("/sys/class/drm").ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(suffix) = name.strip_prefix("card") else { continue };
            if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }

            let device_dir = entry.path().join("device");
            if let Ok(vendor) = fs::read_to_string(device_dir.join("vendor")) {
                if vendor.trim().eq_ignore_ascii_case("0x1002") {
                    let busy_path = device_dir.join("gpu_busy_percent");
                    if busy_path.exists() {
                        return Some(busy_path);
                    }
                }
            }
        }
        None
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    /// Stub for other platforms: GPU usage unavailable, always `Ok(0.0)`.
    pub struct GpuMonitor;

    impl GpuMonitor {
        pub fn new() -> anyhow::Result<Self> {
            Ok(Self)
        }

        pub fn sample(&mut self) -> anyhow::Result<f32> {
            Ok(0.0)
        }
    }
}

pub use imp::GpuMonitor;
