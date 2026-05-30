//! Minimal Unix-socket daemon protocol for kittwm.
//!
//! Single-line text requests; reply is one line. RAII guard removes the
//! socket file on drop. The server runs an accept loop on a worker
//! thread and exits when the main thread drops the [`DaemonServer`].

use anyhow::{anyhow, Result};
use base64::Engine;
use kittui_wm::native::SurfaceEvent;
use kittwm_sdk::{
    ActionKind, ComponentAction, ComponentNode, ComponentRole, ComponentState, ComponentValue,
    SemanticComponentId, SemanticSurfaceSnapshot,
};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as FmtWrite;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_WAIT_TEXT_MARGIN: Duration = Duration::from_secs(5);
const CLIENT_EVENTS_DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_EVENTS_MAX_TIMEOUT: Duration = Duration::from_secs(60);
const CLIENT_EVENTS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const NATIVE_EVENT_SCHEMA_VERSION: u8 = 1;
const NATIVE_EVENT_BACKLOG_LIMIT: usize = 512;
const NATIVE_CHROME_TOP_BAR_ROWS: u16 = 1;

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
    user_socket_path(&user)
}

fn user_socket_path(user: &str) -> PathBuf {
    PathBuf::from(user_socket_path_string(user))
}

fn user_socket_path_string(user: &str) -> String {
    let mut path = String::with_capacity("/tmp/kittwm-.sock".len() + user.len());
    path.push_str("/tmp/kittwm-");
    path.push_str(user);
    path.push_str(".sock");
    path
}

fn display_id_socket_path(id: &str) -> PathBuf {
    PathBuf::from(display_id_socket_path_string(id))
}

fn display_id_socket_path_string(id: &str) -> String {
    let mut path = String::with_capacity("/tmp/kittui-wm-.sock".len() + id.len());
    path.push_str("/tmp/kittui-wm-");
    path.push_str(id);
    path.push_str(".sock");
    path
}

/// Convert a DISPLAY-like token into a socket path.
pub fn display_to_socket_path(display: &str) -> PathBuf {
    if let Some(id) = display.strip_prefix(':') {
        let id = id.split('.').next().unwrap_or(id);
        display_id_socket_path(id)
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

fn u32_decimal_len(value: u32) -> usize {
    u64_decimal_len(value as u64)
}

fn usize_decimal_len(value: usize) -> usize {
    u64_decimal_len(value as u64)
}

fn u64_decimal_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 10 {
        value /= 10;
        len += 1;
    }
    len
}

fn tracked_pane_window(pane_id: u32) -> String {
    let mut window = String::with_capacity("daemon-".len() + u32_decimal_len(pane_id));
    write!(window, "daemon-{pane_id}").expect("write to string");
    window
}

fn tracked_pane_layout(pane_id: u32) -> String {
    let mut layout = String::with_capacity("tile:".len() + u32_decimal_len(pane_id));
    write!(layout, "tile:{pane_id}").expect("write to string");
    layout
}

fn tab_pair_arg(first: &str, second: &str) -> String {
    let mut arg = String::with_capacity(first.len() + 1 + second.len());
    arg.push_str(first);
    arg.push('\t');
    arg.push_str(second);
    arg
}

fn i16_decimal_len(value: i16) -> usize {
    if value < 0 {
        1 + u32_decimal_len(value.unsigned_abs() as u32)
    } else {
        u32_decimal_len(value as u32)
    }
}

fn tab_i16_arg(first: &str, second: i16) -> String {
    let mut arg = String::with_capacity(first.len() + 1 + i16_decimal_len(second));
    arg.push_str(first);
    write!(arg, "\t{second}").expect("write to string");
    arg
}

fn tab_i16_pair_arg(first: &str, second: i16, third: i16) -> String {
    let mut arg = String::with_capacity(
        first.len() + 1 + i16_decimal_len(second) + 1 + i16_decimal_len(third),
    );
    arg.push_str(first);
    write!(arg, "\t{second}\t{third}").expect("write to string");
    arg
}

fn space_pair_arg(first: &str, second: &str) -> String {
    let mut arg = String::with_capacity(first.len() + 1 + second.len());
    arg.push_str(first);
    arg.push(' ');
    arg.push_str(second);
    arg
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
            window: tracked_pane_window(pane_id),
            pid,
            argv: argv.to_string(),
            layout: tracked_pane_layout(pane_id),
            focused: true,
        };
        self.panes.push(pane.clone());
        pane
    }
}

