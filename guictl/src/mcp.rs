//! `guictl mcp`: guictl as an MCP server, so an AI client can call its
//! commands as tools instead of running the program once per action.
//!
//! The server speaks MCP's "Streamable HTTP" transport on 127.0.0.1, which is
//! what lets it run in the background, apart from any one client: `start`
//! launches `serve` as a detached copy of this program and returns, `stop`
//! asks it to exit. It answers every request with plain JSON (no event
//! streams), which the transport allows. It's written on the standard
//! library alone: the protocol is small, and guictl should stay one
//! dependency-light binary.
//!
//! Commands run one at a time on a single worker thread. They act on one
//! desktop, so two at once would fight over the mouse and focus, and the
//! Windows COM objects they create belong to the thread that made them.

use crate::cli::{Cli, Command, McpCmd, ScreenshotArgs};
use crate::commands;
use crate::output::{CmdError, CmdResult};
use clap::{CommandFactory, Parser};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn run(cmd: McpCmd) -> CmdResult {
    match cmd {
        McpCmd::Start(args) => start(args.port),
        McpCmd::Stop => stop(),
        McpCmd::Status => status(),
        McpCmd::Serve(args) => serve(args.port),
    }
}

// ---- the running server's record ----

/// Written by `serve` once it listens, so `stop` and `status` can find it.
#[derive(Serialize, Deserialize)]
struct State {
    pid: u32,
    port: u16,
    /// Required by `/shutdown`, so only someone who can read this file (the
    /// same user) can stop the server.
    token: String,
}

fn state_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

fn state_path() -> PathBuf {
    state_dir().join("guictl-mcp.json")
}

fn log_path() -> PathBuf {
    state_dir().join("guictl-mcp.log")
}

fn read_state() -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(state_path()).ok()?).ok()
}

/// Writes the record readable by this user alone, since it holds the token.
/// XDG_RUNTIME_DIR is private already, but the fallback, /tmp, is not.
fn write_private(path: &std::path::Path, contents: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    // `mode` only applies to a new file; tighten one left from before.
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    file.write_all(contents.as_bytes())
}

/// Removes the record, unless a different server has replaced it since.
fn remove_state(pid: u32) {
    if read_state().is_some_and(|s| s.pid == pid) {
        let _ = std::fs::remove_file(state_path());
    }
}

/// The recorded server, if it is up and is the one the record describes.
fn live_server() -> Option<State> {
    let state = read_state()?;
    (health(state.port) == Some(state.pid)).then_some(state)
}

fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

fn new_token() -> String {
    use std::hash::{BuildHasher, RandomState};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    // Each RandomState is seeded from the OS's randomness.
    let (a, b) = (RandomState::new().hash_one(nanos), RandomState::new().hash_one((nanos, std::process::id())));
    format!("{a:016x}{b:016x}")
}

// ---- start / stop / status ----

