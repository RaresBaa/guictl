use crate::output::CmdError;
use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINTL, RECT};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplayMonitors, EnumDisplaySettingsW,
    GetMonitorInfoW, CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW,
    DEVMODE_FIELD_FLAGS, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISP_CHANGE_SUCCESSFUL,
    DMDO_180, DMDO_270, DMDO_90, DMDO_DEFAULT, DM_DISPLAYFREQUENCY, DM_DISPLAYORIENTATION,
    DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS, ENUM_REGISTRY_SETTINGS, HDC,
    HMONITOR, MONITORINFOEXW,
};

/// MONITORINFOF_PRIMARY, not exposed as a named constant by this version of `windows`.
const MONITORINFOF_PRIMARY: u32 = 1;

#[derive(Serialize)]
pub struct MonitorInfo {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
    pub refresh_rate: u32,
    /// False for a monitor that's been `--enabled off`: it has no HMONITOR
    /// (so `id` is its device name, e.g. `\\.\DISPLAY5`, not a hex handle),
    /// and its geometry comes from its last saved settings, not live state.
    pub attached: bool,
}

extern "system" fn enum_proc(hmonitor: HMONITOR, _hdc: HDC, _rect: *mut RECT, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<MonitorInfo>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(hmonitor, &mut info as *mut MONITORINFOEXW as *mut _) };
    if ok.as_bool() {
        let rect = info.monitorInfo.rcMonitor;
        let primary = (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;
        let name_end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..name_end]);

        let mut devmode = DEVMODEW::default();
        devmode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        let mut refresh = 0u32;
        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        if unsafe {
            EnumDisplaySettingsW(PCWSTR(name_wide.as_ptr()), ENUM_CURRENT_SETTINGS, &mut devmode)
        }
        .as_bool()
        {
            refresh = devmode.dmDisplayFrequency;
        }

        list.push(MonitorInfo {
            id: format!("{:#x}", hmonitor.0 as usize),
            name,
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
            primary,
            refresh_rate: refresh,
            attached: true,
        });
    }
    BOOL(1)
}

/// Surfaces display adapters that exist but aren't attached to the desktop
/// (disabled via `--enabled off`, or just disconnected) - `EnumDisplayMonitors`
/// above only sees attached ones, so without this a disabled monitor would
/// have no id to re-enable it with. Geometry comes from each adapter's last
/// saved settings (`ENUM_REGISTRY_SETTINGS`), which Windows keeps even while
/// detached.
fn list_detached(skip_names: &std::collections::HashSet<String>) -> Vec<MonitorInfo> {
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
        let mut dd = DISPLAY_DEVICEW::default();
        dd.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        let ok = unsafe { EnumDisplayDevicesW(PCWSTR(std::ptr::null()), i, &mut dd, 0) };
        if !ok.as_bool() {
            break;
        }
        i += 1;

        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0 {
            continue;
        }
        let name_end = dd.DeviceName.iter().position(|&c| c == 0).unwrap_or(dd.DeviceName.len());
        let name = String::from_utf16_lossy(&dd.DeviceName[..name_end]);
        if skip_names.contains(&name) {
            continue;
        }

        let name_wide = wide(&name);
        let mut devmode = DEVMODEW::default();
        devmode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        let read_ok = unsafe {
            EnumDisplaySettingsW(PCWSTR(name_wide.as_ptr()), ENUM_REGISTRY_SETTINGS, &mut devmode)
        }
        .as_bool();
        // Windows reserves many more "\\.\DISPLAYn" adapter slots than are
        // ever used; an unconfigured phantom slot has no saved geometry at
        // all, which is how we tell it apart from a monitor someone actually
        // disabled (which still has its last-known width/height/position).
        if !read_ok || devmode.dmPelsWidth == 0 || devmode.dmPelsHeight == 0 {
            continue;
        }
        let w = devmode.dmPelsWidth as i32;
        let h = devmode.dmPelsHeight as i32;
        let refresh = devmode.dmDisplayFrequency;
        let pos = unsafe { devmode.Anonymous1.Anonymous2.dmPosition };
        let (x, y) = (pos.x, pos.y);

        out.push(MonitorInfo {
            id: name.clone(),
            name,
            x,
            y,
            width: w,
            height: h,
            primary: false,
            refresh_rate: refresh,
            attached: false,
        });
    }
    out
}

pub fn list() -> Result<Vec<MonitorInfo>, CmdError> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let lparam = LPARAM(&mut monitors as *mut Vec<MonitorInfo> as isize);
    let ok = unsafe { EnumDisplayMonitors(HDC::default(), None, Some(enum_proc), lparam) };
    if !ok.as_bool() {
        return Err(CmdError::os("EnumDisplayMonitors failed"));
    }
    let attached_names: std::collections::HashSet<String> =
        monitors.iter().map(|m| m.name.clone()).collect();
    monitors.extend(list_detached(&attached_names));
    Ok(monitors)
}

