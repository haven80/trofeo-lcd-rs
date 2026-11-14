//! The tray icon (`start_in_tray = true`, or `--start-in-tray`): when the program starts, the console
//! window goes away and an icon appears in the notification area of the taskbar. A click on the
//! icon shows or hides the window again, the right button opens a small menu (settings editor,
//! show / hide, exit).
//!
//! Windows only. On other systems the option is accepted and ignored with a warning. The window to
//! hide is the console of the program; when it runs inside Windows Terminal that is the whole
//! Terminal window (it is hidden and shown again as a whole, with all its tabs).
//!
//! The icon lives on a thread of its own with a hidden window to receive the clicks: the main
//! loop does not know about it.

/// `config`: the configuration file in use, handed to the settings editor.
#[cfg(windows)]
pub fn start(config: Option<std::path::PathBuf>) {
    imp::start(config);
}

#[cfg(not(windows))]
pub fn start(_config: Option<std::path::PathBuf>) {
    eprintln!("WARNING: start_in_tray only works on Windows, ignored in this build.");
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TRAY: u32 = WM_APP + 1;
    const ID_TOGGLE: usize = 1;
    const ID_EXIT: usize = 2;
    const ID_EDITOR: usize = 3;
    /// Profile menu items: ID_PROFILE_BASE is "no profile", the profiles follow.
    const ID_PROFILE_BASE: usize = 100;
    /// The configuration file to open in the settings editor.
    static CONFIG: OnceLock<Option<PathBuf>> = OnceLock::new();
    /// The number Windows gives to "the taskbar has been created again" (Explorer restarted).
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    /// The window that shows the program's text: the console, or the Terminal that holds it.
    fn console() -> Option<HWND> {
        unsafe {
            let c = GetConsoleWindow();
            if c.0.is_null() {
                return None;
            }
            let root = GetAncestor(c, GA_ROOTOWNER);
            Some(if root.0.is_null() { c } else { root })
        }
    }

    fn visible() -> bool {
        console().is_some_and(|w| unsafe { IsWindowVisible(w).as_bool() })
    }

    fn set_visible(show: bool) {
        if let Some(w) = console() {
            unsafe {
                let _ = ShowWindow(w, if show { SW_SHOW } else { SW_HIDE });
                if show {
                    let _ = SetForegroundWindow(w);
                }
            }
        }
    }

    /// `trofeo_config.exe`, next to this program, when it is there.
    fn editor() -> Option<PathBuf> {
        let p = std::env::current_exe().ok()?.parent()?.join("trofeo_config.exe");
        p.is_file().then_some(p)
    }

    fn open_editor() {
        let Some(exe) = editor() else { return };
        let mut cmd = std::process::Command::new(exe);
        if let Some(Some(conf)) = CONFIG.get() {
            cmd.arg(conf);
        }
        let _ = cmd.spawn();
    }

    pub fn start(config: Option<PathBuf>) {
        let _ = CONFIG.set(config);
        let _ = std::thread::Builder::new().name("tray".into()).spawn(|| unsafe { run() });
    }

    /// A 32x32 icon made in memory: a dark disc with three bars of a spectrum.
    fn make_icon() -> Option<HICON> {
        const N: usize = 32;
        let mut bgra = vec![0u8; N * N * 4];
        let bars = [(9.0f32, 14.0f32, (0x38u8, 0xC6u8, 0xF4u8)), (15.0, 8.0, (0xFF, 0x6F, 0xB1)), (21.0, 11.0, (0xFF, 0xC0, 0x45))];
        for y in 0..N {
            for x in 0..N {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = (fx - 16.0).hypot(fy - 16.0);
                let disc = (15.5 - d).clamp(0.0, 1.0);
                if disc <= 0.0 {
                    continue;
                }
                let (mut r, mut g, mut b) = (24.0f32, 28.0f32, 46.0f32);
                for (bx, top, c) in bars {
                    let inside = (fx - bx).abs() <= 2.0 && fy >= top + 4.0 && fy <= 25.0;
                    if inside {
                        (r, g, b) = (c.0 as f32, c.1 as f32, c.2 as f32);
                    }
                }
                let i = (y * N + x) * 4;
                // premultiplied by the coverage of the edge
                bgra[i] = (b * disc) as u8;
                bgra[i + 1] = (g * disc) as u8;
                bgra[i + 2] = (r * disc) as u8;
                bgra[i + 3] = (255.0 * disc) as u8;
            }
        }
        let mask = vec![0u8; N * N / 8];
        unsafe {
            let hmod = GetModuleHandleW(None).ok()?;
            CreateIcon(HINSTANCE(hmod.0), N as i32, N as i32, 1, 32, mask.as_ptr(), bgra.as_ptr()).ok()
        }
    }

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = make_icon().unwrap_or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() });
        for (i, c) in "Trofeo LCD".encode_utf16().enumerate().take(127) {
            nid.szTip[i] = c;
        }
        nid
    }

    unsafe fn add_icon(hwnd: HWND) {
        let nid = icon_data(hwnd);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }

    unsafe fn remove_icon(hwnd: HWND) {
        let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }

    unsafe fn run() {
        let Ok(hmod) = GetModuleHandleW(None) else { return };
        let hinst = HINSTANCE(hmod.0);
        let class = w!("TrofeoLcdTray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst, lpszClassName: class, ..Default::default() };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Trofeo LCD"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, hinst, None) else {
            return;
        };
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        add_icon(hwnd);
        // the icon is there: now the window can go
        set_visible(false);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe fn menu(hwnd: HWND) {
        let Ok(hm) = CreatePopupMenu() else { return };
        let no_editor = if editor().is_some() { MENU_ITEM_FLAGS(0) } else { MF_GRAYED };
        let _ = AppendMenuW(hm, MF_STRING | no_editor, ID_EDITOR, w!("Open settings editor"));
        let _ = AppendMenuW(hm, MF_SEPARATOR, 0, PCWSTR::null());
        // Profiles (only when there are some): a sub-menu to choose one by hand.
        let pdir = CONFIG.get().and_then(|c| c.as_ref()).and_then(|c| trofeo_lcd::profiles::folder_of(c));
        let names = pdir.as_deref().map(trofeo_lcd::profiles::list).unwrap_or_default();
        let chosen = pdir.as_deref().and_then(trofeo_lcd::profiles::read_active);
        if !names.is_empty() {
            if let Ok(sub) = CreatePopupMenu() {
                let none_flags = if chosen.as_deref().map_or(true, |c| c == "none") { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(sub, none_flags, ID_PROFILE_BASE, w!("No profile"));
                for (i, n) in names.iter().enumerate() {
                    let label: Vec<u16> = n.encode_utf16().chain(std::iter::once(0)).collect();
                    let flags = if chosen.as_deref() == Some(n.as_str()) { MF_STRING | MF_CHECKED } else { MF_STRING };
                    let _ = AppendMenuW(sub, flags, ID_PROFILE_BASE + 1 + i, PCWSTR(label.as_ptr()));
                }
                let _ = AppendMenuW(hm, MF_STRING | MF_POPUP, sub.0 as usize, w!("Profile"));
            }
            let _ = AppendMenuW(hm, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let grey = if console().is_some() { MENU_ITEM_FLAGS(0) } else { MF_GRAYED };
        let label: PCWSTR = if visible() { w!("Hide window") } else { w!("Show window") };
        let _ = AppendMenuW(hm, MF_STRING | grey, ID_TOGGLE, label);
        let _ = AppendMenuW(hm, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hm, MF_STRING, ID_EXIT, w!("Exit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // without this the menu would not close when the user clicks somewhere else
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(hm, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, None);
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(hm);
        match cmd.0 as usize {
            id if id >= ID_PROFILE_BASE => {
                if let Some(dir) = pdir.as_deref() {
                    let pick = if id == ID_PROFILE_BASE { None } else { names.get(id - ID_PROFILE_BASE - 1).map(String::as_str) };
                    let _ = trofeo_lcd::profiles::write_active(dir, pick);
                }
            }
            ID_EDITOR => open_editor(),
            ID_TOGGLE => set_visible(!visible()),
            ID_EXIT => {
                remove_icon(hwnd);
                // the window comes back, so it closes with the program (nothing is left hidden)
                set_visible(true);
                std::process::exit(0);
            }
            _ => {}
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                match (lp.0 as u32) & 0xFFFF {
                    WM_LBUTTONUP => set_visible(!visible()),
                    WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                remove_icon(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                add_icon(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
