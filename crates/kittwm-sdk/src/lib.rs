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
use serde_json::Value;

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
    /// Publish semantic component trees.
    PublishSemanticTree,
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
                Capability::PublishSemanticTree,
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
    /// Browser-backed surface. Today this uses the first-party `kittwm-browser`
    /// app over the PTY spawn transport; a dedicated browser surface protocol is
    /// future work.
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

    /// Build a browser surface spec using the first-party `kittwm-browser` app.
    pub fn browser(target: impl Into<String>) -> Self {
        Self {
            kind: SurfaceKind::Browser,
            command: target.into(),
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

/// Common event metadata from the native `EVENTS [ms]` stream.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Event schema version.
    #[serde(default)]
    pub schema_version: Option<u64>,
    /// Monotonic event sequence.
    #[serde(default)]
    pub seq: Option<u64>,
    /// Event timestamp in milliseconds since epoch.
    #[serde(default)]
    pub at_ms: Option<u128>,
    /// Affected/focused window, if supplied.
    #[serde(default)]
    pub window: Option<String>,
    /// Event-specific detail object.
    #[serde(default)]
    pub detail: Value,
}

/// Native socket event parsed from `EVENTS [ms]`.
#[derive(Clone, Debug, PartialEq)]
pub enum KittwmEvent {
    /// Initial status snapshot.
    Status(EventEnvelope),
    /// Status changed.
    StatusChanged(EventEnvelope),
    /// Pane opened.
    PaneOpened(EventEnvelope),
    /// Pane closed.
    PaneClosed(EventEnvelope),
    /// Pane metadata/text/status changed.
    PaneChanged(EventEnvelope),
    /// Focus changed.
    FocusChanged(EventEnvelope),
    /// Layout changed.
    LayoutChanged(EventEnvelope),
    /// A semantic snapshot is ready/was published.
    SemanticSnapshotReady(EventEnvelope),
    /// Semantic component focus changed.
    SemanticFocusChanged(EventEnvelope),
    /// Semantic action was invoked.
    SemanticActionInvoked(EventEnvelope),
    /// Semantic component value changed.
    SemanticValueChanged(EventEnvelope),
    /// Unknown event kind; raw JSON is preserved for forward compatibility.
    Unknown {
        /// Unknown kind string.
        kind: String,
        /// Raw event object.
        raw: Value,
    },
}

impl KittwmEvent {
    /// Parse one JSON event line from the native event stream.
    pub fn parse_line(line: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(line)?;
        Ok(parse_event_value(value))
    }

    /// Return this event's kind label.
    pub fn kind(&self) -> &str {
        match self {
            Self::Status(_) => "status",
            Self::StatusChanged(_) => "status_changed",
            Self::PaneOpened(_) => "pane_opened",
            Self::PaneClosed(_) => "pane_closed",
            Self::PaneChanged(_) => "pane_changed",
            Self::FocusChanged(_) => "focus_changed",
            Self::LayoutChanged(_) => "layout_changed",
            Self::SemanticSnapshotReady(_) => "semantic_snapshot_ready",
            Self::SemanticFocusChanged(_) => "semantic_focus_changed",
            Self::SemanticActionInvoked(_) => "semantic_action_invoked",
            Self::SemanticValueChanged(_) => "semantic_value_changed",
            Self::Unknown { kind, .. } => kind.as_str(),
        }
    }
}

fn parse_event_value(value: Value) -> KittwmEvent {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let envelope = || EventEnvelope {
        schema_version: value.get("schema_version").and_then(Value::as_u64),
        seq: value.get("seq").and_then(Value::as_u64),
        at_ms: value.get("at_ms").and_then(Value::as_u64).map(u128::from),
        window: value
            .get("window")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        detail: value.get("detail").cloned().unwrap_or(Value::Null),
    };
    match kind.as_str() {
        "status" => KittwmEvent::Status(envelope()),
        "status_changed" => KittwmEvent::StatusChanged(envelope()),
        "pane_opened" => KittwmEvent::PaneOpened(envelope()),
        "pane_closed" => KittwmEvent::PaneClosed(envelope()),
        "pane_changed" => KittwmEvent::PaneChanged(envelope()),
        "focus_changed" => KittwmEvent::FocusChanged(envelope()),
        "layout_changed" => KittwmEvent::LayoutChanged(envelope()),
        "semantic_snapshot_ready" => KittwmEvent::SemanticSnapshotReady(envelope()),
        "semantic_focus_changed" => KittwmEvent::SemanticFocusChanged(envelope()),
        "semantic_action_invoked" => KittwmEvent::SemanticActionInvoked(envelope()),
        "semantic_value_changed" => KittwmEvent::SemanticValueChanged(envelope()),
        _ => KittwmEvent::Unknown { kind, raw: value },
    }
}

/// Dirty-frame metrics reported by native panes when measurement is enabled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirtyFrameStatus {
    /// Changed dirty-grid tiles.
    pub changed_tiles: u32,
    /// Total dirty-grid tiles.
    pub total_tiles: u32,
    /// Changed tile fraction in `[0, 1]`.
    pub changed_fraction: f32,
    /// Whether the frame upload was skipped because it was clean.
    pub skipped_upload: bool,
}

