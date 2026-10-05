use crate::cli::{MouseButton, ScrollDirection};
use crate::output::CmdError;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

const WHEEL_DELTA: i32 = 120;

pub fn position() -> Result<(i32, i32), CmdError> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.map_err(|e| CmdError::os(e.to_string()))?;
    Ok((point.x, point.y))
}

pub fn move_to(x: i32, y: i32) -> Result<(), CmdError> {
    unsafe { SetCursorPos(x, y) }.map_err(|e| CmdError::os(e.to_string()))
}

fn send_mouse_event_raw(flags: MOUSE_EVENT_FLAGS, mouse_data: i32) -> Result<(), CmdError> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data as u32,
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

fn send_mouse_event(flags: MOUSE_EVENT_FLAGS) -> Result<(), CmdError> {
    send_mouse_event_raw(flags, 0)
}

fn down_up_flags(button: MouseButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
    }
}

pub fn press(button: MouseButton) -> Result<(), CmdError> {
    let (down, _) = down_up_flags(button);
    send_mouse_event(down)
}

pub fn release(button: MouseButton) -> Result<(), CmdError> {
    let (_, up) = down_up_flags(button);
    send_mouse_event(up)
}

pub fn click(button: MouseButton, double: bool, at: Option<(i32, i32)>) -> Result<(), CmdError> {
    if let Some((x, y)) = at {
        move_to(x, y)?;
    }
    let (down, up) = down_up_flags(button);
    send_mouse_event(down)?;
    send_mouse_event(up)?;
    if double {
        send_mouse_event(down)?;
        send_mouse_event(up)?;
    }
    Ok(())
}

pub fn scroll(amount: i32, direction: ScrollDirection) -> Result<(), CmdError> {
    let delta = amount * WHEEL_DELTA;
    let (flags, data) = match direction {
        ScrollDirection::Up => (MOUSEEVENTF_WHEEL, delta),
        ScrollDirection::Down => (MOUSEEVENTF_WHEEL, -delta),
        ScrollDirection::Right => (MOUSEEVENTF_HWHEEL, delta),
        ScrollDirection::Left => (MOUSEEVENTF_HWHEEL, -delta),
    };
    send_mouse_event_raw(flags, data)
}
