//! Monitors and backlight on GNOME, through Mutter's `DisplayConfig` D-Bus
//! interface: the one GNOME Settings uses.

use super::dbus;
use super::monitor::MonitorInfo;
use crate::output::CmdError;
use serde_json::{json, Value};
use std::collections::HashMap;
use zbus::zvariant::{OwnedValue, Value as ZValue};

const SERVICE: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";

type Props = HashMap<String, OwnedValue>;
/// Connector, vendor, product, serial.
type Spec = (String, String, String, String);
/// Id, width, height, refresh rate, preferred scale, supported scales, properties.
type Mode = (String, i32, i32, f64, f64, Vec<f64>, Props);
type Monitor = (Spec, Vec<Mode>, Props);
/// x, y, scale, transform, primary, monitors, properties.
type Logical = (i32, i32, f64, u32, bool, Vec<Spec>, Props);
type State = (u32, Vec<Monitor>, Vec<Logical>, Props);
/// What `ApplyMonitorsConfig` takes for one logical monitor: x, y, scale,
/// transform, primary, and its monitors as connector, mode id, properties.
type LogicalConfig<'a> = (i32, i32, f64, u32, bool, Vec<(String, String, HashMap<String, ZValue<'a>>)>);

/// Mutter's layout modes: in "logical" the desktop is measured in scaled
/// pixels, in "physical" in the monitors' own.
const LAYOUT_PHYSICAL: u32 = 2;
/// Applies until the monitors change. A persistent change makes GNOME ask a
/// person to keep it, and undo it if nobody answers.
const METHOD_TEMPORARY: u32 = 1;

fn mutter_err(e: zbus::Error) -> CmdError {
    let message = e.to_string();
    if message.contains("ServiceUnknown") || message.contains("UnknownObject") {
        CmdError::unsupported(format!("Mutter's display configuration interface is not available: {message}"))
    } else {
        CmdError::os(format!("Mutter rejected the display configuration: {message}"))
    }
}

fn state() -> Result<State, CmdError> {
    dbus::session()?
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "GetCurrentState", &())
        .and_then(|reply| reply.body().deserialize())
        .map_err(mutter_err)
}

fn prop_bool(props: &Props, key: &str) -> bool {
    props.get(key).and_then(|v| bool::try_from(&**v).ok()).unwrap_or(false)
}

fn prop_str(props: &Props, key: &str) -> Option<String> {
    props.get(key).and_then(|v| <&str>::try_from(&**v).ok()).map(str::to_string)
}

fn prop_u32(props: &Props, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| u32::try_from(&**v).ok())
}

fn current_mode(monitor: &Monitor) -> Option<&Mode> {
    monitor.1.iter().find(|m| prop_bool(&m.6, "is-current"))
}

fn logical_of<'a>(state: &'a State, connector: &str) -> Option<&'a Logical> {
    state.2.iter().find(|l| l.5.iter().any(|s| s.0 == connector))
}

fn describe(state: &State, monitor: &Monitor) -> MonitorInfo {
    let connector = monitor.0.0.clone();
    let mode = current_mode(monitor);
    let logical = logical_of(state, &connector);
    let (mut width, mut height) = mode.map(|m| (m.1 as f64, m.2 as f64)).unwrap_or((0.0, 0.0));
    if logical.is_some_and(|l| l.3 % 2 == 1) {
        std::mem::swap(&mut width, &mut height);
    }
    let physical = prop_u32(&state.3, "layout-mode") == Some(LAYOUT_PHYSICAL);
    let scale = logical.map(|l| l.2).filter(|s| *s > 0.0 && !physical).unwrap_or(1.0);
    MonitorInfo {
        name: prop_str(&monitor.2, "display-name").unwrap_or_else(|| connector.clone()),
        id: connector,
        x: logical.map(|l| l.0).unwrap_or(0),
        y: logical.map(|l| l.1).unwrap_or(0),
        width: (width / scale).round() as i32,
        height: (height / scale).round() as i32,
        primary: logical.is_some_and(|l| l.4),
        refresh_rate: mode.map(|m| m.3.round() as u32).unwrap_or(0),
        attached: logical.is_some() && mode.is_some(),
    }
}

