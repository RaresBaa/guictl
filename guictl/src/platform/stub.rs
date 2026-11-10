//! Backend for platforms guictl has no support for (anything other than
//! Windows and Linux). It exists so the tool still builds there and every
//! command fails cleanly with `unsupported` (exit 10) instead of the build
//! breaking. `RUSTFLAGS="--cfg guictl_stub" cargo check` compiles it on a
//! supported platform, to catch it falling behind the real backends.

use crate::output::CmdError;

pub const IMPLEMENTED: &[&str] = &["capabilities", "batch", "guide"];

/// Implemented commands this machine can't run right now; none here.
pub fn unavailable() -> &'static [&'static str] {
    &[]
}

pub const NOT_IMPLEMENTED: &[&str] = &[
    "mouse", "keyboard", "window", "workspace", "clipboard", "monitor", "open", "launch",
    "system", "media", "notify", "screenshot", "element", "volume", "audio", "brightness",
    "wait", "ocr", "setup", "menu",
];

pub const GUIDE: &str = "ON THIS SYSTEM\n- guictl does not support this operating system. Every command that touches\n  the desktop fails with exit code 10.\n";

fn unsupported<T>(area: &str) -> Result<T, CmdError> {
    Err(CmdError::unsupported(format!(
        "{area} is not supported on {}; guictl runs on Windows and on Linux with KDE Plasma",
        std::env::consts::OS
    )))
}

pub mod mouse {
    use super::unsupported;
    use crate::cli::{MouseButton, ScrollDirection};
    use crate::output::CmdError;

    pub fn position() -> Result<(i32, i32), CmdError> {
        unsupported("mouse")
    }
    pub fn move_to(_x: i32, _y: i32) -> Result<(), CmdError> {
        unsupported("mouse")
    }
    pub fn press(_button: MouseButton) -> Result<(), CmdError> {
        unsupported("mouse")
    }
    pub fn release(_button: MouseButton) -> Result<(), CmdError> {
        unsupported("mouse")
    }
    pub fn click(_button: MouseButton, _double: bool, _at: Option<(i32, i32)>) -> Result<(), CmdError> {
        unsupported("mouse")
    }
    pub fn scroll(_amount: i32, _direction: ScrollDirection) -> Result<(), CmdError> {
        unsupported("mouse")
    }
}

pub mod keyboard {
    use super::unsupported;
    use crate::output::CmdError;

    pub fn type_text(_text: &str) -> Result<(), CmdError> {
        unsupported("keyboard")
    }
    pub fn key(_name: &str, _repeat: u32) -> Result<(), CmdError> {
        unsupported("keyboard")
    }
    pub fn hotkey(_combo: &str) -> Result<(), CmdError> {
        unsupported("keyboard")
    }
    pub fn hold(_name: &str) -> Result<(), CmdError> {
        unsupported("keyboard")
    }
    pub fn release(_name: &str) -> Result<(), CmdError> {
        unsupported("keyboard")
    }
}

pub mod window {
    use super::unsupported;
    use crate::cli::WindowState;
    use crate::output::CmdError;
    use serde::Serialize;

    pub type WindowHandle = u64;

    #[derive(Serialize)]
    pub struct WindowInfo {
        pub id: String,
        pub title: String,
        pub app: String,
        pub pid: u32,
    }

    pub fn list(_app_filter: Option<&str>) -> Result<Vec<WindowInfo>, CmdError> {
        unsupported("window")
    }
    pub fn active() -> Result<WindowInfo, CmdError> {
        unsupported("window")
    }
    pub fn resolve(
        _id: Option<&str>,
        _active_flag: bool,
        _title: Option<&str>,
        _app: Option<&str>,
        _pid: Option<u32>,
    ) -> Result<WindowHandle, CmdError> {
        unsupported("window")
    }
    pub fn rect(_handle: WindowHandle) -> Result<(i32, i32, i32, i32), CmdError> {
        unsupported("window")
    }
    pub fn focus(_handle: WindowHandle) -> Result<(), CmdError> {
        unsupported("window")
    }
    pub fn move_window(_handle: WindowHandle, _x: i32, _y: i32) -> Result<(), CmdError> {
        unsupported("window")
    }
    pub fn resize_window(_handle: WindowHandle, _w: i32, _h: i32) -> Result<(), CmdError> {
        unsupported("window")
    }
    pub fn set_state(_handle: WindowHandle, _state: WindowState) -> Result<(), CmdError> {
        unsupported("window")
    }
    pub fn close(_handle: WindowHandle) -> Result<(), CmdError> {
        unsupported("window")
    }
    pub fn kill(_handle: WindowHandle) -> Result<u32, CmdError> {
        unsupported("window")
    }
    pub fn pin(_handle: WindowHandle, _on: bool) -> Result<(), CmdError> {
        unsupported("window")
    }
}

