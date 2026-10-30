use crate::cli::{MouseButton, ScrollDirection};
use crate::output::CmdError;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, VIRTUAL_KEY, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, SetCursorPos, WindowFromPoint};

const WHEEL_DELTA: i32 = 120;

pub fn position() -> Result<(i32, i32), CmdError> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.map_err(|e| CmdError::os(e.to_string()))?;
    Ok((point.x, point.y))
}

pub fn move_to(x: i32, y: i32) -> Result<(), CmdError> {
    super::system::check_unlocked()?;
    // Windows refuses without giving a reason when the pointer is captured
    // elsewhere: a virtual machine window holding the mouse, a UAC prompt,
    // or a program running as administrator in front.
    unsafe { SetCursorPos(x, y) }.map_err(|_| {
        super::window::check_access(unsafe { GetForegroundWindow() }, "The active window").err().unwrap_or_else(|| {
            CmdError::os(
                "Windows refused to move the mouse pointer. Something else has it: a virtual machine or remote desktop window that captured the mouse (the active window is usually that program) or a UAC prompt. Check `window active`",
            )
        })
    })
}

/// SendInput reports success even when Windows drops the event because of
/// who the window under the pointer belongs to.
fn check_under_pointer() -> Result<(), CmdError> {
    super::system::check_unlocked()?;
    let Ok((x, y)) = position() else { return Ok(()) };
    let hwnd = unsafe { WindowFromPoint(POINT { x, y }) };
    super::window::check_access(hwnd, "The window under the pointer")
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

fn virtual_key(button: MouseButton) -> VIRTUAL_KEY {
    match button {
        MouseButton::Left => VK_LBUTTON,
        MouseButton::Middle => VK_MBUTTON,
        MouseButton::Right => VK_RBUTTON,
    }
}

/// SendInput only queues the event, while SetCursorPos moves the pointer at
/// once. A move right after a press could overtake it, so the press would
/// land at the new position, or a release would come before the last move.
/// Waiting until Windows has handled the event keeps them in order. Returns
/// false if the button never got there.
fn wait_until_handled(button: MouseButton, down: bool) -> bool {
    for _ in 0..50 {
        let is_down = unsafe { GetAsyncKeyState(virtual_key(button).0 as i32) } as u16 & 0x8000 != 0;
        if is_down == down {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

pub fn press(button: MouseButton) -> Result<(), CmdError> {
    check_under_pointer()?;
    let (down, _) = down_up_flags(button);
    send_mouse_event(down)?;
    wait_until_handled(button, true);
    Ok(())
}

pub fn release(button: MouseButton) -> Result<(), CmdError> {
    let (_, up) = down_up_flags(button);
    send_mouse_event(up)?;
    super::window::check_release("The mouse button")?;
    if !wait_until_handled(button, false) {
        return Err(super::window::still_down("The mouse button"));
    }
    Ok(())
}

pub fn click(button: MouseButton, double: bool, at: Option<(i32, i32)>) -> Result<(), CmdError> {
    if let Some((x, y)) = at {
        move_to(x, y)?;
    }
    check_under_pointer()?;
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
    check_under_pointer()?;
    let delta = amount * WHEEL_DELTA;
    let (flags, data) = match direction {
        ScrollDirection::Up => (MOUSEEVENTF_WHEEL, delta),
        ScrollDirection::Down => (MOUSEEVENTF_WHEEL, -delta),
        ScrollDirection::Right => (MOUSEEVENTF_HWHEEL, delta),
        ScrollDirection::Left => (MOUSEEVENTF_HWHEEL, -delta),
    };
    send_mouse_event_raw(flags, data)
}
