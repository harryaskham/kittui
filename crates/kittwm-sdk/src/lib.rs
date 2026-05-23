//! Small typed client skeleton for kittwm's DISPLAY/socket control plane.
//!
//! This crate intentionally starts as a thin wrapper around the existing native
//! socket protocol. As the SDK grows, the public handle types here should remain
//! the app-facing API while the transport can evolve underneath.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use std::env;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Result alias for kittwm SDK calls.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by the kittwm SDK skeleton.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No socket/display environment was available.
    #[error("no kittwm socket/display environment found")]
    MissingEnvironment,
    /// Underlying I/O failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON decoding failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// The daemon returned an error line.
    #[error("kittwm daemon error: {0}")]
    Daemon(String),
}

/// A connected kittwm client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kittwm {
    socket: PathBuf,
}

/// Coarse surface kind accepted by the v0 SDK.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// PTY-backed terminal surface.
    Terminal,
    /// Browser-backed surface. Transport support is planned, but not yet wired.
    Browser,
    /// External/unknown surface kind.
    Other(String),
}

/// Typed surface spawn request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceSpec {
    /// Surface kind.
    pub kind: SurfaceKind,
    /// Command or target for the surface.
    pub command: String,
    /// Optional title to apply once a stable id is known.
    pub title: Option<String>,
}

impl SurfaceSpec {
    /// Build a PTY terminal surface spec.
    pub fn terminal(command: impl Into<String>) -> Self {
        Self {
            kind: SurfaceKind::Terminal,
            command: command.into(),
            title: None,
        }
    }

    /// Attach a display title.
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// Result of queueing a surface spawn on the current socket transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceSpawn {
    /// Raw daemon reply for diagnostics.
    pub reply: String,
    /// Best-effort handle. Native `SPAWN_PTY` focuses the spawned pane, so this
    /// starts as `focused` until event APIs provide stable ids.
    pub handle: SurfaceHandle,
}

/// A typed handle to a kittwm surface/window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceHandle {
    client: Kittwm,
    /// Window/surface id, e.g. `native-1`, or the protocol alias `focused`.
    pub id: String,
}

/// Text snapshot returned by `READ_TEXT_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSnapshot {
    /// Window id.
    pub window: String,
    /// Screen text.
    pub text: String,
    /// Cursor column, when the daemon provides it.
    #[serde(default)]
    pub cursor_col: Option<u16>,
    /// Cursor row, when the daemon provides it.
    #[serde(default)]
    pub cursor_row: Option<u16>,
}

/// A typed handle to a kittwm window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowHandle {
    /// Window id, e.g. `native-1`.
    pub id: String,
}

/// Minimal status response shape shared by standalone and native daemons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Pending command count when available.
    #[serde(default)]
    pub pending: Option<u64>,
    /// Pane/window count when available.
    #[serde(default)]
    pub panes: Option<u64>,
    /// Focused window id when available.
    #[serde(default)]
    pub focus: Option<String>,
    /// Layout label when available.
    #[serde(default)]
    pub layout: Option<String>,
}

/// Basic window creation/replacement request. This is currently translated to
/// existing socket verbs and will grow into a richer SDK request later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSpec {
    /// Human-readable title.
    pub title: Option<String>,
    /// Command to spawn in a native PTY/window context.
    pub command: String,
}

impl Kittwm {
    /// Connect using `KITTWM_SOCKET` / `KITTWM_SOCK` / DISPLAY-like env vars.
    pub fn connect_from_env() -> Result<Self> {
        let socket = socket_path_from_env().ok_or(Error::MissingEnvironment)?;
        Ok(Self { socket })
    }

    /// Connect to an explicit kittwm socket path.
    pub fn connect_path(path: impl Into<PathBuf>) -> Self {
        Self {
            socket: path.into(),
        }
    }

    /// Return the socket path used by this client.
    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Return the current window id from `KITTWM_WINDOW`, when this process was
    /// launched inside a kittwm-managed window.
    pub fn current_window_from_env(&self) -> Option<WindowHandle> {
        env::var("KITTWM_WINDOW")
            .ok()
            .filter(|id| !id.trim().is_empty())
            .map(|id| WindowHandle { id })
    }

