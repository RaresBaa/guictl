use crate::output::CmdError;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12,
    VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP, VK_MENU,
    VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};

/// Keys whose VK code is ambiguous with a numpad key unless this flag is set
/// (standard Win32 SendInput requirement, see MSDN KEYBDINPUT.dwFlags).
fn is_extended_key(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_HOME | VK_END | VK_PRIOR | VK_NEXT
            | VK_INSERT | VK_DELETE
            | VK_MEDIA_PLAY_PAUSE | VK_MEDIA_NEXT_TRACK | VK_MEDIA_PREV_TRACK | VK_MEDIA_STOP
    )
}

fn key_code(name: &str) -> Result<VIRTUAL_KEY, CmdError> {
    let lower = name.to_lowercase();
    let vk = match lower.as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" | "escape" => VK_ESCAPE,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
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
        _ => {
            let chars: Vec<char> = name.chars().collect();
            if chars.len() == 1 && chars[0].is_ascii_alphanumeric() {
                VIRTUAL_KEY(chars[0].to_ascii_uppercase() as u16)
            } else {
                return Err(CmdError::invalid_args(format!("unknown key: {name}")));
            }
        }
    };
    Ok(vk)
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

pub fn type_text(text: &str) -> Result<(), CmdError> {
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
    let vk = key_code(name)?;
    for _ in 0..repeat.max(1) {
        send_key_event(vk, true)?;
        send_key_event(vk, false)?;
    }
    Ok(())
}

pub fn hotkey(combo: &str) -> Result<(), CmdError> {
    let codes: Vec<VIRTUAL_KEY> =
        combo.split('+').map(key_code).collect::<Result<_, _>>()?;
    for &vk in &codes {
        send_key_event(vk, true)?;
    }
    for &vk in codes.iter().rev() {
        send_key_event(vk, false)?;
    }
    Ok(())
}

pub(crate) fn tap(vk: VIRTUAL_KEY) -> Result<(), CmdError> {
    send_key_event(vk, true)?;
    send_key_event(vk, false)
}

pub fn hold(name: &str) -> Result<(), CmdError> {
    send_key_event(key_code(name)?, true)
}

pub fn release(name: &str) -> Result<(), CmdError> {
    send_key_event(key_code(name)?, false)
}
