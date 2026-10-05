use super::dbus;
use crate::output::CmdError;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

pub const SERVICE: &str = "org.kde.KWin";

const REPLY_PATH: &str = "/guictl";

/// `__BODY__` runs as a function body inside KWin; whatever it returns comes
/// back as JSON. A thrown string starting with `not_found:` becomes a
/// not-found error, anything else a generic one.
const WRAPPER: &str = r#"(function () {
    var out;
    try {
        out = { ok: true, value: (function () {
__BODY__
        })() };
    } catch (e) {
        out = { ok: false, error: String(e) };
    }
    callDBus("__DEST__", "/guictl", "io.github.guictl.Reply", "Result", JSON.stringify(out));
})();
"#;

struct Reply {
    tx: Mutex<mpsc::Sender<String>>,
}

#[zbus::interface(name = "io.github.guictl.Reply")]
impl Reply {
    fn result(&self, payload: String) {
        let _ = self.tx.lock().unwrap().send(payload);
    }
}

/// Quotes a string as a JavaScript literal.
pub fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// Runs `body` inside KWin's scripting engine and returns its result. KWin
/// has no window-management protocol for outside clients on Wayland, and
/// scripts can't return values directly, so the script calls back into a
/// D-Bus object this process exposes for the duration of the call - the same
/// approach `kdotool` takes.
pub fn run(body: &str) -> Result<serde_json::Value, CmdError> {
    let conn = dbus::session()?;
    let (tx, rx) = mpsc::channel();
    conn.object_server()
        .at(REPLY_PATH, Reply { tx: Mutex::new(tx) })
        .map_err(dbus::err)?;
    let dest = conn
        .unique_name()
        .map(|n| n.to_string())
        .ok_or_else(|| CmdError::os("the D-Bus connection has no unique name"))?;

    let name = format!("guictl-{}", std::process::id());
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("{name}.js"));
    let source = WRAPPER.replace("__DEST__", &dest).replace("__BODY__", body);
    std::fs::write(&path, source)
        .map_err(|e| CmdError::os(format!("could not write the KWin script: {e}")))?;

    let payload = run_file(&conn, &path.to_string_lossy(), &name, &rx);
    let _ = conn.call_method(
        Some(SERVICE),
        "/Scripting",
        Some("org.kde.kwin.Scripting"),
        "unloadScript",
        &(name.as_str(),),
    );
    let _ = std::fs::remove_file(&path);

    let reply: serde_json::Value = serde_json::from_str(&payload?)
        .map_err(|e| CmdError::os(format!("KWin script returned invalid JSON: {e}")))?;
    if reply["ok"].as_bool() == Some(true) {
        return Ok(reply["value"].clone());
    }
    let message = reply["error"].as_str().unwrap_or("unknown KWin script error");
    Err(match message.strip_prefix("not_found:") {
        Some(rest) => CmdError::not_found(rest.trim()),
        None => CmdError::os(format!("KWin script failed: {message}")),
    })
}

fn run_file(
    conn: &zbus::blocking::Connection,
    path: &str,
    name: &str,
    rx: &mpsc::Receiver<String>,
) -> Result<String, CmdError> {
    let id: i32 = conn
        .call_method(
            Some(SERVICE),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "loadScript",
            &(path, name),
        )
        .and_then(|reply| reply.body().deserialize())
        .map_err(|e| {
            CmdError::unsupported(format!(
                "could not reach KWin's scripting interface (is this a KDE Plasma session?): {e}"
            ))
        })?;
    conn.call_method(
        Some(SERVICE),
        format!("/Scripting/Script{id}").as_str(),
        Some("org.kde.kwin.Script"),
        "run",
        &(),
    )
    .map_err(dbus::err)?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| CmdError::os("KWin script did not reply (it may have a syntax error)"))
}