pub fn list() -> Result<Vec<MonitorInfo>, CmdError> {
    let state = state()?;
    Ok(state.1.iter().map(|m| describe(&state, m)).collect())
}

/// The monitor's modes in the shape `monitor::pick_mode` reads.
fn modes_value(monitor: &Monitor) -> Value {
    let modes: Vec<Value> = monitor
        .1
        .iter()
        .map(|m| json!({ "id": m.0, "size": { "width": m.1, "height": m.2 }, "refreshRate": m.3 }))
        .collect();
    json!({ "currentModeId": current_mode(monitor).map(|m| m.0.clone()), "modes": modes })
}

/// One logical monitor of a new configuration.
struct Planned {
    x: i32,
    y: i32,
    scale: f64,
    transform: u32,
    primary: bool,
    connector: String,
    mode: String,
}

/// The scale closest to `wanted` that the mode supports.
fn fit_scale(mode: &Mode, wanted: f64) -> f64 {
    mode.5
        .iter()
        .copied()
        .min_by(|a, b| (a - wanted).abs().total_cmp(&(b - wanted).abs()))
        .unwrap_or(mode.4)
}

#[allow(clippy::too_many_arguments)]
pub fn set(
    id: &str,
    resolution: Option<&str>,
    refresh_rate: Option<u32>,
    rotation: Option<u32>,
    position: Option<(i32, i32)>,
    make_primary: bool,
    enabled: Option<bool>,
) -> Result<(), CmdError> {
    let state = state()?;
    let monitor = state
        .1
        .iter()
        .find(|m| m.0.0 == id)
        .ok_or_else(|| CmdError::not_found(format!("no monitor with id {id}")))?;

    // Every monitor that stays on has to be in the new configuration, one
    // logical monitor each (mirrored monitors keep only their first).
    let mut plan: Vec<Planned> = state
        .2
        .iter()
        .filter_map(|l| {
            let spec = l.5.first()?;
            let mode = state.1.iter().find(|m| m.0 == *spec).and_then(current_mode)?;
            Some(Planned {
                x: l.0,
                y: l.1,
                scale: l.2,
                transform: l.3,
                primary: l.4,
                connector: spec.0.clone(),
                mode: mode.0.clone(),
            })
        })
        .collect();

    if enabled == Some(false) {
        plan.retain(|p| p.connector != id);
        if plan.is_empty() {
            return Err(CmdError::invalid_args("can't turn off the only monitor that is on"));
        }
        if !plan.iter().any(|p| p.primary) {
            plan[0].primary = true;
        }
        return apply(&state, &plan);
    }

    if !plan.iter().any(|p| p.connector == id) {
        if enabled != Some(true) {
            return Err(CmdError::invalid_args(format!(
                "monitor {id} is off; pass --enabled on to turn it on"
            )));
        }
        // A monitor turned on goes right of the others, in its preferred mode.
        let mode = monitor
            .1
            .iter()
            .find(|m| prop_bool(&m.6, "is-preferred"))
            .or(monitor.1.first())
            .ok_or_else(|| CmdError::os(format!("monitor {id} has no modes")))?;
        let others = list()?;
        let right = others.iter().filter(|m| m.attached).map(|m| m.x + m.width).max().unwrap_or(0);
        plan.push(Planned {
            x: right,
            y: 0,
            scale: mode.4,
            transform: 0,
            primary: false,
            connector: id.to_string(),
            mode: mode.0.clone(),
        });
    }

    let target = plan.iter_mut().find(|p| p.connector == id).expect("added above");
    if resolution.is_some() || refresh_rate.is_some() {
        let mut modes = modes_value(monitor);
        modes["currentModeId"] = Value::from(target.mode.clone());
        target.mode = super::monitor::pick_mode(&modes, resolution, refresh_rate)?;
    }
    if let Some(mode) = monitor.1.iter().find(|m| m.0 == target.mode) {
        target.scale = fit_scale(mode, target.scale);
    }
    if let Some(rotation) = rotation {
        // Mutter's transforms turn the picture counter-clockwise, so 90 puts
        // its top on the panel's left edge, as on Windows.
        target.transform = match rotation {
            0 => 0,
            90 => 1,
            180 => 2,
            270 => 3,
            _ => return Err(CmdError::invalid_args("--rotation must be 0, 90, 180 or 270")),
        };
    }
    if let Some((x, y)) = position {
        if x < 0 || y < 0 {
            return Err(CmdError::invalid_args(
                "--position can't be negative: GNOME keeps the desktop's top-left corner at 0,0, so place this monitor at 0,0 and move the others instead",
            ));
        }
        (target.x, target.y) = (x, y);
    }
    if make_primary {
        for p in plan.iter_mut() {
            p.primary = p.connector == id;
        }
    }
    if resolution.is_none()
        && refresh_rate.is_none()
        && rotation.is_none()
        && position.is_none()
        && !make_primary
        && enabled.is_none()
    {
        return Err(CmdError::invalid_args("nothing to change: pass at least one setting"));
    }
    apply(&state, &plan)
}

