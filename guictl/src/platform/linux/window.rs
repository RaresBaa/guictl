use super::kwin::{self, js_str};
use crate::cli::WindowState;
use crate::output::CmdError;
use serde::{Deserialize, Serialize};

/// KWin's internal window UUID. Kept as a number rather than a string so the
/// handle is `Copy`, like `HWND` on Windows.
pub type WindowHandle = u128;

#[derive(Serialize)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    /// Executable name of the owning process.
    pub app: String,
    /// The window's resource class / app id, e.g. `org.kde.dolphin`.
    pub class: String,
    pub pid: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub state: String,
    pub monitor: String,
    /// Stacking rank among the listed windows, 0 = topmost.
    pub z_order: u32,
}

#[derive(Deserialize)]
struct RawWindow {
    id: String,
    title: String,
    class: String,
    pid: i64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    state: String,
    monitor: String,
    z: i64,
    listed: bool,
    active: bool,
}

const PRELUDE: &str = r#"
function windowId(w) {
    return String(w.internalId).replace(/[{}]/g, "");
}
function findWindow(id) {
    var all = workspace.windowList();
    for (var i = 0; i < all.length; i++) {
        if (windowId(all[i]) === id) return all[i];
    }
    throw "not_found: no window with id " + id;
}
function describe(w) {
    var g = w.frameGeometry;
    var state = w.minimized ? "minimized"
        : w.fullScreen ? "fullscreen"
        : w.maximizeMode === 3 ? "maximized"
        : "normal";
    return {
        id: windowId(w), title: w.caption, class: w.resourceClass, pid: w.pid,
        x: Math.round(g.x), y: Math.round(g.y), w: Math.round(g.width), h: Math.round(g.height),
        state: state, monitor: w.output ? w.output.name : "", z: w.stackingOrder,
        // Panels, the desktop, docks and popups are windows too as far as KWin
        // is concerned; only normal windows and dialogs are worth listing.
        listed: w.normalWindow || w.dialog, active: w === workspace.activeWindow
    };
}
"#;

fn process_name(pid: u32) -> String {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| {
            std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
        })
        .unwrap_or_default()
}

fn window_info(raw: RawWindow) -> WindowInfo {
    let pid = u32::try_from(raw.pid).unwrap_or(0);
    let app = match process_name(pid) {
        name if name.is_empty() => raw.class.clone(),
        name => name,
    };
    WindowInfo {
        id: raw.id,
        title: raw.title,
        app,
        class: raw.class,
        pid,
        x: raw.x,
        y: raw.y,
        w: raw.w,
        h: raw.h,
        state: raw.state,
        monitor: raw.monitor,
        z_order: 0,
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, CmdError> {
    serde_json::from_value(value)
        .map_err(|e| CmdError::os(format!("unexpected window data from KWin: {e}")))
}

fn matches_app(w: &WindowInfo, app: &str) -> bool {
    w.app.eq_ignore_ascii_case(app) || w.class.eq_ignore_ascii_case(app)
}

/// Every listable window, topmost first, with whether it's the active one.
/// With `include_active`, the active window is included even if it isn't a
/// listable kind.
fn snapshot(include_active: bool) -> Result<Vec<(WindowInfo, bool)>, CmdError> {
    let value = kwin::run(&format!("{PRELUDE}\nreturn workspace.windowList().map(describe);"))?;
    let mut raw: Vec<RawWindow> = parse(value)?;
    raw.retain(|w| w.listed || (include_active && w.active));
    raw.sort_by_key(|w| std::cmp::Reverse(w.z));
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, w)| {
            let active = w.active;
            let mut info = window_info(w);
            info.z_order = i as u32;
            (info, active)
        })
        .collect())
}

pub fn list(app_filter: Option<&str>) -> Result<Vec<WindowInfo>, CmdError> {
    // Ranks are assigned before filtering so they stay comparable across filters.
    let mut windows: Vec<WindowInfo> = snapshot(false)?.into_iter().map(|(w, _)| w).collect();
    if let Some(app) = app_filter {
        windows.retain(|w| matches_app(w, app));
    }
    Ok(windows)
}

pub fn active() -> Result<WindowInfo, CmdError> {
    snapshot(true)?
        .into_iter()
        .find(|(_, active)| *active)
        .map(|(w, _)| w)
        .ok_or_else(|| CmdError::not_found("no active window"))
}

pub(crate) fn parse_id(id: &str) -> Result<WindowHandle, CmdError> {
    let hex: String = id.chars().filter(|c| !matches!(c, '{' | '}' | '-')).collect();
    if hex.len() != 32 {
        return Err(CmdError::invalid_args(format!("invalid window id: {id}")));
    }
    u128::from_str_radix(&hex, 16)
        .map_err(|_| CmdError::invalid_args(format!("invalid window id: {id}")))
}

