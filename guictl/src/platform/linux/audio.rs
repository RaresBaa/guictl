use super::window::unwrapped_name;
use crate::output::CmdError;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
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
                .any(|key| s["properties"][*key].as_str().is_some_and(|v| unwrapped_name(v).eq_ignore_ascii_case(unwrapped_name(app))))
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

/// The devices an ALSA card profile gives, when PipeWire built it from the
/// card's mappings: "output:analog-stereo+input:analog-stereo" gives those two
/// parts and "off" none. Other profiles (pro-audio, UCM's "HiFi", Bluetooth's)
/// aren't made of parts, so they give None.
fn profile_parts(profile: &str) -> Option<BTreeSet<&str>> {
    if profile == "off" {
        return Some(BTreeSet::new());
    }
    profile
        .split('+')
        .map(|part| (part.starts_with("output:") || part.starts_with("input:")).then_some(part))
        .collect()
}

/// The card's available profile that gives exactly `parts`.
fn profile_with<'a>(card: &'a Value, parts: &BTreeSet<&str>) -> Option<&'a str> {
    card["profiles"]
        .as_object()?
        .iter()
        .find(|(name, info)| {
            info["available"].as_bool() != Some(false) && profile_parts(name).as_ref() == Some(parts)
        })
        .map(|(name, _)| name.as_str())
}

/// The id PipeWire gives the device that a profile part makes, such as
/// "alsa_input.pci-0000_00_1f.3.analog-stereo" for "input:analog-stereo" on
/// "alsa_card.pci-0000_00_1f.3", so a disabled device keeps the id it has
/// while enabled.
fn part_id(card_name: &str, part: &str) -> Option<String> {
    let (direction, mapping) = part.split_once(':')?;
    Some(format!("alsa_{direction}.{}.{mapping}", card_name.strip_prefix("alsa_card.")?))
}

/// The parts the card's active profile gives, or None when it isn't made of parts.
fn active_parts(card: &Value) -> Option<BTreeSet<&str>> {
    profile_parts(card["active_profile"].as_str()?)
}

/// Every part any of the card's profiles gives.
fn all_parts(card: &Value) -> BTreeSet<&str> {
    card["profiles"]
        .as_object()
        .into_iter()
        .flat_map(|profiles| profiles.keys())
        .filter_map(|name| profile_parts(name))
        .flatten()
        .collect()
}

/// The parts that are off on a card that is on, but that one of its profiles
/// could add without dropping any device that is on.
fn addable_parts(card: &Value) -> Vec<&str> {
    let Some(active) = active_parts(card).filter(|parts| !parts.is_empty()) else {
        return Vec::new();
    };
    all_parts(card)
        .into_iter()
        .filter(|part| {
            let mut parts = active.clone();
            parts.insert(part) && profile_with(card, &parts).is_some()
        })
        .collect()
}

/// Lists outputs (sinks) and inputs (sources). A device switched off on its
/// own, by a card profile that leaves it out, is listed as disabled under
/// the id it has while on. A sound card whose profile is "off" has no
/// devices, so it's listed as one disabled entry with the card's name as its
/// id; `audio device enable` accepts either.
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
        let card_name = card["name"].as_str().unwrap_or_default();
        let description = card["properties"]["device.description"].as_str().unwrap_or_default();
        for part in addable_parts(&card) {
            if let Some(id) = part_id(card_name, part) {
                devices.push(AudioDeviceInfo {
                    id,
                    name: description.to_string(),
                    direction: if part.starts_with("output:") { "output" } else { "input" },
                    state: "disabled",
                });
            }
        }
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