pub mod workspace {
    use super::unsupported;
    use super::window::WindowHandle;
    use crate::output::CmdError;
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct WorkspaceInfo {
        pub id: String,
    }

    pub fn list() -> Result<Vec<WorkspaceInfo>, CmdError> {
        unsupported("workspace")
    }
    pub fn switch(_id: &str) -> Result<(), CmdError> {
        unsupported("workspace")
    }
    pub fn remove(_id: &str) -> Result<(), CmdError> {
        unsupported("workspace")
    }
    pub fn create(_name: Option<&str>) -> Result<WorkspaceInfo, CmdError> {
        unsupported("workspace")
    }
    pub fn send_window(_window: WindowHandle, _id: &str) -> Result<(), CmdError> {
        unsupported("workspace")
    }
}

pub mod element {
    use super::unsupported;
    use super::window::WindowHandle;
    use crate::output::CmdError;
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct ElementInfo {
        pub id: String,
        pub value: Option<String>,
        pub disabled: bool,
    }

    pub fn list(_window: WindowHandle, _role: Option<&str>) -> Result<Vec<ElementInfo>, CmdError> {
        unsupported("element")
    }
    pub fn find(
        _window: WindowHandle,
        _name: Option<&str>,
        _role: Option<&str>,
    ) -> Result<ElementInfo, CmdError> {
        unsupported("element")
    }
    pub fn find_all(
        _window: WindowHandle,
        _name: Option<&str>,
        _role: Option<&str>,
    ) -> Result<Vec<ElementInfo>, CmdError> {
        unsupported("element")
    }
    pub fn rect(_id: &str) -> Result<(i32, i32, i32, i32), CmdError> {
        unsupported("element")
    }
    pub fn at(_x: i32, _y: i32) -> Result<ElementInfo, CmdError> {
        unsupported("element")
    }
    pub fn click(_id: &str) -> Result<(), CmdError> {
        unsupported("element")
    }
    pub fn invoke(_id: &str) -> Result<(), CmdError> {
        unsupported("element")
    }
    pub fn check(_id: &str, _on: bool) -> Result<(), CmdError> {
        unsupported("element")
    }
    pub fn select(_id: &str, _item: Option<&str>) -> Result<(), CmdError> {
        unsupported("element")
    }
    pub fn expand(_id: &str, _open: bool) -> Result<(), CmdError> {
        unsupported("element")
    }
    pub fn set_value(_id: &str, _value: f64) -> Result<f64, CmdError> {
        unsupported("element")
    }
    pub fn text_get(_id: &str) -> Result<String, CmdError> {
        unsupported("element")
    }
    pub fn text_set(_id: &str, _text: &str) -> Result<(), CmdError> {
        unsupported("element")
    }
}

pub mod monitor {
    use super::unsupported;
    use crate::output::CmdError;
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct MonitorInfo {
        pub id: String,
        pub x: i32,
        pub y: i32,
        pub primary: bool,
    }

    pub fn list() -> Result<Vec<MonitorInfo>, CmdError> {
        unsupported("monitor")
    }
    pub fn find(_id: &str) -> Result<MonitorInfo, CmdError> {
        unsupported("monitor")
    }
    pub fn set(
        _id: &str,
        _resolution: Option<&str>,
        _refresh_rate: Option<u32>,
        _rotation: Option<u32>,
        _position: Option<&str>,
        _make_primary: bool,
        _enabled: Option<bool>,
    ) -> Result<(), CmdError> {
        unsupported("monitor")
    }
    pub fn power(_id: &str, _on: bool) -> Result<(), CmdError> {
        unsupported("monitor")
    }
}

pub mod screenshot {
    use super::unsupported;
    use crate::output::CmdError;
    use crate::platform::Screenshot;

    pub fn take(
        _monitor: Option<&str>,
        _window: Option<&str>,
        _region: Option<&str>,
        _scale: Option<f32>,
        _cursor: bool,
        _grid: bool,
    ) -> Result<Screenshot, CmdError> {
        unsupported("screenshot")
    }
}

pub mod ocr {
    use super::unsupported;
    use crate::output::CmdError;
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct OcrMatch {
        pub text: String,
    }

