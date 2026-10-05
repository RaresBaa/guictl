use crate::output::CmdError;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICONDATAW_0, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadIconW, RegisterClassW, HICON, HMENU,
    HWND_MESSAGE, IDI_APPLICATION, WNDCLASSW, WS_OVERLAPPED,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_into(dst: &mut [u16], s: &str) {
    for (slot, unit) in dst.iter_mut().zip(s.encode_utf16().chain(std::iter::once(0))) {
        *slot = unit;
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Shows a balloon-tip notification. This needs a window to own the tray icon,
/// so a throwaway message-only one is created, used, and torn down in one shot.
pub fn send(title: &str, body: &str) -> Result<(), CmdError> {
    let class_name = wide("guictl_notify_wndclass");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    unsafe { RegisterClassW(&wc) };

    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            HMENU::default(),
            None,
            None,
        )
    }
    .map_err(|e| CmdError::os(e.to_string()))?;

    let icon: HICON = unsafe { LoadIconW(None, IDI_APPLICATION) }.map_err(|e| CmdError::os(e.to_string()))?;

    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_INFO | NIF_TIP,
        hIcon: icon,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    copy_into(&mut data.szTip, "guictl");
    copy_into(&mut data.szInfoTitle, title);
    copy_into(&mut data.szInfo, body);

    let result = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
    if !result.as_bool() {
        unsafe { DestroyWindow(hwnd).ok() };
        return Err(CmdError::os("Shell_NotifyIconW(NIM_ADD) failed"));
    }

    // Modern Windows only renders the balloon as a toast once the icon opts
    // into version 4 behavior; without this it's silently swallowed.
    data.Anonymous = NOTIFYICONDATAW_0 { uVersion: NOTIFYICON_VERSION_4 };
    let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };

    // Give the shell a moment to pick up and display the balloon before the
    // icon (and its owning window) goes away.
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    unsafe { DestroyWindow(hwnd).ok() };
    Ok(())
}