/// The sound card a device id refers to, and the profile part that gives the
/// device: the card itself (no part), the card that provides the given sink
/// or source, or the card with a profile part that would give a device of
/// that id. The part is None when the card's profiles aren't made of parts.
fn card_for(device_id: &str) -> Result<(Value, Option<String>), CmdError> {
    let cards = pactl_list("cards")?;
    let find_card = |name: &str| cards.iter().find(|c| c["name"].as_str() == Some(name)).cloned();
    if let Some(card) = find_card(device_id) {
        return Ok((card, None));
    }
    for (kind, direction) in [("sinks", "output"), ("sources", "input")] {
        if let Some(device) =
            pactl_list(kind)?.into_iter().find(|d| d["name"].as_str() == Some(device_id))
        {
            let properties = &device["properties"];
            let card = properties["device.name"].as_str().and_then(find_card).ok_or_else(|| {
                CmdError::unsupported("this device is not backed by a sound card, so it can't be enabled or disabled")
            })?;
            let part = properties["device.profile.name"].as_str().map(|mapping| format!("{direction}:{mapping}"));
            return Ok((card, part));
        }
    }
    for card in &cards {
        let card_name = card["name"].as_str().unwrap_or_default();
        if let Some(part) = all_parts(card).into_iter().find(|part| part_id(card_name, part).as_deref() == Some(device_id)) {
            let part = part.to_string();
            return Ok((card.clone(), Some(part)));
        }
    }
    Err(no_device(device_id))
}

/// PulseAudio has no per-device enable switch; the equivalent is the sound
/// card's profile. A device switches on or off on its own when the card has
/// a profile that gives the same devices as now, plus or minus that one.
/// Otherwise, and for a card id, the whole card switches: disabling sets its
/// profile to "off" (which removes all of its inputs and outputs), enabling
/// picks its highest-priority profile.
pub fn set_enabled(device_id: &str, enabled: bool) -> Result<(), CmdError> {
    let (card, part) = card_for(device_id)?;
    let name = card["name"].as_str().unwrap_or_default();
    if let (Some(part), Some(mut parts)) = (part.as_deref(), active_parts(&card)) {
        if enabled {
            parts.insert(part);
        } else {
            parts.remove(part);
        }
        if let Some(profile) = profile_with(&card, &parts) {
            if card["active_profile"].as_str() == Some(profile) {
                return Ok(());
            }
            return pactl(&["set-card-profile", name, profile]).map(|_| ());
        }
    }
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

    fn card(active: &str) -> Value {
        json!({
            "name": "alsa_card.pci-0000_02_02.0",
            "active_profile": active,
            "profiles": {
                "off": { "available": true },
                "output:analog-stereo+input:analog-stereo": { "available": true },
                "output:analog-stereo": { "available": true },
                "input:analog-stereo": { "available": true },
                "output:hdmi-stereo": { "available": false },
                "pro-audio": { "available": true },
            },
        })
    }

    #[test]
    fn profiles_split_into_parts() {
        let parts = |p| profile_parts(p).map(|s| s.into_iter().collect::<Vec<_>>());
        assert_eq!(parts("off"), Some(vec![]));
        assert_eq!(
            parts("output:analog-stereo+input:analog-stereo"),
            Some(vec!["input:analog-stereo", "output:analog-stereo"])
        );
        assert_eq!(parts("pro-audio"), None);
        assert_eq!(parts("HiFi"), None);
        assert_eq!(parts("output:analog-stereo+pro-audio"), None);
    }

    #[test]
    fn profile_with_skips_unavailable_ones() {
        let card = card("off");
        let set = |parts: &[&'static str]| parts.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            profile_with(&card, &set(&["input:analog-stereo", "output:analog-stereo"])),
            Some("output:analog-stereo+input:analog-stereo")
        );
        assert_eq!(profile_with(&card, &set(&[])), Some("off"));
        assert_eq!(profile_with(&card, &set(&["output:hdmi-stereo"])), None);
    }

    #[test]
    fn part_ids_follow_pipewire_names() {
        assert_eq!(
            part_id("alsa_card.pci-0000_02_02.0", "input:analog-stereo").as_deref(),
            Some("alsa_input.pci-0000_02_02.0.analog-stereo")
        );
        assert_eq!(part_id("bluez_card.00_11_22", "output:a2dp"), None);
    }

    #[test]
    fn addable_parts_are_the_ones_a_profile_adds() {
        assert_eq!(addable_parts(&card("output:analog-stereo")), vec!["input:analog-stereo"]);
        assert!(addable_parts(&card("output:analog-stereo+input:analog-stereo")).is_empty());
        assert!(addable_parts(&card("off")).is_empty());
        assert!(addable_parts(&card("pro-audio")).is_empty());
    }
}
