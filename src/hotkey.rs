//! Global hotkey for LCD frame screenshots (used by trofeo_lcd and
//! trofeo_screen). On Windows it uses `RegisterHotKey` — a "global" hotkey
//! (works even when the program's window isn't focused). Not available on
//! other OSes: `register` fails with a clear message, and the program keeps
//! running without the hotkey.

/// A registered global hotkey. The `id` field is used to filter WM_HOTKEY
/// messages from the thread's message queue (Windows).
pub struct Hotkey {
    pub id: i32,
}

#[cfg(windows)]
mod imp {
    use super::Hotkey;
    use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
    use windows::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
    use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};

    pub fn register(vk: u32) -> anyhow::Result<Hotkey> {
        const HOTKEY_ID: i32 = 1;
        // SAFETY: no window handle = global hotkey for this thread; the id
        // is locally unique and won't collide with other hotkeys in this
        // process.
        unsafe { RegisterHotKey(None, HOTKEY_ID, HOT_KEY_MODIFIERS(0x4000), vk)? };
        Ok(Hotkey { id: HOTKEY_ID })
    }

    pub fn triggered(id: i32) -> bool {
        // SAFETY: msg is a valid local; all windows are skipped (None) so
        // as not to disturb any other window's queue.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_HOTKEY && msg.wParam.0 == id as usize {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Hotkey;

    pub fn register(_vk: u32) -> anyhow::Result<Hotkey> {
        anyhow::bail!("screenshot hotkey is only supported on Windows");
    }

    pub fn triggered(_id: i32) -> bool {
        false
    }
}

/// Register a global hotkey (no modifier keys, with MOD_NOREPEAT so it
/// doesn't fire repeatedly while the key is held). Fails if the key is
/// already in use by another program — the caller may proceed without the
/// hotkey.
pub fn register(vk: u32) -> anyhow::Result<Hotkey> {
    imp::register(vk)
}

/// `true` if the hotkey has been pressed since the last poll (Windows).
pub fn triggered(id: i32) -> bool {
    imp::triggered(id)
}

/// Translate a hotkey key name into a Windows virtual-key code. Supports
/// `f1`-`f12` and `printscreen` (plus the aliases `prtsc`/`print`/`snapshot`).
pub fn parse_key_name(raw: &str) -> anyhow::Result<u32> {
    let s = raw.trim().to_ascii_lowercase();
    let s = s.as_str();
    if matches!(s, "printscreen" | "prtsc" | "print" | "snapshot") {
        return Ok(0x2C); // VK_SNAPSHOT
    }
    if let Some(num) = s.strip_prefix('f') {
        if let Ok(n) = num.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Ok(0x70 + n - 1); // VK_F1 = 0x70
            }
        }
    }
    anyhow::bail!("--screenshot-key: '{raw}' not recognized (use f1-f12 or printscreen).")
}