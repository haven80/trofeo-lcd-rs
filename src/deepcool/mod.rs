//! **DeepCool Digital** display integration (AIO/air cooler/case) for
//! trofeo-lcd — so DeepCool coolers keep showing CPU data without having to
//! run a separate program; everything is bundled into trofeo-lcd.
//!
//! DeepCool devices are driven over HID (`hidapi`), while the sensor data is
//! PULLED FROM the monitor trofeo-lcd already owns:
//! - CPU **temperature + power**: `CpuSensor` (PawnIO on Windows / sysfs on
//!   Linux) — the SAME instance (shared via `Arc<Mutex<_>>`) as the main info
//!   line, so the numbers on both displays stay consistent and only one
//!   driver handle is opened.
//! - CPU **usage**: `sysinfo::System` owned by this thread itself (baseline
//!   taken in `read_instant()`, delta in `get_usage()` — same approach as
//!   `CpuInstant` in the deepcool project).
//! - **Frequency**: `CpuFreq` (PDH on Windows / sysfs cpufreq on Linux).
//!
//! The per-device drivers (`src/deepcool/*.rs`) are ported line-by-line from
//! the [deepcool-digital-linux](https://github.com/Nortank12/deepcool-digital-linux)
//! project (the same version used by `deepcool-digital-windows` in
//! `../deepcool`), with only imports/macros adjusted. The main loop runs on a
//! **background thread** — if the device isn't found or gets unplugged, the
//! thread retries automatically, while the main program (Trofeo LCD) keeps
//! running normally.
//!
//! All of this is ONLY active if `hidapi` can open the device. If there is no
//! DeepCool device at all, the thread silently retries every few seconds
//! (small cost: HID enumeration) without disturbing the main loop.

pub mod ag_series;
pub mod ak400_pro;
pub mod ak620_pro;
pub mod ak_series;
pub mod ch510;
pub mod ch_series;
pub mod ch_series_gen2;
pub mod ld_series;
pub mod lp_series;
pub mod lq_series;
pub mod ls_series;

use crate::cpu_freq::CpuFreq;
use crate::cpu_sensor::CpuSensor;
use hidapi::HidApi;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::System;

/// Main vendor ID for DeepCool devices (HID).
pub const DEFAULT_VENDOR_ID: u16 = 13875;
/// Vendor ID for the CH510 case (different from other DeepCool devices).
pub const CH510_VENDOR_ID: u16 = 13523;
/// Product ID for the CH510 MESH DIGITAL case.
pub const CH510_PRODUCT_ID: u16 = 4352;

/// Mode-switching period in `Mode::Auto` — DeepCool devices switch what's
/// shown at this interval.
pub const AUTO_MODE_INTERVAL: Duration = Duration::from_millis(5000);

#[derive(PartialEq)]
pub enum Mode {
    Default,
    Auto,
    CpuTemperature,
    CpuUsage,
    CpuPower,
    CpuFrequency,
    CpuFan,
    GpuTemperature,
    GpuUsage,
    GpuPower,
    Cpu,
    Gpu,
    Psu,
}

impl Mode {
    pub const fn symbol(&self) -> &'static str {
        match self {
            Mode::Default => "",
            Mode::Auto => "auto",
            Mode::CpuTemperature => "cpu_temp",
            Mode::CpuUsage => "cpu_usage",
            Mode::CpuPower => "cpu_power",
            Mode::CpuFrequency => "cpu_freq",
            Mode::CpuFan => "cpu_fan",
            Mode::GpuTemperature => "gpu_temp",
            Mode::GpuUsage => "gpu_usage",
            Mode::GpuPower => "gpu_power",
            Mode::Cpu => "cpu",
            Mode::Gpu => "gpu",
            Mode::Psu => "psu",
        }
    }

    /// Error path for mode validation — unlike the original, trofeo-lcd
    /// doesn't `exit(1)` here: the device just falls back to the default
    /// mode so the thread doesn't die. The constructor only ever receives
    /// `Mode::Default` (dispatcher), so this is practically never called.
    pub fn support_error(&self) -> Mode {
        eprintln!(
            "DeepCool: display mode \"{}\" is not supported by this device — using default.",
            self.symbol()
        );
        self.clone()
    }

    /// Same as `support_error`, for the secondary display mode.
    pub fn support_error_secondary(&self) -> Mode {
        eprintln!(
            "DeepCool: secondary display mode \"{}\" is not supported by this device — using default.",
            self.symbol()
        );
        self.clone()
    }
}

