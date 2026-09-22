//! Minimal safe wrapper around **PawnIOLib.dll** (<https://github.com/namazso/PawnIO>).
//!
//! Ported from the deepcool-digital-windows project, adapted to the `windows`
//! crate (not `windows-sys`) to stay consistent with the dependencies
//! already used elsewhere in this project.
//!
//! PawnIO is a WHQL-signed kernel driver for reading MSR/SMN/PCI registers
//! directly without needing LibreHardwareMonitor or any other software
//! running in the background. The driver must already be installed once at
//! the system level (via `winget install namazso.PawnIO` or the installer
//! from <https://pawnio.eu>) — this module only "talks" to a driver that is
//! already present, and backs off gracefully (returns `None`) if the driver
//! isn't found.
//!
//! We load `PawnIOLib.dll` dynamically (`LoadLibraryW`/`GetProcAddress`) —
//! not statically linked — so the build doesn't require PawnIO on the build
//! machine, and the program can still run (with the sensor unavailable) on a
//! machine that doesn't have the driver.

// The whole content of this module is only relevant on Windows.
#![cfg(windows)]

use std::ffi::CString;

use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, PCWSTR};

type HResult = i32;

fn succeeded(hr: HResult) -> bool {
    hr >= 0
}

// Function pointer types resolved from PawnIOLib.dll via GetProcAddress.
// Kept as raw types so we can store them as plain fn pointers (the FARPROC
// returned by GetProcAddress can't be stored directly since it isn't Sized).
type PawnioOpenFn = unsafe extern "system" fn(*mut HANDLE) -> HResult;
type PawnioLoadFn = unsafe extern "system" fn(HANDLE, *const u8, usize) -> HResult;
#[allow(clippy::type_complexity)]
type PawnioExecuteFn = unsafe extern "system" fn(
    HANDLE,
    *const i8,
    *const u64,
    usize,
    *mut u64,
    usize,
    *mut usize,
) -> HResult;
type PawnioCloseFn = unsafe extern "system" fn(HANDLE) -> HResult;

struct Api {
    open: PawnioOpenFn,
    load: PawnioLoadFn,
    execute: PawnioExecuteFn,
    close: PawnioCloseFn,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Load PawnIOLib.dll and resolve the 4 functions we need. Try the plain DLL
/// name first (works if it's already on PATH — added by the official
/// installer), then fall back to the default install location at
/// `%ProgramFiles%\PawnIO`.
fn load_api() -> Option<Api> {
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidates = [
        "PawnIOLib.dll".to_string(),
        format!(r"{program_files}\PawnIO\PawnIOLib.dll"),
    ];

    let module: HMODULE = candidates.iter().find_map(|path| {
        let wide = to_wide(path);
        // SAFETY: wide is a valid null-terminated UTF-16 string.
        unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }.ok()
    })?;

    // Resolve a single function from the already-loaded DLL.
    // SAFETY: T is always one of the Pawnio*Fn types above — all of them are
    // pointer-sized (same as the FARPROC that GetProcAddress returns), so
    // the transmute is safe with respect to size.
    unsafe fn resolve<T: Copy>(module: HMODULE, name: &str) -> Option<T> {
        let c_name = CString::new(name).ok()?;
        let addr = GetProcAddress(module, PCSTR(c_name.as_ptr() as *const u8))?;
        Some(std::mem::transmute_copy(&addr))
    }

    // SAFETY: every resolve below follows the ABI documented by PawnIO, and
    // the fn types above are defined to match that signature.
    unsafe {
        Some(Api {
            open: resolve(module, "pawnio_open")?,
            load: resolve(module, "pawnio_load")?,
            execute: resolve(module, "pawnio_execute")?,
            close: resolve(module, "pawnio_close")?,
        })
    }
}

/// A loaded PawnIO module — in this program it always holds the AMD Family
/// 17h (Zen1–Zen4) module embedded at compile time (see `cpu_sensor.rs`).
pub struct PawnIo {
    api: Api,
    handle: HANDLE,
}

impl PawnIo {
    /// Open the PawnIO executor and load the given module blob.
    /// `None` if the PawnIO driver isn't installed/running, or the blob is
    /// rejected (wrong signature, or the CPU isn't supported by that module —
    /// e.g. a non-AMD CPU or a family outside what that module covers).
    pub fn open_with_module(blob: &[u8]) -> Option<Self> {
        let api = load_api()?;

        let mut handle = HANDLE::default(); // null / invalid, filled in by pawnio_open
        if !succeeded(unsafe { (api.open)(&mut handle) }) {
            return None;
        }
        if !succeeded(unsafe { (api.load)(handle, blob.as_ptr(), blob.len()) }) {
            unsafe { (api.close)(handle) };
            return None;
        }
        Some(PawnIo { api, handle })
    }

    /// Call an IOCTL function exported by the module (e.g. `ioctl_read_smn`,
    /// `ioctl_read_msr`) by name. `input`/`out_len` are counted in `u64`
    /// cells, matching the size of the `in[]`/`out[]` buffers in the
    /// module's `.p` source.
    pub fn execute(&self, name: &str, input: &[u64], out_len: usize) -> Option<Vec<u64>> {
        let c_name = CString::new(name).ok()?;
        let mut output = vec![0u64; out_len];
        let mut returned: usize = 0;

        let hr = unsafe {
            (self.api.execute)(
                self.handle,
                c_name.as_ptr(),
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut returned,
            )
        };
        if !succeeded(hr) {
            return None;
        }
        output.truncate(returned);
        Some(output)
    }
}

impl Drop for PawnIo {
    fn drop(&mut self) {
        // SAFETY: handle is valid (filled in by a successful pawnio_open).
        unsafe { (self.api.close)(self.handle) };
    }
}

// SAFETY: the handle is an ordinary kernel object reference; there is no
// thread-local state used here, so moving it between threads is safe.
// We don't claim Sync because execute() is never called concurrently.
unsafe impl Send for PawnIo {}
