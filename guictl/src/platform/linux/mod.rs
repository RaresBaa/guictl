//! Linux backend, for KDE Plasma 6 and GNOME (45+) on Wayland. Most of it
//! talks to the compositor (KWin, or GNOME Shell through guictl's own
//! extension) and the usual desktop services over D-Bus.

mod dbus;
mod gnome;
mod kwin;
mod mutter;
mod portal;

pub mod audio;
pub mod brightness;
pub mod clipboard;
pub mod element;
pub mod input;
pub mod keyboard;
pub mod media;
pub mod menu;
pub mod monitor;
pub mod mouse;
pub mod notify;
pub mod ocr;
pub mod screenshot;
pub mod setup;
pub mod system;
pub mod window;
pub mod workspace;

pub const IMPLEMENTED: &[&str] = &[
    "mouse", "keyboard", "window", "workspace", "clipboard text", "clipboard image", "monitor list", "monitor set", "monitor power",
    "open", "launch", "system lock/sleep/keep-awake",
    "media play/pause/next/previous", "media now-playing", "notify", "screenshot",
    "volume", "audio device list/set-default/enable/disable", "brightness",
    "element", "menu", "wait window", "wait element", "wait idle", "batch", "batch --stop-on-user-input", "capabilities", "ocr", "setup",
];

/// Implemented commands this machine can't run right now: the ones that need
/// the accessibility bus, when nothing provides it, the one that needs to
/// read the input devices, when this user isn't allowed to, and on GNOME the
/// ones that need guictl's Shell extension, while it isn't running.
pub fn unavailable() -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !element::bus_exists() {
        missing.extend(["element", "menu", "wait element"]);
    }
    if !input::available() {
        missing.push("batch --stop-on-user-input");
    }
    if gnome::is_gnome() && !gnome::ready() {
        missing.extend([
            "mouse", "window", "workspace", "clipboard text", "clipboard image", "screenshot", "ocr",
            "wait window", "wait idle", "element", "menu", "wait element",
        ]);
        missing.sort_unstable();
        missing.dedup();
    }
    missing
}

pub const NOT_IMPLEMENTED: &[&str] = &[];

/// The Linux part of `guictl guide`: what differs between the desktops,
/// then what they share.
pub fn guide() -> String {
    if gnome::is_gnome() {
        format!("{GUIDE_GNOME}{GUIDE_SHARED}{GUIDE_GNOME_MONITORS}")
    } else {
        format!("{GUIDE_KDE}{GUIDE_SHARED}{GUIDE_KDE_MONITORS}")
    }
}

const GUIDE_KDE: &str = r#"ON THIS SYSTEM (Linux, KDE Plasma on Wayland)
- If screenshots, `ocr` or `wait idle` fail with "permission denied", run
  `guictl setup` once. It may show a dialog a person has to accept.
- The first `mouse` or `keyboard` command shows Plasma's "Remote control"
  permission dialog and waits for a person to accept it. The dialog names
  the program guictl was started from (the terminal, editor or agent
  host), not guictl. If a command fails with code 4 "permission denied",
  tell the user to accept that dialog, then run the command again. Running
  guictl from another program may ask again.
"#;

const GUIDE_GNOME: &str = r#"ON THIS SYSTEM (Linux, GNOME on Wayland)
- Most commands go through guictl's own GNOME Shell extension. If one fails
  saying the extension is not running, ask the user to run `guictl setup`
  and then log out and back in once: GNOME loads extensions only at login.
- The first `mouse` or `keyboard` command shows GNOME's "Remote Desktop"
  permission dialog and waits for a person to accept it. If a command fails
  with code 4 "permission denied", tell the user to accept that dialog,
  then run the command again. Running guictl from another program may ask
  again.
- Window ids are numbers. Workspace ids are positions (1 is the first), so
  they change when a workspace before them goes away. GNOME normally adds
  and removes workspaces itself and keeps one empty workspace at the end;
  `workspace create` then hands back that one.
"#;

const GUIDE_SHARED: &str = r#"- A key or mouse button can't stay held between two guictl commands. Put
  `keyboard hold` / `mouse press` and the matching release in one `batch`.
- In `window list`, "app" is the program's executable name and "class" its
  application id; --app matches either.
- `screenshot --window <id>` shows that window's own contents even when
  other windows cover it. A click at the same spot hits whatever is on top,
  so run `window focus` first.
- `element list` includes controls that exist but aren't drawn, such as the
  items of closed menus ("showing": false). `element invoke` works on those,
  so a menu command can be run without opening the menu; `element click`
  does not.
- `screenshot --window` includes the window's open menus and dropdowns, and
  grows if they reach past it. The result's x and y say where the image's
  top-left corner is on the desktop, which may not be the window's corner.
- On a dropdown in Qt or GTK 4, and on a Qt tree or list row, `element select`
  and `element expand` bring the window forward and use the keyboard or a
  click. A Qt row scrolled out of view only gets highlighted.
- While a Firefox `<select>` list is open, the ids of the other elements in
  that window are off by one. List them again before using one you got earlier.
- In Qt apps (most of KDE), a right-click menu and other popups that
  don't come from the menu bar are missing from `element list`. Pick from
  them with the arrow keys and enter, or find the item with `ocr` and
  click it.
- The first `element` command of a session switches accessibility support
  on. If a window then lists few or no elements, try again after a moment.
- Start apps built on Chromium or Electron (VS Code, Chrome, Slack) with
  --force-renderer-accessibility, e.g. `launch code --
  --force-renderer-accessibility`; without it they expose no elements. If
  one is already running without it, a second launch only hands over to the
  running one: use `ocr` and screenshots, or ask the user before restarting
  it.
- `monitor power` works only with a single monitor attached, and any input
  turns the screen back on.
- `menu` needs a menu bar, even a hidden one. For a window without one
  (a hamburger menu), `element find --name <item>` the item and `element
  invoke` it.
- `batch --stop-on-user-input` needs the user in the `input` group, and
  fails with "permission denied" otherwise. Joining lets every program they
  run read what they type, so that is for them to decide, not you.
- `media` commands go to one player: the one playing, else a paused one. A
  browser counts as one player however many tabs play.
- `audio device disable` turns off only that device on most sound cards, but
  the whole card, so every input and output on it, when the card is in a
  "Pro Audio", UCM ("HiFi") or Bluetooth profile.
"#;

const GUIDE_KDE_MONITORS: &str = r#"- `system status` can't say how long the computer has been idle, so it has
  no "idle_ms": Wayland doesn't tell programs.
- `monitor set --position` can't be negative: KScreen keeps the desktop's
  top-left corner at 0,0. To put a monitor left of or above another, place
  it at 0,0 and move the others instead.
"#;

const GUIDE_GNOME_MONITORS: &str = r#"- `monitor set` lasts until a monitor is plugged in or out or the user
  logs in again; GNOME doesn't save it. `--position` can't be negative:
  GNOME keeps the desktop's top-left corner at 0,0. To put a monitor left
  of or above another, place it at 0,0 and move the others instead.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_guide_part_fits_a_terminal() {
        for part in [GUIDE_KDE, GUIDE_GNOME, GUIDE_SHARED, GUIDE_KDE_MONITORS, GUIDE_GNOME_MONITORS] {
            assert!(part.ends_with('\n'));
            for line in part.lines() {
                assert!(line.chars().count() <= 80, "longer than 80 columns: {line}");
            }
        }
    }
}
