use super::dbus;
use crate::output::CmdError;
use std::collections::HashMap;
use zbus::zvariant::Value;

pub fn send(title: &str, body: &str) -> Result<(), CmdError> {
    let actions: Vec<&str> = Vec::new();
    let hints: HashMap<&str, Value> = HashMap::new();
    // app name, replaces id, icon, summary, body, actions, hints, timeout (-1 = server default)
    dbus::session()?
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("guictl", 0u32, "", title, body, actions, hints, -1i32),
        )
        .map(|_| ())
        .map_err(dbus::err)
}
