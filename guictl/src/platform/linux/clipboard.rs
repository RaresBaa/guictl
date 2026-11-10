use super::{dbus, gnome};
use crate::output::CmdError;
use std::io::Write;
use std::process::{Command, Stdio};

const SERVICE: &str = "org.kde.klipper";
const PATH: &str = "/klipper";
const INTERFACE: &str = "org.kde.klipper.klipper";

fn klipper_err(e: zbus::Error) -> CmdError {
    CmdError::unsupported(format!(
        "could not reach Klipper, Plasma's clipboard service (is the clipboard widget disabled?): {e}"
    ))
}

/// Goes through Klipper rather than the Wayland clipboard directly: there,
/// clipboard contents belong to the process that set them and vanish
/// when it exits, which a one-off command can't work with. Klipper owns the
/// contents on our behalf.
///
/// On GNOME the shell owns them, through guictl's extension.
pub fn get_text() -> Result<String, CmdError> {
    if gnome::is_gnome() {
        return gnome::call("clipboard", serde_json::json!({ "action": "get_text" }))
            .map(|v| v.as_str().unwrap_or_default().to_string());
    }
    dbus::session()?
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "getClipboardContents", &())
        .and_then(|reply| reply.body().deserialize())
        .map_err(klipper_err)
}

pub fn set_text(text: &str) -> Result<(), CmdError> {
    if gnome::is_gnome() {
        return gnome::call("clipboard", serde_json::json!({ "action": "set_text", "text": text })).map(|_| ());
    }
    dbus::session()?
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "setClipboardContents", &(text,))
        .map(|_| ())
        .map_err(klipper_err)
}

/// Runs one of the `wl-clipboard` tools. Klipper's D-Bus interface only
/// carries text, and a one-off process can't own the clipboard itself (see
/// above); `wl-copy` solves that by leaving a small process behind that
/// serves the data until something else is copied.
fn wl_clipboard(tool: &str) -> Command {
    Command::new(tool)
}

fn missing(tool: &str, e: std::io::Error) -> CmdError {
    CmdError::unsupported(format!(
        "could not run {tool}, which clipboard images need (install wl-clipboard): {e}"
    ))
}

/// Saves the clipboard's image to `path`, in the format its extension asks for.
pub fn get_image(path: &str) -> Result<(), CmdError> {
    if gnome::is_gnome() {
        let file = gnome::TempFile::new("clipboard");
        gnome::call("clipboard", serde_json::json!({ "action": "get_image", "path": file.path() }))?;
        let data = std::fs::read(&file.0).map_err(|e| CmdError::os(format!("could not read the clipboard image: {e}")))?;
        return image::load_from_memory(&data)
            .map_err(|e| CmdError::os(format!("could not decode the clipboard image: {e}")))?
            .save(path)
            .map_err(|e| CmdError::os(e.to_string()));
    }
    let types = wl_clipboard("wl-paste")
        .arg("--list-types")
        .output()
        .map_err(|e| missing("wl-paste", e))?;
    let types = String::from_utf8_lossy(&types.stdout);
    // PNG when the source offers it, otherwise any format `image` can decode.
    let mime = ["image/png", "image/jpeg", "image/bmp", "image/gif"]
        .into_iter()
        .find(|wanted| types.lines().any(|t| t.trim() == *wanted))
        .ok_or_else(|| CmdError::not_found("the clipboard does not hold an image"))?;
    let data = wl_clipboard("wl-paste")
        .args(["--type", mime])
        .output()
        .map_err(|e| missing("wl-paste", e))?;
    if !data.status.success() {
        return Err(CmdError::os(format!(
            "wl-paste failed: {}",
            String::from_utf8_lossy(&data.stderr).trim()
        )));
    }
    image::load_from_memory(&data.stdout)
        .map_err(|e| CmdError::os(format!("could not decode the clipboard image: {e}")))?
        .save(path)
        .map_err(|e| CmdError::os(e.to_string()))
}

/// Loads an image file and puts it on the clipboard as PNG.
pub fn set_image(path: &str) -> Result<(), CmdError> {
    let img = image::open(path)
        .map_err(|e| CmdError::invalid_args(format!("could not read image: {e}")))?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| CmdError::os(e.to_string()))?;

    if gnome::is_gnome() {
        let file = gnome::TempFile::new("clipboard.png");
        std::fs::write(&file.0, &png).map_err(|e| CmdError::os(format!("could not write {}: {e}", file.path())))?;
        return gnome::call("clipboard", serde_json::json!({ "action": "set_image", "path": file.path() })).map(|_| ());
    }

    let mut child = wl_clipboard("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| missing("wl-copy", e))?;
    let written = child.stdin.take().map(|mut stdin| stdin.write_all(&png));
    // wl-copy exits once it has forked its background server.
    let status = child.wait().map_err(|e| CmdError::os(e.to_string()))?;
    match written {
        Some(Ok(())) if status.success() => Ok(()),
        _ => Err(CmdError::os("wl-copy could not take the image")),
    }
}
