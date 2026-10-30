//! Noticing that a person is using the mouse or keyboard while guictl drives
//! them, for `batch --stop-on-user-input`. Wayland doesn't show a client input
//! meant for other windows, so this reads the devices under /dev/input, which
//! takes membership of the `input` group. guictl's own input goes through the
//! compositor and never reaches those devices, so anything seen there came
//! from a physical one. It only notes that something happened: which key was
//! pressed is never looked at, and nothing stops the event reaching the apps.

use crate::output::CmdError;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;

/// `struct input_event`: a timestamp of two longs, then type, code and value.
const TIME_SIZE: usize = 2 * size_of::<usize>();
const EVENT_SIZE: usize = TIME_SIZE + 8;

/// A device plugged in during a batch is picked up within this long.
const RESCAN: Duration = Duration::from_secs(2);

static SEEN: AtomicBool = AtomicBool::new(false);
static WATCHED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub struct Watch;

impl Watch {
    /// Whether someone has used the mouse or keyboard since the last call.
    pub fn take(&self) -> bool {
        SEEN.swap(false, Ordering::SeqCst)
    }
}

/// A key release doesn't count: the key that started guictl, such as the
/// return that ran the command, comes up just after the watch begins.
fn is_person(event: &[u8]) -> bool {
    let kind = u16::from_ne_bytes([event[TIME_SIZE], event[TIME_SIZE + 1]]);
    let value = i32::from_ne_bytes([
        event[TIME_SIZE + 4],
        event[TIME_SIZE + 5],
        event[TIME_SIZE + 6],
        event[TIME_SIZE + 7],
    ]);
    match kind {
        EV_KEY => value != 0,
        EV_REL | EV_ABS => true,
        _ => false,
    }
}

/// The event devices of keyboards, mice, touchpads and touchscreens, from
/// /proc/bus/input/devices. This leaves out what isn't a person at the
/// computer: headphone jacks, accelerometers, a drifting gamepad stick.
fn input_devices(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| line.strip_prefix("H: Handlers="))
        .filter(|handlers| handlers.split_whitespace().any(|h| h == "kbd" || h.starts_with("mouse")))
        .filter_map(|handlers| handlers.split_whitespace().find(|h| h.starts_with("event")))
        .map(String::from)
        .collect()
}

fn devices() -> Result<Vec<String>, CmdError> {
    let listing = std::fs::read_to_string("/proc/bus/input/devices")
        .map_err(|e| CmdError::os(format!("could not list input devices: {e}")))?;
    let names = input_devices(&listing);
    if names.is_empty() {
        return Err(CmdError::os("--stop-on-user-input found no keyboard or mouse to watch"));
    }
    Ok(names)
}

/// None if the device has just been unplugged.
fn open(name: &str) -> Result<Option<File>, CmdError> {
    match File::open(format!("/dev/input/{name}")) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Err(CmdError::permission_denied(format!(
            "--stop-on-user-input can't read /dev/input/{name}: it needs this user in the `input` group. Tell the person, since it is their decision: every program they run could then read what they type. To join, run `sudo usermod -aG input $USER` (on NixOS, list the user in the guictl module's `programs.guictl.inputUsers`) and log in again. Without it, run the batch without this flag"
        ))),
        Err(e) => Err(CmdError::os(format!("could not open /dev/input/{name}: {e}"))),
    }
}

fn read_device(name: String, mut file: File) {
    let mut buf = [0u8; EVENT_SIZE * 64];
    while let Ok(n @ 1..) = file.read(&mut buf) {
        if buf[..n].chunks_exact(EVENT_SIZE).any(is_person) {
            SEEN.store(true, Ordering::SeqCst);
        }
    }
    // Gone, most likely unplugged. The name can come back as another device.
    WATCHED.lock().unwrap().remove(&name);
}

fn start_reading(name: &str, file: File) {
    WATCHED.lock().unwrap().insert(name.to_string());
    let name = name.to_string();
    std::thread::spawn(move || read_device(name, file));
}

/// Whether `watch` can read every device right now.
pub fn available() -> bool {
    devices().is_ok_and(|names| names.iter().all(|name| open(name).is_ok()))
}

/// Starts watching. One device that can't be read fails the whole call: a
/// watch that sees the keyboard and misses the mouse would look like safety.
/// Each device is read on a thread of its own until the process exits.
pub fn watch() -> Result<Watch, CmdError> {
    let mut opened = Vec::new();
    for name in devices()? {
        if let Some(file) = open(&name)? {
            opened.push((name, file));
        }
    }
    for (name, file) in opened {
        start_reading(&name, file);
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(RESCAN);
        for name in devices().unwrap_or_default() {
            if !WATCHED.lock().unwrap().contains(&name) {
                if let Ok(Some(file)) = open(&name) {
                    start_reading(&name, file);
                }
            }
        }
    });
    SEEN.store(false, Ordering::SeqCst);
    Ok(Watch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: u16, value: i32) -> Vec<u8> {
        let mut bytes = vec![0; TIME_SIZE];
        bytes.extend(kind.to_ne_bytes());
        bytes.extend(0u16.to_ne_bytes());
        bytes.extend(value.to_ne_bytes());
        bytes
    }

    #[test]
    fn presses_and_movement_count_but_releases_and_the_rest_do_not() {
        assert!(is_person(&event(EV_KEY, 1)));
        assert!(is_person(&event(EV_KEY, 2)));
        assert!(!is_person(&event(EV_KEY, 0)));
        assert!(is_person(&event(EV_REL, -3)));
        assert!(is_person(&event(EV_ABS, 120)));
        assert!(!is_person(&event(0, 0))); // EV_SYN
        assert!(!is_person(&event(0x05, 1))); // EV_SW, a headphone jack
        assert!(!is_person(&event(0x11, 1))); // EV_LED, caps lock's light
    }

    #[test]
    fn only_keyboards_and_pointers_are_watched() {
        let listing = "\
N: Name=\"Logitech G413\"
H: Handlers=sysrq kbd leds event11

N: Name=\"Razer Basilisk\"
H: Handlers=event7 mouse1

N: Name=\"Razer Basilisk\"
H: Handlers=event9

N: Name=\"HD-Audio Generic Line\"
H: Handlers=event19
";
        assert_eq!(input_devices(listing), ["event11", "event7"]);
    }
}
