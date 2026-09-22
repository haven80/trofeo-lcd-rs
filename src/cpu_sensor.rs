//! Read CPU package temperature and power draw (Watts) directly from hardware.
//!
//! - **Windows**: PawnIO driver + direct MSR/SMN reads (AMD Zen 1–4 via the
//!   official `AMDFamily17.bin` module, embedded at compile time). See
//!   `mod imp` below for register details.
//! - **Linux**: standard kernel sysfs.
//!   - **Temperature**: built-in kernel module `k10temp` (built-in, all Zen CPUs).
//!   - **Power — Zen 4+ (Ryzen 7000/8000/9000, including the 7500F)**: RAPL
//!     powercap `/sys/class/powercap/intel-rapl:N/energy_uj` ("package"
//!     zone). The driver is named `intel_rapl_msr` (config `CONFIG_INTEL_RAPL`,
//!     automatically enabled on all modern distros) — originally for Intel,
//!     but since AMD support was merged (patches from Google/AMD) this driver
//!     reads the AMD RAPL MSR (`MSR_PKG_ENERGY_STAT`/`0xC001_029B`, the same
//!     register as the Windows path) so the numbers are equivalent. This is
//!     the PRIMARY reading for Zen 4+, because `amd_energy` has been removed
//!     from mainline and `zenpower`/`zenpower3` does NOT support Zen 4
//!     (SVI3, not SVI2).
//!   - **Power — Zen 1-3**: `zenpower`/`zenpower3` (out-of-tree community
//!     driver, AUR: `zenpower3-dkms`), reading power from SVI2 VRM
//!     telemetry. See the comments in `mod imp` (Linux) for details on why
//!     and how these sources are distinguished (`PowerSource`).
//! - Other platforms (macOS, etc.): all sensors `None`, program still runs.
//!
//! # CPU coverage
//! Only **AMD Ryzen (Zen 1 through Zen 4)** — on BOTH platforms. Intel CPUs
//! are not covered: `CpuSensor::new()` still succeeds, but all sensors are
//! `None` (the info row shows N/A, the program doesn't crash).
//!
//! # Note on temperature accuracy (Windows)
//! The temperature decode formula (bit layout + the 49°C offset condition)
//! is ported from LibreHardwareMonitor's `Amd17Cpu.cs`. Compare the first
//! readings against Ryzen Master/HWiNFO to confirm on your specific CPU.