    /// Send a raw protocol command and return the text reply.
    pub fn request(&self, command: impl AsRef<str>) -> Result<String> {
        let reply = request_socket(&self.socket, command.as_ref())?;
        if let Some(err) = reply.strip_prefix("ERR ") {
            return Err(Error::Daemon(err.trim().to_string()));
        }
        Ok(reply)
    }

    /// Ping the daemon/control plane.
    pub fn ping(&self) -> Result<()> {
        let reply = self.request("PING")?;
        if reply.trim() == "PONG" {
            Ok(())
        } else {
            Err(Error::Daemon(reply.trim().to_string()))
        }
    }

    /// Fetch typed status JSON.
    pub fn status(&self) -> Result<Status> {
        Ok(serde_json::from_str(&self.request("STATUS_JSON")?)?)
    }

    /// Return a typed handle to an existing surface/window id.
    pub fn surface(&self, id: impl Into<String>) -> SurfaceHandle {
        SurfaceHandle {
            client: self.clone(),
            id: id.into(),
        }
    }

    /// Return a typed handle to the currently focused surface/window.
    pub fn focused_surface(&self) -> SurfaceHandle {
        self.surface("focused")
    }

    /// Ask kittwm to spawn a typed surface. The v0 transport supports terminal
    /// surfaces via `SPAWN_PTY`; richer surface kinds are reserved for later
    /// native protocol work.
    pub fn spawn_surface(&self, spec: &SurfaceSpec) -> Result<SurfaceSpawn> {
        let reply = match &spec.kind {
            SurfaceKind::Terminal => self.request(format!("SPAWN_PTY {}", spec.command))?,
            SurfaceKind::Browser => {
                return Err(Error::Daemon(
                    "browser surface spawning is not yet exposed by the SDK transport".to_string(),
                ))
            }
            SurfaceKind::Other(kind) => {
                return Err(Error::Daemon(format!(
                    "surface kind {kind:?} is not supported by the SDK transport"
                )))
            }
        };
        let handle = self.focused_surface();
        if let Some(title) = &spec.title {
            let _ = handle.rename(title);
        }
        Ok(SurfaceSpawn { reply, handle })
    }

    /// Ask kittwm to create a new native PTY window/surface using today's
    /// `SPAWN_PTY` socket verb. Returns the queued textual response for now.
    pub fn create_window(&self, spec: &WindowSpec) -> Result<String> {
        let spawn = self.spawn_surface(&SurfaceSpec::terminal(&spec.command))?;
        if let Some(title) = &spec.title {
            let _ = spawn.handle.rename(title);
        }
        Ok(spawn.reply)
    }

    /// Replace the current window in the coarse v0 skeleton. Until a dedicated
    /// replace request exists in the SDK transport, this queues a new PTY and
    /// closes the current one when `KITTWM_WINDOW` is available.
    pub fn replace_current(&self, spec: &WindowSpec) -> Result<String> {
        let reply = self.create_window(spec)?;
        if let Some(handle) = self.current_window_from_env() {
            let _ = self.request(format!("CLOSE_PANE {}", handle.id));
        }
        Ok(reply)
    }
}

impl SurfaceHandle {
    /// Focus this surface/window.
    pub fn focus(&self) -> Result<String> {
        self.client.request(format!("FOCUS_PANE {}", self.id))
    }

    /// Close this surface/window.
    pub fn close(&self) -> Result<String> {
        self.client.request(format!("CLOSE_PANE {}", self.id))
    }

    /// Rename this surface/window.
    pub fn rename(&self, title: impl AsRef<str>) -> Result<String> {
        self.client
            .request(format!("RENAME_PANE {} {}", self.id, title.as_ref()))
    }

    /// Resize this surface/window by a relative pane-weight delta.
    pub fn resize_weight(&self, delta: i16) -> Result<String> {
        let label = if delta >= 0 {
            format!("+{delta}")
        } else {
            delta.to_string()
        };
        self.client
            .request(format!("RESIZE_PANE {} {label}", self.id))
    }

    /// Send raw UTF-8 text.
    pub fn send_text(&self, text: impl AsRef<str>) -> Result<String> {
        self.client
            .request(format!("SEND_TEXT {} {}", self.id, text.as_ref()))
    }

