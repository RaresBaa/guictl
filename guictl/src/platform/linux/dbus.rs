use crate::output::CmdError;
use zbus::blocking::Connection;

pub fn session() -> Result<Connection, CmdError> {
    Connection::session()
        .map_err(|e| CmdError::os(format!("could not connect to the D-Bus session bus: {e}")))
}

pub fn system() -> Result<Connection, CmdError> {
    Connection::system()
        .map_err(|e| CmdError::os(format!("could not connect to the D-Bus system bus: {e}")))
}

pub fn err(e: zbus::Error) -> CmdError {
    CmdError::os(e.to_string())
}
