use crate::output::CmdError;
use serde_json::{json, Value};

/// Nothing on Windows needs a one-time permission; the command exists so
/// the same first step works on every platform.
pub fn run() -> Result<Value, CmdError> {
    Ok(json!({ "ok": true }))
}