type SharedPanes = Arc<Mutex<PaneRegistry>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeFramePresented {
    pub renderer: String,
    pub format: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub app_x: Option<u16>,
    pub app_y: Option<u16>,
    pub app_cols: Option<u16>,
    pub app_rows: Option<u16>,
    pub uploaded: bool,
    pub skipped_upload: bool,
    pub changed_tiles: Option<u32>,
    pub total_tiles: Option<u32>,
    pub upload_bytes: Option<usize>,
    pub placement_bytes: Option<usize>,
    pub embed_bytes: Option<usize>,
    pub elapsed_us: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct NativeDirtyFrameStatus {
    pub changed_tiles: u32,
    pub total_tiles: u32,
    pub changed_fraction: f32,
    pub skipped_upload: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct NativePaneStatus {
    pub window: String,
    pub title: String,
    pub focused: bool,
    pub weight: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_index: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_top: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floating_dx: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floating_dy: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floating_moved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_draggable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_drag_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_drag_col: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_drag_row: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_drag_active: Option<bool>,
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
    pub application_cursor_keys: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_reporting: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_button_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_all_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_sgr: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty_frame: Option<NativeDirtyFrameStatus>,
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
    SplitPane {
        window: String,
        axis: String,
        command: String,
    },
    Move {
        window: String,
        direction: String,
    },
    Nudge {
        window: String,
        dx: i16,
        dy: i16,
    },
    ResetOffset {
        window: String,
    },
    ResetAllOffsets,
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
    SendMouse {
        window: String,
        event: String,
        col: u16,
        row: u16,
    },
    RestoreSession(NativeSessionRestore),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeChromeReservationConfig {
    #[serde(default = "default_native_top_bar_rows")]
    pub top_bar_rows: u16,
    #[serde(default)]
    pub bottom_bar_rows: u16,
    #[serde(default)]
    pub left_cols: u16,
    #[serde(default)]
    pub right_cols: u16,
    #[serde(default)]
    pub gap_cols: u16,
    #[serde(default)]
    pub gap_rows: u16,
    #[serde(default)]
    pub owner: Option<String>,
}

impl Default for NativeChromeReservationConfig {
    fn default() -> Self {
        Self {
            top_bar_rows: default_native_top_bar_rows(),
            bottom_bar_rows: 0,
            left_cols: 0,
            right_cols: 0,
            gap_cols: 0,
            gap_rows: 0,
            owner: None,
        }
    }
}

fn default_native_top_bar_rows() -> u16 {
    NATIVE_CHROME_TOP_BAR_ROWS
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
    pub floating_dx: Option<i16>,
    pub floating_dy: Option<i16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeClipboardCache {
    source_window: String,
    selection: String,
    payload_base64: String,
    payload_bytes: usize,
    at_ms: u128,
    seq: u64,
}

#[derive(Default, Debug)]
struct NativeSpawnQueueState {
    pending: Vec<NativePaneCommand>,
    panes: Vec<NativePaneStatus>,
    layout: Option<String>,
    events: VecDeque<serde_json::Value>,
    next_event_seq: u64,
    semantic_snapshots: HashMap<String, SemanticSurfaceSnapshot>,
    clipboard: Option<NativeClipboardCache>,
    chrome_reservation: NativeChromeReservationConfig,
    workspace: Option<String>,
}

/// In-process socket queue used by the live native PTY session.
pub struct NativeSpawnQueue {
    path: PathBuf,
    quit: Arc<AtomicBool>,
    pending: Arc<Mutex<NativeSpawnQueueState>>,
    accept_thread: Option<JoinHandle<()>>,
}

fn active_socket_collision_message(path: &Path, owner: &str) -> String {
    let socket = path.display().to_string();
    let mut message = String::with_capacity(
        "another  is already listening on \nhelp: inspect the active session with `kittwm --socket  --status` or `kittwm --socket  --panes`\nhelp: stop it with `kittwm stop` for the default socket, or `kittwm --socket  stop` for this path\nhelp: start a separate session with `KITTWM_SOCKET=/tmp/kittwm-<name>.sock kittwm`\nnote: stale socket files are removed automatically; this socket answered PING".len()
            + owner.len()
            + socket.len() * 4,
    );
    message.push_str("another ");
    message.push_str(owner);
    message.push_str(" is already listening on ");
    message.push_str(&socket);
    message.push_str("\nhelp: inspect the active session with `kittwm --socket ");
    message.push_str(&socket);
    message.push_str(" --status` or `kittwm --socket ");
    message.push_str(&socket);
    message.push_str(" --panes`");
    message.push_str(
        "\nhelp: stop it with `kittwm stop` for the default socket, or `kittwm --socket ",
    );
    message.push_str(&socket);
    message.push_str(" stop` for this path");
    message.push_str(
        "\nhelp: start a separate session with `KITTWM_SOCKET=/tmp/kittwm-<name>.sock kittwm`",
    );
    message.push_str(
        "\nnote: stale socket files are removed automatically; this socket answered PING",
    );
    message
}

fn cleanup_stale_socket_for_bind(path: &Path, owner: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    match client_request(path, "PING") {
        Ok(reply) if reply.trim() == "PONG" => Err(anyhow::Error::msg(
            active_socket_collision_message(path, owner),
        )),
        _ => std::fs::remove_file(path)
            .map_err(|e| anyhow!("remove stale {owner} socket {}: {e}", path.display())),
    }
}

impl NativeSpawnQueue {
    /// Bind a socket that accepts `SPAWN_PTY <cmd>` requests.
    pub fn bind(path: PathBuf) -> Result<Self> {
        cleanup_stale_socket_for_bind(&path, "native spawn queue")?;
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

    /// Publish a live native pane snapshot for STATUS/PANES/EVENTS requests.
    pub fn update_panes(&self, panes: Vec<NativePaneStatus>) {
        if let Ok(mut state) = self.pending.lock() {
            publish_native_pane_events(&mut state, panes);
        }
    }

    /// Publish drained native surface side-effect events for EVENTS requests.
    pub fn publish_surface_events(&self, window: impl Into<String>, events: Vec<SurfaceEvent>) {
        if events.is_empty() {
            return;
        }
        if let Ok(mut state) = self.pending.lock() {
            publish_native_surface_events(&mut state, window.into(), events);
        }
    }

    /// Publish a graphics-frame presentation event for EVENTS requests.
    pub fn publish_frame_presented(&self, window: impl Into<String>, frame: NativeFramePresented) {
        if let Ok(mut state) = self.pending.lock() {
            publish_native_frame_presented_event(&mut state, window.into(), frame);
        }
    }

    /// Publish the live workspace label for STATUS/CHROME/PANES requests.
    pub fn update_workspace(&self, workspace: impl Into<String>) {
        if let Ok(mut state) = self.pending.lock() {
            state.workspace = normalize_workspace_label(Some(workspace.into().as_str()));
        }
    }

    /// Publish the live native pane layout axis for STATUS/EVENTS requests.
    pub fn update_layout(&self, layout: impl Into<String>) {
        if let Ok(mut state) = self.pending.lock() {
            publish_native_layout_event(&mut state, layout.into());
        }
    }

    /// Current drawable-area reservation requested by chrome/bar apps.
    pub fn chrome_reservation(&self) -> NativeChromeReservationConfig {
        self.pending
            .lock()
            .map(|state| state.chrome_reservation.clone())
            .unwrap_or_default()
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
    let old_pending = state.pending.len();
    let drained = std::mem::take(&mut state.pending);
    push_native_pending_status_event(&mut state, old_pending);
    drained
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
    let cmd = line.trim();
    if cmd == "EVENTS" || cmd.starts_with("EVENTS ") {
        return stream_native_events(stream, pending, cmd);
    }
    let reply = native_spawn_queue_reply(cmd, pending);
    stream.write_all(reply.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn stream_native_events(
    mut stream: UnixStream,
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    cmd: &str,
) -> Result<()> {
    let timeout = parse_events_timeout(cmd);
    let started = Instant::now();
    let mut next_seq = match pending.lock() {
        Ok(mut state) => {
            let seq = state.next_event_seq;
            state.next_event_seq = state.next_event_seq.saturating_add(1);
            let event = native_status_event(&state, seq);
            stream.write_all(event.to_string().as_bytes())?;
            stream.write_all(b"\n")?;
            state.next_event_seq
        }
        Err(_) => {
            stream.write_all(b"{\"error\":\"registry poisoned\"}\nEND\n")?;
            stream.flush()?;
            return Ok(());
        }
    };
    stream.flush()?;

    while started.elapsed() < timeout {
        let events = match pending.lock() {
            Ok(state) => state
                .events
                .iter()
                .filter(|event| event["seq"].as_u64().unwrap_or(0) >= next_seq)
                .cloned()
                .collect::<Vec<_>>(),
            Err(_) => vec![serde_json::json!({ "error": "registry poisoned" })],
        };
        for event in events {
            if let Some(seq) = event["seq"].as_u64() {
                next_seq = seq.saturating_add(1);
            }
            stream.write_all(event.to_string().as_bytes())?;
            stream.write_all(b"\n")?;
        }
        stream.flush()?;
        std::thread::sleep(CLIENT_EVENTS_POLL_INTERVAL);
    }
    stream.write_all(b"END\n")?;
    stream.flush()?;
    Ok(())
}

fn parse_events_timeout(cmd: &str) -> Duration {
    let ms = cmd
        .strip_prefix("EVENTS")
        .unwrap_or("")
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(CLIENT_EVENTS_DEFAULT_TIMEOUT.as_millis() as u64);
    Duration::from_millis(ms.clamp(1, CLIENT_EVENTS_MAX_TIMEOUT.as_millis() as u64))
}

fn native_spawn_queue_reply(cmd: &str, pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    let cmd = cmd.trim();
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
        if !matches!(axis.as_str(), "columns" | "rows" | "grid") {
            return "ERR LAYOUT expects columns|rows|grid\n".to_string();
        }
        return queue_native_pane_command(
            pending,
            &axis,
            "LAYOUT requires columns|rows|grid",
            NativePaneCommand::Layout,
            "LAYOUT_QUEUED",
        );
    }
    if let Some(rest) = cmd.strip_prefix("SPLIT_PANE ") {
        return queue_native_split_pane(pending, rest);
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
            &tab_pair_arg(window, &direction),
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
    if let Some(rest) = cmd.strip_prefix("NUDGE_PANE ") {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 3 {
            return "ERR NUDGE_PANE expects <window|focused> <dx> <dy>\n".to_string();
        }
        let window = parts[0].trim();
        let dx = match parts[1].parse::<i16>() {
            Ok(value) => value,
            Err(_) => return "ERR NUDGE_PANE expects <window|focused> <dx> <dy>\n".to_string(),
        };
        let dy = match parts[2].parse::<i16>() {
            Ok(value) => value,
            Err(_) => return "ERR NUDGE_PANE expects <window|focused> <dx> <dy>\n".to_string(),
        };
        if window.is_empty() || (dx == 0 && dy == 0) {
            return "ERR NUDGE_PANE expects <window|focused> <dx> <dy>\n".to_string();
        }
        return queue_native_pane_command(
            pending,
            &tab_i16_pair_arg(window, dx, dy),
            "NUDGE_PANE requires window and dx/dy",
            |arg| {
                let mut parts = arg.split('\t');
                NativePaneCommand::Nudge {
                    window: parts.next().unwrap_or("").to_string(),
                    dx: parts
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                    dy: parts
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                }
            },
            "NUDGE_QUEUED",
        );
    }
    if let Some(rest) = cmd.strip_prefix("RESET_PANE_OFFSET ") {
        let window = rest.trim();
        if window.is_empty() || window.split_whitespace().count() != 1 {
            return "ERR RESET_PANE_OFFSET expects <window|focused>\n".to_string();
        }
        return queue_native_pane_command(
            pending,
            window,
            "RESET_PANE_OFFSET requires window",
            |arg| NativePaneCommand::ResetOffset { window: arg },
            "RESET_OFFSET_QUEUED",
        );
    }
    if cmd == "RESET_ALL_PANE_OFFSETS" {
        return queue_native_pane_action(
            pending,
            NativePaneCommand::ResetAllOffsets,
            "RESET_ALL_OFFSETS_QUEUED",
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
            &tab_i16_arg(window, delta),
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
            &tab_pair_arg(window, title),
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
    if let Some(rest) = cmd.strip_prefix("SEND_MOUSE ") {
        return queue_native_send_mouse(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("SEND_BYTES_B64 ") {
        return queue_native_send_bytes_b64(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("PASTE_BYTES_B64 ") {
        return queue_native_paste_bytes_b64(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_OUTPUT_JSON_MS ") {
        return native_spawn_wait_output_json_ms_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_OUTPUT_JSON ") {
        return native_spawn_wait_output_json_reply(pending, rest, Duration::from_secs(5));
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_OUTPUT_MS ") {
        return native_spawn_wait_output_ms_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_OUTPUT ") {
        return native_spawn_wait_output_reply(pending, rest, Duration::from_secs(5));
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_TEXT_JSON_MS ") {
        return native_spawn_wait_text_json_ms_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("WAIT_TEXT_JSON ") {
        return native_spawn_wait_text_json_reply(pending, rest, Duration::from_secs(5));
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
    if let Some(target) = cmd.strip_prefix("SEMANTIC_SNAPSHOT ") {
        return native_spawn_semantic_snapshot_reply(pending, target);
    }
    if let Some(rest) = cmd.strip_prefix("SEMANTIC_PUBLISH ") {
        return native_spawn_semantic_publish_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("SEMANTIC_ACTION ") {
        return native_spawn_semantic_action_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("SEMANTIC_FOCUS ") {
        return native_spawn_semantic_focus_reply(pending, rest);
    }
    if let Some(rest) = cmd.strip_prefix("RESERVE_CHROME_JSON ") {
        return native_reserve_chrome_json_reply(pending, rest);
    }
    match cmd {
        "PING" => "PONG\n".to_string(),
        "STATUS" => native_spawn_status_reply(pending),
        "STATUS_JSON" => native_spawn_status_json_reply(pending),
        "CHROME_JSON" => native_chrome_json_reply(pending),
        "SHORTCUTS_JSON" => native_shortcuts_json_reply(),
        "CLIPBOARD_JSON" => native_clipboard_json_reply(pending),
        "PANES" => native_spawn_panes_reply(pending),
        "PANES_JSON" => native_spawn_panes_json_reply(pending),
        "SESSION_JSON" => native_spawn_session_json_reply(pending),
        "APPS" => apps_reply(50),
        "APPS_JSON" => apps_json_reply(50),
        "APPS_FIRST" => apps_first_reply("", false),
        "APPS_LAUNCH_FIRST" => apps_first_reply("", true),
        "HELP" | "?" => native_spawn_help_reply(),
        "HELP_JSON" => native_spawn_help_json_reply(),
        _ => "ERR expected SPAWN_PTY <cmd> | FOCUS_PANE <window> | FOCUS_NEXT | FOCUS_PREV | CLOSE_PANE <window|focused> | LAYOUT <columns|rows|grid> | MOVE_PANE <window|focused> <left|right|up|down|first|last> | NUDGE_PANE <window|focused> <dx> <dy> | RESET_PANE_OFFSET <window|focused> | RESET_ALL_PANE_OFFSETS | RESIZE_PANE <window|focused> <grow|shrink|+N|-N> | BALANCE_PANES | SPLIT_PANE <window|focused> <columns|rows|grid> <cmd> | RESTORE_SESSION_JSON <json> | RENAME_PANE <window> <title> | RESERVE_CHROME_JSON <json> | SEND_TEXT <window|focused> <text> | SEND_LINE <window|focused> <text> | SEND_KEY <window|focused> <key> | SEND_MOUSE <window|focused> <event> <col> <row> | SEND_BYTES_B64 <window|focused> <base64> | PASTE_BYTES_B64 <window|focused> <base64> | READ_TEXT <window|focused> | READ_TEXT_JSON <window|focused> | READ_SCROLLBACK <window|focused> | READ_SCROLLBACK_JSON <window|focused> | SEMANTIC_SNAPSHOT <window|focused> | SEMANTIC_PUBLISH <window|focused> <snapshot-json> | SEMANTIC_ACTION <window|focused> <component> <action> <json> | SEMANTIC_FOCUS <window|focused> <component> | WAIT_TEXT <window|focused> <needle> | WAIT_TEXT_MS <window|focused> <ms> <needle> | WAIT_TEXT_JSON <window|focused> <needle> | WAIT_TEXT_JSON_MS <window|focused> <ms> <needle> | WAIT_OUTPUT <window|focused> <needle> | WAIT_OUTPUT_MS <window|focused> <ms> <needle> | WAIT_OUTPUT_JSON <window|focused> <needle> | WAIT_OUTPUT_JSON_MS <window|focused> <ms> <needle> | SESSION_JSON | STATUS | STATUS_JSON | CHROME_JSON | SHORTCUTS_JSON | CLIPBOARD_JSON | PANES | PANES_JSON | EVENTS [ms] | APPS | APPS_JSON | APPS_FIRST <query> | APPS_LAUNCH_FIRST <query> | PING | HELP | HELP_JSON\n"
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
            "EVENTS [ms]",
            "events",
            "stream JSON status/pane/focus/layout events until timeout",
        ),
        (
            "CHROME_JSON",
            "inspect",
            "JSON native chrome reservation metadata",
        ),
        ("SHORTCUTS_JSON", "inspect", "JSON native shortcut catalog"),
        (
            "CLIPBOARD_JSON",
            "clipboard",
            "policy-gated read of cached OSC52 clipboard writes",
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
            "LAYOUT <columns|rows|grid>",
            "control",
            "switch native pane layout axis",
        ),
        (
            "MOVE_PANE <window|focused> <left|right|up|down|first|last>",
            "control",
            "move a native pane within the layout order",
        ),
        (
            "NUDGE_PANE <window|focused> <dx> <dy>",
            "control",
            "nudge a floating pane by cell deltas",
        ),
        (
            "RESET_PANE_OFFSET <window|focused>",
            "control",
            "reset a floating pane to its generated position",
        ),
        (
            "RESET_ALL_PANE_OFFSETS",
            "control",
            "reset all floating panes to generated positions",
        ),
        (
            "SPLIT_PANE <window|focused> <columns|rows|grid> <cmd>",
            "control",
            "set split axis, spawn command, and place it next to target pane",
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
            "RESERVE_CHROME_JSON <json>",
            "control",
            "reserve a native chrome surface from a JSON request",
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
            "send a named key sequence to a native pane; keys: enter|return|tab|shift-tab|backtab|escape|esc|backspace|bs|insert|ins|shift-insert|alt-insert|ctrl-insert|delete|del|shift-delete|alt-delete|ctrl-delete|left|arrow-left|right|arrow-right|up|arrow-up|down|arrow-down|shift/alt/ctrl arrows|shift/alt/ctrl insert/delete|shift/alt/ctrl home/end/page-up/page-down|f5..f12|ctrl-a..ctrl-z",
        ),
        (
            "SEND_MOUSE <window|focused> <event> <col> <row>",
            "control",
            "send an SGR mouse event to a native pane when mouse reporting is enabled",
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
            "SEMANTIC_SNAPSHOT <window|focused>",
            "semantic",
            "read a semantic component snapshot for a native pane",
        ),
        (
            "SEMANTIC_PUBLISH <window|focused> <snapshot-json>",
            "semantic",
            "publish the latest semantic component snapshot for a native pane",
        ),
        (
            "SEMANTIC_ACTION <window|focused> <component> <action> <json>",
            "semantic",
            "invoke a semantic component action when supported",
        ),
        (
            "SEMANTIC_FOCUS <window|focused> <component>",
            "semantic",
            "focus a semantic component when supported",
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
            "WAIT_TEXT_JSON <window|focused> <needle>",
            "automation",
            "wait for pane text and return JSON match metadata",
        ),
        (
            "WAIT_TEXT_JSON_MS <window|focused> <ms> <needle>",
            "automation",
            "wait for pane text with explicit timeout and return JSON match metadata",
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
        (
            "WAIT_OUTPUT_JSON <window|focused> <needle>",
            "automation",
            "wait for pane text/scrollback and return JSON match metadata",
        ),
        (
            "WAIT_OUTPUT_JSON_MS <window|focused> <ms> <needle>",
            "automation",
            "wait for pane text/scrollback with explicit timeout and return JSON match metadata",
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

fn native_clipboard_json_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    native_clipboard_json_reply_with_policy(pending, native_clipboard_read_allowed())
}

fn native_clipboard_json_reply_with_policy(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    allowed: bool,
) -> String {
    if !allowed {
        return json_value_line(&serde_json::json!({
            "allowed": false,
            "available": false,
            "policy": "set KITTWM_CLIPBOARD_READ=allow to read cached OSC52 clipboard writes",
        }));
    }
    match pending.lock() {
        Ok(state) => match &state.clipboard {
            Some(clipboard) => json_value_line(&serde_json::json!({
                "allowed": true,
                "available": true,
                "source_window": clipboard.source_window,
                "selection": clipboard.selection,
                "payload_base64": clipboard.payload_base64,
                "payload_bytes": clipboard.payload_bytes,
                "at_ms": clipboard.at_ms,
                "seq": clipboard.seq,
                "source": "osc52-cache",
            })),
            None => json_value_line(&serde_json::json!({
                "allowed": true,
                "available": false,
                "source": "osc52-cache",
            })),
        },
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn native_clipboard_read_allowed() -> bool {
    std::env::var("KITTWM_CLIPBOARD_READ")
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "allow"
            )
        })
        .unwrap_or(false)
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
    let entries = native_spawn_help_entries();
    let mut out = String::with_capacity(entries.len().saturating_mul(96));
    out.push_str("{\"commands\":[");
    for (idx, (command, category, description)) in entries.into_iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"command\":{},\"category\":{},\"description\":{}}}",
            serde_json::to_string(command).unwrap(),
            serde_json::to_string(category).unwrap(),
            serde_json::to_string(description).unwrap()
        );
    }
    out.push_str("]}\n");
    out
}

fn publish_native_surface_events(
    state: &mut NativeSpawnQueueState,
    window: String,
    events: Vec<SurfaceEvent>,
) {
    for event in events {
        match event {
            SurfaceEvent::TitleChanged(title) => push_native_event(
                state,
                "surface_title_changed",
                Some(window.clone()),
                serde_json::json!({ "title": title }),
            ),
            SurfaceEvent::Bell { visual, audible } => push_native_event(
                state,
                "surface_bell",
                Some(window.clone()),
                serde_json::json!({ "visual": visual, "audible": audible }),
            ),
            SurfaceEvent::ClipboardSet {
                selection,
                payload_base64,
            } => {
                let payload_bytes = base64::engine::general_purpose::STANDARD
                    .decode(&payload_base64)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0);
                state.clipboard = Some(NativeClipboardCache {
                    source_window: window.clone(),
                    selection: selection.clone(),
                    payload_base64: payload_base64.clone(),
                    payload_bytes,
                    at_ms: now_unix_ms(),
                    seq: state.next_event_seq,
                });
                push_native_event(
                    state,
                    "surface_clipboard_set",
                    Some(window.clone()),
                    serde_json::json!({ "selection": selection, "payload_base64": payload_base64 }),
                );
            }
            SurfaceEvent::Notification { title, body } => push_native_event(
                state,
                "surface_notification",
                Some(window.clone()),
                serde_json::json!({ "title": title, "body": body }),
            ),
        }
    }
}

fn publish_native_frame_presented_event(
    state: &mut NativeSpawnQueueState,
    window: String,
    frame: NativeFramePresented,
) {
    let mut detail = serde_json::json!({
        "renderer": frame.renderer,
        "format": frame.format,
        "pixel_width": frame.pixel_width,
        "pixel_height": frame.pixel_height,
        "app_bounds": native_bounds_value(frame.app_x, frame.app_y, frame.app_cols, frame.app_rows),
        "uploaded": frame.uploaded,
        "skipped_upload": frame.skipped_upload,
    });
    if let Some(obj) = detail.as_object_mut() {
        if let Some(changed_tiles) = frame.changed_tiles {
            obj.insert(
                "changed_tiles".to_string(),
                serde_json::json!(changed_tiles),
            );
        }
        if let Some(total_tiles) = frame.total_tiles {
            obj.insert("total_tiles".to_string(), serde_json::json!(total_tiles));
        }
        if let Some(upload_bytes) = frame.upload_bytes {
            obj.insert("upload_bytes".to_string(), serde_json::json!(upload_bytes));
        }
        if let Some(placement_bytes) = frame.placement_bytes {
            obj.insert(
                "placement_bytes".to_string(),
                serde_json::json!(placement_bytes),
            );
        }
        if let Some(embed_bytes) = frame.embed_bytes {
            obj.insert("embed_bytes".to_string(), serde_json::json!(embed_bytes));
        }
        if let Some(elapsed_us) = frame.elapsed_us {
            obj.insert("elapsed_us".to_string(), serde_json::json!(elapsed_us));
        }
    }
    push_native_event(state, "pane_frame_presented", Some(window), detail);
}

fn publish_native_layout_event(state: &mut NativeSpawnQueueState, layout: String) {
    if state.layout.as_deref() == Some(layout.as_str()) {
        return;
    }
    let old = state.layout.clone();
    state.layout = Some(layout.clone());
    push_native_event(
        state,
        "layout_changed",
        None,
        serde_json::json!({ "old": old, "layout": layout }),
    );
}

fn publish_native_pane_events(state: &mut NativeSpawnQueueState, panes: Vec<NativePaneStatus>) {
    let old_panes = std::mem::replace(&mut state.panes, panes);
    let old_by_window = old_panes
        .iter()
        .map(|pane| (pane.window.as_str(), pane))
        .collect::<std::collections::BTreeMap<_, _>>();
    let new_by_window = state
        .panes
        .iter()
        .map(|pane| (pane.window.as_str(), pane))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut events = Vec::new();
    for (&window, pane) in &new_by_window {
        match old_by_window.get(window) {
            None => events.push((
                "pane_opened",
                Some(window.to_string()),
                serde_json::json!({ "pane": native_pane_status_value(pane) }),
            )),
            Some(old) if old != pane => {
                if native_pane_geometry(old) != native_pane_geometry(pane) {
                    events.push((
                        "pane_resized",
                        Some(window.to_string()),
                        serde_json::json!({
                            "old": native_pane_geometry_value(old),
                            "new": native_pane_geometry_value(pane),
                        }),
                    ));
                }
                events.push((
                    "pane_changed",
                    Some(window.to_string()),
                    serde_json::json!({ "pane": native_pane_status_value(pane) }),
                ));
            }
            _ => {}
        }
    }
    for &window in old_by_window.keys() {
        if !new_by_window.contains_key(window) {
            events.push((
                "pane_closed",
                Some(window.to_string()),
                serde_json::json!({ "window": window }),
            ));
        }
    }

    let old_focus = old_panes
        .iter()
        .find(|pane| pane.focused)
        .map(|pane| pane.window.clone());
    let new_focus = state
        .panes
        .iter()
        .find(|pane| pane.focused)
        .map(|pane| pane.window.clone());
    if old_focus != new_focus {
        events.push((
            "focus_changed",
            new_focus.clone(),
            serde_json::json!({ "old": old_focus, "focus": new_focus }),
        ));
    }

    for (kind, window, detail) in events {
        push_native_event(state, kind, window, detail);
    }
}

fn push_native_input_event(
    state: &mut NativeSpawnQueueState,
    window: &str,
    input_kind: &'static str,
    extra: serde_json::Value,
) {
    let mut detail = serde_json::json!({ "input": input_kind });
    if let (Some(target), Some(extra_obj)) = (detail.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_obj {
            target.insert(key.clone(), value.clone());
        }
    }
    push_native_event(state, "pane_input_sent", Some(window.to_string()), detail);
}

fn push_native_pending_status_event(state: &mut NativeSpawnQueueState, old_pending: usize) {
    let pending = state.pending.len();
    if pending != old_pending {
        push_native_event(
            state,
            "status_changed",
            None,
            serde_json::json!({ "old_pending": old_pending, "pending": pending }),
        );
    }
}

fn push_native_event(
    state: &mut NativeSpawnQueueState,
    kind: &'static str,
    window: Option<String>,
    detail: serde_json::Value,
) {
    let seq = state.next_event_seq;
    state.next_event_seq = state.next_event_seq.saturating_add(1);
    let event = serde_json::json!({
        "schema_version": NATIVE_EVENT_SCHEMA_VERSION,
        "seq": seq,
        "at_ms": now_unix_ms(),
        "kind": kind,
        "window": window,
        "detail": detail,
    });
    state.events.push_back(event);
    while state.events.len() > NATIVE_EVENT_BACKLOG_LIMIT {
        state.events.pop_front();
    }
}

fn native_status_event(state: &NativeSpawnQueueState, seq: u64) -> serde_json::Value {
    let focused = state.panes.iter().find(|pane| pane.focused);
    let focus_label = focused.map(|pane| pane.window.as_str()).unwrap_or("-");
    serde_json::json!({
        "schema_version": NATIVE_EVENT_SCHEMA_VERSION,
        "seq": seq,
        "at_ms": now_unix_ms(),
        "kind": "status",
        "window": focused.map(|pane| pane.window.as_str()),
        "detail": {
            "pending": state.pending.len(),
            "panes": state.panes.len(),
            "focus": focus_label,
            "layout": state.layout.as_deref().unwrap_or("-"),
            "panes_detail": state.panes.iter().map(native_pane_status_value).collect::<Vec<_>>(),
        },
    })
}

fn native_pane_status_value(pane: &NativePaneStatus) -> serde_json::Value {
    serde_json::to_value(pane).unwrap_or_else(|_| serde_json::json!({ "window": pane.window }))
}

type NativePaneGeometry = (
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
);

fn native_pane_geometry(pane: &NativePaneStatus) -> NativePaneGeometry {
    (
        pane.x,
        pane.y,
        pane.cols,
        pane.rows,
        pane.app_x,
        pane.app_y,
        pane.app_cols,
        pane.app_rows,
    )
}

fn native_pane_geometry_value(pane: &NativePaneStatus) -> serde_json::Value {
    serde_json::json!({
        "bounds": native_bounds_value(pane.x, pane.y, pane.cols, pane.rows),
        "app_bounds": native_bounds_value(pane.app_x, pane.app_y, pane.app_cols, pane.app_rows),
    })
}

fn native_bounds_value(
    x: Option<u16>,
    y: Option<u16>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> serde_json::Value {
    match (x, y, cols, rows) {
        (Some(x), Some(y), Some(cols), Some(rows)) => {
            serde_json::json!({ "x": x, "y": y, "cols": cols, "rows": rows })
        }
        _ => serde_json::Value::Null,
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn queue_action_reply(ok_prefix: &str, count: usize) -> String {
    let mut out =
        String::with_capacity(ok_prefix.len() + " command=\n".len() + usize_decimal_len(count));
    out.push_str(ok_prefix);
    out.push_str(" command=");
    write!(out, "{count}").expect("write to string");
    out.push('\n');
    out
}

fn queue_native_pane_action(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    command: NativePaneCommand,
    ok_prefix: &str,
) -> String {
    match pending.lock() {
        Ok(mut state) => {
            let old_pending = state.pending.len();
            state.pending.push(command);
            push_native_pending_status_event(&mut state, old_pending);
            queue_action_reply(ok_prefix, state.pending.len())
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn queue_command_reply(ok_prefix: &str, count: usize, arg: &str) -> String {
    let mut out = String::with_capacity(
        ok_prefix.len() + " command= arg=\n".len() + usize_decimal_len(count) + arg.len(),
    );
    out.push_str(ok_prefix);
    out.push_str(" command=");
    write!(out, "{count}").expect("write to string");
    out.push_str(" arg=");
    out.push_str(arg);
    out.push('\n');
    out
}

fn queue_empty_error_reply(empty_error: &str) -> String {
    let mut out = String::with_capacity("ERR \n".len() + empty_error.len());
    out.push_str("ERR ");
    out.push_str(empty_error);
    out.push('\n');
    out
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
        return queue_empty_error_reply(empty_error);
    }
    match pending.lock() {
        Ok(mut state) => {
            let old_pending = state.pending.len();
            state.pending.push(build(arg.to_string()));
            push_native_pending_status_event(&mut state, old_pending);
            queue_command_reply(ok_prefix, state.pending.len(), arg)
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn queue_native_split_pane(pending: &Arc<Mutex<NativeSpawnQueueState>>, rest: &str) -> String {
    let Some((window, rest)) = rest.trim_start().split_once(' ') else {
        return "ERR SPLIT_PANE requires window, axis, and command\n".to_string();
    };
    let Some((axis, command)) = rest.trim_start().split_once(' ') else {
        return "ERR SPLIT_PANE requires window, axis, and command\n".to_string();
    };
    let window = window.trim();
    let axis = axis.trim().to_ascii_lowercase();
    let command = command.trim();
    if window.is_empty()
        || command.is_empty()
        || !matches!(axis.as_str(), "columns" | "rows" | "grid")
    {
        return "ERR SPLIT_PANE expects <window|focused> <columns|rows|grid> <cmd>\n".to_string();
    }
    queue_native_pane_action(
        pending,
        NativePaneCommand::SplitPane {
            window: window.to_string(),
            axis,
            command: command.to_string(),
        },
        "SPLIT_PANE_QUEUED",
    )
}

fn restore_session_queued_reply(count: usize) -> String {
    let mut out =
        String::with_capacity("RESTORE_SESSION_QUEUED command=\n".len() + usize_decimal_len(count));
    out.push_str("RESTORE_SESSION_QUEUED command=");
    write!(out, "{count}").expect("write to string");
    out.push('\n');
    out
}

fn restore_session_missing_command_reply(idx: usize) -> String {
    let mut out = String::with_capacity(
        "ERR RESTORE_SESSION_JSON pane  missing command\n".len() + usize_decimal_len(idx),
    );
    out.push_str("ERR RESTORE_SESSION_JSON pane ");
    write!(out, "{idx}").expect("write to string");
    out.push_str(" missing command\n");
    out
}

fn restore_session_invalid_json_reply(err: &serde_json::Error) -> String {
    let err = err.to_string();
    let mut out =
        String::with_capacity("ERR RESTORE_SESSION_JSON invalid json: \n".len() + err.len());
    out.push_str("ERR RESTORE_SESSION_JSON invalid json: ");
    out.push_str(&err);
    out.push('\n');
    out
}

fn queue_native_restore_session(pending: &Arc<Mutex<NativeSpawnQueueState>>, json: &str) -> String {
    let json = json.trim();
    if json.is_empty() {
        return "ERR RESTORE_SESSION_JSON requires json\n".to_string();
    }
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(err) => return restore_session_invalid_json_reply(&err),
    };
    let layout = value
        .get("layout")
        .and_then(|v| v.as_str())
        .filter(|layout| matches!(*layout, "columns" | "rows" | "grid"))
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
            return restore_session_missing_command_reply(idx);
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
            floating_dx: item
                .get("floating_dx")
                .and_then(|v| v.as_i64())
                .map(|v| v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16),
            floating_dy: item
                .get("floating_dy")
                .and_then(|v| v.as_i64())
                .map(|v| v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16),
        });
    }
    let focus_index = panes.iter().position(|pane| pane.focused);
    match pending.lock() {
        Ok(mut state) => {
            let old_pending = state.pending.len();
            state
                .pending
                .push(NativePaneCommand::RestoreSession(NativeSessionRestore {
                    layout,
                    panes,
                    focus_index,
                }));
            push_native_pending_status_event(&mut state, old_pending);
            restore_session_queued_reply(state.pending.len())
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn send_text_queued_reply(prefix: &str, count: usize, window: &str, bytes: usize) -> String {
    let mut out = String::with_capacity(
        prefix.len()
            + " command= window= bytes=\n".len()
            + usize_decimal_len(count)
            + window.len()
            + usize_decimal_len(bytes),
    );
    out.push_str(prefix);
    out.push_str(" command=");
    write!(out, "{count}").expect("write to string");
    out.push_str(" window=");
    out.push_str(window);
    out.push_str(" bytes=");
    write!(out, "{bytes}").expect("write to string");
    out.push('\n');
    out
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
            let old_pending = state.pending.len();
            state.pending.push(NativePaneCommand::SendText {
                window: window.to_string(),
                text: text.to_string(),
                newline,
            });
            push_native_pending_status_event(&mut state, old_pending);
            push_native_input_event(
                &mut state,
                window,
                if newline { "line" } else { "text" },
                serde_json::json!({ "bytes": text.len() + usize::from(newline) }),
            );
            let prefix = if newline {
                "SEND_LINE_QUEUED"
            } else {
                "SEND_TEXT_QUEUED"
            };
            send_text_queued_reply(
                prefix,
                state.pending.len(),
                window,
                text.len() + usize::from(newline),
            )
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn base64_queued_reply(prefix: &str, count: usize, window: &str, bytes: usize) -> String {
    let mut out = String::with_capacity(
        prefix.len()
            + " command= window= bytes=\n".len()
            + usize_decimal_len(count)
            + window.len()
            + usize_decimal_len(bytes),
    );
    out.push_str(prefix);
    out.push_str(" command=");
    write!(out, "{count}").expect("write to string");
    out.push_str(" window=");
    out.push_str(window);
    out.push_str(" bytes=");
    write!(out, "{bytes}").expect("write to string");
    out.push('\n');
    out
}

fn queue_native_send_bytes_b64(pending: &Arc<Mutex<NativeSpawnQueueState>>, rest: &str) -> String {
    let (window, bytes) = match parse_window_base64(rest, "SEND_BYTES_B64") {
        Ok(parsed) => parsed,
        Err(err) => return err,
    };
    match pending.lock() {
        Ok(mut state) => {
            let old_pending = state.pending.len();
            state.pending.push(NativePaneCommand::SendBytes {
                window: window.clone(),
                bytes: bytes.clone(),
                label: "base64".to_string(),
            });
            push_native_pending_status_event(&mut state, old_pending);
            push_native_input_event(
                &mut state,
                &window,
                "bytes",
                serde_json::json!({ "bytes": bytes.len() }),
            );
            base64_queued_reply(
                "SEND_BYTES_B64_QUEUED",
                state.pending.len(),
                &window,
                bytes.len(),
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
            let old_pending = state.pending.len();
            state.pending.push(NativePaneCommand::PasteBytes {
                window: window.clone(),
                bytes: bytes.clone(),
            });
            push_native_pending_status_event(&mut state, old_pending);
            push_native_input_event(
                &mut state,
                &window,
                "paste",
                serde_json::json!({ "bytes": bytes.len() }),
            );
            base64_queued_reply(
                "PASTE_BYTES_B64_QUEUED",
                state.pending.len(),
                &window,
                bytes.len(),
            )
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn window_base64_required_reply(verb: &str) -> String {
    let mut out = String::with_capacity("ERR  requires window and base64\n".len() + verb.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" requires window and base64\n");
    out
}

fn window_base64_invalid_reply(verb: &str, err: &base64::DecodeError) -> String {
    let err = err.to_string();
    let mut out = String::with_capacity("ERR  invalid base64: \n".len() + verb.len() + err.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" invalid base64: ");
    out.push_str(&err);
    out.push('\n');
    out
}

fn parse_window_base64(rest: &str, verb: &str) -> Result<(String, Vec<u8>), String> {
    let Some((window, encoded)) = rest.trim().split_once(' ') else {
        return Err(window_base64_required_reply(verb));
    };
    let window = window.trim();
    let encoded = encoded.trim();
    if window.is_empty() || window.contains(char::is_whitespace) || encoded.is_empty() {
        return Err(window_base64_required_reply(verb));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| window_base64_invalid_reply(verb, &err))?;
    Ok((window.to_string(), bytes))
}

fn send_mouse_queued_reply(count: usize, window: &str, event: &str, col: u16, row: u16) -> String {
    let mut out = String::with_capacity(
        "SEND_MOUSE_QUEUED command= window= event= col= row=\n".len()
            + usize_decimal_len(count)
            + window.len()
            + event.len()
            + usize_decimal_len(usize::from(col))
            + usize_decimal_len(usize::from(row)),
    );
    out.push_str("SEND_MOUSE_QUEUED command=");
    write!(out, "{count}").expect("write to string");
    out.push_str(" window=");
    out.push_str(window);
    out.push_str(" event=");
    out.push_str(event);
    out.push_str(" col=");
    write!(out, "{col}").expect("write to string");
    out.push_str(" row=");
    write!(out, "{row}").expect("write to string");
    out.push('\n');
    out
}

fn queue_native_send_mouse(pending: &Arc<Mutex<NativeSpawnQueueState>>, rest: &str) -> String {
    let mut parts = rest.split_whitespace();
    let Some(window) = parts.next() else {
        return "ERR SEND_MOUSE requires window, event, col, and row\n".to_string();
    };
    let Some(event) = parts.next() else {
        return "ERR SEND_MOUSE requires window, event, col, and row\n".to_string();
    };
    let Some(col) = parts.next().and_then(|value| value.parse::<u16>().ok()) else {
        return "ERR SEND_MOUSE col must be an integer\n".to_string();
    };
    let Some(row) = parts.next().and_then(|value| value.parse::<u16>().ok()) else {
        return "ERR SEND_MOUSE row must be an integer\n".to_string();
    };
    if parts.next().is_some()
        || window.contains(char::is_whitespace)
        || !native_mouse_event_known(event)
        || col == 0
        || row == 0
    {
        return "ERR SEND_MOUSE expects <window|focused> <press-left|press-middle|press-right|release|release-left|release-middle|release-right|move|move-left|move-middle|move-right|scroll-up|scroll-down> <col> <row>\n".to_string();
    }
    match pending.lock() {
        Ok(mut state) => {
            let old_pending = state.pending.len();
            state.pending.push(NativePaneCommand::SendMouse {
                window: window.to_string(),
                event: event.to_string(),
                col,
                row,
            });
            push_native_pending_status_event(&mut state, old_pending);
            push_native_input_event(
                &mut state,
                window,
                "mouse",
                serde_json::json!({ "event": event, "col": col, "row": row }),
            );
            send_mouse_queued_reply(state.pending.len(), window, event, col, row)
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn native_mouse_event_known(event: &str) -> bool {
    matches!(
        event,
        "press-left"
            | "press-middle"
            | "press-right"
            | "release"
            | "release-left"
            | "release-middle"
            | "release-right"
            | "move"
            | "move-left"
            | "move-middle"
            | "move-right"
            | "scroll-up"
            | "scroll-down"
    )
}

const NATIVE_SEND_KEY_SUPPORTED_HELP: &str = "enter|return|tab|shift-tab|backtab|escape|esc|backspace|bs|insert|ins|shift-insert|alt-insert|ctrl-insert|delete|del|shift-delete|alt-delete|ctrl-delete|left|arrow-left|right|arrow-right|up|arrow-up|down|arrow-down|shift-left|shift-right|shift-up|shift-down|shift-arrow-left|shift-arrow-right|shift-arrow-up|shift-arrow-down|alt-left|alt-right|alt-up|alt-down|alt-arrow-left|alt-arrow-right|alt-arrow-up|alt-arrow-down|ctrl-left|ctrl-right|ctrl-up|ctrl-down|shift-home|alt-home|ctrl-home|shift-end|alt-end|ctrl-end|home|end|shift-page-up|alt-page-up|ctrl-page-up|shift-page-down|alt-page-down|ctrl-page-down|pageup|page-up|pagedown|page-down|f5..f12|ctrl-a..ctrl-z";

fn send_key_queued_reply(count: usize, window: &str, key: &str, bytes: usize) -> String {
    let mut out = String::with_capacity(
        "SEND_KEY_QUEUED command= window= key= bytes=\n".len()
            + usize_decimal_len(count)
            + window.len()
            + key.len()
            + usize_decimal_len(bytes),
    );
    out.push_str("SEND_KEY_QUEUED command=");
    write!(out, "{count}").expect("write to string");
    out.push_str(" window=");
    out.push_str(window);
    out.push_str(" key=");
    out.push_str(key);
    out.push_str(" bytes=");
    write!(out, "{bytes}").expect("write to string");
    out.push('\n');
    out
}

fn send_key_unsupported_reply() -> String {
    let mut out = String::with_capacity(
        "ERR SEND_KEY unsupported key; expected \n".len() + NATIVE_SEND_KEY_SUPPORTED_HELP.len(),
    );
    out.push_str("ERR SEND_KEY unsupported key; expected ");
    out.push_str(NATIVE_SEND_KEY_SUPPORTED_HELP);
    out.push('\n');
    out
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
        return send_key_unsupported_reply();
    };
    match pending.lock() {
        Ok(mut state) => {
            let old_pending = state.pending.len();
            state.pending.push(NativePaneCommand::SendBytes {
                window: window.to_string(),
                bytes: bytes.clone(),
                label: key.to_string(),
            });
            push_native_pending_status_event(&mut state, old_pending);
            push_native_input_event(
                &mut state,
                window,
                "key",
                serde_json::json!({ "key": key, "bytes": bytes.len() }),
            );
            send_key_queued_reply(state.pending.len(), window, key, bytes.len())
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn native_key_bytes(key: &str) -> Option<Vec<u8>> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
    let bytes: &[u8] = match normalized.as_str() {
        "enter" | "return" => b"\r",
        "tab" => b"\t",
        "shift-tab" | "backtab" => b"\x1b[Z",
        "escape" | "esc" => b"\x1b",
        "backspace" | "bs" => b"\x7f",
        "insert" | "ins" => b"\x1b[2~",
        "shift-insert" | "shift-ins" => b"\x1b[2;2~",
        "alt-insert" | "alt-ins" => b"\x1b[2;3~",
        "ctrl-insert" | "ctrl-ins" => b"\x1b[2;5~",
        "delete" | "del" => b"\x1b[3~",
        "shift-delete" | "shift-del" => b"\x1b[3;2~",
        "alt-delete" | "alt-del" => b"\x1b[3;3~",
        "ctrl-delete" | "ctrl-del" => b"\x1b[3;5~",
        "left" | "arrow-left" => b"\x1b[D",
        "right" | "arrow-right" => b"\x1b[C",
        "up" | "arrow-up" => b"\x1b[A",
        "down" | "arrow-down" => b"\x1b[B",
        "shift-left" | "shift-arrow-left" => b"\x1b[1;2D",
        "shift-right" | "shift-arrow-right" => b"\x1b[1;2C",
        "shift-up" | "shift-arrow-up" => b"\x1b[1;2A",
        "shift-down" | "shift-arrow-down" => b"\x1b[1;2B",
        "alt-left" | "alt-arrow-left" => b"\x1b[1;3D",
        "alt-right" | "alt-arrow-right" => b"\x1b[1;3C",
        "alt-up" | "alt-arrow-up" => b"\x1b[1;3A",
        "alt-down" | "alt-arrow-down" => b"\x1b[1;3B",
        "ctrl-left" | "ctrl-arrow-left" => b"\x1b[1;5D",
        "ctrl-right" | "ctrl-arrow-right" => b"\x1b[1;5C",
        "ctrl-up" | "ctrl-arrow-up" => b"\x1b[1;5A",
        "ctrl-down" | "ctrl-arrow-down" => b"\x1b[1;5B",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "shift-home" => b"\x1b[1;2H",
        "shift-end" => b"\x1b[1;2F",
        "alt-home" => b"\x1b[1;3H",
        "alt-end" => b"\x1b[1;3F",
        "ctrl-home" => b"\x1b[1;5H",
        "ctrl-end" => b"\x1b[1;5F",
        "pageup" | "page-up" => b"\x1b[5~",
        "pagedown" | "page-down" => b"\x1b[6~",
        "shift-pageup" | "shift-page-up" => b"\x1b[5;2~",
        "shift-pagedown" | "shift-page-down" => b"\x1b[6;2~",
        "alt-pageup" | "alt-page-up" => b"\x1b[5;3~",
        "alt-pagedown" | "alt-page-down" => b"\x1b[6;3~",
        "ctrl-pageup" | "ctrl-page-up" => b"\x1b[5;5~",
        "ctrl-pagedown" | "ctrl-page-down" => b"\x1b[6;5~",
        "f5" => b"\x1b[15~",
        "f6" => b"\x1b[17~",
        "f7" => b"\x1b[18~",
        "f8" => b"\x1b[19~",
        "f9" => b"\x1b[20~",
        "f10" => b"\x1b[21~",
        "f11" => b"\x1b[23~",
        "f12" => b"\x1b[24~",
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

fn reserve_chrome_invalid_json_reply(err: &serde_json::Error) -> String {
    let err = err.to_string();
    let mut out =
        String::with_capacity("ERR RESERVE_CHROME_JSON invalid json: \n".len() + err.len());
    out.push_str("ERR RESERVE_CHROME_JSON invalid json: ");
    out.push_str(&err);
    out.push('\n');
    out
}

fn chrome_reserved_reply(reservation: &NativeChromeReservationConfig) -> String {
    let reservation = serde_json::to_string(reservation).expect("serialize chrome reservation");
    let mut out = String::with_capacity("CHROME_RESERVED \n".len() + reservation.len());
    out.push_str("CHROME_RESERVED ");
    out.push_str(&reservation);
    out.push('\n');
    out
}

fn native_reserve_chrome_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    let mut reservation: NativeChromeReservationConfig = match serde_json::from_str(rest.trim()) {
        Ok(value) => value,
        Err(err) => return reserve_chrome_invalid_json_reply(&err),
    };
    reservation.top_bar_rows = reservation.top_bar_rows.min(20);
    reservation.bottom_bar_rows = reservation.bottom_bar_rows.min(20);
    reservation.left_cols = reservation.left_cols.min(80);
    reservation.right_cols = reservation.right_cols.min(80);
    reservation.gap_cols = reservation.gap_cols.min(20);
    reservation.gap_rows = reservation.gap_rows.min(20);
    reservation.owner = reservation
        .owner
        .take()
        .map(|owner| owner.trim().to_string())
        .filter(|owner| !owner.is_empty());
    match pending.lock() {
        Ok(mut state) => {
            state.chrome_reservation = reservation.clone();
            let chrome = native_chrome_status_value(&state);
            push_native_event(
                &mut state,
                "chrome_reservation_changed",
                None,
                serde_json::json!({ "chrome": chrome }),
            );
            chrome_reserved_reply(&reservation)
        }
        Err(_) => "ERR registry poisoned\n".to_string(),
    }
}

fn native_spawn_status_line(pending: usize, panes: usize, focused: &str, layout: &str) -> String {
    let mut out = String::with_capacity(
        "OK pending= panes= focus= layout=\n".len()
            + usize_decimal_len(pending)
            + usize_decimal_len(panes)
            + focused.len()
            + layout.len(),
    );
    out.push_str("OK pending=");
    write!(out, "{pending}").expect("write to string");
    out.push_str(" panes=");
    write!(out, "{panes}").expect("write to string");
    out.push_str(" focus=");
    out.push_str(focused);
    out.push_str(" layout=");
    out.push_str(layout);
    out.push('\n');
    out
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
            native_spawn_status_line(
                state.pending.len(),
                state.panes.len(),
                focused,
                state.layout.as_deref().unwrap_or("-"),
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
    json_value_line(&serde_json::json!({
        "pending": state.pending.len(),
        "panes": state.panes.len(),
        "focus": focused,
        "layout": state.layout.as_deref().unwrap_or("-"),
        "workspace": native_workspace_id_for_state(&state),
        "chrome": native_chrome_status_value(&state),
        "focused_pane": focused_pane,
        "panes_detail": state.panes,
    }))
}

fn json_value_line(value: &serde_json::Value) -> String {
    let value = value.to_string();
    let mut out = String::with_capacity(value.len() + 1);
    out.push_str(&value);
    out.push('\n');
    out
}

fn native_chrome_json_reply(pending: &Arc<Mutex<NativeSpawnQueueState>>) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    json_value_line(&native_chrome_status_value(&state))
}

fn native_shortcuts_json_reply() -> String {
    crate::shortcuts::render_native_shortcuts_json()
}

fn normalize_workspace_label(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn native_workspace_id() -> String {
    normalize_workspace_label(std::env::var("KITTWM_WORKSPACE").ok().as_deref())
        .unwrap_or_else(|| "1".to_string())
}

fn native_workspace_id_for_state(state: &NativeSpawnQueueState) -> String {
    state
        .workspace
        .as_deref()
        .and_then(|value| normalize_workspace_label(Some(value)))
        .unwrap_or_else(native_workspace_id)
}

fn native_chrome_status_value(state: &NativeSpawnQueueState) -> serde_json::Value {
    let reservation = &state.chrome_reservation;
    let tilable_rows = state
        .panes
        .iter()
        .filter_map(|pane| Some(u32::from(pane.y?) + u32::from(pane.rows?)))
        .max()
        .map(|bottom| {
            bottom
                .saturating_sub(u32::from(reservation.top_bar_rows))
                .saturating_sub(u32::from(reservation.bottom_bar_rows))
        });
    let workspace = native_workspace_id_for_state(state);
    serde_json::json!({
        "workspace": workspace,
        "top_bar_rows": reservation.top_bar_rows,
        "bottom_bar_rows": reservation.bottom_bar_rows,
        "left_cols": reservation.left_cols,
        "right_cols": reservation.right_cols,
        "gap_cols": reservation.gap_cols,
        "gap_rows": reservation.gap_rows,
        "owner": reservation.owner,
        "tilable_rows": tilable_rows,
    })
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
            "  window={} focused={} weight={} stack={} top={} floating={},{} moved={} title_draggable={} title_drag_kind={} title_drag={} title_drag_active={} pid={} command={:?} cursor={} cursor_visible={} bracketed_paste={} app_cursor={} mouse={} layout={} title={:?}",
            pane.window,
            pane.focused,
            pane.weight,
            pane.stack_index
                .map(|stack| stack.to_string())
                .unwrap_or_else(|| "-".to_string()),
            native_pane_bool_label(pane.stack_top),
            pane.floating_dx
                .map(|dx| dx.to_string())
                .unwrap_or_else(|| "-".to_string()),
            pane.floating_dy
                .map(|dy| dy.to_string())
                .unwrap_or_else(|| "-".to_string()),
            native_pane_bool_label(pane.floating_moved),
            native_pane_bool_label(pane.title_draggable),
            pane.title_drag_kind.as_deref().unwrap_or("-"),
            native_pane_coord_label(pane.title_drag_col, pane.title_drag_row),
            native_pane_bool_label(pane.title_drag_active),
            pane.pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            pane.command,
            native_pane_cursor_label(pane),
            native_pane_bool_label(pane.cursor_visible),
            native_pane_bracketed_paste_label(pane),
            native_pane_bool_label(pane.application_cursor_keys),
            native_pane_mouse_label(pane),
            native_pane_layout_label(pane),
            pane.title
        );
    }
    out.push_str("END\n");
    out
}

fn native_pane_cursor_label(pane: &NativePaneStatus) -> String {
    let (Some(col), Some(row)) = (pane.cursor_col, pane.cursor_row) else {
        return "-".to_string();
    };
    let mut out = String::with_capacity(8);
    let _ = write!(out, "{col},{row}");
    out
}

fn native_pane_bracketed_paste_label(pane: &NativePaneStatus) -> &'static str {
    native_pane_bool_label(pane.bracketed_paste)
}

fn native_pane_mouse_label(pane: &NativePaneStatus) -> String {
    let mut out = String::new();
    append_native_pane_mouse_mode(&mut out, pane.mouse_reporting, "basic");
    append_native_pane_mouse_mode(&mut out, pane.mouse_button_motion, "button-motion");
    append_native_pane_mouse_mode(&mut out, pane.mouse_all_motion, "all-motion");
    append_native_pane_mouse_mode(&mut out, pane.mouse_sgr, "sgr");
    if out.is_empty() {
        "-".to_string()
    } else {
        out
    }
}

fn append_native_pane_mouse_mode(out: &mut String, enabled: Option<bool>, label: &str) {
    if enabled != Some(true) {
        return;
    }
    if !out.is_empty() {
        out.push(',');
    }
    out.push_str(label);
}

fn native_pane_bool_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "-",
    }
}

fn native_pane_coord_label(col: Option<u16>, row: Option<u16>) -> String {
    let (Some(col), Some(row)) = (col, row) else {
        return "-".to_string();
    };
    let mut out = String::with_capacity(8);
    let _ = write!(out, "{col},{row}");
    out
}

fn native_pane_layout_label(pane: &NativePaneStatus) -> String {
    let (
        Some(x),
        Some(y),
        Some(cols),
        Some(rows),
        Some(app_x),
        Some(app_y),
        Some(app_cols),
        Some(app_rows),
    ) = (
        pane.x,
        pane.y,
        pane.cols,
        pane.rows,
        pane.app_x,
        pane.app_y,
        pane.app_cols,
        pane.app_rows,
    )
    else {
        return "-".to_string();
    };
    let mut out = String::with_capacity(32);
    let _ = write!(
        out,
        "{x},{y} {cols}x{rows} app={app_x},{app_y} {app_cols}x{app_rows}"
    );
    out
}

fn native_spawn_wait_text_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    native_spawn_wait_ms_reply(pending, rest, "WAIT_TEXT_MS", false, false)
}

fn native_spawn_wait_text_json_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    native_spawn_wait_ms_reply(pending, rest, "WAIT_TEXT_JSON_MS", false, true)
}

fn native_spawn_wait_output_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    native_spawn_wait_ms_reply(pending, rest, "WAIT_OUTPUT_MS", true, false)
}

fn native_spawn_wait_output_json_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    native_spawn_wait_ms_reply(pending, rest, "WAIT_OUTPUT_JSON_MS", true, true)
}

fn wait_ms_requires_args_reply(verb: &str) -> String {
    let mut out = String::with_capacity(
        "ERR  requires window, milliseconds, and needle\n".len() + verb.len(),
    );
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" requires window, milliseconds, and needle\n");
    out
}

fn wait_ms_integer_reply(verb: &str) -> String {
    let mut out =
        String::with_capacity("ERR  milliseconds must be an integer\n".len() + verb.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" milliseconds must be an integer\n");
    out
}

fn wait_ms_range_reply(verb: &str) -> String {
    let mut out =
        String::with_capacity("ERR  milliseconds must be in 1..=60000\n".len() + verb.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" milliseconds must be in 1..=60000\n");
    out
}

fn native_spawn_wait_ms_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    verb: &str,
    include_scrollback: bool,
    json: bool,
) -> String {
    let Some((target, rest)) = rest.trim_start().split_once(' ') else {
        return wait_ms_requires_args_reply(verb);
    };
    let Some((ms, needle)) = rest.trim_start().split_once(' ') else {
        return wait_ms_requires_args_reply(verb);
    };
    let Ok(ms) = ms.trim().parse::<u64>() else {
        return wait_ms_integer_reply(verb);
    };
    if ms == 0 || ms > 60_000 {
        return wait_ms_range_reply(verb);
    }
    native_spawn_wait_reply(
        pending,
        &space_pair_arg(target.trim(), needle.trim()),
        Duration::from_millis(ms),
        verb,
        include_scrollback,
        json,
    )
}

fn native_spawn_wait_text_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
) -> String {
    native_spawn_wait_reply(pending, rest, timeout, "WAIT_TEXT", false, false)
}

fn native_spawn_wait_text_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
) -> String {
    native_spawn_wait_reply(pending, rest, timeout, "WAIT_TEXT_JSON", false, true)
}

fn native_spawn_wait_output_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
) -> String {
    native_spawn_wait_reply(pending, rest, timeout, "WAIT_OUTPUT", true, false)
}

fn native_spawn_wait_output_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
) -> String {
    native_spawn_wait_reply(pending, rest, timeout, "WAIT_OUTPUT_JSON", true, true)
}

fn wait_match_reply(match_tag: &str, window: &str, bytes: usize) -> String {
    let mut out = String::with_capacity(
        match_tag.len() + " window= bytes=\n".len() + window.len() + usize_decimal_len(bytes),
    );
    out.push_str(match_tag);
    out.push_str(" window=");
    out.push_str(window);
    out.push_str(" bytes=");
    write!(out, "{bytes}").expect("write to string");
    out.push('\n');
    out
}

fn wait_requires_needle_reply(verb: &str) -> String {
    let mut out = String::with_capacity("ERR  requires window and needle\n".len() + verb.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" requires window and needle\n");
    out
}

fn wait_no_pane_reply(verb: &str, target: &str) -> String {
    let mut out =
        String::with_capacity("ERR  no pane matching \n".len() + verb.len() + target.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" no pane matching ");
    out.push_str(target);
    out.push('\n');
    out
}

fn wait_timeout_reply(verb: &str, window: &str, needle_bytes: usize) -> String {
    let mut out = String::with_capacity(
        "ERR  timeout window= needle_bytes=\n".len()
            + verb.len()
            + window.len()
            + usize_decimal_len(needle_bytes),
    );
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" timeout window=");
    out.push_str(window);
    out.push_str(" needle_bytes=");
    write!(out, "{needle_bytes}").expect("write to string");
    out.push('\n');
    out
}

fn native_spawn_wait_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
    timeout: Duration,
    verb: &str,
    include_scrollback: bool,
    json: bool,
) -> String {
    let Some((target, needle)) = rest.trim_start().split_once(' ') else {
        return wait_requires_needle_reply(verb);
    };
    let target = target.trim();
    let needle = needle.trim();
    if target.is_empty() || needle.is_empty() {
        return wait_requires_needle_reply(verb);
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
            return wait_no_pane_reply(verb, target);
        };
        if text.contains(needle) {
            let match_tag = if include_scrollback {
                "MATCH_OUTPUT"
            } else {
                "MATCH_TEXT"
            };
            if json {
                return json_value_line(&serde_json::json!({
                    "kind": if include_scrollback { "output" } else { "text" },
                    "match": match_tag,
                    "window": window,
                    "bytes": text.len(),
                }));
            }
            return wait_match_reply(match_tag, &window, text.len());
        }
        if Instant::now() >= deadline {
            return wait_timeout_reply(verb, &window, needle.len());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_text_no_pane_reply(target: &str) -> String {
    let target = target.trim();
    let mut out = String::with_capacity("ERR READ_TEXT no pane matching \n".len() + target.len());
    out.push_str("ERR READ_TEXT no pane matching ");
    out.push_str(target);
    out.push('\n');
    out
}

fn read_scrollback_no_pane_reply(target: &str) -> String {
    let target = target.trim();
    let mut out =
        String::with_capacity("ERR READ_SCROLLBACK no pane matching \n".len() + target.len());
    out.push_str("ERR READ_SCROLLBACK no pane matching ");
    out.push_str(target);
    out.push('\n');
    out
}

fn read_json_no_pane_reply(target: &str) -> String {
    json_value_line(
        &serde_json::json!({ "error": "no pane matching target", "target": target.trim() }),
    )
}

fn native_spawn_read_text_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return read_text_no_pane_reply(target);
    };
    let text = pane.text_snapshot.as_deref().unwrap_or("");
    let cursor = native_pane_cursor_label(pane);
    let mut out = String::with_capacity(
        "TEXT window= bytes= cursor=\nEND\n".len()
            + pane.window.len()
            + usize_decimal_len(text.len())
            + cursor.len()
            + text.len(),
    );
    out.push_str("TEXT window=");
    out.push_str(&pane.window);
    out.push_str(" bytes=");
    write!(out, "{}", text.len()).expect("write to string");
    out.push_str(" cursor=");
    out.push_str(&cursor);
    out.push('\n');
    out.push_str(text);
    out.push_str("END\n");
    out
}

fn native_spawn_read_text_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return read_json_no_pane_reply(target);
    };
    json_value_line(&serde_json::json!({
        "window": pane.window,
        "text": pane.text_snapshot.as_deref().unwrap_or(""),
        "cursor_col": pane.cursor_col,
        "cursor_row": pane.cursor_row,
    }))
}

fn native_spawn_read_scrollback_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return read_scrollback_no_pane_reply(target);
    };
    let text = pane.scrollback_snapshot.as_deref().unwrap_or("");
    let mut out = String::with_capacity(
        "SCROLLBACK window= bytes=\nEND\n".len()
            + pane.window.len()
            + usize_decimal_len(text.len())
            + text.len(),
    );
    out.push_str("SCROLLBACK window=");
    out.push_str(&pane.window);
    out.push_str(" bytes=");
    write!(out, "{}", text.len()).expect("write to string");
    out.push('\n');
    out.push_str(text);
    out.push_str("END\n");
    out
}

fn native_spawn_read_scrollback_json_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return read_json_no_pane_reply(target);
    };
    json_value_line(&serde_json::json!({
        "window": pane.window,
        "scrollback": pane.scrollback_snapshot.as_deref().unwrap_or(""),
    }))
}

fn native_spawn_semantic_snapshot_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    target: &str,
) -> String {
    let Ok(state) = pending.lock() else {
        return "{\"error\":\"registry poisoned\"}\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return read_json_no_pane_reply(target);
    };
    let snapshot = state
        .semantic_snapshots
        .get(&pane.window)
        .cloned()
        .unwrap_or_else(|| native_semantic_snapshot_for_pane(pane));
    let mut out = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

fn native_spawn_semantic_publish_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    let Some((target, json)) = rest.trim_start().split_once(' ') else {
        return "ERR SEMANTIC_PUBLISH requires window and snapshot-json\n".to_string();
    };
    let mut snapshot = match serde_json::from_str::<SemanticSurfaceSnapshot>(json.trim()) {
        Ok(snapshot) => snapshot,
        Err(_) => return "ERR SEMANTIC_PUBLISH snapshot must be JSON\n".to_string(),
    };
    if snapshot.schema_version != 1 {
        return "ERR SEMANTIC_PUBLISH schema_version must be 1\n".to_string();
    }
    if snapshot.surface.trim().is_empty() || snapshot.root.id.as_str().trim().is_empty() {
        return "ERR SEMANTIC_PUBLISH requires surface and root.id\n".to_string();
    }
    let Ok(mut state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return semantic_no_pane_reply("SEMANTIC_PUBLISH", target);
    };
    let window = pane.window.clone();
    if snapshot.surface != window {
        return semantic_publish_surface_mismatch_reply(&window, &snapshot.surface);
    }
    snapshot.surface = window.clone();
    let revision = snapshot.revision;
    let focus = snapshot.focus.as_ref().map(|id| id.as_str().to_string());
    state.semantic_snapshots.insert(window.clone(), snapshot);
    push_native_event(
        &mut state,
        "semantic_snapshot_ready",
        Some(window.clone()),
        serde_json::json!({ "revision": revision, "focus": focus }),
    );
    semantic_published_reply(&window)
}

fn semantic_published_reply(window: &str) -> String {
    let mut out = String::with_capacity("SEMANTIC_PUBLISHED window=\n".len() + window.len());
    out.push_str("SEMANTIC_PUBLISHED window=");
    out.push_str(window);
    out.push('\n');
    out
}

fn semantic_no_pane_reply(verb: &str, target: &str) -> String {
    let target = target.trim();
    let mut out =
        String::with_capacity("ERR  no pane matching \n".len() + verb.len() + target.len());
    out.push_str("ERR ");
    out.push_str(verb);
    out.push_str(" no pane matching ");
    out.push_str(target);
    out.push('\n');
    out
}

fn semantic_publish_surface_mismatch_reply(window: &str, snapshot_surface: &str) -> String {
    let mut out = String::with_capacity(
        "ERR SEMANTIC_PUBLISH surface mismatch window= snapshot=\n".len()
            + window.len()
            + snapshot_surface.len(),
    );
    out.push_str("ERR SEMANTIC_PUBLISH surface mismatch window=");
    out.push_str(window);
    out.push_str(" snapshot=");
    out.push_str(snapshot_surface);
    out.push('\n');
    out
}

fn semantic_action_applied_reply(window: &str, component: &str, action: &str) -> String {
    let component = component.trim();
    let action = action.trim();
    let mut out = String::with_capacity(
        "SEMANTIC_ACTION_APPLIED window= component= action=\n".len()
            + window.len()
            + component.len()
            + action.len(),
    );
    out.push_str("SEMANTIC_ACTION_APPLIED window=");
    out.push_str(window);
    out.push_str(" component=");
    out.push_str(component);
    out.push_str(" action=");
    out.push_str(action);
    out.push('\n');
    out
}

fn semantic_focused_reply(window: &str, component: &str) -> String {
    let component = component.trim();
    let mut out = String::with_capacity(
        "SEMANTIC_FOCUSED window= component=\n".len() + window.len() + component.len(),
    );
    out.push_str("SEMANTIC_FOCUSED window=");
    out.push_str(window);
    out.push_str(" component=");
    out.push_str(component);
    out.push('\n');
    out
}

fn semantic_action_unsupported_reply(window: &str, component: &str, action: &str) -> String {
    let component = component.trim();
    let action = action.trim();
    let mut out = String::with_capacity(
        "ERR SEMANTIC_ACTION unsupported window= component= action=\n".len()
            + window.len()
            + component.len()
            + action.len(),
    );
    out.push_str("ERR SEMANTIC_ACTION unsupported window=");
    out.push_str(window);
    out.push_str(" component=");
    out.push_str(component);
    out.push_str(" action=");
    out.push_str(action);
    out.push('\n');
    out
}

fn semantic_focus_unsupported_reply(window: &str, component: &str) -> String {
    let component = component.trim();
    let mut out = String::with_capacity(
        "ERR SEMANTIC_FOCUS unsupported window= component=\n".len()
            + window.len()
            + component.len(),
    );
    out.push_str("ERR SEMANTIC_FOCUS unsupported window=");
    out.push_str(window);
    out.push_str(" component=");
    out.push_str(component);
    out.push('\n');
    out
}

fn semantic_action_failed_reply(window: &str, component: &str, action: &str, err: &str) -> String {
    let component = component.trim();
    let action = action.trim();
    let mut out = String::with_capacity(
        "ERR SEMANTIC_ACTION window= component= action= \n".len()
            + window.len()
            + component.len()
            + action.len()
            + err.len(),
    );
    out.push_str("ERR SEMANTIC_ACTION window=");
    out.push_str(window);
    out.push_str(" component=");
    out.push_str(component);
    out.push_str(" action=");
    out.push_str(action);
    out.push(' ');
    out.push_str(err);
    out.push('\n');
    out
}

fn semantic_focus_failed_reply(window: &str, component: &str, err: &str) -> String {
    let component = component.trim();
    let mut out = String::with_capacity(
        "ERR SEMANTIC_FOCUS window= component= \n".len()
            + window.len()
            + component.len()
            + err.len(),
    );
    out.push_str("ERR SEMANTIC_FOCUS window=");
    out.push_str(window);
    out.push_str(" component=");
    out.push_str(component);
    out.push(' ');
    out.push_str(err);
    out.push('\n');
    out
}

fn semantic_component_id(window: &str, suffix: &str) -> String {
    let mut id = String::with_capacity(window.len() + 1 + suffix.len());
    id.push_str(window);
    id.push('.');
    id.push_str(suffix);
    id
}

fn native_semantic_snapshot_for_pane(pane: &NativePaneStatus) -> SemanticSurfaceSnapshot {
    let text_id = semantic_component_id(&pane.window, "screen");
    let mut text_state = ComponentState {
        focusable: true,
        focused: pane.focused,
        ..ComponentState::default()
    };
    text_state.sensitive = false;
    let text = ComponentNode::new(&text_id, ComponentRole::TextArea)
        .labeled("terminal screen")
        .valued(ComponentValue::Text(
            pane.text_snapshot.clone().unwrap_or_default(),
        ))
        .state(text_state)
        .actions(vec![
            ComponentAction::new("focus", ActionKind::Focus),
            ComponentAction::new("insert_text", ActionKind::InsertText),
        ]);
    let root = ComponentNode::new(
        semantic_component_id(&pane.window, "root"),
        ComponentRole::Group,
    )
    .labeled(pane.title.clone())
    .children(vec![text]);
    SemanticSurfaceSnapshot::new(pane.window.clone(), 1, root).focused(text_id)
}

fn native_spawn_semantic_action_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    let Some((target, rest)) = rest.trim_start().split_once(' ') else {
        return "ERR SEMANTIC_ACTION requires window component action json\n".to_string();
    };
    let Some((component, rest)) = rest.trim_start().split_once(' ') else {
        return "ERR SEMANTIC_ACTION requires window component action json\n".to_string();
    };
    let Some((action, payload)) = rest.trim_start().split_once(' ') else {
        return "ERR SEMANTIC_ACTION requires window component action json\n".to_string();
    };
    let payload = match serde_json::from_str::<serde_json::Value>(payload.trim()) {
        Ok(value) => value,
        Err(_) => return "ERR SEMANTIC_ACTION payload must be JSON\n".to_string(),
    };
    let Ok(mut state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return semantic_no_pane_reply("SEMANTIC_ACTION", target);
    };
    let window = pane.window.clone();
    let action_result = {
        let Some(snapshot) = state.semantic_snapshots.get_mut(&window) else {
            return semantic_action_unsupported_reply(&window, component, action);
        };
        apply_semantic_action(snapshot, component.trim(), action.trim(), &payload)
            .map(|detail| (snapshot.revision, detail.value))
    };
    match action_result {
        Ok((revision, value)) => {
            push_native_event(
                &mut state,
                "semantic_action_invoked",
                Some(window.clone()),
                serde_json::json!({
                    "component": component.trim(),
                    "action": action.trim(),
                    "revision": revision,
                    "value": value,
                }),
            );
            if value != serde_json::Value::Null {
                push_native_event(
                    &mut state,
                    "semantic_value_changed",
                    Some(window.clone()),
                    serde_json::json!({
                        "component": component.trim(),
                        "revision": revision,
                        "value": value,
                    }),
                );
            }
            semantic_action_applied_reply(&window, component, action)
        }
        Err(err) => semantic_action_failed_reply(&window, component, action, err),
    }
}

fn native_spawn_semantic_focus_reply(
    pending: &Arc<Mutex<NativeSpawnQueueState>>,
    rest: &str,
) -> String {
    let Some((target, component)) = rest.trim_start().split_once(' ') else {
        return "ERR SEMANTIC_FOCUS requires window and component\n".to_string();
    };
    let Ok(mut state) = pending.lock() else {
        return "ERR registry poisoned\n".to_string();
    };
    let Some(pane) = native_find_pane_target(&state.panes, target) else {
        return semantic_no_pane_reply("SEMANTIC_FOCUS", target);
    };
    let window = pane.window.clone();
    let focus_result = {
        let Some(snapshot) = state.semantic_snapshots.get_mut(&window) else {
            return semantic_focus_unsupported_reply(&window, component);
        };
        apply_semantic_focus(snapshot, component.trim()).map(|()| snapshot.revision)
    };
    match focus_result {
        Ok(revision) => {
            push_native_event(
                &mut state,
                "semantic_focus_changed",
                Some(window.clone()),
                serde_json::json!({ "component": component.trim(), "revision": revision }),
            );
            semantic_focused_reply(&window, component)
        }
        Err(err) => semantic_focus_failed_reply(&window, component, err),
    }
}

struct SemanticActionDetail {
    value: serde_json::Value,
}

fn apply_semantic_focus(
    snapshot: &mut SemanticSurfaceSnapshot,
    component: &str,
) -> std::result::Result<(), &'static str> {
    if !semantic_component_exists(&snapshot.root, component) {
        return Err("component not found");
    }
    clear_semantic_focus(&mut snapshot.root);
    set_semantic_component_state(&mut snapshot.root, component, |node| {
        node.state.focused = true;
        node.state.focusable = true;
    })?;
    snapshot.focus = Some(SemanticComponentId::new(component));
    snapshot.revision = snapshot.revision.saturating_add(1);
    Ok(())
}

fn apply_semantic_action(
    snapshot: &mut SemanticSurfaceSnapshot,
    component: &str,
    action: &str,
    payload: &serde_json::Value,
) -> std::result::Result<SemanticActionDetail, &'static str> {
    match action {
        "focus" => {
            apply_semantic_focus(snapshot, component)?;
            Ok(SemanticActionDetail {
                value: serde_json::Value::Null,
            })
        }
        "toggle" => apply_semantic_toggle(snapshot, component),
        "set" | "set_value" | "insert_text" => {
            apply_semantic_set_value(snapshot, component, payload)
        }
        "select" => apply_semantic_select(snapshot, component, payload),
        _ => Err("unsupported action"),
    }
}

fn apply_semantic_toggle(
    snapshot: &mut SemanticSurfaceSnapshot,
    component: &str,
) -> std::result::Result<SemanticActionDetail, &'static str> {
    let node = snapshot_component_mut(snapshot, component)?;
    let next = match node.value.as_ref() {
        Some(ComponentValue::Bool(value)) => !value,
        _ => !node.state.checked,
    };
    node.value = Some(ComponentValue::Bool(next));
    node.state.checked = next;
    node.state.selected = next;
    snapshot.revision = snapshot.revision.saturating_add(1);
    Ok(SemanticActionDetail {
        value: serde_json::json!(next),
    })
}

fn apply_semantic_set_value(
    snapshot: &mut SemanticSurfaceSnapshot,
    component: &str,
    payload: &serde_json::Value,
) -> std::result::Result<SemanticActionDetail, &'static str> {
    let detail = {
        let node = snapshot_component_mut(snapshot, component)?;
        if let Some(text) = payload.get("text").and_then(serde_json::Value::as_str) {
            node.value = Some(ComponentValue::Text(text.to_string()));
            serde_json::json!(text)
        } else if let Some(value) = payload.get("value") {
            if let Some(text) = value.as_str() {
                node.value = Some(ComponentValue::Text(text.to_string()));
                serde_json::json!(text)
            } else if let Some(number) = value.as_f64() {
                node.value = Some(ComponentValue::Number(number as f32));
                serde_json::json!(number)
            } else if let Some(boolean) = value.as_bool() {
                node.value = Some(ComponentValue::Bool(boolean));
                node.state.checked = boolean;
                serde_json::json!(boolean)
            } else {
                return Err("payload must contain text or scalar value");
            }
        } else {
            return Err("payload must contain text or scalar value");
        }
    };
    snapshot.revision = snapshot.revision.saturating_add(1);
    Ok(SemanticActionDetail { value: detail })
}

fn apply_semantic_select(
    snapshot: &mut SemanticSurfaceSnapshot,
    component: &str,
    payload: &serde_json::Value,
) -> std::result::Result<SemanticActionDetail, &'static str> {
    let selection = payload
        .get("selection")
        .or_else(|| payload.get("value"))
        .and_then(|value| {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .or_else(|| {
            payload
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(|id| vec![id.to_string()])
        })
        .ok_or("payload must contain selection/value array or id")?;
    let node = snapshot_component_mut(snapshot, component)?;
    node.value = Some(ComponentValue::Selection(selection.clone()));
    for child in &mut node.children {
        child.state.selected = selection.iter().any(|id| id == child.id.as_str());
        child.state.checked = child.state.selected;
    }
    snapshot.revision = snapshot.revision.saturating_add(1);
    Ok(SemanticActionDetail {
        value: serde_json::json!(selection),
    })
}

fn snapshot_component_mut<'a>(
    snapshot: &'a mut SemanticSurfaceSnapshot,
    component: &str,
) -> std::result::Result<&'a mut ComponentNode, &'static str> {
    find_semantic_component_mut(&mut snapshot.root, component).ok_or("component not found")
}

fn semantic_component_exists(node: &ComponentNode, component: &str) -> bool {
    node.id.as_str() == component
        || node
            .children
            .iter()
            .any(|child| semantic_component_exists(child, component))
}

fn find_semantic_component_mut<'a>(
    node: &'a mut ComponentNode,
    component: &str,
) -> Option<&'a mut ComponentNode> {
    if node.id.as_str() == component {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_semantic_component_mut(child, component) {
            return Some(found);
        }
    }
    None
}

fn set_semantic_component_state(
    node: &mut ComponentNode,
    component: &str,
    update: impl FnOnce(&mut ComponentNode),
) -> std::result::Result<(), &'static str> {
    let Some(node) = find_semantic_component_mut(node, component) else {
        return Err("component not found");
    };
    update(node);
    Ok(())
}

fn clear_semantic_focus(node: &mut ComponentNode) {
    node.state.focused = false;
    for child in &mut node.children {
        clear_semantic_focus(child);
    }
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
    let layout = state.layout.as_deref().unwrap_or("-");
    let mut out = String::with_capacity(state.panes.len().saturating_mul(128).saturating_add(128));
    let _ = write!(
        out,
        "{{\"schema_version\":1,\"kind\":\"kittwm-native-session\",\"layout\":{},\"focus\":{},\"panes\":[",
        serde_json::to_string(layout).unwrap(),
        serde_json::to_string(focused).unwrap()
    );
    for (index, pane) in state.panes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"index\":{index},\"window\":{},\"title\":{},\"command\":{},\"weight\":{},\"focused\":{},\"floating_dx\":{},\"floating_dy\":{}}}",
            serde_json::to_string(&pane.window).unwrap(),
            serde_json::to_string(&pane.title).unwrap(),
            serde_json::to_string(&pane.command).unwrap(),
            pane.weight,
            pane.focused,
            pane.floating_dx.unwrap_or(0),
            pane.floating_dy.unwrap_or(0)
        );
    }
    out.push_str("]}\n");
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
    json_value_line(&serde_json::json!({
        "panes": state.panes.len(),
        "focus": focused,
        "layout": state.layout.as_deref().unwrap_or("-"),
        "workspace": native_workspace_id_for_state(&state),
        "chrome": native_chrome_status_value(&state),
        "panes_detail": state.panes,
    }))
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
        cleanup_stale_socket_for_bind(&path, "kittwm daemon")?;
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
            "APPS_FIRST" => apps_first_reply("", false),
            "APPS_LAUNCH_FIRST" => apps_first_reply("", true),
            "SPAWN" => spawn_reply("", path, panes),
            "PANES" => panes_reply(panes),
            "PANES_JSON" => panes_json_reply(panes),
            "HELP" | "?" => daemon_help_reply(),
            "HELP_JSON" => daemon_help_json_reply(),
            "QUIT" => {
                quit.store(true, Ordering::SeqCst);
                "BYE\n".to_string()
            }
            other => daemon_unknown_command_reply(other),
        }
    };
    writer.write_all(reply.as_bytes())?;
    writer.flush()?;
    Ok(())
}

