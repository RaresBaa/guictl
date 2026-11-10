//! GNOME support. GNOME Shell has no interface for outside programs to list
//! or arrange windows, capture the screen without a dialog, or hold the
//! clipboard, so guictl ships a small Shell extension (`gnome-extension/`)
//! that does those things on request, over D-Bus. `guictl setup` installs it.

use super::dbus;
use crate::output::CmdError;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Desktop {
    Kde,
    Gnome,
}

/// The desktop this session runs. Anything that isn't GNOME is taken for
/// Plasma, whose errors then say what's missing.
pub fn desktop() -> Desktop {
    static DESKTOP: OnceLock<Desktop> = OnceLock::new();
    *DESKTOP.get_or_init(|| {
        if let Some(desktop) = std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref().and_then(from_current_desktop) {
            return desktop;
        }
        // Started without the session's environment: see who is on the bus.
        if name_has_owner(SERVICE) { Desktop::Gnome } else { Desktop::Kde }
    })
}

pub fn is_gnome() -> bool {
    desktop() == Desktop::Gnome
}

/// `XDG_CURRENT_DESKTOP` is a colon-separated list, e.g. `ubuntu:GNOME`.
fn from_current_desktop(value: &str) -> Option<Desktop> {
    value.split(':').find_map(|part| match part.trim().to_ascii_lowercase().as_str() {
        "kde" => Some(Desktop::Kde),
        "gnome" | "gnome-classic" => Some(Desktop::Gnome),
        _ => None,
    })
}

fn name_has_owner(name: &str) -> bool {
    dbus::session()
        .and_then(|conn| {
            conn.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(name,),
            )
            .and_then(|reply| reply.body().deserialize::<bool>())
            .map_err(dbus::err)
        })
        .unwrap_or(false)
}

pub const UUID: &str = "guictl@guictl.github.io";
const SERVICE: &str = "org.gnome.Shell";
const PATH: &str = "/io/github/guictl/Shell";
const INTERFACE: &str = "io.github.guictl.Shell";
/// Must match `VERSION` in extension.js.
const VERSION: u64 = 1;

const EXTENSION_JS: &str = include_str!("gnome-extension/extension.js");

fn not_running() -> CmdError {
    CmdError::unsupported(
        "guictl's GNOME Shell extension is not running. Run `guictl setup`, then log out and back in once so GNOME Shell loads it",
    )
}

fn outdated() -> CmdError {
    CmdError::unsupported(
        "the running guictl GNOME Shell extension is from another guictl version. Run `guictl setup`, then log out and back in once so GNOME Shell loads the new one",
    )
}

fn raw_call(method: &str, args: &Value) -> Result<Value, CmdError> {
    let reply: String = dbus::session()?
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "Call", &(method, args.to_string()))
        .and_then(|reply| reply.body().deserialize())
        .map_err(|e| {
            let message = e.to_string();
            if ["UnknownObject", "UnknownMethod", "UnknownInterface", "ServiceUnknown"]
                .iter()
                .any(|m| message.contains(m))
            {
                not_running()
            } else {
                CmdError::os(format!("GNOME Shell did not answer: {message}"))
            }
        })?;
    let reply: Value = serde_json::from_str(&reply)
        .map_err(|e| CmdError::os(format!("guictl's GNOME Shell extension returned invalid JSON: {e}")))?;
    if reply["ok"].as_bool() == Some(true) {
        return Ok(reply["value"].clone());
    }
    let message = reply["error"].as_str().unwrap_or("unknown error");
    Err(if let Some(rest) = message.strip_prefix("not_found:") {
        CmdError::not_found(rest.trim())
    } else if let Some(rest) = message.strip_prefix("not_authorized:") {
        CmdError::permission_denied(rest.trim())
    } else {
        CmdError::os(format!("GNOME Shell: {message}"))
    })
}

/// Whether the extension is running, answers this executable and is the
/// version this guictl expects.
fn check() -> Result<(), CmdError> {
    match raw_call("version", &Value::Null)?.as_u64() {
        Some(VERSION) => Ok(()),
        _ => Err(outdated()),
    }
}

pub fn ready() -> bool {
    check().is_ok()
}

/// Calls one of the extension's methods. The first call of a process checks
/// that the extension is the right version, so a stale one fails clearly.
pub fn call(method: &str, args: Value) -> Result<Value, CmdError> {
    static CHECKED: OnceLock<()> = OnceLock::new();
    if CHECKED.get().is_none() {
        check()?;
        let _ = CHECKED.set(());
    }
    raw_call(method, &args)
}

/// A private file in the runtime directory for the shell to write into or
/// read from, removed when dropped.
pub struct TempFile(pub PathBuf);

impl TempFile {
    pub fn new(what: &str) -> TempFile {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let n = COUNT.fetch_add(1, Ordering::SeqCst);
        TempFile(dir.join(format!("guictl-{}-{n}-{what}", std::process::id())))
    }

    pub fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}

pub fn extension_dir() -> PathBuf {
    data_home().join("gnome-shell/extensions").join(UUID)
}