// ---------------------------------------------------------------------------
// Windows: PawnIO + direct MSR/SMN access
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use crate::pawnio::PawnIo;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::{
        CreateMutexW, ReleaseMutex, WaitForSingleObject, INFINITE,
    };
    use windows::core::PCWSTR;

    /// Official PawnIO module for AMD Family 17h–1Ah, embedded at compile time.
    /// Downloaded from the `AMDFamily17.bin` release in the PawnIO.Modules repo.
    pub(super) static AMD_MODULE: &[u8] = include_bytes!("resources/AMDFamily17.bin");

    // --- Register addresses ---

    /// AMD MSR: energy unit (Joules per energy counter LSB).
    pub(super) const MSR_PWR_UNIT: u64 = 0xC001_0299;
    /// AMD MSR: cumulative CPU package energy counter (effectively 32-bit, wraps around).
    pub(super) const MSR_PKG_ENERGY_STAT: u64 = 0xC001_029B;
    /// SMN register offset `THM_TCON_CUR_TMP` — CPU package temperature.
    pub(super) const SMN_THM_TCON_CUR_TMP: u64 = 0x5980_0;

    /// Named mutex that must be held before accessing SMN (indirect PCI config
    /// space), per the AMD PawnIO module documentation — prevents races with
    /// other tools (HWiNFO, Ryzen Master, etc.) that also access the same path.
    const PCI_MUTEX: &str = "Global\\Access_PCI";

    /// RAII guard for a named Win32 mutex: acquired when created, released
    /// and closed automatically when dropped. Used to serialize SMN access.
    pub(super) struct MutexGuard(HANDLE);

    impl MutexGuard {
        pub(super) fn acquire(name: &str) -> Option<Self> {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: wide is a valid null-terminated UTF-16 string; null attr = default.
            let handle = unsafe {
                CreateMutexW(
                    None,  // default security attributes
                    false, // not the initial owner
                    PCWSTR(wide.as_ptr()),
                )
            }
            .ok()?;

            // WAIT_FAILED = 0xFFFF_FFFF — every other value (including
            // WAIT_ABANDONED = 0x80) means we now hold the mutex.
            let wait_result = unsafe { WaitForSingleObject(handle, INFINITE) };
            if wait_result.0 == 0xFFFF_FFFF {
                unsafe { let _ = CloseHandle(handle); }
                return None;
            }
            Some(MutexGuard(handle))
        }

        pub(super) fn acquire_pci() -> Option<Self> {
            Self::acquire(PCI_MUTEX)
        }
    }

    impl Drop for MutexGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = ReleaseMutex(self.0);
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Decode the energy unit from `MSR_PWR_UNIT`: bits `[12:8]` are `ESU`,
    /// and unit = 0.5^ESU Joules per LSB (per AMD RAPL documentation).
    pub(super) fn read_energy_unit(pawnio: &PawnIo) -> Option<f64> {
        let raw = pawnio
            .execute("ioctl_read_msr", &[MSR_PWR_UNIT], 1)?
            .first()
            .copied()?;
        let esu = (raw >> 8) & 0x1F;
        Some(0.5_f64.powi(esu as i32))
    }
}

