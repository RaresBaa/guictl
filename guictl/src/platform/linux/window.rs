use super::kwin::{self, js_str};
use crate::cli::WindowState;
use crate::output::CmdError;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

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
    /// The window this one belongs to, for a dialog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// True while one of the window's modal dialogs is open and waiting for
    /// an answer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
    /// True once KWin has found the program not answering: frozen, or busy
    /// for several seconds. KWin only finds out when it needs an answer, so a
    /// missing flag doesn't prove the program is well.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_responding: bool,
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
    #[serde(default)]
    owner: String,
    #[serde(default)]
    modal: bool,
    #[serde(default)]
    unresponsive: bool,
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
        listed: w.normalWindow || w.dialog, active: w === workspace.activeWindow,
        owner: w.transientFor ? windowId(w.transientFor) : "", modal: w.modal,
        unresponsive: !!w.unresponsive
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

/// The program's own name for one that Nix's `wrapProgram` runs as
/// `.name-wrapped`, the name a person would ask for.
pub(crate) fn unwrapped_name(name: &str) -> &str {
    name.strip_prefix('.')
        .and_then(|n| n.trim_end_matches('_').strip_suffix("-wrapped"))
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
}

fn window_info(raw: RawWindow) -> WindowInfo {
    let pid = u32::try_from(raw.pid).unwrap_or(0);
    let app = match unwrapped_name(&process_name(pid)) {
        name if name.is_empty() => raw.class.clone(),
        name => name.to_string(),
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
        owner: Some(raw.owner).filter(|o| !o.is_empty()),
        blocked: false,
        not_responding: raw.unresponsive,
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, CmdError> {
    serde_json::from_value(value)
        .map_err(|e| CmdError::os(format!("unexpected window data from KWin: {e}")))
}

fn matches_app(w: &WindowInfo, app: &str) -> bool {
    w.app.eq_ignore_ascii_case(unwrapped_name(app)) || w.class.eq_ignore_ascii_case(app)
}

/// Every listable window, topmost first, with whether it's the active one.
/// With `include_active`, the active window is included even if it isn't a
/// listable kind.
fn snapshot(include_active: bool) -> Result<Vec<(WindowInfo, bool)>, CmdError> {
    let value = kwin::run(&format!("{PRELUDE}\nreturn workspace.windowList().map(describe);"))?;
    let mut raw: Vec<RawWindow> = parse(value)?;
    let blocked: Vec<String> = raw.iter().filter(|w| w.modal && !w.owner.is_empty()).map(|w| w.owner.clone()).collect();
    raw.retain(|w| w.listed || (include_active && w.active));
    raw.sort_by_key(|w| std::cmp::Reverse(w.z));
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, w)| {
            let active = w.active;
            let mut info = window_info(w);
            info.z_order = i as u32;
            info.blocked = blocked.contains(&info.id);
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
    pid: Option<u32>,
) -> Result<WindowHandle, CmdError> {
    if active_flag {
        return parse_id(&active()?.id);
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
            && app.is_none_or(|a| matches_app(w, a))
            && pid.is_none_or(|p| w.pid == p)
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

/// A menu, dropdown or tooltip open on a window. KWin counts each as a window
/// of its own, so captures and `window list` don't include them by themselves.
#[derive(Deserialize)]
pub(super) struct Popup {
    pub id: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Deserialize)]
pub(super) struct WithPopups {
    pub x: i32,
    pub y: i32,
    pub popups: Vec<Popup>,
}

/// The window's corner and its open popups, lowest first. A submenu belongs to
/// the menu it opened from, so the walk goes up through popups to the window.
/// Under XWayland a popup names no owner, so it goes with its process's active
/// window: the one a person must be using for a menu to stay open.
pub(super) fn with_popups(handle: WindowHandle) -> Result<WithPopups, CmdError> {
    parse(with_window(
        handle,
        r#"var popups = [], order = workspace.stackingOrder;
        for (var i = 0; i < order.length; i++) {
            var p = order[i], up = p;
            while (up && up !== w && up.popupWindow) up = up.transientFor;
            var ownerless = !up && p.pid === w.pid && w === workspace.activeWindow;
            if (p !== w && (up === w || ownerless) && !p.deleted) {
                popups.push({ id: windowId(p), x: Math.round(p.frameGeometry.x), y: Math.round(p.frameGeometry.y) });
            }
        }
        return { x: Math.round(w.frameGeometry.x), y: Math.round(w.frameGeometry.y), popups: popups };"#,
    )?)
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
    pub frame_x: i32,
    pub frame_y: i32,
    pub frame_w: i32,
    pub frame_h: i32,
}

/// The window's own surface and its visible frame. For a window that draws its
/// own title bar and shadow (GTK, Firefox) the surface reaches past the frame;
/// for one KWin decorates it's the contents without title bar and borders.
pub(super) fn client_area(handle: WindowHandle) -> Result<ClientArea, CmdError> {
    parse(with_window(
        handle,
        "var b = w.bufferGeometry;
var f = w.frameGeometry;
return { pid: w.pid, title: w.caption, x: Math.round(b.x), y: Math.round(b.y),
         w: Math.round(b.width), h: Math.round(b.height),
         frame_x: Math.round(f.x), frame_y: Math.round(f.y),
         frame_w: Math.round(f.width), frame_h: Math.round(f.height) };",
    )?)
}

pub fn focus(handle: WindowHandle) -> Result<(), CmdError> {
    with_window(handle, "w.minimized = false;\nworkspace.activeWindow = w;").map(|_| ())
}

/// Focuses the window unless it's already the active one, and waits until
/// it is, so that a click sent next lands on it. Leaving an active window
/// alone keeps a menu open in it from closing.
pub(super) fn bring_forward(handle: WindowHandle) -> Result<(), CmdError> {
    let ready = |w: &RawWindow| w.active && w.state != "minimized";
    let raw: RawWindow = parse(with_window(handle, "return describe(w);")?)?;
    if ready(&raw) {
        return Ok(());
    }
    focus(handle)?;
    settle(handle, ready)
}

/// How long a window gets to take on a new geometry or state.
const SETTLE_TIMEOUT: Duration = Duration::from_millis(1000);

/// A window takes on a new size or state only once its app has drawn it,
/// so KWin goes on reporting the old one for a moment. This polls until
/// `done` holds, so that the caller can read the result straight back. It
/// gives up quietly at the timeout: the app may have refused the change or
/// adjusted it (a minimum size, size steps), and then what the window
/// reports is the answer.
fn settle(handle: WindowHandle, done: impl Fn(&RawWindow) -> bool) -> Result<(), CmdError> {
    let start = Instant::now();
    loop {
        let raw: RawWindow = parse(with_window(handle, "return describe(w);")?)?;
        if done(&raw) || start.elapsed() >= SETTLE_TIMEOUT {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn move_window(handle: WindowHandle, x: i32, y: i32) -> Result<(), CmdError> {
    with_window(
        handle,
        &format!(
            "var g = w.frameGeometry;\nw.frameGeometry = {{ x: {x}, y: {y}, width: g.width, height: g.height }};"
        ),
    )?;
    settle(handle, |w| (w.x, w.y) == (x, y))
}

pub fn resize_window(handle: WindowHandle, w: i32, h: i32) -> Result<(), CmdError> {
    with_window(
        handle,
        &format!(
            "var g = w.frameGeometry;\nw.frameGeometry = {{ x: g.x, y: g.y, width: {w}, height: {h} }};"
        ),
    )?;
    settle(handle, |raw| (raw.w, raw.h) == (w, h))
}

pub fn set_state(handle: WindowHandle, state: WindowState) -> Result<(), CmdError> {
    let (action, reached) = match state {
        WindowState::Minimize => ("w.minimized = true;", "minimized"),
        WindowState::Maximize => ("w.minimized = false;\nw.setMaximize(true, true);", "maximized"),
        WindowState::Restore => {
            ("w.minimized = false;\nw.fullScreen = false;\nw.setMaximize(false, false);", "normal")
        }
        WindowState::Fullscreen => ("w.minimized = false;\nw.fullScreen = true;", "fullscreen"),
    };
    with_window(handle, action)?;
    settle(handle, |w| w.state == reached)
}

pub fn close(handle: WindowHandle) -> Result<(), CmdError> {
    with_window(handle, "w.closeWindow();").map(|_| ())
}

/// Ends the window's process and returns its id.
pub fn kill(handle: WindowHandle) -> Result<u32, CmdError> {
    let pid = client_area(handle)?.pid;
    if pid == 0 {
        return Err(CmdError::not_found("KWin doesn't know this window's process"));
    }
    let output = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output()
        .map_err(|e| CmdError::os(format!("could not run kill: {e}")))?;
    if !output.status.success() {
        return Err(CmdError::permission_denied(format!(
            "could not end process {pid}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(pid)
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
            listed: true, active: false, owner: "".into(), modal: false, unresponsive: false,
        });
        // No such pid, so the app name falls back to the class.
        assert_eq!(w.app, "org.kde.dolphin");
        assert!(matches_app(&w, "ORG.KDE.Dolphin"));
        assert!(!matches_app(&w, "dolphi"));
    }

    #[test]
    fn wrapped_programs_go_by_their_own_name() {
        assert_eq!(unwrapped_name(".dolphin-wrapped"), "dolphin");
        assert_eq!(unwrapped_name(".jellyfin-desktop-wrapped"), "jellyfin-desktop");
        assert_eq!(unwrapped_name(".lact-wrapped_"), "lact");
        for name in ["dolphin", "not-wrapped", ".hidden", ".-wrapped", ""] {
            assert_eq!(unwrapped_name(name), name);
        }
    }
}
