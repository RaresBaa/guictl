use crate::output::CmdError;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT, HANDLE_FLAGS};
use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
use windows::Win32::System::RemoteDesktop::{
    WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW, WTS_CURRENT_SERVER_HANDLE,
    WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::System::Power::{
    SetSuspendState, SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
    ES_SYSTEM_REQUIRED,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Starts a program that outlives guictl. Windows hands every inheritable
/// handle to a child, and guictl's stdout is usually an inheritable pipe
/// from the caller's shell; a GUI program holding it open would keep a
/// caller that reads guictl's output waiting until the program exits.
pub fn spawn(program: &str, args: &[String]) -> Result<u32, CmdError> {
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        if let Ok(handle) = unsafe { GetStdHandle(which) } {
            let _ = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) };
        }
    }
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|child| child.id())
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn open(target: &str) -> Result<(), CmdError> {
    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns an HINSTANCE that is actually an error code when <= 32.
    if (result.0 as isize) <= 32 {
        return Err(CmdError::os(format!(
            "ShellExecute failed with code {}",
            result.0 as isize
        )));
    }
    Ok(())
}

pub fn lock() -> Result<(), CmdError> {
    unsafe { LockWorkStation() }.map_err(|e| CmdError::os(e.to_string()))
}

pub fn sleep() -> Result<(), CmdError> {
    let ok = unsafe { SetSuspendState(false, false, false) };
    if !ok.as_bool() {
        return Err(CmdError::os("SetSuspendState failed"));
    }
    Ok(())
}

/// Fails while the screen is locked: Windows lets input through, but it goes
/// to the lock screen rather than the apps, and typing lands in the password
/// box. If the state can't be read, input goes ahead as it always did.
pub(super) fn check_unlocked() -> Result<(), CmdError> {
    if is_locked().unwrap_or(false) {
        return Err(CmdError::permission_denied(
            "The screen is locked, so mouse and keyboard input would go to the lock screen, not the apps. Tell the person and wait until they unlock it; `system status` shows when",
        ));
    }
    Ok(())
}

fn is_locked() -> Result<bool, CmdError> {
    unsafe {
        let mut buffer = PWSTR::null();
        let mut len = 0u32;
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buffer,
            &mut len,
        )
        .map_err(|e| CmdError::os(format!("could not read the session's state: {e}")))?;
        let info = &*(buffer.0 as *const WTSINFOEXW);
        let flags = info.Data.WTSInfoExLevel1.SessionFlags;
        WTSFreeMemory(buffer.0 as *mut _);
        Ok(flags == WTS_SESSIONSTATE_LOCK as i32)
    }
}

/// Whether this session is locked, and how long since the last mouse or
/// keyboard input to it, which includes input guictl itself sent.
pub fn status() -> Result<(bool, Option<u64>), CmdError> {
    let locked = is_locked()?;
    let mut input = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe { GetLastInputInfo(&mut input) }.ok().map_err(|e| CmdError::os(e.to_string()))?;
    // Both are tick counts that wrap around together after 49.7 days.
    let idle = unsafe { GetTickCount() }.wrapping_sub(input.dwTime);
    Ok((locked, Some(idle as u64)))
}

/// Holds the display/system awake for `duration_ms`, then releases it. Because this is a
/// one-off process, the execution state can only last as long as this call blocks - there
/// is no persistent session to hold it afterward.
pub fn keep_awake_for(duration_ms: u64) -> Result<(), CmdError> {
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
    }
    std::thread::sleep(std::time::Duration::from_millis(duration_ms));
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS);
    }
    Ok(())
}
