use crate::output::CmdError;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    VK_ADD, VK_APPS, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME,
    VK_INSERT, VK_LEFT, VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP,
    VK_MENU, VK_MULTIPLY, VK_NEXT, VK_NUMLOCK, VK_NUMPAD0, VK_PAUSE, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SCROLL,
    VK_SHIFT, VK_SNAPSHOT, VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};

/// Keys whose VK code is ambiguous with a numpad key unless this flag is set
/// (standard Win32 SendInput requirement, see MSDN KEYBDINPUT.dwFlags).
fn is_extended_key(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_HOME | VK_END | VK_PRIOR | VK_NEXT
            | VK_INSERT | VK_DELETE
            | VK_MEDIA_PLAY_PAUSE | VK_MEDIA_NEXT_TRACK | VK_MEDIA_PREV_TRACK | VK_MEDIA_STOP
            | VK_VOLUME_UP | VK_VOLUME_DOWN | VK_VOLUME_MUTE
            | VK_SNAPSHOT | VK_NUMLOCK | VK_DIVIDE | VK_APPS
    )
}

/// The keys to hold down, in order, to press the named key: more than one for
/// a character the layout puts behind Shift or AltGr, such as "?".
fn key_codes(name: &str) -> Result<Vec<VIRTUAL_KEY>, CmdError> {
    let lower = name.to_lowercase();
    let vk = match lower.as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" | "escape" => VK_ESCAPE,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" => VK_INSERT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" => VK_MENU,
        "win" | "super" | "meta" => VK_LWIN,
        "f1" => VK_F1,
        "f2" => VK_F2,
        "f3" => VK_F3,
        "f4" => VK_F4,
        "f5" => VK_F5,
        "f6" => VK_F6,
        "f7" => VK_F7,
        "f8" => VK_F8,
        "f9" => VK_F9,
        "f10" => VK_F10,
        "f11" => VK_F11,
        "f12" => VK_F12,
        "capslock" => VK_CAPITAL,
        "numlock" => VK_NUMLOCK,
        "scrolllock" => VK_SCROLL,
        "printscreen" => VK_SNAPSHOT,
        "pause" => VK_PAUSE,
        "menu" => VK_APPS,
        "numpadadd" => VK_ADD,
        "numpadsubtract" => VK_SUBTRACT,
        "numpadmultiply" => VK_MULTIPLY,
        "numpaddivide" => VK_DIVIDE,
        "numpaddecimal" => VK_DECIMAL,
        "playpause" => VK_MEDIA_PLAY_PAUSE,
        "nexttrack" => VK_MEDIA_NEXT_TRACK,
        "prevtrack" => VK_MEDIA_PREV_TRACK,
        "stop" => VK_MEDIA_STOP,
        "volumeup" => VK_VOLUME_UP,
        "volumedown" => VK_VOLUME_DOWN,
        "mute" => VK_VOLUME_MUTE,
        // `+` joins the keys of a combination, so it has a name of its own.
        "plus" => return character_keys('+', name),
        _ => {
            if let Some(digit) = lower.strip_prefix("numpad").and_then(|d| d.parse::<u16>().ok()).filter(|d| *d <= 9) {
                return Ok(vec![VIRTUAL_KEY(VK_NUMPAD0.0 + digit)]);
            }
            let mut chars = lower.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => Ok(vec![VIRTUAL_KEY(c.to_ascii_uppercase() as u16)]),
                (Some(c), None) if !c.is_control() => character_keys(c, name),
                _ => Err(CmdError::invalid_args(format!("unknown key: {name}"))),
            };
        }
    };
    Ok(vec![vk])
}

/// Where the current keyboard layout puts a character.
fn character_keys(c: char, name: &str) -> Result<Vec<VIRTUAL_KEY>, CmdError> {
    let not_there = || {
        CmdError::invalid_args(format!(
            "the current keyboard layout has no key for {name}; use `keyboard type` for text"
        ))
    };
    let unit = u16::try_from(c as u32).map_err(|_| not_there())?;
    let scan = unsafe { VkKeyScanW(unit) };
    if scan == -1 {
        return Err(not_there());
    }
    let (vk, modifiers) = (scan as u16 & 0xff, scan as u16 >> 8);
    let mut keys = Vec::new();
    for (bit, modifier) in [(1, VK_SHIFT), (2, VK_CONTROL), (4, VK_MENU)] {
        if modifiers & bit != 0 {
            keys.push(modifier);
        }
    }
    keys.push(VIRTUAL_KEY(vk));
    Ok(keys)
}