// ---------------------------------------------------------------------------
// Linux: sysfs — no root needed, just reading plain text files under /sys.
//
// Temperature: k10temp (built into the kernel, supports all Zen CPUs).
//
// Power — three sources, whose classification and calculation method are
// fully explained in the `PowerSource` enum below:
//
// 1. RAPL powercap (/sys/class/powercap/intel-rapl:N/energy_uj, "package"
//    zone) — the PRIMARY path for Zen 4+ (Ryzen 7000/8000/9000, including
//    the 7500F). The MAINLINE kernel driver `intel_rapl_msr` (from config
//    CONFIG_INTEL_RAPL) was originally for Intel, but since AMD support was
//    merged (patches from Google starting around Linux 5.8, later extended),
//    this driver also reads the AMD RAPL MSR — `MSR_PKG_ENERGY_STAT`,
//    register 0xC001_029B, the EXACT SAME register as this project's
//    Windows path. This means on Zen 4+ (which uses SVI3, not SVI2) power
//    can be read again on Linux via the RAPL path, matching the Windows
//    numbers.
//
// 2. amd_energy (hwmon RAPL) — this module was ACTUALLY REMOVED ENTIRELY
//    from mainstream Linux kernels since version 5.13 (April 2021) due to a
//    dispute between AMD and the hwmon maintainer over Platypus security
//    vulnerability mitigations. The code below still TRIES it first (just in
//    case some distro/custom kernel still carries it), then falls back to
//    other sources.
//
// 3. zenpower/zenpower3 (hwmon SVI2) — out-of-tree community driver (AUR:
//    `zenpower3-dkms`). Only supports Zen 1-3; on Zen 4+, which has moved to
//    SVI3 telemetry, this driver does NOT work (not a configuration issue,
//    it's simply unsupported). The data format is different: it's already
//    INSTANTANEOUS WATTS (`powerN_input`), not a cumulative energy counter —
//    so there's no need to compute a delta over time.
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Find the `/sys/class/hwmon/hwmonN` folder whose `name` file content
    /// exactly matches the driver name (e.g. "k10temp", "amd_energy", "zenpower").
    pub(super) fn find_hwmon_dir(driver_name: &str) -> Option<PathBuf> {
        let entries = fs::read_dir("/sys/class/hwmon").ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(name) = fs::read_to_string(path.join("name")) {
                if name.trim() == driver_name {
                    return Some(path);
                }
            }
        }
        None
    }

    /// CPU power reading source — the shape of the data differs in meaning
    /// between sources:
    #[derive(Clone)]
    pub(super) enum PowerSource {
        /// **RAPL powercap** — the PRIMARY path on Zen 4+ (Ryzen
        /// 7000/8000/9000, including the Ryzen 7500F). The mainline kernel
        /// driver `intel_rapl_msr` (originally built for Intel, but since
        /// Linux ~5.8+ also reads AMD RAPL via patches from Google/AMD)
        /// exposes the package energy counter as
        /// `/sys/class/powercap/intel-rapl:N/energy_uj` ("package-N" zone,
        /// cumulative microjoules) — a delta over time MUST be computed to
        /// get Watts. The numbers are equivalent to the Windows MSR path
        /// (`MSR_PKG_ENERGY_STAT`), since it's reading the same register.
        Rapl(PathBuf),
        /// amd_energy (RAPL hwmon, if it happens to be present on a custom
        /// kernel) — cumulative `energyN_input` in microjoules, a delta over
        /// time MUST be computed to get Watts (see `calc_power_watts`).
        Energy(PathBuf),
        /// zenpower — one or more `powerN_input` files that are ALREADY
        /// INSTANTANEOUS microwatts (not cumulative), just read and summed
        /// directly. zenpower typically exposes more than one rail (e.g.
        /// separate "SVI2 Core" + "SVI2 SoC") — all of them are summed to
        /// get the total estimate closest to the CPU package's actual power
        /// draw (a VRM-telemetry-based approximation, not RAPL — don't
        /// expect accuracy identical to Windows/amd_energy, but good enough
        /// to display as an indicator).
        Instant(Vec<PathBuf>),
    }

    /// Inside the `amd_energy` hwmon dir, find the `energyN_input` file whose
    /// label contains "ocket" (e.g. label "Esocket0" = total energy of 1 CPU
    /// socket) — the closest match to "CPU package power" on the Windows
    /// path. If not found (different driver version / different label),
    /// fall back to `energy1_input` as-is.
    pub(super) fn find_socket_energy_input(hwmon_dir: &Path) -> Option<PathBuf> {
        if let Ok(entries) = fs::read_dir(hwmon_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let file_name = file_name.to_string_lossy();
                if let Some(idx) = file_name
                    .strip_prefix("energy")
                    .and_then(|s| s.strip_suffix("_label"))
                {
                    if let Ok(label) = fs::read_to_string(entry.path()) {
                        if label.to_lowercase().contains("ocket") {
                            return Some(hwmon_dir.join(format!("energy{idx}_input")));
                        }
                    }
                }
            }
        }
        let fallback = hwmon_dir.join("energy1_input");
        fallback.exists().then_some(fallback)
    }

    /// Find the first "package" zone in `/sys/class/powercap` (e.g.
    /// `intel-rapl:0`, whose `${name}` starts with "package") and return the
    /// path to its `energy_uj` file.
    ///
    /// This is the PRIMARY power source for Zen 4+ CPUs on modern mainline
    /// kernels: the `intel_rapl_msr` driver (part of `CONFIG_INTEL_RAPL`,
    /// generally already built-in/auto-modprobed on all distros) reads the
    /// AMD RAPL MSR (`MSR_PKG_ENERGY_STAT`/`0xC001_029B`, the SAME register
    /// as this project's Windows path) and exposes it as a package powercap
    /// zone.
    ///
    /// Inner zones (e.g. `intel-rapl:0:0` whose `${name}` is "core") are
    /// deliberately skipped: we want the power of the WHOLE package, not
    /// just the core portion.
    pub(super) fn find_powercap_package_energy() -> Option<PathBuf> {
        let entries = fs::read_dir("/sys/class/powercap").ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // The control-type node (e.g. "intel-rapl") doesn't have
            // energy_uj — only zones (e.g. "intel-rapl:0") do. Check first
            // so the control-type gets skipped automatically.
            let energy = path.join("energy_uj");
            if !energy.exists() {
                continue;
            }
            let name = fs::read_to_string(path.join("name")).unwrap_or_default();
            if !name.trim().starts_with("package") {
                continue;
            }
            return Some(energy);
        }
        None
    }

    /// All `powerN_input` files inside a single hwmon folder (used for
    /// `zenpower`, which can expose more than one separate power rail).
    pub(super) fn find_all_power_inputs(hwmon_dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(hwmon_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let file_name = file_name.to_string_lossy();
                if file_name.starts_with("power") && file_name.ends_with("_input") {
                    out.push(entry.path());
                }
            }
        }
        out.sort();
        out
    }

    pub(super) fn read_u64(path: &Path) -> Option<u64> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }
}

