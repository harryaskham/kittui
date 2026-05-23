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
    /// The SDK client's local capability scope does not allow this action.
    #[error("capability denied: {0:?}")]
    CapabilityDenied(Capability),
}

/// SDK operation capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Send raw protocol commands.
    RawRequest,
    /// Create/spawn new windows or surfaces.
    CreateWindow,
    /// Replace the current window.
    ReplaceWindow,
    /// Focus, resize, rename, or close windows.
    ControlWindow,
    /// Send keyboard/text input.
    SendInput,
    /// Read surface text/snapshots.
    ReadText,
    /// Read/write clipboard through the SDK.
    Clipboard,
    /// Subscribe to global or surface event streams.
    SubscribeEvents,
    /// Read semantic component trees.
    ReadSemanticTree,
    /// Invoke semantic component actions.
    InvokeSemanticAction,
}

/// Local SDK capability scope for a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCapabilities {
    allowed: Vec<Capability>,
}

impl ClientCapabilities {
    /// Allow all currently-known SDK capabilities.
    pub fn all() -> Self {
        Self {
            allowed: vec![
                Capability::RawRequest,
                Capability::CreateWindow,
                Capability::ReplaceWindow,
                Capability::ControlWindow,
                Capability::SendInput,
                Capability::ReadText,
                Capability::Clipboard,
                Capability::SubscribeEvents,
                Capability::ReadSemanticTree,
                Capability::InvokeSemanticAction,
            ],
        }
    }

    /// Allow only low-risk status/inspection helpers. Raw requests are denied.
    pub fn restricted() -> Self {
        Self {
            allowed: vec![Capability::ReadText],
        }
    }

    /// Build an explicit capability scope.
    pub fn only(allowed: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
        }
    }

    /// Whether a capability is allowed.
    pub fn allows(&self, capability: Capability) -> bool {
        self.allowed.contains(&capability)
    }

    fn ensure(&self, capability: Capability) -> Result<()> {
        if self.allows(capability) {
            Ok(())
        } else {
            Err(Error::CapabilityDenied(capability))
        }
    }
}

impl Default for ClientCapabilities {
    fn default() -> Self {
        Self::all()
    }
}

/// A connected kittwm client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kittwm {
    socket: PathBuf,
    capabilities: ClientCapabilities,
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

/// Stable semantic component identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemanticComponentId(pub String);

impl SemanticComponentId {
    /// Create a semantic component id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Role of a semantic component node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentRole {
    /// Generic container/group.
    Group,
    /// Static label.
    Label,
    /// Action button.
    Button,
    /// Checkbox.
    Checkbox,
    /// Single radio option.
    Radio,
    /// Radio group.
    RadioGroup,
    /// Single-line text input.
    TextInput,
    /// Multi-line text area.
    TextArea,
    /// Select/list control.
    SelectList,
    /// Menu or command list.
    Menu,
    /// Slider.
    Slider,
    /// Progress bar.
    Progress,
    /// Tab strip/list.
    Tabs,
    /// Split-pane container.
    SplitPane,
    /// Table.
    Table,
    /// Unknown/custom role with namespaced type.
    Custom(String),
}

/// Typed value carried by a semantic component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ComponentValue {
    /// Boolean value.
    Bool(bool),
    /// Text value.
    Text(String),
    /// Numeric value, usually normalized unless role-specific docs say otherwise.
    Number(f32),
    /// Selected component/option ids.
    Selection(Vec<String>),
}

/// Common semantic component state flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentState {
    /// Component currently has semantic focus.
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,
    /// Component can receive focus.
    #[serde(default, skip_serializing_if = "is_false")]
    pub focusable: bool,
    /// Component is disabled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// Component is active/pressed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub active: bool,
    /// Component is selected.
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    /// Component is checked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    /// Component is expanded.
    #[serde(default, skip_serializing_if = "is_false")]
    pub expanded: bool,
    /// Component value is redacted/sensitive.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sensitive: bool,
}

/// Semantic layout kind hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentLayoutKind {
    /// Renderer may flow/re-wrap children.
    Flow,
    /// Row layout.
    Row,
    /// Column layout.
    Column,
    /// Grid layout.
    Grid,
    /// Stack/overlay layout.
    Stack,
    /// Absolute/fixed rectangle layout.
    Absolute,
}

impl Default for ComponentLayoutKind {
    fn default() -> Self {
        Self::Flow
    }
}

/// Optional semantic layout hints.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentLayout {
    /// Layout kind.
    #[serde(default)]
    pub kind: ComponentLayoutKind,
    /// Optional logical x coordinate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u16>,
    /// Optional logical y coordinate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u16>,
    /// Optional width in cells/logical units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    /// Optional height in cells/logical units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

/// Kind of semantic action a component supports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Activate/click.
    Activate,
    /// Toggle checked/pressed state.
    Toggle,
    /// Set full value.
    SetValue,
    /// Insert text.
    InsertText,
    /// Select an option.
    Select,
    /// Move focus.
    Focus,
    /// Expand.
    Expand,
    /// Collapse.
    Collapse,
    /// Open a menu/popup.
    OpenMenu,
    /// Close/dismiss.
    Close,
    /// Scroll.
    Scroll,
    /// Custom namespaced action.
    Custom(String),
}

