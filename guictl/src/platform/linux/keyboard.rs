use super::portal;
use crate::output::CmdError;

#[derive(Clone, Copy)]
enum Key {
    /// A Linux evdev key code: a physical key, whatever the layout.
    Code(i32),
    /// An X keysym: a character, wherever the current layout puts it.
    Sym(i32),
}

fn keysym(c: char) -> i32 {
    let code = c as u32;
    // Latin-1 keysyms equal their code points; the rest of Unicode is offset.
    (if (0x20..=0xff).contains(&code) { code } else { 0x0100_0000 | code }) as i32
}

fn key_code(name: &str) -> Result<Key, CmdError> {
    let lower = name.to_lowercase();
    let code = match lower.as_str() {
        "enter" | "return" => 28,
        "tab" => 15,
        "esc" | "escape" => 1,
        "space" => 57,
        "backspace" => 14,
        "delete" | "del" => 111,
        "insert" => 110,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "ctrl" | "control" => 29,
        "shift" => 42,
        "alt" => 56,
        "win" | "super" | "meta" => 125,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        "capslock" => 58,
        "numlock" => 69,
        "scrolllock" => 70,
        "printscreen" => 99,
        "pause" => 119,
        "menu" => 127,
        "numpad0" => 82,
        "numpad1" => 79,
        "numpad2" => 80,
        "numpad3" => 81,
        "numpad4" => 75,
        "numpad5" => 76,
        "numpad6" => 77,
        "numpad7" => 71,
        "numpad8" => 72,
        "numpad9" => 73,
        "numpadadd" => 78,
        "numpadsubtract" => 74,
        "numpadmultiply" => 55,
        "numpaddivide" => 98,
        "numpaddecimal" => 83,
        "playpause" => 164,
        "nexttrack" => 163,
        "prevtrack" => 165,
        "stop" => 166,
        "volumeup" => 115,
        "volumedown" => 114,
        "mute" => 113,
        // `+` joins the keys of a combination, so it has a name of its own.
        "plus" => return Ok(Key::Sym(keysym('+'))),
        _ => {
            let mut chars = lower.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_control() => Ok(Key::Sym(keysym(c))),
                _ => Err(CmdError::invalid_args(format!("unknown key: {name}"))),
            };
        }
    };
    Ok(Key::Code(code))
}

fn send(key: Key, down: bool) -> Result<(), CmdError> {
    let session = portal::session()?;
    match key {
        Key::Code(code) => session.keyboard_keycode(code, down),
        Key::Sym(sym) => session.keyboard_keysym(sym, down),
    }
}

fn tap(key: Key) -> Result<(), CmdError> {
    send(key, true)?;
    send(key, false)
}

/// Types by keysym: KWin presses whichever key (and Shift level) produces
/// each character, and lends a spare key to one the layout doesn't have.
pub fn type_text(text: &str) -> Result<(), CmdError> {
    super::system::check_unlocked()?;
    for c in text.chars() {
        let key = match c {
            '\n' => Key::Code(28),
            '\t' => Key::Code(15),
            c => Key::Sym(keysym(c)),
        };
        tap(key).map_err(|e| {
            CmdError::os(format!("could not type {c:?}: {}", e.message))
        })?;
    }
    Ok(())
}

pub fn key(name: &str, repeat: u32) -> Result<(), CmdError> {
    let key = key_code(name)?;
    super::system::check_unlocked()?;
    for _ in 0..repeat.max(1) {
        tap(key)?;
    }
    Ok(())
}

pub fn hotkey(combo: &str) -> Result<(), CmdError> {
    let keys: Vec<Key> = combo.split('+').map(key_code).collect::<Result<_, _>>()?;
    super::system::check_unlocked()?;
    for &key in &keys {
        send(key, true)?;
    }
    for &key in keys.iter().rev() {
        send(key, false)?;
    }
    Ok(())
}

pub fn hold(name: &str) -> Result<(), CmdError> {
    let key = key_code(name)?;
    super::system::check_unlocked()?;
    send(key, true)
}

pub fn release(name: &str) -> Result<(), CmdError> {
    send(key_code(name)?, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_map_to_keycodes() {
        assert!(matches!(key_code("enter"), Ok(Key::Code(28))));
        assert!(matches!(key_code("Return"), Ok(Key::Code(28))));
        assert!(matches!(key_code("ESC"), Ok(Key::Code(1))));
        assert!(matches!(key_code("ctrl"), Ok(Key::Code(29))));
        assert!(matches!(key_code("f12"), Ok(Key::Code(88))));
    }

    #[test]
    fn single_characters_map_to_keysyms() {
        assert!(matches!(key_code("a"), Ok(Key::Sym(0x61))));
        // Upper case names the same key as lower case.
        assert!(matches!(key_code("A"), Ok(Key::Sym(0x61))));
        assert!(matches!(key_code("/"), Ok(Key::Sym(0x2f))));
    }

    #[test]
    fn unknown_keys_are_invalid_args() {
        for name in ["", "notakey", "f13", "ctrl+a"] {
            assert_eq!(key_code(name).err().unwrap().code, "invalid_args", "{name}");
        }
    }

    #[test]
    fn keysyms_follow_the_x11_rule() {
        assert_eq!(keysym('A'), 0x41);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('€'), 0x0100_20ac);
        assert_eq!(keysym('ș'), 0x0100_0219);
    }
}
