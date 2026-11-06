use crate::cli::WindowState;
use crate::output::CmdError;
use serde::Serialize;
use windows::Win32::Foundation::{BOOL, CloseHandle, HANDLE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::ProcessStatus::GetModuleBaseNameW;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ,
};
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowRect, GW_OWNER, SW_SHOW, GetWindowTextW, GetWindowThreadProcessId,
    IsHungAppWindow, IsIconic, IsWindowVisible, IsZoomed, PostMessageW, SetForegroundWindow, SetWindowPos,
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
    /// The window this one belongs to, for a dialog or tool window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// True while the window ignores input, which almost always means one of
    /// its dialogs is open and waiting for an answer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
    /// True while the program isn't handling its window's messages: frozen,
    /// or busy for several seconds. Input sent to it does nothing for now.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_responding: bool,
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

fn integrity_level(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        // Aligned for the label, and room for it and the SID it points to.
        let mut buf = [0u64; 16];
        let mut len = 0u32;
        let read = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr().cast()),
            std::mem::size_of_val(&buf) as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        read.ok()?;
        let sid = (*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
        let last = (*GetSidSubAuthorityCount(sid) as u32).checked_sub(1)?;
        Some(*GetSidSubAuthority(sid, last))
    }
}

/// Fails if `hwnd` belongs to a process with more rights than this one, such
/// as a program running as administrator. Windows drops whatever this process
/// sends it, usually without saying so. `what` names the window in the error.
pub(super) fn check_access(hwnd: HWND, what: &str) -> Result<(), CmdError> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return Ok(());
    }
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return Ok(());
    };
    let theirs = integrity_level(process);
    let _ = unsafe { CloseHandle(process) };
    match (theirs, integrity_level(unsafe { GetCurrentProcess() })) {
        (Some(theirs), Some(ours)) if theirs > ours => Err(CmdError::permission_denied(format!(
            "{what} belongs to a program running as administrator, and Windows blocks a guictl that isn't. To go ahead, tell the person they will get a prompt, then retry with --elevate"
        ))),
        _ => Ok(()),
    }
}

/// Fails after sending a release, if a program running as administrator is in
/// front. Windows drops the release then, and also hides from this process
/// that the key is still down, so asking for its state would say it's up.
pub(super) fn check_release(what: &str) -> Result<(), CmdError> {
    check_access(unsafe { GetForegroundWindow() }, "").map_err(|_| {
        CmdError::permission_denied(format!(
            "{what} may still be held down: Windows drops a release while a program running as administrator is in front. Bring a normal window to the front with `window focus`, then release again"
        ))
    })
}

/// The error for a key or button that is still down after its release was sent.
pub(super) fn still_down(what: &str) -> CmdError {
    CmdError::os(format!("{what} is still held down after releasing it; someone may be holding it physically"))
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
        owner: unsafe { GetWindow(hwnd, GW_OWNER) }
            .ok()
            .filter(|owner| !owner.0.is_null())
            .map(|owner| format!("{:#x}", owner.0 as usize)),
        blocked: !unsafe { IsWindowEnabled(hwnd) }.as_bool(),
        not_responding: unsafe { IsHungAppWindow(hwnd) }.as_bool(),
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
    pid: Option<u32>,
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
    if title.is_none() && app.is_none() && pid.is_none() {
        return Err(CmdError::invalid_args(
            "specify one of --window, --active, --title, --app or --pid",
        ));
    }
    let windows = list(None)?;
    let found = windows.into_iter().find(|w| {
        title.is_none_or(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
            && app.is_none_or(|a| w.app.eq_ignore_ascii_case(a))
            && pid.is_none_or(|p| w.pid == p)
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

/// Whether `hwnd`, or another window of its process such as a dialog it
/// opened, is the one in front.
pub(super) fn in_front(hwnd: HWND) -> bool {
    let front = unsafe { GetForegroundWindow() };
    if front == hwnd {
        return true;
    }
    let (mut ours, mut theirs) = (0u32, 0u32);
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut ours));
        GetWindowThreadProcessId(front, Some(&mut theirs));
    }
    ours != 0 && ours == theirs
}

pub fn focus(hwnd: HWND) -> Result<(), CmdError> {
    unsafe {
        // SW_RESTORE would also un-maximize a maximized window.
        let _ = ShowWindow(hwnd, if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(hwnd);
    }
    if in_front(hwnd) {
        return Ok(());
    }
    // Windows only lets the process that has the foreground hand it on.
    // Sharing input state with that process's thread for a moment counts.
    unsafe {
        let front_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let this_thread = GetCurrentThreadId();
        let attached = front_thread != 0 && AttachThreadInput(this_thread, front_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(this_thread, front_thread, false);
        }
    }
    for _ in 0..20 {
        if in_front(hwnd) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(CmdError::os(
        "Windows kept another window in front, so keyboard and mouse input would not reach this one; check `window active`",
    ))
}

pub fn move_window(hwnd: HWND, x: i32, y: i32) -> Result<(), CmdError> {
    check_access(hwnd, "This window")?;
    unsafe { SetWindowPos(hwnd, HWND::default(), x, y, 0, 0, SWP_NOZORDER | SWP_NOSIZE) }
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn resize_window(hwnd: HWND, w: i32, h: i32) -> Result<(), CmdError> {
    check_access(hwnd, "This window")?;
    unsafe { SetWindowPos(hwnd, HWND::default(), 0, 0, w, h, SWP_NOZORDER | SWP_NOMOVE) }
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn set_state(hwnd: HWND, state: WindowState) -> Result<(), CmdError> {
    check_access(hwnd, "This window")?;
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
    check_access(hwnd, "This window")?;
    unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .map_err(|e| CmdError::os(e.to_string()))
}

/// Ends the window's process and returns its id.
pub fn kill(hwnd: HWND) -> Result<u32, CmdError> {
    check_access(hwnd, "This window")?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return Err(CmdError::not_found("no such window"));
    }
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| CmdError::permission_denied(format!("could not end process {pid}: {e}")))?;
        let ended = TerminateProcess(process, 1);
        let _ = CloseHandle(process);
        ended.map_err(|e| CmdError::os(format!("could not end process {pid}: {e}")))?;
    }
    Ok(pid)
}

pub fn pin(hwnd: HWND, on: bool) -> Result<(), CmdError> {
    check_access(hwnd, "This window")?;
    let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE) }
        .map_err(|e| CmdError::os(e.to_string()))
}