fn start(port: u16) -> CmdResult {
    if let Some(s) = live_server() {
        return Ok(json!({ "ok": true, "already_running": true, "url": url(s.port), "port": s.port, "pid": s.pid }));
    }
    let exe = std::env::current_exe().map_err(|e| CmdError::os(format!("could not find guictl's own path: {e}")))?;
    let log = std::fs::File::create(log_path())
        .map_err(|e| CmdError::os(format!("could not create {}: {e}", log_path().display())))?;
    let log_err = log.try_clone().map_err(|e| CmdError::os(e.to_string()))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["mcp", "serve", "--port", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log_err);
    let mut child = spawn_detached(&mut cmd).map_err(|e| CmdError::os(format!("could not start the server: {e}")))?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(exit)) = child.try_wait() {
            let log = std::fs::read_to_string(log_path()).unwrap_or_default();
            let reason = serde_json::from_str::<Value>(log.trim())
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| log.trim().to_owned());
            return Err(CmdError::os(format!("the server stopped before it was ready ({exit}): {reason}")));
        }
        if let Some(s) = read_state().filter(|s| s.pid == child.id())
            && health(s.port) == Some(s.pid)
        {
            return Ok(json!({ "ok": true, "url": url(s.port), "port": s.port, "pid": s.pid }));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(CmdError::timeout(format!(
                "the server did not answer within 10 seconds; see {}",
                log_path().display()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn stop() -> CmdResult {
    let not_running = || CmdError::not_found("no guictl MCP server is running");
    let Some(state) = live_server() else {
        // A record with nothing of ours answering is left over from a server
        // that was killed or a computer that was restarted.
        if let Some(stale) = read_state() {
            remove_state(stale.pid);
        }
        return Err(not_running());
    };
    // The server exits as soon as it answers, so a lost answer is fine;
    // whether it went away is checked below.
    if let Ok((status, body)) = http(state.port, "POST", "/shutdown", &[("X-Guictl-Token", &state.token)])
        && status != 200
    {
        return Err(CmdError::os(format!("the server refused to stop ({status}): {}", body.trim())));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while health(state.port).is_some() {
        if Instant::now() > deadline {
            return Err(CmdError::timeout(format!("the server (pid {}) is still running", state.pid)));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    remove_state(state.pid);
    Ok(json!({ "ok": true, "stopped": state.pid }))
}

fn status() -> CmdResult {
    Ok(match live_server() {
        Some(s) => json!({ "running": true, "url": url(s.port), "port": s.port, "pid": s.pid }),
        None => json!({ "running": false }),
    })
}

/// Starts the server so that it outlives this guictl and the terminal it
/// was run from.
#[cfg(windows)]
fn spawn_detached(cmd: &mut std::process::Command) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    use windows::Win32::Foundation::{HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation};
    use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Windows hands every inheritable handle to a child. If our stdout is a
    // pipe from the caller, the server holding it would keep the caller
    // waiting for output until the server exits.
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        if let Ok(handle) = unsafe { GetStdHandle(which) } {
            let _ = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) };
        }
    }
    let flags = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    // Terminals and agent harnesses often run us in a job that is killed
    // with them; leave it if it allows that.
    cmd.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB).spawn().or_else(|_| cmd.creation_flags(flags).spawn())
}

#[cfg(unix)]
fn spawn_detached(cmd: &mut std::process::Command) -> io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    // Its own process group keeps the terminal's Ctrl+C and hangup away.
    cmd.process_group(0).spawn()
}

#[cfg(not(any(windows, unix)))]
fn spawn_detached(cmd: &mut std::process::Command) -> io::Result<std::process::Child> {
    cmd.spawn()
}

/// A minimal HTTP client for talking to our own server.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> io::Result<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 0\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response.split_whitespace().nth(1).and_then(|s| s.parse().ok());
    let status = status.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an HTTP response"))?;
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body).to_owned();
    Ok((status, body))
}

/// The pid of the guictl server on this port, if one answers there.
fn health(port: u16) -> Option<u32> {
    let (status, body) = http(port, "GET", "/health", &[]).ok()?;
    let body: Value = serde_json::from_str(&body).ok()?;
    if status != 200 || body["name"] != "guictl" {
        return None;
    }
    body["pid"].as_u64()?.try_into().ok()
}

// ---- the server ----

fn serve(port: u16) -> CmdResult {
    if let Some(s) = live_server() {
        return Err(CmdError::invalid_args(format!(
            "a guictl MCP server is already running at {} (pid {}); `guictl mcp stop` stops it",
            url(s.port),
            s.pid
        )));
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| CmdError::os(format!("could not listen on 127.0.0.1:{port}: {e}")))?;
    let port = listener.local_addr().map_or(port, |a| a.port());
    let state = State { pid: std::process::id(), port, token: new_token() };
    let record = serde_json::to_string(&state).unwrap();
    write_private(&state_path(), &record)
        .map_err(|e| CmdError::os(format!("could not write {}: {e}", state_path().display())))?;
    eprintln!("guictl MCP server listening at {} (pid {})", url(port), state.pid);

    let worker = spawn_worker();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (worker, token) = (worker.clone(), state.token.clone());
        std::thread::spawn(move || serve_connection(stream, &worker, &token));
    }
    unreachable!("the listener never stops yielding connections")
}

