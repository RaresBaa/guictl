//! `guictl setup`: the two one-time permissions Plasma asks for, done in
//! one go so nobody has to write a .desktop file by hand, and the
//! "Remote Control Started" popup switched off.

use super::{dbus, portal, screenshot};
use crate::output::CmdError;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// KWin finds the calling binary's .desktop file through Plasma's
/// application cache, which only covers the standard `applications`
/// directories - so it has to live here, not next to the executable.
fn desktop_file_path() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    data.join("applications").join("guictl.desktop")
}

fn authorize_screenshots() -> Result<PathBuf, CmdError> {
    // KWin compares against the resolved path of the running binary.
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| CmdError::os(format!("could not find guictl's own path: {e}")))?;
    let path = desktop_file_path();
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=guictl\nExec={}\nNoDisplay=true\nX-KDE-DBUS-Restricted-Interfaces={}\n",
        exe.display(),
        screenshot::INTERFACE,
    );
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &contents)
    };
    write().map_err(|e| CmdError::os(format!("could not write {}: {e}", path.display())))?;

    let rebuilt = Command::new("kbuildsycoca6").output().map_err(|e| {
        CmdError::unsupported(format!(
            "could not run kbuildsycoca6 to refresh Plasma's application cache (is this a KDE Plasma session?): {e}"
        ))
    })?;
    if !rebuilt.status.success() {
        return Err(CmdError::os(format!(
            "kbuildsycoca6 failed: {}",
            String::from_utf8_lossy(&rebuilt.stderr).trim()
        )));
    }

    // KWin notices the refreshed cache a moment later.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match screenshot::take(None, None, Some("0,0,1,1"), None, false, false) {
            Ok(_) => return Ok(path),
            Err(e) if e.code == "permission_denied" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Every guictl input command is its own portal session, and the KDE portal
/// announces each new session with a popup, so an agent at work stacks them
/// up. This switches that event's popup off in the portal's notification
/// settings, which is what unticking it in System Settings does. It is one
/// setting for the portal, so it applies to every program, not just guictl.
fn silence_remote_control_popup() -> Result<(), CmdError> {
    const APP: &str = "xdg-desktop-portal-kde";
    let written = Command::new("kwriteconfig6")
        .args(["--file", &format!("{APP}.notifyrc"), "--group", "Event/remotedesktopstarted", "--key", "Action", ""])
        .output()
        .map_err(|e| CmdError::unsupported(format!("could not run kwriteconfig6: {e}")))?;
    if !written.status.success() {
        return Err(CmdError::os(format!(
            "kwriteconfig6 failed: {}",
            String::from_utf8_lossy(&written.stderr).trim()
        )));
    }
    // The running portal reads the file only at start; System Settings sends
    // this signal to make it read it again.
    dbus::session()?
        .emit_signal(None::<&str>, "/Config", "org.kde.knotification", "reparseConfiguration", &(APP,))
        .map_err(dbus::err)
}

pub fn run() -> Result<Value, CmdError> {
    let desktop_file = authorize_screenshots()?;
    silence_remote_control_popup()?;
    // Starting an input session is what makes Plasma show its consent
    // dialog; with a stored token it just succeeds.
    portal::session()?;
    Ok(json!({
        "ok": true,
        "screenshot": "authorized",
        "input": "authorized",
        "remote_control_popup": "off",
        "note": "Plasma's \"Remote Control Started\" popup is now off for every program, not just guictl. Turn it back on in System Settings > Notifications > Applications > KDE Portal Integration > Configure Events.",
        "desktop_file": desktop_file,
    }))
}