pub(crate) fn format_id(handle: WindowHandle) -> String {
    let s = format!("{handle:032x}");
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

pub fn resolve(
    id: Option<&str>,
    active_flag: bool,
    title: Option<&str>,
    app: Option<&str>,
) -> Result<WindowHandle, CmdError> {
    if active_flag {
        return parse_id(&active()?.id);
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
        title.is_none_or(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
            && app.is_none_or(|a| matches_app(w, a))
    });
    match found {
        Some(w) => parse_id(&w.id),
        None => Err(CmdError::not_found("no matching window")),
    }
}

/// Runs `action` inside KWin with `w` bound to the window.
pub(super) fn with_window(handle: WindowHandle, action: &str) -> Result<serde_json::Value, CmdError> {
    let id = js_str(&format_id(handle));
    kwin::run(&format!("{PRELUDE}\nvar w = findWindow({id});\n{action}"))
}

pub fn rect(handle: WindowHandle) -> Result<(i32, i32, i32, i32), CmdError> {
    let raw: RawWindow = parse(with_window(handle, "return describe(w);")?)?;
    Ok((raw.x, raw.y, raw.w, raw.h))
}

/// What `element` needs to pair a window with its accessibility tree.
#[derive(Deserialize)]
pub(super) struct ClientArea {
    pub pid: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The window's own surface. For a window that draws its own title bar and
/// shadow (GTK, Firefox) that reaches past the visible frame; for one KWin
/// decorates it's the contents without title bar and borders. Either way
/// it's what an application measures window-relative coordinates from.
pub(super) fn client_area(handle: WindowHandle) -> Result<ClientArea, CmdError> {
    parse(with_window(
        handle,
        "var b = w.bufferGeometry;
return { pid: w.pid, title: w.caption, x: Math.round(b.x), y: Math.round(b.y),
         w: Math.round(b.width), h: Math.round(b.height) };",
    )?)
}

pub fn focus(handle: WindowHandle) -> Result<(), CmdError> {
    with_window(handle, "w.minimized = false;\nworkspace.activeWindow = w;").map(|_| ())
}

pub fn move_window(handle: WindowHandle, x: i32, y: i32) -> Result<(), CmdError> {
    with_window(
        handle,
        &format!(
            "var g = w.frameGeometry;\nw.frameGeometry = {{ x: {x}, y: {y}, width: g.width, height: g.height }};"
        ),
    )
    .map(|_| ())
}

pub fn resize_window(handle: WindowHandle, w: i32, h: i32) -> Result<(), CmdError> {
    with_window(
        handle,
        &format!(
            "var g = w.frameGeometry;\nw.frameGeometry = {{ x: g.x, y: g.y, width: {w}, height: {h} }};"
        ),
    )
    .map(|_| ())
}

pub fn set_state(handle: WindowHandle, state: WindowState) -> Result<(), CmdError> {
    let action = match state {
        WindowState::Minimize => "w.minimized = true;",
        WindowState::Maximize => "w.minimized = false;\nw.setMaximize(true, true);",
        WindowState::Restore => {
            "w.minimized = false;\nw.fullScreen = false;\nw.setMaximize(false, false);"
        }
        WindowState::Fullscreen => "w.minimized = false;\nw.fullScreen = true;",
    };
    with_window(handle, action).map(|_| ())
}

pub fn close(handle: WindowHandle) -> Result<(), CmdError> {
    with_window(handle, "w.closeWindow();").map(|_| ())
}

pub fn pin(handle: WindowHandle, on: bool) -> Result<(), CmdError> {
    with_window(handle, &format!("w.keepAbove = {on};")).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ids_round_trip() {
        let id = "904e1e2e-24e9-440b-b822-93fd437caadf";
        assert_eq!(format_id(parse_id(id).unwrap()), id);
        // KWin prints them in braces; leading zeros must survive.
        let braced = "{00000000-0000-0000-0000-00000000002a}";
        assert_eq!(parse_id(braced).unwrap(), 42);
        assert_eq!(format_id(42), "00000000-0000-0000-0000-00000000002a");
    }

    #[test]
    fn bad_window_ids_are_invalid_args() {
        for id in ["", "bogus", "0x1a2b", "904e1e2e-24e9-440b-b822-93fd437caad", "zz4e1e2e-24e9-440b-b822-93fd437caadf"] {
            assert_eq!(parse_id(id).unwrap_err().code, "invalid_args", "{id}");
        }
    }

    #[test]
    fn app_filter_matches_process_name_or_class() {
        let w = window_info(RawWindow {
            id: "x".into(), title: "t".into(), class: "org.kde.dolphin".into(), pid: 0,
            x: 0, y: 0, w: 1, h: 1, state: "normal".into(), monitor: "".into(), z: 0,
            listed: true, active: false,
        });
        // No such pid, so the app name falls back to the class.
        assert_eq!(w.app, "org.kde.dolphin");
        assert!(matches_app(&w, "ORG.KDE.Dolphin"));
        assert!(!matches_app(&w, "dolphi"));
    }
}
