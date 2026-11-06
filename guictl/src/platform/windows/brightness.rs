use crate::output::CmdError;
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitor, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, PHYSICAL_MONITOR,
};
use windows::Win32::Graphics::Gdi::HMONITOR;

pub(super) fn parse_monitor_id(id: &str) -> Result<HMONITOR, CmdError> {
    let raw = usize::from_str_radix(id.trim_start_matches("0x"), 16)
        .map_err(|_| CmdError::invalid_args(format!("invalid monitor id: {id}")))?;
    Ok(HMONITOR(raw as *mut _))
}

/// DDC/CI exposes one monitor per physical display, but a single `HMONITOR`
/// can in principle map to more than one (e.g. a KVM). We operate on the
/// first, which matches every normal single-display setup.
pub(super) struct PhysicalMonitor(PHYSICAL_MONITOR);

impl PhysicalMonitor {
    pub(super) fn open(hmonitor: HMONITOR) -> Result<Self, CmdError> {
        unsafe {
            let mut count = 0u32;
            GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count)
                .map_err(|e| CmdError::os(format!("could not query physical monitors: {e}")))?;
            if count == 0 {
                return Err(CmdError::not_found(
                    "this monitor has no DDC/CI-addressable physical monitor",
                ));
            }
            let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
            GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut monitors)
                .map_err(|e| CmdError::os(format!("could not open physical monitor: {e}")))?;
            let chosen = monitors[0];
            for extra in &monitors[1..] {
                let _ = DestroyPhysicalMonitor(extra.hPhysicalMonitor);
            }
            Ok(Self(chosen))
        }
    }

    pub(super) fn handle(&self) -> windows::Win32::Foundation::HANDLE {
        self.0.hPhysicalMonitor
    }
}

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitor(self.0.hPhysicalMonitor);
        }
    }
}

pub fn get(monitor_id: &str) -> Result<u32, CmdError> {
    let hmonitor = parse_monitor_id(monitor_id)?;
    let physical = PhysicalMonitor::open(hmonitor)?;
    let (mut min, mut current, mut max) = (0u32, 0u32, 0u32);
    let ok = unsafe { GetMonitorBrightness(physical.handle(), &mut min, &mut current, &mut max) };
    if ok == 0 {
        return Err(CmdError::os(
            "GetMonitorBrightness failed; the monitor may not support DDC/CI brightness control",
        ));
    }
    if max <= min {
        return Ok(0);
    }
    Ok(((current - min) as f64 / (max - min) as f64 * 100.0).round() as u32)
}

pub fn set(monitor_id: &str, level: u32) -> Result<(), CmdError> {
    let hmonitor = parse_monitor_id(monitor_id)?;
    let physical = PhysicalMonitor::open(hmonitor)?;
    let (mut min, mut current, mut max) = (0u32, 0u32, 0u32);
    let ok = unsafe { GetMonitorBrightness(physical.handle(), &mut min, &mut current, &mut max) };
    if ok == 0 {
        return Err(CmdError::os(
            "GetMonitorBrightness failed; the monitor may not support DDC/CI brightness control",
        ));
    }
    let level = level.min(100);
    let target = min + ((max - min) as f64 * (level as f64 / 100.0)).round() as u32;
    let ok = unsafe { SetMonitorBrightness(physical.handle(), target) };
    if ok == 0 {
        return Err(CmdError::os("SetMonitorBrightness failed"));
    }
    Ok(())
}
