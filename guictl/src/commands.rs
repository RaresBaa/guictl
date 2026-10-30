use crate::cli::*;
use crate::output::{CmdError, CmdResult};
use crate::platform::os;
use clap::Parser;
use serde_json::json;

pub fn dispatch(command: Command) -> CmdResult {
    match command {
        Command::Mouse(cmd) => mouse(cmd),
        Command::Keyboard(cmd) => keyboard(cmd),
        Command::Window(cmd) => window(cmd),
        Command::Screenshot(args) => screenshot(args),
        Command::Element(cmd) => element(cmd),
        Command::Volume(cmd) => volume(cmd),
        Command::Audio(cmd) => audio(cmd),
        Command::Brightness(cmd) => brightness(cmd),
        Command::Clipboard(cmd) => clipboard(cmd),
        Command::Monitor(cmd) => monitor(cmd),
        Command::Open(args) => {
            os::system::open(&args.target)?;
            Ok(json!({ "ok": true }))
        }
        Command::Launch(args) => launch(args),
        Command::System(cmd) => system(cmd),
        Command::Media(cmd) => media(cmd),
        Command::Notify(args) => {
            os::notify::send(&args.title, &args.body)?;
            Ok(json!({ "ok": true }))
        }
        Command::Capabilities => capabilities(),
        Command::Setup => os::setup::run(),
        // Only reached from `batch`; `main` prints the guide as plain text.
        Command::Guide => Ok(json!({ "guide": crate::guide::text() })),
        Command::Wait(cmd) => wait(cmd),
        Command::Batch(args) => batch(args),
        Command::Menu(args) => {
            let path: Vec<&str> = args.path.split('>').map(str::trim).filter(|s| !s.is_empty()).collect();
            if path.is_empty() {
                return Err(CmdError::invalid_args("the menu path is empty; give names joined by >, e.g. \"File>Save As\""));
            }
            os::menu::select(resolve(&args.selector)?, &path)?;
            Ok(json!({ "ok": true }))
        }
        Command::Ocr(args) => {
            let (monitor, window, region) = (args.monitor.as_deref(), args.window.as_deref(), args.region.as_deref());
            match args.text {
                Some(text) => Ok(serde_json::to_value(os::ocr::find(&text, monitor, window, region)?).unwrap()),
                None => Ok(serde_json::to_value(os::ocr::read(monitor, window, region)?).unwrap()),
            }
        }
        Command::Workspace(WorkspaceCmd::List) => {
            let workspaces = os::workspace::list()?;
            Ok(serde_json::to_value(workspaces).unwrap())
        }
        Command::Workspace(WorkspaceCmd::Switch { id }) => {
            os::workspace::switch(&id)?;
            Ok(json!({ "ok": true }))
        }
    }
}

/// Where coordinates given with `--window` / `--monitor` are measured from:
/// that window's or monitor's top-left corner, else the desktop's.
fn mouse_origin(window: &WindowSelector, monitor: Option<&str>) -> Result<(i32, i32), CmdError> {
    let has_window = window.window.is_some()
        || window.active
        || window.title.is_some()
        || window.app.is_some()
        || window.pid.is_some();
    match (has_window, monitor) {
        (true, Some(_)) => Err(CmdError::invalid_args("specify a window or --monitor, not both")),
        (true, None) => {
            let (x, y, _, _) = os::window::rect(resolve(window)?)?;
            Ok((x, y))
        }
        (false, Some(id)) => {
            let m = os::monitor::find(id)?;
            Ok((m.x, m.y))
        }
        (false, None) => Ok((0, 0)),
    }
}

/// Moves the pointer to a point, spread over `duration` ms when given so
/// that applications see it travel rather than jump.
fn glide_to(x: i32, y: i32, duration: Option<u64>) -> Result<(), CmdError> {
    const STEP_MS: u64 = 20;
    let steps = duration.map_or(1, |ms| (ms / STEP_MS).clamp(1, 100)) as i32;
    if steps > 1 {
        let (sx, sy) = os::mouse::position()?;
        let start = std::time::Instant::now();
        for i in 1..steps {
            os::mouse::move_to(sx + (x - sx) * i / steps, sy + (y - sy) * i / steps)?;
            let due = std::time::Duration::from_millis(STEP_MS * i as u64);
            std::thread::sleep(due.saturating_sub(start.elapsed()));
        }
    }
    os::mouse::move_to(x, y)
}

