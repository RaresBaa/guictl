use super::kwin::{self, js_str};
use super::window::WindowHandle;
use crate::output::CmdError;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// KWin's UUID for the virtual desktop.
    pub id: String,
    pub name: String,
    /// 1-based position, as shown in the pager.
    pub index: u32,
    pub current: bool,
}

const PRELUDE: &str = r#"
function findDesktop(id) {
    var all = workspace.desktops;
    for (var i = 0; i < all.length; i++) {
        if (all[i].id === id || String(all[i].x11DesktopNumber) === id) return all[i];
    }
    throw "not_found: no workspace with id " + id;
}
"#;

const DESCRIBE: &str = r#"
function describe(d) {
    return { id: d.id, name: d.name, index: d.x11DesktopNumber, current: d === workspace.currentDesktop };
}
"#;

fn parse<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, CmdError> {
    serde_json::from_value(value).map_err(|e| CmdError::os(format!("unexpected workspace data from KWin: {e}")))
}

pub fn list() -> Result<Vec<WorkspaceInfo>, CmdError> {
    parse(kwin::run(&format!("{DESCRIBE}\nreturn workspace.desktops.map(describe);"))?)
}

/// KWin moves its windows to another desktop.
pub fn remove(id: &str) -> Result<(), CmdError> {
    if list()?.len() == 1 {
        return Err(CmdError::invalid_args("can't remove the only workspace"));
    }
    kwin::run(&format!("{PRELUDE}\nworkspace.removeDesktop(findDesktop({}));", js_str(id))).map(|_| ())
}

/// Given an empty name, KWin calls the desktop "Desktop N" itself.
pub fn create(name: Option<&str>) -> Result<WorkspaceInfo, CmdError> {
    parse(kwin::run(&format!(
        "{DESCRIBE}\nvar at = workspace.desktops.length;\nworkspace.createDesktop(at, {});\nreturn describe(workspace.desktops[at]);",
        js_str(name.unwrap_or(""))
    ))?)
}

/// `id` is a workspace id from `list`, or its 1-based index.
pub fn switch(id: &str) -> Result<(), CmdError> {
    kwin::run(&format!("{PRELUDE}\nworkspace.currentDesktop = findDesktop({});", js_str(id))).map(|_| ())
}

pub fn send_window(window: WindowHandle, id: &str) -> Result<(), CmdError> {
    super::window::with_window(
        window,
        &format!("{PRELUDE}\nw.desktops = [findDesktop({})];", js_str(id)),
    )
    .map(|_| ())
}