type Job = Box<dyn FnOnce() + Send>;
type Worker = mpsc::Sender<Job>;

fn spawn_worker() -> Worker {
    let (tx, rx) = mpsc::channel::<Job>();
    std::thread::spawn(move || {
        for job in rx {
            // A panic loses that call's answer, which then reports an
            // error, but not the server.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        }
    });
    tx
}

fn on_worker<T: Send + 'static>(worker: &Worker, f: impl FnOnce() -> T + Send + 'static) -> Result<T, CmdError> {
    let (tx, rx) = mpsc::channel();
    worker
        .send(Box::new(move || {
            let _ = tx.send(f());
        }))
        .map_err(|_| CmdError::os("the server's worker thread has stopped"))?;
    rx.recv().map_err(|_| CmdError::os("the command crashed; see the server's log"))
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    allow: Option<&'static str>,
    /// Exit the server once this has been sent.
    then_exit: bool,
}

impl Response {
    fn json(status: u16, value: &Value) -> Self {
        Self { status, content_type: "application/json", body: value.to_string().into_bytes(), allow: None, then_exit: false }
    }

    fn text(status: u16, text: &str) -> Self {
        Self { status, content_type: "text/plain; charset=utf-8", body: text.as_bytes().to_vec(), allow: None, then_exit: false }
    }
}

const MAX_BODY: usize = 16 << 20;

fn serve_connection(stream: TcpStream, worker: &Worker, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let request = match read_request(&mut reader, &mut writer) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(e) => {
                let _ = write_response(&mut writer, &Response::text(400, &format!("bad request: {e}")), true);
                return;
            }
        };
        let close = request.header("connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
        let response = route(&request, worker, token);
        let sent = write_response(&mut writer, &response, close || response.then_exit);
        if response.then_exit {
            // End the connection properly first: exiting with it open makes
            // Windows reset it, and the client may lose the answer.
            let _ = writer.shutdown(std::net::Shutdown::Write);
            remove_state(std::process::id());
            eprintln!("guictl MCP server stopped");
            std::process::exit(0);
        }
        if sent.is_err() || close {
            return;
        }
    }
}

fn read_request(reader: &mut impl BufRead, writer: &mut impl Write) -> io::Result<Option<Request>> {
    let bad = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else { return Err(bad("malformed request line")) };
    let (method, path) = (method.to_owned(), path.to_owned());
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(bad("connection closed in the headers"));
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').ok_or_else(|| bad("malformed header"))?;
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
        if headers.len() > 100 {
            return Err(bad("too many headers"));
        }
    }
    let mut request = Request { method, path, headers, body: Vec::new() };
    if request.header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    if request.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = line.trim().split(';').next().unwrap_or("");
            let size = usize::from_str_radix(size, 16).map_err(|_| bad("malformed chunk size"))?;
            if request.body.len() + size > MAX_BODY {
                return Err(bad("body too large"));
            }
            if size == 0 {
                // Trailers, up to the blank line that ends the request.
                loop {
                    line.clear();
                    if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                break;
            }
            let start = request.body.len();
            request.body.resize(start + size, 0);
            reader.read_exact(&mut request.body[start..])?;
            line.clear();
            reader.read_line(&mut line)?;
        }
    } else if let Some(length) = request.header("content-length") {
        let length: usize = length.parse().map_err(|_| bad("malformed Content-Length"))?;
        if length > MAX_BODY {
            return Err(bad("body too large"));
        }
        request.body.resize(length, 0);
        reader.read_exact(&mut request.body)?;
    }
    Ok(Some(request))
}

fn write_response(writer: &mut impl Write, response: &Response, close: bool) -> io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    if let Some(allow) = response.allow {
        head.push_str(&format!("Allow: {allow}\r\n"));
    }
    if close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    writer.write_all(head.as_bytes())?;
    writer.write_all(&response.body)?;
    writer.flush()
}