impl Clone for Mode {
    fn clone(&self) -> Self {
        match self {
            Mode::Default => Mode::Default,
            Mode::Auto => Mode::Auto,
            Mode::CpuTemperature => Mode::CpuTemperature,
            Mode::CpuUsage => Mode::CpuUsage,
            Mode::CpuPower => Mode::CpuPower,
            Mode::CpuFrequency => Mode::CpuFrequency,
            Mode::CpuFan => Mode::CpuFan,
            Mode::GpuTemperature => Mode::GpuTemperature,
            Mode::GpuUsage => Mode::GpuUsage,
            Mode::GpuPower => Mode::GpuPower,
            Mode::Cpu => Mode::Cpu,
            Mode::Gpu => Mode::Gpu,
            Mode::Psu => Mode::Psu,
        }
    }
}

/// CPU temperature/usage/power/frequency handle for DeepCool device drivers,
/// wired up to the sensors trofeo-lcd already owns (`CpuSensor`, `CpuFreq`,
/// sysinfo).
///
/// Each method mirrors the `Cpu` API in the deepcool project
/// (`get_temp(fahrenheit)`, `read_energy()`, `get_power(prev, delta_ms)`,
/// `get_usage(instant)`, `get_frequency()`), so the device drivers can be
/// ported over with almost no changes.
pub struct DeepCpu {
    sensor: Arc<Mutex<CpuSensor>>,
    freq: RefCell<CpuFreq>,
    sys: RefCell<System>,
    temp_warned: AtomicBool,
    power_warned: AtomicBool,
}

/// Baseline CPU usage snapshot (the delta is computed in `DeepCpu::get_usage`).
pub struct CpuInstant;

impl DeepCpu {
    pub fn new(sensor: Arc<Mutex<CpuSensor>>) -> Self {
        DeepCpu {
            sensor,
            freq: RefCell::new(CpuFreq::new()),
            sys: RefCell::new(System::new()),
            temp_warned: AtomicBool::new(false),
            power_warned: AtomicBool::new(false),
        }
    }

    /// Takes the baseline CPU usage (resets the `sysinfo` counter). Pairs
    /// with `get_usage()` — called in sequence with a sleep in between, like
    /// `CpuInstant` in the deepcool project.
    pub fn read_instant(&self) -> CpuInstant {
        self.sys.borrow_mut().refresh_cpu();
        CpuInstant
    }

    /// CPU usage (`0-99`) since `read_instant()` was last called.
    pub fn get_usage(&self, _instant: &CpuInstant) -> u8 {
        let mut sys = self.sys.borrow_mut();
        sys.refresh_cpu();
        sys.global_cpu_info().cpu_usage().round().clamp(0.0, 99.0) as u8
    }

    /// CPU package temperature, `°C` or `°F`. `0` if the sensor is
    /// unavailable (which is why `get_temp` in the deepcool project also
    /// returns `0`).
    pub fn get_temp(&self, fahrenheit: bool) -> u8 {
        let c = self.sensor.lock().ok().and_then(|s| s.get_temp_c());
        match c {
            Some(c) => {
                let v = if fahrenheit { c * 9.0 / 5.0 + 32.0 } else { c };
                (v.round().max(0.0) as i64).min(i64::from(u8::MAX)) as u8
            }
            None => 0,
        }
    }

    /// Snapshot of the cumulative energy counter — pairs with `get_power()`.
    pub fn read_energy(&self) -> u64 {
        self.sensor.lock().ok().map(|s| s.sample_energy()).unwrap_or(0)
    }

    /// Average CPU package power draw (Watts) since `prev_energy` was taken,
    /// `delta_ms` milliseconds ago. `0` if the sensor is unavailable.
    pub fn get_power(&self, prev_energy: u64, delta_ms: u64) -> u16 {
        match self.sensor.lock().ok().and_then(|s| s.calc_power_watts(prev_energy, delta_ms)) {
            Some(w) => (w.round() as i64).clamp(0, i64::from(u16::MAX)) as u16,
            None => 0,
        }
    }

    /// Real-time CPU frequency (MHz). `0` if unavailable.
    pub fn get_frequency(&self) -> u16 {
        match self.freq.borrow_mut().sample_mhz() {
            Some(m) => (m as i64).clamp(0, i64::from(u16::MAX)) as u16,
            None => 0,
        }
    }

    /// One-time warning if the CPU temperature sensor is unavailable (avoid
    /// spamming on every reconnect).
    pub fn warn_temp(&self) {
        let available = self.sensor.lock().ok().and_then(|s| s.get_temp_c()).is_some();
        if !available && !self.temp_warned.swap(true, Ordering::Relaxed) {
            eprintln!(
                "DeepCool: CPU temperature sensor unavailable (PawnIO/driver), the \
                 cooler display will show 0."
            );
        }
    }

    /// One-time warning if the CPU power sensor is unavailable.
    pub fn warn_rapl(&self) {
        let available = self.sensor.lock().ok().and_then(|s| s.calc_power_watts(0, 500)).is_some();
        if !available && !self.power_warned.swap(true, Ordering::Relaxed) {
            eprintln!(
                "DeepCool: CPU power sensor unavailable (PawnIO/RAPL driver), the \
                 cooler display will show 0."
            );
        }
    }
}

