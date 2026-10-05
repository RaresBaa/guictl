//! Input injection through `org.freedesktop.portal.RemoteDesktop`. The first
//! use shows Plasma's consent dialog; the restore token it hands back is
//! saved so later commands start a session without asking again. That
//! consent is a deliberate security boundary, not something to route around.

use super::dbus;
use crate::output::CmdError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{DynamicType, ObjectPath, OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";

const DEVICE_KEYBOARD: u32 = 1;
const DEVICE_POINTER: u32 = 2;
/// Keep the permission until the user revokes it.
const PERSIST_UNTIL_REVOKED: u32 = 2;

/// How long to wait for the user to answer the consent dialog.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(120);

type Results = HashMap<String, OwnedValue>;
/// A portal request's `Response` signal: the request's path, the response
/// code and the results.
type Response = (String, u32, Results);

pub struct Session {
    conn: Connection,
    handle: OwnedObjectPath,
}

/// One session per process, so the steps of a `batch` share it. The session
/// ends when the process exits, and KWin releases anything still held then -
/// a key or button can only stay down across steps of one batch.
static SESSION: OnceLock<Session> = OnceLock::new();

pub fn session() -> Result<&'static Session, CmdError> {
    if let Some(session) = SESSION.get() {
        return Ok(session);
    }
    let session = start()?;
    Ok(SESSION.get_or_init(|| session))
}

fn token_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir);
    state.join("guictl").join("remote-desktop-token")
}

fn portal_err(e: zbus::Error) -> CmdError {
    CmdError::unsupported(format!(
        "the RemoteDesktop portal is not available (xdg-desktop-portal and xdg-desktop-portal-kde need to be installed and running): {e}"
    ))
}

/// Calls a portal method that answers through a `Response` signal on a
/// request object, and waits for that signal.
fn request<B: serde::Serialize + DynamicType>(
    conn: &Connection,
    responses: &mpsc::Receiver<Response>,
    method: &str,
    body: &B,
    timeout: Duration,
) -> Result<Results, CmdError> {
    let request_path: OwnedObjectPath = conn
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), method, body)
        .and_then(|reply| reply.body().deserialize())
        .map_err(portal_err)?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let (path, code, results) = responses.recv_timeout(remaining).map_err(|_| {
            CmdError::permission_denied(
                "timed out waiting for the remote-control request to be approved; accept Plasma's \"Remote control\" dialog and run the command again",
            )
        })?;
        if path != request_path.as_str() {
            continue;
        }
        return match code {
            0 => Ok(results),
            1 => Err(CmdError::permission_denied("the remote-control request was denied")),
            _ => Err(CmdError::os(format!("the RemoteDesktop portal's {method} request failed"))),
        };
    }
}

fn start() -> Result<Session, CmdError> {
    let conn = dbus::session()?;

    // Subscribe before making any request so no response can be missed.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|b| b.member("Response"))
        .map_err(dbus::err)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, &conn, Some(16)).map_err(dbus::err)?;
    let (tx, responses) = mpsc::channel::<Response>();
    std::thread::spawn(move || {
        for message in signals.flatten() {
            let path = message.header().path().map(|p| p.to_string()).unwrap_or_default();
            if let Ok((code, results)) = message.body().deserialize::<(u32, Results)>() {
                if tx.send((path, code, results)).is_err() {
                    break;
                }
            }
        }
    });

    let token = |name: &str| Value::from(format!("guictl_{}_{name}", std::process::id()));
    let quick = Duration::from_secs(10);

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", token("create"));
    options.insert("session_handle_token", token("session"));
    let results = request(&conn, &responses, "CreateSession", &(options,), quick)?;
    let handle = results
        .get("session_handle")
        .and_then(|v| <&str>::try_from(&**v).ok())
        .and_then(|s| ObjectPath::try_from(s.to_string()).ok())
        .map(OwnedObjectPath::from)
        .ok_or_else(|| CmdError::os("the RemoteDesktop portal returned no session handle"))?;

    let saved_token = std::fs::read_to_string(token_path()).ok();
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", token("devices"));
    options.insert("types", Value::from(DEVICE_KEYBOARD | DEVICE_POINTER));
    options.insert("persist_mode", Value::from(PERSIST_UNTIL_REVOKED));
    if let Some(saved) = saved_token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        options.insert("restore_token", Value::from(saved));
    }
    request(&conn, &responses, "SelectDevices", &(&handle, options), quick)?;

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", token("start"));
    let results = request(&conn, &responses, "Start", &(&handle, "", options), CONSENT_TIMEOUT)?;

    // Restore tokens are single-use: every start hands back the next one.
    if let Some(new_token) = results.get("restore_token").and_then(|v| <&str>::try_from(&**v).ok()) {
        let path = token_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&path, new_token).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }

    Ok(Session { conn, handle })
}

impl Session {
    fn notify<B: serde::Serialize + DynamicType>(&self, method: &str, body: &B) -> Result<(), CmdError> {
        self.conn
            .call_method(Some(SERVICE), PATH, Some(INTERFACE), method, body)
            .map(|_| ())
            .map_err(|e| CmdError::os(format!("the RemoteDesktop portal rejected {method}: {e}")))
    }

    fn no_options() -> HashMap<&'static str, Value<'static>> {
        HashMap::new()
    }

    pub fn pointer_motion(&self, dx: f64, dy: f64) -> Result<(), CmdError> {
        self.notify("NotifyPointerMotion", &(&self.handle, Self::no_options(), dx, dy))
    }

    /// `button` is a Linux evdev button code.
    pub fn pointer_button(&self, button: i32, pressed: bool) -> Result<(), CmdError> {
        self.notify("NotifyPointerButton", &(&self.handle, Self::no_options(), button, pressed as u32))
    }

    /// `axis` is 0 for vertical, 1 for horizontal; positive steps scroll down/right.
    pub fn pointer_axis_discrete(&self, axis: u32, steps: i32) -> Result<(), CmdError> {
        self.notify("NotifyPointerAxisDiscrete", &(&self.handle, Self::no_options(), axis, steps))
    }

    /// `keycode` is a Linux evdev key code.
    pub fn keyboard_keycode(&self, keycode: i32, pressed: bool) -> Result<(), CmdError> {
        self.notify("NotifyKeyboardKeycode", &(&self.handle, Self::no_options(), keycode, pressed as u32))
    }

    pub fn keyboard_keysym(&self, keysym: i32, pressed: bool) -> Result<(), CmdError> {
        self.notify("NotifyKeyboardKeysym", &(&self.handle, Self::no_options(), keysym, pressed as u32))
    }
}
