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

    pub fn register(mods: u32, vk: u32, id: i32) -> anyhow::Result<Hotkey> {
        // SAFETY: no window handle = global hotkey for this thread; the id
        // is locally unique and won't collide with other hotkeys in this
        // process. 0x4000 = MOD_NOREPEAT.
        unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(mods | 0x4000), vk)? };
        Ok(Hotkey { id })
    }

    /// Presses that were read from the queue for another hotkey, waiting for their turn.
    static PENDING: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

    pub fn triggered(id: i32) -> bool {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: msg is a valid local; all windows are skipped (None) so
        // as not to disturb any other window's queue. Every hotkey press is
        // kept (not dropped) when it is for another hotkey.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_HOTKEY {
                    pending.push(msg.wParam.0 as i32);
                }
            }
        }
        // one press = one action (the presses for other hotkeys stay for their turn)
        let had = pending.contains(&id);
        pending.retain(|p| *p != id);
        had
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Hotkey;

    pub fn register(_mods: u32, _vk: u32, _id: i32) -> anyhow::Result<Hotkey> {
        anyhow::bail!("global hotkeys are only supported on Windows");
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
    imp::register(0, vk, 1)
}

/// Register a global hotkey with modifier keys (see [`parse_combo`]); `id` must differ from
/// the other hotkeys of the process (the screenshot one is 1).
pub fn register_combo(mods: u32, vk: u32, id: i32) -> anyhow::Result<Hotkey> {
    imp::register(mods, vk, id)
}

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// `ctrl+alt+p`, `shift+f9`, `f9`, `win+1`: modifiers (`ctrl`, `alt`, `shift`, `win`) and one
/// key (`f1`-`f12`, `a`-`z`, `0`-`9`, `printscreen`), joined with `+`. Returns the Windows
/// modifier flags and virtual-key code. A key alone is allowed only for the function keys.
pub fn parse_combo(raw: &str) -> anyhow::Result<(u32, u32)> {
    let s = raw.trim().to_ascii_lowercase();
    let mut mods = 0u32;
    let mut key: Option<u32> = None;
    for part in s.split('+').map(str::trim) {
        let m = match part {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "windows" | "super" => MOD_WIN,
            _ => 0,
        };
        if m != 0 {
            mods |= m;
            continue;
        }
        if key.is_some() {
            anyhow::bail!("'{raw}': only one key besides ctrl / alt / shift / win");
        }
        let one = part.chars().next().filter(|_| part.chars().count() == 1);
        key = Some(match one {
            Some(c) if c.is_ascii_lowercase() => c.to_ascii_uppercase() as u32,
            Some(c) if c.is_ascii_digit() => c as u32,
            _ => parse_key_name(part).map_err(|_| anyhow::anyhow!("'{raw}': key not recognized (f1-f12, a-z, 0-9, printscreen; with ctrl, alt, shift, win)"))?,
        });
    }
    let vk = key.ok_or_else(|| anyhow::anyhow!("'{raw}': no key"))?;
    let function_key = (0x70..=0x7B).contains(&vk) || vk == 0x2C;
    if mods == 0 && !function_key {
        anyhow::bail!("'{raw}': a letter or digit needs ctrl, alt, shift or win with it (for example ctrl+alt+p)");
    }
    Ok((mods, vk))
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_are_read_with_their_modifiers() {
        assert_eq!(parse_combo("ctrl+alt+p").unwrap(), (MOD_CONTROL | MOD_ALT, 'P' as u32));
        assert_eq!(parse_combo(" Shift + F9 ").unwrap(), (MOD_SHIFT, 0x78));
        assert_eq!(parse_combo("f9").unwrap(), (0, 0x78));
        assert_eq!(parse_combo("win+1").unwrap(), (MOD_WIN, '1' as u32));
        assert_eq!(parse_combo("printscreen").unwrap(), (0, 0x2C));
        for bad in ["p", "ctrl", "", "ctrl+a+b", "ctrl+f13", "ctrl+??"] {
            assert!(parse_combo(bad).is_err(), "{bad:?}");
        }
    }
}