/// Semantic action descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentAction {
    /// Stable action id within the component.
    pub id: String,
    /// Action kind.
    pub kind: ActionKind,
    /// Optional human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether this action can currently be invoked.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl ComponentAction {
    /// Build an enabled action with id and kind.
    pub fn new(id: impl Into<String>, kind: ActionKind) -> Self {
        Self {
            id: id.into(),
            kind,
            label: None,
            enabled: true,
        }
    }

    /// Attach a label.
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set enabled state.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Semantic component tree node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentNode {
    /// Stable component id.
    pub id: SemanticComponentId,
    /// Component role.
    pub role: ComponentRole,
    /// Optional accessible label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional accessible description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional typed value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<ComponentValue>,
    /// State flags.
    #[serde(default)]
    pub state: ComponentState,
    /// Layout hints.
    #[serde(default)]
    pub layout: ComponentLayout,
    /// Supported actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ComponentAction>,
    /// Child nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ComponentNode>,
}

impl ComponentNode {
    /// Build a component node.
    pub fn new(id: impl Into<String>, role: ComponentRole) -> Self {
        Self {
            id: SemanticComponentId::new(id),
            role,
            label: None,
            description: None,
            value: None,
            state: ComponentState::default(),
            layout: ComponentLayout::default(),
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Set label.
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set value.
    pub fn valued(mut self, value: ComponentValue) -> Self {
        self.value = Some(value);
        self
    }

    /// Set state.
    pub fn state(mut self, state: ComponentState) -> Self {
        self.state = state;
        self
    }

    /// Set actions.
    pub fn actions(mut self, actions: Vec<ComponentAction>) -> Self {
        self.actions = actions;
        self
    }

    /// Set children.
    pub fn children(mut self, children: Vec<ComponentNode>) -> Self {
        self.children = children;
        self
    }
}

/// Snapshot of one semantic surface revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticSurfaceSnapshot {
    /// Schema version.
    pub schema_version: u32,
    /// Surface/window id.
    pub surface: String,
    /// Monotonic revision.
    pub revision: u64,
    /// Root component node.
    pub root: ComponentNode,
    /// Focused component id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<SemanticComponentId>,
}

impl SemanticSurfaceSnapshot {
    /// Build a schema v1 snapshot.
    pub fn new(surface: impl Into<String>, revision: u64, root: ComponentNode) -> Self {
        Self {
            schema_version: 1,
            surface: surface.into(),
            revision,
            root,
            focus: None,
        }
    }

    /// Set focused component id.
    pub fn focused(mut self, id: impl Into<String>) -> Self {
        self.focus = Some(SemanticComponentId::new(id));
        self
    }
}

/// Semantic event emitted by a surface/runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SemanticSurfaceEvent {
    /// A new snapshot is available.
    SnapshotReady {
        /// Surface id.
        surface: String,
        /// Snapshot revision.
        revision: u64,
    },
    /// Focus changed.
    FocusChanged {
        /// Surface id.
        surface: String,
        /// Focused component id.
        component: Option<SemanticComponentId>,
    },
    /// Component value changed.
    ValueChanged {
        /// Surface id.
        surface: String,
        /// Component id.
        component: SemanticComponentId,
        /// New value.
        value: ComponentValue,
    },
    /// Component action was invoked.
    ActionInvoked {
        /// Surface id.
        surface: String,
        /// Component id.
        component: SemanticComponentId,
        /// Action id.
        action: String,
    },
    /// Accessibility/live-region announcement.
    Announcement {
        /// Surface id.
        surface: String,
        /// Announcement message.
        message: String,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_true() -> bool {
    true
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
        Ok(Self {
            socket,
            capabilities: ClientCapabilities::default(),
        })
    }

    /// Connect to an explicit kittwm socket path.
    pub fn connect_path(path: impl Into<PathBuf>) -> Self {
        Self {
            socket: path.into(),
            capabilities: ClientCapabilities::default(),
        }
    }

    /// Restrict this client to a local SDK capability scope.
    pub fn with_capabilities(mut self, capabilities: ClientCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Return the client's local SDK capability scope.
    pub fn capabilities(&self) -> &ClientCapabilities {
        &self.capabilities
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
        self.capabilities.ensure(Capability::RawRequest)?;
        self.request_protocol(command)
    }

    fn request_protocol(&self, command: impl AsRef<str>) -> Result<String> {
        let reply = request_socket(&self.socket, command.as_ref())?;
        if let Some(err) = reply.strip_prefix("ERR ") {
            return Err(Error::Daemon(err.trim().to_string()));
        }
        Ok(reply)
    }

    /// Ping the daemon/control plane.
    pub fn ping(&self) -> Result<()> {
        let reply = self.request_protocol("PING")?;
        if reply.trim() == "PONG" {
            Ok(())
        } else {
            Err(Error::Daemon(reply.trim().to_string()))
        }
    }

    /// Fetch typed status JSON.
    pub fn status(&self) -> Result<Status> {
        Ok(serde_json::from_str(
            &self.request_protocol("STATUS_JSON")?,
        )?)
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
        self.capabilities.ensure(Capability::CreateWindow)?;
        let reply = match &spec.kind {
            SurfaceKind::Terminal => {
                self.request_protocol(format!("SPAWN_PTY {}", spec.command))?
            }
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
        self.capabilities.ensure(Capability::ReplaceWindow)?;
        let reply = self.create_window(spec)?;
        if let Some(handle) = self.current_window_from_env() {
            let _ = self.request_protocol(format!("CLOSE_PANE {}", handle.id));
        }
        Ok(reply)
    }
}

impl SurfaceHandle {
    /// Focus this surface/window.
    pub fn focus(&self) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client
            .request_protocol(format!("FOCUS_PANE {}", self.id))
    }

    /// Close this surface/window.
    pub fn close(&self) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client
            .request_protocol(format!("CLOSE_PANE {}", self.id))
    }

    /// Rename this surface/window.
    pub fn rename(&self, title: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client
            .request_protocol(format!("RENAME_PANE {} {}", self.id, title.as_ref()))
    }

    /// Resize this surface/window by a relative pane-weight delta.
    pub fn resize_weight(&self, delta: i16) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        let label = if delta >= 0 {
            format!("+{delta}")
        } else {
            delta.to_string()
        };
        self.client
            .request_protocol(format!("RESIZE_PANE {} {label}", self.id))
    }