    /// Send one line, appending a newline in the daemon.
    pub fn send_line(&self, text: impl AsRef<str>) -> Result<String> {
        self.client
            .request(format!("SEND_LINE {} {}", self.id, text.as_ref()))
    }

    /// Send a named key such as `ctrl-c`, `escape`, or `up`.
    pub fn send_key(&self, key: impl AsRef<str>) -> Result<String> {
        self.client
            .request(format!("SEND_KEY {} {}", self.id, key.as_ref()))
    }

    /// Read the current screen text snapshot.
    pub fn read_text(&self) -> Result<TextSnapshot> {
        Ok(serde_json::from_str(
            &self.client.request(format!("READ_TEXT_JSON {}", self.id))?,
        )?)
    }
}

/// Resolve a socket path using kittwm's current environment conventions.
pub fn socket_path_from_env() -> Option<PathBuf> {
    for key in ["KITTWM_SOCKET", "KITTWM_SOCK"] {
        if let Ok(path) = env::var(key) {
            if !path.trim().is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    for key in ["KITTUI_WM_DISPLAY", "KITTWM_DISPLAY"] {
        if let Ok(display) = env::var(key) {
            if !display.trim().is_empty() {
                return Some(display_to_socket_path(&display));
            }
        }
    }
    None
}

/// Map a DISPLAY-like token to the default kittwm socket path.
pub fn display_to_socket_path(display: &str) -> PathBuf {
    if display.starts_with('/') {
        return PathBuf::from(display);
    }
    let token = display
        .trim_start_matches(':')
        .split('.')
        .next()
        .unwrap_or(display)
        .replace('/', "_");
    env::temp_dir().join(format!("kittwm-{token}.sock"))
}

fn request_socket(path: &Path, command: &str) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.write_all(command.as_bytes())?;
        stream.write_all(b"\n")?;
        let mut out = String::new();
        stream.read_to_string(&mut out)?;
        Ok(out)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, command);
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "kittwm SDK socket transport is currently Unix-only",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn display_tokens_map_to_socket_paths() {
        assert_eq!(
            display_to_socket_path(":7.0"),
            env::temp_dir().join("kittwm-7.sock")
        );
        assert_eq!(
            display_to_socket_path("/tmp/custom.sock"),
            PathBuf::from("/tmp/custom.sock")
        );
    }

    #[test]
    fn connect_from_env_prefers_socket_over_display() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::set_var("KITTWM_SOCKET", "/tmp/kittwm-sdk.sock");
        env::set_var("KITTWM_DISPLAY", ":9");
        let client = Kittwm::connect_from_env().unwrap();
        assert_eq!(client.socket_path(), Path::new("/tmp/kittwm-sdk.sock"));
        env::remove_var("KITTWM_SOCKET");
        env::remove_var("KITTWM_DISPLAY");
    }

    #[test]
    fn current_window_handle_reads_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::set_var("KITTWM_WINDOW", "native-7");
        let client = Kittwm::connect_path("/tmp/kittwm-sdk.sock");
        assert_eq!(
            client.current_window_from_env(),
            Some(WindowHandle {
                id: "native-7".to_string()
            })
        );
        env::remove_var("KITTWM_WINDOW");
    }

    #[test]
    fn surface_spec_builds_terminal_specs() {
        assert_eq!(
            SurfaceSpec::terminal("htop").titled("monitor"),
            SurfaceSpec {
                kind: SurfaceKind::Terminal,
                command: "htop".to_string(),
                title: Some("monitor".to_string())
            }
        );
    }

    #[test]
    fn surface_handles_keep_client_and_id() {
        let client = Kittwm::connect_path("/tmp/kittwm-sdk.sock");
        let focused = client.focused_surface();
        assert_eq!(focused.id, "focused");
        assert_eq!(
            focused.client.socket_path(),
            Path::new("/tmp/kittwm-sdk.sock")
        );
    }

    #[test]
    fn text_snapshot_decodes_json_shape() {
        let snapshot: TextSnapshot = serde_json::from_str(
            r#"{"window":"native-1","text":"hello\n","cursor_col":2,"cursor_row":0}"#,
        )
        .unwrap();
        assert_eq!(snapshot.window, "native-1");
        assert_eq!(snapshot.text, "hello\n");
        assert_eq!(snapshot.cursor_col, Some(2));
    }
}
