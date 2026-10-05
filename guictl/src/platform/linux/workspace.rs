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

pub fn list() -> Result<Vec<WorkspaceInfo>, CmdError> {
    let value = kwin::run(
        "return workspace.desktops.map(function (d) {\n    return { id: d.id, name: d.name, index: d.x11DesktopNumber, current: d === workspace.currentDesktop };\n});",
    )?;
    serde_json::from_value(value)
        .map_err(|e| CmdError::os(format!("unexpected workspace data from KWin: {e}")))
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