/// Rich native pane detail returned by `PANES_JSON` / `STATUS_JSON`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativePaneDetail {
    /// Window id.
    pub window: String,
    /// Human-readable title.
    pub title: String,
    /// Whether this pane is focused.
    pub focused: bool,
    /// Layout weight.
    pub weight: u16,
    /// Process id, if known.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Spawned command, if known.
    #[serde(default)]
    pub command: Option<String>,
    /// Outer x cell.
    #[serde(default)]
    pub x: Option<u16>,
    /// Outer y cell.
    #[serde(default)]
    pub y: Option<u16>,
    /// Outer columns.
    #[serde(default)]
    pub cols: Option<u16>,
    /// Outer rows.
    #[serde(default)]
    pub rows: Option<u16>,
    /// App/content x cell.
    #[serde(default)]
    pub app_x: Option<u16>,
    /// App/content y cell.
    #[serde(default)]
    pub app_y: Option<u16>,
    /// App/content columns.
    #[serde(default)]
    pub app_cols: Option<u16>,
    /// App/content rows.
    #[serde(default)]
    pub app_rows: Option<u16>,
    /// Cursor column.
    #[serde(default)]
    pub cursor_col: Option<u16>,
    /// Cursor row.
    #[serde(default)]
    pub cursor_row: Option<u16>,
    /// Cursor visibility.
    #[serde(default)]
    pub cursor_visible: Option<bool>,
    /// Bracketed paste mode.
    #[serde(default)]
    pub bracketed_paste: Option<bool>,
    /// Application cursor keys mode.
    #[serde(default)]
    pub application_cursor_keys: Option<bool>,
    /// Basic mouse reporting mode.
    #[serde(default)]
    pub mouse_reporting: Option<bool>,
    /// Button-motion mouse mode.
    #[serde(default)]
    pub mouse_button_motion: Option<bool>,
    /// All-motion mouse mode.
    #[serde(default)]
    pub mouse_all_motion: Option<bool>,
    /// SGR mouse mode.
    #[serde(default)]
    pub mouse_sgr: Option<bool>,
    /// Dirty-frame metrics, when reported.
    #[serde(default)]
    pub dirty_frame: Option<DirtyFrameStatus>,
    /// Transport diagnostics/future extension fields, when reported.
    #[serde(default)]
    pub transport: Option<Value>,
}

/// Typed `PANES_JSON` response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanesStatus {
    /// Pane count.
    pub panes: u64,
    /// Focused window id.
    pub focus: String,
    /// Layout label.
    pub layout: String,
    /// Detailed panes.
    #[serde(default)]
    pub panes_detail: Vec<NativePaneDetail>,
}

