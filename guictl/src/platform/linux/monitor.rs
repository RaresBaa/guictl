use crate::output::CmdError;
use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Serialize)]
pub struct MonitorInfo {
    /// The output's connector name, e.g. `DP-1`.
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    /// Size in logical pixels (after scaling and rotation), the same
    /// coordinate space windows and the mouse use.
    pub width: i32,
    pub height: i32,
    pub primary: bool,
    pub refresh_rate: u32,
    /// False for a connected output that's been `--enabled off`.
    pub attached: bool,
}

// KScreen's rotation values.
const ROTATION_LEFT: i64 = 2;
const ROTATION_RIGHT: i64 = 8;

/// KScreen is the service that owns output configuration on Plasma, and
/// `kscreen-doctor` (shipped with it) is its supported command-line front
/// end, so it's used instead of re-implementing its D-Bus config format.
pub(super) fn kscreen_doctor(args: &[String]) -> Result<String, CmdError> {
    let output = Command::new("kscreen-doctor").args(args).output().map_err(|e| {
        CmdError::unsupported(format!(
            "could not run kscreen-doctor (part of Plasma's kscreen package): {e}"
        ))
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // kscreen-doctor exits 0 even when KScreen rejects the configuration;
    // the message it prints is the only sign of it.
    if let Some(line) = stdout.lines().find(|l| l.contains("applying config failed")) {
        return Err(CmdError::os(format!("kscreen-doctor: {}", line.trim())));
    }
    if !output.status.success() {
        return Err(CmdError::os(format!(
            "kscreen-doctor failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(stdout)
}

fn outputs() -> Result<Vec<Value>, CmdError> {
    let raw = kscreen_doctor(&["-j".to_string()])?;
    let config: Value = serde_json::from_str(&raw)
        .map_err(|e| CmdError::os(format!("could not parse kscreen-doctor output: {e}")))?;
    Ok(config["outputs"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|o| o["connected"].as_bool().unwrap_or(false))
        .collect())
}

pub(super) fn output(id: &str) -> Result<Value, CmdError> {
    outputs()?
        .into_iter()
        .find(|o| o["name"].as_str() == Some(id))
        .ok_or_else(|| CmdError::not_found(format!("no monitor with id {id}")))
}

fn current_mode(output: &Value) -> Option<&Value> {
    let id = output["currentModeId"].as_str()?;
    output["modes"].as_array()?.iter().find(|m| m["id"].as_str() == Some(id))
}

fn monitor_info(output: &Value) -> MonitorInfo {
    let name = output["name"].as_str().unwrap_or_default().to_string();
    let mode = current_mode(output);
    let mut width = mode.and_then(|m| m["size"]["width"].as_f64()).unwrap_or(0.0);
    let mut height = mode.and_then(|m| m["size"]["height"].as_f64()).unwrap_or(0.0);
    if matches!(output["rotation"].as_i64(), Some(ROTATION_LEFT | ROTATION_RIGHT)) {
        std::mem::swap(&mut width, &mut height);
    }
    let scale = output["scale"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0);
    MonitorInfo {
        id: name.clone(),
        name,
        x: output["pos"]["x"].as_i64().unwrap_or(0) as i32,
        y: output["pos"]["y"].as_i64().unwrap_or(0) as i32,
        width: (width / scale).round() as i32,
        height: (height / scale).round() as i32,
        primary: output["priority"].as_i64() == Some(1),
        refresh_rate: mode.and_then(|m| m["refreshRate"].as_f64()).unwrap_or(0.0).round() as u32,
        attached: output["enabled"].as_bool().unwrap_or(false),
    }
}

pub fn list() -> Result<Vec<MonitorInfo>, CmdError> {
    if super::gnome::is_gnome() {
        return super::mutter::list();
    }
    Ok(outputs()?.iter().map(monitor_info).collect())
}

pub fn find(id: &str) -> Result<MonitorInfo, CmdError> {
    list()?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| CmdError::not_found(format!("no monitor with id {id}")))
}

fn parse_pair(spec: &str, sep: char, what: &str) -> Result<(i32, i32), CmdError> {
    let err = || CmdError::invalid_args(format!("{what} is not valid: {spec}"));
    let (a, b) = spec.split_once(sep).ok_or_else(err)?;
    Ok((a.trim().parse().map_err(|_| err())?, b.trim().parse().map_err(|_| err())?))
}

/// Picks the mode id for the requested resolution and/or refresh rate,
/// keeping whichever of the two wasn't given as close as possible to the
/// current mode.
pub(super) fn pick_mode(
    output: &Value,
    resolution: Option<&str>,
    refresh_rate: Option<u32>,
) -> Result<String, CmdError> {
    let current = current_mode(output);
    let (width, height) = match resolution {
        Some(spec) => parse_pair(&spec.to_lowercase(), 'x', "--resolution")?,
        None => {
            let size = current
                .map(|m| &m["size"])
                .ok_or_else(|| CmdError::os("the monitor has no current mode"))?;
            (
                size["width"].as_i64().unwrap_or(0) as i32,
                size["height"].as_i64().unwrap_or(0) as i32,
            )
        }
    };
    let wanted_rate = refresh_rate
        .map(f64::from)
        .or_else(|| current.and_then(|m| m["refreshRate"].as_f64()))
        .unwrap_or(60.0);
    let rate = |m: &Value| m["refreshRate"].as_f64().unwrap_or(0.0);

    let best = output["modes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            m["size"]["width"].as_i64() == Some(width as i64)
                && m["size"]["height"].as_i64() == Some(height as i64)
        })
        .filter(|m| refresh_rate.is_none_or(|r| rate(m).round() as u32 == r))
        .min_by(|a, b| (rate(a) - wanted_rate).abs().total_cmp(&(rate(b) - wanted_rate).abs()));
    best.and_then(|m| m["id"].as_str())
        .map(str::to_string)
        .ok_or_else(|| CmdError::not_found("the monitor has no mode matching that resolution and refresh rate"))
}

pub fn set(
    id: &str,
    resolution: Option<&str>,
    refresh_rate: Option<u32>,
    rotation: Option<u32>,
    position: Option<&str>,
    make_primary: bool,
    enabled: Option<bool>,
) -> Result<(), CmdError> {
    if super::gnome::is_gnome() {
        let position = position.map(|spec| parse_pair(spec, ',', "--position")).transpose()?;
        return super::mutter::set(id, resolution, refresh_rate, rotation, position, make_primary, enabled);
    }
    let output = &output(id)?;

    let mut args: Vec<String> = Vec::new();
    // Enable first so the other settings apply to a live output.
    if enabled == Some(true) {
        args.push(format!("output.{id}.enable"));
    }
    if resolution.is_some() || refresh_rate.is_some() {
        args.push(format!("output.{id}.mode.{}", pick_mode(output, resolution, refresh_rate)?));
    }
    if let Some(rotation) = rotation {
        // Matches Windows, where 90 ("Portrait") puts the top of the picture
        // on the panel's left edge. KScreen calls that "left"; its "right"
        // puts the top on the right edge.
        let name = match rotation {
            0 => "none",
            90 => "left",
            180 => "inverted",
            270 => "right",
            _ => return Err(CmdError::invalid_args("--rotation must be 0, 90, 180 or 270")),
        };
        args.push(format!("output.{id}.rotation.{name}"));
    }
    if let Some(spec) = position {
        let (x, y) = parse_pair(spec, ',', "--position")?;
        if x < 0 || y < 0 {
            return Err(CmdError::invalid_args(
                "--position can't be negative: KScreen keeps the desktop's top-left corner at 0,0, so place this monitor at 0,0 and move the others instead",
            ));
        }
        args.push(format!("output.{id}.position.{x},{y}"));
    }
    if make_primary {
        args.push(format!("output.{id}.priority.1"));
    }
    if enabled == Some(false) {
        args.push(format!("output.{id}.disable"));
    }
    if args.is_empty() {
        return Err(CmdError::invalid_args("nothing to change: pass at least one setting"));
    }
    kscreen_doctor(&args).map(|_| ())
}

/// DPMS through KScreen, which switches every monitor at once. Its
/// `--dpms-excluded` option is meant to spare some of them, but on Plasma 6.7
/// an excluded output went dark with the rest, so with more than one monitor
/// attached this refuses rather than blank them all.
pub fn power(id: &str, on: bool) -> Result<(), CmdError> {
    if super::gnome::is_gnome() {
        return super::mutter::power(id, on);
    }
    let monitors = list()?;
    if !monitors.iter().any(|m| m.id == id) {
        return Err(CmdError::not_found(format!("no monitor with id {id}")));
    }
    if monitors.iter().any(|m| m.id != id && m.attached) {
        return Err(CmdError::unsupported(
            "Plasma can only switch all monitors off or on together, so monitor power works only while a single monitor is attached",
        ));
    }
    let state = if on { "on" } else { "off" };
    kscreen_doctor(&["--dpms".to_string(), state.to_string()]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn output() -> Value {
        json!({
            "name": "DP-1", "enabled": true, "priority": 1, "rotation": 1, "scale": 1.0,
            "pos": { "x": 1920, "y": 0 }, "currentModeId": "2",
            "modes": [
                { "id": "1", "size": { "width": 1920, "height": 1080 }, "refreshRate": 60.0 },
                { "id": "2", "size": { "width": 2560, "height": 1440 }, "refreshRate": 59.95 },
                { "id": "3", "size": { "width": 2560, "height": 1440 }, "refreshRate": 143.97 },
                { "id": "4", "size": { "width": 1920, "height": 1080 }, "refreshRate": 143.99 },
            ]
        })
    }

    #[test]
    fn describes_an_output() {
        let m = monitor_info(&output());
        assert_eq!((m.id.as_str(), m.x, m.y, m.width, m.height), ("DP-1", 1920, 0, 2560, 1440));
        assert!(m.primary && m.attached);
        assert_eq!(m.refresh_rate, 60);
    }

    #[test]
    fn size_is_logical_pixels() {
        let mut o = output();
        o["scale"] = json!(2.0);
        o["rotation"] = json!(ROTATION_LEFT);
        o["priority"] = json!(2);
        let m = monitor_info(&o);
        // Turned on its side, then halved by the scale.
        assert_eq!((m.width, m.height), (720, 1280));
        assert!(!m.primary);
    }

    #[test]
    fn parses_pairs() {
        assert_eq!(parse_pair("1920x1080", 'x', "--resolution").unwrap(), (1920, 1080));
        assert_eq!(parse_pair("-10, 20", ',', "--position").unwrap(), (-10, 20));
        for spec in ["1920", "1920x", "axb", "1x2x3"] {
            assert_eq!(parse_pair(spec, 'x', "--resolution").unwrap_err().code, "invalid_args", "{spec}");
        }
    }

    #[test]
    fn picks_modes() {
        let o = output();
        // A new resolution keeps the current refresh rate as nearly as it can.
        assert_eq!(pick_mode(&o, Some("1920x1080"), None).unwrap(), "1");
        assert_eq!(pick_mode(&o, Some("1920X1080"), Some(144)).unwrap(), "4");
        // A new refresh rate keeps the current resolution.
        assert_eq!(pick_mode(&o, None, Some(144)).unwrap(), "3");
        assert_eq!(pick_mode(&o, Some("800x600"), None).unwrap_err().code, "not_found");
        assert_eq!(pick_mode(&o, None, Some(75)).unwrap_err().code, "not_found");
        assert_eq!(pick_mode(&o, Some("big"), None).unwrap_err().code, "invalid_args");
    }
}
