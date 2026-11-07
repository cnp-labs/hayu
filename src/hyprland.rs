use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_LINE_BUFFER: usize = 64 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub focused: bool,
    pub active_workspace: WorkspaceRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WorkspaceRef {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub monitor: String,
    pub monitor_id: i64,
    pub windows: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ActiveWindow {
    pub title: String,
    pub class: String,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: WorkspaceRef,
    pub active_window: ActiveWindow,
}

#[derive(Debug, Clone)]
pub struct Hyprland {
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum Event {
    Workspace { id: i64, name: Option<String> },
    ActiveWindow { title: String },
    FocusMonitor { monitor: String },
    CreateWorkspace { id: i64, name: String },
    DestroyWorkspace { name: String },
    Refresh,
    Resync,
    Disconnected,
}

impl Hyprland {
    pub fn discover() -> Option<Self> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
        let base = runtime.join("hypr");
        if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
            let dir = base.join(sig);
            if dir.join(".socket.sock").exists() {
                return Some(Self { dir });
            }
        }
        let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&base)
            .ok()?
            .filter_map(Result::ok)
            .filter(|e| e.path().join(".socket.sock").exists())
            .filter_map(|e| {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((modified, e.path()))
            })
            .collect();
        candidates.sort_by_key(|(m, _)| *m);
        candidates.pop().map(|(_, dir)| Self { dir })
    }

    pub fn request_socket(&self) -> PathBuf {
        self.dir.join(".socket.sock")
    }

    pub fn event_socket(&self) -> PathBuf {
        self.dir.join(".socket2.sock")
    }

    pub fn version(&self) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Version {
            version: String,
        }
        let version: Version = self.request("j/version")?;
        Ok(version.version)
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let monitors: Vec<Monitor> = self.request("j/monitors")?;
        let workspaces: Vec<Workspace> = self.request("j/workspaces")?;
        // These two can be empty or transiently unavailable; the snapshot is still useful.
        let active_window: ActiveWindow = self.request("j/activewindow").unwrap_or_default();
        let active_workspace: WorkspaceRef = self.request("j/activeworkspace").unwrap_or_default();
        Ok(Snapshot {
            monitors,
            workspaces,
            active_workspace,
            active_window,
        })
    }

    /// Focus a workspace by id. Hyprland 0.57 with a Lua config rejects the
    /// legacy `dispatch` string, so fall back to the Lua `repl` form.
    pub fn switch_workspace(&self, id: i64) -> Result<(), String> {
        match self.dispatch(&format!("dispatch workspace {id}")) {
            Ok(_) => Ok(()),
            Err(legacy) => self
                .dispatch(&format!(
                    "repl hl.dispatch(hl.dsp.focus({{ workspace = {id} }}))"
                ))
                .map(|_| ())
                .map_err(|lua| format!("legacy dispatch failed ({legacy}); Lua dispatch failed ({lua})")),
        }
    }
    /// Send a plain Hyprland command (`dispatch ...`) and return its raw reply.
    pub fn dispatch(&self, command: &str) -> Result<String, String> {
        let reply = self.exchange(command)?;
        if reply.trim_start().starts_with("ok") {
            Ok(reply)
        } else {
            Err(reply.trim().to_string())
        }
    }
    fn exchange(&self, command: &str) -> Result<String, String> {
        let mut stream = UnixStream::connect(self.request_socket())
            .map_err(|e| format!("cannot connect to Hyprland request socket: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
        stream
            .write_all(command.as_bytes())
            .map_err(|e| format!("cannot write Hyprland request: {e}"))?;
        stream.shutdown(std::net::Shutdown::Write).ok();
        let mut response = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stream
                .read(&mut chunk)
                .map_err(|e| format!("Hyprland read failed: {e}"))?;
            if n == 0 {
                break;
            }
            if response.len() + n > MAX_RESPONSE {
                return Err("Hyprland response exceeded 4 MiB".into());
            }
            response.extend_from_slice(&chunk[..n]);
        }
        String::from_utf8(response).map_err(|e| format!("invalid Hyprland reply for {command}: {e}"))
    }
    fn request<T: for<'de> Deserialize<'de>>(&self, command: &str) -> Result<T, String> {
        let reply = self.exchange(command)?;
        serde_json::from_str(&reply)
            .map_err(|e| format!("invalid Hyprland response for {command}: {e}"))
    }

    fn connect_events(&self) -> Result<UnixStream, String> {
        UnixStream::connect(self.event_socket())
            .map_err(|e| format!("cannot connect to Hyprland event socket: {e}"))
    }
}

