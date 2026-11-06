use crate::output::CmdError;
use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, SetDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_PATH_SOURCE_INFO, DISPLAYCONFIG_PATH_TARGET_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DISPLAYCONFIG_TARGET_PREFERRED_MODE, QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS,
    QUERY_DISPLAY_CONFIG_FLAGS, SDC_ALLOW_CHANGES, SDC_APPLY, SDC_SAVE_TO_DATABASE, SDC_USE_SUPPLIED_DISPLAY_CONFIG,
};
use windows::Win32::Foundation::{BOOL, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HWND, LPARAM, POINTL, RECT};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayMonitors, EnumDisplaySettingsW,
    GetMonitorInfoW, CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW,
    DEVMODE_FIELD_FLAGS, DISP_CHANGE_SUCCESSFUL,
    DMDO_180, DMDO_270, DMDO_90, DMDO_DEFAULT, DM_DISPLAYFREQUENCY, DM_DISPLAYORIENTATION,
    DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS, HDC,
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
    /// False for a monitor that's plugged in but `--enabled off`: it has no
    /// HMONITOR, so `id` names its connector (`target-...`), `name` is the
    /// monitor's own name rather than `\\.\DISPLAYn`, and the size and
    /// refresh rate are its preferred mode, at position 0,0.
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

        let mut devmode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
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

/// `DISPLAYCONFIG_PATH_ACTIVE` and `DISPLAYCONFIG_PATH_MODE_IDX_INVALID`, not
/// exposed as named constants by this version of `windows`.
const PATH_ACTIVE: u32 = 0x1;
const MODE_IDX_INVALID: u32 = 0xffff_ffff;

/// The display topology as Windows' CCD (Connecting and Configuring Displays)
/// API reports it: one path per source-to-monitor link, plus the modes the
/// active ones use.
struct DisplayConfig {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

fn query_display_config(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<DisplayConfig, CmdError> {
    loop {
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        let err = unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) };
        if err != ERROR_SUCCESS {
            return Err(CmdError::os(format!("could not size the display configuration (code {})", err.0)));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let err = unsafe {
            QueryDisplayConfig(flags, &mut path_count, paths.as_mut_ptr(), &mut mode_count, modes.as_mut_ptr(), None)
        };
        // The topology changed between the two calls; size it again.
        if err == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if err != ERROR_SUCCESS {
            return Err(CmdError::os(format!("could not read the display configuration (code {})", err.0)));
        }
        paths.truncate(path_count as usize);
        modes.truncate(mode_count as usize);
        return Ok(DisplayConfig { paths, modes });
    }
}

/// Applies exactly these paths as the new topology and saves it, so it
/// survives a reboot. Only the modes the paths still point at are passed,
/// renumbered to match.
fn apply_display_config(paths: &[DISPLAYCONFIG_PATH_INFO], modes: &[DISPLAYCONFIG_MODE_INFO]) -> Result<(), CmdError> {
    let mut paths = paths.to_vec();
    let mut kept_modes: Vec<DISPLAYCONFIG_MODE_INFO> = Vec::new();
    let mut renumbered: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for path in &mut paths {
        let indices = unsafe {
            [&mut path.sourceInfo.Anonymous.modeInfoIdx, &mut path.targetInfo.Anonymous.modeInfoIdx]
        };
        for idx in indices {
            if *idx == MODE_IDX_INVALID {
                continue;
            }
            *idx = *renumbered.entry(*idx).or_insert_with(|| {
                kept_modes.push(modes[*idx as usize]);
                (kept_modes.len() - 1) as u32
            });
        }
    }
    let modes = (!kept_modes.is_empty()).then_some(kept_modes.as_slice());
    let result = unsafe {
        SetDisplayConfig(
            Some(&paths),
            modes,
            SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES | SDC_SAVE_TO_DATABASE,
        )
    };
    if result != 0 {
        return Err(CmdError::os(format!("Windows refused the new display layout (code {result})")));
    }
    Ok(())
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// The GDI name, e.g. `\\.\DISPLAY1`, that `MONITORINFOEXW::szDevice` reports
/// for the monitor this source drives.
fn source_gdi_name(source: &DISPLAYCONFIG_PATH_SOURCE_INFO) -> Option<String> {
    let mut request = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    request.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    request.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    request.header.adapterId = source.adapterId;
    request.header.id = source.id;
    (unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } == 0)
        .then(|| from_wide(&request.viewGdiDeviceName))
}

/// The monitor's own name from its EDID, such as "DELL P2419H".
fn target_friendly_name(target: &DISPLAYCONFIG_PATH_TARGET_INFO) -> Option<String> {
    let mut request = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    request.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    request.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
    request.header.adapterId = target.adapterId;
    request.header.id = target.id;
    (unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } == 0)
        .then(|| from_wide(&request.monitorFriendlyDeviceName))
        .filter(|name| !name.is_empty())
}

