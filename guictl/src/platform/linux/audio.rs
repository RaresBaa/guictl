use crate::output::CmdError;
use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Serialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub direction: &'static str,
    pub state: &'static str,
}

/// `pactl` speaks the PulseAudio protocol, which both PipeWire (through
/// pipewire-pulse, Plasma's default) and a real PulseAudio daemon serve, so
/// one code path covers either sound server.
fn pactl(args: &[&str]) -> Result<String, CmdError> {
    let output = Command::new("pactl").args(args).output().map_err(|e| {
        CmdError::unsupported(format!("could not run pactl (part of libpulse): {e}"))
    })?;
    if !output.status.success() {
        return Err(CmdError::os(format!(
            "pactl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn pactl_list(kind: &str) -> Result<Vec<Value>, CmdError> {
    let raw = pactl(&["-f", "json", "list", kind])?;
    serde_json::from_str(&raw)
        .map_err(|e| CmdError::os(format!("could not parse pactl output: {e}")))
}

/// Average of the channels' volumes, as a percentage.
fn volume_percent(object: &Value) -> u32 {
    let levels: Vec<f64> = object["volume"]
        .as_object()
        .into_iter()
        .flat_map(|channels| channels.values())
        .filter_map(|c| c["value_percent"].as_str()?.trim_end_matches('%').trim().parse().ok())
        .collect();
    if levels.is_empty() {
        return 0;
    }
    (levels.iter().sum::<f64>() / levels.len() as f64).round() as u32
}

fn default_sink() -> Result<Value, CmdError> {
    let name = pactl(&["get-default-sink"])?;
    pactl_list("sinks")?
        .into_iter()
        .find(|s| s["name"].as_str() == Some(name.as_str()))
        .ok_or_else(|| CmdError::not_found("no default audio output"))
}

/// The playback streams belonging to `app`, matched against the stream's
/// process name or the application name it announces.
fn app_streams(app: &str) -> Result<Vec<Value>, CmdError> {
    let streams: Vec<Value> = pactl_list("sink-inputs")?
        .into_iter()
        .filter(|s| {
            ["application.process.binary", "application.name"]
                .iter()
                .any(|key| s["properties"][*key].as_str().is_some_and(|v| v.eq_ignore_ascii_case(app)))
        })
        .collect();
    if streams.is_empty() {
        return Err(CmdError::not_found(format!("no audio stream found for app '{app}'")));
    }
    Ok(streams)
}

fn for_each_stream(app: &str, command: &str, value: &str) -> Result<(), CmdError> {
    for stream in app_streams(app)? {
        let index = stream["index"].as_u64().unwrap_or(0).to_string();
        pactl(&[command, &index, value])?;
    }
    Ok(())
}

pub fn get_volume(app: Option<&str>) -> Result<u32, CmdError> {
    match app {
        Some(app) => Ok(volume_percent(&app_streams(app)?[0])),
        None => Ok(volume_percent(&default_sink()?)),
    }
}

pub fn set_volume(app: Option<&str>, level: u32) -> Result<(), CmdError> {
    let value = format!("{}%", level.min(100));
    match app {
        Some(app) => for_each_stream(app, "set-sink-input-volume", &value),
        None => pactl(&["set-sink-volume", "@DEFAULT_SINK@", &value]).map(|_| ()),
    }
}

pub fn set_mute(app: Option<&str>, mute: bool) -> Result<(), CmdError> {
    let value = if mute { "1" } else { "0" };
    match app {
        Some(app) => for_each_stream(app, "set-sink-input-mute", value),
        None => pactl(&["set-sink-mute", "@DEFAULT_SINK@", value]).map(|_| ()),
    }
}

fn is_monitor_source(source: &Value) -> bool {
    source["properties"]["device.class"].as_str() == Some("monitor")
}

fn device_info(device: &Value, direction: &'static str) -> AudioDeviceInfo {
    AudioDeviceInfo {
        id: device["name"].as_str().unwrap_or_default().to_string(),
        name: device["description"].as_str().unwrap_or_default().to_string(),
        direction,
        state: "active",
    }
}

/// Lists outputs (sinks) and inputs (sources). A sound card whose profile is
/// "off" has neither, so it's listed as one disabled entry with the card's
/// name as its id, which `audio device enable` accepts.
pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, CmdError> {
    let mut devices: Vec<AudioDeviceInfo> =
        pactl_list("sinks")?.iter().map(|s| device_info(s, "output")).collect();
    devices.extend(
        pactl_list("sources")?
            .iter()
            .filter(|s| !is_monitor_source(s))
            .map(|s| device_info(s, "input")),
    );
    for card in pactl_list("cards")? {
        if card["active_profile"].as_str() != Some("off") {
            continue;
        }
        let has_output = card["profiles"]
            .as_object()
            .is_some_and(|profiles| profiles.keys().any(|name| name.contains("output")));
        devices.push(AudioDeviceInfo {
            id: card["name"].as_str().unwrap_or_default().to_string(),
            name: card["properties"]["device.description"].as_str().unwrap_or_default().to_string(),
            direction: if has_output { "output" } else { "input" },
            state: "disabled",
        });
    }
    Ok(devices)
}

fn no_device(id: &str) -> CmdError {
    CmdError::not_found(format!("no audio device with id {id}"))
}

pub fn set_default_device(device_id: &str) -> Result<(), CmdError> {
    let is = |kind: &str| -> Result<bool, CmdError> {
        Ok(pactl_list(kind)?.iter().any(|d| d["name"].as_str() == Some(device_id)))
    };
    if is("sinks")? {
        pactl(&["set-default-sink", device_id]).map(|_| ())
    } else if is("sources")? {
        pactl(&["set-default-source", device_id]).map(|_| ())
    } else {
        Err(no_device(device_id))
    }
}

/// The sound card a device id refers to: the card itself, or the one that
/// provides the given sink or source.
fn card_for(device_id: &str) -> Result<Value, CmdError> {
    let mut card_name = device_id.to_string();
    for kind in ["sinks", "sources"] {
        if let Some(device) =
            pactl_list(kind)?.into_iter().find(|d| d["name"].as_str() == Some(device_id))
        {
            card_name = device["properties"]["device.name"]
                .as_str()
                .ok_or_else(|| {
                    CmdError::unsupported("this device is not backed by a sound card, so it can't be enabled or disabled")
                })?
                .to_string();
            break;
        }
    }
    pactl_list("cards")?
        .into_iter()
        .find(|c| c["name"].as_str() == Some(card_name.as_str()))
        .ok_or_else(|| no_device(device_id))
}

/// PulseAudio has no per-device enable switch; the equivalent is the sound
/// card's profile. Disabling sets it to "off" (which removes all of the
/// card's inputs and outputs), enabling picks its highest-priority profile.
pub fn set_enabled(device_id: &str, enabled: bool) -> Result<(), CmdError> {
    let card = card_for(device_id)?;
    let name = card["name"].as_str().unwrap_or_default();
    if !enabled {
        return pactl(&["set-card-profile", name, "off"]).map(|_| ());
    }
    if card["active_profile"].as_str() != Some("off") {
        return Ok(());
    }
    let best = card["profiles"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(profile, info)| *profile != "off" && info["available"].as_bool() != Some(false))
        .max_by_key(|(_, info)| info["priority"].as_i64().unwrap_or(0))
        .map(|(profile, _)| profile.as_str())
        .ok_or_else(|| CmdError::os("the sound card has no usable profile"))?;
    pactl(&["set-card-profile", name, best]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn volume_is_the_channel_average() {
        let sink = json!({ "volume": {
            "front-left": { "value": 32768, "value_percent": "50%", "db": "-18.06 dB" },
            "front-right": { "value_percent": "61%" },
        }});
        assert_eq!(volume_percent(&sink), 56);
        assert_eq!(volume_percent(&json!({ "volume": {} })), 0);
        assert_eq!(volume_percent(&json!({})), 0);
    }

    #[test]
    fn monitor_sources_are_recognised() {
        assert!(is_monitor_source(&json!({ "properties": { "device.class": "monitor" } })));
        assert!(!is_monitor_source(&json!({ "properties": { "device.class": "sound" } })));
        assert!(!is_monitor_source(&json!({})));
    }
}
