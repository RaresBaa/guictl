//! `--elevate`: runs one command in a copy of guictl started as administrator,
//! which brings up the UAC prompt, and passes on what it printed and its exit
//! code. An elevated process can't inherit the console of a normal one, so its
//! output comes back through two temp files.

use crate::cli::Command;
use crate::output::CmdError;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::IntoRawHandle;
use std::path::{Path, PathBuf};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// FILE_SHARE_READ | FILE_SHARE_WRITE, and not FILE_SHARE_DELETE: while the
/// normal copy holds its files open, nothing can swap them for a link that
/// would make the elevated copy overwrite something else.
const SHARE_READ_WRITE: u32 = 1 | 2;

fn output_paths(base: &str) -> (PathBuf, PathBuf) {
    (PathBuf::from(format!("{base}.out")), PathBuf::from(format!("{base}.err")))
}

/// Quotes an argument the way the C runtime and `CommandLineToArgvW` read it.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

fn create_output(path: &Path) -> Result<File, CmdError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(SHARE_READ_WRITE)
        .open(path)
        .map_err(|e| CmdError::os(format!("could not create {}: {e}", path.display())))
}

/// Starts the elevated copy and waits for it. Asking only returns once the
/// person has answered the prompt.
fn run_elevated(exe: &Path, parameters: &str, cwd: &Path) -> Result<i32, CmdError> {
    let (file, parameters, directory) = (HSTRING::from(exe.as_os_str()), HSTRING::from(parameters), HSTRING::from(cwd.as_os_str()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        // ShellExecuteEx may hand the work to a shell extension, which needs COM.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        ShellExecuteExW(&mut info).map_err(|e| {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                CmdError::permission_denied("the person declined the administrator prompt or didn't answer it, so the command did not run")
            } else {
                CmdError::os(format!("could not start guictl as administrator: {e}"))
            }
        })?;
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0u32;
        let read = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        read.map_err(|e| CmdError::os(e.to_string()))?;
        Ok(code as i32)
    }
}

/// Runs this same command line elevated and returns its exit code. The copy
/// tells itself apart by `--elevated-output`, so `--elevate` can stay in the
/// arguments, however clap would have read them.
pub fn relaunch(command: &Command) -> Result<i32, CmdError> {
    if matches!(command, Command::Batch(args) if args.file.is_none()) {
        return Err(CmdError::invalid_args(
            "--elevate can't pass stdin on to the elevated command; use `batch --file`",
        ));
    }
    let exe = std::env::current_exe().map_err(|e| CmdError::os(e.to_string()))?;
    let cwd = std::env::current_dir().map_err(|e| CmdError::os(e.to_string()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let base = std::env::temp_dir().join(format!("guictl-elevate-{}-{stamp}", std::process::id()));
    let base = base.to_string_lossy().into_owned();
    let (out_path, err_path) = output_paths(&base);
    let mut out = create_output(&out_path)?;
    let mut err = match create_output(&err_path) {
        Ok(err) => err,
        Err(e) => {
            drop(out);
            let _ = std::fs::remove_file(&out_path);
            return Err(e);
        }
    };

    let mut parameters = vec![
        "--elevated-output".to_string(),
        quote_arg(&base),
        "--elevated-cwd".to_string(),
        quote_arg(&cwd.to_string_lossy()),
    ];
    parameters.extend(std::env::args_os().skip(1).map(|a| quote_arg(&a.to_string_lossy())));
    let result = run_elevated(&exe, &parameters.join(" "), &cwd);

    if result.is_ok() {
        let _ = std::io::copy(&mut out, &mut std::io::stdout());
        let _ = std::io::copy(&mut err, &mut std::io::stderr());
    }
    drop((out, err));
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    result
}

fn redirect(which: STD_HANDLE, path: &Path) -> Result<(), CmdError> {
    let file = OpenOptions::new()
        .write(true)
        .share_mode(SHARE_READ_WRITE)
        .open(path)
        .map_err(|e| CmdError::os(format!("could not open {}: {e}", path.display())))?;
    unsafe { SetStdHandle(which, HANDLE(file.into_raw_handle())) }.map_err(|e| CmdError::os(e.to_string()))
}

/// In the elevated copy: sends its output to the files the normal copy is
/// waiting on, and goes to the directory it started from, since Windows starts
/// an elevated program in System32.
pub fn attach(base: &str, cwd: Option<&str>) -> Result<(), CmdError> {
    let (out, err) = output_paths(base);
    redirect(STD_OUTPUT_HANDLE, &out)?;
    redirect(STD_ERROR_HANDLE, &err)?;
    if let Some(cwd) = cwd {
        std::env::set_current_dir(cwd).map_err(|e| {
            CmdError::os(format!("could not go to {cwd} as administrator: {e}; a mapped network drive isn't visible there"))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn arguments_survive_the_command_line() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("two words"), "\"two words\"");
        assert_eq!(quote_arg(r"C:\dir\file"), r"C:\dir\file");
        assert_eq!(quote_arg(r"C:\my dir\"), r#""C:\my dir\\""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }
}
