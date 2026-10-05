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
    "mouse", "keyboard", "window", "clipboard", "monitor list",
    "open", "launch", "system lock/sleep/keep-awake",
    "media play/pause/next/previous", "media now-playing", "notify", "screenshot", "element",
    "volume", "audio device list/set-default/enable/disable", "brightness", "monitor set",
    "wait", "batch", "capabilities", "ocr", "setup",
];

pub const NOT_IMPLEMENTED: &[&str] = &[
    "monitor power", "monitor set --enabled",
    "workspace", "window send --workspace",
];

/// The Windows part of `guictl guide`.
pub const GUIDE: &str = r#"ON THIS SYSTEM (Windows)
- Virtual desktops (`workspace`, `window send --workspace`), `monitor power`
  and `monitor set --enabled` are not supported; they exit with code 10.
- Windows of programs running as administrator ignore a guictl that is not
  itself running as administrator, and Windows hides the refusal: the
  command still answers "ok". Check the result with `window list`.
- Focus can return to the terminal guictl was started from between two
  commands, so `keyboard type` after a separate `window focus` may land in
  the wrong place. Put both in one `batch`, or use `element text set`.
- `brightness` works on external monitors that support DDC/CI, not on a
  laptop's built-in screen.
"#;
