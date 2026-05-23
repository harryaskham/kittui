//! Minimal Unix-socket daemon protocol for kittwm.
//!
//! Single-line text requests; reply is one line. RAII guard removes the
//! socket file on drop. The server runs an accept loop on a worker
//! thread and exits when the main thread drops the [`DaemonServer`].

use anyhow::{anyhow, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Default socket path for the kittwm daemon.
///
/// Honors, in order:
///
/// - `KITTWM_SOCKET` / `KITTWM_SOCK`: explicit socket path.
/// - `KITTUI_WM_DISPLAY` / `KITTWM_DISPLAY`: display-style path or `:N`
///   shorthand, where `:1` maps to `/tmp/kittui-wm-1.sock`.
/// - fallback `/tmp/kittwm-$USER.sock`.
pub fn default_socket_path() -> PathBuf {
    for key in ["KITTWM_SOCKET", "KITTWM_SOCK"] {
        if let Ok(p) = std::env::var(key) {
            return PathBuf::from(p);
        }
    }
    for key in ["KITTUI_WM_DISPLAY", "KITTWM_DISPLAY"] {
        if let Ok(display) = std::env::var(key) {
            return display_to_socket_path(&display);
        }
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "anon".to_string());
    PathBuf::from(format!("/tmp/kittwm-{user}.sock"))
}

/// Convert a DISPLAY-like token into a socket path.
pub fn display_to_socket_path(display: &str) -> PathBuf {
    if let Some(id) = display.strip_prefix(':') {
        let id = id.split('.').next().unwrap_or(id);
        PathBuf::from(format!("/tmp/kittui-wm-{id}.sock"))
    } else {
        PathBuf::from(display)
    }
}

/// Metadata for a process spawned through the daemon protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedPane {
    /// Monotonic pane id assigned by the daemon.
    pub pane_id: u32,
    /// Window token exported to the child via `KITTWM_WINDOW`.
    pub window: String,
    /// OS process id.
    pub pid: u32,
    /// Original shell argv string.
    pub argv: String,
    /// Layout slot/orientation label.
    pub layout: String,
    /// Whether this pane is currently focused.
    pub focused: bool,
}

#[derive(Debug, Default)]
struct PaneRegistry {
    next_id: u32,
    panes: Vec<TrackedPane>,
    focused: Option<u32>,
}

impl PaneRegistry {
    fn track_spawn(&mut self, pid: u32, argv: &str) -> TrackedPane {
        self.next_id = self.next_id.saturating_add(1).max(1);
        let pane_id = self.next_id;
        for pane in &mut self.panes {
            pane.focused = false;
        }
        self.focused = Some(pane_id);
        let pane = TrackedPane {
            pane_id,
            window: format!("daemon-{pane_id}"),
            pid,
            argv: argv.to_string(),
            layout: format!("tile:{pane_id}"),
            focused: true,
        };
        self.panes.push(pane.clone());
        pane
    }
}

type SharedPanes = Arc<Mutex<PaneRegistry>>;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct NativePaneStatus {
    pub window: String,
    pub title: String,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativePaneCommand {
    SpawnPty(String),
    Focus(String),
    Close(String),
    Layout(String),
}

#[derive(Default, Debug)]
struct NativeSpawnQueueState {
    pending: Vec<NativePaneCommand>,
    panes: Vec<NativePaneStatus>,
    layout: Option<String>,
}

/// In-process socket queue used by the live native PTY session.
pub struct NativeSpawnQueue {
    path: PathBuf,
    quit: Arc<AtomicBool>,
    pending: Arc<Mutex<NativeSpawnQueueState>>,
    accept_thread: Option<JoinHandle<()>>,
}

impl NativeSpawnQueue {
    /// Bind a socket that accepts `SPAWN_PTY <cmd>` requests.
    pub fn bind(path: PathBuf) -> Result<Self> {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)
            .map_err(|e| anyhow!("bind native spawn queue {}: {e}", path.display()))?;
        let quit = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let quit_t = quit.clone();
        let pending_t = pending.clone();
        let path_t = path.clone();
        let accept_thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if quit_t.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = handle_native_spawn_request(stream, &pending_t);
            }
            let _ = std::fs::remove_file(&path_t);
        });
        Ok(Self {
            path,
            quit,
            pending,
            accept_thread: Some(accept_thread),
        })
    }

    /// Socket path bound by this queue.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Drain all queued native pane commands in FIFO order.
    pub fn drain(&self) -> Vec<NativePaneCommand> {
        drain_native_spawn_pending(&self.pending)
    }

    /// Publish a live native pane snapshot for STATUS/PANES requests.
    pub fn update_panes(&self, panes: Vec<NativePaneStatus>) {
        if let Ok(mut state) = self.pending.lock() {
            state.panes = panes;
        }
    }

    /// Publish the live native pane layout axis for STATUS requests.
    pub fn update_layout(&self, layout: impl Into<String>) {
        if let Ok(mut state) = self.pending.lock() {
            state.layout = Some(layout.into());
        }
    }
}

