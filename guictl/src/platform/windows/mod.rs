pub mod audio;
pub mod brightness;
pub mod clipboard;
pub mod element;
pub mod elevate;
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
    "mouse", "keyboard", "window", "clipboard", "monitor list",
    "open", "launch", "system lock/sleep/keep-awake",
    "media play/pause/next/previous", "media now-playing", "notify", "screenshot", "element",
    "volume", "audio device list/set-default/enable/disable", "brightness", "monitor set",
    "wait", "batch", "batch --stop-on-user-input", "capabilities", "ocr", "setup", "menu",
    "--elevate",
];

/// Implemented commands this machine can't run right now; none here.
pub fn unavailable() -> &'static [&'static str] {
    &[]
}

pub const NOT_IMPLEMENTED: &[&str] = &[
    "monitor power", "monitor set --enabled",
    "workspace", "window send --workspace",
];

/// The Windows part of `guictl guide`.
pub const GUIDE: &str = r#"ON THIS SYSTEM (Windows)
- Virtual desktops (`workspace`, `window send --workspace`), `monitor power`
  and `monitor set --enabled` are not supported; they exit with code 10.
- Windows of programs running as administrator ignore a guictl that is not
  itself running as administrator: commands that act on one, or read its
  elements, fail with "permission_denied". `window focus` and screenshots
  still work. Add `--elevate` to run one command as administrator. It shows a
  prompt that only the person can answer, and waits about 2 minutes for them,
  so tell them first and don't use it unless the task needs it. If they say No
  or don't answer, it fails with "permission_denied" and nothing ran. Each run
  prompts again, so put several commands in one `--elevate batch --file <path>`.
  Whatever that command launches runs as administrator too.
- Focus can return to the terminal guictl was started from between two
  commands, so `keyboard type` after a separate `window focus` may land in
  the wrong place. Put both in one `batch`, or use `element text set`.
  `window focus` fails, rather than answering "ok", when Windows keeps
  another window in front.
- If a mouse command fails because Windows "refused to move the mouse
  pointer", a virtual machine or remote desktop window has captured the
  mouse, or a prompt holds it. Stop and tell the person.
- On a locked screen `window active` fails with "not_found", and a screenshot
  shows the lock screen.
- `system status` also prints "idle_ms", how long since the last mouse or
  keyboard input. If it is small and you didn't just send any, someone is
  probably using the computer. Your own input resets it too.
- `brightness` works on external monitors that support DDC/CI, not on a
  laptop's built-in screen.
- A half-checked box shows "mixed": true instead of "checked". Otherwise a
  missing "checked" doesn't mean off: items of a native menu only report it
  when ticked, and a control that draws itself reports nothing (it may list
  as a plain pane). Take a screenshot to see its state.
"#;
