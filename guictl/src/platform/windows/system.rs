use crate::output::CmdError;
use windows::core::PCWSTR;
use windows::Win32::System::Power::{
    SetSuspendState, SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
    ES_SYSTEM_REQUIRED,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

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