/// Whether a Host or Origin authority names this computer. Checking both
/// stops a web page in a browser from driving the desktop through the
/// server, including by pointing its own domain name at 127.0.0.1.
fn is_local(authority: &str) -> bool {
    ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|host| authority == *host || authority.strip_prefix(host).is_some_and(|rest| rest.starts_with(':')))
}

fn route(request: &Request, worker: &Worker, token: &str) -> Response {
    let origin_ok = request.header("origin").is_none_or(|o| o.split_once("://").is_some_and(|(_, a)| is_local(a)));
    if !origin_ok || !request.header("host").is_none_or(is_local) {
        return Response::text(403, "guictl only answers requests made from this computer, not from web pages");
    }
    let path = request.path.split('?').next().unwrap_or("");
    match (request.method.as_str(), path) {
        ("POST", "/mcp") => mcp_post(&request.body, worker),
        // No event streams and no sessions, so no GET or DELETE.
        (_, "/mcp") => Response { allow: Some("POST"), ..Response::text(405, "use POST") },
        ("GET", "/health") => Response::json(
            200,
            &json!({ "name": "guictl", "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() }),
        ),
        ("POST", "/shutdown") if request.header("x-guictl-token") == Some(token) => {
            Response { then_exit: true, ..Response::json(200, &json!({ "ok": true })) }
        }
        ("POST", "/shutdown") => Response::text(403, "wrong or missing X-Guictl-Token; use `guictl mcp stop`"),
        _ => Response::text(404, "not found; the MCP endpoint is /mcp"),
    }
}

fn mcp_post(body: &[u8], worker: &Worker) -> Response {
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return Response::json(400, &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } }));
    };
    let reply = match message {
        // Batches are from the 2025-03-26 protocol; still cheap to accept.
        Value::Array(messages) => {
            let replies: Vec<Value> = messages.iter().filter_map(|m| handle_message(m, worker)).collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        message => handle_message(&message, worker),
    };
    match reply {
        Some(reply) => Response::json(200, &reply),
        None => Response::text(202, ""),
    }
}

// ---- MCP ----

const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Answers one JSON-RPC message; notifications and responses get nothing.
fn handle_message(message: &Value, worker: &Worker) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(&params, worker),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

const INSTRUCTIONS_PREFACE: &str = "This server runs guictl commands on the computer it runs on. \
Call the `guictl` tool with a command's arguments as a list, e.g. {\"args\": [\"window\", \"list\"]}; \
add \"--help\" to see a command's flags. A failed command comes back as a tool error holding \
{\"error\":{\"code\",\"message\"}}. `screenshot` without --output returns the image itself. \
Everything the guide below says about commands, stdout, and exit codes applies to these tool results.\n\n";

fn initialize(params: &Value) -> Value {
    let requested = params["protocolVersion"].as_str();
    let version = requested.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "guictl", "title": "guictl", "version": env!("CARGO_PKG_VERSION") },
        "instructions": format!("{INSTRUCTIONS_PREFACE}{}", crate::guide::text()),
    })
}