fn apply(state: &State, plan: &[Planned]) -> Result<(), CmdError> {
    let logical: Vec<LogicalConfig> = plan
        .iter()
        .map(|p| {
            (p.x, p.y, p.scale, p.transform, p.primary, vec![(p.connector.clone(), p.mode.clone(), HashMap::new())])
        })
        .collect();
    let properties: HashMap<String, ZValue> = HashMap::new();
    dbus::session()?
        .call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "ApplyMonitorsConfig",
            &(state.0, METHOD_TEMPORARY, logical, properties),
        )
        .map(|_| ())
        .map_err(mutter_err)
}

/// Mutter's power saving switches every monitor at once, so with more than
/// one attached this refuses rather than blank them all.
pub fn power(id: &str, on: bool) -> Result<(), CmdError> {
    let monitors = list()?;
    if !monitors.iter().any(|m| m.id == id) {
        return Err(CmdError::not_found(format!("no monitor with id {id}")));
    }
    if monitors.iter().any(|m| m.id != id && m.attached) {
        return Err(CmdError::unsupported(
            "GNOME can only switch all monitors off or on together, so monitor power works only while a single monitor is attached",
        ));
    }
    // DPMS modes: 0 on, 3 off.
    let mode: i32 = if on { 0 } else { 3 };
    dbus::session()?
        .call_method(
            Some(SERVICE),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &(INTERFACE, "PowerSaveMode", ZValue::from(mode)),
        )
        .map(|_| ())
        .map_err(mutter_err)
}

/// One backlight: connector, lowest, highest and current value.
struct Backlight {
    connector: String,
    min: i32,
    max: i32,
    value: i32,
}

/// Mutter (GNOME 47+) lists a backlight per monitor that has one: a laptop
/// panel, or an external monitor that takes DDC/CI.
fn backlights() -> Result<Option<(u32, Vec<Backlight>)>, CmdError> {
    let reply = dbus::session()?.call_method(
        Some(SERVICE),
        PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(INTERFACE, "Backlight"),
    );
    let Ok(reply) = reply else {
        // An older Mutter without the property.
        return Ok(None);
    };
    let value: OwnedValue = reply.body().deserialize().map_err(dbus::err)?;
    let (serial, entries): (u32, Vec<Props>) = value
        .try_clone()
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| CmdError::os("unexpected backlight data from Mutter"))?;
    let int = |p: &Props, k: &str| p.get(k).and_then(|v| i32::try_from(&**v).ok());
    Ok(Some((
        serial,
        entries
            .iter()
            .filter_map(|p| {
                Some(Backlight {
                    connector: prop_str(p, "connector")?,
                    min: int(p, "min")?,
                    max: int(p, "max")?,
                    value: int(p, "value")?,
                })
            })
            .collect(),
    )))
}

fn no_brightness() -> CmdError {
    CmdError::unsupported("this monitor does not report a brightness")
}