/// Minimal status response shape shared by standalone and native daemons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Focused pane detail when available.
    #[serde(default)]
    pub focused_pane: Option<NativePaneDetail>,
    /// Pane details when available.
    #[serde(default)]
    pub panes_detail: Vec<NativePaneDetail>,
}

/// Native app discovery catalog returned by `APPS_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppsCatalog {
    /// Default launcher command configured for the runtime.
    pub default_command: String,
    /// Resolved executable path for the default command, when found on PATH.
    #[serde(default)]
    pub default_resolved: Option<String>,
    /// Executable command names discovered on PATH.
    #[serde(default)]
    pub path_commands: Vec<String>,
    /// macOS `.app` bundle names discovered under Applications directories.
    #[serde(default)]
    pub macos_apps: Vec<String>,
}

/// Candidate selected by native app discovery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCandidate {
    /// Candidate source kind, such as `path` or `macos_app`.
    pub kind: String,
    /// Candidate display/command name.
    pub name: String,
}

/// Result of launching an app-discovery candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLaunch {
    /// Process id reported by the native launcher.
    pub pid: u32,
    /// Candidate that was launched.
    pub candidate: AppCandidate,
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

    /// Fetch typed native pane details from `PANES_JSON`.
    pub fn panes(&self) -> Result<PanesStatus> {
        Ok(serde_json::from_str(&self.request_protocol("PANES_JSON")?)?)
    }

    /// Fetch the native app discovery catalog from `APPS_JSON`.
    pub fn apps(&self) -> Result<AppsCatalog> {
        self.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(&self.request_protocol("APPS_JSON")?)?)
    }

    /// Return the first app-discovery candidate matching a query.
    pub fn app_first(&self, query: impl AsRef<str>) -> Result<AppCandidate> {
        self.capabilities.ensure(Capability::ReadText)?;
        parse_app_first_reply(&self.request_protocol(format!("APPS_FIRST {}", query.as_ref()))?)
    }

    /// Launch the first app-discovery candidate matching a query.
    pub fn app_launch_first(&self, query: impl AsRef<str>) -> Result<AppLaunch> {
        self.capabilities.ensure(Capability::CreateWindow)?;
        parse_app_launch_reply(
            &self.request_protocol(format!("APPS_LAUNCH_FIRST {}", query.as_ref()))?,
        )
    }

    /// Fetch a bounded batch of native JSON-lines events.
    pub fn events_ms(&self, ms: u64) -> Result<Vec<KittwmEvent>> {
        self.capabilities.ensure(Capability::SubscribeEvents)?;
        let ms = ms.clamp(1, 60_000);
        let reply = self.request_protocol(format!("EVENTS {ms}"))?;
        reply
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && trimmed != "END"
            })
            .map(KittwmEvent::parse_line)
            .collect()
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
    /// surfaces via `SPAWN_PTY`; browser surfaces currently dogfood that same
    /// transport by launching the first-party `kittwm-browser` app.
    pub fn spawn_surface(&self, spec: &SurfaceSpec) -> Result<SurfaceSpawn> {
        self.capabilities.ensure(Capability::CreateWindow)?;
        let command = match &spec.kind {
            SurfaceKind::Terminal => spec.command.clone(),
            SurfaceKind::Browser => browser_surface_command(&spec.command),
            SurfaceKind::Other(kind) => {
                return Err(Error::Daemon(format!(
                    "surface kind {kind:?} is not supported by the SDK transport"
                )))
            }
        };
        let reply = self.request_protocol(format!("SPAWN_PTY {command}"))?;
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

    /// Read the semantic component snapshot for this surface.
    pub fn semantic_snapshot(&self) -> Result<SemanticSurfaceSnapshot> {
        self.client
            .capabilities
            .ensure(Capability::ReadSemanticTree)?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            format!("SEMANTIC_SNAPSHOT {}", self.id),
        )?)?)
    }

    /// Publish the current semantic component snapshot for this surface.
    pub fn semantic_publish(&self, snapshot: &SemanticSurfaceSnapshot) -> Result<String> {
        self.client
            .capabilities
            .ensure(Capability::PublishSemanticTree)?;
        let payload = serde_json::to_string(snapshot)?;
        self.client
            .request_protocol(format!("SEMANTIC_PUBLISH {} {}", self.id, payload))
    }

    /// Invoke a semantic component action with a JSON payload.
    pub fn semantic_action(
        &self,
        component: impl AsRef<str>,
        action: impl AsRef<str>,
        payload: impl Serialize,
    ) -> Result<String> {
        self.client
            .capabilities
            .ensure(Capability::InvokeSemanticAction)?;
        let payload = serde_json::to_string(&payload)?;
        self.client.request_protocol(format!(
            "SEMANTIC_ACTION {} {} {} {}",
            self.id,
            component.as_ref(),
            action.as_ref(),
            payload
        ))
    }

    /// Request semantic focus for a component.
    pub fn semantic_focus(&self, component: impl AsRef<str>) -> Result<String> {
        self.client
            .capabilities
            .ensure(Capability::InvokeSemanticAction)?;
        self.client
            .request_protocol(format!("SEMANTIC_FOCUS {} {}", self.id, component.as_ref()))
    }

    /// Convenience alias for [`SurfaceHandle::semantic_focus`].
    pub fn semantic_focus_component(&self, component: impl AsRef<str>) -> Result<String> {
        self.semantic_focus(component)
    }

    /// Toggle a semantic boolean/checked component.
    pub fn semantic_toggle(&self, component: impl AsRef<str>) -> Result<String> {
        self.semantic_action(component, "toggle", serde_json::json!({}))
    }

    /// Set a semantic text component value.
    pub fn semantic_set_text(
        &self,
        component: impl AsRef<str>,
        text: impl AsRef<str>,
    ) -> Result<String> {
        self.semantic_action(
            component,
            "set",
            serde_json::json!({ "text": text.as_ref() }),
        )
    }

    /// Insert text into a semantic text component.
    pub fn semantic_insert_text(
        &self,
        component: impl AsRef<str>,
        text: impl AsRef<str>,
    ) -> Result<String> {
        self.semantic_action(
            component,
            "insert_text",
            serde_json::json!({ "text": text.as_ref() }),
        )
    }

    /// Set a semantic numeric component value.
    pub fn semantic_set_number(&self, component: impl AsRef<str>, value: f32) -> Result<String> {
        self.semantic_action(component, "set", serde_json::json!({ "value": value }))
    }

    /// Set a semantic boolean component value.
    pub fn semantic_set_bool(&self, component: impl AsRef<str>, value: bool) -> Result<String> {
        self.semantic_action(component, "set", serde_json::json!({ "value": value }))
    }

    /// Select one option id on a semantic select/list/radio-group component.
    pub fn semantic_select_one(
        &self,
        component: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> Result<String> {
        self.semantic_action(
            component,
            "select",
            serde_json::json!({ "id": id.as_ref() }),
        )
    }

    /// Select many option ids on a semantic multi-select component.
    pub fn semantic_select_many<I, S>(&self, component: impl AsRef<str>, ids: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let selection = ids
            .into_iter()
            .map(|id| id.as_ref().to_string())
            .collect::<Vec<_>>();
        self.semantic_action(
            component,
            "select",
            serde_json::json!({ "selection": selection }),
        )
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

fn parse_app_first_reply(reply: &str) -> Result<AppCandidate> {
    let line = reply.trim();
    let fields = line
        .strip_prefix("APPS_FIRST ")
        .ok_or_else(|| Error::Daemon(line.to_string()))?;
    parse_app_candidate_fields(fields)
}

fn parse_app_launch_reply(reply: &str) -> Result<AppLaunch> {
    let line = reply.trim();
    let fields = line
        .strip_prefix("APPS_LAUNCH_FIRST ")
        .ok_or_else(|| Error::Daemon(line.to_string()))?;
    let mut pid = None;
    let mut rest = Vec::new();
    for field in fields.split_whitespace() {
        if let Some(value) = field.strip_prefix("pid=") {
            pid =
                Some(value.parse::<u32>().map_err(|_| {
                    Error::Daemon(format!("invalid APPS_LAUNCH_FIRST pid: {value}"))
                })?);
        } else {
            rest.push(field);
        }
    }
    Ok(AppLaunch {
        pid: pid.ok_or_else(|| Error::Daemon(format!("missing APPS_LAUNCH_FIRST pid: {line}")))?,
        candidate: parse_app_candidate_fields(&rest.join(" "))?,
    })
}

fn parse_app_candidate_fields(fields: &str) -> Result<AppCandidate> {
    let kind = fields
        .split_whitespace()
        .find_map(|field| field.strip_prefix("kind="))
        .ok_or_else(|| Error::Daemon(format!("missing app candidate kind: {fields}")))?;
    let name = fields
        .split_once("name=")
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Daemon(format!("missing app candidate name: {fields}")))?;
    Ok(AppCandidate {
        kind: kind.to_string(),
        name: name.to_string(),
    })
}

fn browser_surface_command(target: &str) -> String {
    format!("kittwm-browser {}", shell_quote(target))
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
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

    #[cfg(unix)]
    use std::io::{BufRead, BufReader};
    #[cfg(unix)]
    use std::os::unix::net::UnixListener;
    #[cfg(unix)]
    use std::thread;

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
    fn surface_spec_builds_terminal_and_browser_specs() {
        assert_eq!(
            SurfaceSpec::terminal("htop").titled("monitor"),
            SurfaceSpec {
                kind: SurfaceKind::Terminal,
                command: "htop".to_string(),
                title: Some("monitor".to_string())
            }
        );
        assert_eq!(
            SurfaceSpec::browser("https://example.com").titled("web"),
            SurfaceSpec {
                kind: SurfaceKind::Browser,
                command: "https://example.com".to_string(),
                title: Some("web".to_string())
            }
        );
    }

    #[test]
    fn browser_surface_command_quotes_targets() {
        assert_eq!(
            browser_surface_command("https://example.com/a%20b"),
            "kittwm-browser 'https://example.com/a%20b'"
        );
        assert_eq!(
            browser_surface_command("https://example.com/it's"),
            "kittwm-browser 'https://example.com/it'\\''s'"
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
    fn native_pane_detail_decodes_rich_status_shape() {
        let panes: PanesStatus = serde_json::from_str(
            r#"{
              "panes": 1,
              "focus": "native-1",
              "layout": "columns",
              "panes_detail": [{
                "window": "native-1",
                "title": "shell",
                "focused": true,
                "weight": 2,
                "pid": 123,
                "command": "/bin/sh",
                "x": 0,
                "y": 0,
                "cols": 80,
                "rows": 24,
                "app_x": 0,
                "app_y": 1,
                "app_cols": 80,
                "app_rows": 23,
                "cursor_col": 4,
                "cursor_row": 5,
                "cursor_visible": true,
                "bracketed_paste": true,
                "application_cursor_keys": false,
                "mouse_reporting": true,
                "mouse_button_motion": false,
                "mouse_all_motion": false,
                "mouse_sgr": true,
                "dirty_frame": {
                  "changed_tiles": 1,
                  "total_tiles": 4,
                  "changed_fraction": 0.25,
                  "skipped_upload": false
                },
                "transport": { "selected": "file", "compression": "auto" }
              }]
            }"#,
        )
        .unwrap();
        assert_eq!(panes.focus, "native-1");
        let pane = &panes.panes_detail[0];
        assert_eq!(pane.cursor_col, Some(4));
        assert_eq!(pane.mouse_sgr, Some(true));
        assert_eq!(pane.dirty_frame.as_ref().unwrap().changed_fraction, 0.25);
        assert_eq!(pane.transport.as_ref().unwrap()["selected"], "file");
    }

    #[test]
    fn status_decodes_without_optional_pane_details() {
        let status: Status =
            serde_json::from_str(r#"{"pending":0,"panes":1,"focus":"native-1","layout":"rows"}"#)
                .unwrap();
        assert_eq!(status.focus.as_deref(), Some("native-1"));
        assert!(status.focused_pane.is_none());
        assert!(status.panes_detail.is_empty());
    }

    #[test]
    fn app_catalog_and_candidate_shapes_decode() {
        let catalog: AppsCatalog = serde_json::from_str(
            r#"{"default_command":"xterm","default_resolved":"/usr/bin/xterm","path_commands":["bash","vim"],"macos_apps":["Safari.app"]}"#,
        )
        .unwrap();
        assert_eq!(catalog.default_command, "xterm");
        assert_eq!(catalog.default_resolved.as_deref(), Some("/usr/bin/xterm"));
        assert_eq!(catalog.path_commands, ["bash", "vim"]);
        assert_eq!(catalog.macos_apps, ["Safari.app"]);

        assert_eq!(
            parse_app_first_reply("APPS_FIRST kind=path name=Visual Studio Code\n").unwrap(),
            AppCandidate {
                kind: "path".to_string(),
                name: "Visual Studio Code".to_string(),
            }
        );
        assert_eq!(
            parse_app_launch_reply("APPS_LAUNCH_FIRST pid=1234 kind=macos_app name=Safari\n")
                .unwrap(),
            AppLaunch {
                pid: 1234,
                candidate: AppCandidate {
                    kind: "macos_app".to_string(),
                    name: "Safari".to_string(),
                }
            }
        );
    }

    #[test]
    fn event_parser_handles_known_and_unknown_events() {
        let status = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":7,"at_ms":10,"kind":"focus_changed","window":"native-2","detail":{"focus":"native-2"}}"#,
        )
        .unwrap();
        assert_eq!(status.kind(), "focus_changed");
        match status {
            KittwmEvent::FocusChanged(envelope) => {
                assert_eq!(envelope.seq, Some(7));
                assert_eq!(envelope.window.as_deref(), Some("native-2"));
                assert_eq!(envelope.detail["focus"], "native-2");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let semantic = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":8,"kind":"semantic_value_changed","window":"native-1","detail":{"component":"settings.name","revision":3,"value":"Grace"}}"#,
        )
        .unwrap();
        assert_eq!(semantic.kind(), "semantic_value_changed");
        match semantic {
            KittwmEvent::SemanticValueChanged(envelope) => {
                assert_eq!(envelope.window.as_deref(), Some("native-1"));
                assert_eq!(envelope.detail["component"], "settings.name");
                assert_eq!(envelope.detail["revision"], 3);
                assert_eq!(envelope.detail["value"], "Grace");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        for (kind, expected) in [
            ("semantic_snapshot_ready", "semantic_snapshot_ready"),
            ("semantic_focus_changed", "semantic_focus_changed"),
            ("semantic_action_invoked", "semantic_action_invoked"),
        ] {
            let event = KittwmEvent::parse_line(&format!(
                r#"{{"kind":"{kind}","window":"native-1","detail":{{}}}}"#
            ))
            .unwrap();
            assert_eq!(event.kind(), expected);
        }

        let unknown =
            KittwmEvent::parse_line(r#"{"kind":"new_future_event","detail":{"x":1}}"#).unwrap();
        assert_eq!(unknown.kind(), "new_future_event");
        assert!(matches!(unknown, KittwmEvent::Unknown { .. }));
    }

    #[test]
    fn event_capability_denies_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        assert!(matches!(
            client.events_ms(100),
            Err(Error::CapabilityDenied(Capability::SubscribeEvents))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn app_discovery_helpers_send_expected_socket_commands() {
        let path = PathBuf::from(format!(
            "/tmp/kwa-{}-{}.sock",
            std::process::id(),
            now_test_nanos() % 1_000_000
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let command = request.trim().to_string();
                seen.push(command.clone());
                let reply = match command.as_str() {
                    "APPS_JSON" => "{\"default_command\":\"xterm\",\"default_resolved\":null,\"path_commands\":[\"bash\"],\"macos_apps\":[]}",
                    "APPS_FIRST Visual Studio Code" => "APPS_FIRST kind=path name=Visual Studio Code",
                    "APPS_LAUNCH_FIRST Safari" => "APPS_LAUNCH_FIRST pid=42 kind=macos_app name=Safari",
                    other => panic!("unexpected command {other}"),
                };
                stream.write_all(reply.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
            seen
        });
        let client = Kittwm::connect_path(&path);
        assert_eq!(client.apps().unwrap().path_commands, ["bash"]);
        assert_eq!(
            client.app_first("Visual Studio Code").unwrap().name,
            "Visual Studio Code"
        );
        assert_eq!(client.app_launch_first("Safari").unwrap().pid, 42);
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            seen,
            [
                "APPS_JSON",
                "APPS_FIRST Visual Studio Code",
                "APPS_LAUNCH_FIRST Safari"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn spawn_surface_sends_browser_as_first_party_browser_app() {
        let path = PathBuf::from(format!(
            "/tmp/kwb-{}-{}.sock",
            std::process::id(),
            now_test_nanos() % 1_000_000
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let command = request.trim().to_string();
                seen.push(command);
                stream.write_all(b"SPAWNED native-1\n").unwrap();
            }
            seen
        });
        let client = Kittwm::connect_path(&path);
        assert_eq!(
            client
                .spawn_surface(&SurfaceSpec::terminal("htop"))
                .unwrap()
                .reply
                .trim(),
            "SPAWNED native-1"
        );
        assert_eq!(
            client
                .spawn_surface(&SurfaceSpec::browser("https://example.com/a%20b"))
                .unwrap()
                .reply
                .trim(),
            "SPAWNED native-1"
        );
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            seen,
            [
                "SPAWN_PTY htop",
                "SPAWN_PTY kittwm-browser 'https://example.com/a%20b'"
            ]
        );
    }

    #[test]
    fn app_discovery_capabilities_deny_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::SubscribeEvents]));
        assert!(matches!(
            client.apps(),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
        assert!(matches!(
            client.app_first("vim"),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
        assert!(matches!(
            client.app_launch_first("vim"),
            Err(Error::CapabilityDenied(Capability::CreateWindow))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn events_ms_parses_json_lines_until_end() {
        let path = PathBuf::from(format!(
            "/tmp/kwe-{}-{}.sock",
            std::process::id(),
            now_test_nanos() % 1_000_000
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            stream
                .write_all(
                    b"{\"kind\":\"status\",\"seq\":1,\"detail\":{\"panes\":1}}\n{\"kind\":\"layout_changed\",\"seq\":2,\"detail\":{\"layout\":\"rows\"}}\nEND\n",
                )
                .unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let events = client.events_ms(250).unwrap();
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, "EVENTS 250");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind(), "status");
        assert_eq!(events[1].kind(), "layout_changed");
    }

    #[test]
    fn semantic_capabilities_deny_wrappers_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        let surface = client.focused_surface();
        assert!(matches!(
            surface.semantic_snapshot(),
            Err(Error::CapabilityDenied(Capability::ReadSemanticTree))
        ));
        let snapshot = SemanticSurfaceSnapshot::new(
            "focused",
            1,
            ComponentNode::new("focused.root", ComponentRole::Group),
        );
        assert!(matches!(
            surface.semantic_publish(&snapshot),
            Err(Error::CapabilityDenied(Capability::PublishSemanticTree))
        ));
        assert!(matches!(
            surface.semantic_action("field", "set", serde_json::json!({"value":"x"})),
            Err(Error::CapabilityDenied(Capability::InvokeSemanticAction))
        ));
        assert!(matches!(
            surface.semantic_focus("field"),
            Err(Error::CapabilityDenied(Capability::InvokeSemanticAction))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn semantic_wrappers_send_expected_socket_commands() {
        let path = PathBuf::from(format!(
            "/tmp/kws-{}-{}.sock",
            std::process::id(),
            now_test_nanos() % 1_000_000
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let command = request.trim().to_string();
                seen.push(command.clone());
                let reply = if command.starts_with("SEMANTIC_SNAPSHOT") {
                    serde_json::to_string(&SemanticSurfaceSnapshot::new(
                        "native-1",
                        7,
                        ComponentNode::new("root", ComponentRole::Group),
                    ))
                    .unwrap()
                } else if command.starts_with("SEMANTIC_PUBLISH") {
                    "SEMANTIC_PUBLISHED window=native-1".to_string()
                } else if command.starts_with("SEMANTIC_ACTION") {
                    "ERR SEMANTIC_ACTION unsupported window=native-1 component=field action=set"
                        .to_string()
                } else {
                    "ERR SEMANTIC_FOCUS unsupported window=native-1 component=field".to_string()
                };
                stream.write_all(reply.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
            seen
        });

        let surface = Kittwm::connect_path(&path).surface("native-1");
        let snapshot = surface.semantic_snapshot().unwrap();
        assert_eq!(snapshot.surface, "native-1");
        assert_eq!(
            surface.semantic_publish(&snapshot).unwrap().trim(),
            "SEMANTIC_PUBLISHED window=native-1"
        );
        assert!(matches!(
            surface.semantic_action("field", "set", serde_json::json!({"value":"x"})),
            Err(Error::Daemon(_))
        ));
        assert!(matches!(
            surface.semantic_focus("field"),
            Err(Error::Daemon(_))
        ));
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen[0], "SEMANTIC_SNAPSHOT native-1");
        assert!(seen[1].starts_with("SEMANTIC_PUBLISH native-1 {"));
        assert_eq!(
            seen[2],
            "SEMANTIC_ACTION native-1 field set {\"value\":\"x\"}"
        );
        assert_eq!(seen[3], "SEMANTIC_FOCUS native-1 field");
    }

    #[cfg(unix)]
    #[test]
    fn semantic_convenience_helpers_send_expected_commands() {
        let path = PathBuf::from(format!(
            "/tmp/kwh-{}-{}.sock",
            std::process::id(),
            now_test_nanos() % 1_000_000
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..7 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                seen.push(request.trim().to_string());
                stream.write_all(b"OK\n").unwrap();
            }
            seen
        });

        let surface = Kittwm::connect_path(&path).surface("native-1");
        let _ = surface.semantic_focus_component("field");
        let _ = surface.semantic_toggle("flag");
        let _ = surface.semantic_set_text("name", "Ada");
        let _ = surface.semantic_insert_text("name", "!");
        let _ = surface.semantic_set_number("volume", 0.5);
        let _ = surface.semantic_set_bool("flag", true);
        let _ = surface.semantic_select_many("choices", ["a", "b"]);
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen[0], "SEMANTIC_FOCUS native-1 field");
        assert_eq!(seen[1], "SEMANTIC_ACTION native-1 flag toggle {}");
        assert_eq!(
            seen[2],
            "SEMANTIC_ACTION native-1 name set {\"text\":\"Ada\"}"
        );
        assert_eq!(
            seen[3],
            "SEMANTIC_ACTION native-1 name insert_text {\"text\":\"!\"}"
        );
        assert_eq!(
            seen[4],
            "SEMANTIC_ACTION native-1 volume set {\"value\":0.5}"
        );
        assert_eq!(
            seen[5],
            "SEMANTIC_ACTION native-1 flag set {\"value\":true}"
        );
        assert_eq!(
            seen[6],
            "SEMANTIC_ACTION native-1 choices select {\"selection\":[\"a\",\"b\"]}"
        );
    }

    #[cfg(unix)]
    fn now_test_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
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