impl Drop for NativeSpawnQueue {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        if let Some(join) = self.accept_thread.take() {
            let _ = join.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn drain_native_spawn_pending(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
) -> Vec<NativePaneCommand> {
    let Ok(mut state) = pending.lock() else {
        return Vec::new();
    };
    std::mem::take(&mut state.pending)
}

fn handle_native_spawn_request(
    mut stream: UnixStream,
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
) -> Result<()> {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(stream.try_clone()?);
        reader.read_line(&mut line)?;
    }
    let reply = native_spawn_queue_reply(line.trim(), pending);
    stream.write_all(reply.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn native_spawn_queue_reply(cmd: &str, pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    if let Some(query) = cmd.strip_prefix("APPS_FIRST ") {
        return apps_first_reply(query, false);
    }
    if let Some(query) = cmd.strip_prefix("APPS_LAUNCH_FIRST ") {
        return apps_first_reply(query, true);
    }
    if let Some(argv) = cmd.strip_prefix("SPAWN_PTY ") {
        return queue_native_pane_command(
            pending,
            argv,
            "SPAWN_PTY requires argv",
            NativePaneCommand::SpawnPty,
            "QUEUED",
        );
    }
    if let Some(window) = cmd.strip_prefix("FOCUS_PANE ") {
        return queue_native_pane_command(
            pending,
            window,
            "FOCUS_PANE requires window",
            NativePaneCommand::Focus,
            "FOCUS_QUEUED",
        );
    }
    if let Some(window) = cmd.strip_prefix("CLOSE_PANE ") {
        return queue_native_pane_command(
            pending,
            window,
            "CLOSE_PANE requires window",
            NativePaneCommand::Close,
            "CLOSE_QUEUED",
        );
    }
    if let Some(axis) = cmd.strip_prefix("LAYOUT ") {
        let axis = axis.trim().to_ascii_lowercase();
        if !matches!(axis.as_str(), "columns" | "rows") {
            return "ERR LAYOUT expects columns|rows\n".to_string();
        }
        return queue_native_pane_command(
            pending,
            &axis,
            "LAYOUT requires columns|rows",
            NativePaneCommand::Layout,
            "LAYOUT_QUEUED",
        );
    }
    match cmd {
        "PING" => "PONG\n".to_string(),
        "STATUS" => native_spawn_status_reply(pending),
        "STATUS_JSON" => native_spawn_status_json_reply(pending),
        "PANES" => native_spawn_panes_reply(pending),
        "PANES_JSON" => native_spawn_panes_json_reply(pending),
        "APPS" => apps_reply(50),
        "APPS_JSON" => apps_json_reply(50),
        "HELP" | "?" => native_spawn_help_reply(),
        "HELP_JSON" => native_spawn_help_json_reply(),
        _ => "ERR expected SPAWN_PTY <cmd> | FOCUS_PANE <window> | CLOSE_PANE <window|focused> | LAYOUT <columns|rows> | STATUS_JSON | PANES_JSON | APPS | APPS_JSON | HELP\n"
            .to_string(),
    }
}

fn native_spawn_help_entries() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("PING", "health", "return PONG"),
        (
            "STATUS",
            "inspect",
            "text status with pending/panes/focus/layout",
        ),
        (
            "STATUS_JSON",
            "inspect",
            "JSON status with pending/panes/focus/layout",
        ),
        ("PANES", "inspect", "text visible native pane listing"),
        ("PANES_JSON", "inspect", "JSON visible native pane listing"),
        (
            "SPAWN_PTY <cmd>",
            "control",
            "spawn a visible native PTY pane",
        ),
        (
            "FOCUS_PANE <window>",
            "control",
            "focus a native pane by window token",
        ),
        (
            "CLOSE_PANE <window|focused>",
            "control",
            "close a native pane",
        ),
        (
            "LAYOUT <columns|rows>",
            "control",
            "switch native pane layout axis",
        ),
        ("APPS", "apps", "text app discovery listing"),
        ("APPS_JSON", "apps", "JSON app discovery listing"),
        (
            "APPS_FIRST <query>",
            "apps",
            "find the first app matching query",
        ),
        (
            "APPS_LAUNCH_FIRST <query>",
            "apps",
            "find and launch the first app matching query",
        ),
        ("HELP", "help", "show this command catalog"),
        ("HELP_JSON", "help", "show this command catalog as JSON"),
    ]
}

fn native_spawn_help_reply() -> String {
    use std::fmt::Write as _;
    let mut out = String::from("kittwm native socket commands\n");
    for (command, category, description) in native_spawn_help_entries() {
        let _ = writeln!(out, "  {command} [{category}] — {description}");
    }
    out.push_str("END\n");
    out
}

fn native_spawn_help_json_reply() -> String {
    let commands = native_spawn_help_entries()
        .into_iter()
        .map(|(command, category, description)| {
            serde_json::json!({
                "command": command,
                "category": category,
                "description": description,
            })
        })
        .collect::<Vec<_>>();
    format!("{}\n", serde_json::json!({ "commands": commands }))
}

fn queue_native_pane_command(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    arg: &str,
    empty_error: &str,
    build: impl FnOnce(String) -> NativePaneCommand,
    ok_prefix: &str,
) -> String {
    let arg = arg.trim();
    if arg.is_empty() {
        return format!("ERR {empty_error}\n");
    }
    match pending.lock() {
        Ok(mut state) => {
            state.pending.push(build(arg.to_string()));
            format!("{ok_prefix} command={} arg={}\n", state.pending.len(), arg)
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn native_spawn_status_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    pending
        .lock()
        .map(|state| {
            let focused = state
                .panes
                .iter()
                .find(|pane| pane.focused)
                .map(|pane| pane.window.as_str())
                .unwrap_or("-");
            format!(
                "OK pending={} panes={} focus={} layout={}\n",
                state.pending.len(),
                state.panes.len(),
                focused,
                state.layout.as_deref().unwrap_or("-")
            )
        })
        .unwrap_or_else(|_| "ERR registry poisoned\n".to_string())
}

fn native_spawn_status_json_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let focused = state
        .panes
        .iter()
        .find(|pane| pane.focused)
        .map(|pane| pane.window.as_str())
        .unwrap_or("-");
    format!(
        "{}\n",
        serde_json::json!({
            "pending": state.pending.len(),
            "panes": state.panes.len(),
            "focus": focused,
            "layout": state.layout.as_deref().unwrap_or("-"),
        })
    )
}

fn native_spawn_panes_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    use std::fmt::Write as _;
    let Ok(state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let focused = state
        .panes
        .iter()
        .find(|pane| pane.focused)
        .map(|pane| pane.window.as_str())
        .unwrap_or("-");
    let mut out = String::new();
    let _ = writeln!(out, "PANES {} focus={}", state.panes.len(), focused);
    for pane in &state.panes {
        let _ = writeln!(
            out,
            "  window={} focused={} title={:?}",
            pane.window, pane.focused, pane.title
        );
    }
    out.push_str("END\n");
    out
}

fn native_spawn_panes_json_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let focused = state
        .panes
        .iter()
        .find(|pane| pane.focused)
        .map(|pane| pane.window.as_str())
        .unwrap_or("-");
    format!(
        "{}\n",
        serde_json::json!({
            "panes": state.panes.len(),
            "focus": focused,
            "layout": state.layout.as_deref().unwrap_or("-"),
            "panes_detail": state.panes,
        })
    )
}

/// Accept-loop daemon that answers `PING` / `STATUS` / `QUIT`.
pub struct DaemonServer {
    path: PathBuf,
    started: Instant,
    quit: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
    panes: SharedPanes,
}

impl std::fmt::Debug for DaemonServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonServer")
            .field("path", &self.path)
            .field("uptime", &self.started.elapsed())
            .field("quit_requested", &self.quit.load(Ordering::SeqCst))
            .field(
                "panes",
                &self.panes.lock().map(|p| p.panes.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl DaemonServer {
    pub fn bind(path: PathBuf) -> Result<Self> {
        // If a stale socket exists, try to ping it. If a real server is
        // there we fail loudly; otherwise we unlink and rebind.
        if path.exists() {
            match client_request(&path, "PING") {
                Ok(reply) if reply.trim() == "PONG" => {
                    return Err(anyhow!(
                        "another kittwm daemon is already listening on {}",
                        path.display()
                    ));
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        let listener =
            UnixListener::bind(&path).map_err(|e| anyhow!("bind {}: {e}", path.display()))?;
        listener
            .set_nonblocking(false)
            .map_err(|e| anyhow!("set_nonblocking: {e}"))?;
        let started = Instant::now();
        let quit = Arc::new(AtomicBool::new(false));
        let panes = Arc::new(Mutex::new(PaneRegistry::default()));
        let quit_t = quit.clone();
        let panes_t = panes.clone();
        let path_t = path.clone();
        let accept_thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if quit_t.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = handle_request(stream, started, &path_t, &quit_t, &panes_t);
            }
            let _ = std::fs::remove_file(&path_t);
        });
        Ok(Self {
            path,
            started,
            quit,
            accept_thread: Some(accept_thread),
            panes,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True if a `QUIT` request was received by the accept thread.
    pub fn quit_requested(&self) -> bool {
        self.quit.load(Ordering::SeqCst)
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// Snapshot of panes spawned through this daemon.
    pub fn panes(&self) -> Vec<TrackedPane> {
        self.panes
            .lock()
            .map(|p| p.panes.clone())
            .unwrap_or_default()
    }
}

impl Drop for DaemonServer {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        // Wake the accept loop by connecting once.
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.accept_thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle_request(
    stream: UnixStream,
    started: Instant,
    path: &Path,
    quit: &AtomicBool,
    panes: &SharedPanes,
) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let cmd = line.trim();
    let reply = if let Some(query) = cmd.strip_prefix("APPS_FIRST ") {
        apps_first_reply(query, false)
    } else if let Some(query) = cmd.strip_prefix("APPS_LAUNCH_FIRST ") {
        apps_first_reply(query, true)
    } else if let Some(argv) = cmd.strip_prefix("SPAWN ") {
        spawn_reply(argv, path, panes)
    } else {
        match cmd {
        "PING" => "PONG\n".to_string(),
        "STATUS" => format!(
            "pid={} uptime_s={} sock={} panes={} focus={}\n",
            std::process::id(),
            started.elapsed().as_secs(),
            path.display(),
            panes.lock().map(|p| p.panes.len()).unwrap_or(0),
            panes.lock().ok().and_then(|p| p.focused).map(|id| id.to_string()).unwrap_or_else(|| "-".to_string())
        ),
        "WINDOWS" => windows_reply(),
        "DISPLAYS" => displays_reply(),
        "APPS" => apps_reply(50),
        "APPS_JSON" => apps_json_reply(50),
        "PANES" => panes_reply(panes),
        "HELP" | "?" => {
            "PING | STATUS | WINDOWS | DISPLAYS | APPS | APPS_JSON | APPS_FIRST <query> | APPS_LAUNCH_FIRST <query> | SPAWN <argv> | PANES | QUIT | HELP\n".to_string()
        }
        "QUIT" => {
            quit.store(true, Ordering::SeqCst);
            "BYE\n".to_string()
        }
        other => format!("ERR unknown: {other}\n"),
        }
    };
    writer.write_all(reply.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// Send a single-line request and return the reply line.
pub fn client_request(path: &Path, cmd: &str) -> Result<String> {
    let mut stream =
        UnixStream::connect(path).map_err(|e| anyhow!("connect {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_sock() -> PathBuf {
        std::env::temp_dir().join(format!("kittwm-test-{}.sock", std::process::id()))
    }

    #[test]
    fn display_to_socket_path_supports_colon_display() {
        assert_eq!(
            display_to_socket_path(":7"),
            PathBuf::from("/tmp/kittui-wm-7.sock")
        );
        assert_eq!(
            display_to_socket_path(":7.0"),
            PathBuf::from("/tmp/kittui-wm-7.sock")
        );
        assert_eq!(
            display_to_socket_path("/tmp/custom.sock"),
            PathBuf::from("/tmp/custom.sock")
        );
    }

    #[test]
    fn ping_pong_round_trip() {
        let p = tmp_sock();
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let reply = client_request(server.path(), "PING").unwrap();
        assert_eq!(reply.trim(), "PONG");
    }

    #[test]
    fn status_includes_pid_and_uptime() {
        let p =
            std::env::temp_dir().join(format!("kittwm-test-status-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let reply = client_request(server.path(), "STATUS").unwrap();
        assert!(reply.contains("pid="), "{reply}");
        assert!(reply.contains("uptime_s="), "{reply}");
        assert!(reply.contains("sock="), "{reply}");
    }

    #[test]
    fn quit_sets_flag() {
        let p = std::env::temp_dir().join(format!("kittwm-test-quit-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let reply = client_request(server.path(), "QUIT").unwrap();
        assert_eq!(reply.trim(), "BYE");
        // Give the accept thread a moment.
        std::thread::sleep(Duration::from_millis(50));
        assert!(server.quit_requested());
    }

    #[test]
    fn spawn_command_returns_tracked_pane() {
        let p = std::env::temp_dir().join(format!("kittwm-test-spawn-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let reply = client_request(server.path(), "SPAWN /bin/echo daemon-spawn-ok").unwrap();
        assert!(
            reply.starts_with("SPAWNED pane=1 window=daemon-1 pid="),
            "{reply}"
        );
        assert!(reply.contains("daemon-spawn-ok"), "{reply}");
        assert!(reply.contains("layout=tile:1"), "{reply}");
        let panes = client_request_multi(server.path(), "PANES").unwrap();
        assert!(panes.contains("PANES 1"), "{panes}");
        assert!(panes.contains("pane=1 window=daemon-1"), "{panes}");
        assert_eq!(server.panes().len(), 1);
    }

    #[test]
    fn native_spawn_queue_parses_and_drains_fifo() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        assert_eq!(native_spawn_queue_reply("PING", &pending).trim(), "PONG");
        assert!(native_spawn_queue_reply("SPAWN_PTY", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SPAWN_PTY htop", &pending).starts_with("QUEUED"));
        assert!(
            native_spawn_queue_reply("SPAWN_PTY bash -lc true", &pending).starts_with("QUEUED")
        );
        assert_eq!(
            drain_native_spawn_pending(&pending),
            vec![
                NativePaneCommand::SpawnPty("htop".to_string()),
                NativePaneCommand::SpawnPty("bash -lc true".to_string())
            ]
        );
        assert!(drain_native_spawn_pending(&pending).is_empty());
    }

    #[test]
    fn native_spawn_queue_parses_focus_close_and_layout_commands() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        assert!(
            native_spawn_queue_reply("FOCUS_PANE native-2", &pending).starts_with("FOCUS_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("CLOSE_PANE focused", &pending).starts_with("CLOSE_QUEUED")
        );
        assert!(native_spawn_queue_reply("LAYOUT rows", &pending).starts_with("LAYOUT_QUEUED"));
        assert!(native_spawn_queue_reply("LAYOUT diagonal", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("FOCUS_PANE", &pending).contains("ERR"));
        assert_eq!(
            drain_native_spawn_pending(&pending),
            vec![
                NativePaneCommand::Focus("native-2".to_string()),
                NativePaneCommand::Close("focused".to_string()),
                NativePaneCommand::Layout("rows".to_string())
            ]
        );
    }

    #[test]
    fn native_spawn_queue_serves_help_catalogs() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let help = native_spawn_queue_reply("HELP", &pending);
        assert!(help.contains("SPAWN_PTY <cmd>"), "{help}");
        assert!(help.contains("STATUS_JSON"), "{help}");
        assert!(help.contains("PANES_JSON"), "{help}");
        assert!(help.contains("LAYOUT <columns|rows>"), "{help}");
        assert!(help.contains("APPS_JSON"), "{help}");

        let help_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("HELP_JSON", &pending)).unwrap();
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "LAYOUT <columns|rows>" && entry["category"] == "control"
            }));
    }

    #[test]
    fn native_spawn_queue_serves_app_discovery_commands() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let apps_json = native_spawn_queue_reply("APPS_JSON", &pending);
        let value: serde_json::Value = serde_json::from_str(&apps_json).unwrap();
        assert!(value.get("default_command").is_some(), "{apps_json}");
        assert!(
            value.get("path_commands").unwrap().is_array(),
            "{apps_json}"
        );

        let first = native_spawn_queue_reply("APPS_FIRST sh", &pending);
        assert!(
            first.starts_with("APP ")
                || first.starts_with("APPS_FIRST ")
                || first.starts_with("ERR no app match"),
            "{first}"
        );
    }

    #[test]
    fn native_spawn_queue_reports_live_pane_status() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        pending.lock().unwrap().panes = vec![
            NativePaneStatus {
                window: "native-1".to_string(),
                title: "shell".to_string(),
                focused: false,
            },
            NativePaneStatus {
                window: "native-2".to_string(),
                title: "htop".to_string(),
                focused: true,
            },
        ];
        pending.lock().unwrap().layout = Some("rows".to_string());
        assert_eq!(
            native_spawn_queue_reply("STATUS", &pending).trim(),
            "OK pending=0 panes=2 focus=native-2 layout=rows"
        );
        let panes = native_spawn_queue_reply("PANES", &pending);
        assert!(panes.contains("PANES 2 focus=native-2"), "{panes}");
        assert!(
            panes.contains("window=native-1 focused=false title=\"shell\""),
            "{panes}"
        );
        assert!(
            panes.contains("window=native-2 focused=true title=\"htop\""),
            "{panes}"
        );
        let status_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("STATUS_JSON", &pending)).unwrap();
        assert_eq!(status_json["pending"], 0);
        assert_eq!(status_json["panes"], 2);
        assert_eq!(status_json["focus"], "native-2");
        assert_eq!(status_json["layout"], "rows");
        let panes_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("PANES_JSON", &pending)).unwrap();
        assert_eq!(panes_json["panes_detail"].as_array().unwrap().len(), 2);
        assert_eq!(panes_json["panes_detail"][1]["window"], "native-2");
        assert_eq!(panes_json["panes_detail"][1]["focused"], true);
    }

    #[test]
    fn double_bind_detects_existing_daemon() {
        let p = std::env::temp_dir().join(format!("kittwm-test-dup-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let _a = DaemonServer::bind(p.clone()).unwrap();
        let err = DaemonServer::bind(p.clone()).unwrap_err();
        assert!(err.to_string().contains("already listening"), "{err}");
    }
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn windows_reply() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let wins = kittui_quartz::QuartzServer::list_app_windows();
    let _ = writeln!(out, "WINDOWS {}", wins.len());
    for w in wins {
        let _ = writeln!(
            out,
            "  id={} owner={:?} title={:?} bounds=({:.0},{:.0} {:.0}x{:.0})",
            w.id,
            w.owner_name,
            w.title,
            w.bounds.origin.0,
            w.bounds.origin.1,
            w.bounds.width,
            w.bounds.height,
        );
    }
    out.push_str("END\n");
    out
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn windows_reply() -> String {
    "ERR WINDOWS requires --features quartz on macOS\n".to_string()
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn displays_reply() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let ds = kittui_quartz::QuartzServer::displays();
    let _ = writeln!(out, "DISPLAYS {}", ds.len());
    for d in ds {
        let _ = writeln!(
            out,
            "  index={} id={} bounds=({:.0},{:.0} {:.0}x{:.0})",
            d.index, d.id, d.bounds.origin.0, d.bounds.origin.1, d.bounds.width, d.bounds.height
        );
    }
    out.push_str("END\n");
    out
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn displays_reply() -> String {
    "ERR DISPLAYS requires --features quartz on macOS\n".to_string()
}

/// Multi-line client request — keeps reading until EOF, or until a line
/// containing exactly "END" arrives (so multi-line replies like WINDOWS
/// don't drop after the first line).
pub fn client_request_multi(path: &Path, cmd: &str) -> Result<String> {
    let mut stream =
        UnixStream::connect(path).map_err(|e| anyhow!("connect {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut out = String::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if line.trim() == "END" {
            break;
        }
        out.push_str(&line);
        // Single-line replies don't send END; break after one if it
        // doesn't look like a known multi-line header.
        let first = out.lines().next().unwrap_or("");
        if !first.starts_with("WINDOWS ")
            && !first.starts_with("DISPLAYS ")
            && !first.starts_with("APPS ")
            && !first.starts_with("PANES ")
        {
            break;
        }
    }
    Ok(out)
}

fn panes_reply(panes: &SharedPanes) -> String {
    use std::fmt::Write;
    let Ok(registry) = panes.lock() else {
        return "ERR PANES registry poisoned\n".to_string();
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "PANES {} focus={}",
        registry.panes.len(),
        registry
            .focused
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    for pane in &registry.panes {
        let _ = writeln!(
            out,
            "  pane={} window={} pid={} layout={} focused={} argv={:?}",
            pane.pane_id, pane.window, pane.pid, pane.layout, pane.focused, pane.argv
        );
    }
    out.push_str("END\n");
    out
}

fn spawn_reply(argv: &str, path: &Path, panes: &SharedPanes) -> String {
    if argv.trim().is_empty() {
        return "ERR SPAWN requires argv\n".to_string();
    }
    let next_window = panes
        .lock()
        .map(|p| format!("daemon-{}", p.next_id.saturating_add(1).max(1)))
        .unwrap_or_else(|_| "daemon-unknown".to_string());
    match std::process::Command::new("/bin/sh")
        .arg("-lc")
        .arg(argv)
        .env("KITTWM_SOCKET", path)
        .env("KITTWM_SOCK", path)
        .env("KITTUI_WM_DISPLAY", path)
        .env("KITTWM_DISPLAY", path)
        .env("KITTWM_WINDOW", &next_window)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => match panes.lock() {
            Ok(mut registry) => {
                let pane = registry.track_spawn(child.id(), argv);
                format!(
                    "SPAWNED pane={} window={} pid={} layout={} focused={} argv={argv}\n",
                    pane.pane_id, pane.window, pane.pid, pane.layout, pane.focused
                )
            }
            Err(_) => format!("ERR SPAWN registry poisoned after pid={}\n", child.id()),
        },
        Err(e) => format!("ERR SPAWN {argv}: {e}\n"),
    }
}

fn apps_reply(limit: usize) -> String {
    use std::fmt::Write;
    let default_cmd = crate::session::launcher_command();
    let default_prog = default_cmd.split_whitespace().next().unwrap_or("xterm");
    let default_path = find_on_path(default_prog)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<not found on PATH>".to_string());
    let path_cmds = path_commands(limit);
    #[cfg(target_os = "macos")]
    let mac_apps = macos_apps(limit);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "APPS default={default_cmd:?} resolved={default_path:?}"
    );
    let _ = writeln!(out, "PATH_COMMANDS {}", path_cmds.len());
    for cmd in path_cmds {
        let _ = writeln!(out, "  {cmd}");
    }
    let _ = writeln!(out, "MACOS_APPS {}", mac_apps.len());
    for app in mac_apps {
        let _ = writeln!(out, "  {app}");
    }
    out.push_str("END\n");
    out
}

fn apps_json_reply(limit: usize) -> String {
    let default_cmd = crate::session::launcher_command();
    let default_prog = default_cmd.split_whitespace().next().unwrap_or("xterm");
    let default_path = find_on_path(default_prog);
    let path_cmds = path_commands(limit);
    #[cfg(target_os = "macos")]
    let mac_apps = macos_apps(limit);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();
    format!(
        "{{\"default_command\": {:?}, \"default_resolved\": {}, \"path_commands\": [{}], \"macos_apps\": [{}]}}\n",
        default_cmd,
        default_path
            .as_ref()
            .map(|p| format!("{:?}", p.display().to_string()))
            .unwrap_or_else(|| "null".to_string()),
        json_string_array(&path_cmds),
        json_string_array(&mac_apps),
    )
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.exists().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(program);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn path_commands(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(read) = std::fs::read_dir(dir) else {
                continue;
            };
            for ent in read.flatten() {
                let path = ent.path();
                if !path.is_file() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                out.insert(name.to_string());
                if out.len() >= limit {
                    break;
                }
            }
            if out.len() >= limit {
                break;
            }
        }
    }
    out.into_iter().take(limit).collect()
}

#[cfg(target_os = "macos")]
fn macos_apps(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for root in ["/Applications", "/System/Applications"] {
        let Ok(read) = std::fs::read_dir(root) else {
            continue;
        };
        for ent in read.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("app") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            out.insert(name.trim_end_matches(".app").to_string());
            if out.len() >= limit {
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.into_iter().take(limit).collect()
}

fn json_string_array(items: &[String]) -> String {
    items
        .iter()
        .map(|s| format!("{:?}", s))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone)]
struct AppCandidate {
    kind: &'static str,
    name: String,
}

fn apps_first_reply(query: &str, launch: bool) -> String {
    let query = query.trim();
    if query.is_empty() {
        return "ERR APPS_FIRST requires a query\n".to_string();
    }
    let path_cmds = filter_candidates(path_commands(5000), Some(query), 1);
    #[cfg(target_os = "macos")]
    let mac_apps = filter_candidates(macos_apps(5000), Some(query), 1);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();
    let Some(candidate) = first_app_candidate(&path_cmds, &mac_apps) else {
        return format!("ERR no app candidates matched {query:?}\n");
    };
    if launch {
        match launch_app_candidate(&candidate) {
            Ok(pid) => format!(
                "APPS_LAUNCH_FIRST pid={} kind={} name={}\n",
                pid, candidate.kind, candidate.name
            ),
            Err(e) => format!("ERR launch {}:{}: {e}\n", candidate.kind, candidate.name),
        }
    } else {
        format!(
            "APPS_FIRST kind={} name={}\n",
            candidate.kind, candidate.name
        )
    }
}

fn first_app_candidate(path_cmds: &[String], mac_apps: &[String]) -> Option<AppCandidate> {
    path_cmds
        .first()
        .map(|name| AppCandidate {
            kind: "path",
            name: name.clone(),
        })
        .or_else(|| {
            mac_apps.first().map(|name| AppCandidate {
                kind: "macos",
                name: name.clone(),
            })
        })
}

fn launch_app_candidate(candidate: &AppCandidate) -> Result<u32> {
    let mut cmd = if candidate.kind == "macos" {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(&candidate.name);
        c
    } else {
        std::process::Command::new(&candidate.name)
    };
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(child.id())
}

fn filter_candidates(items: Vec<String>, query: Option<&str>, limit: usize) -> Vec<String> {
    let Some(query) = query else {
        return items.into_iter().take(limit).collect();
    };
    let q = query.to_ascii_lowercase();
    items
        .into_iter()
        .filter(|item| item.to_ascii_lowercase().contains(&q))
        .take(limit)
        .collect()
}
