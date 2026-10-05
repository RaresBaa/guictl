use serde::Serialize;

pub type CmdResult = Result<serde_json::Value, CmdError>;

#[derive(Debug)]
pub struct CmdError {
    pub exit_code: i32,
    pub code: &'static str,
    pub message: String,
}

impl CmdError {
    pub fn invalid_args(message: impl Into<String>) -> Self {
        Self { exit_code: 2, code: "invalid_args", message: message.into() }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self { exit_code: 3, code: "not_found", message: message.into() }
    }

    #[allow(dead_code)] // only the Linux backend raises it so far
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self { exit_code: 4, code: "permission_denied", message: message.into() }
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self { exit_code: 10, code: "unsupported", message: message.into() }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self { exit_code: 5, code: "timeout", message: message.into() }
    }

    pub fn os(message: impl Into<String>) -> Self {
        Self { exit_code: 1, code: "os_error", message: message.into() }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorFields<'a>,
}

#[derive(Serialize)]
struct ErrorFields<'a> {
    code: &'a str,
    message: &'a str,
}

pub fn print_ok(value: serde_json::Value, pretty: bool) {
    let out = if pretty {
        serde_json::to_string_pretty(&value).unwrap()
    } else {
        serde_json::to_string(&value).unwrap()
    };
    println!("{out}");
}

pub fn print_err(err: &CmdError, pretty: bool) {
    let body = ErrorBody { error: ErrorFields { code: err.code, message: &err.message } };
    let out = if pretty {
        serde_json::to_string_pretty(&body).unwrap()
    } else {
        serde_json::to_string(&body).unwrap()
    };
    eprintln!("{out}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        let all = [
            (CmdError::os("x"), 1, "os_error"),
            (CmdError::invalid_args("x"), 2, "invalid_args"),
            (CmdError::not_found("x"), 3, "not_found"),
            (CmdError::permission_denied("x"), 4, "permission_denied"),
            (CmdError::timeout("x"), 5, "timeout"),
            (CmdError::unsupported("x"), 10, "unsupported"),
        ];
        for (err, exit_code, code) in all {
            assert_eq!((err.exit_code, err.code), (exit_code, code));
        }
    }
}