/// GPU stub — in the deepcool project, GPU monitoring isn't implemented yet
/// (only cases use it, and it's 0 everywhere). Kept so the CH/LP series
/// drivers can be ported without meaningful changes.
pub struct Gpu;

impl Gpu {
    pub fn get_temp(&self, _fahrenheit: bool) -> u8 {
        0
    }
    pub fn get_usage(&self) -> u8 {
        0
    }
    pub fn get_power(&self) -> u16 {
        0
    }
    pub fn get_frequency(&self) -> u16 {
        0
    }
    pub fn warn_missing(&self) {
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!("DeepCool: GPU monitoring is not implemented, the GPU section on the case will show 0.");
        }
    }
}

/// Options for the DeepCool integration thread (see `spawn`).
pub struct Options {
    /// Interval for sending data to the DeepCool display, ms (clamped to 100-2000).
    pub update_ms: u64,
}

/// Run the DeepCool driver on a background thread. The main program (Trofeo
/// LCD) does not depend on this thread at all: if the device isn't found, it
/// just retries a few seconds later.
pub fn spawn(sensor: Arc<Mutex<CpuSensor>>, opts: Options) {
    std::thread::Builder::new()
        .name("deepcool".to_string())
        .spawn(move || {
            let update = Duration::from_millis(opts.update_ms.clamp(100, 2000));

            loop {
                let api = match HidApi::new() {
                    Ok(api) => api,
                    Err(_) => {
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };

                // Look for a DeepCool device: default vendor, or the CH510 case.
                let mut product_id: u16 = 0;
                for device in api.device_list() {
                    if device.vendor_id() == DEFAULT_VENDOR_ID {
                        product_id = device.product_id();
                        break;
                    } else if device.vendor_id() == CH510_VENDOR_ID
                        && device.product_id() == CH510_PRODUCT_ID
                    {
                        product_id = device.product_id();
                        break;
                    }
                }
                if product_id == 0 {
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }

                // The device drivers use `.unwrap()` on `device.write()` — if
                // the device gets unplugged, a panic occurs; catch it here so
                // the loop can retry (instead of the thread silently dying).
                let cpu = DeepCpu::new(Arc::clone(&sensor));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match product_id {
                        // AK Series (AK400/500/620 DIGITAL...)
                        1..=4 => {
                            let mut d = ak_series::Display::new(
                                cpu, &Mode::Default,
                                update, false, false,
                            );
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // LS Series (LS520 SE / LS720 SE DIGITAL)
                        6 => {
                            let d = ls_series::Display::new(
                                cpu, &Mode::Default, update, false, false,
                            );
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // AG Series (AG300/400/500/620 DIGITAL)
                        8 => {
                            let d = ag_series::Display::new(cpu, &Mode::Default, update, false);
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // LD Series (LD240/LD360)
                        10 => {
                            let d = ld_series::Display::new(cpu, update, false, false);
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // LP Series (LP240/LP360)
                        12 => {
                            let d = lp_series::Display::new(
                                cpu, Gpu,
                                &Mode::Default, &Mode::Default, update, false, 0,
                            );
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // LQ Series, ASSASSIN IV, AK G2 Series, AK700
                        13 | 15 | 31 | 41 | 42 | 43 | 44 => {
                            let d = lq_series::Display::new(cpu, update, false);
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // AK400 PRO
                        16 => {
                            let d = ak400_pro::Display::new(cpu, update, false);
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // AK500 / AK620 PRO
                        17 | 18 => {
                            let d = ak620_pro::Display::new(cpu, update, false);
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // CH170 | CH270 | CH690
                        19 | 22 | 27 => {
                            let d = ch_series_gen2::Display::new(
                                cpu, Gpu, &Mode::Default, update, false,
                            );
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // CH Series & MORPHEUS
                        5 | 7 | 21 => {
                            let d = ch_series::Display::new(
                                cpu, Gpu, &Mode::Default, &Mode::Default, update, false,
                            );
                            d.run(&api, DEFAULT_VENDOR_ID, product_id);
                        }
                        // CH510 MESH DIGITAL
                        CH510_PRODUCT_ID => {
                            let d = ch510::Display::new(cpu, Gpu, &Mode::Default, update, false);
                            d.run(&api, CH510_VENDOR_ID, product_id);
                        }
                        _ => {
                            eprintln!(
                                "DeepCool: device detected (PID {product_id}) but not yet \
                                 supported by this program — retrying later."
                            );
                        }
                    }
                }));
                std::mem::drop(result);
                eprintln!("DeepCool: device unplugged/failed, retrying...");
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .ok();
}