use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST: usize = 4096;
const MAX_RESPONSE: usize = 4096;
const IO_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    LauncherToggle,
    LauncherOpen,
    LauncherClose,
    Quit,
    Reload,
}

impl Command {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "launcher.toggle" => Ok(Self::LauncherToggle),
            "launcher.open" => Ok(Self::LauncherOpen),
            "launcher.close" => Ok(Self::LauncherClose),
            "quit" => Ok(Self::Quit),
            "reload" => Ok(Self::Reload),
            other => Err(format!("unknown command: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub cmd: String,
}

impl Request {
    pub fn new(id: u64, cmd: impl Into<String>) -> Self {
        Self {
            id,
            cmd: cmd.into(),
        }
    }

    pub fn command(&self) -> Result<Command, String> {
        Command::parse(&self.cmd)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Applied, but the running shell cannot pick this up; the CLI prints it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

impl Response {
    pub fn ok(id: u64) -> Self {
        Self {
            id,
            ok: true,
            error: None,
            notice: None,
        }
    }
    pub fn ok_with_notice(id: u64, notice: String) -> Self {
        Self {
            id,
            ok: true,
            error: None,
            notice: Some(notice),
        }
    }

    pub fn error(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            error: Some(error.into()),
            notice: None,
        }
    }
}

pub fn session_id() -> String {
    let raw = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("WAYLAND_DISPLAY")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "default".into());
    sanitize_session(&raw)
}

pub fn sanitize_session(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .take(64)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() || out.chars().all(|ch| ch == '.') {
        out = "default".into();
    }
    out
}

pub fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_string())?;
    if !runtime.is_absolute() || !runtime.is_dir() {
        return Err(format!(
            "XDG_RUNTIME_DIR is not an absolute directory: {}",
            runtime.display()
        ));
    }
    Ok(socket_path_for(&runtime, &session_id()))
}

pub fn socket_path_for(runtime: &Path, session: &str) -> PathBuf {
    runtime
        .join("hayu")
        .join(sanitize_session(session))
        .join("control.sock")
}

#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

impl Server {
    pub fn bind() -> Result<Self, String> {
        let path = socket_path()?;
        Self::bind_path(path)
    }

    fn bind_path(path: PathBuf) -> Result<Self, String> {
        let Some(parent) = path.parent() else {
            return Err(format!("invalid control socket path: {}", path.display()));
        };
        if parent.exists() {
            if !parent.is_dir() {
                return Err(format!(
                    "control socket directory is not a directory: {}",
                    parent.display()
                ));
            }
        } else {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "cannot create control socket directory {}: {e}",
                    parent.display()
                )
            })?;
        }
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(|e| {
            format!(
                "cannot set permissions on control socket directory {}: {e}",
                parent.display()
            )
        })?;

        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket() {
                return Err(format!(
                    "control socket path exists and is not a socket: {}",
                    path.display()
                ));
            }
            match UnixStream::connect(&path) {
                Ok(_) => {
                    return Err(format!(
                        "another hayu instance is already listening at {}",
                        path.display()
                    ));
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::ConnectionRefused
                        || error.kind() == std::io::ErrorKind::NotFound =>
                {
                    std::fs::remove_file(&path).map_err(|e| {
                        format!("cannot remove stale control socket {}: {e}", path.display())
                    })?;
                }
                Err(error) => {
                    return Err(format!(
                        "cannot connect to existing control socket {}: {error}",
                        path.display()
                    ));
                }
            }
        }

        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("cannot bind control socket {}: {e}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot set control socket permissions: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot make control socket nonblocking: {e}"))?;
        Ok(Self { listener, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_parts(self) -> (UnixListener, PathBuf) {
        (self.listener, self.path)
    }
}

pub struct Job {
    pub request: Result<Request, String>,
    pub stream: UnixStream,
}

/// Read one request with the size cap and a one-second timeout. Runs on a worker thread.
pub fn handle_connection(mut stream: UnixStream, tx: calloop::channel::Sender<Job>) {
    let request = read_request(&mut stream);
    let _ = tx.send(Job { request, stream });
}

fn read_request(stream: &mut UnixStream) -> Result<Request, String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("cannot set control read timeout: {e}"))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|e| format!("cannot read control request: {e}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_REQUEST {
            return Err(format!("request exceeds {MAX_REQUEST} bytes"));
        }
        if buffer.contains(&b'\n') {
            break;
        }
    }
    parse_request(&buffer)
}