fn daemon_unknown_command_reply(command: &str) -> String {
    let mut out = String::with_capacity("ERR unknown: \n".len() + command.len());
    out.push_str("ERR unknown: ");
    out.push_str(command);
    out.push('\n');
    out
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
    let entries = daemon_help_entries();
    let mut out = String::with_capacity(entries.len().saturating_mul(16));
    for (idx, (command, _, _)) in entries.into_iter().enumerate() {
        if idx > 0 {
            out.push_str(" | ");
        }
        out.push_str(command);
    }
    out.push('\n');
    out
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
    json_value_line(&serde_json::json!({ "commands": commands }))
}

fn client_read_timeout_for(cmd: &str) -> Duration {
    let trimmed = cmd.trim_start();
    if trimmed == "EVENTS" || trimmed.starts_with("EVENTS ") {
        return parse_events_timeout(trimmed).saturating_add(CLIENT_WAIT_TEXT_MARGIN);
    }
    let Some(rest) = trimmed
        .strip_prefix("WAIT_TEXT_MS ")
        .or_else(|| trimmed.strip_prefix("WAIT_OUTPUT_MS "))
        .or_else(|| trimmed.strip_prefix("WAIT_TEXT_JSON_MS "))
        .or_else(|| trimmed.strip_prefix("WAIT_OUTPUT_JSON_MS "))
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_socket_filename(prefix: &str, pid: u32) -> String {
        let mut name = String::with_capacity(prefix.len() + 1 + 10 + ".sock".len());
        name.push_str(prefix);
        name.push('-');
        let _ = write!(name, "{pid}");
        name.push_str(".sock");
        name
    }

    fn tmp_sock() -> PathBuf {
        std::env::temp_dir().join(test_socket_filename("kittwm-test", std::process::id()))
    }

    #[test]
    fn test_socket_filename_builds_directly() {
        let name = test_socket_filename("kittwm-test", 123);
        assert_eq!(name, "kittwm-test-123.sock");
        assert!(name.capacity() >= name.len());
    }

    #[test]
    fn daemon_socket_path_strings_build_directly() {
        let user = user_socket_path_string("alice");
        assert_eq!(user, "/tmp/kittwm-alice.sock");
        assert_eq!(user.capacity(), user.len());

        let display = display_id_socket_path_string("7");
        assert_eq!(display, "/tmp/kittui-wm-7.sock");
        assert_eq!(display.capacity(), display.len());
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
            client_read_timeout_for("EVENTS 250"),
            Duration::from_millis(250).saturating_add(CLIENT_WAIT_TEXT_MARGIN)
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
    fn native_spawn_status_line_builds_directly() {
        let line = native_spawn_status_line(12, 3, "native-2", "rows");
        assert_eq!(line, "OK pending=12 panes=3 focus=native-2 layout=rows\n");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn daemon_status_line_builds_directly() {
        let line = daemon_status_line(42, 7, "/tmp/kittwm-test.sock", 3, "2");
        assert_eq!(
            line,
            "pid=42 uptime_s=7 sock=/tmp/kittwm-test.sock panes=3 focus=2\n"
        );
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn status_includes_pid_and_uptime() {
        let p = std::env::temp_dir().join(test_socket_filename(
            "kittwm-test-status",
            std::process::id(),
        ));
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
    fn daemon_unknown_command_reply_builds_directly() {
        let reply = daemon_unknown_command_reply("NOPE");
        assert_eq!(reply, "ERR unknown: NOPE\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn daemon_help_json_reply_uses_json_value_line() {
        let reply = daemon_help_json_reply();
        assert!(reply.ends_with('\n'));
        let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert!(value["commands"]
            .as_array()
            .is_some_and(|commands| !commands.is_empty()));
    }

    #[test]
    fn standalone_daemon_help_json_lists_commands() {
        let p =
            std::env::temp_dir().join(test_socket_filename("kittwm-test-help", std::process::id()));
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
    fn standalone_daemon_help_catalog_commands_are_all_handled() {
        // Coverage guard: every command advertised in the standalone daemon HELP
        // catalog must actually be handled by the dispatcher (never "ERR unknown").
        // Skips side-effecting commands that would disrupt the test daemon.
        let p = std::env::temp_dir().join(test_socket_filename(
            "kittwm-help-coverage",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        for (command, _category, _description) in daemon_help_entries() {
            let keyword = command.split_whitespace().next().unwrap_or(command);
            if keyword == "QUIT" {
                continue;
            }
            let reply = client_request(server.path(), keyword).unwrap();
            assert!(
                !reply.starts_with("ERR unknown"),
                "HELP catalog advertises {keyword:?} but the daemon does not handle it: {reply}"
            );
        }
        // The bare APPS_FIRST/APPS_LAUNCH_FIRST/SPAWN keywords now return a helpful
        // recognized error instead of "ERR unknown".
        assert_eq!(
            client_request(server.path(), "APPS_FIRST").unwrap(),
            "ERR APPS_FIRST requires a query\n"
        );
        assert_eq!(
            client_request(server.path(), "APPS_LAUNCH_FIRST").unwrap(),
            "ERR APPS_FIRST requires a query\n"
        );
        assert_eq!(
            client_request(server.path(), "SPAWN").unwrap(),
            "ERR SPAWN requires argv\n"
        );
    }

    #[test]
    fn standalone_daemon_help_catalog_has_no_empty_or_duplicate_entries() {
        // Quality guard for the standalone daemon HELP/HELP_JSON surface: every
        // entry must carry a non-empty command, category, and description, and
        // command keywords must be unique.
        let mut seen = std::collections::HashSet::new();
        for (command, category, description) in daemon_help_entries() {
            assert!(!command.trim().is_empty(), "empty command entry");
            assert!(
                !category.trim().is_empty(),
                "empty category for {command:?}"
            );
            assert!(
                !description.trim().is_empty(),
                "empty description for {command:?}"
            );
            let keyword = command.split_whitespace().next().unwrap_or(command);
            assert!(
                seen.insert(keyword),
                "duplicate daemon HELP catalog command keyword {keyword:?}"
            );
        }
    }

    #[test]
    fn quit_sets_flag() {
        let p =
            std::env::temp_dir().join(test_socket_filename("kittwm-test-quit", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let reply = client_request(server.path(), "QUIT").unwrap();
        assert_eq!(reply.trim(), "BYE");
        // Give the accept thread a moment.
        std::thread::sleep(Duration::from_millis(50));
        assert!(server.quit_requested());
    }

    #[test]
    fn tracked_pane_labels_build_directly() {
        assert_eq!(u32_decimal_len(0), 1);
        assert_eq!(u32_decimal_len(9), 1);
        assert_eq!(u32_decimal_len(10), 2);
        assert_eq!(u32_decimal_len(123), 3);

        let window = tracked_pane_window(42);
        assert_eq!(window, "daemon-42");
        assert_eq!(window.capacity(), window.len());

        let layout = tracked_pane_layout(42);
        assert_eq!(layout, "tile:42");
        assert_eq!(layout.capacity(), layout.len());
    }

    #[test]
    fn spawn_error_replies_build_directly() {
        let poisoned = spawn_registry_poisoned_reply(42);
        assert_eq!(poisoned, "ERR SPAWN registry poisoned after pid=42\n");
        assert_eq!(poisoned.capacity(), poisoned.len());

        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing shell");
        let reply = spawn_error_reply("/no/such-command", &err);
        assert_eq!(reply, "ERR SPAWN /no/such-command: missing shell\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn spawn_success_reply_builds_directly() {
        let pane = TrackedPane {
            pane_id: 7,
            window: "daemon-7".to_string(),
            pid: 42,
            argv: "echo hello".to_string(),
            layout: "tile:7".to_string(),
            focused: true,
        };
        let reply = spawn_success_reply(&pane);
        assert_eq!(
            reply,
            "SPAWNED pane=7 window=daemon-7 pid=42 layout=tile:7 focused=true argv=echo hello\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn spawn_command_returns_tracked_pane() {
        let p = std::env::temp_dir().join(test_socket_filename(
            "kittwm-test-spawn",
            std::process::id(),
        ));
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

    fn restore_session_json_request(manifest: &serde_json::Value) -> String {
        let manifest = manifest.to_string();
        let mut request = String::with_capacity("RESTORE_SESSION_JSON ".len() + manifest.len());
        request.push_str("RESTORE_SESSION_JSON ");
        request.push_str(&manifest);
        request
    }

    #[test]
    fn restore_session_queued_reply_builds_directly() {
        let reply = restore_session_queued_reply(12);
        assert_eq!(reply, "RESTORE_SESSION_QUEUED command=12\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn restore_session_missing_command_reply_builds_directly() {
        let reply = restore_session_missing_command_reply(12);
        assert_eq!(reply, "ERR RESTORE_SESSION_JSON pane 12 missing command\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn restore_session_invalid_json_reply_builds_directly() {
        let err = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let reply = restore_session_invalid_json_reply(&err);
        assert!(
            reply.starts_with("ERR RESTORE_SESSION_JSON invalid json: EOF while parsing"),
            "{reply}"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn restore_session_json_request_builds_directly() {
        let manifest = serde_json::json!({ "layout": "rows", "panes": [] });
        let request = restore_session_json_request(&manifest);
        assert_eq!(
            request,
            "RESTORE_SESSION_JSON {\"layout\":\"rows\",\"panes\":[]}"
        );
        assert_eq!(request.capacity(), request.len());
    }

    #[test]
    fn invalid_restore_session_reports_errors_without_pending_ghost_panes() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let missing_command = serde_json::json!({
            "layout": "columns",
            "panes": [
                {"title": "missing command", "weight": 1, "focused": true}
            ]
        });
        let reply =
            native_spawn_queue_reply(&restore_session_json_request(&missing_command), &pending);
        assert!(
            reply.contains("ERR RESTORE_SESSION_JSON pane 0 missing command"),
            "{reply}"
        );
        assert!(drain_native_spawn_pending(&pending).is_empty());

        let empty = serde_json::json!({ "layout": "rows", "panes": [] });
        let reply = native_spawn_queue_reply(&restore_session_json_request(&empty), &pending);
        assert!(
            reply.contains("ERR RESTORE_SESSION_JSON requires at least one pane"),
            "{reply}"
        );
        assert!(drain_native_spawn_pending(&pending).is_empty());
    }

    #[test]
    fn native_send_key_unsupported_help_lists_current_aliases() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let reply = native_spawn_queue_reply("SEND_KEY focused nope", &pending);
        assert!(reply.starts_with("ERR SEND_KEY unsupported key; expected "));
        assert!(reply.contains("shift-tab"));
        assert!(reply.contains("backtab"));
        assert!(reply.contains("shift-insert"));
        assert!(reply.contains("ctrl-delete"));
        assert!(reply.contains("arrow-left"));
        assert!(reply.contains("shift-left"));
        assert!(reply.contains("alt-left"));
        assert!(reply.contains("ctrl-left"));
        assert!(reply.contains("shift-home"));
        assert!(reply.contains("ctrl-page-up"));
        assert!(reply.contains("page-up"));
        assert!(reply.contains("f5..f12"));
        assert!(reply.contains("ctrl-a..ctrl-z"));
    }

    #[test]
    fn queue_action_reply_builds_directly() {
        let reply = queue_action_reply("FOCUS_NEXT_QUEUED", 12);
        assert_eq!(reply, "FOCUS_NEXT_QUEUED command=12\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn queue_command_reply_builds_directly() {
        let reply = queue_command_reply("MOVE_QUEUED", 12, "focused\tlast");
        assert_eq!(reply, "MOVE_QUEUED command=12 arg=focused\tlast\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn queue_empty_error_reply_builds_directly() {
        let reply = queue_empty_error_reply("FOCUS_PANE requires window");
        assert_eq!(reply, "ERR FOCUS_PANE requires window\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn queue_native_pane_command_empty_arg_uses_direct_empty_error_reply() {
        // bd-cf9749: confirm the queued-command empty-arg path returns the direct
        // empty-error reply (no format!) when the argument is whitespace-only.
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let reply = queue_native_pane_command(
            &pending,
            "   ",
            "FOCUS_PANE requires window",
            NativePaneCommand::Focus,
            "FOCUS_QUEUED",
        );
        assert_eq!(reply, "ERR FOCUS_PANE requires window\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn tab_pair_arg_builds_move_and_rename_queue_args_directly() {
        let arg = tab_pair_arg("focused", "last");
        assert_eq!(arg, "focused\tlast");
        assert_eq!(arg.capacity(), arg.len());

        let rename = tab_pair_arg("native-2", "editor pane");
        assert_eq!(rename, "native-2\teditor pane");
        assert_eq!(rename.capacity(), rename.len());
    }

    #[test]
    fn tab_i16_arg_builds_resize_queue_arg_directly() {
        let arg = tab_i16_arg("focused", 2);
        assert_eq!(arg, "focused\t2");
        assert_eq!(arg.capacity(), arg.len());

        let negative = tab_i16_arg("native-1", -12);
        assert_eq!(negative, "native-1\t-12");
        assert_eq!(negative.capacity(), negative.len());
    }

    #[test]
    fn space_pair_arg_builds_wait_target_needle_arg_directly() {
        let arg = space_pair_arg("focused", "second line");
        assert_eq!(arg, "focused second line");
        assert_eq!(arg.capacity(), arg.len());
    }

    #[test]
    fn wait_ms_requires_args_reply_builds_directly() {
        let reply = wait_ms_requires_args_reply("WAIT_TEXT_MS");
        assert_eq!(
            reply,
            "ERR WAIT_TEXT_MS requires window, milliseconds, and needle\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn wait_ms_integer_reply_builds_directly() {
        let reply = wait_ms_integer_reply("WAIT_TEXT_MS");
        assert_eq!(reply, "ERR WAIT_TEXT_MS milliseconds must be an integer\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn wait_ms_range_reply_builds_directly() {
        let reply = wait_ms_range_reply("WAIT_TEXT_MS");
        assert_eq!(
            reply,
            "ERR WAIT_TEXT_MS milliseconds must be in 1..=60000\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn wait_match_reply_builds_directly() {
        let reply = wait_match_reply("MATCH_TEXT", "native-2", 17);
        assert_eq!(reply, "MATCH_TEXT window=native-2 bytes=17\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn wait_requires_needle_reply_builds_directly() {
        let reply = wait_requires_needle_reply("WAIT_TEXT");
        assert_eq!(reply, "ERR WAIT_TEXT requires window and needle\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn wait_no_pane_reply_builds_directly() {
        let reply = wait_no_pane_reply("WAIT_TEXT", "missing");
        assert_eq!(reply, "ERR WAIT_TEXT no pane matching missing\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn read_text_no_pane_reply_builds_directly() {
        let reply = read_text_no_pane_reply(" missing ");
        assert_eq!(reply, "ERR READ_TEXT no pane matching missing\n");
        assert_eq!(reply.capacity(), reply.len());

        let json: serde_json::Value =
            serde_json::from_str(&read_json_no_pane_reply(" missing ")).unwrap();
        assert_eq!(json["error"], "no pane matching target");
        assert_eq!(json["target"], "missing");
    }

    #[test]
    fn read_scrollback_no_pane_reply_builds_directly() {
        let reply = read_scrollback_no_pane_reply(" missing ");
        assert_eq!(reply, "ERR READ_SCROLLBACK no pane matching missing\n");
        assert_eq!(reply.capacity(), reply.len());

        let json: serde_json::Value =
            serde_json::from_str(&read_json_no_pane_reply(" missing ")).unwrap();
        assert_eq!(json["error"], "no pane matching target");
        assert_eq!(json["target"], "missing");
    }

    #[test]
    fn wait_timeout_reply_builds_directly() {
        let reply = wait_timeout_reply("WAIT_TEXT", "native-2", 7);
        assert_eq!(
            reply,
            "ERR WAIT_TEXT timeout window=native-2 needle_bytes=7\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn tab_i16_pair_arg_builds_nudge_queue_arg_directly() {
        assert_eq!(i16_decimal_len(0), 1);
        assert_eq!(i16_decimal_len(9), 1);
        assert_eq!(i16_decimal_len(10), 2);
        assert_eq!(i16_decimal_len(-1), 2);
        assert_eq!(i16_decimal_len(i16::MIN), 6);

        let arg = tab_i16_pair_arg("focused", 3, -2);
        assert_eq!(arg, "focused\t3\t-2");
        assert_eq!(arg.capacity(), arg.len());
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
        assert!(native_spawn_queue_reply("LAYOUT grid", &pending).starts_with("LAYOUT_QUEUED"));
        assert!(
            native_spawn_queue_reply("SPLIT_PANE focused grid kittwm-top", &pending)
                .starts_with("SPLIT_PANE_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("MOVE_PANE focused last", &pending).starts_with("MOVE_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("NUDGE_PANE focused 3 -2", &pending)
                .starts_with("NUDGE_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("RESET_PANE_OFFSET focused", &pending)
                .starts_with("RESET_OFFSET_QUEUED")
        );
        assert!(native_spawn_queue_reply("RESET_ALL_PANE_OFFSETS", &pending)
            .starts_with("RESET_ALL_OFFSETS_QUEUED"));
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
            native_spawn_queue_reply("SEND_KEY native-2 shift-tab", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 shift-insert", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 ctrl-delete", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 shift-left", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 alt-left", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 ctrl-left", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 ctrl-home", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_KEY native-2 shift-page-down", &pending)
                .starts_with("SEND_KEY_QUEUED")
        );
        assert!(native_spawn_queue_reply("SEND_KEY native-2 f12", &pending)
            .starts_with("SEND_KEY_QUEUED"));
        assert!(
            native_spawn_queue_reply("SEND_MOUSE focused press-left 7 9", &pending)
                .starts_with("SEND_MOUSE_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_MOUSE focused move-left 7 9", &pending)
                .starts_with("SEND_MOUSE_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("SEND_MOUSE focused release-right 7 9", &pending)
                .starts_with("SEND_MOUSE_QUEUED")
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
            "layout": "grid",
            "panes": [
                {"title": "shell", "command": "bash", "weight": 2, "focused": false},
                {"title": "logs", "command": "tail -f app.log", "weight": 1, "focused": true},
                {"title": "top", "command": "kittwm-top", "weight": 1, "focused": false},
                {"title": "browser", "command": "kittwm-browser https://example.com", "weight": 1, "focused": false}
            ]
        });
        assert!(
            native_spawn_queue_reply(&restore_session_json_request(&manifest), &pending)
                .starts_with("RESTORE_SESSION_QUEUED")
        );
        assert!(native_spawn_queue_reply("LAYOUT diagonal", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("FOCUS_PANE", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("MOVE_PANE focused diagonal", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("NUDGE_PANE focused nope 1", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("NUDGE_PANE focused 0 0", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("RESET_PANE_OFFSET", &pending).contains("ERR"));
        assert!(
            native_spawn_queue_reply("RESET_PANE_OFFSET focused extra", &pending).contains("ERR")
        );
        assert!(native_spawn_queue_reply("RESIZE_PANE focused nope", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("RENAME_PANE native-2", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_TEXT focused", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_LINE", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_KEY focused nope", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_KEY focused page down", &pending).contains("ERR"));
        assert!(native_spawn_queue_reply("SEND_MOUSE focused drag 7 9", &pending).contains("ERR"));
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
                NativePaneCommand::Layout("grid".to_string()),
                NativePaneCommand::SplitPane {
                    window: "focused".to_string(),
                    axis: "grid".to_string(),
                    command: "kittwm-top".to_string(),
                },
                NativePaneCommand::Move {
                    window: "focused".to_string(),
                    direction: "last".to_string(),
                },
                NativePaneCommand::Nudge {
                    window: "focused".to_string(),
                    dx: 3,
                    dy: -2,
                },
                NativePaneCommand::ResetOffset {
                    window: "focused".to_string(),
                },
                NativePaneCommand::ResetAllOffsets,
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
                    window: "native-2".to_string(),
                    bytes: b"\x1b[Z".to_vec(),
                    label: "shift-tab".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[2;2~".to_vec(),
                    label: "shift-insert".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[3;5~".to_vec(),
                    label: "ctrl-delete".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[1;2D".to_vec(),
                    label: "shift-left".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[1;3D".to_vec(),
                    label: "alt-left".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[1;5D".to_vec(),
                    label: "ctrl-left".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[1;5H".to_vec(),
                    label: "ctrl-home".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[6;2~".to_vec(),
                    label: "shift-page-down".to_string(),
                },
                NativePaneCommand::SendBytes {
                    window: "native-2".to_string(),
                    bytes: b"\x1b[24~".to_vec(),
                    label: "f12".to_string(),
                },
                NativePaneCommand::SendMouse {
                    window: "focused".to_string(),
                    event: "press-left".to_string(),
                    col: 7,
                    row: 9,
                },
                NativePaneCommand::SendMouse {
                    window: "focused".to_string(),
                    event: "move-left".to_string(),
                    col: 7,
                    row: 9,
                },
                NativePaneCommand::SendMouse {
                    window: "focused".to_string(),
                    event: "release-right".to_string(),
                    col: 7,
                    row: 9,
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
                    layout: Some("grid".to_string()),
                    focus_index: Some(1),
                    panes: vec![
                        NativeSessionRestorePane {
                            title: Some("shell".to_string()),
                            command: "bash".to_string(),
                            weight: 2,
                            floating_dx: None,
                            floating_dy: None,
                            focused: false,
                        },
                        NativeSessionRestorePane {
                            title: Some("logs".to_string()),
                            command: "tail -f app.log".to_string(),
                            weight: 1,
                            floating_dx: None,
                            floating_dy: None,
                            focused: true,
                        },
                        NativeSessionRestorePane {
                            title: Some("top".to_string()),
                            command: "kittwm-top".to_string(),
                            weight: 1,
                            floating_dx: None,
                            floating_dy: None,
                            focused: false,
                        },
                        NativeSessionRestorePane {
                            title: Some("browser".to_string()),
                            command: "kittwm-browser https://example.com".to_string(),
                            weight: 1,
                            floating_dx: None,
                            floating_dy: None,
                            focused: false,
                        },
                    ],
                })
            ]
        );
    }

    #[test]
    fn send_text_queued_reply_builds_directly() {
        let reply = send_text_queued_reply("SEND_TEXT_QUEUED", 12, "focused", 5);
        assert_eq!(
            reply,
            "SEND_TEXT_QUEUED command=12 window=focused bytes=5\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn send_mouse_queued_reply_builds_directly() {
        let reply = send_mouse_queued_reply(12, "focused", "press-left", 7, 9);
        assert_eq!(
            reply,
            "SEND_MOUSE_QUEUED command=12 window=focused event=press-left col=7 row=9\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn send_key_queued_reply_builds_directly() {
        let reply = send_key_queued_reply(12, "focused", "shift-tab", 3);
        assert_eq!(
            reply,
            "SEND_KEY_QUEUED command=12 window=focused key=shift-tab bytes=3\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn send_key_unsupported_reply_builds_directly() {
        let reply = send_key_unsupported_reply();
        assert!(reply.starts_with("ERR SEND_KEY unsupported key; expected enter|return|tab"));
        assert!(reply.contains("ctrl-a..ctrl-z"));
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn base64_queued_reply_builds_directly() {
        let reply = base64_queued_reply("SEND_BYTES_B64_QUEUED", 12, "focused", 5);
        assert_eq!(
            reply,
            "SEND_BYTES_B64_QUEUED command=12 window=focused bytes=5\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn window_base64_required_reply_builds_directly() {
        let reply = window_base64_required_reply("SEND_BYTES_B64");
        assert_eq!(reply, "ERR SEND_BYTES_B64 requires window and base64\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn window_base64_invalid_reply_builds_directly() {
        let err = base64::engine::general_purpose::STANDARD
            .decode("!!!")
            .unwrap_err();
        let reply = window_base64_invalid_reply("SEND_BYTES_B64", &err);
        assert!(
            reply.starts_with("ERR SEND_BYTES_B64 invalid base64: "),
            "{reply}"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn native_spawn_queue_preserves_exact_decoded_bytes_for_send_and_paste() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        assert!(
            native_spawn_queue_reply("SEND_BYTES_B64 focused AP8bWzMxbQ==", &pending)
                .starts_with("SEND_BYTES_B64_QUEUED")
        );
        assert!(
            native_spawn_queue_reply("PASTE_BYTES_B64 native-2 AP8bWzMxbQ==", &pending)
                .starts_with("PASTE_BYTES_B64_QUEUED")
        );
        let state = pending.lock().unwrap();
        assert_eq!(state.pending.len(), 2);
        assert_eq!(
            state.pending[0],
            NativePaneCommand::SendBytes {
                window: "focused".to_string(),
                bytes: b"\0\xff\x1b[31m".to_vec(),
                label: "base64".to_string(),
            }
        );
        assert_eq!(
            state.pending[1],
            NativePaneCommand::PasteBytes {
                window: "native-2".to_string(),
                bytes: b"\0\xff\x1b[31m".to_vec(),
            }
        );
    }

    fn native_status(window: &str, focused: bool, weight: u16) -> NativePaneStatus {
        NativePaneStatus {
            window: window.to_string(),
            title: window.to_string(),
            focused,
            weight,
            stack_index: None,
            stack_top: None,
            floating_dx: None,
            floating_dy: None,
            floating_moved: None,
            title_draggable: None,
            title_drag_kind: None,
            title_drag_col: None,
            title_drag_row: None,
            title_drag_active: None,
            pid: None,
            command: Some("/bin/sh".to_string()),
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
            application_cursor_keys: Some(false),
            mouse_reporting: Some(false),
            mouse_button_motion: Some(false),
            mouse_all_motion: Some(false),
            mouse_sgr: Some(false),
            dirty_frame: None,
            text_snapshot: Some("secret live text is not serialized".to_string()),
            scrollback_snapshot: Some("secret scrollback is not serialized".to_string()),
            app_rows: None,
        }
    }

    #[test]
    fn native_pane_layout_label_builds_bounds_directly() {
        let mut pane = native_status("native-1", true, 1);
        assert_eq!(native_pane_layout_label(&pane), "-");
        pane.x = Some(1);
        pane.y = Some(2);
        pane.cols = Some(80);
        pane.rows = Some(24);
        pane.app_x = Some(2);
        pane.app_y = Some(3);
        pane.app_cols = Some(78);
        pane.app_rows = Some(22);
        assert_eq!(native_pane_layout_label(&pane), "1,2 80x24 app=2,3 78x22");
    }

    #[test]
    fn native_pane_cursor_label_builds_position_directly() {
        let mut pane = native_status("native-1", true, 1);
        assert_eq!(native_pane_cursor_label(&pane), "-");
        pane.cursor_col = Some(12);
        pane.cursor_row = Some(34);
        assert_eq!(native_pane_cursor_label(&pane), "12,34");
    }

    #[test]
    fn native_pane_mouse_label_builds_enabled_modes_directly() {
        let mut pane = native_status("native-1", true, 1);
        assert_eq!(native_pane_mouse_label(&pane), "-");
        pane.mouse_reporting = Some(true);
        pane.mouse_all_motion = Some(true);
        pane.mouse_sgr = Some(true);
        assert_eq!(native_pane_mouse_label(&pane), "basic,all-motion,sgr");
    }

    #[test]
    fn native_surface_events_publish_explicit_event_kinds() {
        let mut state = NativeSpawnQueueState::default();
        publish_native_surface_events(
            &mut state,
            "native-1".to_string(),
            vec![
                SurfaceEvent::TitleChanged("editor".to_string()),
                SurfaceEvent::Bell {
                    visual: true,
                    audible: false,
                },
                SurfaceEvent::ClipboardSet {
                    selection: "c".to_string(),
                    payload_base64: "aGVsbG8=".to_string(),
                },
                SurfaceEvent::Notification {
                    title: "build".to_string(),
                    body: "done".to_string(),
                },
            ],
        );
        let events = state.events.into_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0]["kind"], "surface_title_changed");
        assert_eq!(events[0]["window"], "native-1");
        assert_eq!(events[0]["detail"]["title"], "editor");
        assert_eq!(events[1]["kind"], "surface_bell");
        assert_eq!(events[1]["detail"]["visual"], true);
        assert_eq!(events[1]["detail"]["audible"], false);
        assert_eq!(events[2]["kind"], "surface_clipboard_set");
        assert_eq!(events[2]["detail"]["payload_base64"], "aGVsbG8=");
        assert_eq!(events[3]["kind"], "surface_notification");
        assert_eq!(events[3]["detail"]["body"], "done");
    }

    #[test]
    fn native_clipboard_json_is_policy_gated_and_cache_only() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        {
            let mut state = pending.lock().unwrap();
            publish_native_surface_events(
                &mut state,
                "native-1".to_string(),
                vec![SurfaceEvent::ClipboardSet {
                    selection: "clipboard".to_string(),
                    payload_base64: "aGVsbG8=".to_string(),
                }],
            );
        }

        let denied: serde_json::Value =
            serde_json::from_str(&native_clipboard_json_reply_with_policy(&pending, false))
                .unwrap();
        assert_eq!(denied["allowed"], false);
        assert_eq!(denied["available"], false);
        assert!(denied.get("payload_base64").is_none());

        let allowed: serde_json::Value =
            serde_json::from_str(&native_clipboard_json_reply_with_policy(&pending, true)).unwrap();
        assert_eq!(allowed["allowed"], true);
        assert_eq!(allowed["available"], true);
        assert_eq!(allowed["source_window"], "native-1");
        assert_eq!(allowed["selection"], "clipboard");
        assert_eq!(allowed["payload_base64"], "aGVsbG8=");
        assert_eq!(allowed["payload_bytes"], 5);
        assert_eq!(allowed["source"], "osc52-cache");
    }

    #[test]
    fn native_clipboard_json_reports_empty_allowed_cache() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let allowed: serde_json::Value =
            serde_json::from_str(&native_clipboard_json_reply_with_policy(&pending, true)).unwrap();
        assert_eq!(allowed["allowed"], true);
        assert_eq!(allowed["available"], false);
        assert!(allowed.get("payload_base64").is_none());
    }

    #[test]
    fn native_frame_presented_event_reports_metadata_without_payload() {
        let mut state = NativeSpawnQueueState::default();
        publish_native_frame_presented_event(
            &mut state,
            "native-1".to_string(),
            NativeFramePresented {
                renderer: "kitty".to_string(),
                format: "rgba".to_string(),
                pixel_width: 640,
                pixel_height: 384,
                app_x: Some(0),
                app_y: Some(1),
                app_cols: Some(80),
                app_rows: Some(24),
                uploaded: false,
                skipped_upload: true,
                changed_tiles: Some(0),
                total_tiles: Some(120),
                upload_bytes: Some(0),
                placement_bytes: Some(12),
                embed_bytes: Some(0),
                elapsed_us: Some(321),
            },
        );
        let event = state.events.pop_front().expect("frame event");
        assert_eq!(event["kind"], "pane_frame_presented");
        assert_eq!(event["window"], "native-1");
        assert_eq!(event["detail"]["renderer"], "kitty");
        assert_eq!(event["detail"]["format"], "rgba");
        assert_eq!(event["detail"]["pixel_width"], 640);
        assert_eq!(event["detail"]["pixel_height"], 384);
        assert_eq!(event["detail"]["app_bounds"]["cols"], 80);
        assert_eq!(event["detail"]["uploaded"], false);
        assert_eq!(event["detail"]["skipped_upload"], true);
        assert_eq!(event["detail"]["changed_tiles"], 0);
        assert_eq!(event["detail"]["total_tiles"], 120);
        assert_eq!(event["detail"]["upload_bytes"], 0);
        assert_eq!(event["detail"]["placement_bytes"], 12);
        assert_eq!(event["detail"]["embed_bytes"], 0);
        assert_eq!(event["detail"]["elapsed_us"], 321);
        assert!(!event.to_string().contains("rgba_bytes"));
        assert_eq!(event["schema_version"], NATIVE_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn native_input_events_omit_sensitive_payloads() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let text = native_spawn_queue_reply("SEND_TEXT focused super-secret", &pending);
        assert!(text.starts_with("SEND_TEXT_QUEUED"), "{text}");
        let key = native_spawn_queue_reply("SEND_KEY focused enter", &pending);
        assert!(key.starts_with("SEND_KEY_QUEUED"), "{key}");
        let paste = native_spawn_queue_reply("PASTE_BYTES_B64 focused c2VjcmV0LXBhc3Rl", &pending);
        assert!(paste.starts_with("PASTE_BYTES_B64_QUEUED"), "{paste}");
        let mouse = native_spawn_queue_reply("SEND_MOUSE focused press-left 3 4", &pending);
        assert!(mouse.starts_with("SEND_MOUSE_QUEUED"), "{mouse}");

        let state = pending.lock().unwrap();
        let input_events = state
            .events
            .iter()
            .filter(|event| event["kind"] == "pane_input_sent")
            .collect::<Vec<_>>();
        assert_eq!(input_events.len(), 4);
        assert_eq!(input_events[0]["window"], "focused");
        assert_eq!(input_events[0]["detail"]["input"], "text");
        assert_eq!(input_events[0]["detail"]["bytes"], 12);
        assert!(!input_events[0].to_string().contains("super-secret"));
        assert_eq!(input_events[1]["detail"]["input"], "key");
        assert_eq!(input_events[1]["detail"]["key"], "enter");
        assert_eq!(input_events[2]["detail"]["input"], "paste");
        assert_eq!(input_events[2]["detail"]["bytes"], 12);
        assert!(!input_events[2].to_string().contains("secret-paste"));
        assert_eq!(input_events[3]["detail"]["input"], "mouse");
        assert_eq!(input_events[3]["detail"]["event"], "press-left");
        assert_eq!(input_events[3]["detail"]["col"], 3);
        assert_eq!(input_events[3]["detail"]["row"], 4);
    }

    #[test]
    fn native_pane_resize_event_reports_old_and_new_bounds() {
        let mut state = NativeSpawnQueueState::default();
        let mut old = native_status("native-1", true, 1);
        old.x = Some(0);
        old.y = Some(0);
        old.cols = Some(80);
        old.rows = Some(24);
        old.app_x = Some(0);
        old.app_y = Some(1);
        old.app_cols = Some(80);
        old.app_rows = Some(23);
        publish_native_pane_events(&mut state, vec![old.clone()]);
        state.events.clear();

        let mut new = old;
        new.cols = Some(100);
        new.app_cols = Some(100);
        publish_native_pane_events(&mut state, vec![new]);
        let events = state.events.into_iter().collect::<Vec<_>>();
        assert!(events.iter().any(|event| event["kind"] == "pane_changed"));
        let resized = events
            .iter()
            .find(|event| event["kind"] == "pane_resized")
            .expect("pane_resized event");
        assert_eq!(resized["window"], "native-1");
        assert_eq!(resized["detail"]["old"]["bounds"]["cols"], 80);
        assert_eq!(resized["detail"]["new"]["bounds"]["cols"], 100);
        assert_eq!(resized["detail"]["old"]["app_bounds"]["cols"], 80);
        assert_eq!(resized["detail"]["new"]["app_bounds"]["cols"], 100);
    }

    #[test]
    fn native_spawn_queue_streams_status_and_change_events() {
        let p = tmp_sock().with_file_name(test_socket_filename(
            "kittwm-native-events",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&p);
        let queue = NativeSpawnQueue::bind(p).unwrap();
        queue.update_layout("columns");
        queue.update_panes(vec![native_status("native-1", true, 1)]);

        let stream_registered_seq = queue
            .pending
            .lock()
            .unwrap()
            .next_event_seq
            .saturating_add(1);
        let path = queue.path().to_path_buf();
        let reader = std::thread::spawn(move || client_request_multi(&path, "EVENTS 300").unwrap());
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            if queue.pending.lock().unwrap().next_event_seq >= stream_registered_seq {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            queue.pending.lock().unwrap().next_event_seq >= stream_registered_seq,
            "EVENTS reader did not register before test updates"
        );
        queue.update_panes(vec![
            native_status("native-1", false, 1),
            native_status("native-2", true, 3),
        ]);
        queue.update_layout("rows");

        let stream = reader.join().unwrap();
        let events = stream
            .lines()
            .filter(|line| line.trim_start().starts_with('{'))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(
            events.iter().any(|event| event["kind"] == "status"
                && event["window"].as_str().is_some_and(
                    |window| window == event["detail"]["focus"].as_str().unwrap_or("")
                )),
            "{stream}"
        );
        assert!(
            events
                .iter()
                .any(|event| event["kind"] == "pane_opened" && event["window"] == "native-2"),
            "{stream}"
        );
        assert!(
            events
                .iter()
                .any(|event| event["kind"] == "focus_changed"
                    && event["detail"]["focus"] == "native-2"),
            "{stream}"
        );
        assert!(
            events
                .iter()
                .any(|event| event["kind"] == "layout_changed"
                    && event["detail"]["layout"] == "rows"),
            "{stream}"
        );
        assert!(
            events
                .iter()
                .all(|event| event["schema_version"] == NATIVE_EVENT_SCHEMA_VERSION),
            "{stream}"
        );
        assert!(
            events
                .iter()
                .all(|event| !event["detail"].to_string().contains("secret live text")),
            "{stream}"
        );
    }

    #[test]
    fn native_spawn_queue_read_text_round_trip_over_socket() {
        let p = tmp_sock().with_file_name(test_socket_filename(
            "kittwm-native-read",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&p);
        let queue = NativeSpawnQueue::bind(p).unwrap();
        queue.update_panes(vec![NativePaneStatus {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            stack_index: Some(0),
            stack_top: Some(true),
            floating_dx: Some(0),
            floating_dy: Some(0),
            floating_moved: None,
            title_draggable: Some(false),
            title_drag_kind: None,
            title_drag_col: None,
            title_drag_row: None,
            title_drag_active: None,
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
            application_cursor_keys: Some(false),
            mouse_reporting: Some(false),
            mouse_button_motion: Some(false),
            mouse_all_motion: Some(false),
            mouse_sgr: Some(false),
            dirty_frame: None,
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
        let p = tmp_sock().with_file_name(test_socket_filename(
            "kittwm-native-concurrent",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&p);
        let queue = NativeSpawnQueue::bind(p).unwrap();
        queue.update_panes(vec![NativePaneStatus {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            stack_index: Some(0),
            stack_top: Some(true),
            floating_dx: Some(0),
            floating_dy: Some(0),
            floating_moved: None,
            title_draggable: Some(false),
            title_drag_kind: None,
            title_drag_col: None,
            title_drag_row: None,
            title_drag_active: None,
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
            application_cursor_keys: Some(false),
            mouse_reporting: Some(false),
            mouse_button_motion: Some(false),
            mouse_all_motion: Some(false),
            mouse_sgr: Some(false),
            dirty_frame: None,
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
        assert!(waited.contains("ERR WAIT_TEXT_MS timeout"), "{waited}");
    }

    #[test]
    fn native_spawn_queue_serves_help_catalogs() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let help = native_spawn_queue_reply("HELP", &pending);
        assert!(help.contains("SPAWN_PTY <cmd>"), "{help}");
        assert!(help.contains("STATUS_JSON"), "{help}");
        assert!(help.contains("PANES_JSON"), "{help}");
        assert!(help.contains("SESSION_JSON"), "{help}");
        assert!(help.contains("SHORTCUTS_JSON"), "{help}");
        assert!(help.contains("EVENTS [ms]"), "{help}");
        assert!(help.contains("FOCUS_NEXT"), "{help}");
        assert!(help.contains("FOCUS_PREV"), "{help}");
        assert!(help.contains("LAYOUT <columns|rows|grid>"), "{help}");
        assert!(help.contains("MOVE_PANE <window|focused>"), "{help}");
        assert!(
            help.contains("NUDGE_PANE <window|focused> <dx> <dy>"),
            "{help}"
        );
        assert!(
            help.contains("RESET_PANE_OFFSET <window|focused>"),
            "{help}"
        );
        assert!(help.contains("RESET_ALL_PANE_OFFSETS"), "{help}");
        assert!(help.contains("RESIZE_PANE <window|focused>"), "{help}");
        assert!(help.contains("BALANCE_PANES"), "{help}");
        assert!(help.contains("RESTORE_SESSION_JSON <json>"), "{help}");
        assert!(help.contains("RESERVE_CHROME_JSON <json>"), "{help}");
        assert!(help.contains("RENAME_PANE <window> <title>"), "{help}");
        assert!(help.contains("SEND_TEXT <window|focused> <text>"), "{help}");
        assert!(help.contains("SEND_LINE <window|focused> <text>"), "{help}");
        assert!(help.contains("SEND_KEY <window|focused> <key>"), "{help}");
        assert!(help.contains("shift-tab|backtab"), "{help}");
        assert!(help.contains("shift/alt/ctrl arrows"), "{help}");
        assert!(
            help.contains("shift/alt/ctrl home/end/page-up/page-down"),
            "{help}"
        );
        assert!(help.contains("f5..f12"), "{help}");
        assert!(
            help.contains("SEND_MOUSE <window|focused> <event> <col> <row>"),
            "{help}"
        );
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
                entry["command"] == "LAYOUT <columns|rows|grid>" && entry["category"] == "control"
            }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "EVENTS [ms]" && entry["category"] == "events" }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "NUDGE_PANE <window|focused> <dx> <dy>"
                    && entry["category"] == "control"
            }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "RESET_PANE_OFFSET <window|focused>"
                    && entry["category"] == "control"
            }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "RESET_ALL_PANE_OFFSETS" && entry["category"] == "control"
            }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "SEMANTIC_SNAPSHOT <window|focused>"
                    && entry["category"] == "semantic"
            }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "SHORTCUTS_JSON" && entry["category"] == "inspect"
            }));
        assert!(help_json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["command"] == "SEND_KEY <window|focused> <key>"
                    && entry["description"].as_str().is_some_and(|description| {
                        description.contains("shift-tab|backtab")
                            && description.contains("shift/alt/ctrl arrows")
                            && description.contains("shift/alt/ctrl home/end/page-up/page-down")
                            && description.contains("f5..f12")
                    })
            }));
    }

    #[test]
    fn native_spawn_queue_serves_shortcuts_json() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let value: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("SHORTCUTS_JSON", &pending)).unwrap();
        assert_eq!(value["kind"], "kittwm-native-shortcuts");
        let shortcuts = value["shortcuts"].as_array().unwrap();
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "launch_terminal"));
        assert!(shortcuts.iter().any(|entry| entry["id"] == "toggle_help"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "switch_workspace"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["keys"] == "Ctrl-C×3 then y / Ctrl-]"));
    }

    #[test]
    fn semantic_published_reply_builds_directly() {
        let reply = semantic_published_reply("native-1");
        assert_eq!(reply, "SEMANTIC_PUBLISHED window=native-1\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn semantic_no_pane_reply_builds_directly() {
        let reply = semantic_no_pane_reply("SEMANTIC_ACTION", " missing ");
        assert_eq!(reply, "ERR SEMANTIC_ACTION no pane matching missing\n");
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn semantic_publish_surface_mismatch_reply_builds_directly() {
        let reply = semantic_publish_surface_mismatch_reply("native-1", "native-2");
        assert_eq!(
            reply,
            "ERR SEMANTIC_PUBLISH surface mismatch window=native-1 snapshot=native-2\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn apps_first_replies_build_directly() {
        let m = apps_first_match_reply("path", "htop");
        assert_eq!(m, "APPS_FIRST kind=path name=htop\n");
        assert_eq!(m.capacity(), m.len());

        let l = apps_launch_first_reply(4321, "macos", "Safari");
        assert_eq!(l, "APPS_LAUNCH_FIRST pid=4321 kind=macos name=Safari\n");
        assert_eq!(l.capacity(), l.len());

        let e = apps_launch_error_reply("path", "htop", "No such file");
        assert_eq!(e, "ERR launch path:htop: No such file\n");
        assert_eq!(e.capacity(), e.len());
    }

    #[test]
    fn unknown_command_help_lists_apps_first_commands() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let reply = native_spawn_queue_reply("NOT_A_REAL_COMMAND", &pending);
        assert!(reply.starts_with("ERR expected "), "{reply}");
        assert!(reply.contains("APPS_FIRST <query>"), "{reply}");
        assert!(reply.contains("APPS_LAUNCH_FIRST <query>"), "{reply}");
    }

    #[test]
    fn native_spawn_bare_apps_first_returns_clean_error() {
        // Consistency with the standalone daemon: bare APPS_FIRST/APPS_LAUNCH_FIRST
        // return a recognized requires-query error rather than the full help dump.
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        assert_eq!(
            native_spawn_queue_reply("APPS_FIRST", &pending),
            "ERR APPS_FIRST requires a query\n"
        );
        assert_eq!(
            native_spawn_queue_reply("APPS_LAUNCH_FIRST", &pending),
            "ERR APPS_FIRST requires a query\n"
        );
    }

    #[test]
    fn unknown_command_help_lists_core_commands() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let reply = native_spawn_queue_reply("NOT_A_REAL_COMMAND", &pending);
        for needle in [
            " PING ",
            " STATUS ",
            " PANES ",
            "SPLIT_PANE <window|focused> <columns|rows|grid> <cmd>",
        ] {
            assert!(reply.contains(needle), "missing {needle:?} in: {reply}");
        }
    }

    #[test]
    fn unknown_command_help_covers_help_catalog() {
        // Regression guard: every command in the structured HELP catalog must also
        // appear in the unknown-command error help so the two discoverability
        // surfaces cannot drift (prevents recurrence of the APPS_FIRST/PING/STATUS
        // /PANES/SPLIT_PANE gaps).
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let reply = native_spawn_queue_reply("NOT_A_REAL_COMMAND", &pending);
        for (command, _category, _description) in native_spawn_help_entries() {
            let token = command.split_whitespace().next().unwrap_or(command);
            assert!(
                reply.contains(token),
                "unknown-command help is missing catalog command {token:?}; reply: {reply}"
            );
        }
    }

    #[test]
    fn help_catalog_has_no_empty_or_duplicate_entries() {
        // Quality guard: every HELP catalog entry must carry a non-empty command,
        // category, and description, and command keywords must be unique so
        // HELP/HELP_JSON cannot ship malformed or duplicated rows.
        let mut seen = std::collections::HashSet::new();
        for (command, category, description) in native_spawn_help_entries() {
            assert!(!command.trim().is_empty(), "empty command entry");
            assert!(
                !category.trim().is_empty(),
                "empty category for {command:?}"
            );
            assert!(
                !description.trim().is_empty(),
                "empty description for {command:?}"
            );
            let keyword = command.split_whitespace().next().unwrap_or(command);
            assert!(
                seen.insert(keyword),
                "duplicate HELP catalog command keyword {keyword:?}"
            );
        }
    }

    #[test]
    fn semantic_success_replies_build_directly() {
        let action = semantic_action_applied_reply("native-1", " settings.notify ", " toggle ");
        assert_eq!(
            action,
            "SEMANTIC_ACTION_APPLIED window=native-1 component=settings.notify action=toggle\n"
        );
        assert_eq!(action.capacity(), action.len());

        let focus = semantic_focused_reply("native-1", " settings.name ");
        assert_eq!(
            focus,
            "SEMANTIC_FOCUSED window=native-1 component=settings.name\n"
        );
        assert_eq!(focus.capacity(), focus.len());
    }

    #[test]
    fn semantic_unsupported_replies_build_directly() {
        let action = semantic_action_unsupported_reply("native-1", " settings.notify ", " toggle ");
        assert_eq!(
            action,
            "ERR SEMANTIC_ACTION unsupported window=native-1 component=settings.notify action=toggle\n"
        );
        assert_eq!(action.capacity(), action.len());

        let focus = semantic_focus_unsupported_reply("native-1", " settings.name ");
        assert_eq!(
            focus,
            "ERR SEMANTIC_FOCUS unsupported window=native-1 component=settings.name\n"
        );
        assert_eq!(focus.capacity(), focus.len());
    }

    #[test]
    fn semantic_failure_replies_build_directly() {
        let action = semantic_action_failed_reply(
            "native-1",
            " settings.notify ",
            " toggle ",
            "component not found",
        );
        assert_eq!(
            action,
            "ERR SEMANTIC_ACTION window=native-1 component=settings.notify action=toggle component not found\n"
        );
        assert_eq!(action.capacity(), action.len());

        let focus =
            semantic_focus_failed_reply("native-1", " settings.name ", "component not found");
        assert_eq!(
            focus,
            "ERR SEMANTIC_FOCUS window=native-1 component=settings.name component not found\n"
        );
        assert_eq!(focus.capacity(), focus.len());
    }

    #[test]
    fn semantic_component_id_builds_directly() {
        let id = semantic_component_id("native-1", "screen");
        assert_eq!(id, "native-1.screen");
        assert_eq!(id.capacity(), id.len());
    }

    #[test]
    fn native_spawn_queue_serves_semantic_snapshot_skeleton() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        pending.lock().unwrap().panes = vec![native_status("native-1", true, 1)];
        let reply = native_spawn_queue_reply("SEMANTIC_SNAPSHOT focused", &pending);
        let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["surface"], "native-1");
        assert_eq!(value["root"]["role"], "group");
        assert_eq!(value["root"]["children"][0]["role"], "text_area");
        assert_eq!(value["root"]["children"][0]["value"]["kind"], "text");
        assert_eq!(value["root"]["children"][0]["state"]["focused"], true);
        assert_eq!(value["focus"], "native-1.screen");
    }

    fn semantic_publish_request(target: &str, snapshot: &SemanticSurfaceSnapshot) -> String {
        let snapshot = serde_json::to_string(snapshot).unwrap();
        let mut request =
            String::with_capacity("SEMANTIC_PUBLISH  ".len() + target.len() + snapshot.len());
        request.push_str("SEMANTIC_PUBLISH ");
        request.push_str(target);
        request.push(' ');
        request.push_str(&snapshot);
        request
    }

    #[test]
    fn semantic_publish_request_builds_directly() {
        let snapshot = SemanticSurfaceSnapshot::new(
            "native-1",
            42,
            ComponentNode::new("native-1.button", ComponentRole::Button).labeled("Run"),
        );
        let request = semantic_publish_request("focused", &snapshot);
        assert!(
            request.starts_with("SEMANTIC_PUBLISH focused {"),
            "{request}"
        );
        assert!(request.contains("\"surface\":\"native-1\""), "{request}");
        assert_eq!(request.capacity(), request.len());
    }

    #[test]
    fn native_spawn_queue_publishes_and_reads_semantic_snapshot() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        pending.lock().unwrap().panes = vec![native_status("native-1", true, 1)];
        let snapshot = SemanticSurfaceSnapshot::new(
            "native-1",
            42,
            ComponentNode::new("native-1.button", ComponentRole::Button).labeled("Run"),
        );
        let publish =
            native_spawn_queue_reply(&semantic_publish_request("focused", &snapshot), &pending);
        assert_eq!(publish, "SEMANTIC_PUBLISHED window=native-1\n");
        let reply = native_spawn_queue_reply("SEMANTIC_SNAPSHOT native-1", &pending);
        let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(value["revision"], 42);
        assert_eq!(value["root"]["role"], "button");
        assert_eq!(value["root"]["label"], "Run");
    }

    #[test]
    fn native_spawn_queue_rejects_invalid_semantic_publish() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        pending.lock().unwrap().panes = vec![native_status("native-1", true, 1)];
        assert_eq!(
            native_spawn_queue_reply("SEMANTIC_PUBLISH focused not-json", &pending),
            "ERR SEMANTIC_PUBLISH snapshot must be JSON\n"
        );
        let mismatch = SemanticSurfaceSnapshot::new(
            "native-2",
            1,
            ComponentNode::new("native-2.root", ComponentRole::Group),
        );
        let reply = native_spawn_queue_reply(
            &format!(
                "SEMANTIC_PUBLISH focused {}",
                serde_json::to_string(&mismatch).unwrap()
            ),
            &pending,
        );
        assert!(
            reply.starts_with("ERR SEMANTIC_PUBLISH surface mismatch"),
            "{reply}"
        );
    }

    #[test]
    fn native_spawn_queue_rejects_fallback_semantic_action_and_focus_until_supported() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        pending.lock().unwrap().panes = vec![native_status("native-1", true, 1)];
        let action = native_spawn_queue_reply(
            "SEMANTIC_ACTION focused native-1.screen insert_text {\"text\":\"hi\"}",
            &pending,
        );
        assert!(
            action.starts_with("ERR SEMANTIC_ACTION unsupported window=native-1"),
            "{action}"
        );
        let bad_json = native_spawn_queue_reply(
            "SEMANTIC_ACTION focused native-1.screen insert_text not-json",
            &pending,
        );
        assert_eq!(bad_json, "ERR SEMANTIC_ACTION payload must be JSON\n");
        let focus = native_spawn_queue_reply("SEMANTIC_FOCUS focused native-1.screen", &pending);
        assert!(
            focus.starts_with("ERR SEMANTIC_FOCUS unsupported window=native-1"),
            "{focus}"
        );
    }

    #[test]
    fn native_spawn_queue_routes_published_semantic_focus_and_actions() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        pending.lock().unwrap().panes = vec![native_status("native-1", true, 1)];
        let snapshot = SemanticSurfaceSnapshot::new(
            "native-1",
            1,
            ComponentNode::new("settings", ComponentRole::Group).children(vec![
                ComponentNode::new("settings.name", ComponentRole::TextInput)
                    .valued(ComponentValue::Text("Ada".to_string())),
                ComponentNode::new("settings.notify", ComponentRole::Checkbox)
                    .valued(ComponentValue::Bool(false)),
                ComponentNode::new("settings.profile", ComponentRole::SelectList).children(vec![
                    ComponentNode::new("settings.profile.dev", ComponentRole::Label),
                    ComponentNode::new("settings.profile.ops", ComponentRole::Label),
                ]),
            ]),
        );
        assert_eq!(
            native_spawn_queue_reply(
                &format!(
                    "SEMANTIC_PUBLISH focused {}",
                    serde_json::to_string(&snapshot).unwrap()
                ),
                &pending,
            ),
            "SEMANTIC_PUBLISHED window=native-1\n"
        );
        assert_eq!(
            native_spawn_queue_reply("SEMANTIC_FOCUS focused settings.name", &pending),
            "SEMANTIC_FOCUSED window=native-1 component=settings.name\n"
        );
        assert_eq!(
            native_spawn_queue_reply(
                "SEMANTIC_ACTION focused settings.notify toggle {}",
                &pending,
            ),
            "SEMANTIC_ACTION_APPLIED window=native-1 component=settings.notify action=toggle\n"
        );
        assert_eq!(
            native_spawn_queue_reply(
                "SEMANTIC_ACTION focused settings.name set {\"text\":\"Grace\"}",
                &pending,
            ),
            "SEMANTIC_ACTION_APPLIED window=native-1 component=settings.name action=set\n"
        );
        assert_eq!(
            native_spawn_queue_reply(
                "SEMANTIC_ACTION focused settings.profile select {\"id\":\"settings.profile.ops\"}",
                &pending,
            ),
            "SEMANTIC_ACTION_APPLIED window=native-1 component=settings.profile action=select\n"
        );
        let reply = native_spawn_queue_reply("SEMANTIC_SNAPSHOT focused", &pending);
        let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
        let children = value["root"]["children"].as_array().unwrap();
        assert_eq!(value["focus"], "settings.name");
        assert_eq!(
            children[0]["value"],
            serde_json::json!({"kind":"text","value":"Grace"})
        );
        assert_eq!(children[0]["state"]["focused"], true);
        assert_eq!(
            children[1]["value"],
            serde_json::json!({"kind":"bool","value":true})
        );
        assert_eq!(children[1]["state"]["checked"], true);
        assert_eq!(children[2]["children"][1]["state"]["selected"], true);

        let events = pending
            .lock()
            .unwrap()
            .events
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert!(events
            .iter()
            .any(|event| event["kind"] == "semantic_snapshot_ready"));
        assert!(events
            .iter()
            .any(|event| event["kind"] == "semantic_focus_changed"));
        assert!(events
            .iter()
            .any(|event| event["kind"] == "semantic_action_invoked"));
        assert!(events
            .iter()
            .any(|event| event["kind"] == "semantic_value_changed"
                && event["detail"]["component"] == "settings.notify"));
        assert!(events
            .iter()
            .all(|event| event["schema_version"] == NATIVE_EVENT_SCHEMA_VERSION));
    }

    #[test]
    fn daemon_json_string_array_escapes_items_without_joining_rows() {
        assert_eq!(json_string_array(&[]), "");
        assert_eq!(
            json_string_array(&["alpha".to_string(), "quote \" item".to_string()]),
            r#""alpha", "quote \" item""#
        );
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
    fn app_candidate_filter_matches_ascii_case_without_lowercasing_items() {
        let huge = format!(
            "{}KittWM-Terminal{}",
            "x".repeat(10_000),
            "y".repeat(10_000)
        );
        let filtered = filter_candidates(
            vec![
                "Browser.app".to_string(),
                huge.clone(),
                "Ghostty".to_string(),
            ],
            Some("wm-terminal"),
            2,
        );
        assert_eq!(filtered, vec![huge]);
        assert!(ascii_casefold_contains_lower("RésuméNeedle", "needle"));
        assert!(!ascii_casefold_contains_lower("Résumé", "needle"));
    }

    #[test]
    fn app_candidate_filter_preserves_limit_and_empty_query() {
        let items = vec!["A".to_string(), "b".to_string(), "C".to_string()];
        assert_eq!(filter_candidates(items.clone(), None, 2), vec!["A", "b"]);
        assert_eq!(filter_candidates(items, Some(""), 2), vec!["A", "b"]);
    }

    #[test]
    fn native_chrome_json_honors_workspace_env_label() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WORKSPACE", " dev ");
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let chrome: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("CHROME_JSON", &pending)).unwrap();
        assert_eq!(chrome["workspace"], "dev");
        assert_eq!(chrome["top_bar_rows"], 1);
        assert!(chrome["tilable_rows"].is_null());
        let status: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("STATUS_JSON", &pending)).unwrap();
        assert_eq!(status["workspace"], "dev");
        assert_eq!(status["chrome"]["workspace"], "dev");
        std::env::set_var("KITTWM_WORKSPACE", "   ");
        assert_eq!(native_workspace_id(), "1");
        std::env::remove_var("KITTWM_WORKSPACE");
    }

    #[test]
    fn json_value_line_builds_directly() {
        let value = serde_json::json!({"chrome":"ok"});
        let line = json_value_line(&value);
        assert_eq!(line, "{\"chrome\":\"ok\"}\n");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn native_chrome_json_prefers_published_workspace_label() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WORKSPACE", " env ");
        let mut state = NativeSpawnQueueState {
            workspace: Some(" 4 ".to_string()),
            ..Default::default()
        };
        assert_eq!(native_workspace_id_for_state(&state), "4");
        let chrome = native_chrome_status_value(&state);
        assert_eq!(chrome["workspace"], "4");
        state.workspace = Some("   ".to_string());
        assert_eq!(native_workspace_id_for_state(&state), "env");
        std::env::remove_var("KITTWM_WORKSPACE");
    }

    #[test]
    fn chrome_reserved_reply_builds_directly() {
        let reservation = NativeChromeReservationConfig {
            top_bar_rows: 2,
            bottom_bar_rows: 1,
            left_cols: 4,
            right_cols: 3,
            gap_cols: 1,
            gap_rows: 2,
            owner: Some("bar".to_string()),
        };
        let reply = chrome_reserved_reply(&reservation);
        assert_eq!(
            reply,
            "CHROME_RESERVED {\"top_bar_rows\":2,\"bottom_bar_rows\":1,\"left_cols\":4,\"right_cols\":3,\"gap_cols\":1,\"gap_rows\":2,\"owner\":\"bar\"}\n"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn reserve_chrome_invalid_json_reply_builds_directly() {
        let err = serde_json::from_str::<NativeChromeReservationConfig>("{").unwrap_err();
        let reply = reserve_chrome_invalid_json_reply(&err);
        assert!(
            reply.starts_with("ERR RESERVE_CHROME_JSON invalid json: EOF while parsing"),
            "{reply}"
        );
        assert_eq!(reply.capacity(), reply.len());
    }

    #[test]
    fn native_chrome_reservation_json_updates_drawable_contract() {
        let pending = Arc::new(Mutex::new(NativeSpawnQueueState::default()));
        let reply = native_spawn_queue_reply(
            r#"RESERVE_CHROME_JSON {"top_bar_rows":2,"bottom_bar_rows":1,"left_cols":4,"right_cols":3,"gap_cols":1,"gap_rows":2,"owner":" bar "}"#,
            &pending,
        );
        assert!(reply.starts_with("CHROME_RESERVED "), "{reply}");
        let chrome: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("CHROME_JSON", &pending)).unwrap();
        assert_eq!(chrome["top_bar_rows"], 2);
        assert_eq!(chrome["bottom_bar_rows"], 1);
        assert_eq!(chrome["left_cols"], 4);
        assert_eq!(chrome["right_cols"], 3);
        assert_eq!(chrome["gap_cols"], 1);
        assert_eq!(chrome["gap_rows"], 2);
        assert_eq!(chrome["owner"], "bar");
        let reply = native_spawn_queue_reply(
            r#"RESERVE_CHROME_JSON {"top_bar_rows":1,"owner":"   "}"#,
            &pending,
        );
        assert!(reply.starts_with("CHROME_RESERVED "), "{reply}");
        let chrome: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("CHROME_JSON", &pending)).unwrap();
        assert_eq!(chrome["owner"], serde_json::Value::Null);
        let events = pending
            .lock()
            .unwrap()
            .events
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .any(|event| event["kind"] == "chrome_reservation_changed"),
            "{events:?}"
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
                stack_index: Some(0),
                stack_top: Some(false),
                floating_dx: Some(0),
                floating_dy: Some(0),
                floating_moved: Some(false),
                title_draggable: Some(false),
                title_drag_kind: None,
                title_drag_col: None,
                title_drag_row: None,
                title_drag_active: None,
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
                application_cursor_keys: Some(true),
                mouse_reporting: Some(true),
                mouse_button_motion: Some(true),
                mouse_all_motion: Some(false),
                mouse_sgr: Some(true),
                dirty_frame: None,
                text_snapshot: Some("shell line\n".to_string()),
                scrollback_snapshot: Some("shell history\n".to_string()),
                app_rows: Some(23),
            },
            NativePaneStatus {
                window: "native-2".to_string(),
                title: "htop".to_string(),
                focused: true,
                weight: 3,
                stack_index: Some(1),
                stack_top: Some(true),
                floating_dx: Some(3),
                floating_dy: Some(-2),
                floating_moved: Some(true),
                title_draggable: Some(true),
                title_drag_kind: Some("reposition".to_string()),
                title_drag_col: Some(44),
                title_drag_row: Some(1),
                title_drag_active: Some(true),
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
                application_cursor_keys: Some(false),
                mouse_reporting: Some(false),
                mouse_button_motion: Some(false),
                mouse_all_motion: Some(false),
                mouse_sgr: Some(false),
                dirty_frame: None,
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
            panes.contains("window=native-1 focused=false weight=1 stack=0 top=off floating=0,0 moved=off title_draggable=off title_drag_kind=- title_drag=- title_drag_active=- pid=101 command=Some(\"/bin/sh\") cursor=4,1 cursor_visible=on bracketed_paste=on app_cursor=on mouse=basic,button-motion,sgr layout=0,0 40x24 app=0,1 40x23 title=\"shell\""),
            "{panes}"
        );
        assert!(
            panes.contains("window=native-2 focused=true weight=3 stack=1 top=on floating=3,-2 moved=on title_draggable=on title_drag_kind=reposition title_drag=44,1 title_drag_active=on pid=202 command=Some(\"htop\") cursor=12,2 cursor_visible=off bracketed_paste=off app_cursor=off mouse=- layout=40,0 80x24 app=40,1 80x23 title=\"htop\""),
            "{panes}"
        );
        let chrome_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("CHROME_JSON", &pending)).unwrap();
        assert_eq!(chrome_json["workspace"], "1");
        assert_eq!(chrome_json["top_bar_rows"], 1);
        assert_eq!(chrome_json["tilable_rows"], 23);

        let status_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("STATUS_JSON", &pending)).unwrap();
        assert_eq!(status_json["pending"], 0);
        assert_eq!(status_json["panes"], 2);
        assert_eq!(status_json["focus"], "native-2");
        assert_eq!(status_json["layout"], "rows");
        assert_eq!(status_json["workspace"], "1");
        assert_eq!(status_json["chrome"]["workspace"], "1");
        assert_eq!(status_json["chrome"]["top_bar_rows"], 1);
        assert_eq!(status_json["chrome"]["tilable_rows"], 23);
        assert_eq!(status_json["focused_pane"]["window"], "native-2");
        assert_eq!(status_json["focused_pane"]["weight"], 3);
        assert_eq!(status_json["focused_pane"]["stack_index"], 1);
        assert_eq!(status_json["focused_pane"]["stack_top"], true);
        assert_eq!(status_json["focused_pane"]["floating_dx"], 3);
        assert_eq!(status_json["focused_pane"]["floating_dy"], -2);
        assert_eq!(status_json["focused_pane"]["floating_moved"], true);
        assert_eq!(status_json["focused_pane"]["title_draggable"], true);
        assert_eq!(status_json["focused_pane"]["title_drag_kind"], "reposition");
        assert_eq!(status_json["focused_pane"]["title_drag_col"], 44);
        assert_eq!(status_json["focused_pane"]["title_drag_row"], 1);
        assert_eq!(status_json["focused_pane"]["title_drag_active"], true);
        assert_eq!(status_json["focused_pane"]["pid"], 202);
        assert_eq!(status_json["focused_pane"]["command"], "htop");
        assert_eq!(status_json["focused_pane"]["app_cols"], 80);
        assert_eq!(status_json["focused_pane"]["cursor_col"], 12);
        assert_eq!(status_json["focused_pane"]["cursor_row"], 2);
        assert_eq!(status_json["focused_pane"]["cursor_visible"], false);
        assert_eq!(status_json["focused_pane"]["bracketed_paste"], false);
        assert_eq!(
            status_json["focused_pane"]["application_cursor_keys"],
            false
        );
        assert_eq!(status_json["panes_detail"].as_array().unwrap().len(), 2);
        let panes_json: serde_json::Value =
            serde_json::from_str(&native_spawn_queue_reply("PANES_JSON", &pending)).unwrap();
        assert_eq!(panes_json["workspace"], "1");
        assert_eq!(panes_json["chrome"]["top_bar_rows"], 1);
        assert_eq!(panes_json["chrome"]["tilable_rows"], 23);
        assert_eq!(panes_json["panes_detail"].as_array().unwrap().len(), 2);
        assert_eq!(panes_json["panes_detail"][1]["window"], "native-2");
        assert_eq!(panes_json["panes_detail"][1]["focused"], true);
        assert_eq!(panes_json["panes_detail"][1]["weight"], 3);
        assert_eq!(panes_json["panes_detail"][1]["stack_index"], 1);
        assert_eq!(panes_json["panes_detail"][1]["stack_top"], true);
        assert_eq!(panes_json["panes_detail"][1]["floating_dx"], 3);
        assert_eq!(panes_json["panes_detail"][1]["floating_dy"], -2);
        assert_eq!(panes_json["panes_detail"][1]["floating_moved"], true);
        assert_eq!(panes_json["panes_detail"][1]["title_draggable"], true);
        assert_eq!(
            panes_json["panes_detail"][1]["title_drag_kind"],
            "reposition"
        );
        assert_eq!(panes_json["panes_detail"][1]["title_drag_col"], 44);
        assert_eq!(panes_json["panes_detail"][1]["title_drag_row"], 1);
        assert_eq!(panes_json["panes_detail"][1]["title_drag_active"], true);
        assert_eq!(panes_json["panes_detail"][0]["floating_moved"], false);
        assert_eq!(panes_json["panes_detail"][0]["title_draggable"], false);
        assert!(panes_json["panes_detail"][0]
            .get("title_drag_col")
            .is_none());
        assert_eq!(panes_json["panes_detail"][1]["x"], 40);
        assert_eq!(panes_json["panes_detail"][1]["app_cols"], 80);
        assert_eq!(panes_json["panes_detail"][1]["cursor_col"], 12);
        assert_eq!(panes_json["panes_detail"][1]["cursor_row"], 2);
        assert_eq!(panes_json["panes_detail"][0]["cursor_visible"], true);
        assert_eq!(panes_json["panes_detail"][0]["bracketed_paste"], true);
        assert_eq!(
            panes_json["panes_detail"][0]["application_cursor_keys"],
            true
        );
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
        assert_eq!(session_json["panes"][0]["floating_dx"], 0);
        assert_eq!(session_json["panes"][0]["floating_dy"], 0);
        assert_eq!(session_json["panes"][1]["weight"], 3);
        assert_eq!(session_json["panes"][1]["floating_dx"], 3);
        assert_eq!(session_json["panes"][1]["floating_dy"], -2);
        assert!(session_json["panes"][1].get("pid").is_none());
        assert!(session_json["panes"][1].get("x").is_none());
        assert!(session_json["panes"][1].get("text_snapshot").is_none());

        let restore_reply = native_spawn_queue_reply(
            &format!(
                "RESTORE_SESSION_JSON {}",
                serde_json::to_string(&session_json).unwrap()
            ),
            &pending,
        );
        assert!(
            restore_reply.starts_with("RESTORE_SESSION_QUEUED"),
            "{restore_reply}"
        );
        let restore = drain_native_spawn_pending(&pending)
            .into_iter()
            .find_map(|cmd| match cmd {
                NativePaneCommand::RestoreSession(restore) => Some(restore),
                _ => None,
            })
            .expect("restore command queued");
        assert_eq!(restore.layout.as_deref(), Some("rows"));
        assert_eq!(restore.focus_index, Some(1));
        assert_eq!(restore.panes.len(), 2);
        assert_eq!(restore.panes[0].command, "/bin/sh");
        assert_eq!(restore.panes[0].weight, 1);
        assert_eq!(restore.panes[1].command, "htop");
        assert_eq!(restore.panes[1].weight, 3);
        assert_eq!(restore.panes[1].floating_dx, Some(3));
        assert_eq!(restore.panes[1].floating_dy, Some(-2));
        assert!(restore.panes[1].focused);

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
        let wait_text_json: serde_json::Value =
            serde_json::from_str(&native_spawn_wait_text_json_reply(
                &pending,
                "focused second",
                Duration::from_millis(1),
            ))
            .unwrap();
        assert_eq!(wait_text_json["kind"], "text");
        assert_eq!(wait_text_json["match"], "MATCH_TEXT");
        assert_eq!(wait_text_json["window"], "native-2");
        assert_eq!(wait_text_json["bytes"], 17);
        let wait_text_json_ms: serde_json::Value = serde_json::from_str(&native_spawn_queue_reply(
            "WAIT_TEXT_JSON_MS focused 10 second",
            &pending,
        ))
        .unwrap();
        assert_eq!(wait_text_json_ms["kind"], "text");
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
        let wait_output_json: serde_json::Value =
            serde_json::from_str(&native_spawn_wait_output_json_reply(
                &pending,
                "focused history",
                Duration::from_millis(1),
            ))
            .unwrap();
        assert_eq!(wait_output_json["kind"], "output");
        assert_eq!(wait_output_json["match"], "MATCH_OUTPUT");
        assert_eq!(wait_output_json["window"], "native-2");
        assert_eq!(wait_output_json["bytes"], 30);
        let wait_output_json_ms: serde_json::Value = serde_json::from_str(
            &native_spawn_queue_reply("WAIT_OUTPUT_JSON_MS focused 10 history", &pending),
        )
        .unwrap();
        assert_eq!(wait_output_json_ms["kind"], "output");
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
    fn daemon_bind_removes_stale_socket_file() {
        let p = std::env::temp_dir().join(test_socket_filename(
            "kittwm-test-stale",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"not a socket").unwrap();
        let server = DaemonServer::bind(p.clone()).unwrap();
        assert!(p.exists());
        assert_eq!(
            client_request(server.path(), "PING").unwrap().trim(),
            "PONG"
        );
    }

    #[test]
    fn native_spawn_queue_bind_removes_stale_socket_file() {
        let p = std::env::temp_dir().join(test_socket_filename(
            "kittwm-native-stale",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"not a socket").unwrap();
        let queue = NativeSpawnQueue::bind(p.clone()).unwrap();
        assert!(p.exists());
        assert_eq!(client_request(queue.path(), "PING").unwrap().trim(), "PONG");
    }

    #[test]
    fn active_socket_collision_message_is_actionable() {
        let p = std::env::temp_dir().join(test_socket_filename(
            "kittwm-test-collision-help",
            std::process::id(),
        ));
        let message = active_socket_collision_message(&p, "native spawn queue");
        let socket = p.display().to_string();
        assert!(message.contains("already listening"), "{message}");
        assert!(
            message.contains(&format!("kittwm --socket {socket} --status")),
            "{message}"
        );
        assert!(
            message.contains(&format!("kittwm --socket {socket} --panes")),
            "{message}"
        );
        assert!(message.contains("kittwm stop"), "{message}");
        assert!(
            message.contains(&format!("kittwm --socket {socket} stop")),
            "{message}"
        );
        assert!(
            message.contains("KITTWM_SOCKET=/tmp/kittwm-<name>.sock kittwm"),
            "{message}"
        );
        assert!(
            message.contains("stale socket files are removed automatically"),
            "{message}"
        );
        assert_eq!(message.capacity(), message.len());
    }

    #[test]
    fn double_bind_detects_existing_daemon() {
        let p =
            std::env::temp_dir().join(test_socket_filename("kittwm-test-dup", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let _a = DaemonServer::bind(p.clone()).unwrap();
        let err = DaemonServer::bind(p.clone()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("already listening"), "{message}");
        assert!(message.contains("kittwm stop"), "{message}");
        assert!(
            message.contains("stale socket files are removed automatically"),
            "{message}"
        );
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
        if !(first.starts_with("WINDOWS ")
            || first.starts_with("DISPLAYS ")
            || first.starts_with("APPS ")
            || first.starts_with("PANES ")
            || cmd.trim_start() == "EVENTS"
            || cmd.trim_start().starts_with("EVENTS ")
            || first.starts_with("TEXT ")
            || first.starts_with("SCROLLBACK "))
        {
            break;
        }
    }
    Ok(out)
}

fn daemon_status_line(pid: u32, uptime_s: u64, socket: &str, panes: usize, focus: &str) -> String {
    let mut out = String::with_capacity(
        "pid= uptime_s= sock= panes= focus=\n".len()
            + u32_decimal_len(pid)
            + u64_decimal_len(uptime_s)
            + socket.len()
            + usize_decimal_len(panes)
            + focus.len(),
    );
    writeln!(
        out,
        "pid={pid} uptime_s={uptime_s} sock={socket} panes={panes} focus={focus}"
    )
    .expect("write to string");
    out
}

fn daemon_status_reply(started: Instant, path: &Path, panes: &SharedPanes) -> String {
    let snapshot = panes.lock().ok();
    let pane_count = snapshot.as_ref().map(|p| p.panes.len()).unwrap_or(0);
    let focus = snapshot
        .and_then(|p| p.focused)
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_string());
    daemon_status_line(
        std::process::id(),
        started.elapsed().as_secs(),
        &path.display().to_string(),
        pane_count,
        &focus,
    )
}

fn daemon_status_json_reply(started: Instant, path: &Path, panes: &SharedPanes) -> String {
    let Ok(registry) = panes.lock() else {
        return "{\"error\":\"PANES registry poisoned\"}\n".to_string();
    };
    json_value_line(&serde_json::json!({
        "pid": std::process::id(),
        "uptime_s": started.elapsed().as_secs(),
        "sock": path.display().to_string(),
        "panes": registry.panes.len(),
        "focus": registry.focused.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string()),
    }))
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
    json_value_line(&serde_json::json!({
        "panes": registry.panes.len(),
        "focus": registry.focused.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string()),
        "panes_detail": registry.panes,
    }))
}

fn bool_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn spawn_success_reply(pane: &TrackedPane) -> String {
    let focused = bool_str(pane.focused);
    let mut out = String::with_capacity(
        "SPAWNED pane= window= pid= layout= focused= argv=\n".len()
            + u32_decimal_len(pane.pane_id)
            + pane.window.len()
            + u32_decimal_len(pane.pid)
            + pane.layout.len()
            + focused.len()
            + pane.argv.len(),
    );
    write!(
        out,
        "SPAWNED pane={} window={} pid={} layout={} focused={} argv=",
        pane.pane_id, pane.window, pane.pid, pane.layout, focused
    )
    .expect("write to string");
    out.push_str(&pane.argv);
    out.push('\n');
    out
}

fn spawn_registry_poisoned_reply(pid: u32) -> String {
    let mut out = String::with_capacity(
        "ERR SPAWN registry poisoned after pid=\n".len() + u32_decimal_len(pid),
    );
    writeln!(out, "ERR SPAWN registry poisoned after pid={pid}").expect("write to string");
    out
}

fn spawn_error_reply(argv: &str, error: &std::io::Error) -> String {
    let error = error.to_string();
    let mut out = String::with_capacity("ERR SPAWN : \n".len() + argv.len() + error.len());
    out.push_str("ERR SPAWN ");
    out.push_str(argv);
    out.push_str(": ");
    out.push_str(&error);
    out.push('\n');
    out
}

fn spawn_reply(argv: &str, path: &Path, panes: &SharedPanes) -> String {
    if argv.trim().is_empty() {
        return "ERR SPAWN requires argv\n".to_string();
    }
    let next_window = panes
        .lock()
        .map(|p| tracked_pane_window(p.next_id.saturating_add(1).max(1)))
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
                spawn_success_reply(&pane)
            }
            Err(_) => spawn_registry_poisoned_reply(child.id()),
        },
        Err(e) => spawn_error_reply(argv, &e),
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
    let resolved = default_path.as_ref().map(|p| p.display().to_string());
    let path_arr = json_string_array(&path_cmds);
    let mac_arr = json_string_array(&mac_apps);
    let mut out = String::new();
    out.push_str("{\"default_command\": ");
    write!(out, "{default_cmd:?}").expect("write to string");
    out.push_str(", \"default_resolved\": ");
    match &resolved {
        Some(path) => write!(out, "{path:?}").expect("write to string"),
        None => out.push_str("null"),
    }
    out.push_str(", \"path_commands\": [");
    out.push_str(&path_arr);
    out.push_str("], \"macos_apps\": [");
    out.push_str(&mac_arr);
    out.push_str("]}\n");
    out
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
    let capacity = items
        .iter()
        .map(|item| item.len().saturating_add(4))
        .sum::<usize>()
        .saturating_sub((items.is_empty() as usize).saturating_mul(2));
    let mut out = String::with_capacity(capacity);
    for item in items {
        if !out.is_empty() {
            out.push_str(", ");
        }
        let _ = write!(out, "{item:?}");
    }
    out
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
            Ok(pid) => apps_launch_first_reply(pid, candidate.kind, &candidate.name),
            Err(e) => apps_launch_error_reply(candidate.kind, &candidate.name, &e.to_string()),
        }
    } else {
        apps_first_match_reply(candidate.kind, &candidate.name)
    }
}

fn apps_first_match_reply(kind: &str, name: &str) -> String {
    let mut out = String::with_capacity("APPS_FIRST kind= name=\n".len() + kind.len() + name.len());
    out.push_str("APPS_FIRST kind=");
    out.push_str(kind);
    out.push_str(" name=");
    out.push_str(name);
    out.push('\n');
    out
}

fn apps_launch_first_reply(pid: u32, kind: &str, name: &str) -> String {
    let mut out = String::with_capacity(
        "APPS_LAUNCH_FIRST pid= kind= name=\n".len()
            + u32_decimal_len(pid)
            + kind.len()
            + name.len(),
    );
    out.push_str("APPS_LAUNCH_FIRST pid=");
    write!(out, "{pid}").expect("write to string");
    out.push_str(" kind=");
    out.push_str(kind);
    out.push_str(" name=");
    out.push_str(name);
    out.push('\n');
    out
}

fn apps_launch_error_reply(kind: &str, name: &str, err: &str) -> String {
    let mut out =
        String::with_capacity("ERR launch :: \n".len() + kind.len() + name.len() + err.len());
    out.push_str("ERR launch ");
    out.push_str(kind);
    out.push(':');
    out.push_str(name);
    out.push_str(": ");
    out.push_str(err);
    out.push('\n');
    out
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
        .filter(|item| ascii_casefold_contains_lower(item, &q))
        .take(limit)
        .collect()
}

fn ascii_casefold_contains_lower(item: &str, lower_query: &str) -> bool {
    let item = item.as_bytes();
    let lower_query = lower_query.as_bytes();
    if lower_query.is_empty() {
        return true;
    }
    item.len() >= lower_query.len()
        && item.windows(lower_query.len()).any(|window| {
            window
                .iter()
                .zip(lower_query.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        })
}
