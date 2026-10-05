use super::dbus;
use crate::output::CmdError;
use std::process::{Command, Stdio};

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
