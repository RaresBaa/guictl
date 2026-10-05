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
        Command::Ocr(args) => {
            let m = os::ocr::find(&args.text, args.monitor.as_deref(), args.window.as_deref())?;
            Ok(serde_json::to_value(m).unwrap())
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
    let has_window =
        window.window.is_some() || window.active || window.title.is_some() || window.app.is_some();
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
        MouseCmd::Drag { x1, y1, x2, y2, button, duration, window, monitor } => {
            let (ox, oy) = mouse_origin(&window, monitor.as_deref())?;
            os::mouse::move_to(ox + x1, oy + y1)?;
            os::mouse::press(button)?;
            let moved = glide_to(ox + x2, oy + y2, duration);
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
    let (width, height, bytes) = os::screenshot::take(
        args.monitor.as_deref(),
        args.window.as_deref(),
        args.region.as_deref(),
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
            Ok(json!({ "path": path, "width": width, "height": height }))
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
        ElementCmd::Find { selector, name, role } => {
            let hwnd = resolve(&selector)?;
            let el = os::element::find(hwnd, name.as_deref(), role.as_deref())?;
            Ok(serde_json::to_value(el).unwrap())
        }
        ElementCmd::Click { id } => {
            os::element::click(&id)?;
            Ok(json!({ "ok": true }))
        }
        ElementCmd::Invoke { id } => {
            os::element::invoke(&id)?;
            Ok(json!({ "ok": true }))
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

fn launch(args: LaunchArgs) -> CmdResult {
    let child = std::process::Command::new(&args.program)
        .args(&args.args)
        .spawn()
        .map_err(|e| CmdError::os(e.to_string()))?;
    let pid = child.id();
    if !args.wait_window {
        return Ok(json!({ "pid": pid }));
    }
    let timeout = std::time::Duration::from_millis(args.timeout.unwrap_or(5000));
    let start = std::time::Instant::now();
    loop {
        if let Ok(windows) = os::window::list(None) {
            if let Some(w) = windows.into_iter().find(|w| w.pid == pid) {
                return Ok(json!({ "pid": pid, "window": w }));
            }
        }
        if start.elapsed() >= timeout {
            return Err(CmdError::timeout(
                "timed out waiting for the launched app's window",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
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
        WaitCmd::Window { title, app, timeout } => {
            if title.is_none() && app.is_none() {
                return Err(CmdError::invalid_args("specify --title or --app"));
            }
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let start = std::time::Instant::now();
            loop {
                if let Ok(windows) = os::window::list(None) {
                    if let Some(w) = windows.into_iter().find(|w| {
                        title.as_deref().is_none_or(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
                            && app.as_deref().is_none_or(|a| w.app.eq_ignore_ascii_case(a))
                    }) {
                        return Ok(serde_json::to_value(w).unwrap());
                    }
                }
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout("timed out waiting for a matching window"));
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
        WaitCmd::Element { selector, name, role, timeout } => {
            if name.is_none() && role.is_none() {
                return Err(CmdError::invalid_args("specify --name or --role"));
            }
            let hwnd = resolve(&selector)?;
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let start = std::time::Instant::now();
            loop {
                if let Ok(el) = os::element::find(hwnd, name.as_deref(), role.as_deref()) {
                    return Ok(serde_json::to_value(el).unwrap());
                }
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout("timed out waiting for a matching element"));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        WaitCmd::Idle { monitor, window, region, timeout } => {
            use std::hash::{Hash, Hasher};
            let timeout = std::time::Duration::from_millis(timeout.unwrap_or(5000));
            let start = std::time::Instant::now();
            // Idle means this many captures in a row came out identical. Two
            // would let a slow animation through whenever both happened to
            // catch the same frame.
            const STABLE_CAPTURES: u32 = 3;
            let mut last_hash: Option<u64> = None;
            let mut identical = 1;
            // A region is usually small, so it's compared at full size;
            // downscaling could blur a small change away.
            let scale = if region.is_some() { None } else { Some(0.25) };
            loop {
                let (_, _, bytes) = os::screenshot::take(
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
                identical = if last_hash == Some(hash) { identical + 1 } else { 1 };
                if identical >= STABLE_CAPTURES {
                    return Ok(json!({ "ok": true }));
                }
                last_hash = Some(hash);
                if start.elapsed() >= timeout {
                    return Err(CmdError::timeout("screen did not settle before timeout"));
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }
}

/// Batch file format: a JSON array of argv arrays, one per command, e.g.
/// `[["mouse", "click", "100", "200"], ["keyboard", "type", "hi"]]`. Each
/// step runs even if an earlier one failed; the response reports every
/// step's outcome rather than aborting on the first error, since a partial
/// batch with no record of what happened is worse than a full report.
fn batch(args: BatchArgs) -> CmdResult {
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

    let mut results = Vec::with_capacity(steps.len());
    let mut all_ok = true;
    for argv in steps {
        let mut full_argv = vec!["guictl".to_string()];
        full_argv.extend(argv.iter().cloned());
        let outcome = match Cli::try_parse_from(&full_argv) {
            Ok(cli) => match dispatch(cli.command) {
                Ok(value) => json!({ "ok": true, "result": value }),
                Err(err) => {
                    all_ok = false;
                    json!({ "ok": false, "error": { "code": err.code, "message": err.message } })
                }
            },
            Err(e) => {
                all_ok = false;
                json!({ "ok": false, "error": { "code": "invalid_args", "message": e.to_string() } })
            }
        };
        results.push(json!({ "args": argv, "outcome": outcome }));
    }

    Ok(json!({ "results": results, "all_ok": all_ok }))
}

fn capabilities() -> CmdResult {
    Ok(json!({
        "platform": std::env::consts::OS,
        "implemented": os::IMPLEMENTED,
        "not_implemented": os::NOT_IMPLEMENTED,
    }))
}