pub fn find(id: &str) -> Result<MonitorInfo, CmdError> {
    list()?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| CmdError::not_found(format!("no monitor with id {id}")))
}

fn parse_resolution(spec: &str) -> Result<(u32, u32), CmdError> {
    let (w, h) = spec
        .split_once('x')
        .or_else(|| spec.split_once('X'))
        .ok_or_else(|| CmdError::invalid_args("--resolution must be formatted as WxH"))?;
    let w: u32 = w
        .trim()
        .parse()
        .map_err(|_| CmdError::invalid_args("--resolution width must be a positive integer"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| CmdError::invalid_args("--resolution height must be a positive integer"))?;
    Ok((w, h))
}

fn parse_position(spec: &str) -> Result<(i32, i32), CmdError> {
    let (x, y) = spec
        .split_once(',')
        .ok_or_else(|| CmdError::invalid_args("--position must be formatted as X,Y"))?;
    let x: i32 = x
        .trim()
        .parse()
        .map_err(|_| CmdError::invalid_args("--position X must be an integer"))?;
    let y: i32 = y
        .trim()
        .parse()
        .map_err(|_| CmdError::invalid_args("--position Y must be an integer"))?;
    Ok((x, y))
}

fn wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_devmode(device_name: &[u16]) -> Result<DEVMODEW, CmdError> {
    let mut devmode = DEVMODEW::default();
    devmode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    let ok = unsafe {
        EnumDisplaySettingsW(PCWSTR(device_name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut devmode)
    };
    if !ok.as_bool() {
        return Err(CmdError::os("could not read the monitor's current display settings"));
    }
    Ok(devmode)
}

fn apply_devmode(device_name: &[u16], devmode: &DEVMODEW, flags: CDS_TYPE) -> Result<(), CmdError> {
    let result = unsafe {
        ChangeDisplaySettingsExW(PCWSTR(device_name.as_ptr()), Some(devmode), HWND::default(), flags, None)
    };
    if result != DISP_CHANGE_SUCCESSFUL {
        return Err(CmdError::os(format!("ChangeDisplaySettingsExW failed (code {})", result.0)));
    }
    Ok(())
}

/// Commits every pending `CDS_NORESET` change queued by earlier
/// `apply_devmode` calls (NULL device name + NULL devmode means "apply what
/// was queued"). Any change that alters the desktop's overall shape - which
/// monitor is primary, attaching/detaching a monitor - silently does
/// nothing as a single `CDS_UPDATEREGISTRY`-only call; Windows only commits
/// it once every affected monitor's queued change is flushed together. That
/// surprised me twice live-testing this (make-primary, then enable/disable)
/// before I started reaching for this pattern by default for anything
/// touching more than a single monitor's own resolution/refresh/rotation.
fn commit_pending() -> Result<(), CmdError> {
    let result = unsafe {
        ChangeDisplaySettingsExW(PCWSTR(std::ptr::null()), None, HWND::default(), CDS_TYPE(0), None)
    };
    if result != DISP_CHANGE_SUCCESSFUL {
        return Err(CmdError::os(format!("committing the pending display change failed (code {})", result.0)));
    }
    Ok(())
}

/// `--enabled on/off` is NOT implemented, despite MSDN documenting
/// `ChangeDisplaySettingsExW` with a zero-width/height `DEVMODE` as the way
/// to detach a display (and the converse to reattach one). Live-tested
/// against 3 real connected monitors on this machine (Windows 11): every
/// variant - a plain call, the `CDS_NORESET`-batch-then-commit pattern that
/// fixed `--make-primary`, and that same pattern extended to reassert every
/// other attached monitor's position too - reported `DISP_CHANGE_SUCCESSFUL`
/// while changing nothing. The legacy API appears to be silently ignored
/// for physically-connected displays under this machine's CCD (Connecting
/// and Configuring Displays) driver stack; actually detaching a display here
/// most likely needs `SetDisplayConfig`/`QueryDisplayConfig` with an
/// explicit topology instead, a meaningfully larger piece of work this
/// wasn't extended to cover.
fn set_enabled(_id: &str, _enabled: bool, _resolution: Option<&str>, _refresh_rate: Option<u32>) -> Result<(), CmdError> {
    Err(CmdError::unsupported(
        "monitor enable/disable isn't implemented: the documented ChangeDisplaySettingsExW \
         detach mechanism was live-tested on this machine's 3-monitor setup and silently did \
         nothing (reported success, changed nothing) - likely needs the newer \
         SetDisplayConfig/QueryDisplayConfig topology API instead",
    ))
}

/// Applies resolution/refresh-rate/rotation/position/primary changes via the
/// documented `ChangeDisplaySettingsExW` API.
///
/// `--make-primary` is the one case that can't be a single call: Windows
/// silently ignores `CDS_SET_PRIMARY` unless every other monitor is
/// repositioned in the same transaction (the old primary can't stay at
/// (0,0) once a different monitor claims it). So when `--make-primary` is
/// set, every monitor gets shifted by the target's old position (making the
/// target's old position the new origin) via `CDS_NORESET`, then one final
/// no-op `ChangeDisplaySettingsExW` call commits them all together.
/// `--position` on its own (no `--make-primary`) stays a single call and,
/// like before, doesn't check for overlaps against the rest of the
/// arrangement - use it to manually fix a layout `--make-primary` leaves
/// non-primary monitors in (their position relative to the new origin is
/// still correct, just not confirmed against real hardware beyond this
/// session's test).
pub fn set(
    id: &str,
    resolution: Option<&str>,
    refresh_rate: Option<u32>,
    rotation: Option<u32>,
    position: Option<&str>,
    make_primary: bool,
    enabled: Option<bool>,
) -> Result<(), CmdError> {
    if let Some(enabled) = enabled {
        return set_enabled(id, enabled, resolution, refresh_rate);
    }

    let target = find(id)?;
    let device_name = wide(&target.name);

    let mut devmode = read_devmode(&device_name)?;
    let original_w = devmode.dmPelsWidth;
    let original_h = devmode.dmPelsHeight;
    let original_orientation = unsafe { devmode.Anonymous1.Anonymous2.dmDisplayOrientation };
    devmode.dmFields = DEVMODE_FIELD_FLAGS(0);

    if let Some(res) = resolution {
        let (w, h) = parse_resolution(res)?;
        devmode.dmPelsWidth = w;
        devmode.dmPelsHeight = h;
        devmode.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
    }
    if let Some(hz) = refresh_rate {
        devmode.dmDisplayFrequency = hz;
        devmode.dmFields |= DM_DISPLAYFREQUENCY;
    }
    if let Some(deg) = rotation {
        let orientation = match deg {
            0 => DMDO_DEFAULT,
            90 => DMDO_90,
            180 => DMDO_180,
            270 => DMDO_270,
            _ => return Err(CmdError::invalid_args("--rotation must be 0, 90, 180, or 270")),
        };
        // Crossing the landscape/portrait boundary requires swapping the
        // resolution's width and height too, or the mode is invalid and
        // ChangeDisplaySettingsExW rejects it (DISP_CHANGE_BADMODE) - so
        // auto-swap from the monitor's current size unless the caller
        // already gave an explicit --resolution (trust that instead).
        let is_portrait = |o: windows::Win32::Graphics::Gdi::DEVMODE_DISPLAY_ORIENTATION| {
            o == DMDO_90 || o == DMDO_270
        };
        if resolution.is_none() && is_portrait(orientation) != is_portrait(original_orientation) {
            devmode.dmPelsWidth = original_h;
            devmode.dmPelsHeight = original_w;
            devmode.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
        }
        devmode.Anonymous1.Anonymous2.dmDisplayOrientation = orientation;
        devmode.dmFields |= DM_DISPLAYORIENTATION;
    }

    if make_primary {
        // Explicit --position for the target is superseded by the forced
        // new-origin requirement below; every other monitor also needs its
        // own ChangeDisplaySettingsExW call, so the target's isn't applied
        // here - it's folded into the loop below instead.
        let (old_px, old_py) = (target.x, target.y);
        devmode.Anonymous1.Anonymous2.dmPosition = POINTL { x: 0, y: 0 };
        devmode.dmFields |= DM_POSITION;
        apply_devmode(&device_name, &devmode, CDS_UPDATEREGISTRY | CDS_NORESET | CDS_SET_PRIMARY)?;

        for other in list()?.into_iter().filter(|m| m.id != target.id) {
            let other_device_name = wide(&other.name);
            let mut other_devmode = read_devmode(&other_device_name)?;
            other_devmode.dmFields = DM_POSITION;
            other_devmode.Anonymous1.Anonymous2.dmPosition =
                POINTL { x: other.x - old_px, y: other.y - old_py };
            apply_devmode(&other_device_name, &other_devmode, CDS_UPDATEREGISTRY | CDS_NORESET)?;
        }

        return commit_pending();
    }

    if let Some(spec) = position {
        let (x, y) = parse_position(spec)?;
        devmode.Anonymous1.Anonymous2.dmPosition = POINTL { x, y };
        devmode.dmFields |= DM_POSITION;
    }

    if devmode.dmFields.0 == 0 {
        return Err(CmdError::invalid_args(
            "specify at least one of --resolution, --refresh-rate, --rotation, --position, --make-primary",
        ));
    }

    apply_devmode(&device_name, &devmode, CDS_UPDATEREGISTRY)
}

pub fn power(_id: &str, _on: bool) -> Result<(), CmdError> {
    Err(CmdError::unsupported(
        "per-monitor power control needs a DDC/CI VCP write (code 0xD6) whose on/standby/off value mapping varies enough by panel that it isn't safe to ship unverified; not implemented",
    ))
}
