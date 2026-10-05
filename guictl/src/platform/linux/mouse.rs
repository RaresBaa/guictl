use super::{kwin, portal};
use crate::cli::{MouseButton, ScrollDirection};
use crate::output::CmdError;

pub fn position() -> Result<(i32, i32), CmdError> {
    let value = kwin::run("var p = workspace.cursorPos;\nreturn [Math.round(p.x), Math.round(p.y)];")?;
    serde_json::from_value(value)
        .map_err(|e| CmdError::os(format!("unexpected cursor position from KWin: {e}")))
}

/// The portal's absolute pointer motion only works relative to a screen-cast
/// stream, which would mean a second consent and a running capture. Relative
/// motion needs neither, so this moves by the distance left to go and checks
/// where the cursor ended up, repeating if something scaled the motion.
pub fn move_to(x: i32, y: i32) -> Result<(), CmdError> {
    let session = portal::session()?;
    let mut last = None;
    for _ in 0..6 {
        let (cx, cy) = position()?;
        // Not moving any more means the target is past a screen edge.
        if (cx, cy) == (x, y) || last == Some((cx, cy)) {
            break;
        }
        last = Some((cx, cy));
        session.pointer_motion((x - cx) as f64, (y - cy) as f64)?;
    }
    Ok(())
}

// Linux evdev button codes (BTN_LEFT, BTN_RIGHT, BTN_MIDDLE).
fn button_code(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
    }
}

pub fn press(button: MouseButton) -> Result<(), CmdError> {
    portal::session()?.pointer_button(button_code(button), true)
}

pub fn release(button: MouseButton) -> Result<(), CmdError> {
    portal::session()?.pointer_button(button_code(button), false)
}

pub fn click(button: MouseButton, double: bool, at: Option<(i32, i32)>) -> Result<(), CmdError> {
    if let Some((x, y)) = at {
        move_to(x, y)?;
    }
    for _ in 0..if double { 2 } else { 1 } {
        press(button)?;
        release(button)?;
    }
    Ok(())
}

pub fn scroll(amount: i32, direction: ScrollDirection) -> Result<(), CmdError> {
    let (axis, steps) = match direction {
        ScrollDirection::Up => (0, -amount),
        ScrollDirection::Down => (0, amount),
        ScrollDirection::Left => (1, -amount),
        ScrollDirection::Right => (1, amount),
    };
    portal::session()?.pointer_axis_discrete(axis, steps)
}