    /// Send raw UTF-8 text.
    pub fn send_text(&self, text: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        self.client
            .request_protocol(format!("SEND_TEXT {} {}", self.id, text.as_ref()))
    }

    /// Send one line, appending a newline in the daemon.
    pub fn send_line(&self, text: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        self.client
            .request_protocol(format!("SEND_LINE {} {}", self.id, text.as_ref()))
    }

    /// Send a named key such as `ctrl-c`, `escape`, or `up`.
    pub fn send_key(&self, key: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        self.client
            .request_protocol(format!("SEND_KEY {} {}", self.id, key.as_ref()))
    }

    /// Read the current screen text snapshot.
    pub fn read_text(&self) -> Result<TextSnapshot> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            format!("READ_TEXT_JSON {}", self.id),
        )?)?)
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
    fn capability_scopes_deny_disallowed_operations_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        assert!(matches!(
            client.request("PING"),
            Err(Error::CapabilityDenied(Capability::RawRequest))
        ));
        assert!(matches!(
            client.spawn_surface(&SurfaceSpec::terminal("true")),
            Err(Error::CapabilityDenied(Capability::CreateWindow))
        ));
        assert!(matches!(
            client.focused_surface().send_key("enter"),
            Err(Error::CapabilityDenied(Capability::SendInput))
        ));
    }

    #[test]
    fn capability_helpers_report_allowed_values() {
        let caps = ClientCapabilities::restricted();
        assert!(caps.allows(Capability::ReadText));
        assert!(!caps.allows(Capability::CreateWindow));
        assert!(ClientCapabilities::all().allows(Capability::SubscribeEvents));
        assert!(ClientCapabilities::all().allows(Capability::ReadSemanticTree));
        assert!(ClientCapabilities::all().allows(Capability::InvokeSemanticAction));
    }

    #[test]
    fn semantic_snapshot_serializes_stable_json_shape() {
        let snapshot = SemanticSurfaceSnapshot::new(
            "native-1",
            42,
            ComponentNode::new("settings", ComponentRole::Group)
                .labeled("Settings")
                .children(vec![
                    ComponentNode::new("notify", ComponentRole::Checkbox)
                        .labeled("Notifications")
                        .valued(ComponentValue::Bool(true))
                        .state(ComponentState {
                            checked: true,
                            focusable: true,
                            ..ComponentState::default()
                        })
                        .actions(vec![ComponentAction::new("toggle", ActionKind::Toggle)]),
                    ComponentNode::new("theme", ComponentRole::RadioGroup)
                        .labeled("Theme")
                        .valued(ComponentValue::Selection(vec!["dark".to_string()])),
                ]),
        )
        .focused("notify");
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["surface"], "native-1");
        assert_eq!(value["root"]["role"], "group");
        assert_eq!(value["root"]["children"][0]["role"], "checkbox");
        assert_eq!(value["root"]["children"][0]["state"]["checked"], true);
        assert_eq!(value["root"]["children"][0]["actions"][0]["kind"], "toggle");
        assert_eq!(value["root"]["children"][1]["value"]["kind"], "selection");
        assert_eq!(value["focus"], "notify");

        let decoded: SemanticSurfaceSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.revision, 42);
        assert_eq!(decoded.root.children.len(), 2);
    }

    #[test]
    fn semantic_event_decodes_snake_case_tagged_shape() {
        let event: SemanticSurfaceEvent = serde_json::from_str(
            r#"{"kind":"value_changed","surface":"native-1","component":"field","value":{"kind":"text","value":"Ada"}}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            SemanticSurfaceEvent::ValueChanged {
                surface: "native-1".to_string(),
                component: SemanticComponentId::new("field"),
                value: ComponentValue::Text("Ada".to_string())
            }
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