fn tools() -> Value {
    let mut commands = String::new();
    for sub in Cli::command().get_subcommands() {
        if matches!(sub.get_name(), "batch" | "mcp" | "help") {
            continue;
        }
        let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
        commands.push_str(&format!("\n  {} - {about}", sub.get_name()));
    }
    json!([
        {
            "name": "guictl",
            "title": "Run a guictl command",
            "description": format!(
                "Control this computer's desktop: run one guictl command and get its JSON result. \
`args` is the command line after `guictl`, one item per argument, e.g. [\"window\",\"list\"], \
[\"mouse\",\"click\",\"640\",\"400\"], [\"keyboard\",\"type\",\"hello world\"]. \
Append \"--help\" to any command for its flags. Read the server's instructions, or run [\"guide\"], \
before driving the desktop. `screenshot` without --output returns the picture as an image; \
add --scale 0.5 on large desktops.\n\nCommands:{commands}"
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The command and its arguments, e.g. [\"window\",\"focus\",\"--app\",\"firefox\"]",
                    },
                },
                "required": ["args"],
            },
        },
        {
            "name": "guictl_batch",
            "title": "Run several guictl commands",
            "description": "Run several guictl commands in order and get every step's outcome. \
The batch stops at the first step that fails; the steps after it are listed with code \"skipped\". A `wait window --new` or `wait change` step sees what \
the step before it did, which separate calls can miss. With stop_on_user_input, the remaining steps \
are skipped as soon as someone touches the mouse or keyboard.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": { "type": "array", "items": { "type": "string" } },
                        "description": "One argument list per command, e.g. [[\"window\",\"focus\",\"--app\",\"kwrite\"],[\"keyboard\",\"type\",\"hello\"]]",
                    },
                    "stop_on_user_input": {
                        "type": "boolean",
                        "description": "Skip the remaining steps once someone uses the mouse or keyboard",
                    },
                },
                "required": ["steps"],
            },
        },
    ])
}

fn call_tool(params: &Value, worker: &Worker) -> Result<Value, (i64, String)> {
    let name = params["name"].as_str().ok_or((-32602, "tools/call needs a tool name".to_owned()))?;
    let arguments = &params["arguments"];
    match name {
        "guictl" => Ok(match string_list(&arguments["args"]) {
            Some(argv) => run_command(argv, worker),
            None => tool_error(&CmdError::invalid_args("`args` must be a list of strings, e.g. [\"window\",\"list\"]")),
        }),
        "guictl_batch" => {
            let steps = arguments["steps"].as_array().and_then(|s| s.iter().map(string_list).collect::<Option<Vec<_>>>());
            let Some(steps) = steps else {
                return Ok(tool_error(&CmdError::invalid_args("`steps` must be a list of argument lists")));
            };
            Ok(run_batch(steps, arguments["stop_on_user_input"].as_bool().unwrap_or(false), worker))
        }
        _ => Err((-32602, format!("unknown tool: {name}"))),
    }
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    value.as_array()?.iter().map(|v| v.as_str().map(str::to_owned)).collect()
}

/// Commands that make no sense, or would take the server down, inside it.
fn check_allowed(command: &Command) -> Result<(), CmdError> {
    match command {
        Command::Mcp(_) => Err(CmdError::invalid_args("`mcp` commands can't run inside the MCP server")),
        Command::Batch(_) => Err(CmdError::invalid_args("use the guictl_batch tool instead of `batch`")),
        Command::Screenshot(ScreenshotArgs { output: Some(o), .. }) if o == "-" => Err(CmdError::invalid_args(
            "leave out --output to get the screenshot back as an image",
        )),
        _ => Ok(()),
    }
}

fn parse(argv: &[String]) -> Result<Cli, Result<String, CmdError>> {
    use clap::error::ErrorKind;
    let cli = Cli::try_parse_from(std::iter::once("guictl").chain(argv.iter().map(String::as_str))).map_err(|e| {
        match e.kind() {
            // Asked-for help is an answer, not a failure.
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Ok(e.to_string()),
            _ => Err(CmdError::invalid_args(e.to_string())),
        }
    })?;
    #[cfg(all(windows, not(guictl_stub)))]
    if cli.elevate {
        return Err(Err(CmdError::invalid_args(
            "--elevate isn't available through the MCP server; start the server from an administrator terminal instead",
        )));
    }
    check_allowed(&cli.command).map_err(Err)?;
    Ok(cli)
}