/// The monitor's preferred width, height and refresh rate.
fn target_preferred_mode(target: &DISPLAYCONFIG_PATH_TARGET_INFO) -> Option<(u32, u32, u32)> {
    let mut request = DISPLAYCONFIG_TARGET_PREFERRED_MODE::default();
    request.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE;
    request.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>() as u32;
    request.header.adapterId = target.adapterId;
    request.header.id = target.id;
    if unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } != 0 {
        return None;
    }
    let vsync = request.targetMode.targetVideoSignalInfo.vSyncFreq;
    let refresh = if vsync.Denominator == 0 {
        0
    } else {
        (vsync.Numerator as f64 / vsync.Denominator as f64).round() as u32
    };
    Some((request.width, request.height, refresh))
}

/// The id `monitor list` gives a disabled monitor. It has no HMONITOR, so the
/// id names its CCD target: the graphics adapter and the connector on it.
/// Adapter ids are only fixed until a reboot, like HMONITOR ids.
fn target_monitor_id(target: &DISPLAYCONFIG_PATH_TARGET_INFO) -> String {
    let adapter = match target.adapterId.HighPart {
        0 => format!("{:x}", target.adapterId.LowPart),
        high => format!("{:x}{:08x}", high as u32, target.adapterId.LowPart),
    };
    format!("target-{adapter}-{}", target.id)
}

fn same_target(a: &DISPLAYCONFIG_PATH_TARGET_INFO, b: &DISPLAYCONFIG_PATH_TARGET_INFO) -> bool {
    a.adapterId == b.adapterId && a.id == b.id
}

fn same_source(a: &DISPLAYCONFIG_PATH_SOURCE_INFO, b: &DISPLAYCONFIG_PATH_SOURCE_INFO) -> bool {
    a.adapterId == b.adapterId && a.id == b.id
}

fn is_active(path: &DISPLAYCONFIG_PATH_INFO) -> bool {
    path.flags & PATH_ACTIVE != 0
}

