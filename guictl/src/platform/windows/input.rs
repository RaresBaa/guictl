//! Noticing that a person is using the mouse or keyboard while guictl drives
//! them, for `batch --stop-on-user-input`. Low-level hooks see every input
//! event and mark the ones a program sent, guictl's own among them, as
//! injected; anything else came from a physical device.

use crate::output::CmdError;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED,
    MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL,
};

static SEEN: AtomicBool = AtomicBool::new(false);

pub struct Watch;

impl Watch {
    /// Whether someone has used the mouse or keyboard since the last call.
    pub fn take(&self) -> bool {
        SEEN.swap(false, Ordering::SeqCst)
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && (*(lparam.0 as *const MSLLHOOKSTRUCT)).flags & LLMHF_INJECTED == 0 {
            SEEN.store(true, Ordering::SeqCst);
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && (*(lparam.0 as *const KBDLLHOOKSTRUCT)).flags.0 & LLKHF_INJECTED.0 == 0 {
            SEEN.store(true, Ordering::SeqCst);
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Starts watching. The hooks run on a thread of their own that pumps
/// messages, as low-level hooks require, until the process exits.
pub fn watch() -> Result<Watch, CmdError> {
    let (ready, started) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        let hooks = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), HINSTANCE::default(), 0)
            .and_then(|_| SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), HINSTANCE::default(), 0));
        let failed = hooks.is_err();
        let _ = ready.send(hooks.map(|_| ()).map_err(|e| e.to_string()));
        if failed {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
    started
        .recv()
        .map_err(|e| CmdError::os(e.to_string()))?
        .map_err(|e| CmdError::os(format!("could not watch for user input: {e}")))?;
    SEEN.store(false, Ordering::SeqCst);
    Ok(Watch)
}