fn run_command(argv: Vec<String>, worker: &Worker) -> Value {
    let cli = match parse(&argv) {
        Ok(cli) => cli,
        Err(Ok(help)) => return text_result(help),
        Err(Err(e)) => return tool_error(&e),
    };
    match cli.command {
        Command::Guide => text_result(crate::guide::text()),
        Command::Screenshot(args) if args.output.is_none() => {
            match on_worker(worker, move || commands::capture(&args)).and_then(|r| r) {
                Ok((width, height, png, origin)) => {
                    let mut info = json!({ "width": width, "height": height });
                    if let Some((x, y)) = origin {
                        info["x"] = json!(x);
                        info["y"] = json!(y);
                    }
                    json!({
                        "content": [
                            { "type": "image", "data": base64(&png), "mimeType": "image/png" },
                            { "type": "text", "text": info.to_string() },
                        ],
                    })
                }
                Err(e) => tool_error(&e),
            }
        }
        command => match on_worker(worker, move || commands::dispatch(command)).and_then(|r| r) {
            Ok(value) => text_result(value.to_string()),
            Err(e) => tool_error(&e),
        },
    }
}

fn run_batch(steps: Vec<Vec<String>>, stop_on_user_input: bool, worker: &Worker) -> Value {
    // Steps that don't parse are reported by the batch itself, in place.
    for argv in &steps {
        if let Some(Err(e)) = commands::parse_step(argv).ok().map(|cli| check_allowed(&cli.command)) {
            return tool_error(&e);
        }
    }
    let result = on_worker(worker, move || -> CmdResult {
        // One watch for the server's life: each one starts threads that
        // listen to the input devices until the process ends.
        thread_local! {
            static WATCH: std::cell::OnceCell<crate::platform::os::input::Watch> = const { std::cell::OnceCell::new() };
        }
        if !stop_on_user_input {
            return Ok(commands::run_steps(steps, None));
        }
        WATCH.with(|cell| {
            if cell.get().is_none() {
                let _ = cell.set(crate::platform::os::input::watch()?);
            }
            let watch = cell.get().unwrap();
            watch.take(); // forget input from before this batch
            Ok(commands::run_steps(steps, Some(watch)))
        })
    });
    match result.and_then(|r| r) {
        Ok(value) => {
            let failed = value["all_ok"] == false;
            json!({ "content": [{ "type": "text", "text": value.to_string() }], "isError": failed })
        }
        Err(e) => tool_error(&e),
    }
}

fn text_result(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn tool_error(err: &CmdError) -> Value {
    let body = json!({ "error": { "code": err.code, "message": err.message } });
    json!({ "content": [{ "type": "text", "text": body.to_string() }], "isError": true })
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn only_this_computer_is_local() {
        for ok in ["localhost", "localhost:47800", "127.0.0.1:47800", "[::1]:47800"] {
            assert!(is_local(ok), "{ok}");
        }
        for bad in ["evil.com", "localhost.evil.com", "127.0.0.1.evil.com:80", "null"] {
            assert!(!is_local(bad), "{bad}");
        }
    }

    #[test]
    fn server_breaking_commands_are_refused() {
        let refused = |args: &[&str]| matches!(parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()), Err(Err(_)));
        assert!(refused(&["mcp", "stop"]));
        assert!(refused(&["batch", "--lines"]));
        assert!(refused(&["screenshot", "--output", "-"]));
        assert!(!refused(&["window", "list"]));
        assert!(!refused(&["screenshot"]));
        assert!(matches!(parse(&["window".into(), "--help".into()]), Err(Ok(_))));
    }

    #[test]
    fn requests_are_read_with_either_body_framing() {
        let raw = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}\
POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n1\r\n{\r\n1\r\n}\r\n0\r\n\r\n";
        let mut reader = BufReader::new(&raw[..]);
        let mut sink = Vec::new();
        for _ in 0..2 {
            let request = read_request(&mut reader, &mut sink).unwrap().unwrap();
            assert_eq!((request.method.as_str(), request.path.as_str(), &request.body[..]), ("POST", "/mcp", &b"{}"[..]));
        }
        assert!(read_request(&mut reader, &mut sink).unwrap().is_none());
    }

    #[test]
    fn notifications_get_no_reply() {
        let worker = spawn_worker();
        assert!(handle_message(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), &worker).is_none());
        let reply = handle_message(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }), &worker).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        let tools = handle_message(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }), &worker).unwrap();
        assert_eq!(tools["result"]["tools"][0]["name"], "guictl");
    }
}
