//! Minimal Unix-socket daemon protocol for kittwm.
//!
//! Single-line text requests; reply is one line. RAII guard removes the
//! socket file on drop. The server runs an accept loop on a worker
//! thread and exits when the main thread drops the [`DaemonServer`].

use anyhow::{anyhow, Result};
use base64::Engine;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_WAIT_TEXT_MARGIN: Duration = Duration::from_secs(5);

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
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
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
    pub weight: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_x: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_y: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_col: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_row: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bracketed_paste: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_reporting: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_button_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_all_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_sgr: Option<bool>,
    #[serde(skip_serializing)]
    pub text_snapshot: Option<String>,
    #[serde(skip_serializing)]
    pub scrollback_snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_rows: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativePaneCommand {
    SpawnPty(String),
    Focus(String),
    FocusNext,
    FocusPrev,
    Close(String),
    Layout(String),
    Move {
        window: String,
        direction: String,
    },
    Resize {
        window: String,
        delta: i16,
    },
    Balance,
    Rename {
        window: String,
        title: String,
    },
    SendText {
        window: String,
        text: String,
        newline: bool,
    },
    SendBytes {
        window: String,
        bytes: Vec<u8>,
        label: String,
    },
    PasteBytes {
        window: String,
        bytes: Vec<u8>,
    },
    RestoreSession(NativeSessionRestore),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSessionRestore {
    pub layout: Option<String>,
    pub panes: Vec<NativeSessionRestorePane>,
    pub focus_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSessionRestorePane {
    pub title: Option<String>,
    pub command: String,
    pub weight: u16,
    pub focused: bool,
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
                let pending_client = pending_t.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT));
                    let _ = handle_native_spawn_request(stream, &pending_client);
                });
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
    if cmd == "FOCUS_NEXT" {
        return queue_native_pane_action(
            pending,
            NativePaneCommand::FocusNext,
            "FOCUS_NEXT_QUEUED",
        );
    }
    if cmd == "FOCUS_PREV" {
        return queue_native_pane_action(
            pending,
            NativePaneCommand::FocusPrev,
            "FOCUS_PREV_QUEUED",
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
    if let Some(rest) = cmd.strip_prefix("MOVE_PANE ") {
        let Some((window, direction)) = rest.trim().split_once(' ') else {
            return "ERR MOVE_PANE requires window and direction\n".to_string();
        };
        let window = window.trim();
        let direction = direction.trim().to_ascii_lowercase();
        if window.is_empty()
            || !matches!(
                direction.as_str(),
                "left" | "right" | "up" | "down" | "first" | "last"
            )
        {
            return "ERR MOVE_PANE expects <window|focused> <left|right|up|down|first|last>\n"
                .to_string();
        }
        return queue_native_pane_command(
            pending,
            &format!("{window}\t{direction}"),
            "MOVE_PANE requires window and direction",
            |arg| {
                let (window, direction) = arg.split_once('\t').unwrap_or((&arg, ""));
                NativePaneCommand::Move {
                    window: window.to_string(),
                    direction: direction.to_string(),
                }
            },
            "MOVE_QUEUED",
        );
    }
    if let Some(rest) = cmd.strip_prefix("RESIZE_PANE ") {
        let Some((window, amount)) = rest.trim().split_once(' ') else {
            return "ERR RESIZE_PANE requires window and amount\n".to_string();
        };
        let window = window.trim();
        let amount = amount.trim().to_ascii_lowercase();
        let delta = match amount.as_str() {
            "grow" | "+" => 1,
            "shrink" | "-" => -1,
            other => match other.parse::<i16>() {
                Ok(n) if n != 0 => n,
                _ => {
                    return "ERR RESIZE_PANE expects <window|focused> <grow|shrink|+N|-N>\n"
                        .to_string()
                }
            },
        };
        if window.is_empty() {
            return "ERR RESIZE_PANE requires window and amount\n".to_string();
        }
        return queue_native_pane_command(
            pending,
            &format!("{window}\t{delta}"),
            "RESIZE_PANE requires window and amount",
            |arg| {
                let (window, delta) = arg.split_once('\t').unwrap_or((&arg, "0"));
                NativePaneCommand::Resize {
                    window: window.to_string(),
                    delta: delta.parse().unwrap_or(0),
                }
            },
            "RESIZE_QUEUED",
        );
    }
    if cmd == "BALANCE_PANES" {
        return queue_native_pane_action(pending, NativePaneCommand::Balance, "BALANCE_QUEUED");
    }
    if let Some(rest) = cmd.strip_prefix("RESTORE_SESSION_JSON ") {
        return queue_native_restore_session(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("RENAME_PANE ") {
        let Some((window, title)) = rest.trim().split_once(' ') else {
            return "ERR RENAME_PANE requires window and title\n".to_string();
        };
        let window = window.trim();
        let title = title.trim();
        if window.is_empty() || title.is_empty() {
            return "ERR RENAME_PANE requires window and title\n".to_string();
        }
        return queue_native_pane_command(
            pending,
            &format!("{window}\t{title}"),
            "RENAME_PANE requires window and title",
            |arg| {
                let (window, title) = arg.split_once('\t').unwrap_or((&arg, ""));
                NativePaneCommand::Rename {
                    window: window.to_string(),
                    title: title.to_string(),
                }
            },
            "RENAME_QUEUED",
        );
    }
    if let Some(rest) = cmd.strip_prefix("SEND_TEXT ") {
        return queue_native_send_text(pending, rest, false);
    }
    if let Some(rest) = cmd.strip_prefix("SEND_LINE ") {
        return queue_native_send_text(pending, rest, true);
    }
    if let Some(rest) = cmd.strip_prefix("SEND_KEY ") {
        return queue_native_send_key(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("SEND_BYTES_B64 ") {
        return queue_native_send_bytes_b64(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("PASTE_BYTES_B64 ") {
        return queue_native_paste_bytes_b64(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_OUTPUT_MS ") {
        return native_spawn_wait_output_ms_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_OUTPUT ") {
        return native_spawn_wait_output_reply(pending, rest, Duration::from_secs(5));
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_TEXT_MS ") {
        return native_spawn_wait_text_ms_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_TEXT ") {
        return native_spawn_wait_text_reply(pending, rest, Duration::from_secs(5));
    }
    if let Some(target) = cmd.strip_prefix("READ_SCROLLBACK_JSON ") {
        return native_spawn_read_scrollback_json_reply(pending, target);
    }
    if let Some(target) = cmd.strip_prefix("READ_SCROLLBACK ") {
        return native_spawn_read_scrollback_reply(pending, target);
    }
    if let Some(target) = cmd.strip_prefix("READ_TEXT_JSON ") {
        return native_spawn_read_text_json_reply(pending, target);
    }
    if let Some(target) = cmd.strip_prefix("READ_TEXT ") {
        return native_spawn_read_text_reply(pending, target);
    }
    match cmd {
        "PING" => "PONG\n".to_string(),
        "STATUS" => native_spawn_status_reply(pending),
        "STATUS_JSON" => native_spawn_status_json_reply(pending),
        "PANES" => native_spawn_panes_reply(pending),
        "PANES_JSON" => native_spawn_panes_json_reply(pending),
        "SESSION_JSON" => native_spawn_session_json_reply(pending),
        "APPS" => apps_reply(50),
        "APPS_JSON" => apps_json_reply(50),
        "HELP" | "?" => native_spawn_help_reply(),
        "HELP_JSON" => native_spawn_help_json_reply(),
        _ => "ERR expected SPAWN_PTY <cmd> | FOCUS_PANE <window> | FOCUS_NEXT | FOCUS_PREV | CLOSE_PANE <window|focused> | LAYOUT <columns|rows> | MOVE_PANE <window|focused> <left|right|up|down|first|last> | RESIZE_PANE <window|focused> <grow|shrink|+N|-N> | BALANCE_PANES | RESTORE_SESSION_JSON <json> | RENAME_PANE <window> <title> | SEND_TEXT <window|focused> <text> | SEND_LINE <window|focused> <text> | SEND_KEY <window|focused> <key> | SEND_BYTES_B64 <window|focused> <base64> | PASTE_BYTES_B64 <window|focused> <base64> | READ_TEXT <window|focused> | READ_TEXT_JSON <window|focused> | READ_SCROLLBACK <window|focused> | READ_SCROLLBACK_JSON <window|focused> | WAIT_TEXT <window|focused> <needle> | WAIT_TEXT_MS <window|focused> <ms> <needle> | WAIT_OUTPUT <window|focused> <needle> | WAIT_OUTPUT_MS <window|focused> <ms> <needle> | SESSION_JSON | STATUS_JSON | PANES_JSON | APPS | APPS_JSON | HELP\n"
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
            "SESSION_JSON",
            "inspect",
            "JSON persistence-oriented native session manifest",
        ),
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
        ("FOCUS_NEXT", "control", "focus the next native pane"),
        ("FOCUS_PREV", "control", "focus the previous native pane"),
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
        (
            "MOVE_PANE <window|focused> <left|right|up|down|first|last>",
            "control",
            "move a native pane within the layout order",
        ),
        (
            "RESIZE_PANE <window|focused> <grow|shrink|+N|-N>",
            "control",
            "adjust a native pane layout weight",
        ),
        (
            "BALANCE_PANES",
            "control",
            "reset native pane weights to equal values",
        ),
        (
            "RESTORE_SESSION_JSON <json>",
            "control",
            "replace native panes from a SESSION_JSON manifest",
        ),
        (
            "RENAME_PANE <window> <title>",
            "control",
            "set display title for a native pane",
        ),
        (
            "SEND_TEXT <window|focused> <text>",
            "control",
            "send UTF-8 text bytes to a native pane",
        ),
        (
            "SEND_LINE <window|focused> <text>",
            "control",
            "send UTF-8 text plus newline to a native pane",
        ),
        (
            "SEND_KEY <window|focused> <key>",
            "control",
            "send a named key sequence to a native pane",
        ),
        (
            "SEND_BYTES_B64 <window|focused> <base64>",
            "automation",
            "send base64-decoded bytes to a native pane",
        ),
        (
            "PASTE_BYTES_B64 <window|focused> <base64>",
            "automation",
            "paste base64-decoded bytes, wrapping when bracketed paste is enabled",
        ),
        (
            "READ_TEXT <window|focused>",
            "inspect",
            "read a native pane text snapshot",
        ),
        (
            "READ_TEXT_JSON <window|focused>",
            "inspect",
            "read a native pane text snapshot as JSON",
        ),
        (
            "READ_SCROLLBACK <window|focused>",
            "inspect",
            "read native pane scrollback lines",
        ),
        (
            "READ_SCROLLBACK_JSON <window|focused>",
            "inspect",
            "read native pane scrollback lines as JSON",
        ),
        (
            "WAIT_TEXT <window|focused> <needle>",
            "automation",
            "wait until a native pane text snapshot contains text",
        ),
        (
            "WAIT_TEXT_MS <window|focused> <ms> <needle>",
            "automation",
            "wait until pane text contains text with explicit timeout",
        ),
        (
            "WAIT_OUTPUT <window|focused> <needle>",
            "automation",
            "wait until pane text or scrollback contains text",
        ),
        (
            "WAIT_OUTPUT_MS <window|focused> <ms> <needle>",
            "automation",
            "wait until pane text or scrollback contains text with explicit timeout",
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

fn queue_native_pane_action(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    command: NativePaneCommand,
    ok_prefix: &str,
) -> String {
    match pending.lock() {
        Ok(mut state) => {
            state.pending.push(command);
            format!("{ok_prefix} command={}\n", state.pending.len())
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
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

fn queue_native_restore_session(pending: &Arc<Mutex<NativeSpawnQueueState>>, json: &str) -> String {
    let json = json.trim();
    if json.is_empty() {
        return "ERR RESTORE_SESSION_JSON requires json\n".to_string();
    }
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(err) => return format!("ERR RESTORE_SESSION_JSON invalid json: {err}\n"),
    };
    let layout = value
        .get("layout")
        .and_then(|v| v.as_str())
        .filter(|layout| matches!(*layout, "columns" | "rows"))
        .map(str::to_string);
    let Some(items) = value.get("panes").and_then(|v| v.as_array()) else {
        return "ERR RESTORE_SESSION_JSON requires panes array\n".to_string();
    };
    if items.is_empty() {
        return "ERR RESTORE_SESSION_JSON requires at least one pane\n".to_string();
    }
    let mut panes = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let command = item
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if command.is_empty() {
            return format!("ERR RESTORE_SESSION_JSON pane {idx} missing command\n");
        }
        let weight = item
            .get("weight")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, u64::from(u16::MAX)) as u16;
        panes.push(NativeSessionRestorePane {
            title: item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|title| !title.trim().is_empty())
                .map(str::to_string),
            command: command.to_string(),
            weight,
            focused: item
                .get("focused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }
    let focus_index = panes.iter().position(|pane| pane.focused);
    match pending.lock() {
        Ok(mut state) => {
            state
                .pending
                .push(NativePaneCommand::RestoreSession(NativeSessionRestore {
                    layout,
                    panes,
                    focus_index,
                }));
            format!("RESTORE_SESSION_QUEUED command={}\n", state.pending.len())
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn queue_native_send_text(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    newline: bool,
) -> String {
    let Some((window, text)) = rest.trim_start().split_once(' ') else {
        return if newline {
            "ERR SEND_LINE requires window and text\n".to_string()
        } else {
            "ERR SEND_TEXT requires window and text\n".to_string()
        };
    };
    let window = window.trim();
    if window.is_empty() || text.is_empty() {
        return if newline {
            "ERR SEND_LINE requires window and text\n".to_string()
        } else {
            "ERR SEND_TEXT requires window and text\n".to_string()
        };
    }
    match pending.lock() {
        Ok(mut state) => {
            state.pending.push(NativePaneCommand::SendText {
                window: window.to_string(),
                text: text.to_string(),
                newline,
            });
            let prefix = if newline {
                "SEND_LINE_QUEUED"
            } else {
                "SEND_TEXT_QUEUED"
            };
            format!(
                "{prefix} command={} window={} bytes={}\n",
                state.pending.len(),
                window,
                text.len() + usize::from(newline)
            )
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn queue_native_send_bytes_b64(pending: &Arc<Mutex<NativeSpawnQueueState>>, rest: &str) -> String {
    let (window, bytes) = match parse_window_base64(rest, "SEND_BYTES_B64") {
        Ok(parsed) => parsed,
        Err(err) => return err,
    };
    match pending.lock() {
        Ok(mut state) => {
            state.pending.push(NativePaneCommand::SendBytes {
                window: window.clone(),
                bytes: bytes.clone(),
                label: "base64".to_string(),
            });
            format!(
                "SEND_BYTES_B64_QUEUED command={} window={} bytes={}\n",
                state.pending.len(),
                window,
                bytes.len()
            )
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn queue_native_paste_bytes_b64(pending: &Arc<Mutex<NativeSpawnQueueState>>, rest: &str) -> String {
    let (window, bytes) = match parse_window_base64(rest, "PASTE_BYTES_B64") {
        Ok(parsed) => parsed,
        Err(err) => return err,
    };
    match pending.lock() {
        Ok(mut state) => {
            state.pending.push(NativePaneCommand::PasteBytes {
                window: window.clone(),
                bytes: bytes.clone(),
            });
            format!(
                "PASTE_BYTES_B64_QUEUED command={} window={} bytes={}\n",
                state.pending.len(),
                window,
                bytes.len()
            )
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn parse_window_base64(rest: &str, verb: &str) -> Result<(String, Vec<u8>), String> {
    let Some((window, encoded)) = rest.trim().split_once(' ') else {
        return Err(format!("ERR {verb} requires window and base64\n"));
    };
    let window = window.trim();
    let encoded = encoded.trim();
    if window.is_empty() || window.contains(char::is_whitespace) || encoded.is_empty() {
        return Err(format!("ERR {verb} requires window and base64\n"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| format!("ERR {verb} invalid base64: {err}\n"))?;
    Ok((window.to_string(), bytes))
}

fn queue_native_send_key(pending: &Arc<Mutex<NativeSpawnQueueState>>, rest: &str) -> String {
    let Some((window, key)) = rest.trim().split_once(' ') else {
        return "ERR SEND_KEY requires window and key\n".to_string();
    };
    let window = window.trim();
    let key = key.trim();
    if window.is_empty() || key.is_empty() || key.contains(char::is_whitespace) {
        return "ERR SEND_KEY requires window and single key name\n".to_string();
    }
    let Some(bytes) = native_key_bytes(key) else {
        return "ERR SEND_KEY unsupported key; expected enter|tab|escape|backspace|delete|left|right|up|down|home|end|pageup|pagedown|ctrl-a..ctrl-z\n".to_string();
    };
    match pending.lock() {
        Ok(mut state) => {
            state.pending.push(NativePaneCommand::SendBytes {
                window: window.to_string(),
                bytes: bytes.clone(),
                label: key.to_string(),
            });
            format!(
                "SEND_KEY_QUEUED command={} window={} key={} bytes={}\n",
                state.pending.len(),
                window,
                key,
                bytes.len()
            )
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn native_key_bytes(key: &str) -> Option<Vec<u8>> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
    let bytes: &[u8] = match normalized.as_str() {
        "enter" | "return" => b"\r",
        "tab" => b"\t",
        "escape" | "esc" => b"\x1b",
        "backspace" | "bs" => b"\x7f",
        "delete" | "del" => b"\x1b[3~",
        "left" | "arrow-left" => b"\x1b[D",
        "right" | "arrow-right" => b"\x1b[C",
        "up" | "arrow-up" => b"\x1b[A",
        "down" | "arrow-down" => b"\x1b[B",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" | "page-up" => b"\x1b[5~",
        "pagedown" | "page-down" => b"\x1b[6~",
        _ => return native_ctrl_key_bytes(&normalized),
    };
    Some(bytes.to_vec())
}

fn native_ctrl_key_bytes(normalized: &str) -> Option<Vec<u8>> {
    let suffix = normalized
        .strip_prefix("ctrl-")
        .or_else(|| normalized.strip_prefix("c-"))?;
    let mut chars = suffix.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || !ch.is_ascii_alphabetic() {
        return None;
    }
    Some(vec![(ch.to_ascii_lowercase() as u8) & 0x1f])
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
    let focused_pane = state.panes.iter().find(|pane| pane.focused).cloned();
    format!(
        "{}\n",
        serde_json::json!({
            "pending": state.pending.len(),
            "panes": state.panes.len(),
            "focus": focused,
            "layout": state.layout.as_deref().unwrap_or("-"),
            "focused_pane": focused_pane,
            "panes_detail": state.panes,
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
            "  window={} focused={} weight={} pid={} command={:?} cursor={} cursor_visible={} bracketed_paste={} mouse={} layout={} title={:?}",
            pane.window,
            pane.focused,
            pane.weight,
            pane.pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            pane.command,
            native_pane_cursor_label(pane),
            native_pane_bool_label(pane.cursor_visible),
            native_pane_bracketed_paste_label(pane),
            native_pane_mouse_label(pane),
            native_pane_layout_label(pane),
            pane.title
        );
    }
    out.push_str("END\n");
    out
}

fn native_pane_cursor_label(pane: &NativePaneStatus) -> String {
    match (pane.cursor_col, pane.cursor_row) {
        (Some(col), Some(row)) => format!("{col},{row}"),
        _ => "-".to_string(),
    }
}

fn native_pane_bracketed_paste_label(pane: &NativePaneStatus) -> &'static str {
    native_pane_bool_label(pane.bracketed_paste)
}

fn native_pane_mouse_label(pane: &NativePaneStatus) -> String {
    let mut modes = Vec::new();
    if pane.mouse_reporting == Some(true) {
        modes.push("basic");
    }
    if pane.mouse_button_motion == Some(true) {
        modes.push("button-motion");
    }
    if pane.mouse_all_motion == Some(true) {
        modes.push("all-motion");
    }
    if pane.mouse_sgr == Some(true) {
        modes.push("sgr");
    }
    if modes.is_empty() {
        "-".to_string()
    } else {
        modes.join(",")
    }
}

fn native_pane_bool_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "-",
    }
}

fn native_pane_layout_label(pane: &NativePaneStatus) -> String {
    match (
        pane.x,
        pane.y,
        pane.cols,
        pane.rows,
        pane.app_x,
        pane.app_y,
        pane.app_cols,
        pane.app_rows,
    ) {
        (
            Some(x),
            Some(y),
            Some(cols),
            Some(rows),
            Some(app_x),
            Some(app_y),
            Some(app_cols),
            Some(app_rows),
        ) => {
            format!("{x},{y} {cols}x{rows} app={app_x},{app_y} {app_cols}x{app_rows}")
        }
        _ => "-".to_string(),
    }
}

fn native_spawn_wait_text_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    native_spawn_wait_ms_reply(pending, rest, "WAIT_TEXT_MS", false)
}

fn native_spawn_wait_output_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    native_spawn_wait_ms_reply(pending, rest, "WAIT_OUTPUT_MS", true)
}

fn native_spawn_wait_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    verb: &str,
    include_scrollback: bool,
) -> String {
    let Some((target, rest)) = rest.trim_start().split_once(' ') else {
        return format!("ERR {verb} requires window, milliseconds, and needle\n");
    };
    let Some((ms, needle)) = rest.trim_start().split_once(' ') else {
        return format!("ERR {verb} requires window, milliseconds, and needle\n");
    };
    let Ok(ms) = ms.trim().parse::<u64>() else {
        return format!("ERR {verb} milliseconds must be an integer\n");
    };
    if ms == 0 || ms > 60_000 {
        return format!("ERR {verb} milliseconds must be in 1..=60000\n");
    }
    native_spawn_wait_reply(
        pending,
        &format!("{} {}", target.trim(), needle.trim()),
        Duration::from_millis(ms),
        if include_scrollback {
            "WAIT_OUTPUT"
        } else {
            "WAIT_TEXT"
        },
        include_scrollback,
    )
}

fn native_spawn_wait_text_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
) -> String {
    native_spawn_wait_reply(pending, rest, timeout, "WAIT_TEXT", false)
}

fn native_spawn_wait_output_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
) -> String {
    native_spawn_wait_reply(pending, rest, timeout, "WAIT_OUTPUT", true)
}

fn native_spawn_wait_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
    verb: &str,
    include_scrollback: bool,
) -> String {
    let Some((target, needle)) = rest.trim_start().split_once(' ') else {
        return format!("ERR {verb} requires window and needle\n");
    };
    let target = target.trim();
    let needle = needle.trim();
    if target.is_empty() || needle.is_empty() {
        return format!("ERR {verb} requires window and needle\n");
    }
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = match pending.lock() {
            Ok(state) => native_find_pane_target(&state.panes, target).map(|pane| {
                let mut text = pane.text_snapshot.clone().unwrap_or_default();
                if include_scrollback {
                    text.push_str(pane.scrollback_snapshot.as_deref().unwrap_or(""));
                }
                (pane.window.clone(), text)
            }),
            Err(_) => return "ERR registry poisoned\n".to_string(),
        };
        let Some((window, text)) = snapshot else {
            return format!("ERR {verb} no pane matching {target}\n");
        };
        if text.contains(needle) {
            let match_tag = if include_scrollback {
                "MATCH_OUTPUT"
            } else {
                "MATCH_TEXT"
            };
            return format!("{match_tag} window={window} bytes={}\n", text.len());
        }
        if Instant::now() >= deadline {
            return format!(
                "ERR {verb} timeout window={window} needle_bytes={}\n",
                needle.len()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn native_spawn_read_text_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return format!("ERR READ_TEXT no pane matching {}\n", target.trim());
    };
    let text = pane.text_snapshot.as_deref().unwrap_or("");
    format!(
        "TEXT window={} bytes={} cursor={}\n{}END\n",
        pane.window,
        text.len(),
        native_pane_cursor_label(pane),
        text
    )
}

fn native_spawn_read_text_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return format!(
            "{}\n",
            serde_json::json!({ "error": "no pane matching target", "target": target.trim() })
        );
    };
    format!(
        "{}\n",
        serde_json::json!({
            "window": pane.window,
            "text": pane.text_snapshot.as_deref().unwrap_or(""),
            "cursor_col": pane.cursor_col,
            "cursor_row": pane.cursor_row,
        })
    )
}

fn native_spawn_read_scrollback_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return format!("ERR READ_SCROLLBACK no pane matching {}\n", target.trim());
    };
    let text = pane.scrollback_snapshot.as_deref().unwrap_or("");
    format!(
        "SCROLLBACK window={} bytes={}\n{}END\n",
        pane.window,
        text.len(),
        text
    )
}

fn native_spawn_read_scrollback_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return format!(
            "{}\n",
            serde_json::json!({ "error": "no pane matching target", "target": target.trim() })
        );
    };
    format!(
        "{}\n",
        serde_json::json!({
            "window": pane.window,
            "scrollback": pane.scrollback_snapshot.as_deref().unwrap_or(""),
        })
    )
}

fn native_find_pane_target<'a>(
    panes: &'a [NativePaneStatus],
    target: &str,
) -> Option<&'a NativePaneStatus> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    if target == "focused" {
        panes.iter().find(|pane| pane.focused)
    } else {
        panes.iter().find(|pane| pane.window == target)
    }
}

fn native_spawn_session_json_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let focused = state
        .panes
        .iter()
        .find(|pane| pane.focused)
        .map(|pane| pane.window.as_str())
        .unwrap_or("-");
    let panes = state
        .panes
        .iter()
        .enumerate()
        .map(|(index, pane)| {
            serde_json::json!({
                "index": index,
                "window": pane.window,
                "title": pane.title,
                "command": pane.command,
                "weight": pane.weight,
                "focused": pane.focused,
            })
        })
        .collect::<Vec<_>>();
    format!(
        "{}\n",
        serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-native-session",
            "layout": state.layout.as_deref().unwrap_or("-"),
            "focus": focused,
            "panes": panes,
        })
    )
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
            "STATUS" => daemon_status_reply(started, path, panes),
            "STATUS_JSON" => daemon_status_json_reply(started, path, panes),
            "WINDOWS" => windows_reply(),
            "DISPLAYS" => displays_reply(),
            "APPS" => apps_reply(50),
            "APPS_JSON" => apps_json_reply(50),
            "PANES" => panes_reply(panes),
            "PANES_JSON" => panes_json_reply(panes),
            "HELP" | "?" => daemon_help_reply(),
            "HELP_JSON" => daemon_help_json_reply(),
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

fn daemon_help_entries() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("PING", "health", "return PONG"),
        ("STATUS", "inspect", "text daemon status"),
        ("STATUS_JSON", "inspect", "JSON daemon status"),
        ("WINDOWS", "inspect", "list platform windows when supported"),
        (
            "DISPLAYS",
            "inspect",
            "list platform displays when supported",
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
        (
            "SPAWN <argv>",
            "control",
            "spawn a detached tracked daemon process",
        ),
        ("PANES", "inspect", "text tracked pane listing"),
        ("PANES_JSON", "inspect", "JSON tracked pane listing"),
        ("QUIT", "control", "stop the daemon"),
        ("HELP", "help", "show this command catalog"),
        ("HELP_JSON", "help", "show this command catalog as JSON"),
    ]
}

fn daemon_help_reply() -> String {
    let commands = daemon_help_entries()
        .into_iter()
        .map(|(command, _, _)| command)
        .collect::<Vec<_>>()
        .join(" | ");
    format!("{commands}\n")
}

fn daemon_help_json_reply() -> String {
    let commands = daemon_help_entries()
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

fn client_read_timeout_for(cmd: &str) -> Duration {
    let trimmed = cmd.trim_start();
    let Some(rest) = trimmed
        .strip_prefix("WAIT_TEXT_MS ")
        .or_else(|| trimmed.strip_prefix("WAIT_OUTPUT_MS "))
    else {
        return CLIENT_READ_TIMEOUT;
    };
    let mut parts = rest.split_whitespace();
    let _window = parts.next();
    let Some(ms) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
        return CLIENT_READ_TIMEOUT;
    };
    Duration::from_millis(ms).saturating_add(CLIENT_WAIT_TEXT_MARGIN)
}

/// Send a single-line request and return the reply line.
pub fn client_request(path: &Path, cmd: &str) -> Result<String> {
    let mut stream =
        UnixStream::connect(path).map_err(|e| anyhow!("connect {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(client_read_timeout_for(cmd)))?;
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
    fn client_read_timeout_tracks_wait_text_ms() {
        assert_eq!(client_read_timeout_for("PING"), Duration::from_secs(10));
        assert_eq!(
            client_read_timeout_for("WAIT_TEXT focused ready"),
            Duration::from_secs(10)
        );
        assert_eq!(
            client_read_timeout_for("WAIT_TEXT_MS focused 60000 build finished"),
            Duration::from_secs(65)
        );
        assert_eq!(
            client_read_timeout_for("WAIT_OUTPUT_MS focused 60000 build finished"),
            Duration::from_secs(65)
        );
        assert_eq!(
            client_read_timeout_for("WAIT_TEXT_MS focused nope build finished"),
            Duration::from_secs(10)
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

        let value: serde_json::Value =
            serde_json::from_str(&client_request(server.path(), "STATUS_JSON").unwrap()).unwrap();
        assert_eq!(value["pid"], std::process::id());
        assert_eq!(value["panes"], 0);
        assert_eq!(value["focus"], "-");
        assert!(value["sock"]
            .as_str()
            .unwrap()
            .contains("kittwm-test-status"));
    }

    #[test]
    fn standalone_daemon_help_json_lists_commands() {
        let p = std::env::temp_dir().join(format!("kittwm-test-help-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let help = client_request(server.path(), "HELP").unwrap();
        assert!(help.contains("HELP_JSON"), "{help}");
        assert!(help.contains("SPAWN <argv>"), "{help}");
        let value: serde_json::Value =
            serde_json::from_str(&client_request(server.path(), "HELP_JSON").unwrap()).unwrap();
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "SPAWN <argv>" && entry["category"] == "control" }));
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
        let panes_json: serde_json::Value =
            serde_json::from_str(&client_request(server.path(), "PANES_JSON").unwrap()).unwrap();
        assert_eq!(panes_json["panes"], 1);
        assert_eq!(panes_json["focus"], "1");
        assert_eq!(panes_json["panes_detail"][0]["window"], "daemon-1");
        assert_eq!(panes_json["panes_detail"][0]["focused"], true);
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
    fn native_spawn_queue_parses_focus_close_layout_and_rename_commands() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        assert!(
            native_spawn_queue_reply("FOCUS_PANE native-2", &pending).starts_with("FOCUS_QUEUED")
        );
        assert!(native_spawn_queue_reply("FOCUS_NEXT", &pending).starts_with("FOCUS_NEXT_QUEUED"));
        assert!(native_spawn_queue_reply("FOCUS_PREV", &pending).starts_with("FOCUS_PREV_QUEUED"));
        assert!(
            native_spawn_queue_reply("CLOSE_PANE focused", &pending).starts_with("CLOSE_QUEUED")
        );
        assert!(native_spawn_queue_reply("LAYOUT rows", &pending).starts_with("LAYOUT_QUEUED"));
        assert!(
            native_spawn_queue_reply("MOVE_PANE focused last", &pending).starts_with("MOVE_QUEUED")
        );
        assert!(native_spawn_queue_reply("RESIZE_PANE focused +2", &pending)
            .starts_with("RESIZE_QUEUED"));
        assert!(native_spawn_queue_reply("BALANCE_PANES", &pending).starts_with("BALANCE_QUEUED"));
        assert!(
            native_spawn_queue_reply("RENAME_PANE native-2 editor pane", &pending)
                .starts_with("RENAME_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_TEXT focused echo hi", &pending)
                .starts_with("SEND_TEXT_QUEUED")
        );
        assert!(native_spawn_queue_reply("SEND_LINE native-2 pwd", &pending)
            .starts_with("SEND_LINE_QUEUED"));
        assert!(
            native_spawn_queue_reply("SEND_KEY focused ctrl-c", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 page-down", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_BYTES_B64 focused aGkKAA==", &pending)
                .starts_with("SEND_BYTES_B64_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("PASTE_BYTES_B64 focused cGFzdGUK", &pending)
                .starts_with("PASTE_BYTES_B64_QUEUED")
        );
        let manifest = serde_json::json!({
            "layout": "rows",
            "panes": [
                {"title": "shell", "command": "bash", "weight": 2, "focused": false},
                {"title": "logs", "command": "tail -f app.log", "weight": 1, "focused": true}
            ]
        });
        assert!(
            native_spawn_queue_reply(&format!("RESTORE_SESSION_JSON {manifest}"), &pending)
                .starts_with("RESTORE_SESSION_QUEUED")
        );
        assert!(native_spawn_queue_reply("LAYOUT diagonal", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("FOCUS_PANE", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("MOVE_PANE focused diagonal", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("RESIZE_PANE focused nope", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("RENAME_PANE native-2", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_TEXT focused", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_LINE", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_KEY focused nope", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_KEY focused page down", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_BYTES_B64 focused !!!", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("PASTE_BYTES_B64 focused !!!", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("RESTORE_SESSION_JSON {}", &pending).contains("ERR"));
        assert_eq!(
            drain_native_spawn_pending(&pending),
            vec![
                NativePaneCommand::Focus("native-2".to_string()),
                NativePaneCommand::FocusNext,
                NativePaneCommand::FocusPrev,
                NativePaneCommand::Close("focused".to_string()),
                NativePaneCommand::Layout("rows".to_string()),
                NativePaneCommand::Move {
                    window: "focused".to_string(),
                    direction: "last".to_string(),
                },
                NativePaneCommand::Resize {
                    window: "focused".to_string(),
                    delta: 2,
                },
                NativePaneCommand::Balance,
                NativePaneCommand::Rename {
                    window: "native-2".to_string(),
                    title: "editor pane".to_string(),
                },
                NativePaneCommand::SendText {
                    window: "focused".to_string(),
                    text: "echo hi".to_string(),
                    newline: false,
                },
                NativePaneCommand::SendText {
                    window: "native-2".to_string(),
                    text: "pwd".to_string(),
                    newline: true,
                },
                NativePaneCommand::SendBytes {
                    window: "focused".to_string(),
                    bytes: vec![0x03],
                    label: "ctrl-c".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[6~".to_vec(),
                    label: "page-down".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "focused".to_string(),
                    bytes: b"hi\n\0".to_vec(),
                    label: "base64".to_string(),
                },
                NativePaneCommand::PasteBytes {
                    window: "focused".to_string(),
                    bytes: b"paste\n".to_vec(),
                },
                NativePaneCommand::RestoreSession(NativeSessionRestore {
                    layout: Some("rows".to_string()),
                    focus_index: Some(1),
                    panes: vec![
                        NativeSessionRestorePane {
                            title: Some("shell".to_string()),
                            command: "bash".to_string(),
                            weight: 2,
                            focused: false,
                        },
                        NativeSessionRestorePane {
                            title: Some("logs".to_string()),
                            command: "tail -f app.log".to_string(),
                            weight: 1,
                            focused: true,
                        },
                    ],
                })
            ]
        );
    }

    #[test]
    fn native_spawn_queue_read_text_round_trip_over_socket() {
        let p =
            tmp_sock().with_file_name(format!("kittwm-native-read-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let queue = NativeSpawnQueue::bind(p).unwrap();
        queue.update_panes(vec![NativePaneStatus {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            pid: None,
            command: None,
            x: None,
            y: None,
            cols: None,
            rows: None,
            app_x: None,
            app_y: None,
            app_cols: None,
            cursor_col: None,
            cursor_row: None,
            cursor_visible: Some(true),
            bracketed_paste: Some(false),
            mouse_reporting: Some(false),
            mouse_button_motion: Some(false),
            mouse_all_motion: Some(false),
            mouse_sgr: Some(false),
            text_snapshot: Some("ready\n$ ".to_string()),
            scrollback_snapshot: Some("boot\n".to_string()),
            app_rows: None,
        }]);
        let reply = client_request_multi(queue.path(), "READ_TEXT focused").unwrap();
        assert!(reply.starts_with("TEXT window=native-1"), "{reply}");
        assert!(reply.contains("ready\n$ "), "{reply}");

        let scrollback = client_request_multi(queue.path(), "READ_SCROLLBACK focused").unwrap();
        assert!(
            scrollback.starts_with("SCROLLBACK window=native-1 bytes=5"),
            "{scrollback}"
        );
        assert!(scrollback.contains("boot\n"), "{scrollback}");
    }

    #[test]
    fn native_spawn_queue_wait_text_does_not_block_ping() {
        let p = tmp_sock().with_file_name(format!(
            "kittwm-native-concurrent-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        let queue = NativeSpawnQueue::bind(p).unwrap();
        queue.update_panes(vec![NativePaneStatus {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            pid: None,
            command: None,
            x: None,
            y: None,
            cols: None,
            rows: None,
            app_x: None,
            app_y: None,
            app_cols: None,
            cursor_col: None,
            cursor_row: None,
            cursor_visible: Some(true),
            bracketed_paste: Some(false),
            mouse_reporting: Some(false),
            mouse_button_motion: Some(false),
            mouse_all_motion: Some(false),
            mouse_sgr: Some(false),
            text_snapshot: Some("waiting\n".to_string()),
            scrollback_snapshot: Some("previous\n".to_string()),
            app_rows: None,
        }]);
        let path = queue.path().to_path_buf();
        let waiter = std::thread::spawn(move || {
            client_request(&path, "WAIT_TEXT_MS focused 300 never-appears").unwrap()
        });
        std::thread::sleep(Duration::from_millis(50));
        let ping = client_request(queue.path(), "PING").unwrap();
        assert_eq!(ping.trim(), "PONG");
        let waited = waiter.join().unwrap();
        assert!(waited.contains("ERR WAIT_TEXT timeout"), "{waited}");
    }

    #[test]
    fn native_spawn_queue_serves_help_catalogs() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let help = native_spawn_queue_reply("HELP", &pending);
        assert!(help.contains("SPAWN_PTY <cmd>"), "{help}");
        assert!(help.contains("STATUS_JSON"), "{help}");
        assert!(help.contains("PANES_JSON"), "{help}");
        assert!(help.contains("SESSION_JSON"), "{help}");
        assert!(help.contains("FOCUS_NEXT"), "{help}");
        assert!(help.contains("FOCUS_PREV"), "{help}");
        assert!(help.contains("LAYOUT <columns|rows>"), "{help}");
        assert!(help.contains("MOVE_PANE <window|focused>"), "{help}");
        assert!(help.contains("RESIZE_PANE <window|focused>"), "{help}");
        assert!(help.contains("BALANCE_PANES"), "{help}");
        assert!(help.contains("RESTORE_SESSION_JSON <json>"), "{help}");
        assert!(help.contains("RENAME_PANE <window> <title>"), "{help}");
        assert!(help.contains("SEND_TEXT <window|focused> <text>"), "{help}");
        assert!(help.contains("SEND_LINE <window|focused> <text>"), "{help}");
        assert!(help.contains("SEND_KEY <window|focused> <key>"), "{help}");
        assert!(
            help.contains("SEND_BYTES_B64 <window|focused> <base64>"),
            "{help}"
        );
        assert!(
            help.contains("PASTE_BYTES_B64 <window|focused> <base64>"),
            "{help}"
        );
        assert!(help.contains("READ_TEXT <window|focused>"), "{help}");
        assert!(help.contains("READ_TEXT_JSON <window|focused>"), "{help}");
        assert!(help.contains("READ_SCROLLBACK <window|focused>"), "{help}");
        assert!(
            help.contains("READ_SCROLLBACK_JSON <window|focused>"),
            "{help}"
        );
        assert!(
            help.contains("WAIT_TEXT <window|focused> <needle>"),
            "{help}"
        );
        assert!(
            help.contains("WAIT_TEXT_MS <window|focused> <ms> <needle>"),
            "{help}"
        );
        assert!(
            help.contains("WAIT_OUTPUT <window|focused> <needle>"),
            "{help}"
        );
        assert!(
            help.contains("WAIT_OUTPUT_MS <window|focused> <ms> <needle>"),
            "{help}"
        );
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
                weight: 1,
                pid: Some(101),
                command: Some("/bin/sh".to_string()),
                x: Some(0),
                y: Some(0),
                cols: Some(40),
                rows: Some(24),
                app_x: Some(0),
                app_y: Some(1),
                app_cols: Some(40),
                cursor_col: Some(4),
                cursor_row: Some(1),
                cursor_visible: Some(true),
                bracketed_paste: Some(true),
                mouse_reporting: Some(true),
                mouse_button_motion: Some(true),
                mouse_all_motion: Some(false),
                mouse_sgr: Some(true),
                text_snapshot: Some("shell line\n".to_string()),
                scrollback_snapshot: Some("shell history\n".to_string()),
                app_rows: Some(23),
            },
            NativePaneStatus {
                window: "native-2".to_string(),
                title: "htop".to_string(),
                focused: true,
                weight: 3,
                pid: Some(202),
                command: Some("htop".to_string()),
                x: Some(40),
                y: Some(0),
                cols: Some(80),
                rows: Some(24),
                app_x: Some(40),
                app_y: Some(1),
                app_cols: Some(80),
                cursor_col: Some(12),
                cursor_row: Some(2),
                cursor_visible: Some(false),
                bracketed_paste: Some(false),
                mouse_reporting: Some(false),
                mouse_button_motion: Some(false),
                mouse_all_motion: Some(false),
                mouse_sgr: Some(false),
                text_snapshot: Some("htop line\nsecond\n".to_string()),
                scrollback_snapshot: Some("htop history\n".to_string()),
                app_rows: Some(23),
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
            panes.contains("window=native-1 focused=false weight=1 pid=101 command=Some(\"/bin/sh\") cursor=4,1 cursor_visible=on bracketed_paste=on mouse=basic,button-motion,sgr layout=0,0 40x24 app=0,1 40x23 title=\"shell\""),
            "{panes}"
        );
        assert!(
            panes.contains("window=native-2 focused=true weight=3 pid=202 command=Some(\"htop\") cursor=12,2 cursor_visible=off bracketed_paste=off mouse=- layout=40,0 80x24 app=40,1 80x23 title=\"htop\""),
            "{panes}"
        );
        let status_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("STATUS_JSON", &pending)).unwrap();
        assert_eq!(status_json["pending"], 0);
        assert_eq!(status_json["panes"], 2);
        assert_eq!(status_json["focus"], "native-2");
        assert_eq!(status_json["layout"], "rows");
        assert_eq!(status_json["focused_pane"]["window"], "native-2");
        assert_eq!(status_json["focused_pane"]["weight"], 3);
        assert_eq!(status_json["focused_pane"]["pid"], 202);
        assert_eq!(status_json["focused_pane"]["command"], "htop");
        assert_eq!(status_json["focused_pane"]["app_cols"], 80);
        assert_eq!(status_json["focused_pane"]["cursor_col"], 12);
        assert_eq!(status_json["focused_pane"]["cursor_row"], 2);
        assert_eq!(status_json["focused_pane"]["cursor_visible"], false);
        assert_eq!(status_json["focused_pane"]["bracketed_paste"], false);
        assert_eq!(status_json["panes_detail"].as_array().unwrap().len(), 2);
        let panes_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("PANES_JSON", &pending)).unwrap();
        assert_eq!(panes_json["panes_detail"].as_array().unwrap().len(), 2);
        assert_eq!(panes_json["panes_detail"][1]["window"], "native-2");
        assert_eq!(panes_json["panes_detail"][1]["focused"], true);
        assert_eq!(panes_json["panes_detail"][1]["weight"], 3);
        assert_eq!(panes_json["panes_detail"][1]["x"], 40);
        assert_eq!(panes_json["panes_detail"][1]["app_cols"], 80);
        assert_eq!(panes_json["panes_detail"][1]["cursor_col"], 12);
        assert_eq!(panes_json["panes_detail"][1]["cursor_row"], 2);
        assert_eq!(panes_json["panes_detail"][0]["cursor_visible"], true);
        assert_eq!(panes_json["panes_detail"][0]["bracketed_paste"], true);
        assert_eq!(panes_json["panes_detail"][0]["mouse_reporting"], true);
        assert_eq!(panes_json["panes_detail"][0]["mouse_button_motion"], true);
        assert_eq!(panes_json["panes_detail"][0]["mouse_all_motion"], false);
        assert_eq!(panes_json["panes_detail"][0]["mouse_sgr"], true);
        assert!(panes_json["panes_detail"][1].get("text_snapshot").is_none());

        let session_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("SESSION_JSON", &pending)).unwrap();
        assert_eq!(session_json["schema_version"], 1);
        assert_eq!(session_json["kind"], "kittwm-native-session");
        assert_eq!(session_json["layout"], "rows");
        assert_eq!(session_json["focus"], "native-2");
        assert_eq!(session_json["panes"].as_array().unwrap().len(), 2);
        assert_eq!(session_json["panes"][0]["index"], 0);
        assert_eq!(session_json["panes"][0]["command"], "/bin/sh");
        assert_eq!(session_json["panes"][1]["weight"], 3);
        assert!(session_json["panes"][1].get("pid").is_none());
        assert!(session_json["panes"][1].get("x").is_none());
        assert!(session_json["panes"][1].get("text_snapshot").is_none());

        let text = native_spawn_queue_reply("READ_TEXT focused", &pending);
        assert!(
            text.starts_with("TEXT window=native-2 bytes=17 cursor=12,2"),
            "{text}"
        );
        assert!(text.contains("htop line\nsecond\nEND\n"), "{text}");
        let text_json: serde_json::Value = serde_json::from_str(&native_spawn_queue_reply(
            "READ_TEXT_JSON native-1",
            &pending,
        ))
        .unwrap();
        assert_eq!(text_json["window"], "native-1");
        assert_eq!(text_json["text"], "shell line\n");
        assert_eq!(text_json["cursor_col"], 4);
        assert_eq!(text_json["cursor_row"], 1);
        let scrollback = native_spawn_queue_reply("READ_SCROLLBACK focused", &pending);
        assert!(
            scrollback.starts_with("SCROLLBACK window=native-2 bytes=13"),
            "{scrollback}"
        );
        assert!(scrollback.contains("htop history\nEND\n"), "{scrollback}");
        let scrollback_json: serde_json::Value = serde_json::from_str(&native_spawn_queue_reply(
            "READ_SCROLLBACK_JSON native-1",
            &pending,
        ))
        .unwrap();
        assert_eq!(scrollback_json["window"], "native-1");
        assert_eq!(scrollback_json["scrollback"], "shell history\n");
        assert_eq!(
            native_spawn_wait_text_reply(&pending, "focused second", Duration::from_millis(1))
                .trim(),
            "MATCH_TEXT window=native-2 bytes=17"
        );
        assert_eq!(
            native_spawn_wait_text_ms_reply(&pending, "focused 10 second").trim(),
            "MATCH_TEXT window=native-2 bytes=17"
        );
        assert_eq!(
            native_spawn_wait_output_reply(&pending, "focused history", Duration::from_millis(1))
                .trim(),
            "MATCH_OUTPUT window=native-2 bytes=30"
        );
        assert!(native_spawn_wait_text_reply(
            &pending,
            "focused history",
            Duration::from_millis(1)
        )
        .contains("ERR WAIT_TEXT timeout"));
        assert_eq!(
            native_spawn_wait_output_ms_reply(&pending, "focused 10 history").trim(),
            "MATCH_OUTPUT window=native-2 bytes=30"
        );
        assert!(
            native_spawn_wait_text_ms_reply(&pending, "focused nope second")
                .contains("ERR WAIT_TEXT_MS milliseconds")
        );
        assert!(
            native_spawn_wait_text_reply(&pending, "missing nope", Duration::from_millis(1))
                .contains("ERR WAIT_TEXT no pane")
        );
        assert!(
            native_spawn_wait_text_reply(&pending, "focused absent", Duration::from_millis(1))
                .contains("ERR WAIT_TEXT timeout")
        );
        assert!(native_spawn_queue_reply("READ_TEXT missing", &pending).contains("ERR"));
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
    stream.set_read_timeout(Some(client_read_timeout_for(cmd)))?;
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
            && !first.starts_with("TEXT ")
            && !first.starts_with("SCROLLBACK ")
        {
            break;
        }
    }
    Ok(out)
}

fn daemon_status_reply(started: Instant, path: &Path, panes: &SharedPanes) -> String {
    let snapshot = panes.lock().ok();
    format!(
        "pid={} uptime_s={} sock={} panes={} focus={}\n",
        std::process::id(),
        started.elapsed().as_secs(),
        path.display(),
        snapshot.as_ref().map(|p| p.panes.len()).unwrap_or(0),
        snapshot
            .and_then(|p| p.focused)
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string())
    )
}

fn daemon_status_json_reply(started: Instant, path: &Path, panes: &SharedPanes) -> String {
    let Ok(registry) = panes.lock() else {
        return "{\"error\":\"PANES registry poisoned\"}\n".to_string();
    };
    format!(
        "{}\n",
        serde_json::json!({
            "pid": std::process::id(),
            "uptime_s": started.elapsed().as_secs(),
            "sock": path.display().to_string(),
            "panes": registry.panes.len(),
            "focus": registry.focused.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string()),
        })
    )
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

fn panes_json_reply(panes: &SharedPanes) -> String {
    let Ok(registry) = panes.lock() else {
        return "{\"error\":\"PANES registry poisoned\"}\n".to_string();
    };
    format!(
        "{}\n",
        serde_json::json!({
            "panes": registry.panes.len(),
            "focus": registry.focused.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string()),
            "panes_detail": registry.panes,
        })
    )
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