/// Before Mutter had backlights, GNOME's settings daemon controlled the
/// built-in panel's, as a percentage.
const GSD_SERVICE: &str = "org.gnome.SettingsDaemon.Power";
const GSD_PATH: &str = "/org/gnome/SettingsDaemon/Power";
const GSD_INTERFACE: &str = "org.gnome.SettingsDaemon.Power.Screen";

fn gsd_brightness() -> Result<i32, CmdError> {
    let reply = dbus::session()?
        .call_method(
            Some(GSD_SERVICE),
            GSD_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(GSD_INTERFACE, "Brightness"),
        )
        .map_err(|_| no_brightness())?;
    let value: OwnedValue = reply.body().deserialize().map_err(dbus::err)?;
    i32::try_from(&*value).ok().filter(|b| *b >= 0).ok_or_else(no_brightness)
}

fn percent(b: &Backlight) -> u32 {
    if b.max <= b.min {
        return 0;
    }
    (((b.value - b.min) as f64 * 100.0) / (b.max - b.min) as f64).round() as u32
}

pub fn brightness(id: &str) -> Result<u32, CmdError> {
    super::monitor::find(id)?;
    match backlights()? {
        Some((_, all)) => all.iter().find(|b| b.connector == id).map(percent).ok_or_else(no_brightness),
        None => gsd_brightness().map(|b| b as u32),
    }
}

pub fn set_brightness(id: &str, level: u32) -> Result<(), CmdError> {
    super::monitor::find(id)?;
    let level = level.min(100);
    match backlights()? {
        Some((serial, all)) => {
            let b = all.iter().find(|b| b.connector == id).ok_or_else(no_brightness)?;
            let value = b.min + ((b.max - b.min) as f64 * level as f64 / 100.0).round() as i32;
            dbus::session()?
                .call_method(Some(SERVICE), PATH, Some(INTERFACE), "SetBacklight", &(serial, id, value))
                .map(|_| ())
                .map_err(mutter_err)
        }
        None => {
            gsd_brightness()?;
            dbus::session()?
                .call_method(
                    Some(GSD_SERVICE),
                    GSD_PATH,
                    Some("org.freedesktop.DBus.Properties"),
                    "Set",
                    &(GSD_INTERFACE, "Brightness", ZValue::from(level as i32)),
                )
                .map(|_| ())
                .map_err(dbus::err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, bool)]) -> Props {
        pairs.iter().map(|(k, v)| (k.to_string(), OwnedValue::from(*v))).collect()
    }

    fn sample(transform: u32, scale: f64) -> State {
        let spec: Spec = ("DP-1".into(), "v".into(), "p".into(), "s".into());
        let modes: Vec<Mode> = vec![
            ("2560x1440@59.951".into(), 2560, 1440, 59.951, 1.0, vec![1.0, 2.0], props(&[("is-current", true)])),
            ("1920x1080@60.000".into(), 1920, 1080, 60.0, 1.0, vec![1.0, 1.25], props(&[])),
        ];
        let off: Spec = ("HDMI-1".into(), "v".into(), "p".into(), "s".into());
        (
            7,
            vec![(spec.clone(), modes, Props::new()), (off, vec![], Props::new())],
            vec![(1920, 0, scale, transform, true, vec![spec], Props::new())],
            Props::new(),
        )
    }

    #[test]
    fn describes_monitors_in_logical_pixels() {
        let s = sample(0, 1.0);
        let m = describe(&s, &s.1[0]);
        assert_eq!((m.id.as_str(), m.x, m.y, m.width, m.height, m.refresh_rate), ("DP-1", 1920, 0, 2560, 1440, 60));
        assert!(m.primary && m.attached);
        let s = sample(1, 2.0);
        let m = describe(&s, &s.1[0]);
        assert_eq!((m.width, m.height), (720, 1280));
        assert!(!describe(&s, &s.1[1]).attached);
    }

    #[test]
    fn modes_go_through_the_shared_picker() {
        let s = sample(0, 1.0);
        let modes = modes_value(&s.1[0]);
        assert_eq!(super::super::monitor::pick_mode(&modes, Some("1920x1080"), None).unwrap(), "1920x1080@60.000");
        assert_eq!(fit_scale(&s.1[0].1[1], 2.0), 1.25);
    }
}
