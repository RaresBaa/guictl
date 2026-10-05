//! Linux backend, KDE Plasma 6 on Wayland only. Most of it talks to
//! KWin and the usual desktop services over D-Bus.

mod dbus;
mod kwin;
mod portal;

pub mod audio;
pub mod brightness;
pub mod clipboard;
pub mod element;
pub mod keyboard;
pub mod media;
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
    "element", "wait window", "wait element", "wait idle", "batch", "capabilities", "ocr", "setup",
];

pub const NOT_IMPLEMENTED: &[&str] = &[];

/// The Linux part of `guictl guide`.
pub const GUIDE: &str = r#"ON THIS SYSTEM (Linux, KDE Plasma on Wayland)
- If screenshots, `ocr` or `wait idle` fail with "permission denied", run
  `guictl setup` once. It may show a dialog a person has to accept.
- A key or mouse button can't stay held between two guictl commands. Put
  `keyboard hold` / `mouse press` and the matching release in one `batch`.
- `keyboard type` fails on a character the current keyboard layout can't
  produce. Put such text on the clipboard with `clipboard set` and paste
  it with `keyboard hotkey ctrl+v`.
- In `window list`, "app" is the program's executable name and "class" its
  application id; --app matches either.
- `screenshot --window <id>` shows that window's own contents even when
  other windows cover it. A click at the same spot hits whatever is on top,
  so run `window focus` first.
- Moving, resizing or maximizing a window takes effect a moment after the
  command returns. Read the new geometry with `window list` before using it.
- `element list` includes controls that exist but aren't drawn, such as the
  items of closed menus ("showing": false). `element invoke` works on those,
  so a menu command can be run without opening the menu; `element click`
  does not.
- Element rectangles are wrong for the items of an open popup menu. Use
  `element invoke` on them, not `element click` or the mouse.
- The first `element` command of a session switches accessibility support
  on. If a window then lists few or no elements, try again after a moment.
- Apps built on Chromium or Electron (VS Code, Chrome, Slack) expose no
  elements unless they were started with --force-renderer-accessibility. Use
  `ocr` and screenshots for them.
- `monitor power` works only with a single monitor attached, and any input
  turns the screen back on.
- `media` commands go to one player: the one playing, else a paused one. A
  browser counts as one player however many tabs play.
- `audio device disable` turns off the whole sound card the device belongs
  to, so every input and output on it.
"#;