pub fn parse_request(bytes: &[u8]) -> Result<Request, String> {
    if bytes.len() > MAX_REQUEST {
        return Err(format!("request exceeds {MAX_REQUEST} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "request is not valid UTF-8".to_string())?;
    let text = text.trim_end_matches(['\n', '\r']);
    if text.is_empty() {
        return Err("empty request".into());
    }
    if text.contains(['\n', '\r']) {
        return Err("request must contain exactly one line".into());
    }
    serde_json::from_str(text).map_err(|e| format!("invalid request JSON: {e}"))
}

pub fn read_response(stream: &mut UnixStream) -> Result<Response, String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("cannot set control read timeout: {e}"))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|e| format!("cannot read control response: {e}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_RESPONSE {
            return Err(format!("response exceeds {MAX_RESPONSE} bytes"));
        }
        if buffer.contains(&b'\n') {
            break;
        }
    }
    let text =
        std::str::from_utf8(&buffer).map_err(|_| "response is not valid UTF-8".to_string())?;
    let text = text.trim_end_matches(['\n', '\r']);
    serde_json::from_str(text).map_err(|e| format!("invalid response JSON: {e}"))
}

pub fn write_response(stream: &mut UnixStream, response: &Response) -> Result<(), String> {
    let mut line =
        serde_json::to_vec(response).map_err(|e| format!("cannot encode response: {e}"))?;
    line.push(b'\n');
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("cannot set control write timeout: {e}"))?;
    stream
        .write_all(&line)
        .map_err(|e| format!("cannot write control response: {e}"))
}

pub fn send_command(command: &str) -> Result<Option<String>, String> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "hayu shell is not running (no control socket at {})",
                path.display()
            )
        } else {
            format!(
                "cannot connect to hayu shell at {}: {error}",
                path.display()
            )
        }
    })?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("cannot set control write timeout: {e}"))?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let request = Request::new(id, command);
    let mut line =
        serde_json::to_vec(&request).map_err(|e| format!("cannot encode request: {e}"))?;
    line.push(b'\n');
    stream
        .write_all(&line)
        .map_err(|e| format!("cannot send control request: {e}"))?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(|e| format!("cannot finish control request: {e}"))?;
    let response = read_response(&mut stream)?;
    if response.id != id {
        return Err(format!(
            "control response id mismatch: expected {id}, got {}",
            response.id
        ));
    }
    if response.ok {
        Ok(response.notice)
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "hayu rejected the command".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hayu-ipc-{}-{}-{label}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitizes_session_ids() {
        assert_eq!(sanitize_session("wayland-1"), "wayland-1");
        assert_eq!(sanitize_session("../evil /x"), "..-evil--x");
        assert_eq!(sanitize_session(""), "default");
        assert_eq!(sanitize_session("..."), "default");
        let path = socket_path_for(Path::new("/run/user/1000"), "../evil /x");
        assert_eq!(
            path,
            PathBuf::from("/run/user/1000/hayu/..-evil--x/control.sock")
        );
    }

    #[test]
    fn parses_and_serializes_requests() {
        let request = parse_request(b"{\"id\":7,\"cmd\":\"launcher.toggle\"}\n").unwrap();
        assert_eq!(request.id, 7);
        assert_eq!(request.command().unwrap(), Command::LauncherToggle);
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            "{\"id\":7,\"cmd\":\"launcher.toggle\"}"
        );
        assert_eq!(Command::parse("quit").unwrap(), Command::Quit);
        assert_eq!(Command::parse("reload").unwrap(), Command::Reload);
        assert!(Command::parse("shell.exec").is_err());
    }

    #[test]
    fn rejects_bad_and_oversized_requests() {
        assert!(parse_request(b"").is_err());
        assert!(parse_request(b"not json\n").is_err());
        assert!(parse_request(b"{\"id\":1,\"cmd\":\"quit\"}\n{\"id\":2}\n").is_err());
        let mut oversized = vec![b'x'; MAX_REQUEST + 1];
        oversized.push(b'\n');
        assert!(parse_request(&oversized).unwrap_err().contains("exceeds"));
    }

    #[test]
    fn serializes_ok_and_error_responses() {
        let ok = serde_json::to_string(&Response::ok(1)).unwrap();
        assert_eq!(ok, "{\"id\":1,\"ok\":true}");
        let error = serde_json::to_string(&Response::error(2, "nope")).unwrap();
        assert_eq!(error, "{\"id\":2,\"ok\":false,\"error\":\"nope\"}");
        let parsed: Response = serde_json::from_str(&error).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.error.as_deref(), Some("nope"));
    }

    #[test]
    fn refuses_second_listener_and_replaces_stale_socket() {
        let runtime = temp_dir("runtime");
        let path = runtime.join("hayu").join("session").join("control.sock");
        let first = Server::bind_path(path.clone()).unwrap();
        let error = Server::bind_path(path.clone()).unwrap_err();
        assert!(error.contains("already listening"), "{error}");
        drop(first);
        // Dropping the listener without unlinking leaves a stale socket.
        let second = Server::bind_path(path.clone()).unwrap();
        assert_eq!(second.path(), path);
        let _ = std::fs::remove_dir_all(runtime);
    }

    #[test]
    fn refuses_non_socket_path() {
        let runtime = temp_dir("file");
        let path = runtime.join("hayu").join("session").join("control.sock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not a socket").unwrap();
        let error = Server::bind_path(path.clone()).unwrap_err();
        assert!(error.contains("not a socket"), "{error}");
        let _ = std::fs::remove_dir_all(runtime);
    }
}