/// The running GNOME Shell's major version, e.g. "50".
fn shell_version() -> Option<String> {
    let reply = dbus::session()
        .ok()?
        .call_method(
            Some(SERVICE),
            "/org/gnome/Shell",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.gnome.Shell", "ShellVersion"),
        )
        .ok()?;
    let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    let version = <&str>::try_from(&*value).ok()?.to_string();
    version.split('.').next().map(str::to_string)
}

fn metadata() -> String {
    // GNOME Shell loads an extension only if it names the shell's version.
    // 45 is the first with the module format extension.js uses.
    let mut versions: Vec<String> = (45..=50).map(|v| v.to_string()).collect();
    if let Some(running) = shell_version()
        && !versions.contains(&running)
    {
        versions.push(running);
    }
    serde_json::json!({
        "uuid": UUID,
        "name": "guictl",
        "description": "Lets the guictl command-line tool list and arrange windows, take screenshots and use the clipboard.",
        "url": "https://github.com/RaresBaa/guictl",
        "shell-version": versions,
    })
    .to_string()
}

fn gsettings(args: &[&str]) -> Result<String, CmdError> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|e| CmdError::unsupported(format!("could not run gsettings: {e}")))?;
    if !output.status.success() {
        return Err(CmdError::os(format!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Parses the strings out of a GVariant `as` the way gsettings prints it,
/// e.g. `['a@b', 'c']` or `@as []`.
fn parse_strv(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\'' && c != '"' {
            continue;
        }
        let mut item = String::new();
        while let Some(d) = chars.next() {
            match d {
                '\\' => item.extend(chars.next()),
                d if d == c => break,
                d => item.push(d),
            }
        }
        out.push(item);
    }
    out
}

fn format_strv(items: &[String]) -> String {
    let quoted: Vec<String> =
        items.iter().map(|s| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))).collect();
    format!("[{}]", quoted.join(", "))
}

/// Adds or removes the extension from one of GNOME Shell's extension lists.
fn set_listed(key: &str, listed: bool) -> Result<(), CmdError> {
    let mut items = parse_strv(&gsettings(&["get", "org.gnome.shell", key])?);
    let present = items.iter().any(|i| i == UUID);
    if present == listed {
        return Ok(());
    }
    if listed {
        items.push(UUID.to_string());
    } else {
        items.retain(|i| i != UUID);
    }
    gsettings(&["set", "org.gnome.shell", key, &format_strv(&items)]).map(|_| ())
}

pub enum InstallState {
    /// Running and answering this executable.
    Active,
    /// Installed and enabled, but GNOME Shell loads new extensions only at
    /// login.
    NeedsLogin,
}

/// Writes the extension, lets this executable use it and enables it.
pub fn install() -> Result<(InstallState, PathBuf), CmdError> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| CmdError::os(format!("could not find guictl's own path: {e}")))?;
    let dir = extension_dir();
    let allowed_path = dir.join("allowed-executables");
    let mut allowed: Vec<String> = std::fs::read_to_string(&allowed_path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let exe = exe.to_string_lossy().into_owned();
    if !allowed.contains(&exe) {
        allowed.push(exe);
    }
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("extension.js"), EXTENSION_JS)?;
        std::fs::write(dir.join("metadata.json"), metadata())?;
        std::fs::write(&allowed_path, allowed.join("\n") + "\n")
    };
    write().map_err(|e| CmdError::os(format!("could not write {}: {e}", dir.display())))?;

    if gsettings(&["get", "org.gnome.shell", "disable-user-extensions"])? == "true" {
        return Err(CmdError::unsupported(
            "GNOME Shell extensions are switched off for this user (the Extensions app's main switch), and guictl needs its own extension. Turn extensions on and run `guictl setup` again",
        ));
    }
    set_listed("disabled-extensions", false)?;
    set_listed("enabled-extensions", true)?;

    // The extension reads its allowed executables on every call, so one
    // already running answers this executable straight away. GNOME Shell
    // loads a new or updated extension only at the next login.
    let state = if check().is_ok() { InstallState::Active } else { InstallState::NeedsLogin };
    Ok((state, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_current_desktop() {
        assert_eq!(from_current_desktop("ubuntu:GNOME"), Some(Desktop::Gnome));
        assert_eq!(from_current_desktop("GNOME-Classic:GNOME"), Some(Desktop::Gnome));
        assert_eq!(from_current_desktop("KDE"), Some(Desktop::Kde));
        assert_eq!(from_current_desktop("XFCE"), None);
        assert_eq!(from_current_desktop(""), None);
    }

    #[test]
    fn string_lists_round_trip() {
        assert_eq!(parse_strv("@as []"), Vec::<String>::new());
        let items = parse_strv("['ding@rastersoft.com', 'it\\'s']");
        assert_eq!(items, vec!["ding@rastersoft.com".to_string(), "it's".to_string()]);
        assert_eq!(parse_strv(&format_strv(&items)), items);
    }
}
