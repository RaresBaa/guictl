use super::dbus;
use crate::output::CmdError;
use std::process::{Command, Stdio};

/// Starts a program that outlives guictl, without guictl's stdout: a caller
/// reading that through a pipe would otherwise wait until the program exits.
pub fn spawn(program: &str, args: &[String]) -> Result<u32, CmdError> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|child| child.id())
        .map_err(|e| CmdError::os(e.to_string()))
}

pub fn open(target: &str) -> Result<(), CmdError> {
    Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| CmdError::os(format!("could not run xdg-open: {e}")))
}

pub fn lock() -> Result<(), CmdError> {
    dbus::session()?
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "Lock",
            &(),
        )
        .map(|_| ())
        .map_err(dbus::err)
}

pub fn sleep() -> Result<(), CmdError> {
    // `false`: don't prompt for authorization, just fail if it isn't allowed.
    dbus::system()?
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Suspend",
            &(false,),
        )
        .map(|_| ())
        .map_err(dbus::err)
}

/// Fails while the screen is locked: input would go to the lock screen rather
/// than the apps, and typing would land in the password box. If the state
/// can't be read, input goes ahead as it always did.
pub(super) fn check_unlocked() -> Result<(), CmdError> {
    if is_locked().unwrap_or(false) {
        return Err(CmdError::permission_denied(
            "The screen is locked, so mouse and keyboard input would go to the lock screen, not the apps. Tell the person and wait until they unlock it; `system status` shows when",
        ));
    }
    Ok(())
}

fn is_locked() -> Result<bool, CmdError> {
    dbus::session()?
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "GetActive",
            &(),
        )
        .and_then(|reply| reply.body().deserialize())
        .map_err(dbus::err)
}

/// Whether the screen is locked. Wayland doesn't tell a client how long the
/// session has been idle, so the second value is always None here.
pub fn status() -> Result<(bool, Option<u64>), CmdError> {
    Ok((is_locked()?, None))
}

/// Holds the display/system awake for `duration_ms`, then releases it. Because this is a
/// one-off process, the inhibition can only last as long as this call blocks - there
/// is no persistent session to hold it afterward.
pub fn keep_awake_for(duration_ms: u64) -> Result<(), CmdError> {
    let conn = dbus::session()?;
    // Screen-saver inhibition covers the display; PowerDevil also treats it as
    // a suspend inhibitor. Both are dropped when this connection closes, so
    // an interrupted call can't leave the machine stuck awake.
    let cookie: u32 = conn
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "Inhibit",
            &("guictl", "keep-awake requested"),
        )
        .and_then(|reply| reply.body().deserialize())
        .map_err(dbus::err)?;
    std::thread::sleep(std::time::Duration::from_millis(duration_ms));
    conn.call_method(
        Some("org.freedesktop.ScreenSaver"),
        "/ScreenSaver",
        Some("org.freedesktop.ScreenSaver"),
        "UnInhibit",
        &(cookie,),
    )
    .map(|_| ())
    .map_err(dbus::err)
}