pub fn spawn_event_thread(tx: calloop::channel::Sender<Event>) {
    thread::spawn(move || {
        let mut backoff = Duration::from_millis(250);
        loop {
            let Some(hypr) = Hyprland::discover() else {
                if tx.send(Event::Disconnected).is_err() {
                    return;
                }
                thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            };
            let mut stream = match hypr.connect_events() {
                Ok(stream) => stream,
                Err(_) => {
                    if tx.send(Event::Disconnected).is_err() {
                        return;
                    }
                    thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                    continue;
                }
            };
            backoff = Duration::from_millis(250);
            if tx.send(Event::Resync).is_err() {
                return;
            }
            let mut lines = LineBuffer::default();
            let mut chunk = [0u8; 8192];
            let mut received = false;
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if !received {
                            received = true;
                            backoff = Duration::from_millis(250);
                        }
                        for event in lines.feed(&chunk[..n]) {
                            if tx.send(event).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            let _ = tx.send(Event::Disconnected);
            thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    });
}

#[derive(Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(data);
        if self.buf.len() > MAX_LINE_BUFFER {
            // A peer that never sends a newline must not grow this buffer without bound.
            self.buf.clear();
            return Vec::new();
        }
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(event) = parse_event_line(&String::from_utf8_lossy(&line)) {
                events.push(event);
            }
        }
        events
    }
}

pub fn parse_event_line(line: &str) -> Option<Event> {
    let (name, payload) = line.split_once(">>")?;
    match name {
        "workspace" | "workspacev2" => {
            let (id, label) = match payload.split_once(',') {
                Some((id, label)) => (id.parse().ok()?, Some(label.to_string())),
                None => (payload.parse().ok()?, None),
            };
            Some(Event::Workspace { id, name: label })
        }
        "activewindow" => {
            let title = payload
                .split_once(',')
                .map(|(_, title)| title)
                .unwrap_or("");
            Some(Event::ActiveWindow {
                title: title.to_string(),
            })
        }
        "focusedmon" => {
            let (monitor, _) = payload.split_once(',')?;
            Some(Event::FocusMonitor {
                monitor: monitor.to_string(),
            })
        }
        "createworkspace" | "createworkspacev2" => {
            let (id, name) = match payload.split_once(',') {
                Some((id, name)) => (id.parse().unwrap_or(0), name.to_string()),
                None => (payload.parse().unwrap_or(0), payload.to_string()),
            };
            Some(Event::CreateWorkspace { id, name })
        }
        "destroyworkspace" | "destroyworkspacev2" => {
            let name = payload
                .rsplit_once(',')
                .map(|(_, name)| name)
                .unwrap_or(payload);
            Some(Event::DestroyWorkspace {
                name: name.to_string(),
            })
        }
        "monitoradded" | "monitorremoved" | "monitoraddedv2" | "monitorremovedv2"
        | "moveworkspace" | "renameworkspace" => Some(Event::Refresh),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_partial_lines() {
        let mut buf = LineBuffer::default();
        assert!(buf.feed(b"workspace>").is_empty());
        assert_eq!(buf.feed(b">3\nactivewin").len(), 1);
        let events = buf.feed(b"dow>>firefox,Hello, world\n");
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ActiveWindow { title } => assert_eq!(title, "Hello, world"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn multiple_lines_and_unknown_ignored() {
        let mut buf = LineBuffer::default();
        let events = buf
            .feed(b"unknown>>thing\nworkspacev2>>2,web\nactivewindowv2>>abc\nmonitoradded>>DP-1\n");
        assert_eq!(events.len(), 2);
        match &events[0] {
            Event::Workspace {
                id: 2,
                name: Some(name),
            } => assert_eq!(name, "web"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(events[1], Event::Refresh));
    }

    #[test]
    fn focused_monitor_and_workspace_lifecycle() {
        let mut buf = LineBuffer::default();
        let events = buf.feed(b"focusedmon>>DP-1,2\nrenameworkspace>>2,new\n");
        assert!(matches!(&events[0], Event::FocusMonitor { monitor } if monitor == "DP-1"));
        assert!(matches!(&events[1], Event::Refresh));
    }

    #[test]
    fn malformed_workspace_is_ignored() {
        assert!(parse_event_line("workspace>>not-a-number").is_none());
        assert!(parse_event_line("no separator").is_none());
    }
}