    pub fn find(
        _needle: &str,
        _monitor: Option<&str>,
        _window: Option<&str>,
        _region: Option<&str>,
    ) -> Result<OcrMatch, CmdError> {
        unsupported("ocr")
    }
    pub fn read(
        _monitor: Option<&str>,
        _window: Option<&str>,
        _region: Option<&str>,
    ) -> Result<Vec<OcrMatch>, CmdError> {
        unsupported("ocr")
    }
}

pub mod clipboard {
    use super::unsupported;
    use crate::output::CmdError;

    pub fn get_text() -> Result<String, CmdError> {
        unsupported("clipboard")
    }
    pub fn set_text(_text: &str) -> Result<(), CmdError> {
        unsupported("clipboard")
    }
    pub fn get_image(_path: &str) -> Result<(), CmdError> {
        unsupported("clipboard")
    }
    pub fn set_image(_path: &str) -> Result<(), CmdError> {
        unsupported("clipboard")
    }
}

pub mod audio {
    use super::unsupported;
    use crate::output::CmdError;
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct AudioDeviceInfo {
        pub id: String,
    }

    pub fn get_volume(_app: Option<&str>) -> Result<u32, CmdError> {
        unsupported("volume")
    }
    pub fn set_volume(_app: Option<&str>, _level: u32) -> Result<(), CmdError> {
        unsupported("volume")
    }
    pub fn set_mute(_app: Option<&str>, _mute: bool) -> Result<(), CmdError> {
        unsupported("volume")
    }
    pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, CmdError> {
        unsupported("audio")
    }
    pub fn set_default_device(_device_id: &str) -> Result<(), CmdError> {
        unsupported("audio")
    }
    pub fn set_enabled(_device_id: &str, _enabled: bool) -> Result<(), CmdError> {
        unsupported("audio")
    }
}

pub mod brightness {
    use super::unsupported;
    use crate::output::CmdError;

    pub fn get(_monitor_id: &str) -> Result<u32, CmdError> {
        unsupported("brightness")
    }
    pub fn set(_monitor_id: &str, _level: u32) -> Result<(), CmdError> {
        unsupported("brightness")
    }
}

pub mod media {
    use super::unsupported;
    use crate::output::CmdError;
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct NowPlaying {
        pub title: String,
    }

    pub fn play() -> Result<(), CmdError> {
        unsupported("media")
    }
    pub fn pause() -> Result<(), CmdError> {
        unsupported("media")
    }
    pub fn next() -> Result<(), CmdError> {
        unsupported("media")
    }
    pub fn previous() -> Result<(), CmdError> {
        unsupported("media")
    }
    pub fn now_playing() -> Result<NowPlaying, CmdError> {
        unsupported("media")
    }
}

pub mod notify {
    use super::unsupported;
    use crate::output::CmdError;

    pub fn send(_title: &str, _body: &str) -> Result<(), CmdError> {
        unsupported("notify")
    }
}

pub mod system {
    use super::unsupported;
    use crate::output::CmdError;

    pub fn spawn(_program: &str, _args: &[String]) -> Result<u32, CmdError> {
        unsupported("launch")
    }
    pub fn open(_target: &str) -> Result<(), CmdError> {
        unsupported("open")
    }
    pub fn lock() -> Result<(), CmdError> {
        unsupported("system")
    }
    pub fn sleep() -> Result<(), CmdError> {
        unsupported("system")
    }
    pub fn status() -> Result<(bool, Option<u64>), CmdError> {
        unsupported("system")
    }
    pub fn keep_awake_for(_duration_ms: u64) -> Result<(), CmdError> {
        unsupported("system")
    }
}

pub mod setup {
    use super::unsupported;
    use crate::output::CmdError;

    pub fn run() -> Result<serde_json::Value, CmdError> {
        unsupported("setup")
    }
}

pub mod menu {
    use super::unsupported;
    use super::window::WindowHandle;
    use crate::output::CmdError;

    pub fn select(_window: WindowHandle, _path: &[&str]) -> Result<(), CmdError> {
        unsupported("menu")
    }
}

pub mod input {
    use super::unsupported;
    use crate::output::CmdError;

    pub struct Watch;

    impl Watch {
        pub fn take(&self) -> bool {
            false
        }
    }

    pub fn watch() -> Result<Watch, CmdError> {
        unsupported("--stop-on-user-input")
    }
}

pub fn guide() -> String {
    GUIDE.to_string()
}
