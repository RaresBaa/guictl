use crate::output::CmdError;

/// Plasma (6.2+) exposes brightness per output through KScreen, covering
/// both laptop backlights and DDC/CI external monitors.
pub fn get(monitor_id: &str) -> Result<u32, CmdError> {
    let output = super::monitor::output(monitor_id)?;
    output["brightness"]
        .as_f64()
        .map(|b| (b * 100.0).round() as u32)
        .ok_or_else(|| CmdError::unsupported("this monitor does not report a brightness"))
}

pub fn set(monitor_id: &str, level: u32) -> Result<(), CmdError> {
    super::monitor::output(monitor_id)?;
    super::monitor::kscreen_doctor(&[format!("output.{monitor_id}.brightness.{}", level.min(100))])
        .map(|_| ())
}
