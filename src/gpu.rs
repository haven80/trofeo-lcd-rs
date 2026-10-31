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

        /// Every active engine instance and its utilization (%): the raw data of the counter.
        /// `None` for the first call (a rate counter needs two samples) or when there is none.
        fn items(&mut self) -> anyhow::Result<Option<Vec<(String, f64)>>> {
            unsafe {
                let status = PdhCollectQueryData(self.query);
                if status != 0 {
                    anyhow::bail!("PdhCollectQueryData failed (code {status:#x})");
                }

                if !self.primed {
                    self.primed = true;
                    return Ok(None);
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
                    return Ok(None);
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
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if item.szName.is_null() {
                        continue;
                    }
                    let name = item.szName.to_string().unwrap_or_default();
                    out.push((name, item.FmtValue.Anonymous.doubleValue));
                }
                Ok(Some(out))
            }
        }

        /// Total GPU usage (0.0-100.0), or `Ok(0.0)` while no GPU 3D
        /// activity has been detected yet / first sample.
        pub fn sample(&mut self) -> anyhow::Result<f32> {
            let Some(items) = self.items()? else { return Ok(0.0) };
            let total: f64 = items
                .iter()
                .filter(|(name, _)| name.to_ascii_lowercase().contains("engtype_3d"))
                .map(|(_, v)| *v)
                .sum();
            Ok(total.clamp(0.0, 100.0) as f32)
        }

        /// Utilization summed per engine type (`3D`, `Compute_0`, `VideoDecode`, `Copy`...),
        /// for `--diag`: it shows what the 3D-only total of `sample` leaves out.
        pub fn engine_totals(&mut self) -> Vec<(String, f64)> {
            let Ok(Some(items)) = self.items() else { return Vec::new() };
            super::engine_totals_of(&items)
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

        /// Only the Windows counter has engines.
        pub fn engine_totals(&mut self) -> Vec<(String, f64)> {
            Vec::new()
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

        pub fn engine_totals(&mut self) -> Vec<(String, f64)> {
            Vec::new()
        }
    }
}

pub use imp::GpuMonitor;

/// Where the GPU usage comes from (`gpu_usage_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UsageSource {
    /// The driver's own figure when it gives one (Radeon cards on Windows), the Windows
    /// counter otherwise.
    #[default]
    Auto,
    /// Always the Windows "GPU Engine" counter (3D engines only), as before version 2.7.1.
    Windows,
}

impl UsageSource {
    pub const NAMES: &'static str = "auto | windows";

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "driver" | "automatic" | "automatico" => Some(UsageSource::Auto),
            "windows" | "counter" | "task-manager" | "taskmanager" => Some(UsageSource::Windows),
            _ => None,
        }
    }
}

/// The GPU usage to show: the driver's figure (`driver`, a percentage) when the source allows
/// it and the driver gave one, the Windows counter's (`windows`) otherwise.
pub fn pick_usage(source: UsageSource, windows: Option<f32>, driver: Option<i32>) -> Option<f32> {
    match (source, driver) {
        (UsageSource::Auto, Some(d)) if (0..=100).contains(&d) => Some(d as f32),
        _ => windows,
    }
}

/// Sums the instances of the counter per engine type. An instance is named like
/// `pid_1234_luid_0x00000000_0x0000A1B2_phys_0_eng_0_engtype_3D`; the engine type is what
/// follows `engtype_`. Biggest first, engines with nothing to report left out.
pub fn engine_totals_of(items: &[(String, f64)]) -> Vec<(String, f64)> {
    let mut totals: Vec<(String, f64)> = Vec::new();
    for (name, v) in items {
        let kind = match name.to_ascii_lowercase().find("engtype_") {
            Some(i) => name[i + "engtype_".len()..].to_string(),
            None => continue,
        };
        match totals.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, t)) => *t += v,
            None => totals.push((kind, *v)),
        }
    }
    totals.retain(|(_, v)| *v >= 0.05);
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_driver_figure_wins_on_auto_only() {
        assert_eq!(pick_usage(UsageSource::Auto, Some(60.0), Some(100)), Some(100.0));
        assert_eq!(pick_usage(UsageSource::Windows, Some(60.0), Some(100)), Some(60.0));
        assert_eq!(pick_usage(UsageSource::Auto, Some(60.0), None), Some(60.0));
        assert_eq!(pick_usage(UsageSource::Auto, None, None), None);
        assert_eq!(pick_usage(UsageSource::Auto, Some(60.0), Some(250)), Some(60.0), "nonsense is ignored");
        assert_eq!(pick_usage(UsageSource::Auto, Some(60.0), Some(0)), Some(0.0), "an idle card is 0, not missing");
    }

    #[test]
    fn the_source_is_read_from_its_names() {
        assert_eq!(UsageSource::parse("AUTO"), Some(UsageSource::Auto));
        assert_eq!(UsageSource::parse("driver"), Some(UsageSource::Auto));
        assert_eq!(UsageSource::parse("Windows"), Some(UsageSource::Windows));
        assert_eq!(UsageSource::parse("nvidia"), None);
    }

    #[test]
    fn engines_are_summed_per_type() {
        let items = vec![
            ("pid_10_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 40.0),
            ("pid_11_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 20.0),
            ("pid_10_luid_0x0_0xA_phys_0_eng_3_engtype_Compute_0".to_string(), 33.5),
            ("pid_12_luid_0x0_0xA_phys_0_eng_5_engtype_Copy".to_string(), 0.0),
            ("something_without_a_type".to_string(), 9.0),
        ];
        let t = engine_totals_of(&items);
        assert_eq!(t, vec![("3D".to_string(), 60.0), ("Compute_0".to_string(), 33.5)]);
    }
}
