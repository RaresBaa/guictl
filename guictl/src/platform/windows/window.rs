use crate::cli::WindowState;
use crate::output::CmdError;
use serde::Serialize;
use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::ProcessStatus::GetModuleBaseNameW;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, IsZoomed, PostMessageW, SetForegroundWindow, SetWindowPos,
    ShowWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE,
    SW_MINIMIZE, SW_RESTORE, WM_CLOSE,
};

pub type WindowHandle = HWND;

#[derive(Serialize)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub app: String,
    pub pid: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub state: String,
    pub monitor: String,
    /// Stacking rank among all visible top-level windows, 0 = topmost.
    pub z_order: u32,
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

pub(crate) fn process_name(pid: u32) -> String {
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid) {
            Ok(handle) => {
                let mut buf = [0u16; 260];
                let len = GetModuleBaseNameW(handle, None, &mut buf);
                let _ = CloseHandle(handle);
                String::from_utf16_lossy(&buf[..len as usize])
            }
            Err(_) => String::new(),
        }
    }
}

fn window_state(hwnd: HWND) -> &'static str {
    if unsafe { IsIconic(hwnd) }.as_bool() {
        "minimized"
    } else if unsafe { IsZoomed(hwnd) }.as_bool() {
        "maximized"
    } else {
        "normal"
    }
}

fn window_monitor(hwnd: HWND) -> String {
    let hmonitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    format!("{:#x}", hmonitor.0 as usize)
}

fn window_info(hwnd: HWND) -> WindowInfo {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    WindowInfo {
        id: format!("{:#x}", hwnd.0 as usize),
        title: window_title(hwnd),
        app: process_name(pid),
        pid,
        x: rect.left,
        y: rect.top,
        w: rect.right - rect.left,
        h: rect.bottom - rect.top,
        state: window_state(hwnd).to_string(),
        monitor: window_monitor(hwnd),
        z_order: 0,
    }
}

extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let title = window_title(hwnd);
    if visible && !title.is_empty() {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };
        list.push(window_info(hwnd));
    }
    BOOL(1)
}

pub fn list(app_filter: Option<&str>) -> Result<Vec<WindowInfo>, CmdError> {
    let mut windows: Vec<WindowInfo> = Vec::new();
    let lparam = LPARAM(&mut windows as *mut Vec<WindowInfo> as isize);
    unsafe { EnumWindows(Some(enum_proc), lparam) }.map_err(|e| CmdError::os(e.to_string()))?;
    // EnumWindows returns windows topmost-first, so the enumeration index is the z-order
    // rank. Assign it before filtering so ranks stay comparable across different filters.
    for (i, w) in windows.iter_mut().enumerate() {
        w.z_order = i as u32;
    }
    if let Some(app) = app_filter {
        windows.retain(|w| w.app.eq_ignore_ascii_case(app));
    }
    Ok(windows)
}

pub fn active() -> Result<WindowInfo, CmdError> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(CmdError::not_found("no active window"));
    }
    let id = format!("{:#x}", hwnd.0 as usize);
    list(None)?
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| CmdError::not_found("active window not found among visible windows"))
}

pub(crate) fn parse_id(id: &str) -> Result<HWND, CmdError> {
    let raw = usize::from_str_radix(id.trim_start_matches("0x"), 16)
        .map_err(|_| CmdError::invalid_args(format!("invalid window id: {id}")))?;
    Ok(HWND(raw as *mut _))
}

pub fn resolve(
    id: Option<&str>,
    active_flag: bool,
    title: Option<&str>,
    app: Option<&str>,
) -> Result<HWND, CmdError> {
    if active_flag {
        let hwnd = unsafe { GetForegroundWindow() };
        return if hwnd.0.is_null() {
            Err(CmdError::not_found("no active window"))
        } else {
            Ok(hwnd)
        };
    }
    if let Some(id) = id {
        return parse_id(id);
    }
    if title.is_none() && app.is_none() {
        return Err(CmdError::invalid_args(
            "specify one of --window, --active, --title or --app",
        ));
    }
    let windows = list(None)?;
    let found = windows.into_iter().find(|w| {
        title.map_or(true, |t| w.title.to_lowercase().contains(&t.to_lowercase()))
            && app.map_or(true, |a| w.app.eq_ignore_ascii_case(a))
    });
    match found {
        Some(w) => parse_id(&w.id),
        None => Err(CmdError::not_found("no matching window")),
    }
}

pub fn rect(hwnd: HWND) -> Result<(i32, i32, i32, i32), CmdError> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| CmdError::os(e.to_string()))?;
    Ok((rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top))
}

pub fn focus(hwnd: HWND) -> Result<(), CmdError> {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

pub fn move_window(hwnd: HWND, x: i32, y: i32) -> Result<(), CmdError> {
    unsafe { SetWindowPos(hwnd, HWND::default(), x, y, 0, 0, SWP_NOZORDER | SWP_NOSIZE) }
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn resize_window(hwnd: HWND, w: i32, h: i32) -> Result<(), CmdError> {
    unsafe { SetWindowPos(hwnd, HWND::default(), 0, 0, w, h, SWP_NOZORDER | SWP_NOMOVE) }
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn set_state(hwnd: HWND, state: WindowState) -> Result<(), CmdError> {
    let cmd = match state {
        WindowState::Minimize => SW_MINIMIZE,
        WindowState::Maximize | WindowState::Fullscreen => SW_MAXIMIZE,
        WindowState::Restore => SW_RESTORE,
    };
    unsafe {
        let _ = ShowWindow(hwnd, cmd);
    }
    Ok(())
}

pub fn close(hwnd: HWND) -> Result<(), CmdError> {
    unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn pin(hwnd: HWND, on: bool) -> Result<(), CmdError> {
    let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE) }
        .map_err(|e| CmdError::os(e.to_string()))
}