pub(crate) fn send_key_event(vk: VIRTUAL_KEY, down: bool) -> Result<(), CmdError> {
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
    if is_extended_key(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        return Err(CmdError::os("SendInput sent 0 events"));
    }
    Ok(())
}

fn send_unicode_char(unit: u16, down: bool) -> Result<(), CmdError> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: if down {
                    KEYEVENTF_UNICODE
                } else {
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        return Err(CmdError::os("SendInput sent 0 events"));
    }
    Ok(())
}

/// SendInput reports success even when Windows drops the event because of
/// who the active window belongs to.
fn check_active() -> Result<(), CmdError> {
    super::system::check_unlocked()?;
    super::window::check_access(unsafe { GetForegroundWindow() }, "The active window")
}

fn press_all(codes: &[VIRTUAL_KEY]) -> Result<(), CmdError> {
    for &vk in codes {
        send_key_event(vk, true)?;
    }
    Ok(())
}

fn release_all(codes: &[VIRTUAL_KEY]) -> Result<(), CmdError> {
    for &vk in codes.iter().rev() {
        send_key_event(vk, false)?;
    }
    Ok(())
}

pub fn type_text(text: &str) -> Result<(), CmdError> {
    check_active()?;
    let mut buf = [0u16; 2];
    for c in text.chars() {
        for unit in c.encode_utf16(&mut buf) {
            send_unicode_char(*unit, true)?;
            send_unicode_char(*unit, false)?;
        }
    }
    Ok(())
}

pub fn key(name: &str, repeat: u32) -> Result<(), CmdError> {
    let codes = key_codes(name)?;
    check_active()?;
    for _ in 0..repeat.max(1) {
        press_all(&codes)?;
        release_all(&codes)?;
    }
    Ok(())
}

pub fn hotkey(combo: &str) -> Result<(), CmdError> {
    let mut codes: Vec<VIRTUAL_KEY> = Vec::new();
    for name in combo.split('+') {
        for vk in key_codes(name)? {
            // "shift+?" would otherwise press Shift twice.
            if !codes.contains(&vk) {
                codes.push(vk);
            }
        }
    }
    check_active()?;
    press_all(&codes)?;
    release_all(&codes)
}

pub(crate) fn tap(vk: VIRTUAL_KEY) -> Result<(), CmdError> {
    send_key_event(vk, true)?;
    send_key_event(vk, false)
}

pub fn hold(name: &str) -> Result<(), CmdError> {
    let codes = key_codes(name)?;
    check_active()?;
    press_all(&codes)
}

pub fn release(name: &str) -> Result<(), CmdError> {
    let codes = key_codes(name)?;
    release_all(&codes)?;
    let what = format!("The {name} key");
    super::window::check_release(&what)?;
    // SendInput only queues the event, so give Windows a moment to handle it.
    for _ in 0..50 {
        if codes.iter().all(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 == 0) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Err(super::window::still_down(&what))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_map_to_one_key() {
        assert_eq!(key_codes("Enter").unwrap(), [VK_RETURN]);
        assert_eq!(key_codes("numpad7").unwrap(), [VIRTUAL_KEY(VK_NUMPAD0.0 + 7)]);
        assert_eq!(key_codes("printscreen").unwrap(), [VK_SNAPSHOT]);
        assert_eq!(key_codes("A").unwrap(), key_codes("a").unwrap());
    }

    #[test]
    fn unknown_keys_are_invalid_args() {
        for name in ["", "notakey", "f13", "numpad10", "ctrl+a", "\u{1F600}"] {
            assert_eq!(key_codes(name).err().unwrap().code, "invalid_args", "{name}");
        }
    }
}