// ---------------------------------------------------------------------------

#[cfg(windows)]
struct Inner {
    pawnio: crate::pawnio::PawnIo,
    energy_unit_joules: Option<f64>,
}

/// Internal Linux state: sysfs paths already discovered once in `new()`
/// (no need to re-scan `/sys/class/hwmon` on every sample).
#[cfg(target_os = "linux")]
struct Inner {
    temp_path: Option<std::path::PathBuf>,
    power_source: Option<imp::PowerSource>,
}

#[cfg(windows)]
type PlatformInner = Inner;
#[cfg(target_os = "linux")]
type PlatformInner = Inner;
#[cfg(not(any(windows, target_os = "linux")))]
type PlatformInner = ();

/// CPU temperature + power draw monitor (AMD Ryzen).
///
/// On unsupported platforms/CPUs, all methods gracefully return
/// `None`/`0` — the program keeps running normally with that data
/// displayed as "N/A".
pub struct CpuSensor {
    inner: Option<PlatformInner>,
}

impl CpuSensor {
    /// Initialize the sensor. If it can't be found/isn't supported, print a
    /// warning to stderr and continue (not exit).
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            use crate::pawnio::PawnIo;
            use imp::{AMD_MODULE, read_energy_unit};

            let pawnio = PawnIo::open_with_module(AMD_MODULE);
            if pawnio.is_none() {
                eprintln!(
                    "WARNING: CPU temperature/power sensors are unavailable. \
                     Make sure:\n  1. The PawnIO driver is installed: winget install namazso.PawnIO\n  \
                     2. The CPU is an AMD Ryzen (Zen1-Zen4 / Family 17h-1Ah)\n  \
                     The info row will show N/A for temperature & power."
                );
            }
            let energy_unit_joules = pawnio.as_ref().and_then(read_energy_unit);
            return CpuSensor {
                inner: pawnio.map(|pw| Inner { pawnio: pw, energy_unit_joules }),
            };
        }

        #[cfg(target_os = "linux")]
        {
            let temp_path = imp::find_hwmon_dir("k10temp")
                .or_else(|| imp::find_hwmon_dir("zenpower"))
                .map(|d| d.join("temp1_input"));

            // Power source priority (order matters — see the explanation in
            // `imp::PowerSource`):
            //   1. RAPL powercap — the only one that works on Zen 4+
            //      (Ryzen 7000/8000/9000, including the 7500F): the mainline
            //      intel_rapl_msr driver reading the AMD RAPL MSR.
            //   2. amd_energy hwmon — older custom kernels that still carry
            //      this module (removed from mainline since 5.13).
            //   3. zenpower/zenpower3 — ONLY Zen 1-3 CPUs (SVI2); on Zen 4+,
            //      which use SVI3, this driver will never work (not a
            //      configuration issue, it's simply unsupported).
            let power_source = imp::find_powercap_package_energy()
                .map(imp::PowerSource::Rapl)
                .or_else(|| {
                    imp::find_hwmon_dir("amd_energy")
                        .and_then(|d| imp::find_socket_energy_input(&d))
                        .map(imp::PowerSource::Energy)
                })
                .or_else(|| {
                    let zp_dir = imp::find_hwmon_dir("zenpower")?;
                    let inputs = imp::find_all_power_inputs(&zp_dir);
                    (!inputs.is_empty()).then_some(imp::PowerSource::Instant(inputs))
                });

            if temp_path.is_none() {
                eprintln!(
                    "WARNING: CPU temperature sensor not found (needs the \
                     'k10temp' kernel module, usually already built-in — check: \
                     ls /sys/class/hwmon/*/name | xargs grep -l k10temp 2>/dev/null, \
                     or 'sudo modprobe k10temp'). CPU temperature will be N/A."
                );
            }
            if power_source.is_none() {
                eprintln!(
                    "WARNING: CPU power sensor not found (no readable RAPL \
                     source).\n  \
                     For Zen 4+ (Ryzen 7000/7500F etc.) this program uses the \
                     RAPL powercap path (/sys/class/powercap/intel-rapl:N/energy_uj) — \
                     the 'intel_rapl_msr' kernel driver. If this warning shows up even \
                     though the CPU is Zen 4+, check:\n  \
                     1. the RAPL module is active: 'sudo modprobe intel_rapl_msr' (usually \
                     automatic), then check the energy value is readable: \
                     'ls /sys/class/powercap/*/energy_uj'\n  \
                     2. the file is readable by a normal user (root-only by default; if \
                     'cat ...' says Permission denied, add a udev rule with chmod 0444 in \
                     /etc/udev/rules.d/, see the README).\n  \
                     Note: 'zenpower3' (SVI2) only works for Zen 1-3, it does NOT \
                     support Zen 4+ which uses SVI3; 'amd_energy' has been removed from \
                     the mainline kernel since 5.13. CPU power will be N/A until a \
                     readable source is found."
                );
            }
            return CpuSensor { inner: Some(Inner { temp_path, power_source }) };
        }

        #[cfg(not(any(windows, target_os = "linux")))]
        CpuSensor { inner: None }
    }

    /// CPU package temperature in °C. `None` if the sensor is unavailable.
    pub fn get_temp_c(&self) -> Option<f32> {
        #[cfg(windows)]
        {
            use imp::{MutexGuard, SMN_THM_TCON_CUR_TMP};
            let inner = self.inner.as_ref()?;
            let _guard = MutexGuard::acquire_pci();
            let raw = inner
                .pawnio
                .execute("ioctl_read_smn", &[SMN_THM_TCON_CUR_TMP], 1)?
                .first()
                .map(|&v| v as u32)?;

            // Decoded from LibreHardwareMonitor's Amd17Cpu.cs:
            //   bits [31:21] = temperature × 0.125 °C
            //   bit 19 ("range select") or bits [17:16] both set ("Tj select")
            //   → subtract 49 °C from the raw value
            let range_sel = raw & 0x0008_0000 != 0;
            let tj_sel = raw & 0x0003_0000 == 0x0003_0000;
            let mut milli_c = (raw >> 21) as i32 * 125;
            if range_sel || tj_sel {
                milli_c -= 49_000;
            }
            return Some((milli_c as f32 / 1000.0).max(0.0));
        }

        #[cfg(target_os = "linux")]
        {
            let inner = self.inner.as_ref()?;
            let path = inner.temp_path.as_ref()?;
            let milli_c = imp::read_u64(path)? as f32;
            return Some(milli_c / 1000.0);
        }

        #[allow(unreachable_code)]
        None
    }

    /// Take a snapshot of the cumulative energy counter — ONLY relevant for
    /// `PowerSource::Energy` (amd_energy/RAPL) and `PowerSource::Rapl`
    /// (RAPL powercap). For `PowerSource::Instant`
    /// (zenpower) this value isn't used at all (`calc_power_watts`
    /// reads directly without needing a time delta), so `0` there is safe.
    ///
    /// - Windows: raw MSR LSB (effectively 32-bit, wraps around ~every 40 seconds).
    /// - Linux (`Energy`): cumulative microjoules (µJ) from hwmon (64-bit,
    ///   practically never wraps within a realistic duration).
    /// - Linux (`Rapl`): cumulative microjoules (µJ) from powercap — the
    ///   package zone counter can wrap at `max_energy_range_uj` (~65 kJ on
    ///   the dev machine, meaning it wraps every few minutes depending on
    ///   load); safe because the sample interval here is only ~500 ms, well
    ///   below the wrap period, and `wrapping_sub` in `calc_power_watts`
    ///   handles it correctly.
    ///
    /// Store the value, then pass it to `calc_power_watts()` along with the
    /// elapsed time to get the average Watts over that interval.
    pub fn sample_energy(&self) -> u64 {
        #[cfg(windows)]
        {
            use imp::MSR_PKG_ENERGY_STAT;
            if let Some(inner) = &self.inner {
                return inner
                    .pawnio
                    .execute("ioctl_read_msr", &[MSR_PKG_ENERGY_STAT], 1)
                    .and_then(|v| v.first().copied())
                    .unwrap_or(0)
                    & 0xFFFF_FFFF;
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(inner) = &self.inner {
                if let Some(p) = &inner.power_source {
                    if let imp::PowerSource::Energy(path) | imp::PowerSource::Rapl(path) = p {
                        return imp::read_u64(path).unwrap_or(0);
                    }
                }
            }
        }
        0
    }

    /// Compute the CPU package's average power draw (Watts) since
    /// `prev_energy` was taken, `delta_ms` milliseconds ago. `None` if the
    /// sensor is unavailable or `delta_ms` is 0.
    ///
    /// On Linux, each source has a different calculation method (see
    /// `imp::PowerSource`):
    /// - `Rapl` (powercap) & `Energy` (amd_energy/RAPL): cumulative counter,
    ///   MUST compute a delta from `prev_energy`/`delta_ms` — exactly like
    ///   the Windows MSR path above.
    /// - `Instant` (zenpower): ALREADY instantaneous watts, just sum all
    ///   rails & convert units — `prev_energy`/`delta_ms` are NOT used at
    ///   all in this branch (the parameters stay for a matching signature,
    ///   just ignored).
    pub fn calc_power_watts(&self, prev_energy: u64, delta_ms: u64) -> Option<f32> {
        #[cfg(windows)]
        {
            let inner = self.inner.as_ref()?;
            let unit = inner.energy_unit_joules?;
            if delta_ms == 0 {
                return None;
            }
            // Use wrapping_sub because the 32-bit MSR counter can wrap
            // around — safe as long as the interval doesn't span more than
            // one wrap.
            let current = self.sample_energy();
            let delta_lsb = (current as u32).wrapping_sub(prev_energy as u32);
            let joules = delta_lsb as f64 * unit;
            let watts = joules / (delta_ms as f64 / 1000.0);
            return Some(watts.clamp(0.0, 9999.0) as f32);
        }

        #[cfg(target_os = "linux")]
        {
            let inner = self.inner.as_ref()?;
            return match inner.power_source.as_ref()? {
                imp::PowerSource::Energy(_) | imp::PowerSource::Rapl(_) => {
                    if delta_ms == 0 {
                        return None;
                    }
                    let current = self.sample_energy();
                    let delta_uj = current.wrapping_sub(prev_energy);
                    let joules = delta_uj as f64 / 1_000_000.0;
                    let watts = joules / (delta_ms as f64 / 1000.0);
                    Some(watts.clamp(0.0, 9999.0) as f32)
                }
                imp::PowerSource::Instant(paths) => {
                    let total_microwatts: u64 =
                        paths.iter().filter_map(|p| imp::read_u64(p)).sum();
                    Some((total_microwatts as f64 / 1_000_000.0).clamp(0.0, 9999.0) as f32)
                }
            };
        }

        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = (prev_energy, delta_ms);
        }

        #[allow(unreachable_code)]
        None
    }
}
