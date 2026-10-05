//! `guictl setup`: the two one-time permissions Plasma asks for, done in
//! one go so nobody has to write a .desktop file by hand.

use super::{portal, screenshot};
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

pub fn run() -> Result<Value, CmdError> {
    let desktop_file = authorize_screenshots()?;
    // Starting an input session is what makes Plasma show its consent
    // dialog; with a stored token it just succeeds.
    portal::session()?;
    Ok(json!({
        "ok": true,
        "screenshot": "authorized",
        "input": "authorized",
        "desktop_file": desktop_file,
    }))
}