fn mouse(cmd: MouseCmd) -> CmdResult {
    match cmd {
        MouseCmd::Position => {
            let (x, y) = os::mouse::position()?;
            Ok(json!({ "x": x, "y": y }))
        }
        MouseCmd::Move { x, y, window, monitor, duration } => {
            let (ox, oy) = mouse_origin(&window, monitor.as_deref())?;
            glide_to(ox + x, oy + y, duration)?;
            Ok(json!({ "x": ox + x, "y": oy + y }))
        }
        MouseCmd::Click { x, y, button, double, window, monitor } => {
            let (ox, oy) = mouse_origin(&window, monitor.as_deref())?;
            let at = match (x, y) {
                (Some(x), Some(y)) => Some((ox + x, oy + y)),
                _ => None,
            };
            os::mouse::click(button, double, at)?;
            Ok(json!({ "ok": true }))
        }
        MouseCmd::Press { x, y, button, window, monitor } => {
            let (ox, oy) = mouse_origin(&window, monitor.as_deref())?;
            if let (Some(x), Some(y)) = (x, y) {
                os::mouse::move_to(ox + x, oy + y)?;
            }
            os::mouse::press(button)?;
            Ok(json!({ "ok": true }))
        }
        MouseCmd::Release { button } => {
            os::mouse::release(button)?;
            Ok(json!({ "ok": true }))
        }
        MouseCmd::Drag { x1, y1, x2, y2, button, duration, hold, window, monitor } => {
            // Older apps poll the mouse or act on the last move they saw
            // before the release, so an instant press-jump-release can come
            // out short or not register. Pausing around each stage and
            // travelling in steps gives them a move event at every point.
            const SETTLE: std::time::Duration = std::time::Duration::from_millis(50);
            let (ox, oy) = mouse_origin(&window, monitor.as_deref())?;
            os::mouse::move_to(ox + x1, oy + y1)?;
            std::thread::sleep(SETTLE);
            os::mouse::press(button)?;
            std::thread::sleep(SETTLE);
            let moved = glide_to(ox + x2, oy + y2, Some(duration.unwrap_or(300)));
            if moved.is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(hold.unwrap_or(100)));
            }
            // Let go even if the move failed, or the button stays down.
            os::mouse::release(button)?;
            moved?;
            Ok(json!({ "ok": true }))
        }
        MouseCmd::Scroll { amount, direction } => {
            os::mouse::scroll(amount, direction)?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn keyboard(cmd: KeyboardCmd) -> CmdResult {
    match cmd {
        KeyboardCmd::Type { text } => {
            os::keyboard::type_text(&text)?;
            Ok(json!({ "ok": true }))
        }
        KeyboardCmd::Key { key, repeat } => {
            os::keyboard::key(&key, repeat)?;
            Ok(json!({ "ok": true }))
        }
        KeyboardCmd::Hotkey { combo } => {
            os::keyboard::hotkey(&combo)?;
            Ok(json!({ "ok": true }))
        }
        KeyboardCmd::Hold { key } => {
            os::keyboard::hold(&key)?;
            Ok(json!({ "ok": true }))
        }
        KeyboardCmd::Release { key } => {
            os::keyboard::release(&key)?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn resolve(selector: &WindowSelector) -> Result<os::window::WindowHandle, CmdError> {
    os::window::resolve(
        selector.window.as_deref(),
        selector.active,
        selector.title.as_deref(),
        selector.app.as_deref(),
        selector.pid,
    )
}

fn window(cmd: WindowCmd) -> CmdResult {
    match cmd {
        WindowCmd::List { app } => {
            let windows = os::window::list(app.as_deref())?;
            Ok(serde_json::to_value(windows).unwrap())
        }
        WindowCmd::Active => {
            let w = os::window::active()?;
            Ok(serde_json::to_value(w).unwrap())
        }
        WindowCmd::Focus { selector } => {
            let hwnd = resolve(&selector)?;
            os::window::focus(hwnd)?;
            Ok(json!({ "ok": true }))
        }
        WindowCmd::Move { selector, x, y } => {
            let hwnd = resolve(&selector)?;
            os::window::move_window(hwnd, x, y)?;
            Ok(json!({ "ok": true }))
        }
        WindowCmd::Resize { selector, w, h } => {
            let hwnd = resolve(&selector)?;
            os::window::resize_window(hwnd, w, h)?;
            Ok(json!({ "ok": true }))
        }
        WindowCmd::State { selector, state } => {
            let hwnd = resolve(&selector)?;
            os::window::set_state(hwnd, state)?;
            Ok(json!({ "ok": true }))
        }
        WindowCmd::Close { selector } => {
            let hwnd = resolve(&selector)?;
            let ids = |windows: &[os::window::WindowInfo]| -> Vec<String> {
                windows.iter().map(|w| w.id.clone()).collect()
            };
            let before = ids(&os::window::list(None)?);
            os::window::close(hwnd)?;
            // Closing is only a request: an app with unsaved work answers
            // with a dialog instead. Watch briefly for which it was, so the
            // caller isn't told "ok" about a window that is still there.
            let start = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let closed = os::window::rect(hwnd).is_err();
                let opened: Vec<_> = os::window::list(None)?
                    .into_iter()
                    .filter(|w| !before.contains(&w.id))
                    .collect();
                if closed || !opened.is_empty() || start.elapsed().as_millis() >= 1500 {
                    return Ok(json!({ "ok": true, "closed": closed, "new_windows": opened }));
                }
            }
        }
        WindowCmd::Kill { selector } => {
            let hwnd = resolve(&selector)?;
            let pid = os::window::kill(hwnd)?;
            let start = std::time::Instant::now();
            while os::window::rect(hwnd).is_ok() && start.elapsed().as_millis() < 2000 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(json!({ "ok": true, "pid": pid, "closed": os::window::rect(hwnd).is_err() }))
        }
        WindowCmd::Pin { selector, state } => {
            let hwnd = resolve(&selector)?;
            os::window::pin(hwnd, matches!(state, OnOff::On))?;
            Ok(json!({ "ok": true }))
        }
        WindowCmd::Send { selector, monitor, workspace } => {
            if let Some(workspace_id) = workspace {
                if monitor.is_some() {
                    return Err(CmdError::invalid_args("specify one of --monitor or --workspace, not both"));
                }
                let hwnd = resolve(&selector)?;
                os::workspace::send_window(hwnd, &workspace_id)?;
                return Ok(json!({ "ok": true }));
            }
            let monitor_id = monitor.ok_or_else(|| {
                CmdError::invalid_args("specify one of --monitor or --workspace")
            })?;
            let hwnd = resolve(&selector)?;
            let target = os::monitor::find(&monitor_id)?;
            os::window::move_window(hwnd, target.x + 40, target.y + 40)?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn screenshot(args: ScreenshotArgs) -> CmdResult {
    let element_region = match args.element.as_deref() {
        Some(id) => {
            let (x, y, w, h) = os::element::rect(id)?;
            if w <= 0 || h <= 0 {
                return Err(CmdError::not_found("element is not showing on screen"));
            }
            Some(format!("{x},{y},{w},{h}"))
        }
        None => None,
    };
    let (width, height, bytes, origin) = os::screenshot::take(
        args.monitor.as_deref(),
        args.window.as_deref(),
        element_region.as_deref().or(args.region.as_deref()),
        args.scale,
        args.cursor,
        args.grid,
    )?;
    match args.output.as_deref() {
        Some("-") => {
            use std::io::Write;
            std::io::stdout().write_all(&bytes).map_err(|e| CmdError::os(e.to_string()))?;
            std::process::exit(0);
        }
        Some(path) => {
            std::fs::write(path, &bytes).map_err(|e| CmdError::os(e.to_string()))?;
            let mut result = json!({ "path": path, "width": width, "height": height });
            if let Some((x, y)) = origin {
                result["x"] = json!(x);
                result["y"] = json!(y);
            }
            Ok(result)
        }
        None => Err(CmdError::invalid_args("screenshot needs --output <path>|-")),
    }
}

fn element(cmd: ElementCmd) -> CmdResult {
    match cmd {
        ElementCmd::List { selector, role } => {
            let hwnd = resolve(&selector)?;
            let elements = os::element::list(hwnd, role.as_deref())?;
            Ok(serde_json::to_value(elements).unwrap())
        }
        ElementCmd::Find { selector, name, role, all, index } => {
            let hwnd = resolve(&selector)?;
            if !all && index.is_none() {
                let el = os::element::find(hwnd, name.as_deref(), role.as_deref())?;
                return Ok(serde_json::to_value(el).unwrap());
            }
            let found = os::element::find_all(hwnd, name.as_deref(), role.as_deref())?;
            let Some(index) = index else {
                return Ok(serde_json::to_value(found).unwrap());
            };
            let count = found.len();
            let el = found.into_iter().nth(index).ok_or_else(|| {
                CmdError::not_found(format!("--index {index} is past the end: {count} elements match"))
            })?;
            Ok(serde_json::to_value(el).unwrap())
        }
        ElementCmd::At { x, y } => Ok(serde_json::to_value(os::element::at(x, y)?).unwrap()),
        ElementCmd::Click { id } => {
            os::element::click(&id)?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Invoke { id } => {
            os::element::invoke(&id)?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Check { id, state } => {
            os::element::check(&id, matches!(state, OnOff::On))?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Select { id, item } => {
            os::element::select(&id, item.as_deref())?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Expand { id } => {
            os::element::expand(&id, true)?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Collapse { id } => {
            os::element::expand(&id, false)?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Value { id, value } => {
            let value = os::element::set_value(&id, value)?;
            Ok(json!({ "ok": true, "value": value }))
        }
        ElementCmd::Text(ElementTextCmd::Get { id }) => {
            let text = os::element::text_get(&id)?;
            Ok(json!({ "text": text }))
        }
        ElementCmd::Text(ElementTextCmd::Set { id, text }) => {
            os::element::text_set(&id, &text)?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn volume(cmd: VolumeCmd) -> CmdResult {
    match cmd {
        VolumeCmd::Get { app } => {
            let level = os::audio::get_volume(app.as_deref())?;
            Ok(json!({ "level": level }))
        }
        VolumeCmd::Set { app, level } => {
            os::audio::set_volume(app.as_deref(), level)?;
            Ok(json!({ "ok": true }))
        }
        VolumeCmd::Mute { app, state } => {
            os::audio::set_mute(app.as_deref(), matches!(state, OnOff::On))?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn audio(cmd: AudioCmd) -> CmdResult {
    match cmd {
        AudioCmd::Device(AudioDeviceCmd::List) => {
            let devices = os::audio::list_devices()?;
            Ok(serde_json::to_value(devices).unwrap())
        }
        AudioCmd::Device(AudioDeviceCmd::SetDefault { id }) => {
            os::audio::set_default_device(&id)?;
            Ok(json!({ "ok": true }))
        }
        AudioCmd::Device(AudioDeviceCmd::Enable { id }) => {
            os::audio::set_enabled(&id, true)?;
            Ok(json!({ "ok": true }))
        }
        AudioCmd::Device(AudioDeviceCmd::Disable { id }) => {
            os::audio::set_enabled(&id, false)?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn brightness(cmd: BrightnessCmd) -> CmdResult {
    match cmd {
        BrightnessCmd::Get { monitor } => {
            let level = os::brightness::get(&monitor)?;
            Ok(json!({ "level": level }))
        }
        BrightnessCmd::Set { monitor, level } => {
            os::brightness::set(&monitor, level)?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn clipboard(cmd: ClipboardCmd) -> CmdResult {
    match cmd {
        ClipboardCmd::Get { image: false, .. } => {
            let text = os::clipboard::get_text()?;
            Ok(json!({ "text": text }))
        }
        ClipboardCmd::Get { image: true, output: Some(output) } => {
            os::clipboard::get_image(&output)?;
            Ok(json!({ "path": output }))
        }
        ClipboardCmd::Get { image: true, output: None } => {
            Err(CmdError::invalid_args("--image needs --output <path>"))
        }
        ClipboardCmd::Set { text: Some(text), image: None } => {
            os::clipboard::set_text(&text)?;
            Ok(json!({ "ok": true }))
        }
        ClipboardCmd::Set { text: None, image: Some(path) } => {
            os::clipboard::set_image(&path)?;
            Ok(json!({ "ok": true }))
        }
        ClipboardCmd::Set { .. } => Err(CmdError::invalid_args(
            "specify either text or --image, not both",
        )),
    }
}

fn resolve_monitor(selector: &MonitorSelector) -> Result<String, CmdError> {
    if let Some(id) = &selector.monitor {
        return Ok(id.clone());
    }
    if selector.primary {
        let monitors = os::monitor::list()?;
        return monitors
            .into_iter()
            .find(|m| m.primary)
            .map(|m| m.id)
            .ok_or_else(|| CmdError::not_found("no primary monitor found"));
    }
    Err(CmdError::invalid_args("specify one of --monitor or --primary"))
}

fn monitor(cmd: MonitorCmd) -> CmdResult {
    match cmd {
        MonitorCmd::List => {
            let monitors = os::monitor::list()?;
            Ok(serde_json::to_value(monitors).unwrap())
        }
        MonitorCmd::Set {
            selector,
            resolution,
            refresh_rate,
            rotation,
            position,
            make_primary,
            enabled,
        } => {
            let id = resolve_monitor(&selector)?;
            os::monitor::set(
                &id,
                resolution.as_deref(),
                refresh_rate,
                rotation,
                position.as_deref(),
                make_primary,
                enabled.map(|s| matches!(s, OnOff::On)),
            )?;
            Ok(json!({ "ok": true }))
        }
        MonitorCmd::Power { id, state } => {
            os::monitor::power(&id, matches!(state, OnOff::On))?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn window_ids() -> Result<Vec<String>, CmdError> {
    Ok(os::window::list(None)?.into_iter().map(|w| w.id).collect())
}

fn launch(args: LaunchArgs) -> CmdResult {
    let before = if args.wait_window { window_ids()? } else { Vec::new() };
    let pid = os::system::spawn(&args.program, &args.args)?;
    if !args.wait_window {
        return Ok(json!({ "pid": pid }));
    }
    // A launcher may hand over to another process, so a new window of the
    // same program counts as well as one of the started process.
    let stem = |name: &str| {
        std::path::Path::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    let program = stem(&args.program);
    let ours = |w: &os::window::WindowInfo| {
        !before.contains(&w.id) && (w.pid == pid || stem(&w.app) == program)
    };
    let timeout = std::time::Duration::from_millis(args.timeout.unwrap_or(5000));
    let start = std::time::Instant::now();
    let first = loop {
        if let Ok(windows) = os::window::list(None) {
            if let Some(w) = windows.into_iter().find(|w| ours(w)) {
                break w;
            }
        }
        if start.elapsed() >= timeout {
            return Err(CmdError::timeout(
                "timed out waiting for the launched app's window",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    };
    // Programs often open more than one window at start: a splash screen,
    // a tip of the day, a licence or update prompt. Keep looking until no
    // new one has turned up for a while, so the caller sees all of them.
    const QUIET: std::time::Duration = std::time::Duration::from_millis(1000);
    let mut windows = vec![];
    let mut last_change = std::time::Instant::now();
    while last_change.elapsed() < QUIET && start.elapsed() < timeout + QUIET {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let now: Vec<_> = os::window::list(None)?.into_iter().filter(|w| ours(w)).collect();
        let ids = |list: &[os::window::WindowInfo]| list.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
        if ids(&now) != ids(&windows) {
            last_change = std::time::Instant::now();
        }
        windows = now;
    }
    Ok(json!({ "pid": pid, "window": first, "windows": windows }))
}

fn system(cmd: SystemCmd) -> CmdResult {
    match cmd {
        SystemCmd::Lock => {
            os::system::lock()?;
            Ok(json!({ "ok": true }))
        }
        SystemCmd::Sleep => {
            os::system::sleep()?;
            Ok(json!({ "ok": true }))
        }
        SystemCmd::Status => {
            let (locked, idle_ms) = os::system::status()?;
            let mut out = json!({ "locked": locked });
            if let Some(idle_ms) = idle_ms {
                out["idle_ms"] = json!(idle_ms);
            }
            Ok(out)
        }
        SystemCmd::KeepAwake { state: OnOff::On, duration: Some(duration) } => {
            os::system::keep_awake_for(duration)?;
            Ok(json!({ "ok": true }))
        }
        SystemCmd::KeepAwake { state: OnOff::On, duration: None } => Err(CmdError::invalid_args(
            "keep-awake on needs --duration: each guictl command is one-off, so the awake state can only last as long as this call blocks for",
        )),
        SystemCmd::KeepAwake { state: OnOff::Off, .. } => Err(CmdError::invalid_args(
            "keep-awake off is a no-op: a previous 'on' call already released the state when it returned",
        )),
    }
}

fn media(cmd: MediaCmd) -> CmdResult {
    match cmd {
        MediaCmd::Play => {
            os::media::play()?;
            Ok(json!({ "ok": true }))
        }
        MediaCmd::Pause => {
            os::media::pause()?;
            Ok(json!({ "ok": true }))
        }
        MediaCmd::Next => {
            os::media::next()?;
            Ok(json!({ "ok": true }))
        }
        MediaCmd::Previous => {
            os::media::previous()?;
            Ok(json!({ "ok": true }))
        }
        MediaCmd::NowPlaying => {
            let info = os::media::now_playing()?;
            Ok(serde_json::to_value(info).unwrap())
        }
    }
}

fn wait(cmd: WaitCmd) -> CmdResult {
    match cmd {
        WaitCmd::Window { window, title, app, pid, new, gone, timeout } => {
            if window.is_none() && title.is_none() && app.is_none() && pid.is_none() && !new {
                return Err(CmdError::invalid_args("specify --window, --title, --app, --pid or --new"));
            }
            let before = match (new, take_batch_baseline()) {
                (false, _) => Vec::new(),
                (true, Some(baseline)) => baseline,
                (true, None) => window_ids()?,
            };
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let start = std::time::Instant::now();
            loop {
                if let Ok(windows) = os::window::list(app.as_deref()) {
                    let found = windows.into_iter().find(|w| {
                        window.as_deref().is_none_or(|id| w.id == id)
                            && title.as_deref().is_none_or(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
                            && pid.is_none_or(|p| w.pid == p)
                            && !before.contains(&w.id)
                    });
                    match found {
                        Some(w) if !gone => return Ok(serde_json::to_value(w).unwrap()),
                        None if gone => return Ok(json!({ "ok": true })),
                        _ => {}
                    }
                }
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout(if gone {
                        "timed out waiting for the window to go away"
                    } else {
                        "timed out waiting for a matching window"
                    }));
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
        WaitCmd::Element { selector, name, role, gone, enabled, value, timeout } => {
            if name.is_none() && role.is_none() {
                return Err(CmdError::invalid_args("specify --name or --role"));
            }
            let hwnd = match resolve(&selector) {
                // No window, no element.
                Err(e) if gone && e.code == "not_found" => return Ok(json!({ "ok": true })),
                other => other?,
            };
            let wanted_value = value.as_deref().map(str::to_lowercase);
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let start = std::time::Instant::now();
            loop {
                match os::element::find(hwnd, name.as_deref(), role.as_deref()) {
                    Ok(el) if !gone => {
                        let value_ok = wanted_value.as_ref().is_none_or(|wanted| {
                            el.value.as_ref().is_some_and(|v| v.to_lowercase().contains(wanted))
                        });
                        if value_ok && !(enabled && el.disabled) {
                            return Ok(serde_json::to_value(el).unwrap());
                        }
                    }
                    Ok(_) => {}
                    // Waiting won't change who may look inside the window.
                    Err(e) if e.code == "permission_denied" => return Err(e),
                    Err(e) if gone && (e.code == "not_found" || os::window::rect(hwnd).is_err()) => {
                        return Ok(json!({ "ok": true }));
                    }
                    Err(_) => {}
                }
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout(if gone {
                        "timed out waiting for the element to go away"
                    } else if enabled || value.is_some() {
                        "timed out waiting for a matching element in that state"
                    } else {
                        "timed out waiting for a matching element"
                    }));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        WaitCmd::Idle { monitor, window, region, stable, timeout } => {
            use std::hash::{Hash, Hasher};
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let stable = std::time::Duration::from_millis(stable.unwrap_or(600));
            let start = std::time::Instant::now();
            // Idle means the picture stayed the same for `stable`, judged
            // from captures at least three apart so that a single repeat
            // can't let a slow animation through.
            let mut last_hash: Option<u64> = None;
            let mut unchanged_since = std::time::Instant::now();
            let mut identical = 1;
            // A region is usually small, so it's compared at full size;
            // downscaling could blur a small change away.
            let scale = if region.is_some() { None } else { Some(0.25) };
            loop {
                let (_, _, bytes, _) = os::screenshot::take(
                    monitor.as_deref(),
                    window.as_deref(),
                    region.as_deref(),
                    scale,
                    false,
                    false,
                )?;
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut hasher);
                let hash = hasher.finish();
                if last_hash == Some(hash) {
                    identical += 1;
                } else {
                    identical = 1;
                    unchanged_since = std::time::Instant::now();
                }
                if identical >= 3 && unchanged_since.elapsed() >= stable {
                    return Ok(json!({ "ok": true }));
                }
                last_hash = Some(hash);
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout("screen did not settle before timeout"));
                }
                std::thread::sleep((stable / 3).clamp(
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_millis(300),
                ));
            }
        }
        WaitCmd::Change { monitor, window, region, timeout } => {
            let picture = || picture_hash(monitor.as_deref(), window.as_deref(), region.as_deref());
            let before = match take_batch_picture() {
                Some(before) => before,
                None => picture()?,
            };
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let start = std::time::Instant::now();
            loop {
                if picture()? != before {
                    return Ok(json!({ "ok": true }));
                }
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout("screen did not change before timeout"));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// Compared at full size: downscaling could blur a small change away.
fn picture_hash(monitor: Option<&str>, window: Option<&str>, region: Option<&str>) -> Result<u64, CmdError> {
    use std::hash::{Hash, Hasher};
    let (_, _, bytes, _) = os::screenshot::take(monitor, window, region, None, false, false)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

/// Batch file format: a JSON array of argv arrays, one per command, e.g.
/// `[["mouse", "click", "100", "200"], ["keyboard", "type", "hi"]]`. Each
/// step runs even if an earlier one failed; the response reports every
/// step's outcome rather than aborting on the first error, since a partial
/// batch with no record of what happened is worse than a full report.
fn batch(args: BatchArgs) -> CmdResult {
    let watch = if args.stop_on_user_input { Some(os::input::watch()?) } else { None };
    if args.lines {
        return batch_lines(watch.as_ref());
    }
    let raw = match args.file {
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|e| CmdError::invalid_args(format!("could not read {path}: {e}")))?,
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| CmdError::invalid_args(format!("could not read stdin: {e}")))?;
            buf
        }
    };
    let steps: Vec<Vec<String>> = serde_json::from_str(&raw)
        .map_err(|e| CmdError::invalid_args(format!("batch input must be a JSON array of argv arrays: {e}")))?;
    Ok(run_steps(steps, watch.as_ref()))
}

fn parse_step(argv: &[String]) -> Result<Cli, CmdError> {
    let cli = Cli::try_parse_from(std::iter::once("guictl").chain(argv.iter().map(String::as_str)))
        .map_err(|e| CmdError::invalid_args(e.to_string()))?;
    #[cfg(all(windows, not(guictl_stub)))]
    if cli.elevate {
        return Err(CmdError::invalid_args("--elevate goes on the whole batch, not on one of its steps"));
    }
    Ok(cli)
}

fn run_steps(steps: Vec<Vec<String>>, watch: Option<&os::input::Watch>) -> serde_json::Value {
    let mut results = Vec::with_capacity(steps.len());
    let mut all_ok = true;
    let mut interrupted = false;
    for (i, argv) in steps.iter().enumerate() {
        if !interrupted && watch.is_some_and(|w| w.take()) {
            interrupted = true;
        }
        let outcome = if interrupted {
            Err(CmdError::interrupted("skipped: someone used the mouse or keyboard"))
        } else {
            // A `wait window --new` or `wait change` right after this step
            // must not miss what the step does before the wait gets to look.
            match steps.get(i + 1).and_then(|next| parse_step(next).ok()).map(|next| next.command) {
                Some(Command::Wait(WaitCmd::Window { new: true, .. })) => set_batch_baseline(window_ids().ok()),
                Some(Command::Wait(WaitCmd::Change { monitor, window, region, .. })) => {
                    *BATCH_PICTURE.lock().unwrap() =
                        picture_hash(monitor.as_deref(), window.as_deref(), region.as_deref()).ok();
                }
                _ => {}
            }
            parse_step(argv).and_then(|cli| dispatch(cli.command))
        };
        all_ok &= outcome.is_ok();
        results.push(json!({ "args": argv, "outcome": outcome_json(outcome) }));
    }
    let mut out = json!({ "results": results, "all_ok": all_ok });
    if interrupted {
        out["interrupted"] = json!(true);
    }
    out
}

fn outcome_json(outcome: CmdResult) -> serde_json::Value {
    match outcome {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": { "code": err.code, "message": err.message } }),
    }
}

/// `batch --lines`: one request per input line, one answer per output line,
/// for a caller that wants to keep a single guictl running. A line is either
/// one argument list, answered like a batch step's outcome, or a list of
/// them, answered like a whole `batch`.
fn batch_lines(watch: Option<&os::input::Watch>) -> CmdResult {
    use std::io::{BufRead, Write};
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Request {
        One(Vec<String>),
        Many(Vec<Vec<String>>),
    }
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| CmdError::os(format!("could not read stdin: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Request>(&line) {
            Ok(Request::One(argv)) => {
                let mut out = run_steps(vec![argv], watch);
                out["results"][0]["outcome"].take()
            }
            Ok(Request::Many(steps)) => run_steps(steps, watch),
            Err(e) => outcome_json(Err(CmdError::invalid_args(format!(
                "each line must be a JSON argument list or a list of them: {e}"
            )))),
        };
        let mut out = stdout.lock();
        writeln!(out, "{answer}").and_then(|_| out.flush()).map_err(|e| CmdError::os(e.to_string()))?;
    }
    // Every answer has been printed already; there's nothing left to report.
    std::process::exit(0);
}

static BATCH_BASELINE: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

fn set_batch_baseline(ids: Option<Vec<String>>) {
    *BATCH_BASELINE.lock().unwrap() = ids;
}

fn take_batch_baseline() -> Option<Vec<String>> {
    BATCH_BASELINE.lock().unwrap().take()
}

static BATCH_PICTURE: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);

fn take_batch_picture() -> Option<u64> {
    BATCH_PICTURE.lock().unwrap().take()
}

fn capabilities() -> CmdResult {
    let unavailable = os::unavailable();
    let implemented: Vec<&str> =
        os::IMPLEMENTED.iter().copied().filter(|name| !unavailable.contains(name)).collect();
    Ok(json!({
        "platform": std::env::consts::OS,
        "implemented": implemented,
        "not_implemented": os::NOT_IMPLEMENTED,
        "unavailable": unavailable,
    }))
}
