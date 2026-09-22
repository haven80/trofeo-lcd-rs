//! Detects the .exe name of the currently active (foreground) window on Windows.
//!
//! Used by `main.rs` to swap the "NOW PLAYING" display for the name of the
//! game/program currently running when GPU usage is high (an indication
//! that a game is being played, not that music is being listened to).
//!
//! How it works: `GetForegroundWindow` (active window) -> `GetWindowThreadProcessId`
//! (PID owning that window) -> `OpenProcess` + `QueryFullProcessImageNameW`
//! (exe path from that PID). Three standard WinAPI calls, no admin
//! privileges needed for ordinary processes/games (uses
//! `PROCESS_QUERY_LIMITED_INFORMATION`, the minimal access right that's
//! enough for this).

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    /// PID of the process that owns the foreground window.
    pub fn foreground_pid() -> Option<u32> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            (pid != 0).then_some(pid)
        }
    }

    /// Exe file name (without path, without the ".exe" extension) of the
    /// process that owns the current foreground window. `None` if there's
    /// no foreground window (e.g. locked screen), or the process can't be
    /// opened (system process with protection higher than our access
    /// level).
    pub fn foreground_exe_name() -> Option<String> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }

            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return None;
            }

            let process =
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let result = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(process);
            result.ok()?;

            let path = String::from_utf16_lossy(&buf[..size as usize]);
            let file_name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
            let name = file_name.strip_suffix(".exe").unwrap_or(file_name);

            if name.trim().is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    /// Exe file name (without path, without extension) of the process that
    /// owns the currently active window — via X11 (`_NET_ACTIVE_WINDOW` +
    /// `_NET_WM_PID`, the standard EWMH protocol supported by every modern
    /// window manager: GNOME/Mutter, KDE/KWin, XFCE, i3, etc).
    ///
    /// LIMITATION: only works in an X11 session (native OR via XWayland for
    /// apps that aren't native Wayland yet). In a PURE Wayland session
    /// (native Wayland windows without XWayland), `_NET_ACTIVE_WINDOW`
    /// doesn't exist because the base Wayland protocol deliberately doesn't
    /// expose "which window is active" to other applications (a
    /// security/sandboxing restriction of Wayland's own architecture, not
    /// something that can be worked around here) — some compositors
    /// (Sway/wlroots) have their own additional protocol
    /// (`wlr-foreign-toplevel-management`) for this, but it's not standard
    /// across compositors. If this happens, this function returns `None` —
    /// game mode still works (GPU>50% based), it's just that NOW PLAYING
    /// won't switch to the game's name.
    pub fn foreground_exe_name() -> Option<String> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen_num)?.root;

        let net_active_window = intern_atom(&conn, b"_NET_ACTIVE_WINDOW")?;
        let net_wm_pid = intern_atom(&conn, b"_NET_WM_PID")?;

        let active_reply = conn
            .get_property(false, root, net_active_window, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let window = active_reply.value32()?.next()?;
        if window == 0 {
            return None;
        }

        let pid_reply = conn
            .get_property(false, window, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let pid = pid_reply.value32()?.next()?;
        if pid == 0 {
            return None;
        }

        exe_name_from_pid(pid)
    }

    fn intern_atom(conn: &impl Connection, name: &[u8]) -> Option<u32> {
        Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
    }

    /// Exe name from `/proc/{pid}/exe` (full path, take the last file
    /// name segment) — more accurate than `/proc/{pid}/comm`, which the
    /// kernel truncates to a maximum of 15 characters. Falls back to
    /// `comm` if `exe` can't be read (e.g. a process owned by another user
    /// / extra protection).
    fn exe_name_from_pid(pid: u32) -> Option<String> {
        if let Ok(target) = fs::read_link(format!("/proc/{pid}/exe")) {
            if let Some(name) = target.file_name().and_then(|n| n.to_str()) {
                if !name.trim().is_empty() {
                    return Some(name.to_string());
                }
            }
        }
        let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        let name = comm.trim();
        (!name.is_empty()).then(|| name.to_string())
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn foreground_exe_name() -> Option<String> {
        None
    }
}

pub use imp::foreground_exe_name;
#[cfg(windows)]
pub use imp::foreground_pid;
