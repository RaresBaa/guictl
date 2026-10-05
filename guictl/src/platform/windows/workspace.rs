use super::window::WindowHandle;
use crate::output::CmdError;
use serde::Serialize;

#[derive(Serialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub index: u32,
    pub current: bool,
}

// Virtual desktops need an undocumented COM interface on Windows whose GUID
// and vtable change from one build to the next.
fn unsupported() -> CmdError {
    CmdError::unsupported("workspaces are not implemented on Windows")
}

pub fn list() -> Result<Vec<WorkspaceInfo>, CmdError> {
    Err(unsupported())
}

pub fn switch(_id: &str) -> Result<(), CmdError> {
    Err(unsupported())
}

pub fn send_window(_window: WindowHandle, _id: &str) -> Result<(), CmdError> {
    Err(unsupported())
}
