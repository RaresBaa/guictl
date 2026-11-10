//! `guictl setup` on Windows: nothing needs a one-time permission, so the
//! only job is putting the folder that holds guictl.exe on the user's PATH,
//! so a downloaded binary can be run as plain `guictl` from then on.

use crate::output::CmdError;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};

pub fn run() -> Result<Value, CmdError> {
    let exe = std::env::current_exe()
        .map_err(|e| CmdError::os(format!("could not find guictl's own path: {e}")))?;
    let dir = exe
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| CmdError::os("guictl.exe has no parent folder"))?;

    let (user_path, kind) = read_user_path()?;
    let already = std::env::split_paths(&user_path).any(|p| same_dir(&p, &dir))
        || std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|p| same_dir(&p, &dir)));

    if !already {
        let mut entries: Vec<PathBuf> = std::env::split_paths(&user_path)
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        entries.push(dir.clone());
        let joined = std::env::join_paths(entries)
            .map_err(|e| CmdError::os(format!("could not build the new PATH: {e}")))?;
        write_user_path(&joined, kind)?;
        broadcast_environment_change();
    }

    Ok(json!({
        "ok": true,
        "path": {
            "dir": dir.display().to_string(),
            "added": !already,
        },
        "note": if already {
            "guictl's folder is already on PATH"
        } else {
            "added guictl's folder to the user PATH; terminals and agents started from now on can run `guictl` directly, ones already open need a restart"
        },
    }))
}

/// Compares two folders the way Windows does: case-insensitive, ignoring a
/// trailing backslash.
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// Closes the key on every return path.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open_environment() -> Result<Key, CmdError> {
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Environment"),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(CmdError::os(format!(
            "could not open HKCU\\Environment: error {}",
            status.0
        )));
    }
    Ok(Key(key))
}

/// The user's own PATH, unexpanded, and its registry type. A missing value
/// is an empty PATH; a new one is written as REG_EXPAND_SZ like Windows
/// itself does.
fn read_user_path() -> Result<(OsString, REG_VALUE_TYPE), CmdError> {
    let key = open_environment()?;
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(key.0, w!("Path"), None, Some(&mut kind), None, Some(&mut size))
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok((OsString::new(), REG_EXPAND_SZ));
    }
    if status != ERROR_SUCCESS {
        return Err(CmdError::os(format!("could not read the user PATH: error {}", status.0)));
    }
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return Err(CmdError::os("the user PATH in the registry is not a string"));
    }

    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            w!("Path"),
            None,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(CmdError::os(format!("could not read the user PATH: error {}", status.0)));
    }
    buf.truncate((size as usize) / 2);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    Ok((OsString::from_wide(&buf), kind))
}

fn write_user_path(value: &OsString, kind: REG_VALUE_TYPE) -> Result<(), CmdError> {
    let key = open_environment()?;
    let wide: Vec<u16> = value.encode_wide().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|u| u.to_le_bytes()).collect();
    let status = unsafe { RegSetValueExW(key.0, w!("Path"), 0, kind, Some(&bytes)) };
    if status != ERROR_SUCCESS {
        return Err(CmdError::os(format!("could not write the user PATH: error {}", status.0)));
    }
    Ok(())
}

/// Tells Explorer the environment changed, so programs it starts from now on
/// see the new PATH without logging out.
fn broadcast_environment_change() {
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            2000,
            None,
        );
    }
}