/// Monitors that are plugged in but not part of the desktop, which is what
/// `monitor set --enabled off` leaves behind. `EnumDisplayMonitors` only sees
/// the active ones, so without this a disabled monitor would have no id to
/// enable it with. Its size and refresh rate are its preferred mode, since it
/// has no current one.
fn list_detached() -> Vec<MonitorInfo> {
    let Ok(config) = query_display_config(QDC_ALL_PATHS) else {
        return Vec::new();
    };
    let active: Vec<DISPLAYCONFIG_PATH_TARGET_INFO> =
        config.paths.iter().filter(|p| is_active(p)).map(|p| p.targetInfo).collect();
    let mut seen: Vec<DISPLAYCONFIG_PATH_TARGET_INFO> = Vec::new();
    let mut out = Vec::new();
    for path in &config.paths {
        let target = &path.targetInfo;
        if !target.targetAvailable.as_bool()
            || active.iter().any(|a| same_target(a, target))
            || seen.iter().any(|s| same_target(s, target))
        {
            continue;
        }
        seen.push(*target);
        let (width, height, refresh_rate) = target_preferred_mode(target).unwrap_or((0, 0, 0));
        out.push(MonitorInfo {
            id: target_monitor_id(target),
            name: target_friendly_name(target).unwrap_or_else(|| "Unknown monitor".to_string()),
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
            primary: false,
            refresh_rate,
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
    monitors.extend(list_detached());
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
    let mut devmode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
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

/// Takes a monitor off the desktop, as Display settings' "Disconnect this
/// display" does. The older `ChangeDisplaySettingsExW` way of detaching (a
/// zero-size `DEVMODE`) reported success and changed nothing on a
/// three-monitor Windows 11 machine, so this uses the CCD API instead.
fn disable(id: &str) -> Result<(), CmdError> {
    let monitor = find(id)?;
    if !monitor.attached {
        return Ok(());
    }
    if monitor.primary {
        return Err(CmdError::invalid_args(
            "can't disable the primary monitor; make another one primary first with `monitor set --make-primary`",
        ));
    }
    let config = query_display_config(QDC_ONLY_ACTIVE_PATHS)?;
    // A monitor that mirrors another shares its source, so they go together.
    let keep: Vec<DISPLAYCONFIG_PATH_INFO> = config
        .paths
        .iter()
        .filter(|p| source_gdi_name(&p.sourceInfo).as_deref() != Some(monitor.name.as_str()))
        .copied()
        .collect();
    if keep.len() == config.paths.len() {
        return Err(CmdError::os(format!("could not find the display path for {}", monitor.name)));
    }
    if keep.is_empty() {
        return Err(CmdError::invalid_args("can't disable the only monitor"));
    }
    apply_display_config(&keep, &config.modes)
}

/// Puts a disabled monitor back on the desktop and returns its new id.
/// Windows picks its mode and position. In testing it placed the monitor to
/// the right of the others rather than where it was before it was disabled,
/// so pass `--position` along with `--enabled on` to put it somewhere else.
fn enable(id: &str) -> Result<String, CmdError> {
    let monitors = list()?;
    if monitors.iter().any(|m| m.id == id && m.attached) {
        return Ok(id.to_string());
    }
    let config = query_display_config(QDC_ALL_PATHS)?;
    let active: Vec<DISPLAYCONFIG_PATH_INFO> = config.paths.iter().filter(|p| is_active(p)).copied().collect();
    let candidate = config
        .paths
        .iter()
        .filter(|p| !is_active(p) && p.targetInfo.targetAvailable.as_bool())
        .filter(|p| target_monitor_id(&p.targetInfo) == id)
        // Each source drives one picture; one already in use would make this
        // monitor a mirror of another instead of extending the desktop.
        .find(|p| !active.iter().any(|a| same_source(&a.sourceInfo, &p.sourceInfo)));
    let Some(candidate) = candidate else {
        if monitors.iter().any(|m| m.id == id) {
            return Err(CmdError::os("the graphics adapter has no free output left to drive this monitor"));
        }
        return Err(CmdError::not_found(format!(
            "no disabled monitor with id {id}; `monitor list` shows a disabled monitor's id, which starts with \"target-\""
        )));
    };

    let mut path = *candidate;
    path.flags |= PATH_ACTIVE;
    path.sourceInfo.Anonymous.modeInfoIdx = MODE_IDX_INVALID;
    path.targetInfo.Anonymous.modeInfoIdx = MODE_IDX_INVALID;
    let mut paths = active;
    paths.push(path);
    apply_display_config(&paths, &config.modes)?;

    let name = source_gdi_name(&path.sourceInfo);
    list()?
        .into_iter()
        .find(|m| m.attached && Some(&m.name) == name.as_ref())
        .map(|m| m.id)
        .ok_or_else(|| CmdError::os("Windows accepted the new layout, but the monitor didn't come on"))
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
        let other_settings = resolution.is_some()
            || refresh_rate.is_some()
            || rotation.is_some()
            || position.is_some()
            || make_primary;
        if !enabled {
            if other_settings {
                return Err(CmdError::invalid_args("--enabled off can't be combined with other settings"));
            }
            return disable(id);
        }
        let new_id = enable(id)?;
        if !other_settings {
            return Ok(());
        }
        return set(&new_id, resolution, refresh_rate, rotation, position, make_primary, None);
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

/// MCCS VCP code 0xD6, "Power Mode".
const VCP_POWER_MODE: u8 = 0xD6;
/// MCCS power mode values. 0x04 is DPMS off, which keeps the panel's DDC/CI
/// listening so 0x01 can wake it again. 0x05 (hard power off) is avoided on
/// purpose: many panels stop answering DDC/CI in it and only wake from their
/// own power button.
const POWER_ON: u32 = 0x01;
const POWER_OFF: u32 = 0x04;

/// Switches one monitor's panel on or off over DDC/CI. The monitor stays
/// attached to the desktop, so windows don't move and its id doesn't change.
pub fn power(id: &str, on: bool) -> Result<(), CmdError> {
    use super::brightness::{parse_monitor_id, PhysicalMonitor};
    use windows::Win32::Devices::Display::{GetVCPFeatureAndVCPFeatureReply, SetVCPFeature};

    let monitor = find(id)?;
    if !monitor.attached {
        return Err(CmdError::invalid_args(format!(
            "monitor {id} is disabled; enable it with `monitor set {id} --enabled on` instead"
        )));
    }
    let physical = PhysicalMonitor::open(parse_monitor_id(id)?)?;

    // Read the power mode first, so a monitor without DDC/CI, or with it
    // switched off in its own menu, gets a clear error instead of a write
    // that silently does nothing.
    let mut current = 0u32;
    let ok = unsafe {
        GetVCPFeatureAndVCPFeatureReply(physical.handle(), VCP_POWER_MODE, None, &mut current, None)
    };
    if ok == 0 {
        return Err(CmdError::unsupported(
            "the monitor didn't answer a DDC/CI power mode query; it may not support DDC/CI, or DDC/CI may be switched off in the monitor's own menu",
        ));
    }

    let target = if on { POWER_ON } else { POWER_OFF };
    if current == target {
        return Ok(());
    }
    let ok = unsafe { SetVCPFeature(physical.handle(), VCP_POWER_MODE, target) };
    if ok == 0 {
        return Err(CmdError::os("the monitor refused the DDC/CI power mode change"));
    }
    Ok(())
}
