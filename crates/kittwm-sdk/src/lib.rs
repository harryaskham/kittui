//! Small typed client skeleton for kittwm's DISPLAY/socket control plane.
//!
//! This crate intentionally starts as a thin wrapper around the existing native
//! socket protocol. As the SDK grows, the public handle types here should remain
//! the app-facing API while the transport can evolve underneath.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use std::env;
use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::vec::IntoIter;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result alias for kittwm SDK calls.
pub type Result<T> = std::result::Result<T, Error>;

const NORD0: &str = "#2e3440";
const NORD4: &str = "#d8dee9";

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
    /// YAML decoding/encoding failed.
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
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

/// Top-level kittwm YAML configuration loaded from
/// `~/.config/kittwm/config.yaml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KittwmConfig {
    /// Config schema version.
    pub schema_version: u32,
    /// Background surface defaults.
    pub background: BackgroundConfig,
    /// Terminal/app colorscheme exported to SDK apps.
    pub colorscheme: ColorScheme,
    /// Default terminal launch/rendering policy.
    pub terminal: TerminalConfig,
    /// Libghostty-backed terminal renderer options.
    pub libghostty: LibghosttyConfig,
}

/// Background surface configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundConfig {
    /// Base background color. Named colors such as `nord0` are accepted by
    /// higher-level renderers and preserved here.
    pub color: String,
    /// Background opacity from 0.0 to 1.0.
    pub opacity: f32,
    /// Declarative background effects in render order.
    pub effects: Vec<BackgroundEffectConfig>,
}

/// One declarative background effect entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundEffectConfig {
    /// Effect kind, for example `lens_flare`.
    pub kind: String,
    /// Palette/preset used by the effect, for example `nord_aurora`.
    pub palette: String,
    /// Effect opacity from 0.0 to 1.0.
    pub opacity: f32,
}

/// SDK-visible terminal/app colorscheme.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColorScheme {
    /// Scheme name.
    pub name: String,
    /// Default foreground color.
    pub fg: String,
    /// Default background color.
    pub bg: String,
    /// ANSI colors 0 through 15.
    pub colors: [String; 16],
}

/// Default terminal launch configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalConfig {
    /// Backend selector: `pty` or `ghostty`.
    pub backend: String,
    /// Command launched by Ctrl-A t / Ctrl-A Enter.
    pub command: Option<String>,
}

/// Libghostty-backed terminal renderer configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibghosttyConfig {
    /// Renderer/theme preset name. `nord` is the current built-in preset.
    pub theme: String,
    /// Background color as hex or a known Nord name.
    pub background: String,
    /// Background alpha from 0.0 to 1.0.
    pub background_opacity: f32,
    /// Foreground color as hex or a known Nord name.
    pub foreground: String,
    /// Cursor/accent color as hex or a known Nord name.
    pub cursor: String,
    /// Prefer Ghostty/libghostty feature support such as kitty graphics when the linked libghostty-vt exposes it.
    pub enable_ghostty_features: bool,
    /// Prefer/advertise kitty graphics support for the inner libghostty terminal where available.
    pub kitty_graphics: bool,
}

impl KittwmConfig {
    /// Built-in sane default config. Today this is Nord with a nord0 base
    /// background and a Nord Aurora lens-flare background effect at 0.6 opacity.
    pub fn nord_default() -> Self {
        Self {
            schema_version: 1,
            background: BackgroundConfig::nord_default(),
            colorscheme: ColorScheme::nord(),
            terminal: TerminalConfig::default(),
            libghostty: LibghosttyConfig::default(),
        }
    }

    /// Load config from the default kittwm config path, returning Nord defaults
    /// when no file exists.
    pub fn load_default() -> Result<Self> {
        Self::load_path(default_config_path())
    }

    /// Load config from a specific path, returning Nord defaults when the file
    /// does not exist.
    pub fn load_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::nord_default());
        }
        let bytes = std::fs::read(path)?;
        Ok(serde_yaml::from_slice(&bytes)?)
    }

    /// Render this config as YAML suitable for `~/.config/kittwm/config.yaml`.
    pub fn to_yaml_string(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }
}

impl Default for KittwmConfig {
    fn default() -> Self {
        Self::nord_default()
    }
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self::nord_default()
    }
}

impl Default for BackgroundEffectConfig {
    fn default() -> Self {
        Self {
            kind: "lens_flare".to_string(),
            palette: "nord_aurora".to_string(),
            opacity: 0.6,
        }
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::nord()
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            backend: "ghostty".to_string(),
            command: None,
        }
    }
}

impl Default for LibghosttyConfig {
    fn default() -> Self {
        Self {
            theme: "nord".to_string(),
            background: "nord0".to_string(),
            background_opacity: 0.72,
            foreground: NORD4.to_string(),
            cursor: "nord13".to_string(),
            enable_ghostty_features: true,
            kitty_graphics: true,
        }
    }
}

impl BackgroundConfig {
    /// Nord default background config: nord0 plus aurora lens flare at 0.6.
    pub fn nord_default() -> Self {
        Self {
            color: "nord0".to_string(),
            opacity: 0.6,
            effects: vec![BackgroundEffectConfig {
                kind: "lens_flare".to_string(),
                palette: "nord_aurora".to_string(),
                opacity: 0.6,
            }],
        }
    }
}

impl ColorScheme {
    /// Built-in Nord colorscheme.
    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            fg: NORD4.to_string(),
            bg: NORD0.to_string(),
            colors: [
                "#3b4252".to_string(),
                "#bf616a".to_string(),
                "#a3be8c".to_string(),
                "#ebcb8b".to_string(),
                "#81a1c1".to_string(),
                "#b48ead".to_string(),
                "#88c0d0".to_string(),
                "#e5e9f0".to_string(),
                "#4c566a".to_string(),
                "#bf616a".to_string(),
                "#a3be8c".to_string(),
                "#ebcb8b".to_string(),
                "#81a1c1".to_string(),
                "#b48ead".to_string(),
                "#8fbcbb".to_string(),
                "#eceff4".to_string(),
            ],
        }
    }

    /// ANSI color by index 0 through 15.
    pub fn ansi_color(&self, index: usize) -> Option<&str> {
        self.colors.get(index).map(String::as_str)
    }
}

/// Default kittwm YAML config path.
pub fn default_config_path() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("kittwm/config.yaml");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".config/kittwm/config.yaml");
    }
    PathBuf::from("kittwm/config.yaml")
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

    /// Allow no SDK operations. Useful as a baseline for explicit opt-in.
    pub fn none() -> Self {
        Self {
            allowed: Vec::new(),
        }
    }

    /// Allow only low-risk status/inspection helpers. Raw requests and mutation
    /// operations are denied.
    pub fn inspect_only() -> Self {
        Self {
            allowed: vec![
                Capability::ReadText,
                Capability::SubscribeEvents,
                Capability::ReadSemanticTree,
            ],
        }
    }

    /// Allow common automation of existing surfaces without creating/replacing
    /// windows or invoking semantic actions.
    pub fn automation() -> Self {
        Self {
            allowed: vec![
                Capability::ControlWindow,
                Capability::SendInput,
                Capability::ReadText,
                Capability::SubscribeEvents,
                Capability::ReadSemanticTree,
            ],
        }
    }

    /// Allow only the original minimal read scope. Prefer [`inspect_only`](Self::inspect_only)
    /// for new inspection clients that need events or semantic reads.
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

    /// Borrow the allowed capability list in declaration order.
    pub fn allowed(&self) -> &[Capability] {
        &self.allowed
    }

    /// Iterate over allowed capabilities.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.allowed.iter().copied()
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
    /// Browser-backed surface. Today this asks kittwm to spawn the first-party
    /// `kittwm-browser` command; live kittwm recognizes that command and hosts a
    /// direct browser capture surface instead of a terminal PTY.
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

/// Typed placement role derived from a composition plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacePlacementRole {
    /// App/content surface plane.
    AppSurface,
    /// Decoration/chrome plane.
    Decoration,
    /// Overlay plane.
    Overlay,
}

impl SurfacePlacementRole {
    /// Convert a composition plane name into a typed role.
    pub fn from_plane(plane: &str) -> Option<Self> {
        match plane {
            "app-surfaces" => Some(Self::AppSurface),
            "decorations" => Some(Self::Decoration),
            "overlays" => Some(Self::Overlay),
            _ => None,
        }
    }

    /// Return the canonical composition plane name for this role.
    pub fn plane_name(self) -> &'static str {
        match self {
            Self::AppSurface => "app-surfaces",
            Self::Decoration => "decorations",
            Self::Overlay => "overlays",
        }
    }
}

/// Placement/readiness metadata for a typed surface request, derived from the
/// current architecture contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacePlacementContract {
    /// First-party surface name backing the request.
    pub surface: String,
    /// SDK/control-plane surface kind.
    pub surface_kind: String,
    /// SDK entry point apps should use.
    pub sdk_entry: String,
    /// Whether this request is SDK-backed.
    pub sdk_backed: bool,
    /// Whether this request is kitty-graphics-native.
    pub kitty_graphics_native: bool,
    /// Whether the current SDK + kitty-native contract is ready.
    pub native_ready: bool,
    /// Composition plane for kitty/kittui placement.
    pub composition_plane: String,
    /// Kitty/kittui placement z-index.
    pub z_index: i32,
    /// kittui/kittwm rendering entry point responsible for the graphics path.
    pub kittui_entry: String,
}

/// Aggregate coverage counts for first-party native surface placement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacePlacementCoverage {
    /// Number of first-party native surfaces named by the architecture contract.
    pub total_surfaces: usize,
    /// Number of first-party surfaces with a complete placement contract.
    pub placement_contracts: usize,
    /// Number of placement contracts that are SDK-backed, kitty-native, and
    /// have a kittui entry point.
    pub ready_placement_contracts: usize,
    /// Number of app/content surface placement contracts.
    pub app_surfaces: usize,
    /// Number of decoration/chrome placement contracts.
    pub decorations: usize,
    /// Number of overlay placement contracts.
    pub overlays: usize,
    /// Whether every first-party native surface is SDK-backed and kitty-native.
    pub all_native_surfaces_ready: bool,
    /// Whether every complete placement contract is native-ready and all
    /// first-party native surfaces have placement contracts.
    pub all_placement_contracts_ready: bool,
}

/// Count of placement contracts for one typed composition role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacePlacementRoleCoverage {
    /// Typed role represented by this count.
    pub role: SurfacePlacementRole,
    /// Canonical composition plane name for the role.
    pub composition_plane: String,
    /// Number of placement contracts in this role.
    pub count: usize,
}

impl SurfacePlacementCoverage {
    /// Number of placement contracts for a typed role.
    pub fn count_for_role(&self, role: SurfacePlacementRole) -> usize {
        match role {
            SurfacePlacementRole::AppSurface => self.app_surfaces,
            SurfacePlacementRole::Decoration => self.decorations,
            SurfacePlacementRole::Overlay => self.overlays,
        }
    }

    /// Whether at least one placement contract exists for a typed role.
    pub fn has_role(&self, role: SurfacePlacementRole) -> bool {
        self.count_for_role(role) > 0
    }

    /// Serializable role-count breakdown in compositor role order.
    pub fn role_breakdown(&self) -> Vec<SurfacePlacementRoleCoverage> {
        [
            SurfacePlacementRole::AppSurface,
            SurfacePlacementRole::Decoration,
            SurfacePlacementRole::Overlay,
        ]
        .into_iter()
        .map(|role| SurfacePlacementRoleCoverage {
            role,
            composition_plane: role.plane_name().to_string(),
            count: self.count_for_role(role),
        })
        .collect()
    }

    /// Number of first-party surfaces that did not produce a placement contract.
    pub fn missing_placement_contracts(&self) -> usize {
        self.total_surfaces.saturating_sub(self.placement_contracts)
    }

    /// Number of placement contracts that are present but not native-ready.
    pub fn not_ready_placement_contracts(&self) -> usize {
        self.placement_contracts
            .saturating_sub(self.ready_placement_contracts)
    }

    /// Total count of explicit placement coverage gaps.
    pub fn placement_gap_count(&self) -> usize {
        self.missing_placement_contracts() + self.not_ready_placement_contracts()
    }

    /// Whether native surface and placement contract coverage is complete.
    pub fn is_complete(&self) -> bool {
        self.all_native_surfaces_ready
            && self.all_placement_contracts_ready
            && self.placement_gap_count() == 0
    }

    /// Whether the placement coverage summary identifies any gap.
    pub fn has_gaps(&self) -> bool {
        !self.is_complete()
    }
}

impl SurfacePlacementContract {
    /// Build placement/readiness metadata from a native surface contract and
    /// architecture contract.
    pub fn from_native_surface(
        surface: &NativeSurfaceContract,
        contract: &ArchitectureContract,
    ) -> Option<Self> {
        Some(Self {
            surface: surface.name.clone(),
            surface_kind: surface.surface_kind.clone(),
            sdk_entry: surface.sdk_entry.clone(),
            sdk_backed: surface.sdk_backed,
            kitty_graphics_native: surface.kitty_graphics_native,
            native_ready: surface.is_native_ready(),
            composition_plane: surface.composition_plane()?.to_string(),
            z_index: surface.z_index(contract)?,
            kittui_entry: surface.kittui_entry.clone(),
        })
    }

    /// Typed role for this placement contract.
    pub fn role(&self) -> Option<SurfacePlacementRole> {
        SurfacePlacementRole::from_plane(&self.composition_plane)
    }

    /// Whether this placement belongs to the app/content plane.
    pub fn is_app_surface(&self) -> bool {
        self.role() == Some(SurfacePlacementRole::AppSurface)
    }

    /// Whether this placement belongs to the decoration/chrome plane.
    pub fn is_decoration(&self) -> bool {
        self.role() == Some(SurfacePlacementRole::Decoration)
    }

    /// Whether this placement belongs to the overlay plane.
    pub fn is_overlay(&self) -> bool {
        self.role() == Some(SurfacePlacementRole::Overlay)
    }

    /// Whether this placement is above another placement by z-index.
    pub fn is_above(&self, other: &SurfacePlacementContract) -> bool {
        self.z_index > other.z_index
    }

    /// Whether this placement is below another placement by z-index.
    pub fn is_below(&self, other: &SurfacePlacementContract) -> bool {
        self.z_index < other.z_index
    }
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

    /// Build a process-viewer surface spec using the first-party `kittwm-top` app.
    pub fn top() -> Self {
        Self::terminal("kittwm-top")
    }

    /// Return the concrete command used by the current v0 native socket
    /// transport for this typed surface. Browser specs still use a
    /// `kittwm-browser` command string, and live kittwm recognizes that command
    /// as a direct browser capture surface rather than a terminal PTY.
    pub fn native_pty_command(&self) -> Result<String> {
        match &self.kind {
            SurfaceKind::Terminal => {
                Ok(validated_nonempty_trimmed(&self.command, "terminal command")?.to_string())
            }
            SurfaceKind::Browser => Ok(browser_surface_command(validated_nonempty_trimmed(
                &self.command,
                "browser target",
            )?)),
            SurfaceKind::Other(kind) => Err(Error::Daemon(format!(
                "surface kind {kind:?} is not supported by the SDK transport"
            ))),
        }
    }

    /// Return the current first-party native surface contract that backs this
    /// typed surface request, if any.
    pub fn native_surface_contract(&self) -> Option<NativeSurfaceContract> {
        ArchitectureContract::current()
            .native_surface_for_spec(self)
            .cloned()
    }

    /// Whether this typed surface request is covered by the current SDK-backed,
    /// kitty-graphics-native first-party surface contract.
    pub fn is_native_ready(&self) -> bool {
        self.native_surface_contract()
            .map(|surface| surface.is_native_ready())
            .unwrap_or(false)
    }

    /// Composition plane for this typed surface request in the current kittwm
    /// architecture contract.
    pub fn composition_plane(&self) -> Option<&'static str> {
        self.native_surface_contract()
            .and_then(|surface| surface.composition_plane())
    }

    /// Kitty/kittui placement z-index for this typed surface request in the
    /// current kittwm architecture contract.
    pub fn z_index(&self) -> Option<i32> {
        let contract = ArchitectureContract::current();
        contract
            .native_surface_for_spec(self)
            .and_then(|surface| surface.z_index(&contract))
    }

    /// Return a compact placement/readiness contract for this typed surface
    /// request, if the current architecture names a backing first-party surface.
    pub fn placement_contract(&self) -> Option<SurfacePlacementContract> {
        let contract = ArchitectureContract::current();
        contract.placement_contract_for_spec(self)
    }

    /// Attach a display title.
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// Machine-readable kittwm architecture/separation contract.
///
/// This is the typed SDK model behind `kittwm architecture-json`. It is not a
/// live daemon capability negotiation; it is a stable contract that app authors
/// and tests can use to keep SDK/control-plane, tiling, surface rendering,
/// decoration rendering, and kitty transport responsibilities separated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureContract {
    /// Contract schema version.
    pub schema_version: u32,
    /// Artifact kind string.
    pub kind: String,
    /// Human-readable platform goal.
    pub goal: String,
    /// Ordered architecture layers and their boundaries.
    pub layers: Vec<ArchitectureLayer>,
    /// Expected compositor plane ordering.
    pub composition_order: Vec<CompositionPlane>,
    /// First-party native surfaces and their SDK entry points.
    pub first_party_native_surfaces: Vec<NativeSurfaceContract>,
    /// Inspection artifacts that expose the contract or adjacent runtime state.
    pub inspection_artifacts: Vec<String>,
}

/// One responsibility layer in the kittwm architecture contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureLayer {
    /// Stable layer id.
    pub id: String,
    /// Code/module owner for the layer.
    pub owner: String,
    /// Responsibilities owned by this layer.
    pub responsibilities: Vec<String>,
    /// Responsibilities this layer must avoid.
    pub must_not: Vec<String>,
    /// Runtime/layout invariants, when applicable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariants: Vec<String>,
    /// Public/native contracts associated with this layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_contracts: Vec<String>,
}

/// One compositor plane in the WM composition order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositionPlane {
    /// Plane name.
    pub plane: String,
    /// Kitty/kittui placement z-index used by the current contract.
    pub z_index: i32,
}

impl CompositionPlane {
    /// Whether this plane is above another plane in kitty/kittui z-order.
    pub fn is_above(&self, other: &CompositionPlane) -> bool {
        self.z_index > other.z_index
    }

    /// Whether this plane is below another plane in kitty/kittui z-order.
    pub fn is_below(&self, other: &CompositionPlane) -> bool {
        self.z_index < other.z_index
    }
}

/// First-party native surface contract exposed through the SDK/platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeSurfaceContract {
    /// Binary or surface name.
    pub name: String,
    /// SDK/control-plane surface kind.
    pub surface_kind: String,
    /// SDK entry point apps should use.
    pub sdk_entry: String,
    /// Whether normal apps can request/control this surface through typed SDK
    /// APIs rather than raw socket strings.
    pub sdk_backed: bool,
    /// Whether the current first-party path is kitty-graphics-native instead of
    /// a pure text placeholder/fallback.
    pub kitty_graphics_native: bool,
    /// kittui/kittwm rendering entry point responsible for the native graphics
    /// path.
    pub kittui_entry: String,
    /// Current rendering path summary.
    pub rendering: String,
}

impl NativeSurfaceContract {
    /// Whether this first-party surface is fully represented by the current
    /// SDK + kitty-graphics-native contract.
    pub fn is_native_ready(&self) -> bool {
        self.sdk_backed && self.kitty_graphics_native && !self.kittui_entry.trim().is_empty()
    }

    /// Composition plane used by this first-party surface kind.
    ///
    /// Terminal and browser surfaces are app content, while chrome surfaces are
    /// decorations. Unknown/future surface kinds intentionally return `None`
    /// until the architecture contract names their plane.
    pub fn composition_plane(&self) -> Option<&'static str> {
        match self.surface_kind.as_str() {
            "terminal" | "browser" => Some("app-surfaces"),
            "chrome" => Some("decorations"),
            _ => None,
        }
    }

    /// Resolve this surface's current z-index from an architecture contract.
    pub fn z_index(&self, contract: &ArchitectureContract) -> Option<i32> {
        self.composition_plane()
            .and_then(|plane| contract.z_index_for_plane(plane))
    }
}

impl ArchitectureContract {
    /// Return the current built-in kittwm platform contract.
    pub fn current() -> Self {
        Self {
            schema_version: 1,
            kind: "kittwm-architecture-contract".to_string(),
            goal: "usable kitty-graphics-backed terminal window manager with explicit separation of concerns".to_string(),
            layers: vec![
                ArchitectureLayer {
                    id: "sdk-control-plane".to_string(),
                    owner: "kittwm-sdk".to_string(),
                    responsibilities: strings(&[
                        "typed app-facing surface vocabulary",
                        "socket/display discovery",
                        "least-privilege client capabilities",
                        "status/panes/chrome/events/semantic automation helpers",
                    ]),
                    must_not: strings(&[
                        "decide pane geometry",
                        "emit kitty graphics escape sequences",
                        "know terminal chrome pixel placement",
                    ]),
                    invariants: Vec::new(),
                    native_contracts: strings(&[
                        "SurfaceSpec::terminal",
                        "SurfaceSpec::browser",
                        "ChromeReservationRequest",
                        "ChromeReservationStatus",
                        "SemanticSurfaceSnapshot",
                    ]),
                },
                ArchitectureLayer {
                    id: "tiling-engine".to_string(),
                    owner: "kittwm native session layout".to_string(),
                    responsibilities: strings(&[
                        "consume reported terminal cols/rows",
                        "apply chrome reservations and tile gaps",
                        "produce disjoint outer/app bounds",
                        "route focus and pointer/app-local coordinates",
                    ]),
                    must_not: strings(&[
                        "upload images",
                        "paint decorations",
                        "query application semantics",
                    ]),
                    invariants: strings(&[
                        "outer bounds are disjoint",
                        "app bounds are disjoint and inside outer bounds",
                        "drawable rows never exceed reported rows minus reservations",
                        "resize recomputes all pane bounds before surface resize",
                    ]),
                    native_contracts: Vec::new(),
                },
                ArchitectureLayer {
                    id: "surface-renderer".to_string(),
                    owner: "NativeSurface adapters + kittui::Runtime".to_string(),
                    responsibilities: strings(&[
                        "capture PTY/browser/native surfaces into frames or scenes",
                        "fit frames to allocated app cell bounds",
                        "cache/upload/place kitty images",
                        "honor explicit placement/z-plane options",
                    ]),
                    must_not: strings(&[
                        "allocate tiles",
                        "draw WM decorations",
                        "consume SDK policy directly",
                    ]),
                    invariants: Vec::new(),
                    native_contracts: strings(&[
                        "Runtime::place_at_with_options",
                        "Runtime::place_raw_frame_with_options",
                        "Runtime::place_uploaded_image_with_options",
                        "KITTWM_NATIVE_RENDERER=kitty|terminal",
                    ]),
                },
                ArchitectureLayer {
                    id: "decoration-renderer".to_string(),
                    owner: "kittui-affordances + kittwm chrome helpers".to_string(),
                    responsibilities: strings(&[
                        "render top bar, pane titles, borders, footer, overlays as kittui scenes",
                        "use shared theme/style tokens",
                        "label scene layers for diagnostics",
                        "stay above app surfaces on a dedicated z-plane",
                    ]),
                    must_not: strings(&[
                        "capture app pixels",
                        "resize PTYs/browser surfaces",
                        "own app input routing",
                    ]),
                    invariants: Vec::new(),
                    native_contracts: strings(&[
                        "kittwm-bar --scene-json",
                        "kittwm showcase-scene-json",
                        "kittwm showcase-composition-json",
                    ]),
                },
                ArchitectureLayer {
                    id: "kitty-compositor".to_string(),
                    owner: "kittui-kitty transport grammar".to_string(),
                    responsibilities: strings(&[
                        "encode kitty graphics upload/placement/delete commands",
                        "support direct/tmux/file/shared-memory transports",
                        "provide absolute or unicode-placeholder placement options",
                    ]),
                    must_not: strings(&[
                        "know about panes or workspaces",
                        "choose WM layout policy",
                        "special-case first-party apps",
                    ]),
                    invariants: Vec::new(),
                    native_contracts: Vec::new(),
                },
            ],
            composition_order: vec![
                CompositionPlane { plane: "app-surfaces".to_string(), z_index: 0 },
                CompositionPlane { plane: "decorations".to_string(), z_index: 20 },
                CompositionPlane { plane: "overlays".to_string(), z_index: 30 },
            ],
            first_party_native_surfaces: vec![
                NativeSurfaceContract {
                    name: "kittwm-terminal".to_string(),
                    surface_kind: "terminal".to_string(),
                    sdk_entry: "SurfaceSpec::terminal".to_string(),
                    sdk_backed: true,
                    kitty_graphics_native: true,
                    kittui_entry: "PtyTerminalApp -> Runtime::place_raw_frame_with_options".to_string(),
                    rendering: "PTY NativeSurface -> fitted app frame -> kitty graphics".to_string(),
                },
                NativeSurfaceContract {
                    name: "kittwm-top".to_string(),
                    surface_kind: "terminal".to_string(),
                    sdk_entry: "SurfaceSpec::top".to_string(),
                    sdk_backed: true,
                    kitty_graphics_native: true,
                    kittui_entry: "Kittwm::processes -> terminal table renderer".to_string(),
                    rendering: "SDK process snapshot -> hosted terminal text surface".to_string(),
                },
                NativeSurfaceContract {
                    name: "kittwm-browser".to_string(),
                    surface_kind: "browser".to_string(),
                    sdk_entry: "SurfaceSpec::browser".to_string(),
                    sdk_backed: true,
                    kitty_graphics_native: true,
                    kittui_entry: "HeadlessBrowserApp -> Runtime::place_png_frame_with_options".to_string(),
                    rendering: "HeadlessBrowserApp frame -> absolute kitty graphics placement".to_string(),
                },
                NativeSurfaceContract {
                    name: "kittwm-bar".to_string(),
                    surface_kind: "chrome".to_string(),
                    sdk_entry: "Kittwm::chrome / ChromeReservationRequest".to_string(),
                    sdk_backed: true,
                    kitty_graphics_native: true,
                    kittui_entry: "BarModel::scene -> Runtime::place_at_with_options".to_string(),
                    rendering: "BarModel -> kittui Scene JSON / kitty graphics chrome".to_string(),
                },
            ],
            inspection_artifacts: strings(&[
                "kittwm architecture-json",
                "kittwm commands-json",
                "kittwm showcase-composition-json",
                "kittwm tui-smoke-json",
                "STATUS_JSON",
                "PANES_JSON",
                "CHROME_JSON",
            ]),
        }
    }

    /// Look up a layer by stable id.
    pub fn layer(&self, id: &str) -> Option<&ArchitectureLayer> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    /// Look up a compositor plane by stable plane name, such as
    /// `app-surfaces`, `decorations`, or `overlays`.
    pub fn composition_plane(&self, plane: &str) -> Option<&CompositionPlane> {
        self.composition_order
            .iter()
            .find(|entry| entry.plane == plane)
    }

    /// Return the z-index for a named compositor plane.
    pub fn z_index_for_plane(&self, plane: &str) -> Option<i32> {
        self.composition_plane(plane).map(|entry| entry.z_index)
    }

    /// Look up the compositor plane for a typed placement role.
    pub fn composition_plane_for_role(
        &self,
        role: SurfacePlacementRole,
    ) -> Option<&CompositionPlane> {
        self.composition_plane(role.plane_name())
    }

    /// Return the z-index for a typed placement role.
    pub fn z_index_for_role(&self, role: SurfacePlacementRole) -> Option<i32> {
        self.z_index_for_plane(role.plane_name())
    }

    /// Iterate compositor plane names in the contract's intended composition
    /// order, from lower app/content planes to higher overlay planes.
    pub fn ordered_plane_names(&self) -> impl Iterator<Item = &str> {
        self.composition_order
            .iter()
            .map(|plane| plane.plane.as_str())
    }

    /// Whether `upper` is above `lower` in kitty/kittui z-order.
    pub fn plane_is_above(&self, upper: &str, lower: &str) -> Option<bool> {
        let upper = self.composition_plane(upper)?;
        let lower = self.composition_plane(lower)?;
        Some(upper.is_above(lower))
    }

    /// Current z-index for app surface placements.
    pub fn app_surface_z_index(&self) -> Option<i32> {
        self.z_index_for_plane("app-surfaces")
    }

    /// Current z-index for WM decoration/chrome placements.
    pub fn decoration_z_index(&self) -> Option<i32> {
        self.z_index_for_plane("decorations")
    }

    /// Current z-index for overlay placements.
    pub fn overlay_z_index(&self) -> Option<i32> {
        self.z_index_for_plane("overlays")
    }

    /// Look up a first-party native surface by binary/surface name.
    pub fn native_surface(&self, name: &str) -> Option<&NativeSurfaceContract> {
        self.first_party_native_surfaces
            .iter()
            .find(|surface| surface.name == name)
    }

    /// Look up the first first-party native surface with the given SDK/control
    /// plane surface kind, such as `terminal`, `browser`, or `chrome`.
    pub fn native_surface_by_kind(&self, kind: &str) -> Option<&NativeSurfaceContract> {
        self.first_party_native_surfaces
            .iter()
            .find(|surface| surface.surface_kind == kind)
    }

    /// Iterate all first-party native surfaces with the given SDK/control-plane
    /// surface kind.
    pub fn native_surfaces_by_kind<'a>(
        &'a self,
        kind: &'a str,
    ) -> impl Iterator<Item = &'a NativeSurfaceContract> + 'a {
        self.first_party_native_surfaces
            .iter()
            .filter(move |surface| surface.surface_kind == kind)
    }

    /// Look up the first-party native surface contract that backs a typed
    /// surface request. Terminal and browser specs are first-class in the
    /// current architecture contract; unsupported `Other` specs intentionally
    /// return `None`.
    pub fn native_surface_for_spec(&self, spec: &SurfaceSpec) -> Option<&NativeSurfaceContract> {
        match &spec.kind {
            SurfaceKind::Terminal if spec.command.trim() == "kittwm-top" => {
                self.native_surface("kittwm-top")
            }
            SurfaceKind::Terminal => self.native_surface_by_kind("terminal"),
            SurfaceKind::Browser => self.native_surface_by_kind("browser"),
            SurfaceKind::Other(_) => None,
        }
    }

    /// Build a placement/readiness contract for a first-party native surface
    /// by surface name.
    pub fn placement_contract_for_surface(&self, name: &str) -> Option<SurfacePlacementContract> {
        let surface = self.native_surface(name)?;
        SurfacePlacementContract::from_native_surface(surface, self)
    }

    /// Build a placement/readiness contract for the first native surface of a
    /// given SDK/control-plane kind.
    pub fn placement_contract_for_kind(&self, kind: &str) -> Option<SurfacePlacementContract> {
        let surface = self.native_surface_by_kind(kind)?;
        SurfacePlacementContract::from_native_surface(surface, self)
    }

    /// Build a placement/readiness contract for a typed surface request.
    pub fn placement_contract_for_spec(
        &self,
        spec: &SurfaceSpec,
    ) -> Option<SurfacePlacementContract> {
        let surface = self.native_surface_for_spec(spec)?;
        SurfacePlacementContract::from_native_surface(surface, self)
    }

    /// Build placement/readiness contracts for all first-party native surfaces
    /// named by this architecture contract.
    pub fn placement_contracts(&self) -> Vec<SurfacePlacementContract> {
        self.first_party_native_surfaces
            .iter()
            .filter_map(|surface| SurfacePlacementContract::from_native_surface(surface, self))
            .collect()
    }

    /// Build placement/readiness contracts for first-party surfaces that are
    /// currently SDK-backed, kitty-graphics-native, and have a kittui entry.
    pub fn ready_placement_contracts(&self) -> Vec<SurfacePlacementContract> {
        self.placement_contracts()
            .into_iter()
            .filter(|contract| contract.native_ready)
            .collect()
    }

    /// Build placement/readiness contracts that are present but not fully
    /// native-ready.
    pub fn not_ready_placement_contracts(&self) -> Vec<SurfacePlacementContract> {
        self.placement_contracts()
            .into_iter()
            .filter(|contract| !contract.native_ready)
            .collect()
    }

    /// First-party native surfaces that do not currently produce a placement
    /// contract, typically because a plane or z-index is missing from the
    /// architecture contract.
    pub fn missing_placement_contract_surfaces(&self) -> Vec<&NativeSurfaceContract> {
        self.first_party_native_surfaces
            .iter()
            .filter(|surface| {
                SurfacePlacementContract::from_native_surface(surface, self).is_none()
            })
            .collect()
    }

    /// Build placement/readiness contracts sorted in compositor z-index order.
    pub fn placement_contracts_in_composition_order(&self) -> Vec<SurfacePlacementContract> {
        let mut contracts = self.placement_contracts();
        contracts.sort_by_key(|contract| contract.z_index);
        contracts
    }

    /// Build ready placement/readiness contracts sorted in compositor z-index
    /// order.
    pub fn ready_placement_contracts_in_composition_order(&self) -> Vec<SurfacePlacementContract> {
        let mut contracts = self.ready_placement_contracts();
        contracts.sort_by_key(|contract| contract.z_index);
        contracts
    }

    /// Build placement/readiness contracts for first-party surfaces that belong
    /// to a typed placement role.
    pub fn placement_contracts_for_role(
        &self,
        role: SurfacePlacementRole,
    ) -> Vec<SurfacePlacementContract> {
        self.placement_contracts()
            .into_iter()
            .filter(|contract| contract.role() == Some(role))
            .collect()
    }

    /// Build placement/readiness contracts for app/content surfaces.
    pub fn app_surface_placement_contracts(&self) -> Vec<SurfacePlacementContract> {
        self.placement_contracts_for_role(SurfacePlacementRole::AppSurface)
    }

    /// Build placement/readiness contracts for decoration/chrome surfaces.
    pub fn decoration_placement_contracts(&self) -> Vec<SurfacePlacementContract> {
        self.placement_contracts_for_role(SurfacePlacementRole::Decoration)
    }

    /// Build placement/readiness contracts for overlay surfaces.
    pub fn overlay_placement_contracts(&self) -> Vec<SurfacePlacementContract> {
        self.placement_contracts_for_role(SurfacePlacementRole::Overlay)
    }

    /// Summarize first-party native surface placement coverage.
    pub fn placement_coverage(&self) -> SurfacePlacementCoverage {
        let contracts = self.placement_contracts();
        let ready_placement_contracts = contracts
            .iter()
            .filter(|contract| contract.native_ready)
            .count();
        SurfacePlacementCoverage {
            total_surfaces: self.first_party_native_surfaces.len(),
            placement_contracts: contracts.len(),
            ready_placement_contracts,
            app_surfaces: contracts
                .iter()
                .filter(|contract| contract.is_app_surface())
                .count(),
            decorations: contracts
                .iter()
                .filter(|contract| contract.is_decoration())
                .count(),
            overlays: contracts
                .iter()
                .filter(|contract| contract.is_overlay())
                .count(),
            all_native_surfaces_ready: self.all_native_surfaces_ready(),
            all_placement_contracts_ready: contracts.len()
                == self.first_party_native_surfaces.len()
                && ready_placement_contracts == contracts.len(),
        }
    }

    /// Iterate first-party surfaces currently represented as SDK-backed,
    /// kitty-graphics-native paths.
    pub fn native_ready_surfaces(&self) -> impl Iterator<Item = &NativeSurfaceContract> {
        self.first_party_native_surfaces
            .iter()
            .filter(|surface| surface.is_native_ready())
    }

    /// Whether every listed first-party native surface is SDK-backed,
    /// kitty-graphics-native, and has a kittui entry point.
    pub fn all_native_surfaces_ready(&self) -> bool {
        self.first_party_native_surfaces
            .iter()
            .all(NativeSurfaceContract::is_native_ready)
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
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

/// Pane-local mouse event label accepted by `SEND_MOUSE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MouseEvent {
    /// Press the primary/left mouse button.
    PressLeft,
    /// Press the middle mouse button.
    PressMiddle,
    /// Press the secondary/right mouse button.
    PressRight,
    /// Release the currently pressed mouse button.
    Release,
    /// Release the primary/left mouse button.
    ReleaseLeft,
    /// Release the middle mouse button.
    ReleaseMiddle,
    /// Release the secondary/right mouse button.
    ReleaseRight,
    /// Move the pointer without a button held.
    Move,
    /// Move while the left button is held.
    MoveLeft,
    /// Move while the middle button is held.
    MoveMiddle,
    /// Move while the right button is held.
    MoveRight,
    /// Scroll up at the given cell.
    ScrollUp,
    /// Scroll down at the given cell.
    ScrollDown,
}

impl MouseEvent {
    fn protocol_label(self) -> &'static str {
        match self {
            Self::PressLeft => "press-left",
            Self::PressMiddle => "press-middle",
            Self::PressRight => "press-right",
            Self::Release => "release",
            Self::ReleaseLeft => "release-left",
            Self::ReleaseMiddle => "release-middle",
            Self::ReleaseRight => "release-right",
            Self::Move => "move",
            Self::MoveLeft => "move-left",
            Self::MoveMiddle => "move-middle",
            Self::MoveRight => "move-right",
            Self::ScrollUp => "scroll-up",
            Self::ScrollDown => "scroll-down",
        }
    }
}

/// Native layout axis accepted by `LAYOUT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// Split panes into columns.
    Columns,
    /// Split panes into rows.
    Rows,
    /// Arrange panes into a balanced grid.
    Grid,
}

impl LayoutMode {
    fn protocol_label(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Rows => "rows",
            Self::Grid => "grid",
        }
    }
}

/// Pane move direction accepted by `MOVE_PANE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveDirection {
    /// Move left in the layout order.
    Left,
    /// Move right in the layout order.
    Right,
    /// Move upward in the layout order.
    Up,
    /// Move downward in the layout order.
    Down,
    /// Move to the first slot.
    First,
    /// Move to the last slot.
    Last,
}

impl MoveDirection {
    fn protocol_label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::First => "first",
            Self::Last => "last",
        }
    }
}

/// Native kittwm session manifest returned by `SESSION_JSON` and accepted by
/// `RESTORE_SESSION_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionManifest {
    /// Manifest schema version when emitted by the daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    /// Manifest kind marker, currently `kittwm-native-session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Layout label such as `columns` or `rows`.
    #[serde(default)]
    pub layout: String,
    /// Focused window id, or `-` when none is known.
    #[serde(default)]
    pub focus: String,
    /// Panes to restore.
    #[serde(default)]
    pub panes: Vec<SessionPane>,
}

/// One pane entry inside a [`SessionManifest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPane {
    /// Stable order index when emitted by `SESSION_JSON`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// Window id when emitted by `SESSION_JSON`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// Optional pane title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Command used to spawn the pane.
    pub command: String,
    /// Relative layout weight.
    #[serde(default = "default_session_pane_weight")]
    pub weight: u16,
    /// Whether this pane should be focused after restore.
    #[serde(default)]
    pub focused: bool,
    /// Floating-pane x offset from the generated floating layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floating_dx: Option<i16>,
    /// Floating-pane y offset from the generated floating layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floating_dy: Option<i16>,
}

fn default_session_pane_weight() -> u16 {
    1
}

/// Title-drag interaction kind reported by kittwm status metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TitleDragKind {
    /// Dragging the title reorders a tiled pane within the tiled stack.
    Reorder,
    /// Dragging the title repositions a floating pane.
    Reposition,
    /// Unknown future/custom title-drag interaction kind.
    Unknown(String),
}

impl TitleDragKind {
    /// Parse a raw title-drag kind string from status metadata.
    pub fn parse(kind: &str) -> Self {
        match kind {
            "reorder" => Self::Reorder,
            "reposition" => Self::Reposition,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Stable string representation for this kind.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Reorder => "reorder",
            Self::Reposition => "reposition",
            Self::Unknown(kind) => kind.as_str(),
        }
    }
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

/// Scrollback snapshot returned by `READ_SCROLLBACK_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollbackSnapshot {
    /// Window id.
    pub window: String,
    /// Lines that have scrolled off the visible screen.
    #[serde(default)]
    pub scrollback: String,
}

/// Kind of successful wait match returned by `WAIT_TEXT*` / `WAIT_OUTPUT*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitMatchKind {
    /// The match came from visible screen text.
    Text,
    /// The match came from visible screen or scrollback output.
    Output,
}

/// Typed metadata parsed from a successful wait reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitMatch {
    /// Match source.
    pub kind: WaitMatchKind,
    /// Window id reported by the daemon.
    pub window: String,
    /// Byte count reported by the daemon.
    pub bytes: u64,
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
    /// Heading/title text.
    Heading,
    /// Paragraph text.
    Paragraph,
    /// Code/preformatted text.
    Code,
    /// Link/navigation target.
    Link,
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
    /// Table/list row.
    Row,
    /// Table/grid cell.
    Cell,
    /// List container.
    List,
    /// List item.
    ListItem,
    /// Tree container.
    Tree,
    /// Tree item.
    TreeItem,
    /// Image/media.
    Image,
    /// Canvas/pixel region.
    Canvas,
    /// Terminal text/grid region.
    Terminal,
    /// Browser document/root.
    BrowserDocument,
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

fn value_u16(value: &Value, key: &str) -> Option<u16> {
    value.get(key)?.as_u64()?.try_into().ok()
}

fn value_u32(value: &Value, key: &str) -> Option<u32> {
    value.get(key)?.as_u64()?.try_into().ok()
}

fn bounds_tuple(value: &Value) -> Option<(u16, u16, u16, u16)> {
    Some((
        value_u16(value, "x")?,
        value_u16(value, "y")?,
        value_u16(value, "cols")?,
        value_u16(value, "rows")?,
    ))
}

fn app_bounds_tuple(value: &Value) -> Option<(u16, u16, u16, u16)> {
    Some((
        value_u16(value, "app_x")?,
        value_u16(value, "app_y")?,
        value_u16(value, "app_cols")?,
        value_u16(value, "app_rows")?,
    ))
}

impl EventEnvelope {
    /// Borrow a string field from the event detail object.
    pub fn detail_str(&self, key: &str) -> Option<&str> {
        self.detail.get(key).and_then(Value::as_str)
    }

    /// Borrow a boolean field from the event detail object.
    pub fn detail_bool(&self, key: &str) -> Option<bool> {
        self.detail.get(key).and_then(Value::as_bool)
    }

    /// Borrow an unsigned integer field from the event detail object.
    pub fn detail_u64(&self, key: &str) -> Option<u64> {
        self.detail.get(key).and_then(Value::as_u64)
    }

    /// Borrow a signed integer field from the event detail object.
    pub fn detail_i64(&self, key: &str) -> Option<i64> {
        self.detail.get(key).and_then(Value::as_i64)
    }

    /// Parse a nested `pane` detail object into typed native pane metadata.
    pub fn pane_detail(&self) -> Option<NativePaneDetail> {
        serde_json::from_value(self.detail.get("pane")?.clone()).ok()
    }

    /// Outer pane bounds from a nested resize detail such as `old` or `new`.
    pub fn pane_bounds(&self, key: &str) -> Option<(u16, u16, u16, u16)> {
        let value = self.detail.get(key)?;
        value
            .get("bounds")
            .and_then(bounds_tuple)
            .or_else(|| bounds_tuple(value))
    }

    /// App/content pane bounds from a nested resize detail such as `old` or `new`.
    pub fn pane_app_bounds(&self, key: &str) -> Option<(u16, u16, u16, u16)> {
        let value = self.detail.get(key)?;
        value
            .get("app_bounds")
            .and_then(bounds_tuple)
            .or_else(|| app_bounds_tuple(value))
    }

    /// `(cols, rows)` delta between two outer pane bounds detail keys.
    pub fn pane_size_delta(&self, old_key: &str, new_key: &str) -> Option<(i32, i32)> {
        let old = self.pane_bounds(old_key)?;
        let new = self.pane_bounds(new_key)?;
        Some((
            i32::from(new.2) - i32::from(old.2),
            i32::from(new.3) - i32::from(old.3),
        ))
    }

    /// `(cols, rows)` delta between two app/content pane bounds detail keys.
    pub fn pane_app_size_delta(&self, old_key: &str, new_key: &str) -> Option<(i32, i32)> {
        let old = self.pane_app_bounds(old_key)?;
        let new = self.pane_app_bounds(new_key)?;
        Some((
            i32::from(new.2) - i32::from(old.2),
            i32::from(new.3) - i32::from(old.3),
        ))
    }

    /// Renderer label reported by a `pane_frame_presented` event.
    pub fn frame_renderer(&self) -> Option<&str> {
        self.detail_str("renderer")
    }

    /// Pixel/data format reported by a `pane_frame_presented` event.
    pub fn frame_format(&self) -> Option<&str> {
        self.detail_str("format")
    }

    /// Pixel size reported by a `pane_frame_presented` event.
    pub fn frame_pixel_size(&self) -> Option<(u32, u32)> {
        let width =
            value_u32(&self.detail, "pixel_width").or_else(|| value_u32(&self.detail, "width"))?;
        let height = value_u32(&self.detail, "pixel_height")
            .or_else(|| value_u32(&self.detail, "height"))?;
        Some((width, height))
    }

    /// App/content bounds reported by a `pane_frame_presented` event.
    pub fn frame_app_bounds(&self) -> Option<(u16, u16, u16, u16)> {
        self.detail.get("app_bounds").and_then(bounds_tuple)
    }

    /// Changed tile count and total tile count reported by a `pane_frame_presented` event.
    pub fn frame_changed_tiles_ratio(&self) -> Option<(u32, u32)> {
        Some((
            value_u32(&self.detail, "changed_tiles")?,
            value_u32(&self.detail, "total_tiles")?,
        ))
    }

    /// Changed tile count and total tile count as `<changed>/<total>`.
    pub fn frame_changed_tiles_label(&self) -> Option<String> {
        let (changed, total) = self.frame_changed_tiles_ratio()?;
        Some(format!("{changed}/{total}"))
    }

    /// Changed tile fraction in `[0, 1]` reported or derived for a `pane_frame_presented` event.
    pub fn frame_changed_fraction(&self) -> Option<f32> {
        if let Some(fraction) = self.detail.get("changed_fraction").and_then(Value::as_f64) {
            return Some(fraction as f32);
        }
        let (changed, total) = self.frame_changed_tiles_ratio()?;
        if total == 0 {
            return None;
        }
        Some(changed as f32 / total as f32)
    }

    /// Changed tile percentage in `[0, 100]` for a `pane_frame_presented` event.
    pub fn frame_changed_percent(&self) -> Option<f32> {
        self.frame_changed_fraction()
            .map(|fraction| fraction * 100.0)
    }

    /// Whether a `pane_frame_presented` event represents a clean frame.
    pub fn frame_is_clean(&self) -> Option<bool> {
        if self.frame_upload_skipped().unwrap_or(false) {
            return Some(true);
        }
        let (changed, _) = self.frame_changed_tiles_ratio()?;
        Some(changed == 0)
    }

    /// Live-style status label for a `pane_frame_presented` event.
    pub fn frame_status_label(&self) -> Option<String> {
        if self.frame_is_clean()? {
            Some("clean".to_string())
        } else {
            self.frame_changed_tiles_label()
        }
    }

    /// Live-style status-chip label for a `pane_frame_presented` event.
    pub fn frame_chip_label(&self) -> Option<String> {
        self.frame_status_label()
            .map(|status| format!("frame:{status}"))
    }

    /// Whether a `pane_frame_presented` event reported a skipped upload.
    pub fn frame_upload_skipped(&self) -> Option<bool> {
        self.detail_bool("skipped_upload")
    }

    /// Whether a `pane_frame_presented` event reported an upload.
    pub fn frame_uploaded(&self) -> Option<bool> {
        self.detail_bool("uploaded")
    }

    /// Uploaded byte count reported by a `pane_frame_presented` event.
    pub fn frame_upload_bytes(&self) -> Option<u64> {
        self.detail_u64("upload_bytes")
    }

    /// Placement command byte count reported by a `pane_frame_presented` event.
    pub fn frame_placement_bytes(&self) -> Option<u64> {
        self.detail_u64("placement_bytes")
    }

    /// Embedded payload byte count reported by a `pane_frame_presented` event.
    pub fn frame_embed_bytes(&self) -> Option<u64> {
        self.detail_u64("embed_bytes")
    }

    /// Sum of reported upload, placement, and embedded byte counts.
    pub fn frame_total_transport_bytes(&self) -> Option<u64> {
        let mut total = 0_u64;
        let mut any = false;
        for bytes in [
            self.frame_upload_bytes(),
            self.frame_placement_bytes(),
            self.frame_embed_bytes(),
        ]
        .into_iter()
        .flatten()
        {
            total = total.saturating_add(bytes);
            any = true;
        }
        any.then_some(total)
    }

    /// Elapsed render/upload time in microseconds reported by a `pane_frame_presented` event.
    pub fn frame_elapsed_us(&self) -> Option<u64> {
        self.detail_u64("elapsed_us")
    }
}

/// Owning iterator over a bounded `EVENTS [ms]` batch.
#[derive(Clone, Debug)]
pub struct KittwmEventIter {
    inner: IntoIter<KittwmEvent>,
}

impl Iterator for KittwmEventIter {
    type Item = KittwmEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for KittwmEventIter {}

impl From<Vec<KittwmEvent>> for KittwmEventIter {
    fn from(events: Vec<KittwmEvent>) -> Self {
        Self {
            inner: events.into_iter(),
        }
    }
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
    /// Pane outer/app geometry changed.
    PaneResized(EventEnvelope),
    /// Socket-injected input was sent to a pane.
    PaneInputSent(EventEnvelope),
    /// A pane frame was presented/rendered without carrying pixel payloads.
    PaneFramePresented(EventEnvelope),
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
    /// Surface/window title changed.
    SurfaceTitleChanged(EventEnvelope),
    /// Surface emitted a bell.
    SurfaceBell(EventEnvelope),
    /// Surface requested clipboard contents to be set.
    SurfaceClipboardSet(EventEnvelope),
    /// Surface requested a notification.
    SurfaceNotification(EventEnvelope),
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

    /// Return the common event envelope for typed known events.
    pub fn envelope(&self) -> Option<&EventEnvelope> {
        match self {
            Self::Status(envelope)
            | Self::StatusChanged(envelope)
            | Self::PaneOpened(envelope)
            | Self::PaneClosed(envelope)
            | Self::PaneChanged(envelope)
            | Self::PaneResized(envelope)
            | Self::PaneInputSent(envelope)
            | Self::PaneFramePresented(envelope)
            | Self::FocusChanged(envelope)
            | Self::LayoutChanged(envelope)
            | Self::SemanticSnapshotReady(envelope)
            | Self::SemanticFocusChanged(envelope)
            | Self::SemanticActionInvoked(envelope)
            | Self::SurfaceTitleChanged(envelope)
            | Self::SurfaceBell(envelope)
            | Self::SurfaceClipboardSet(envelope)
            | Self::SurfaceNotification(envelope)
            | Self::SemanticValueChanged(envelope) => Some(envelope),
            Self::Unknown { .. } => None,
        }
    }

    /// Return the raw JSON object for unknown events.
    pub fn unknown_raw(&self) -> Option<&Value> {
        match self {
            Self::Unknown { raw, .. } => Some(raw),
            _ => None,
        }
    }

    /// Return this event's kind label.
    pub fn kind(&self) -> &str {
        match self {
            Self::Status(_) => "status",
            Self::StatusChanged(_) => "status_changed",
            Self::PaneOpened(_) => "pane_opened",
            Self::PaneClosed(_) => "pane_closed",
            Self::PaneChanged(_) => "pane_changed",
            Self::PaneResized(_) => "pane_resized",
            Self::PaneInputSent(_) => "pane_input_sent",
            Self::PaneFramePresented(_) => "pane_frame_presented",
            Self::FocusChanged(_) => "focus_changed",
            Self::LayoutChanged(_) => "layout_changed",
            Self::SemanticSnapshotReady(_) => "semantic_snapshot_ready",
            Self::SemanticFocusChanged(_) => "semantic_focus_changed",
            Self::SemanticActionInvoked(_) => "semantic_action_invoked",
            Self::SurfaceTitleChanged(_) => "surface_title_changed",
            Self::SurfaceBell(_) => "surface_bell",
            Self::SurfaceClipboardSet(_) => "surface_clipboard_set",
            Self::SurfaceNotification(_) => "surface_notification",
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
        "pane_resized" => KittwmEvent::PaneResized(envelope()),
        "pane_input_sent" => KittwmEvent::PaneInputSent(envelope()),
        "pane_frame_presented" => KittwmEvent::PaneFramePresented(envelope()),
        "focus_changed" => KittwmEvent::FocusChanged(envelope()),
        "layout_changed" => KittwmEvent::LayoutChanged(envelope()),
        "semantic_snapshot_ready" => KittwmEvent::SemanticSnapshotReady(envelope()),
        "semantic_focus_changed" => KittwmEvent::SemanticFocusChanged(envelope()),
        "semantic_action_invoked" => KittwmEvent::SemanticActionInvoked(envelope()),
        "surface_title_changed" => KittwmEvent::SurfaceTitleChanged(envelope()),
        "surface_bell" => KittwmEvent::SurfaceBell(envelope()),
        "surface_clipboard_set" => KittwmEvent::SurfaceClipboardSet(envelope()),
        "surface_notification" => KittwmEvent::SurfaceNotification(envelope()),
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

impl DirtyFrameStatus {
    /// Whether the frame was reported clean enough to avoid a fresh upload.
    pub fn is_clean(&self) -> bool {
        self.skipped_upload || self.changed_tiles == 0
    }

    /// Changed tile count and total tile count.
    pub fn changed_tiles_ratio(&self) -> (u32, u32) {
        (self.changed_tiles, self.total_tiles)
    }

    /// Changed tile count and total tile count as `<changed>/<total>`.
    pub fn changed_tiles_label(&self) -> String {
        format!("{}/{}", self.changed_tiles, self.total_tiles)
    }

    /// Live pane-chip frame status label: `clean` or `<changed>/<total>`.
    pub fn status_label(&self) -> String {
        if self.is_clean() {
            "clean".to_string()
        } else {
            self.changed_tiles_label()
        }
    }

    /// Full live pane-chip label, including the `frame:` prefix.
    pub fn chip_label(&self) -> String {
        format!("frame:{}", self.status_label())
    }

    /// Changed tile percentage in `[0, 100]`.
    pub fn changed_percent(&self) -> f32 {
        self.changed_fraction * 100.0
    }
}

/// Rich native pane detail returned by `PANES_JSON` / `STATUS_JSON`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NativePaneDetail {
    /// Window id.
    pub window: String,
    /// Human-readable title.
    pub title: String,
    /// Whether this pane is focused.
    pub focused: bool,
    /// Layout weight.
    pub weight: u16,
    /// Zero-based visual stack index; larger values are above smaller values.
    #[serde(default)]
    pub stack_index: Option<u16>,
    /// Whether this pane is currently topmost in the visual stack.
    #[serde(default)]
    pub stack_top: Option<bool>,
    /// Floating-pane x offset from the generated floating layout.
    #[serde(default)]
    pub floating_dx: Option<i16>,
    /// Floating-pane y offset from the generated floating layout.
    #[serde(default)]
    pub floating_dy: Option<i16>,
    /// Whether this pane has a non-zero floating offset.
    #[serde(default)]
    pub floating_moved: Option<bool>,
    /// Whether the pane title row is a window-manager drag handle.
    #[serde(default)]
    pub title_draggable: Option<bool>,
    /// Title drag interaction kind, for example `reorder` or `reposition`.
    #[serde(default)]
    pub title_drag_kind: Option<String>,
    /// Whether this pane is currently the active title-drag target.
    #[serde(default)]
    pub title_drag_active: Option<bool>,
    /// One-based host column suitable for starting a title-row drag.
    #[serde(default)]
    pub title_drag_col: Option<u16>,
    /// One-based host row suitable for starting a title-row drag.
    #[serde(default)]
    pub title_drag_row: Option<u16>,
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

/// Process information for one kittwm-owned pane.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KittwmProcessInfo {
    /// Pane/window id that owns this process, when known.
    pub window: String,
    /// Pane title.
    pub title: String,
    /// Whether this pane is focused.
    pub focused: bool,
    /// Process id, if reported by the session.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Parent process id, when available from the host.
    #[serde(default)]
    pub ppid: Option<u32>,
    /// Spawned pane command, when reported by the session.
    #[serde(default)]
    pub command: Option<String>,
    /// Host process state/stat string, when available.
    #[serde(default)]
    pub state: Option<String>,
    /// Resident set size in KiB, when available.
    #[serde(default)]
    pub rss_kib: Option<u64>,
    /// CPU percentage from the host process table, when available.
    #[serde(default)]
    pub cpu_percent: Option<f32>,
    /// Host command name, when available.
    #[serde(default)]
    pub process_name: Option<String>,
}

/// Process snapshot for the current kittwm session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KittwmProcessSnapshot {
    /// Socket path used for the snapshot.
    pub socket: PathBuf,
    /// Pane/process entries.
    pub processes: Vec<KittwmProcessInfo>,
}

impl KittwmProcessSnapshot {
    /// Number of process entries.
    pub fn len(&self) -> usize {
        self.processes.len()
    }

    /// Whether the snapshot contains no process entries.
    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }
}

impl NativePaneDetail {
    /// Outer pane bounds as `(x, y, cols, rows)` when geometry is available.
    pub fn bounds(&self) -> Option<(u16, u16, u16, u16)> {
        Some((self.x?, self.y?, self.cols?, self.rows?))
    }

    /// App/content bounds as `(x, y, cols, rows)` when geometry is available.
    pub fn app_bounds(&self) -> Option<(u16, u16, u16, u16)> {
        Some((self.app_x?, self.app_y?, self.app_cols?, self.app_rows?))
    }

    /// Zero-based visual stack index when reported.
    pub fn stack_index(&self) -> Option<u16> {
        self.stack_index
    }

    /// Whether this pane is reported as topmost in the current visual stack.
    pub fn is_stack_top(&self) -> bool {
        self.stack_top.unwrap_or(false)
    }

    /// Floating-pane offset from the generated layout when reported.
    pub fn floating_offset(&self) -> Option<(i16, i16)> {
        Some((self.floating_dx?, self.floating_dy?))
    }

    /// Whether this pane has a non-zero floating offset.
    pub fn has_floating_offset(&self) -> bool {
        self.floating_moved.unwrap_or_else(
            || matches!(self.floating_offset(), Some((dx, dy)) if dx != 0 || dy != 0),
        )
    }

    /// Whether this pane has a non-default live layout weight.
    pub fn has_non_default_weight(&self) -> bool {
        self.weight > 1
    }

    /// Live pane-chip weight label such as `wt:3`, when the pane is resized.
    pub fn weight_chip_label(&self) -> Option<String> {
        self.has_non_default_weight()
            .then(|| format!("wt:{}", self.weight))
    }

    /// Alias for [`NativePaneDetail::has_non_default_weight`].
    pub fn is_resized(&self) -> bool {
        self.has_non_default_weight()
    }

    /// Live pane-chip command label, bounded like the native status chip.
    pub fn command_chip_label(&self) -> String {
        bounded_ellipsis(
            self.command
                .as_deref()
                .filter(|command| !command.is_empty())
                .unwrap_or("-"),
            PANE_STATUS_COMMAND_MAX_CHARS,
        )
    }

    /// Live pane-chip pid label, matching `pid:<pid>` or `pid:-`.
    pub fn pid_chip_label(&self) -> String {
        self.pid
            .map(|pid| format!("pid:{pid}"))
            .unwrap_or_else(|| "pid:-".to_string())
    }

    /// Full live pane status chip text: command, optional weight, pid, and frame.
    pub fn status_chip_label(&self) -> String {
        let mut label = self.command_chip_label();
        if let Some(weight) = self.weight_chip_label() {
            label.push_str(" · ");
            label.push_str(&weight);
        }
        label.push_str(" · ");
        label.push_str(&self.pid_chip_label());
        label.push_str(" · ");
        label.push_str(&self.frame_chip_label());
        label
    }

    /// Whether the pane title row is reported as a window-manager drag handle.
    pub fn is_title_draggable(&self) -> bool {
        self.title_draggable.unwrap_or(false)
    }

    /// Whether the pane is reported as the active title-drag target.
    pub fn is_title_drag_active(&self) -> bool {
        self.title_drag_active.unwrap_or(false)
    }

    /// Reported raw title-drag interaction kind, such as `reorder` or `reposition`.
    pub fn title_drag_kind(&self) -> Option<&str> {
        self.title_drag_kind.as_deref()
    }

    /// Parsed title-drag interaction kind, preserving unknown future strings.
    pub fn parsed_title_drag_kind(&self) -> Option<TitleDragKind> {
        self.title_drag_kind().map(TitleDragKind::parse)
    }

    /// Whether the pane title drag handle reports a tiled reorder interaction.
    pub fn title_drag_reorders_pane(&self) -> bool {
        title_drag_kind_matches(self.title_drag_kind(), "reorder")
    }

    /// Whether the pane title drag handle reports a floating reposition interaction.
    pub fn title_drag_repositions_pane(&self) -> bool {
        title_drag_kind_matches(self.title_drag_kind(), "reposition")
    }

    /// Visible title-row marker prefix for this pane in the given layout label.
    ///
    /// The returned prefix mirrors kittwm's compact marker language: focus (`▶`),
    /// tiled reorder (`⇄`), resized (`↔`), fullscreen (`▣`), floating drag (`≡`),
    /// topmost (`▲`), and moved (`●`). Alignment spaces are preserved for the
    /// same marker slots used by live title rows.
    pub fn title_marker_prefix(&self, layout: Option<&str>) -> String {
        title_marker_prefix_for_pane(self, layout)
    }

    /// Visible title-row marker prefix, optionally including the active-drag marker (`◆`).
    pub fn title_marker_prefix_with_active_drag(
        &self,
        layout: Option<&str>,
        active_drag: bool,
    ) -> String {
        title_marker_prefix_for_pane_with_active_drag(self, layout, self.focused, active_drag)
    }

    /// Visible title-row marker prefix using reported active-drag metadata when present.
    pub fn reported_title_marker_prefix(&self, layout: Option<&str>) -> String {
        self.title_marker_prefix_with_active_drag(layout, self.is_title_drag_active())
    }

    /// Host-cell coordinate suitable for starting a title-row drag, when available.
    ///
    /// Returns one-based `(col, row)` coordinates for the title row. The column is
    /// clamped inside the pane and biased right of the focus/drag markers so SDK
    /// automations can begin a drag without hand-computing chrome geometry.
    pub fn title_drag_cell(&self) -> Option<(u16, u16)> {
        if !self.is_title_draggable() {
            return None;
        }
        if let (Some(col), Some(row)) = (self.title_drag_col, self.title_drag_row) {
            return Some((col, row));
        }
        let (x, y, cols, rows) = self.bounds()?;
        if cols == 0 || rows == 0 {
            return None;
        }
        let local_col = cols.saturating_sub(1).min(3);
        Some((
            x.saturating_add(local_col).saturating_add(1),
            y.saturating_add(1),
        ))
    }

    /// Host-cell start/end coordinates for dragging a title row by a delta.
    ///
    /// The returned cells are one-based `(col, row)` pairs suitable for UI
    /// automation paths that inject host-level pointer drags. End coordinates are
    /// saturated at the terminal-cell minimum/maximum instead of wrapping.
    pub fn title_drag_cells_by(
        &self,
        delta_cols: i32,
        delta_rows: i32,
    ) -> Option<((u16, u16), (u16, u16))> {
        let start = self.title_drag_cell()?;
        let end = (
            add_signed_host_cell_delta(start.0, delta_cols),
            add_signed_host_cell_delta(start.1, delta_rows),
        );
        Some((start, end))
    }

    /// Cursor position as `(col, row)` when reported.
    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        Some((self.cursor_col?, self.cursor_row?))
    }

    /// Whether the pane reports a visible cursor.
    pub fn is_cursor_visible(&self) -> bool {
        self.cursor_visible.unwrap_or(false)
    }

    /// Whether bracketed paste mode is enabled.
    pub fn has_bracketed_paste(&self) -> bool {
        self.bracketed_paste.unwrap_or(false)
    }

    /// Whether application cursor-key mode is enabled.
    pub fn has_application_cursor_keys(&self) -> bool {
        self.application_cursor_keys.unwrap_or(false)
    }

    /// Whether any mouse-reporting mode is enabled.
    pub fn has_mouse_reporting(&self) -> bool {
        self.mouse_reporting.unwrap_or(false)
            || self.mouse_button_motion.unwrap_or(false)
            || self.mouse_all_motion.unwrap_or(false)
    }

    /// Whether button-motion mouse reporting is enabled.
    pub fn has_mouse_button_motion(&self) -> bool {
        self.mouse_button_motion.unwrap_or(false)
    }

    /// Whether all-motion mouse reporting is enabled.
    pub fn has_mouse_all_motion(&self) -> bool {
        self.mouse_all_motion.unwrap_or(false)
    }

    /// Whether SGR mouse encoding is enabled.
    pub fn has_mouse_sgr(&self) -> bool {
        self.mouse_sgr.unwrap_or(false)
    }

    /// Whether dirty-frame metrics are present.
    pub fn has_dirty_frame(&self) -> bool {
        self.dirty_frame.is_some()
    }

    /// Live pane-chip frame status label: `new`, `clean`, or `<changed>/<total>`.
    pub fn frame_status_label(&self) -> String {
        self.dirty_frame
            .as_ref()
            .map(DirtyFrameStatus::status_label)
            .unwrap_or_else(|| "new".to_string())
    }

    /// Full live pane-chip label, including the `frame:` prefix.
    pub fn frame_chip_label(&self) -> String {
        format!("frame:{}", self.frame_status_label())
    }

    /// Dirty-frame changed tile count and total tile count, when reported.
    pub fn frame_changed_tiles_ratio(&self) -> Option<(u32, u32)> {
        self.dirty_frame
            .as_ref()
            .map(DirtyFrameStatus::changed_tiles_ratio)
    }

    /// Dirty-frame changed tile count and total tile count as `<changed>/<total>`.
    pub fn frame_changed_tiles_label(&self) -> Option<String> {
        self.dirty_frame
            .as_ref()
            .map(DirtyFrameStatus::changed_tiles_label)
    }

    /// Dirty-frame changed tile fraction, when reported.
    pub fn frame_changed_fraction(&self) -> Option<f32> {
        self.dirty_frame
            .as_ref()
            .map(|metrics| metrics.changed_fraction)
    }

    /// Dirty-frame changed tile percentage in `[0, 100]`, when reported.
    pub fn frame_changed_percent(&self) -> Option<f32> {
        self.dirty_frame
            .as_ref()
            .map(DirtyFrameStatus::changed_percent)
    }

    /// Whether the most recent frame upload was skipped because it was clean, when reported.
    pub fn frame_upload_skipped(&self) -> Option<bool> {
        self.dirty_frame
            .as_ref()
            .map(|metrics| metrics.skipped_upload)
    }

    /// Whether the reported frame is clean, when dirty-frame metrics are present.
    pub fn is_frame_clean(&self) -> Option<bool> {
        self.dirty_frame.as_ref().map(DirtyFrameStatus::is_clean)
    }

    /// Whether transport diagnostics are present.
    pub fn has_transport_diagnostics(&self) -> bool {
        self.transport.is_some()
    }
}

/// Chrome/workspace reservation metadata returned by `STATUS_JSON` / `PANES_JSON`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChromeReservationStatus {
    /// Workspace identifier displayed in the top bar.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Rows reserved for top-bar chrome.
    #[serde(default)]
    pub top_bar_rows: Option<u16>,
    /// Rows reserved for bottom/status-bar chrome.
    #[serde(default)]
    pub bottom_bar_rows: Option<u16>,
    /// Columns reserved on the left edge for dock/sidebar chrome.
    #[serde(default)]
    pub left_cols: Option<u16>,
    /// Columns reserved on the right edge for dock/sidebar chrome.
    #[serde(default)]
    pub right_cols: Option<u16>,
    /// Horizontal gap between tiled app surfaces.
    #[serde(default)]
    pub gap_cols: Option<u16>,
    /// Vertical gap between tiled app surfaces.
    #[serde(default)]
    pub gap_rows: Option<u16>,
    /// Optional window/app token that currently owns the reservation request.
    #[serde(default)]
    pub owner: Option<String>,
    /// Rows available for tiled pane content after chrome reservation.
    #[serde(default)]
    pub tilable_rows: Option<u16>,
}

/// Request body for `RESERVE_CHROME_JSON`, used by bar/dock-style apps that
/// need kittwm to keep normal tiled applications out of their drawable area.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChromeReservationRequest {
    /// Rows reserved for top-bar chrome.
    #[serde(default)]
    pub top_bar_rows: u16,
    /// Rows reserved for bottom/status-bar chrome.
    #[serde(default)]
    pub bottom_bar_rows: u16,
    /// Columns reserved on the left edge.
    #[serde(default)]
    pub left_cols: u16,
    /// Columns reserved on the right edge.
    #[serde(default)]
    pub right_cols: u16,
    /// Horizontal gap between tiled app surfaces.
    #[serde(default)]
    pub gap_cols: u16,
    /// Vertical gap between tiled app surfaces.
    #[serde(default)]
    pub gap_rows: u16,
    /// Optional owner/window token.
    #[serde(default)]
    pub owner: Option<String>,
}

impl ChromeReservationStatus {
    /// Whether any chrome reservation field was reported.
    pub fn is_reported(&self) -> bool {
        self.workspace.is_some()
            || self.top_bar_rows.is_some()
            || self.bottom_bar_rows.is_some()
            || self.left_cols.is_some()
            || self.right_cols.is_some()
            || self.gap_cols.is_some()
            || self.gap_rows.is_some()
            || self.owner.is_some()
            || self.tilable_rows.is_some()
    }

    /// Top-bar rows, defaulting to zero when older daemons omit the field.
    pub fn top_bar_rows_or_zero(&self) -> u16 {
        self.top_bar_rows.unwrap_or(0)
    }

    /// Tilable rows, if reported by the daemon.
    pub fn tilable_rows(&self) -> Option<u16> {
        self.tilable_rows
    }

    /// Bottom/status rows, defaulting to zero when older daemons omit the field.
    pub fn bottom_bar_rows_or_zero(&self) -> u16 {
        self.bottom_bar_rows.unwrap_or(0)
    }

    /// Horizontal tile gap columns, defaulting to zero for older daemons.
    pub fn gap_cols_or_zero(&self) -> u16 {
        self.gap_cols.unwrap_or(0)
    }

    /// Vertical tile gap rows, defaulting to zero for older daemons.
    pub fn gap_rows_or_zero(&self) -> u16 {
        self.gap_rows.unwrap_or(0)
    }
}

fn normalized_optional_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl ChromeReservationRequest {
    /// Reserve only a top bar with the given row count.
    pub fn top_bar(rows: u16) -> Self {
        Self {
            top_bar_rows: rows,
            ..Self::default()
        }
    }

    /// Attach an owner/window token to the request.
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = normalized_optional_string(&owner.into());
        self
    }

    /// Set inter-tile gaps.
    pub fn gaps(mut self, cols: u16, rows: u16) -> Self {
        self.gap_cols = cols;
        self.gap_rows = rows;
        self
    }
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
    /// Workspace identifier, when reported by native kittwm.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Chrome reservation metadata, when reported by native kittwm.
    #[serde(default)]
    pub chrome: Option<ChromeReservationStatus>,
    /// Detailed panes.
    #[serde(default)]
    pub panes_detail: Vec<NativePaneDetail>,
}

impl PanesStatus {
    /// Workspace id, preferring the top-level field and falling back to chrome metadata.
    pub fn workspace_id(&self) -> Option<&str> {
        normalized_workspace_str(self.workspace.as_deref()).or_else(|| {
            self.chrome
                .as_ref()
                .and_then(|chrome| normalized_workspace_str(chrome.workspace.as_deref()))
        })
    }

    /// Layout label, trimmed and empty-filtered.
    pub fn layout_label(&self) -> Option<&str> {
        normalized_layout_str(Some(self.layout.as_str()))
    }

    /// Whether the reported layout is floating mode.
    pub fn is_floating_layout(&self) -> bool {
        layout_label_matches(self.layout_label(), "floating")
    }

    /// Whether the reported layout is fullscreen mode.
    pub fn is_fullscreen_layout(&self) -> bool {
        layout_label_matches(self.layout_label(), "fullscreen")
    }

    /// Whether the reported layout is a tiled axis/grid mode.
    pub fn is_tiled_layout(&self) -> bool {
        layout_label_is_tiled(self.layout_label())
    }

    /// Chrome reservation metadata, if present.
    pub fn chrome_reservation(&self) -> Option<&ChromeReservationStatus> {
        self.chrome.as_ref()
    }

    /// Reserved top-bar rows, defaulting to zero for older daemons.
    pub fn top_bar_rows(&self) -> u16 {
        self.chrome
            .as_ref()
            .and_then(|chrome| chrome.top_bar_rows)
            .unwrap_or(0)
    }

    /// Tilable rows after chrome reservation, if reported.
    pub fn tilable_rows(&self) -> Option<u16> {
        self.chrome.as_ref().and_then(|chrome| chrome.tilable_rows)
    }

    /// Focused pane detail, preferring explicit pane flags and falling back to the focus id.
    pub fn focused_pane(&self) -> Option<&NativePaneDetail> {
        focused_pane_from_details(&self.panes_detail, Some(self.focus.as_str()))
    }

    /// Focused pane detail when the current layout is fullscreen.
    pub fn fullscreen_pane(&self) -> Option<&NativePaneDetail> {
        self.is_fullscreen_layout()
            .then(|| self.focused_pane())
            .flatten()
    }

    /// Whether the focused pane is currently shown in fullscreen layout.
    pub fn focused_is_fullscreen(&self) -> Option<bool> {
        self.focused_pane().map(|_| self.is_fullscreen_layout())
    }

    /// Topmost pane detail, using `stack_top` when reported or the largest stack index.
    pub fn topmost_pane(&self) -> Option<&NativePaneDetail> {
        topmost_pane_from_details(&self.panes_detail)
    }

    /// Whether the focused pane is also the topmost pane.
    pub fn focused_is_topmost(&self) -> Option<bool> {
        focused_is_topmost_from_details(self.focused_pane(), self.topmost_pane())
    }

    /// Whether the focused pane has a non-default layout weight.
    pub fn focused_is_resized(&self) -> Option<bool> {
        self.focused_pane().map(NativePaneDetail::is_resized)
    }

    /// Live pane-chip weight label for the focused pane, such as `wt:3`.
    pub fn focused_weight_chip_label(&self) -> Option<String> {
        self.focused_pane()?.weight_chip_label()
    }

    /// Live pane-chip command label for the focused pane, bounded like the native status chip.
    pub fn focused_command_chip_label(&self) -> Option<String> {
        self.focused_pane()
            .map(NativePaneDetail::command_chip_label)
    }

    /// Live pane-chip pid label for the focused pane, matching `pid:<pid>` or `pid:-`.
    pub fn focused_pid_chip_label(&self) -> Option<String> {
        self.focused_pane().map(NativePaneDetail::pid_chip_label)
    }

    /// Full live pane status chip text for the focused pane.
    pub fn focused_status_chip_label(&self) -> Option<String> {
        self.focused_pane().map(NativePaneDetail::status_chip_label)
    }

    /// Whether the focused pane has a non-zero floating offset.
    pub fn focused_is_moved(&self) -> Option<bool> {
        self.focused_pane()
            .map(NativePaneDetail::has_floating_offset)
    }

    /// Whether the focused pane title row is reported as a window-manager drag handle.
    pub fn focused_is_title_draggable(&self) -> Option<bool> {
        self.focused_pane()
            .map(NativePaneDetail::is_title_draggable)
    }

    /// Whether the focused pane is reported as the active title-drag target.
    pub fn focused_is_title_drag_active(&self) -> Option<bool> {
        self.focused_pane()
            .map(NativePaneDetail::is_title_drag_active)
    }

    /// Whether dragging the focused pane title row reorders tiled panes.
    pub fn focused_title_drag_reorders_pane(&self) -> Option<bool> {
        self.focused_pane().map(|pane| {
            pane.title_drag_reorders_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_tiled_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Whether dragging the focused pane title row repositions a floating pane.
    pub fn focused_title_drag_repositions_pane(&self) -> Option<bool> {
        self.focused_pane().map(|pane| {
            pane.title_drag_repositions_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_floating_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Host-cell coordinate suitable for dragging the focused pane title row.
    pub fn focused_title_drag_cell(&self) -> Option<(u16, u16)> {
        self.focused_pane()?.title_drag_cell()
    }

    /// Start/end host-cell coordinates for dragging the focused pane title row by a delta.
    pub fn focused_title_drag_cells_by(
        &self,
        delta_cols: i32,
        delta_rows: i32,
    ) -> Option<((u16, u16), (u16, u16))> {
        self.focused_pane()?
            .title_drag_cells_by(delta_cols, delta_rows)
    }

    /// Visible title-row marker prefix for the focused pane.
    pub fn focused_title_marker_prefix(&self) -> Option<String> {
        self.focused_pane()
            .map(|pane| title_marker_prefix_for_pane_with_focus(pane, self.layout_label(), true))
    }

    /// Visible title-row marker prefix for the focused pane, optionally including `◆`.
    pub fn focused_title_marker_prefix_with_active_drag(
        &self,
        active_drag: bool,
    ) -> Option<String> {
        self.focused_pane().map(|pane| {
            title_marker_prefix_for_pane_with_active_drag(
                pane,
                self.layout_label(),
                true,
                active_drag,
            )
        })
    }

    /// Visible title-row marker prefix for the focused pane using reported active-drag metadata.
    pub fn focused_reported_title_marker_prefix(&self) -> Option<String> {
        self.focused_pane().map(|pane| {
            title_marker_prefix_for_pane_with_active_drag(
                pane,
                self.layout_label(),
                true,
                pane.is_title_drag_active(),
            )
        })
    }

    /// Whether the focused pane's latest dirty-frame metrics report a clean/skipped frame.
    pub fn focused_frame_is_clean(&self) -> Option<bool> {
        self.focused_pane()?.is_frame_clean()
    }

    /// Whether the focused pane's latest frame upload was skipped, when reported.
    pub fn focused_frame_upload_skipped(&self) -> Option<bool> {
        self.focused_pane()?.frame_upload_skipped()
    }

    /// Live pane-chip frame status label for the focused pane: `new`, `clean`, or `<changed>/<total>`.
    pub fn focused_frame_status_label(&self) -> Option<String> {
        self.focused_pane()
            .map(NativePaneDetail::frame_status_label)
    }

    /// Full live pane-chip label for the focused pane, including the `frame:` prefix.
    pub fn focused_frame_chip_label(&self) -> Option<String> {
        self.focused_pane().map(NativePaneDetail::frame_chip_label)
    }

    /// Dirty-frame changed tile count and total tile count for the focused pane.
    pub fn focused_frame_changed_tiles_ratio(&self) -> Option<(u32, u32)> {
        self.focused_pane()?.frame_changed_tiles_ratio()
    }

    /// Dirty-frame changed tile count and total tile count as `<changed>/<total>`.
    pub fn focused_frame_changed_tiles_label(&self) -> Option<String> {
        self.focused_pane()?.frame_changed_tiles_label()
    }

    /// Dirty-frame changed tile fraction for the focused pane.
    pub fn focused_frame_changed_fraction(&self) -> Option<f32> {
        self.focused_pane()?.frame_changed_fraction()
    }

    /// Dirty-frame changed tile percentage in `[0, 100]` for the focused pane.
    pub fn focused_frame_changed_percent(&self) -> Option<f32> {
        self.focused_pane()?.frame_changed_percent()
    }

    /// Panes whose title row is reported as a window-manager drag handle.
    pub fn title_draggable_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_title_draggable())
    }

    /// Panes reported as active title-drag targets.
    pub fn active_title_drag_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_title_drag_active())
    }

    /// First pane reported as the active title-drag target.
    pub fn active_title_drag_pane(&self) -> Option<&NativePaneDetail> {
        self.active_title_drag_panes().next()
    }

    /// Window id for the first reported active title-drag target.
    pub fn active_title_drag_window(&self) -> Option<&str> {
        Some(self.active_title_drag_pane()?.window.as_str())
    }

    /// Reported raw title-drag interaction kind for the active drag target.
    pub fn active_title_drag_kind(&self) -> Option<&str> {
        self.active_title_drag_pane()?.title_drag_kind()
    }

    /// Parsed title-drag interaction kind for the active drag target.
    pub fn active_parsed_title_drag_kind(&self) -> Option<TitleDragKind> {
        self.active_title_drag_pane()?.parsed_title_drag_kind()
    }

    /// Whether the active title-drag target reorders tiled panes.
    pub fn active_title_drag_reorders_pane(&self) -> Option<bool> {
        self.active_title_drag_pane().map(|pane| {
            pane.title_drag_reorders_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_tiled_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Whether the active title-drag target repositions a floating pane.
    pub fn active_title_drag_repositions_pane(&self) -> Option<bool> {
        self.active_title_drag_pane().map(|pane| {
            pane.title_drag_repositions_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_floating_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Host-cell coordinate suitable for dragging the active title-drag target.
    pub fn active_title_drag_cell(&self) -> Option<(u16, u16)> {
        self.active_title_drag_pane()?.title_drag_cell()
    }

    /// Start/end host-cell coordinates for dragging the active title-drag target by a delta.
    pub fn active_title_drag_cells_by(
        &self,
        delta_cols: i32,
        delta_rows: i32,
    ) -> Option<((u16, u16), (u16, u16))> {
        self.active_title_drag_pane()?
            .title_drag_cells_by(delta_cols, delta_rows)
    }

    /// Panes whose title drag handle reorders the tiled pane stack.
    pub fn title_reorder_draggable_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        let is_tiled = self.is_tiled_layout();
        self.panes_detail.iter().filter(move |pane| {
            pane.title_drag_reorders_pane()
                || (pane.title_drag_kind().is_none() && is_tiled && pane.is_title_draggable())
        })
    }

    /// Panes whose title drag handle repositions a floating pane.
    pub fn title_reposition_draggable_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        let is_floating = self.is_floating_layout();
        self.panes_detail.iter().filter(move |pane| {
            pane.title_drag_repositions_pane()
                || (pane.title_drag_kind().is_none() && is_floating && pane.is_title_draggable())
        })
    }

    /// Panes with non-zero floating offsets.
    pub fn moved_floating_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.has_floating_offset())
    }

    /// Panes with non-default layout weights.
    pub fn resized_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.has_non_default_weight())
    }

    /// Panes with dirty-frame metrics whose latest frame was clean/skipped.
    pub fn clean_frame_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_frame_clean() == Some(true))
    }

    /// Panes with dirty-frame metrics whose latest frame changed tiles.
    pub fn dirty_frame_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_frame_clean() == Some(false))
    }
}

fn add_signed_host_cell_delta(value: u16, delta: i32) -> u16 {
    (i64::from(value) + i64::from(delta)).clamp(1, i64::from(u16::MAX)) as u16
}

fn normalized_workspace_str(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalized_layout_str(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn layout_label_matches(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn title_drag_kind_matches(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn layout_label_is_tiled(value: Option<&str>) -> bool {
    value.is_some_and(|label| {
        label.eq_ignore_ascii_case("columns")
            || label.eq_ignore_ascii_case("rows")
            || label.eq_ignore_ascii_case("grid")
    })
}

const FOCUSED_TITLE_MARKER: &str = "▶";
const UNFOCUSED_TITLE_MARKER: &str = " ";
const ACTIVE_TITLE_DRAG_MARKER: &str = "◆";
const FLOATING_TITLE_DRAG_MARKER: &str = "≡";
const FLOATING_TITLE_TOP_MARKER: &str = "▲";
const FLOATING_TITLE_NOT_TOP_MARKER: &str = " ";
const FLOATING_TITLE_MOVED_MARKER: &str = "●";
const FLOATING_TITLE_NOT_MOVED_MARKER: &str = " ";
const TILED_TITLE_REORDER_MARKER: &str = "⇄";
const RESIZED_TILED_TITLE_MARKER: &str = "↔";
const FULLSCREEN_TITLE_MARKER: &str = "▣";

fn title_marker_prefix_for_pane(pane: &NativePaneDetail, layout: Option<&str>) -> String {
    title_marker_prefix_for_pane_with_focus(pane, layout, pane.focused)
}

fn title_marker_prefix_for_pane_with_focus(
    pane: &NativePaneDetail,
    layout: Option<&str>,
    focused: bool,
) -> String {
    title_marker_prefix_for_pane_with_active_drag(pane, layout, focused, false)
}

fn title_marker_prefix_for_pane_with_active_drag(
    pane: &NativePaneDetail,
    layout: Option<&str>,
    focused: bool,
    active_drag: bool,
) -> String {
    let is_tiled = layout_label_is_tiled(layout);
    let is_floating = layout_label_matches(layout, "floating");
    let is_fullscreen = layout_label_matches(layout, "fullscreen");
    let drag_kind_missing = pane.title_drag_kind().is_none();
    let reorder = pane.title_drag_reorders_pane()
        || (drag_kind_missing && is_tiled && pane.is_title_draggable());
    let reposition = pane.title_drag_repositions_pane()
        || (drag_kind_missing && is_floating && pane.is_title_draggable());
    let mut markers = String::with_capacity(8);
    markers.push_str(if focused {
        FOCUSED_TITLE_MARKER
    } else {
        UNFOCUSED_TITLE_MARKER
    });
    if active_drag {
        markers.push_str(ACTIVE_TITLE_DRAG_MARKER);
    }
    if reorder {
        markers.push_str(TILED_TITLE_REORDER_MARKER);
    }
    if is_fullscreen {
        markers.push_str(FULLSCREEN_TITLE_MARKER);
    }
    if reorder && pane.has_non_default_weight() {
        markers.push_str(RESIZED_TILED_TITLE_MARKER);
    }
    if reposition {
        markers.push_str(FLOATING_TITLE_DRAG_MARKER);
        markers.push_str(if pane.is_stack_top() {
            FLOATING_TITLE_TOP_MARKER
        } else {
            FLOATING_TITLE_NOT_TOP_MARKER
        });
        markers.push_str(if pane.has_floating_offset() {
            FLOATING_TITLE_MOVED_MARKER
        } else {
            FLOATING_TITLE_NOT_MOVED_MARKER
        });
    }
    markers
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
    /// Workspace identifier, when reported by native kittwm.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Chrome reservation metadata, when reported by native kittwm.
    #[serde(default)]
    pub chrome: Option<ChromeReservationStatus>,
    /// Focused pane detail when available.
    #[serde(default)]
    pub focused_pane: Option<NativePaneDetail>,
    /// Pane details when available.
    #[serde(default)]
    pub panes_detail: Vec<NativePaneDetail>,
}

impl Status {
    /// Workspace id, preferring the top-level field and falling back to chrome metadata.
    pub fn workspace_id(&self) -> Option<&str> {
        normalized_workspace_str(self.workspace.as_deref()).or_else(|| {
            self.chrome
                .as_ref()
                .and_then(|chrome| normalized_workspace_str(chrome.workspace.as_deref()))
        })
    }

    /// Layout label, trimmed and empty-filtered.
    pub fn layout_label(&self) -> Option<&str> {
        normalized_layout_str(self.layout.as_deref())
    }

    /// Whether the reported layout is floating mode.
    pub fn is_floating_layout(&self) -> bool {
        layout_label_matches(self.layout_label(), "floating")
    }

    /// Whether the reported layout is fullscreen mode.
    pub fn is_fullscreen_layout(&self) -> bool {
        layout_label_matches(self.layout_label(), "fullscreen")
    }

    /// Whether the reported layout is a tiled axis/grid mode.
    pub fn is_tiled_layout(&self) -> bool {
        layout_label_is_tiled(self.layout_label())
    }

    /// Chrome reservation metadata, if present.
    pub fn chrome_reservation(&self) -> Option<&ChromeReservationStatus> {
        self.chrome.as_ref()
    }

    /// Reserved top-bar rows, defaulting to zero for older daemons.
    pub fn top_bar_rows(&self) -> u16 {
        self.chrome
            .as_ref()
            .and_then(|chrome| chrome.top_bar_rows)
            .unwrap_or(0)
    }

    /// Tilable rows after chrome reservation, if reported.
    pub fn tilable_rows(&self) -> Option<u16> {
        self.chrome.as_ref().and_then(|chrome| chrome.tilable_rows)
    }

    /// Focused pane detail, preferring the top-level `focused_pane` field.
    pub fn focused_pane(&self) -> Option<&NativePaneDetail> {
        self.focused_pane
            .as_ref()
            .or_else(|| focused_pane_from_details(&self.panes_detail, self.focus.as_deref()))
    }

    /// Focused pane detail when the current layout is fullscreen.
    pub fn fullscreen_pane(&self) -> Option<&NativePaneDetail> {
        self.is_fullscreen_layout()
            .then(|| self.focused_pane())
            .flatten()
    }

    /// Whether the focused pane is currently shown in fullscreen layout.
    pub fn focused_is_fullscreen(&self) -> Option<bool> {
        self.focused_pane().map(|_| self.is_fullscreen_layout())
    }

    /// Topmost pane detail, using `stack_top` when reported or the largest stack index.
    pub fn topmost_pane(&self) -> Option<&NativePaneDetail> {
        topmost_pane_from_details(&self.panes_detail)
    }

    /// Whether the focused pane is also the topmost pane.
    pub fn focused_is_topmost(&self) -> Option<bool> {
        focused_is_topmost_from_details(self.focused_pane(), self.topmost_pane())
    }

    /// Whether the focused pane has a non-default layout weight.
    pub fn focused_is_resized(&self) -> Option<bool> {
        self.focused_pane().map(NativePaneDetail::is_resized)
    }

    /// Live pane-chip weight label for the focused pane, such as `wt:3`.
    pub fn focused_weight_chip_label(&self) -> Option<String> {
        self.focused_pane()?.weight_chip_label()
    }

    /// Live pane-chip command label for the focused pane, bounded like the native status chip.
    pub fn focused_command_chip_label(&self) -> Option<String> {
        self.focused_pane()
            .map(NativePaneDetail::command_chip_label)
    }

    /// Live pane-chip pid label for the focused pane, matching `pid:<pid>` or `pid:-`.
    pub fn focused_pid_chip_label(&self) -> Option<String> {
        self.focused_pane().map(NativePaneDetail::pid_chip_label)
    }

    /// Full live pane status chip text for the focused pane.
    pub fn focused_status_chip_label(&self) -> Option<String> {
        self.focused_pane().map(NativePaneDetail::status_chip_label)
    }

    /// Whether the focused pane has a non-zero floating offset.
    pub fn focused_is_moved(&self) -> Option<bool> {
        self.focused_pane()
            .map(NativePaneDetail::has_floating_offset)
    }

    /// Whether the focused pane title row is reported as a window-manager drag handle.
    pub fn focused_is_title_draggable(&self) -> Option<bool> {
        self.focused_pane()
            .map(NativePaneDetail::is_title_draggable)
    }

    /// Whether the focused pane is reported as the active title-drag target.
    pub fn focused_is_title_drag_active(&self) -> Option<bool> {
        self.focused_pane()
            .map(NativePaneDetail::is_title_drag_active)
    }

    /// Whether dragging the focused pane title row reorders tiled panes.
    pub fn focused_title_drag_reorders_pane(&self) -> Option<bool> {
        self.focused_pane().map(|pane| {
            pane.title_drag_reorders_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_tiled_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Whether dragging the focused pane title row repositions a floating pane.
    pub fn focused_title_drag_repositions_pane(&self) -> Option<bool> {
        self.focused_pane().map(|pane| {
            pane.title_drag_repositions_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_floating_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Host-cell coordinate suitable for dragging the focused pane title row.
    pub fn focused_title_drag_cell(&self) -> Option<(u16, u16)> {
        self.focused_pane()?.title_drag_cell()
    }

    /// Start/end host-cell coordinates for dragging the focused pane title row by a delta.
    pub fn focused_title_drag_cells_by(
        &self,
        delta_cols: i32,
        delta_rows: i32,
    ) -> Option<((u16, u16), (u16, u16))> {
        self.focused_pane()?
            .title_drag_cells_by(delta_cols, delta_rows)
    }

    /// Visible title-row marker prefix for the focused pane.
    pub fn focused_title_marker_prefix(&self) -> Option<String> {
        self.focused_pane()
            .map(|pane| title_marker_prefix_for_pane_with_focus(pane, self.layout_label(), true))
    }

    /// Visible title-row marker prefix for the focused pane, optionally including `◆`.
    pub fn focused_title_marker_prefix_with_active_drag(
        &self,
        active_drag: bool,
    ) -> Option<String> {
        self.focused_pane().map(|pane| {
            title_marker_prefix_for_pane_with_active_drag(
                pane,
                self.layout_label(),
                true,
                active_drag,
            )
        })
    }

    /// Visible title-row marker prefix for the focused pane using reported active-drag metadata.
    pub fn focused_reported_title_marker_prefix(&self) -> Option<String> {
        self.focused_pane().map(|pane| {
            title_marker_prefix_for_pane_with_active_drag(
                pane,
                self.layout_label(),
                true,
                pane.is_title_drag_active(),
            )
        })
    }

    /// Whether the focused pane's latest dirty-frame metrics report a clean/skipped frame.
    pub fn focused_frame_is_clean(&self) -> Option<bool> {
        self.focused_pane()?.is_frame_clean()
    }

    /// Whether the focused pane's latest frame upload was skipped, when reported.
    pub fn focused_frame_upload_skipped(&self) -> Option<bool> {
        self.focused_pane()?.frame_upload_skipped()
    }

    /// Live pane-chip frame status label for the focused pane: `new`, `clean`, or `<changed>/<total>`.
    pub fn focused_frame_status_label(&self) -> Option<String> {
        self.focused_pane()
            .map(NativePaneDetail::frame_status_label)
    }

    /// Full live pane-chip label for the focused pane, including the `frame:` prefix.
    pub fn focused_frame_chip_label(&self) -> Option<String> {
        self.focused_pane().map(NativePaneDetail::frame_chip_label)
    }

    /// Dirty-frame changed tile count and total tile count for the focused pane.
    pub fn focused_frame_changed_tiles_ratio(&self) -> Option<(u32, u32)> {
        self.focused_pane()?.frame_changed_tiles_ratio()
    }

    /// Dirty-frame changed tile count and total tile count as `<changed>/<total>`.
    pub fn focused_frame_changed_tiles_label(&self) -> Option<String> {
        self.focused_pane()?.frame_changed_tiles_label()
    }

    /// Dirty-frame changed tile fraction for the focused pane.
    pub fn focused_frame_changed_fraction(&self) -> Option<f32> {
        self.focused_pane()?.frame_changed_fraction()
    }

    /// Dirty-frame changed tile percentage in `[0, 100]` for the focused pane.
    pub fn focused_frame_changed_percent(&self) -> Option<f32> {
        self.focused_pane()?.frame_changed_percent()
    }

    /// Panes whose title row is reported as a window-manager drag handle.
    pub fn title_draggable_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_title_draggable())
    }

    /// Panes reported as active title-drag targets.
    pub fn active_title_drag_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_title_drag_active())
    }

    /// First pane reported as the active title-drag target.
    pub fn active_title_drag_pane(&self) -> Option<&NativePaneDetail> {
        self.active_title_drag_panes().next()
    }

    /// Window id for the first reported active title-drag target.
    pub fn active_title_drag_window(&self) -> Option<&str> {
        Some(self.active_title_drag_pane()?.window.as_str())
    }

    /// Reported raw title-drag interaction kind for the active drag target.
    pub fn active_title_drag_kind(&self) -> Option<&str> {
        self.active_title_drag_pane()?.title_drag_kind()
    }

    /// Parsed title-drag interaction kind for the active drag target.
    pub fn active_parsed_title_drag_kind(&self) -> Option<TitleDragKind> {
        self.active_title_drag_pane()?.parsed_title_drag_kind()
    }

    /// Whether the active title-drag target reorders tiled panes.
    pub fn active_title_drag_reorders_pane(&self) -> Option<bool> {
        self.active_title_drag_pane().map(|pane| {
            pane.title_drag_reorders_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_tiled_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Whether the active title-drag target repositions a floating pane.
    pub fn active_title_drag_repositions_pane(&self) -> Option<bool> {
        self.active_title_drag_pane().map(|pane| {
            pane.title_drag_repositions_pane()
                || (pane.title_drag_kind().is_none()
                    && self.is_floating_layout()
                    && pane.is_title_draggable())
        })
    }

    /// Host-cell coordinate suitable for dragging the active title-drag target.
    pub fn active_title_drag_cell(&self) -> Option<(u16, u16)> {
        self.active_title_drag_pane()?.title_drag_cell()
    }

    /// Start/end host-cell coordinates for dragging the active title-drag target by a delta.
    pub fn active_title_drag_cells_by(
        &self,
        delta_cols: i32,
        delta_rows: i32,
    ) -> Option<((u16, u16), (u16, u16))> {
        self.active_title_drag_pane()?
            .title_drag_cells_by(delta_cols, delta_rows)
    }

    /// Panes whose title drag handle reorders the tiled pane stack.
    pub fn title_reorder_draggable_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        let is_tiled = self.is_tiled_layout();
        self.panes_detail.iter().filter(move |pane| {
            pane.title_drag_reorders_pane()
                || (pane.title_drag_kind().is_none() && is_tiled && pane.is_title_draggable())
        })
    }

    /// Panes whose title drag handle repositions a floating pane.
    pub fn title_reposition_draggable_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        let is_floating = self.is_floating_layout();
        self.panes_detail.iter().filter(move |pane| {
            pane.title_drag_repositions_pane()
                || (pane.title_drag_kind().is_none() && is_floating && pane.is_title_draggable())
        })
    }

    /// Panes with non-zero floating offsets.
    pub fn moved_floating_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.has_floating_offset())
    }

    /// Panes with non-default layout weights.
    pub fn resized_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.has_non_default_weight())
    }

    /// Panes with dirty-frame metrics whose latest frame was clean/skipped.
    pub fn clean_frame_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_frame_clean() == Some(true))
    }

    /// Panes with dirty-frame metrics whose latest frame changed tiles.
    pub fn dirty_frame_panes(&self) -> impl Iterator<Item = &NativePaneDetail> {
        self.panes_detail
            .iter()
            .filter(|pane| pane.is_frame_clean() == Some(false))
    }
}

fn focused_pane_from_details<'a>(
    panes: &'a [NativePaneDetail],
    focus: Option<&str>,
) -> Option<&'a NativePaneDetail> {
    panes
        .iter()
        .find(|pane| pane.focused)
        .or_else(|| focus.and_then(|focus| panes.iter().find(|pane| pane.window == focus)))
}

fn topmost_pane_from_details(panes: &[NativePaneDetail]) -> Option<&NativePaneDetail> {
    panes.iter().find(|pane| pane.is_stack_top()).or_else(|| {
        panes
            .iter()
            .filter_map(|pane| pane.stack_index.map(|idx| (idx, pane)))
            .max_by_key(|(idx, _)| *idx)
            .map(|(_, pane)| pane)
    })
}

fn focused_is_topmost_from_details(
    focused: Option<&NativePaneDetail>,
    topmost: Option<&NativePaneDetail>,
) -> Option<bool> {
    Some(focused?.window == topmost?.window)
}

const PANE_STATUS_COMMAND_MAX_CHARS: usize = 64;

fn bounded_ellipsis(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let mut out = String::with_capacity(max_chars.min(text.len()));
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return out;
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
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

/// Typed cached clipboard policy/read response returned by `CLIPBOARD_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardStatus {
    /// Whether the daemon policy allowed payload disclosure for this request.
    pub allowed: bool,
    /// Whether a cached OSC52 clipboard write is available.
    #[serde(default)]
    pub available: bool,
    /// Source/policy message when denied or otherwise unavailable.
    #[serde(default)]
    pub policy: Option<String>,
    /// Source window that produced the cached OSC52 write, when available.
    #[serde(default)]
    pub source_window: Option<String>,
    /// Clipboard selection name, e.g. `c`/`clipboard`, when available.
    #[serde(default)]
    pub selection: Option<String>,
    /// Base64 payload from the cached OSC52 write. Present only when allowed.
    #[serde(default)]
    pub payload_base64: Option<String>,
    /// Decoded payload byte length reported by the daemon.
    #[serde(default)]
    pub payload_bytes: Option<usize>,
    /// Daemon timestamp for the cached write.
    #[serde(default)]
    pub at_ms: Option<u128>,
    /// Event sequence associated with the cached write.
    #[serde(default)]
    pub seq: Option<u64>,
    /// Cache source label, currently `osc52-cache`.
    #[serde(default)]
    pub source: Option<String>,
}

impl ClipboardStatus {
    /// Whether this reply includes a clipboard payload.
    pub fn has_payload(&self) -> bool {
        self.allowed && self.available && self.payload_base64.is_some()
    }
}

/// Machine-readable native shortcut catalog returned by `SHORTCUTS_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutCatalog {
    /// Schema version for the shortcut catalog shape.
    #[serde(default)]
    pub schema_version: Option<u32>,
    /// Catalog kind marker, currently `kittwm-native-shortcuts`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Native shortcut entries.
    #[serde(default)]
    pub shortcuts: Vec<ShortcutEntry>,
}

impl ShortcutCatalog {
    /// Find a shortcut catalog entry by stable id.
    pub fn entry(&self, id: &str) -> Option<&ShortcutEntry> {
        self.shortcuts.iter().find(|entry| entry.id == id)
    }

    /// Title marker legend entry, when reported by the daemon.
    pub fn title_marker_legend(&self) -> Option<&ShortcutEntry> {
        self.entry("title_markers")
    }

    /// Mouse title-drag entry for reordering tiled panes.
    pub fn tiled_title_drag_shortcut(&self) -> Option<&ShortcutEntry> {
        self.entry("drag_tiled_title")
    }

    /// Mouse title-drag entry for repositioning floating panes.
    pub fn floating_title_drag_shortcut(&self) -> Option<&ShortcutEntry> {
        self.entry("drag_floating_title")
    }

    /// Whether both mouse title-drag shortcut entries are present.
    pub fn has_mouse_title_drag_shortcuts(&self) -> bool {
        self.tiled_title_drag_shortcut().is_some() && self.floating_title_drag_shortcut().is_some()
    }

    /// Whether the catalog includes the title marker legend entry.
    pub fn has_title_marker_legend(&self) -> bool {
        self.title_marker_legend().is_some()
    }
}

/// One native shortcut entry in [`ShortcutCatalog`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutEntry {
    /// Stable machine-readable action id.
    pub id: String,
    /// Human-readable key chord(s).
    pub keys: String,
    /// Human-readable description.
    pub description: String,
}

impl ShortcutEntry {
    /// Whether this entry is the pane-title marker legend.
    pub fn is_title_marker_legend(&self) -> bool {
        self.id == "title_markers"
    }

    /// Whether this entry describes tiled title-drag reordering.
    pub fn is_tiled_title_drag_shortcut(&self) -> bool {
        self.id == "drag_tiled_title"
    }

    /// Whether this entry describes floating title-drag repositioning.
    pub fn is_floating_title_drag_shortcut(&self) -> bool {
        self.id == "drag_floating_title"
    }
}

/// Machine-readable socket help catalog returned by `HELP_JSON`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpCatalog {
    /// Supported commands.
    #[serde(default)]
    pub commands: Vec<HelpCommand>,
}

/// One socket command entry in [`HelpCatalog`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpCommand {
    /// Command syntax.
    pub command: String,
    /// Category such as `control`, `automation`, or `semantic`.
    pub category: String,
    /// Human-readable description.
    pub description: String,
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

fn process_snapshot_from_panes(
    socket: PathBuf,
    panes: &[NativePaneDetail],
) -> KittwmProcessSnapshot {
    let processes = panes
        .iter()
        .map(|pane| {
            let host = pane.pid.and_then(read_host_process_info);
            KittwmProcessInfo {
                window: pane.window.clone(),
                title: pane.title.clone(),
                focused: pane.focused,
                pid: pane.pid,
                ppid: host.as_ref().and_then(|info| info.ppid),
                command: pane.command.clone(),
                state: host.as_ref().and_then(|info| info.state.clone()),
                rss_kib: host.as_ref().and_then(|info| info.rss_kib),
                cpu_percent: host.as_ref().and_then(|info| info.cpu_percent),
                process_name: host.and_then(|info| info.process_name),
            }
        })
        .collect();
    KittwmProcessSnapshot { socket, processes }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct HostProcessInfo {
    ppid: Option<u32>,
    state: Option<String>,
    rss_kib: Option<u64>,
    cpu_percent: Option<f32>,
    process_name: Option<String>,
}

fn read_host_process_info(pid: u32) -> Option<HostProcessInfo> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=,stat=,rss=,pcpu=,comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_ps_process_line(std::str::from_utf8(&output.stdout).ok()?)
}

fn parse_ps_process_line(output: &str) -> Option<HostProcessInfo> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let mut parts = line.split_whitespace();
    let ppid = parts.next().and_then(|value| value.parse().ok());
    let state = parts.next().map(str::to_string);
    let rss_kib = parts.next().and_then(|value| value.parse().ok());
    let cpu_percent = parts.next().and_then(|value| value.parse().ok());
    let process_name = parts.next().map(|first| {
        let remaining_len = parts.clone().map(|part| 1 + part.len()).sum::<usize>();
        let mut rest = String::with_capacity(first.len() + remaining_len);
        rest.push_str(first);
        for part in parts {
            rest.push(' ');
            rest.push_str(part);
        }
        rest
    });
    Some(HostProcessInfo {
        ppid,
        state,
        rss_kib,
        cpu_percent,
        process_name,
    })
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

    /// Fetch process information for panes in the current kittwm session.
    pub fn processes(&self) -> Result<KittwmProcessSnapshot> {
        let panes = self.panes()?;
        Ok(process_snapshot_from_panes(
            self.socket.clone(),
            &panes.panes_detail,
        ))
    }

    /// Fetch process information for panes in the current kittwm session.
    pub fn process_snapshot(&self) -> Result<KittwmProcessSnapshot> {
        self.processes()
    }

    /// Fetch typed native chrome/workspace reservation metadata from `CHROME_JSON`.
    pub fn chrome(&self) -> Result<ChromeReservationStatus> {
        Ok(serde_json::from_str(
            &self.request_protocol("CHROME_JSON")?,
        )?)
    }

    /// Alias for [`Kittwm::chrome`].
    pub fn chrome_json(&self) -> Result<ChromeReservationStatus> {
        self.chrome()
    }

    /// Request drawable screen reservations for bar/dock-style apps via
    /// `RESERVE_CHROME_JSON`. Normal tiled applications should stay inside the
    /// remaining drawable area; specialized chrome apps may use the reserved
    /// bands intentionally.
    pub fn reserve_chrome(&self, request: &ChromeReservationRequest) -> Result<String> {
        self.capabilities.ensure(Capability::ControlWindow)?;
        let payload = serde_json::to_string(request)?;
        self.request_protocol(json_payload_request("RESERVE_CHROME_JSON", &payload))
    }

    /// Clear custom chrome reservation back to the daemon default.
    pub fn clear_chrome_reservation(&self) -> Result<String> {
        self.reserve_chrome(&ChromeReservationRequest::top_bar(1))
    }

    /// Fetch the current native session manifest via `SESSION_JSON`.
    pub fn session(&self) -> Result<SessionManifest> {
        self.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(
            &self.request_protocol("SESSION_JSON")?,
        )?)
    }

    /// Restore native panes from a typed session manifest via
    /// `RESTORE_SESSION_JSON`. This is a window mutation and therefore requires
    /// both create and control capabilities.
    pub fn restore_session(&self, manifest: &SessionManifest) -> Result<String> {
        self.capabilities.ensure(Capability::CreateWindow)?;
        self.capabilities.ensure(Capability::ControlWindow)?;
        let payload = serde_json::to_string(manifest)?;
        self.request_protocol(json_payload_request("RESTORE_SESSION_JSON", &payload))
    }

    /// Fetch the policy-gated cached OSC52 clipboard status via `CLIPBOARD_JSON`.
    ///
    /// The daemon is default-deny: denied replies parse successfully with
    /// `allowed == false` and no payload. This helper does not read the host OS
    /// clipboard; it only inspects kittwm's cached nested-app OSC52 write.
    pub fn clipboard(&self) -> Result<ClipboardStatus> {
        self.capabilities.ensure(Capability::Clipboard)?;
        Ok(serde_json::from_str(
            &self.request_protocol("CLIPBOARD_JSON")?,
        )?)
    }

    /// Alias for [`Kittwm::clipboard`].
    pub fn clipboard_json(&self) -> Result<ClipboardStatus> {
        self.clipboard()
    }

    /// Fetch the native shortcut catalog from `SHORTCUTS_JSON`.
    pub fn shortcuts(&self) -> Result<ShortcutCatalog> {
        self.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(
            &self.request_protocol("SHORTCUTS_JSON")?,
        )?)
    }

    /// Alias for [`Kittwm::shortcuts`].
    pub fn shortcuts_json(&self) -> Result<ShortcutCatalog> {
        self.shortcuts()
    }

    /// Fetch the native socket command catalog from `HELP_JSON`.
    pub fn help_catalog(&self) -> Result<HelpCatalog> {
        self.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(&self.request_protocol("HELP_JSON")?)?)
    }

    /// Alias for [`Kittwm::help_catalog`].
    pub fn help(&self) -> Result<HelpCatalog> {
        self.help_catalog()
    }

    /// Fetch the native app discovery catalog from `APPS_JSON`.
    pub fn apps(&self) -> Result<AppsCatalog> {
        self.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(&self.request_protocol("APPS_JSON")?)?)
    }

    /// Return the first app-discovery candidate matching a query.
    pub fn app_first(&self, query: impl AsRef<str>) -> Result<AppCandidate> {
        self.capabilities.ensure(Capability::ReadText)?;
        let query = validated_nonempty_trimmed(query.as_ref(), "APPS_FIRST query")?;
        parse_app_first_reply(&self.request_protocol(app_lookup_request("APPS_FIRST", query))?)
    }

    /// Launch the first app-discovery candidate matching a query.
    pub fn app_launch_first(&self, query: impl AsRef<str>) -> Result<AppLaunch> {
        self.capabilities.ensure(Capability::CreateWindow)?;
        let query = validated_nonempty_trimmed(query.as_ref(), "APPS_LAUNCH_FIRST query")?;
        parse_app_launch_reply(
            &self.request_protocol(app_lookup_request("APPS_LAUNCH_FIRST", query))?,
        )
    }

    /// Focus the next pane/window.
    pub fn focus_next(&self) -> Result<String> {
        self.capabilities.ensure(Capability::ControlWindow)?;
        self.request_protocol("FOCUS_NEXT")
    }

    /// Focus the previous pane/window.
    pub fn focus_prev(&self) -> Result<String> {
        self.capabilities.ensure(Capability::ControlWindow)?;
        self.request_protocol("FOCUS_PREV")
    }

    /// Set the session layout axis.
    pub fn layout(&self, mode: LayoutMode) -> Result<String> {
        self.capabilities.ensure(Capability::ControlWindow)?;
        self.request_protocol(layout_request(mode))
    }

    /// Split next to a target pane, set the layout axis, and launch a terminal command.
    pub fn split_pane(
        &self,
        target: impl AsRef<str>,
        mode: LayoutMode,
        command: impl AsRef<str>,
    ) -> Result<String> {
        self.capabilities.ensure(Capability::CreateWindow)?;
        self.capabilities.ensure(Capability::ControlWindow)?;
        let target = validated_protocol_token(target.as_ref(), "SPLIT_PANE target")?;
        let command = validated_nonempty_trimmed(command.as_ref(), "SPLIT_PANE command")?;
        self.request_protocol(split_pane_request(target, mode, command))
    }

    /// Split next to the focused pane and launch a terminal command.
    pub fn split_focused(&self, mode: LayoutMode, command: impl AsRef<str>) -> Result<String> {
        self.split_pane("focused", mode, command)
    }

    /// Balance pane weights in the current layout.
    pub fn balance_panes(&self) -> Result<String> {
        self.capabilities.ensure(Capability::ControlWindow)?;
        self.request_protocol("BALANCE_PANES")
    }

    /// Reset every floating pane offset to generated layout positions.
    pub fn reset_all_positions(&self) -> Result<String> {
        self.capabilities.ensure(Capability::ControlWindow)?;
        self.request_protocol("RESET_ALL_PANE_OFFSETS")
    }

    /// Alias for [`Kittwm::reset_all_positions`].
    pub fn reset_all_floating_offsets(&self) -> Result<String> {
        self.reset_all_positions()
    }

    /// Fetch a bounded batch of native JSON-lines events.
    pub fn events_ms(&self, ms: u64) -> Result<Vec<KittwmEvent>> {
        self.capabilities.ensure(Capability::SubscribeEvents)?;
        let ms = ms.clamp(1, 60_000);
        let reply = self.request_protocol(events_request(ms))?;
        reply
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && trimmed != "END"
            })
            .map(KittwmEvent::parse_line)
            .collect()
    }

    /// Fetch a bounded event batch and return it as an owning iterator.
    pub fn events_iter_ms(&self, ms: u64) -> Result<KittwmEventIter> {
        self.events_ms(ms).map(KittwmEventIter::from)
    }

    /// Alias for [`Kittwm::events_iter_ms`].
    pub fn event_iter_ms(&self, ms: u64) -> Result<KittwmEventIter> {
        self.events_iter_ms(ms)
    }

    /// Return a typed handle to an existing surface/window id.
    pub fn surface(&self, id: impl Into<String>) -> SurfaceHandle {
        SurfaceHandle {
            client: self.clone(),
            id: id.into().trim().to_string(),
        }
    }

    /// Return a typed handle to the currently focused surface/window.
    pub fn focused_surface(&self) -> SurfaceHandle {
        self.surface("focused")
    }

    /// Ask kittwm to spawn a typed surface. The v0 transport supports terminal
    /// surfaces via `SPAWN_PTY`; browser surfaces use the same command path, but
    /// live kittwm recognizes `kittwm-browser` as a direct browser capture
    /// surface.
    pub fn spawn_surface(&self, spec: &SurfaceSpec) -> Result<SurfaceSpawn> {
        self.capabilities.ensure(Capability::CreateWindow)?;
        let command = spec.native_pty_command()?;
        let reply = self.request_protocol(spawn_pty_request(&command))?;
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
            let _ = self.request_protocol(surface_token_request("CLOSE_PANE", &handle.id));
        }
        Ok(reply)
    }
}

fn validated_base64_payload<'a>(payload_b64: &'a str, verb: &str) -> Result<&'a str> {
    let payload_b64 = payload_b64.trim();
    if payload_b64.is_empty() {
        return Err(Error::Daemon(format!("{verb} requires nonempty base64")));
    }
    BASE64_STANDARD
        .decode(payload_b64)
        .map_err(|err| Error::Daemon(format!("{verb} invalid base64: {err}")))?;
    Ok(payload_b64)
}

fn validated_text_payload<'a>(text: &'a str, verb: &str) -> Result<&'a str> {
    if text.is_empty() {
        return Err(Error::Daemon(format!("{verb} requires nonempty text")));
    }
    Ok(text)
}

fn validated_pane_title<'a>(title: &'a str) -> Result<&'a str> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Daemon(
            "RENAME_PANE requires nonempty title".to_string(),
        ));
    }
    Ok(title)
}

fn validated_protocol_token<'a>(token: &'a str, label: &str) -> Result<&'a str> {
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(Error::Daemon(format!(
            "{label} must be a single nonempty token"
        )));
    }
    Ok(token)
}

fn validated_wait_needle<'a>(needle: &'a str, verb: &str) -> Result<&'a str> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Err(Error::Daemon(format!("{verb} requires nonempty needle")));
    }
    Ok(needle)
}

impl SurfaceHandle {
    /// Focus this surface/window.
    pub fn focus(&self) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client
            .request_protocol(surface_token_request("FOCUS_PANE", &self.id))
    }

    /// Close this surface/window.
    pub fn close(&self) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client
            .request_protocol(surface_token_request("CLOSE_PANE", &self.id))
    }

    /// Rename this surface/window.
    pub fn rename(&self, title: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        let title = validated_pane_title(title.as_ref())?;
        self.client
            .request_protocol(surface_payload_request("RENAME_PANE", &self.id, title))
    }

    /// Resize this surface/window by a relative pane-weight delta.
    pub fn resize_weight(&self, delta: i16) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        let label = resize_delta_label(delta);
        self.client
            .request_protocol(surface_payload_request("RESIZE_PANE", &self.id, &label))
    }

    /// Move this pane within the current layout.
    pub fn move_pane(&self, direction: MoveDirection) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client.request_protocol(surface_payload_request(
            "MOVE_PANE",
            &self.id,
            direction.protocol_label(),
        ))
    }

    /// Raise this pane to the top of the stack/floating order.
    pub fn raise(&self) -> Result<String> {
        self.move_pane(MoveDirection::Last)
    }

    /// Nudge this pane's floating offset by cell deltas.
    pub fn nudge(&self, dx: i16, dy: i16) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        let payload = nudge_delta_payload(dx, dy);
        self.client
            .request_protocol(surface_payload_request("NUDGE_PANE", &self.id, &payload))
    }

    /// Reset this pane's floating offset to the generated layout position.
    pub fn reset_floating_offset(&self) -> Result<String> {
        self.client.capabilities.ensure(Capability::ControlWindow)?;
        self.client
            .request_protocol(surface_token_request("RESET_PANE_OFFSET", &self.id))
    }

    /// Alias for [`SurfaceHandle::reset_floating_offset`].
    pub fn reset_position(&self) -> Result<String> {
        self.reset_floating_offset()
    }

    /// Lower this pane to the bottom of the stack/floating order.
    pub fn lower(&self) -> Result<String> {
        self.move_pane(MoveDirection::First)
    }

    /// Send raw UTF-8 text.
    pub fn send_text(&self, text: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        let text = validated_text_payload(text.as_ref(), "SEND_TEXT")?;
        self.client
            .request_protocol(surface_payload_request("SEND_TEXT", &self.id, text))
    }

    /// Send one line, appending a newline in the daemon.
    pub fn send_line(&self, text: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        let text = validated_text_payload(text.as_ref(), "SEND_LINE")?;
        self.client
            .request_protocol(surface_payload_request("SEND_LINE", &self.id, text))
    }

    /// Send a named key such as `ctrl-c`, `escape`, or `up`.
    pub fn send_key(&self, key: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        let key = validated_protocol_token(key.as_ref(), "SEND_KEY key")?;
        self.client
            .request_protocol(surface_payload_request("SEND_KEY", &self.id, key))
    }

    /// Send exact bytes, base64-encoding them for `SEND_BYTES_B64`.
    pub fn send_bytes(&self, bytes: impl AsRef<[u8]>) -> Result<String> {
        self.send_bytes_b64(BASE64_STANDARD.encode(bytes.as_ref()))
    }

    /// Send an already-base64-encoded exact byte payload.
    pub fn send_bytes_b64(&self, payload_b64: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        let payload_b64 = validated_base64_payload(payload_b64.as_ref(), "SEND_BYTES_B64")?;
        self.client.request_protocol(surface_payload_request(
            "SEND_BYTES_B64",
            &self.id,
            payload_b64,
        ))
    }

    /// Paste exact bytes, base64-encoding them for `PASTE_BYTES_B64`.
    pub fn paste_bytes(&self, bytes: impl AsRef<[u8]>) -> Result<String> {
        self.paste_bytes_b64(BASE64_STANDARD.encode(bytes.as_ref()))
    }

    /// Paste an already-base64-encoded byte payload.
    pub fn paste_bytes_b64(&self, payload_b64: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        let payload_b64 = validated_base64_payload(payload_b64.as_ref(), "PASTE_BYTES_B64")?;
        self.client.request_protocol(surface_payload_request(
            "PASTE_BYTES_B64",
            &self.id,
            payload_b64,
        ))
    }

    /// Send a pane-local mouse event at cell coordinates.
    pub fn send_mouse(&self, event: MouseEvent, col: u16, row: u16) -> Result<String> {
        self.client.capabilities.ensure(Capability::SendInput)?;
        self.client
            .request_protocol(surface_mouse_request(&self.id, event, col, row))
    }

    /// Read the current screen text snapshot.
    pub fn read_text(&self) -> Result<TextSnapshot> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_token_request("READ_TEXT_JSON", &self.id),
        )?)?)
    }

    /// Read the current scrollback snapshot.
    pub fn read_scrollback(&self) -> Result<ScrollbackSnapshot> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_token_request("READ_SCROLLBACK_JSON", &self.id),
        )?)?)
    }

    /// Wait for text to appear in the visible screen snapshot.
    pub fn wait_text_ms(&self, ms: u64, needle: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_TEXT_MS")?;
        self.client.request_protocol(surface_wait_ms_request(
            "WAIT_TEXT_MS",
            &self.id,
            ms.clamp(1, 60_000),
            needle,
        ))
    }

    /// Wait for text to appear in the visible screen or scrollback snapshots.
    pub fn wait_output_ms(&self, ms: u64, needle: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_OUTPUT_MS")?;
        self.client.request_protocol(surface_wait_ms_request(
            "WAIT_OUTPUT_MS",
            &self.id,
            ms.clamp(1, 60_000),
            needle,
        ))
    }

    /// Wait up to the daemon's default timeout for visible screen text.
    pub fn wait_text(&self, needle: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_TEXT")?;
        self.client
            .request_protocol(surface_payload_request("WAIT_TEXT", &self.id, needle))
    }

    /// Wait up to the daemon's default timeout for visible screen or scrollback text.
    pub fn wait_output(&self, needle: impl AsRef<str>) -> Result<String> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_OUTPUT")?;
        self.client
            .request_protocol(surface_payload_request("WAIT_OUTPUT", &self.id, needle))
    }

    /// Wait for visible screen text and return typed match metadata.
    pub fn wait_text_match_ms(&self, ms: u64, needle: impl AsRef<str>) -> Result<WaitMatch> {
        parse_wait_match(&self.wait_text_ms(ms, needle)?)
    }

    /// Wait for visible screen or scrollback output and return typed match metadata.
    pub fn wait_output_match_ms(&self, ms: u64, needle: impl AsRef<str>) -> Result<WaitMatch> {
        parse_wait_match(&self.wait_output_ms(ms, needle)?)
    }

    /// Wait for visible screen text via the JSON wait command and return typed metadata.
    pub fn wait_text_match_json_ms(&self, ms: u64, needle: impl AsRef<str>) -> Result<WaitMatch> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_TEXT_JSON_MS")?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_wait_ms_request("WAIT_TEXT_JSON_MS", &self.id, ms.clamp(1, 60_000), needle),
        )?)?)
    }

    /// Wait for visible screen or scrollback output via the JSON wait command.
    pub fn wait_output_match_json_ms(&self, ms: u64, needle: impl AsRef<str>) -> Result<WaitMatch> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_OUTPUT_JSON_MS")?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_wait_ms_request("WAIT_OUTPUT_JSON_MS", &self.id, ms.clamp(1, 60_000), needle),
        )?)?)
    }

    /// Wait up to the daemon's default timeout for visible text and return typed metadata.
    pub fn wait_text_match(&self, needle: impl AsRef<str>) -> Result<WaitMatch> {
        parse_wait_match(&self.wait_text(needle)?)
    }

    /// Wait up to the daemon's default timeout for visible or scrollback output and return typed metadata.
    pub fn wait_output_match(&self, needle: impl AsRef<str>) -> Result<WaitMatch> {
        parse_wait_match(&self.wait_output(needle)?)
    }

    /// Wait up to the daemon's default timeout for visible text via the JSON wait command.
    pub fn wait_text_match_json(&self, needle: impl AsRef<str>) -> Result<WaitMatch> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_TEXT_JSON")?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_payload_request("WAIT_TEXT_JSON", &self.id, needle),
        )?)?)
    }

    /// Wait up to the daemon's default timeout for visible or scrollback output via JSON.
    pub fn wait_output_match_json(&self, needle: impl AsRef<str>) -> Result<WaitMatch> {
        self.client.capabilities.ensure(Capability::ReadText)?;
        let needle = validated_wait_needle(needle.as_ref(), "WAIT_OUTPUT_JSON")?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_payload_request("WAIT_OUTPUT_JSON", &self.id, needle),
        )?)?)
    }

    /// Read the semantic component snapshot for this surface.
    pub fn semantic_snapshot(&self) -> Result<SemanticSurfaceSnapshot> {
        self.client
            .capabilities
            .ensure(Capability::ReadSemanticTree)?;
        Ok(serde_json::from_str(&self.client.request_protocol(
            surface_token_request("SEMANTIC_SNAPSHOT", &self.id),
        )?)?)
    }

    /// Publish the current semantic component snapshot for this surface.
    pub fn semantic_publish(&self, snapshot: &SemanticSurfaceSnapshot) -> Result<String> {
        self.client
            .capabilities
            .ensure(Capability::PublishSemanticTree)?;
        let payload = serde_json::to_string(snapshot)?;
        self.client.request_protocol(surface_payload_request(
            "SEMANTIC_PUBLISH",
            &self.id,
            &payload,
        ))
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
        let component = validated_protocol_token(component.as_ref(), "semantic component")?;
        let action = validated_protocol_token(action.as_ref(), "semantic action")?;
        let payload = serde_json::to_string(&payload)?;
        self.client.request_protocol(surface_action_request(
            "SEMANTIC_ACTION",
            &self.id,
            component,
            action,
            &payload,
        ))
    }

    /// Request semantic focus for a component.
    pub fn semantic_focus(&self, component: impl AsRef<str>) -> Result<String> {
        self.client
            .capabilities
            .ensure(Capability::InvokeSemanticAction)?;
        let component = validated_protocol_token(component.as_ref(), "semantic component")?;
        self.client.request_protocol(surface_payload_request(
            "SEMANTIC_FOCUS",
            &self.id,
            component,
        ))
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
        .unwrap_or(display);
    let mut filename = String::with_capacity("kittwm-".len() + token.len() + ".sock".len());
    filename.push_str("kittwm-");
    for ch in token.chars() {
        if ch == '/' {
            filename.push('_');
        } else {
            filename.push(ch);
        }
    }
    filename.push_str(".sock");
    env::temp_dir().join(filename)
}

fn layout_request(mode: LayoutMode) -> String {
    let label = mode.protocol_label();
    let mut out = String::with_capacity("LAYOUT ".len() + label.len());
    out.push_str("LAYOUT ");
    out.push_str(label);
    out
}

fn split_pane_request(target: &str, mode: LayoutMode, command: &str) -> String {
    let label = mode.protocol_label();
    let mut out = String::with_capacity(
        "SPLIT_PANE ".len() + target.len() + 1 + label.len() + 1 + command.len(),
    );
    out.push_str("SPLIT_PANE ");
    out.push_str(target);
    out.push(' ');
    out.push_str(label);
    out.push(' ');
    out.push_str(command);
    out
}

fn spawn_pty_request(command: &str) -> String {
    let mut out = String::with_capacity("SPAWN_PTY ".len() + command.len());
    out.push_str("SPAWN_PTY ");
    out.push_str(command);
    out
}

fn app_lookup_request(verb: &str, query: &str) -> String {
    let mut out = String::with_capacity(verb.len() + 1 + query.len());
    out.push_str(verb);
    out.push(' ');
    out.push_str(query);
    out
}

fn json_payload_request(verb: &str, payload: &str) -> String {
    let mut out = String::with_capacity(verb.len() + 1 + payload.len());
    out.push_str(verb);
    out.push(' ');
    out.push_str(payload);
    out
}

fn events_request(ms: u64) -> String {
    let mut out = String::with_capacity("EVENTS ".len() + 5);
    out.push_str("EVENTS ");
    out.push_str(&ms.to_string());
    out
}

fn surface_token_request(verb: &str, id: &str) -> String {
    let mut out = String::with_capacity(verb.len() + 1 + id.len());
    out.push_str(verb);
    out.push(' ');
    out.push_str(id);
    out
}

fn surface_payload_request(verb: &str, id: &str, payload: &str) -> String {
    let mut out = String::with_capacity(verb.len() + 1 + id.len() + 1 + payload.len());
    out.push_str(verb);
    out.push(' ');
    out.push_str(id);
    out.push(' ');
    out.push_str(payload);
    out
}

fn resize_delta_label(delta: i16) -> String {
    let magnitude = delta.unsigned_abs().to_string();
    let mut out = String::with_capacity(if delta >= 0 {
        1 + magnitude.len()
    } else {
        1 + magnitude.len()
    });
    out.push(if delta >= 0 { '+' } else { '-' });
    out.push_str(&magnitude);
    out
}

fn i16_decimal_len(value: i16) -> usize {
    if value < 0 {
        1 + u32_decimal_len(value.unsigned_abs() as u32)
    } else {
        u32_decimal_len(value as u32)
    }
}

fn u32_decimal_len(mut value: u32) -> usize {
    let mut len = 1;
    while value >= 10 {
        value /= 10;
        len += 1;
    }
    len
}

fn nudge_delta_payload(dx: i16, dy: i16) -> String {
    let mut out = String::with_capacity(i16_decimal_len(dx) + 1 + i16_decimal_len(dy));
    write!(out, "{dx} {dy}").expect("write to string");
    out
}

fn surface_wait_ms_request(verb: &str, id: &str, ms: u64, needle: &str) -> String {
    let ms_text = ms.to_string();
    let mut out =
        String::with_capacity(verb.len() + 1 + id.len() + 1 + ms_text.len() + 1 + needle.len());
    out.push_str(verb);
    out.push(' ');
    out.push_str(id);
    out.push(' ');
    out.push_str(&ms_text);
    out.push(' ');
    out.push_str(needle);
    out
}

fn surface_mouse_request(id: &str, event: MouseEvent, col: u16, row: u16) -> String {
    let event_label = event.protocol_label();
    let col_text = col.to_string();
    let row_text = row.to_string();
    let mut out = String::with_capacity(
        "SEND_MOUSE ".len()
            + id.len()
            + 1
            + event_label.len()
            + 1
            + col_text.len()
            + 1
            + row_text.len(),
    );
    out.push_str("SEND_MOUSE ");
    out.push_str(id);
    out.push(' ');
    out.push_str(event_label);
    out.push(' ');
    out.push_str(&col_text);
    out.push(' ');
    out.push_str(&row_text);
    out
}

fn surface_action_request(
    verb: &str,
    id: &str,
    component: &str,
    action: &str,
    payload: &str,
) -> String {
    let mut out = String::with_capacity(
        verb.len() + 1 + id.len() + 1 + component.len() + 1 + action.len() + 1 + payload.len(),
    );
    out.push_str(verb);
    out.push(' ');
    out.push_str(id);
    out.push(' ');
    out.push_str(component);
    out.push(' ');
    out.push_str(action);
    out.push(' ');
    out.push_str(payload);
    out
}

fn parse_app_first_reply(reply: &str) -> Result<AppCandidate> {
    let line = reply.trim();
    let fields = line
        .strip_prefix("APPS_FIRST ")
        .ok_or_else(|| Error::Daemon(line.to_string()))?;
    parse_app_candidate_fields(fields)
}

fn validated_nonempty_trimmed<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::Daemon(format!("{label} must be nonempty")));
    }
    Ok(value)
}

fn parse_app_launch_reply(reply: &str) -> Result<AppLaunch> {
    let line = reply.trim();
    let fields = line
        .strip_prefix("APPS_LAUNCH_FIRST ")
        .ok_or_else(|| Error::Daemon(line.to_string()))?;
    let mut pid = None;
    let mut candidate_fields = String::with_capacity(fields.len());
    for field in fields.split_whitespace() {
        if let Some(value) = field.strip_prefix("pid=") {
            pid =
                Some(value.parse::<u32>().map_err(|_| {
                    Error::Daemon(format!("invalid APPS_LAUNCH_FIRST pid: {value}"))
                })?);
        } else {
            if !candidate_fields.is_empty() {
                candidate_fields.push(' ');
            }
            candidate_fields.push_str(field);
        }
    }
    Ok(AppLaunch {
        pid: pid.ok_or_else(|| Error::Daemon(format!("missing APPS_LAUNCH_FIRST pid: {line}")))?,
        candidate: parse_app_candidate_fields(&candidate_fields)?,
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
    let quoted = shell_quote(target);
    let mut out = String::with_capacity("kittwm-browser ".len() + quoted.len());
    out.push_str("kittwm-browser ");
    out.push_str(&quoted);
    out
}

fn parse_wait_match(reply: &str) -> Result<WaitMatch> {
    let line = reply.trim();
    let (kind, fields) = if let Some(rest) = line.strip_prefix("MATCH_TEXT ") {
        (WaitMatchKind::Text, rest)
    } else if let Some(rest) = line.strip_prefix("MATCH_OUTPUT ") {
        (WaitMatchKind::Output, rest)
    } else {
        return Err(Error::Daemon(format!("invalid wait match reply: {line}")));
    };
    let mut window = None;
    let mut bytes = None;
    for field in fields.split_whitespace() {
        if let Some(value) = field.strip_prefix("window=") {
            window = Some(value.to_string());
        } else if let Some(value) = field.strip_prefix("bytes=") {
            bytes = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| Error::Daemon(format!("invalid wait match bytes: {value}")))?,
            );
        }
    }
    Ok(WaitMatch {
        kind,
        window: window
            .ok_or_else(|| Error::Daemon(format!("missing wait match window: {line}")))?,
        bytes: bytes.ok_or_else(|| Error::Daemon(format!("missing wait match bytes: {line}")))?,
    })
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    {
        value.to_string()
    } else {
        let quote_count = value.bytes().filter(|byte| *byte == b'\'').count();
        let mut out = String::with_capacity(2 + value.len() + quote_count * 3);
        out.push('\'');
        for ch in value.chars() {
            if ch == '\'' {
                out.push_str("'\\''");
            } else {
                out.push(ch);
            }
        }
        out.push('\'');
        out
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
    use std::fmt::Write as FmtWrite;
    use std::sync::Mutex;

    #[cfg(unix)]
    use std::io::{BufRead, BufReader};
    #[cfg(unix)]
    use std::os::unix::net::UnixListener;
    #[cfg(unix)]
    use std::thread;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_yaml_path(prefix: &str, pid: u32, label: &str) -> PathBuf {
        let mut name = String::with_capacity(
            prefix.len() + 1 + u32_decimal_len(pid) + 1 + label.len() + ".yaml".len(),
        );
        name.push_str(prefix);
        name.push('-');
        write!(name, "{pid}").expect("write to string");
        name.push('-');
        name.push_str(label);
        name.push_str(".yaml");
        env::temp_dir().join(name)
    }

    #[test]
    fn test_yaml_path_builds_directly() {
        let path = test_yaml_path("kittwm-config-test", 1234, "partial");
        let text = path.to_string_lossy();
        assert!(
            text.ends_with("/kittwm-config-test-1234-partial.yaml"),
            "{text}"
        );
    }

    #[test]
    fn kittwm_config_defaults_to_nord_background_and_colorscheme() {
        let config = KittwmConfig::nord_default();
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.background.color, "nord0");
        assert_eq!(config.background.opacity, 0.6);
        assert_eq!(config.background.effects.len(), 1);
        assert_eq!(config.background.effects[0].kind, "lens_flare");
        assert_eq!(config.background.effects[0].palette, "nord_aurora");
        assert_eq!(config.colorscheme.name, "nord");
        assert_eq!(config.colorscheme.fg, "#d8dee9");
        assert_eq!(config.colorscheme.bg, "#2e3440");
        assert_eq!(config.colorscheme.ansi_color(0), Some("#3b4252"));
        assert_eq!(config.colorscheme.ansi_color(15), Some("#eceff4"));
        assert_eq!(config.colorscheme.ansi_color(16), None);
        assert_eq!(config.terminal.backend, "ghostty");
        assert_eq!(config.terminal.command, None);
        assert_eq!(config.libghostty.theme, "nord");
        assert_eq!(config.libghostty.background, "nord0");
        assert_eq!(config.libghostty.background_opacity, 0.72);
        assert!(config.libghostty.enable_ghostty_features);
        assert!(config.libghostty.kitty_graphics);
        let roundtrip: KittwmConfig =
            serde_yaml::from_str(&config.to_yaml_string().unwrap()).unwrap();
        assert_eq!(roundtrip, config);
    }

    #[test]
    fn kittwm_config_loads_partial_yaml_over_nord_defaults() {
        let path = test_yaml_path("kittwm-config-test", std::process::id(), "partial");
        std::fs::write(
            &path,
            "background:\n  opacity: 0.5\ncolorscheme:\n  name: nord\n",
        )
        .unwrap();
        let config = KittwmConfig::load_path(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(config.background.color, "nord0");
        assert_eq!(config.background.opacity, 0.5);
        assert_eq!(config.colorscheme.name, "nord");
        assert_eq!(config.colorscheme.colors.len(), 16);
        assert_eq!(config.terminal.backend, "ghostty");
        assert_eq!(config.libghostty.background_opacity, 0.72);
    }

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
        assert_eq!(
            display_to_socket_path(":team/dev.0"),
            env::temp_dir().join("kittwm-team_dev.sock")
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
    fn architecture_contract_exposes_wm_boundaries_for_apps() {
        let contract = ArchitectureContract::current();
        assert_eq!(contract.schema_version, 1);
        assert_eq!(contract.kind, "kittwm-architecture-contract");
        assert!(contract.layer("sdk-control-plane").is_some());
        let tiling = contract.layer("tiling-engine").unwrap();
        assert!(tiling
            .invariants
            .iter()
            .any(|invariant| invariant.contains("outer bounds are disjoint")));
        assert!(contract
            .composition_order
            .iter()
            .any(|plane| { plane.plane == "decorations" && plane.z_index == 20 }));
        assert_eq!(contract.app_surface_z_index(), Some(0));
        assert_eq!(contract.decoration_z_index(), Some(20));
        assert_eq!(contract.overlay_z_index(), Some(30));
        assert_eq!(contract.z_index_for_plane("decorations"), Some(20));
        assert_eq!(
            contract.z_index_for_role(SurfacePlacementRole::AppSurface),
            Some(0)
        );
        assert_eq!(
            contract.z_index_for_role(SurfacePlacementRole::Decoration),
            Some(20)
        );
        assert_eq!(
            contract.z_index_for_role(SurfacePlacementRole::Overlay),
            Some(30)
        );
        assert_eq!(
            contract
                .composition_plane_for_role(SurfacePlacementRole::Decoration)
                .unwrap()
                .plane,
            "decorations"
        );
        assert_eq!(
            contract.composition_plane("overlays").unwrap().plane,
            "overlays"
        );
        assert_eq!(
            contract.ordered_plane_names().collect::<Vec<_>>(),
            ["app-surfaces", "decorations", "overlays"]
        );
        assert_eq!(
            contract.plane_is_above("decorations", "app-surfaces"),
            Some(true)
        );
        assert_eq!(
            contract.plane_is_above("app-surfaces", "decorations"),
            Some(false)
        );
        assert_eq!(
            contract.plane_is_above("overlays", "decorations"),
            Some(true)
        );
        assert_eq!(contract.plane_is_above("missing", "decorations"), None);
        assert!(contract
            .composition_plane("app-surfaces")
            .unwrap()
            .is_below(contract.composition_plane("decorations").unwrap()));
        assert!(contract.z_index_for_plane("missing").is_none());
        let browser = contract.native_surface("kittwm-browser").unwrap();
        assert_eq!(browser.sdk_entry, "SurfaceSpec::browser");
        assert!(browser.sdk_backed);
        assert!(browser.kitty_graphics_native);
        assert_eq!(browser.composition_plane(), Some("app-surfaces"));
        assert_eq!(browser.z_index(&contract), Some(0));
        assert_eq!(
            browser.kittui_entry,
            "HeadlessBrowserApp -> Runtime::place_png_frame_with_options"
        );
        assert!(contract.all_native_surfaces_ready());
        let ready_names = contract
            .native_ready_surfaces()
            .map(|surface| surface.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ready_names,
            [
                "kittwm-terminal",
                "kittwm-top",
                "kittwm-browser",
                "kittwm-bar"
            ]
        );
        assert_eq!(
            contract.native_surface_by_kind("browser").unwrap().name,
            "kittwm-browser"
        );
        let chrome_surfaces = contract
            .native_surfaces_by_kind("chrome")
            .collect::<Vec<_>>();
        assert_eq!(
            chrome_surfaces
                .iter()
                .map(|surface| surface.name.as_str())
                .collect::<Vec<_>>(),
            ["kittwm-bar"]
        );
        assert_eq!(chrome_surfaces[0].composition_plane(), Some("decorations"));
        assert_eq!(chrome_surfaces[0].z_index(&contract), Some(20));
        let bar_placement = contract
            .placement_contract_for_surface("kittwm-bar")
            .unwrap();
        assert_eq!(bar_placement.surface_kind, "chrome");
        assert!(bar_placement.is_decoration());
        assert_eq!(bar_placement.z_index, 20);
        assert_eq!(
            contract
                .placement_contract_for_kind("browser")
                .unwrap()
                .surface,
            "kittwm-browser"
        );
        let placement_contracts = contract.placement_contracts();
        assert_eq!(
            placement_contracts
                .iter()
                .map(|placement| placement.surface.as_str())
                .collect::<Vec<_>>(),
            [
                "kittwm-terminal",
                "kittwm-top",
                "kittwm-browser",
                "kittwm-bar"
            ]
        );
        assert_eq!(
            placement_contracts
                .iter()
                .map(|placement| placement.role().unwrap())
                .collect::<Vec<_>>(),
            [
                SurfacePlacementRole::AppSurface,
                SurfacePlacementRole::AppSurface,
                SurfacePlacementRole::AppSurface,
                SurfacePlacementRole::Decoration
            ]
        );
        assert_eq!(contract.ready_placement_contracts(), placement_contracts);
        assert!(contract.not_ready_placement_contracts().is_empty());
        assert!(contract.missing_placement_contract_surfaces().is_empty());
        assert_eq!(
            contract
                .placement_contracts_in_composition_order()
                .iter()
                .map(|placement| (placement.surface.as_str(), placement.z_index))
                .collect::<Vec<_>>(),
            [
                ("kittwm-terminal", 0),
                ("kittwm-top", 0),
                ("kittwm-browser", 0),
                ("kittwm-bar", 20)
            ]
        );
        assert_eq!(
            contract.ready_placement_contracts_in_composition_order(),
            contract.placement_contracts_in_composition_order()
        );
        assert_eq!(
            contract
                .placement_contracts_for_role(SurfacePlacementRole::AppSurface)
                .iter()
                .map(|placement| placement.surface.as_str())
                .collect::<Vec<_>>(),
            ["kittwm-terminal", "kittwm-top", "kittwm-browser"]
        );
        assert_eq!(
            contract
                .app_surface_placement_contracts()
                .iter()
                .map(|placement| placement.surface.as_str())
                .collect::<Vec<_>>(),
            ["kittwm-terminal", "kittwm-top", "kittwm-browser"]
        );
        assert_eq!(
            contract
                .decoration_placement_contracts()
                .iter()
                .map(|placement| placement.surface.as_str())
                .collect::<Vec<_>>(),
            ["kittwm-bar"]
        );
        assert!(contract.overlay_placement_contracts().is_empty());
        let placement_coverage = contract.placement_coverage();
        assert_eq!(
            placement_coverage,
            SurfacePlacementCoverage {
                total_surfaces: 4,
                placement_contracts: 4,
                ready_placement_contracts: 4,
                app_surfaces: 3,
                decorations: 1,
                overlays: 0,
                all_native_surfaces_ready: true,
                all_placement_contracts_ready: true,
            }
        );
        assert_eq!(
            placement_coverage.count_for_role(SurfacePlacementRole::AppSurface),
            3
        );
        assert_eq!(
            placement_coverage.count_for_role(SurfacePlacementRole::Decoration),
            1
        );
        assert_eq!(
            placement_coverage.count_for_role(SurfacePlacementRole::Overlay),
            0
        );
        assert!(placement_coverage.has_role(SurfacePlacementRole::AppSurface));
        assert!(placement_coverage.has_role(SurfacePlacementRole::Decoration));
        assert!(!placement_coverage.has_role(SurfacePlacementRole::Overlay));
        assert_eq!(
            placement_coverage
                .role_breakdown()
                .iter()
                .map(|coverage| (
                    coverage.role,
                    coverage.composition_plane.as_str(),
                    coverage.count
                ))
                .collect::<Vec<_>>(),
            [
                (SurfacePlacementRole::AppSurface, "app-surfaces", 3),
                (SurfacePlacementRole::Decoration, "decorations", 1),
                (SurfacePlacementRole::Overlay, "overlays", 0)
            ]
        );
        assert_eq!(placement_coverage.missing_placement_contracts(), 0);
        assert_eq!(placement_coverage.not_ready_placement_contracts(), 0);
        assert_eq!(placement_coverage.placement_gap_count(), 0);
        assert!(placement_coverage.is_complete());
        assert!(!placement_coverage.has_gaps());
        assert_eq!(
            contract
                .native_surface_for_spec(&SurfaceSpec::terminal("htop"))
                .unwrap()
                .name,
            "kittwm-terminal"
        );
        assert_eq!(
            contract
                .native_surface_for_spec(&SurfaceSpec::top())
                .unwrap()
                .name,
            "kittwm-top"
        );
        assert_eq!(
            contract
                .native_surface_for_spec(&SurfaceSpec::browser("https://example.com"))
                .unwrap()
                .name,
            "kittwm-browser"
        );
        assert!(contract
            .native_surface_for_spec(&SurfaceSpec {
                kind: SurfaceKind::Other("canvas".to_string()),
                command: "canvas".to_string(),
                title: None,
            })
            .is_none());
        assert!(contract.native_surface("missing").is_none());
        assert!(contract.native_surface_by_kind("missing").is_none());
        assert!(contract.placement_contract_for_surface("missing").is_none());
        assert!(contract.placement_contract_for_kind("missing").is_none());
        let roundtrip: ArchitectureContract =
            serde_json::from_str(&serde_json::to_string(&contract).unwrap()).unwrap();
        assert_eq!(roundtrip, contract);
    }

    #[test]
    fn surface_spec_native_readiness_uses_architecture_contract() {
        let terminal = SurfaceSpec::terminal("htop");
        let terminal_contract = terminal.native_surface_contract().unwrap();
        assert_eq!(terminal_contract.name, "kittwm-terminal");
        assert!(terminal.is_native_ready());
        assert_eq!(terminal.composition_plane(), Some("app-surfaces"));
        assert_eq!(terminal.z_index(), Some(0));
        let terminal_placement = terminal.placement_contract().unwrap();
        assert_eq!(terminal_placement.surface, "kittwm-terminal");
        assert_eq!(terminal_placement.composition_plane, "app-surfaces");
        assert_eq!(terminal_placement.z_index, 0);
        assert!(terminal_placement.native_ready);
        assert_eq!(
            terminal_placement.role(),
            Some(SurfacePlacementRole::AppSurface)
        );
        assert_eq!(
            SurfacePlacementRole::AppSurface.plane_name(),
            "app-surfaces"
        );
        assert!(terminal_placement.is_app_surface());
        assert!(!terminal_placement.is_decoration());
        assert!(!terminal_placement.is_overlay());

        let browser = SurfaceSpec::browser("https://example.com");
        let browser_contract = browser.native_surface_contract().unwrap();
        assert_eq!(browser_contract.name, "kittwm-browser");
        assert_eq!(browser_contract.surface_kind, "browser");
        assert!(browser.is_native_ready());
        assert_eq!(browser.composition_plane(), Some("app-surfaces"));
        assert_eq!(browser.z_index(), Some(0));
        let browser_placement = browser.placement_contract().unwrap();
        assert_eq!(
            ArchitectureContract::current()
                .placement_contract_for_spec(&browser)
                .unwrap(),
            browser_placement
        );
        assert_eq!(browser_placement.surface, "kittwm-browser");
        assert_eq!(browser_placement.surface_kind, "browser");
        assert_eq!(browser_placement.sdk_entry, "SurfaceSpec::browser");
        assert_eq!(browser_placement.composition_plane, "app-surfaces");
        assert_eq!(browser_placement.z_index, 0);
        assert!(browser_placement
            .kittui_entry
            .contains("Runtime::place_png_frame_with_options"));
        assert!(browser_placement.is_app_surface());
        assert!(!browser_placement.is_decoration());
        let decoration_placement = SurfacePlacementContract {
            surface: "kittwm-bar".to_string(),
            surface_kind: "chrome".to_string(),
            sdk_entry: "Kittwm::chrome / ChromeReservationRequest".to_string(),
            sdk_backed: true,
            kitty_graphics_native: true,
            native_ready: true,
            composition_plane: "decorations".to_string(),
            z_index: 20,
            kittui_entry: "BarModel::scene -> Runtime::place_at_with_options".to_string(),
        };
        assert_eq!(
            decoration_placement.role(),
            Some(SurfacePlacementRole::Decoration)
        );
        assert_eq!(SurfacePlacementRole::Decoration.plane_name(), "decorations");
        assert!(decoration_placement.is_decoration());
        assert!(decoration_placement.is_above(&browser_placement));
        assert!(browser_placement.is_below(&decoration_placement));
        let roundtrip: SurfacePlacementContract =
            serde_json::from_str(&serde_json::to_string(&browser_placement).unwrap()).unwrap();
        assert_eq!(roundtrip, browser_placement);

        let other = SurfaceSpec {
            kind: SurfaceKind::Other("canvas".to_string()),
            command: "canvas".to_string(),
            title: None,
        };
        assert!(other.native_surface_contract().is_none());
        assert!(!other.is_native_ready());
        assert_eq!(other.composition_plane(), None);
        assert_eq!(other.z_index(), None);
        assert!(other.placement_contract().is_none());
        assert_eq!(
            SurfacePlacementRole::from_plane("overlays"),
            Some(SurfacePlacementRole::Overlay)
        );
        assert_eq!(SurfacePlacementRole::from_plane("unknown"), None);
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
        assert_eq!(
            SurfaceSpec::top().titled("top"),
            SurfaceSpec {
                kind: SurfaceKind::Terminal,
                command: "kittwm-top".to_string(),
                title: Some("top".to_string())
            }
        );
    }

    #[test]
    fn surface_spec_exposes_native_pty_command_for_dry_runs() {
        assert_eq!(
            SurfaceSpec::top().native_pty_command().unwrap(),
            "kittwm-top"
        );
        assert_eq!(
            SurfaceSpec::terminal(" htop ")
                .native_pty_command()
                .unwrap(),
            "htop"
        );
        assert_eq!(
            SurfaceSpec::browser(" https://example.com/it's ")
                .native_pty_command()
                .unwrap(),
            "kittwm-browser 'https://example.com/it'\\''s'"
        );
        assert!(SurfaceSpec::terminal("   ").native_pty_command().is_err());
        assert!(SurfaceSpec::browser("   ").native_pty_command().is_err());
        assert!(SurfaceSpec {
            kind: SurfaceKind::Other("canvas".to_string()),
            command: "canvas".to_string(),
            title: None,
        }
        .native_pty_command()
        .is_err());
    }

    #[test]
    fn browser_surface_command_quotes_targets() {
        assert_eq!(
            browser_surface_command("https://example.com/a%20b"),
            "kittwm-browser 'https://example.com/a%20b'"
        );
        let shell_quoted = shell_quote("https://example.com/it's");
        assert_eq!(shell_quoted, "'https://example.com/it'\\''s'");
        assert_eq!(shell_quoted.capacity(), shell_quoted.len());
        let quoted = browser_surface_command("https://example.com/it's");
        assert_eq!(quoted, "kittwm-browser 'https://example.com/it'\\''s'");
        assert_eq!(quoted.capacity(), quoted.len());
    }

    #[test]
    fn surface_handles_keep_client_and_id() {
        let client = Kittwm::connect_path("/tmp/kittwm-sdk.sock");
        let focused = client.focused_surface();
        assert_eq!(focused.id, "focused");
        assert_eq!(client.surface(" native-1 ").id, "native-1");
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
        let none = ClientCapabilities::none();
        assert!(none.allowed().is_empty());
        assert_eq!(none.iter().count(), 0);

        let caps = ClientCapabilities::restricted();
        assert_eq!(caps.allowed(), &[Capability::ReadText]);
        assert!(caps.allows(Capability::ReadText));
        assert!(!caps.allows(Capability::SubscribeEvents));
        assert!(!caps.allows(Capability::CreateWindow));

        let inspect = ClientCapabilities::inspect_only();
        assert!(inspect.allows(Capability::ReadText));
        assert!(inspect.allows(Capability::SubscribeEvents));
        assert!(inspect.allows(Capability::ReadSemanticTree));
        assert!(!inspect.allows(Capability::SendInput));
        assert!(!inspect.allows(Capability::RawRequest));

        let automation = ClientCapabilities::automation();
        assert!(automation.allows(Capability::ControlWindow));
        assert!(automation.allows(Capability::SendInput));
        assert!(automation.allows(Capability::ReadText));
        assert!(automation.allows(Capability::SubscribeEvents));
        assert!(automation.allows(Capability::ReadSemanticTree));
        assert!(!automation.allows(Capability::CreateWindow));
        assert!(!automation.allows(Capability::InvokeSemanticAction));

        assert!(ClientCapabilities::all().allows(Capability::SubscribeEvents));
        assert!(ClientCapabilities::all().allows(Capability::ReadSemanticTree));
        assert!(ClientCapabilities::all().allows(Capability::InvokeSemanticAction));
    }

    #[test]
    fn semantic_role_variants_serialize_to_documented_snake_case() {
        let roles = vec![
            (ComponentRole::Link, "link"),
            (ComponentRole::Heading, "heading"),
            (ComponentRole::Paragraph, "paragraph"),
            (ComponentRole::Code, "code"),
            (ComponentRole::Image, "image"),
            (ComponentRole::Canvas, "canvas"),
            (ComponentRole::Terminal, "terminal"),
            (ComponentRole::BrowserDocument, "browser_document"),
            (ComponentRole::List, "list"),
            (ComponentRole::ListItem, "list_item"),
            (ComponentRole::Tree, "tree"),
            (ComponentRole::TreeItem, "tree_item"),
            (ComponentRole::Row, "row"),
            (ComponentRole::Cell, "cell"),
        ];
        for (role, expected) in roles {
            let value = serde_json::to_value(&role).unwrap();
            assert_eq!(value, expected);
            let decoded: ComponentRole = serde_json::from_value(value).unwrap();
            assert_eq!(decoded, role);
        }
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
              "workspace": "1",
              "chrome": {"workspace":"1","top_bar_rows":1,"tilable_rows":23},
              "panes_detail": [{
                "window": "native-1",
                "title": "shell",
                "focused": true,
                "weight": 2,
                "stack_index": 3,
                "stack_top": true,
                "floating_dx": 4,
                "floating_dy": -2,
                "floating_moved": true,
                "title_draggable": true,
                "title_drag_kind": "reorder",
                "title_drag_active": true,
                "title_drag_col": 6,
                "title_drag_row": 2,
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
        assert_eq!(panes.layout_label(), Some("columns"));
        assert!(panes.is_tiled_layout());
        assert!(!panes.is_floating_layout());
        assert!(!panes.is_fullscreen_layout());
        assert_eq!(panes.workspace_id(), Some("1"));
        assert_eq!(panes.top_bar_rows(), 1);
        assert_eq!(panes.tilable_rows(), Some(23));
        assert!(panes.chrome_reservation().unwrap().is_reported());
        assert_eq!(panes.focused_pane().unwrap().window, "native-1");
        assert_eq!(panes.fullscreen_pane(), None);
        assert_eq!(panes.focused_is_fullscreen(), Some(false));
        assert_eq!(panes.topmost_pane().unwrap().window, "native-1");
        assert_eq!(panes.focused_is_topmost(), Some(true));
        assert_eq!(panes.focused_is_resized(), Some(true));
        assert_eq!(panes.focused_weight_chip_label().as_deref(), Some("wt:2"));
        assert_eq!(
            panes.focused_command_chip_label().as_deref(),
            Some("/bin/sh")
        );
        assert_eq!(panes.focused_pid_chip_label().as_deref(), Some("pid:123"));
        assert_eq!(
            panes.focused_status_chip_label().as_deref(),
            Some("/bin/sh · wt:2 · pid:123 · frame:1/4")
        );
        assert_eq!(panes.focused_is_moved(), Some(true));
        assert_eq!(panes.focused_is_title_draggable(), Some(true));
        assert_eq!(panes.focused_is_title_drag_active(), Some(true));
        assert_eq!(panes.focused_title_drag_reorders_pane(), Some(true));
        assert_eq!(panes.focused_title_drag_repositions_pane(), Some(false));
        assert_eq!(panes.focused_title_drag_cell(), Some((6, 2)));
        assert_eq!(
            panes.focused_title_drag_cells_by(5, 2),
            Some(((6, 2), (11, 4)))
        );
        assert_eq!(panes.focused_title_marker_prefix().as_deref(), Some("▶⇄↔"));
        assert_eq!(
            panes
                .focused_title_marker_prefix_with_active_drag(true)
                .as_deref(),
            Some("▶◆⇄↔")
        );
        assert_eq!(
            panes.focused_reported_title_marker_prefix().as_deref(),
            Some("▶◆⇄↔")
        );
        assert_eq!(
            panes
                .active_title_drag_pane()
                .map(|pane| pane.window.as_str()),
            Some("native-1")
        );
        assert_eq!(panes.active_title_drag_window(), Some("native-1"));
        assert_eq!(
            panes
                .active_title_drag_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-1"]
        );
        assert_eq!(panes.active_title_drag_kind(), Some("reorder"));
        assert_eq!(
            panes.active_parsed_title_drag_kind(),
            Some(TitleDragKind::Reorder)
        );
        assert_eq!(panes.active_title_drag_reorders_pane(), Some(true));
        assert_eq!(panes.active_title_drag_repositions_pane(), Some(false));
        assert_eq!(panes.active_title_drag_cell(), Some((6, 2)));
        assert_eq!(
            panes.active_title_drag_cells_by(5, 2),
            Some(((6, 2), (11, 4)))
        );
        assert_eq!(
            panes
                .focused_pane()
                .unwrap()
                .reported_title_marker_prefix(Some("columns")),
            "▶◆⇄↔"
        );
        assert_eq!(panes.focused_frame_is_clean(), Some(false));
        assert_eq!(panes.focused_frame_upload_skipped(), Some(false));
        assert_eq!(panes.focused_frame_status_label().as_deref(), Some("1/4"));
        assert_eq!(
            panes.focused_frame_chip_label().as_deref(),
            Some("frame:1/4")
        );
        assert_eq!(panes.focused_frame_changed_tiles_ratio(), Some((1, 4)));
        assert_eq!(
            panes.focused_frame_changed_tiles_label().as_deref(),
            Some("1/4")
        );
        assert_eq!(panes.focused_frame_changed_fraction(), Some(0.25));
        assert_eq!(panes.focused_frame_changed_percent(), Some(25.0));
        assert_eq!(panes.clean_frame_panes().count(), 0);
        assert_eq!(
            panes
                .dirty_frame_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-1"]
        );
        assert_eq!(panes.title_draggable_panes().count(), 1);
        assert_eq!(panes.title_reorder_draggable_panes().count(), 1);
        assert_eq!(panes.title_reposition_draggable_panes().count(), 0);
        assert_eq!(
            panes
                .moved_floating_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-1"]
        );
        let pane = &panes.panes_detail[0];
        assert_eq!(pane.cursor_col, Some(4));
        assert_eq!(pane.mouse_sgr, Some(true));
        assert_eq!(pane.stack_index(), Some(3));
        assert!(pane.is_stack_top());
        assert!(pane.has_non_default_weight());
        assert!(pane.is_resized());
        assert_eq!(pane.weight_chip_label().as_deref(), Some("wt:2"));
        assert_eq!(pane.command_chip_label(), "/bin/sh");
        assert_eq!(pane.pid_chip_label(), "pid:123");
        assert_eq!(
            pane.status_chip_label(),
            "/bin/sh · wt:2 · pid:123 · frame:1/4"
        );
        assert_eq!(pane.floating_offset(), Some((4, -2)));
        assert_eq!(pane.floating_moved, Some(true));
        assert!(pane.has_floating_offset());
        assert!(pane.is_title_draggable());
        assert!(pane.is_title_drag_active());
        assert_eq!(pane.title_drag_kind(), Some("reorder"));
        assert_eq!(pane.parsed_title_drag_kind(), Some(TitleDragKind::Reorder));
        assert!(pane.title_drag_reorders_pane());
        assert!(!pane.title_drag_repositions_pane());
        assert_eq!(pane.title_drag_cell(), Some((6, 2)));
        assert_eq!(pane.title_drag_cells_by(5, 2), Some(((6, 2), (11, 4))));
        assert_eq!(pane.title_marker_prefix(Some("columns")), "▶⇄↔");
        assert_eq!(
            pane.title_marker_prefix_with_active_drag(Some("columns"), true),
            "▶◆⇄↔"
        );
        assert_eq!(pane.bounds(), Some((0, 0, 80, 24)));
        assert_eq!(pane.app_bounds(), Some((0, 1, 80, 23)));
        assert_eq!(pane.cursor_position(), Some((4, 5)));
        assert!(pane.is_cursor_visible());
        assert!(pane.has_bracketed_paste());
        assert!(!pane.has_application_cursor_keys());
        assert!(pane.has_mouse_reporting());
        assert!(!pane.has_mouse_button_motion());
        assert!(!pane.has_mouse_all_motion());
        assert!(pane.has_mouse_sgr());
        assert!(pane.has_dirty_frame());
        assert_eq!(pane.frame_status_label(), "1/4");
        assert_eq!(pane.frame_chip_label(), "frame:1/4");
        assert_eq!(pane.frame_changed_tiles_ratio(), Some((1, 4)));
        assert_eq!(pane.frame_changed_tiles_label().as_deref(), Some("1/4"));
        assert_eq!(pane.frame_changed_fraction(), Some(0.25));
        assert_eq!(pane.frame_changed_percent(), Some(25.0));
        assert_eq!(pane.frame_upload_skipped(), Some(false));
        assert_eq!(pane.is_frame_clean(), Some(false));
        let frame = pane.dirty_frame.as_ref().unwrap();
        assert!(!frame.is_clean());
        assert_eq!(frame.changed_tiles_ratio(), (1, 4));
        assert_eq!(frame.changed_tiles_label(), "1/4");
        assert_eq!(frame.status_label(), "1/4");
        assert_eq!(frame.chip_label(), "frame:1/4");
        assert_eq!(frame.changed_percent(), 25.0);
        assert!(pane.has_transport_diagnostics());
        assert_eq!(pane.dirty_frame.as_ref().unwrap().changed_fraction, 0.25);
        assert_eq!(pane.transport.as_ref().unwrap()["selected"], "file");
    }

    #[test]
    fn native_pane_detail_default_is_fixture_friendly() {
        let pane = NativePaneDetail::default();
        assert_eq!(pane.window, "");
        assert_eq!(pane.title, "");
        assert!(!pane.focused);
        assert_eq!(pane.weight, 0);
        assert!(!pane.has_non_default_weight());
        assert!(!pane.is_resized());
        assert_eq!(pane.weight_chip_label(), None);
        assert_eq!(pane.command_chip_label(), "-");
        assert_eq!(pane.pid_chip_label(), "pid:-");
        assert_eq!(pane.status_chip_label(), "- · pid:- · frame:new");
        assert_eq!(pane.bounds(), None);
        assert_eq!(pane.app_bounds(), None);
        assert_eq!(pane.floating_offset(), None);
        assert!(!pane.has_floating_offset());
        assert_eq!(pane.frame_status_label(), "new");
        assert_eq!(pane.frame_chip_label(), "frame:new");
        assert_eq!(pane.frame_changed_tiles_ratio(), None);
        assert_eq!(pane.frame_changed_tiles_label(), None);
        assert_eq!(pane.frame_changed_fraction(), None);
        assert_eq!(pane.frame_changed_percent(), None);
        assert_eq!(pane.frame_upload_skipped(), None);
        assert_eq!(pane.is_frame_clean(), None);
        assert_eq!(pane.title_drag_cell(), None);
        assert_eq!(pane.title_marker_prefix(Some("floating")), " ");
    }

    #[test]
    fn native_pane_title_drag_cell_requires_affordance_and_geometry() {
        let base = NativePaneDetail {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            title_draggable: Some(true),
            x: Some(10),
            y: Some(4),
            cols: Some(20),
            rows: Some(6),
            ..NativePaneDetail::default()
        };
        assert_eq!(base.title_drag_cell(), Some((14, 5)));
        assert_eq!(base.title_drag_cells_by(7, -3), Some(((14, 5), (21, 2))));
        let mut reported = base.clone();
        reported.title_drag_col = Some(12);
        reported.title_drag_row = Some(9);
        assert_eq!(reported.title_drag_cell(), Some((12, 9)));
        assert_eq!(
            reported.title_drag_cells_by(2, -4),
            Some(((12, 9), (14, 5)))
        );
        assert_eq!(base.title_drag_cells_by(-99, -99), Some(((14, 5), (1, 1))));
        assert_eq!(
            base.title_drag_cells_by(i32::MAX, i32::MAX),
            Some(((14, 5), (u16::MAX, u16::MAX)))
        );

        let mut tiny = base.clone();
        tiny.cols = Some(1);
        assert_eq!(tiny.title_drag_cell(), Some((11, 5)));

        let mut unknown_kind = base.clone();
        unknown_kind.title_drag_kind = Some("dock".to_string());
        assert_eq!(unknown_kind.title_drag_kind(), Some("dock"));
        assert_eq!(
            unknown_kind.parsed_title_drag_kind(),
            Some(TitleDragKind::Unknown("dock".to_string()))
        );
        assert_eq!(
            unknown_kind.parsed_title_drag_kind().unwrap().as_str(),
            "dock"
        );
        assert!(!unknown_kind.title_drag_reorders_pane());
        assert!(!unknown_kind.title_drag_repositions_pane());

        let mut mixed_case_kind = base.clone();
        mixed_case_kind.title_drag_kind = Some("RePosition".to_string());
        assert!(!matches!(
            mixed_case_kind.parsed_title_drag_kind(),
            Some(TitleDragKind::Reposition)
        ));
        assert!(!mixed_case_kind.title_drag_reorders_pane());
        assert!(mixed_case_kind.title_drag_repositions_pane());

        let mut not_draggable = base.clone();
        not_draggable.title_draggable = Some(false);
        assert_eq!(not_draggable.title_drag_cell(), None);

        let mut missing_geometry = base;
        missing_geometry.x = None;
        assert_eq!(missing_geometry.title_drag_cell(), None);
    }

    #[test]
    fn status_decodes_without_optional_pane_details() {
        let status: Status =
            serde_json::from_str(r#"{"pending":0,"panes":1,"focus":"native-1","layout":"rows"}"#)
                .unwrap();
        assert_eq!(status.focus.as_deref(), Some("native-1"));
        assert_eq!(status.layout_label(), Some("rows"));
        assert!(status.is_tiled_layout());
        assert!(!status.is_floating_layout());
        assert!(!status.is_fullscreen_layout());
        assert_eq!(status.workspace_id(), None);
        assert_eq!(status.top_bar_rows(), 0);
        assert_eq!(status.tilable_rows(), None);
        assert!(status.chrome_reservation().is_none());
        assert!(status.focused_pane.is_none());
        assert!(status.focused_pane().is_none());
        assert_eq!(status.fullscreen_pane(), None);
        assert_eq!(status.focused_is_fullscreen(), None);
        assert!(status.topmost_pane().is_none());
        assert_eq!(status.focused_is_topmost(), None);
        assert_eq!(status.focused_is_resized(), None);
        assert_eq!(status.focused_weight_chip_label(), None);
        assert_eq!(status.focused_command_chip_label(), None);
        assert_eq!(status.focused_pid_chip_label(), None);
        assert_eq!(status.focused_status_chip_label(), None);
        assert_eq!(status.focused_is_moved(), None);
        assert_eq!(status.focused_is_title_draggable(), None);
        assert_eq!(status.focused_is_title_drag_active(), None);
        assert_eq!(status.focused_title_drag_reorders_pane(), None);
        assert_eq!(status.focused_title_drag_repositions_pane(), None);
        assert_eq!(status.focused_title_drag_cell(), None);
        assert_eq!(status.focused_title_drag_cells_by(1, 1), None);
        assert_eq!(status.focused_title_marker_prefix(), None);
        assert_eq!(status.focused_reported_title_marker_prefix(), None);
        assert!(status.active_title_drag_pane().is_none());
        assert_eq!(status.active_title_drag_window(), None);
        assert_eq!(status.active_title_drag_kind(), None);
        assert_eq!(status.active_parsed_title_drag_kind(), None);
        assert_eq!(status.active_title_drag_reorders_pane(), None);
        assert_eq!(status.active_title_drag_repositions_pane(), None);
        assert_eq!(status.active_title_drag_cell(), None);
        assert_eq!(status.active_title_drag_cells_by(1, 1), None);
        assert_eq!(status.focused_frame_is_clean(), None);
        assert_eq!(status.focused_frame_upload_skipped(), None);
        assert_eq!(status.focused_frame_status_label(), None);
        assert_eq!(status.focused_frame_chip_label(), None);
        assert_eq!(status.focused_frame_changed_tiles_ratio(), None);
        assert_eq!(status.focused_frame_changed_tiles_label(), None);
        assert_eq!(status.focused_frame_changed_fraction(), None);
        assert_eq!(status.focused_frame_changed_percent(), None);
        assert_eq!(status.title_draggable_panes().count(), 0);
        assert_eq!(status.title_reorder_draggable_panes().count(), 0);
        assert_eq!(status.title_reposition_draggable_panes().count(), 0);
        assert!(status.panes_detail.is_empty());
    }

    #[test]
    fn status_layout_state_helpers_parse_window_manager_modes() {
        let floating: Status = serde_json::from_str(
            r#"{"pending":0,"panes":2,"focus":"native-1","layout":" floating "}"#,
        )
        .unwrap();
        assert_eq!(floating.layout_label(), Some("floating"));
        assert!(floating.is_floating_layout());
        assert!(!floating.is_fullscreen_layout());
        assert!(!floating.is_tiled_layout());

        for layout in ["columns", "Rows", " grid ", "COLUMNS"] {
            let status: Status = serde_json::from_str(&format!(
                r#"{{"pending":0,"panes":1,"focus":"native-1","layout":"{layout}"}}"#
            ))
            .unwrap();
            assert!(status.is_tiled_layout(), "{layout}");
        }

        let fullscreen: PanesStatus = serde_json::from_str(
            r#"{"panes":1,"focus":"native-1","layout":"fullscreen","panes_detail":[{"window":"native-1","title":"shell","focused":true,"weight":1}]}"#,
        )
        .unwrap();
        assert!(fullscreen.is_fullscreen_layout());
        assert!(!fullscreen.is_floating_layout());
        assert!(!fullscreen.is_tiled_layout());
        assert_eq!(fullscreen.fullscreen_pane().unwrap().window, "native-1");
        assert_eq!(fullscreen.focused_is_fullscreen(), Some(true));
        assert_eq!(
            fullscreen.focused_title_marker_prefix().as_deref(),
            Some("▶▣")
        );
    }

    #[test]
    fn status_pane_state_accessors_use_details_and_fallbacks() {
        let status: Status = serde_json::from_str(
            r#"{
              "pending": 0,
              "panes": 2,
              "focus": "native-1",
              "layout": "floating",
              "panes_detail": [
                {"window":"native-1","title":"shell","focused":false,"weight":1,"stack_index":0,"stack_top":false,"title_draggable":true,"title_drag_kind":"reposition","title_drag_col":4,"title_drag_row":1,"floating_dx":0,"floating_dy":0,"floating_moved":false,"dirty_frame":{"changed_tiles":0,"total_tiles":4,"changed_fraction":0.0,"skipped_upload":true}},
                {"window":"native-2","title":"editor","focused":false,"weight":3,"stack_index":1,"title_draggable":true,"title_drag_kind":"reposition","floating_dx":0,"floating_dy":0,"floating_moved":true,"dirty_frame":{"changed_tiles":2,"total_tiles":4,"changed_fraction":0.5,"skipped_upload":false}}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(status.focused_pane().unwrap().window, "native-1");
        assert_eq!(status.fullscreen_pane(), None);
        assert_eq!(status.focused_is_fullscreen(), Some(false));
        assert_eq!(status.topmost_pane().unwrap().window, "native-2");
        assert_eq!(status.focused_is_topmost(), Some(false));
        assert_eq!(status.focused_is_resized(), Some(false));
        assert_eq!(status.focused_weight_chip_label(), None);
        assert_eq!(status.focused_command_chip_label().as_deref(), Some("-"));
        assert_eq!(status.focused_pid_chip_label().as_deref(), Some("pid:-"));
        assert_eq!(
            status.focused_status_chip_label().as_deref(),
            Some("- · pid:- · frame:clean")
        );
        assert_eq!(status.focused_is_moved(), Some(false));
        assert_eq!(status.focused_is_title_draggable(), Some(true));
        assert_eq!(status.focused_is_title_drag_active(), Some(false));
        assert_eq!(status.focused_title_drag_reorders_pane(), Some(false));
        assert_eq!(status.focused_title_drag_repositions_pane(), Some(true));
        assert_eq!(status.focused_title_drag_cell(), Some((4, 1)));
        assert_eq!(
            status.focused_title_drag_cells_by(3, 2),
            Some(((4, 1), (7, 3)))
        );
        assert_eq!(
            status.focused_title_marker_prefix().as_deref(),
            Some("▶≡  ")
        );
        assert_eq!(
            status
                .focused_title_marker_prefix_with_active_drag(true)
                .as_deref(),
            Some("▶◆≡  ")
        );
        assert_eq!(
            status.focused_reported_title_marker_prefix().as_deref(),
            Some("▶≡  ")
        );
        assert!(status.active_title_drag_pane().is_none());
        assert_eq!(status.active_title_drag_window(), None);
        assert_eq!(status.active_title_drag_kind(), None);
        assert_eq!(status.active_parsed_title_drag_kind(), None);
        assert_eq!(status.active_title_drag_reorders_pane(), None);
        assert_eq!(status.active_title_drag_repositions_pane(), None);
        assert_eq!(status.active_title_drag_cell(), None);
        assert_eq!(status.active_title_drag_cells_by(1, 1), None);
        assert_eq!(status.focused_frame_is_clean(), Some(true));
        assert_eq!(status.focused_frame_upload_skipped(), Some(true));
        assert_eq!(
            status.focused_frame_status_label().as_deref(),
            Some("clean")
        );
        assert_eq!(
            status.focused_frame_chip_label().as_deref(),
            Some("frame:clean")
        );
        assert_eq!(status.focused_frame_changed_tiles_ratio(), Some((0, 4)));
        assert_eq!(
            status.focused_frame_changed_tiles_label().as_deref(),
            Some("0/4")
        );
        assert_eq!(status.focused_frame_changed_fraction(), Some(0.0));
        assert_eq!(status.focused_frame_changed_percent(), Some(0.0));
        assert_eq!(
            status
                .title_draggable_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-1", "native-2"]
        );
        assert_eq!(status.title_reorder_draggable_panes().count(), 0);
        assert_eq!(
            status
                .title_reposition_draggable_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-1", "native-2"]
        );
        assert_eq!(
            status
                .moved_floating_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-2"]
        );
        assert_eq!(
            status
                .resized_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-2"]
        );
        assert_eq!(
            status
                .clean_frame_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-1"]
        );
        assert_eq!(
            status
                .dirty_frame_panes()
                .map(|pane| pane.window.as_str())
                .collect::<Vec<_>>(),
            vec!["native-2"]
        );
    }

    #[test]
    fn status_decodes_chrome_reservation_metadata() {
        let status: Status = serde_json::from_str(
            r#"{
              "pending": 0,
              "panes": 0,
              "focus": null,
              "layout": "columns",
              "workspace": "1",
              "chrome": {"workspace":"1","top_bar_rows":1,"tilable_rows":23}
            }"#,
        )
        .unwrap();
        assert_eq!(status.workspace_id(), Some("1"));
        assert_eq!(status.top_bar_rows(), 1);
        assert_eq!(status.tilable_rows(), Some(23));
        let chrome = status.chrome_reservation().unwrap();
        assert_eq!(chrome.workspace.as_deref(), Some("1"));
        assert_eq!(chrome.top_bar_rows_or_zero(), 1);
        assert_eq!(chrome.tilable_rows(), Some(23));
    }

    #[test]
    fn status_workspace_id_trims_and_falls_back_to_chrome() {
        let status: Status = serde_json::from_str(
            r#"{
              "pending": 0,
              "panes": 0,
              "focus": null,
              "layout": "columns",
              "workspace": "   ",
              "chrome": {"workspace":" dev ","top_bar_rows":1}
            }"#,
        )
        .unwrap();
        assert_eq!(status.workspace_id(), Some("dev"));

        let panes: PanesStatus = serde_json::from_str(
            r#"{
              "panes": 0,
              "focus": "-",
              "layout": "columns",
              "workspace": "   ",
              "chrome": {"workspace":" ops ","top_bar_rows":1}
            }"#,
        )
        .unwrap();
        assert_eq!(panes.workspace_id(), Some("ops"));
    }

    #[test]
    fn session_manifest_decodes_current_json_shape() {
        let session: SessionManifest = serde_json::from_str(
            r#"{
              "schema_version": 1,
              "kind": "kittwm-native-session",
              "layout": "columns",
              "focus": "native-2",
              "panes": [
                {"index":0,"window":"native-1","title":"shell","command":"bash","weight":1,"focused":false,"floating_dx":0,"floating_dy":0},
                {"index":1,"window":"native-2","title":null,"command":"htop","weight":2,"focused":true,"floating_dx":5,"floating_dy":-3}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(session.schema_version, Some(1));
        assert_eq!(session.kind.as_deref(), Some("kittwm-native-session"));
        assert_eq!(session.layout, "columns");
        assert_eq!(session.focus, "native-2");
        assert_eq!(session.panes.len(), 2);
        assert_eq!(session.panes[0].title.as_deref(), Some("shell"));
        assert_eq!(session.panes[1].weight, 2);
        assert!(session.panes[1].focused);
        assert_eq!(session.panes[1].floating_dx, Some(5));
        assert_eq!(session.panes[1].floating_dy, Some(-3));
    }

    #[test]
    fn session_pane_restore_defaults_weight_and_focus() {
        let pane: SessionPane = serde_json::from_str(r#"{"command":"bash"}"#).unwrap();
        assert_eq!(pane.weight, 1);
        assert!(!pane.focused);
        assert_eq!(pane.floating_dx, None);
        assert_eq!(pane.floating_dy, None);
        assert!(pane.title.is_none());
    }

    #[test]
    fn clipboard_status_decodes_policy_shapes() {
        let denied: ClipboardStatus = serde_json::from_str(
            r#"{"allowed":false,"available":false,"policy":"set KITTWM_CLIPBOARD_READ=allow"}"#,
        )
        .unwrap();
        assert!(!denied.allowed);
        assert!(!denied.available);
        assert!(!denied.has_payload());
        assert!(denied.payload_base64.is_none());

        let empty: ClipboardStatus =
            serde_json::from_str(r#"{"allowed":true,"available":false,"source":"osc52-cache"}"#)
                .unwrap();
        assert!(empty.allowed);
        assert!(!empty.available);
        assert!(!empty.has_payload());

        let cached: ClipboardStatus = serde_json::from_str(
            r#"{"allowed":true,"available":true,"source_window":"native-1","selection":"clipboard","payload_base64":"aGVsbG8=","payload_bytes":5,"at_ms":123,"seq":9,"source":"osc52-cache"}"#,
        )
        .unwrap();
        assert!(cached.has_payload());
        assert_eq!(cached.source_window.as_deref(), Some("native-1"));
        assert_eq!(cached.selection.as_deref(), Some("clipboard"));
        assert_eq!(cached.payload_bytes, Some(5));
    }

    #[test]
    fn clipboard_capability_denies_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        assert!(matches!(
            client.clipboard(),
            Err(Error::CapabilityDenied(Capability::Clipboard))
        ));
    }

    #[test]
    fn help_catalog_decodes_json_shape() {
        let catalog: HelpCatalog = serde_json::from_str(
            r#"{"commands":[{"command":"STATUS_JSON","category":"status","description":"typed status"},{"command":"HELP_JSON","category":"help","description":"catalog"}]}"#,
        )
        .unwrap();
        assert_eq!(catalog.commands.len(), 2);
        assert_eq!(catalog.commands[0].command, "STATUS_JSON");
        assert_eq!(catalog.commands[0].category, "status");
        assert_eq!(catalog.commands[1].description, "catalog");
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
        assert_eq!(
            parse_app_launch_reply(
                "APPS_LAUNCH_FIRST kind=macos_app pid=2345 name=Visual Studio Code\n"
            )
            .unwrap(),
            AppLaunch {
                pid: 2345,
                candidate: AppCandidate {
                    kind: "macos_app".to_string(),
                    name: "Visual Studio Code".to_string(),
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

        let resized = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":8,"kind":"pane_resized","window":"native-1","detail":{"old":{"bounds":{"x":0,"y":0,"cols":80,"rows":24},"app_bounds":{"x":0,"y":1,"cols":80,"rows":23}},"new":{"bounds":{"x":0,"y":0,"cols":100,"rows":30},"app_bounds":{"x":0,"y":1,"cols":100,"rows":29}}}}"#,
        )
        .unwrap();
        assert_eq!(resized.kind(), "pane_resized");
        match resized {
            KittwmEvent::PaneResized(envelope) => {
                assert_eq!(envelope.seq, Some(8));
                assert_eq!(envelope.window.as_deref(), Some("native-1"));
                assert_eq!(envelope.detail["old"]["bounds"]["cols"], 80);
                assert_eq!(envelope.detail["new"]["app_bounds"]["rows"], 29);
                assert_eq!(envelope.pane_bounds("old"), Some((0, 0, 80, 24)));
                assert_eq!(envelope.pane_app_bounds("old"), Some((0, 1, 80, 23)));
                assert_eq!(envelope.pane_bounds("new"), Some((0, 0, 100, 30)));
                assert_eq!(envelope.pane_app_bounds("new"), Some((0, 1, 100, 29)));
                assert_eq!(envelope.pane_size_delta("old", "new"), Some((20, 6)));
                assert_eq!(envelope.pane_app_size_delta("old", "new"), Some((20, 6)));
                assert_eq!(envelope.pane_bounds("missing"), None);
                assert_eq!(envelope.pane_size_delta("old", "missing"), None);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let resized_flat = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":9,"kind":"pane_resized","window":"native-1","detail":{"old":{"x":0,"y":0,"cols":80,"rows":24,"app_x":0,"app_y":1,"app_cols":80,"app_rows":23}}}"#,
        )
        .unwrap();
        match resized_flat {
            KittwmEvent::PaneResized(envelope) => {
                assert_eq!(envelope.pane_bounds("old"), Some((0, 0, 80, 24)));
                assert_eq!(envelope.pane_app_bounds("old"), Some((0, 1, 80, 23)));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let pane_changed = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":10,"kind":"pane_changed","window":"native-1","detail":{"pane":{"window":"native-1","title":"shell","focused":true,"weight":1,"title_draggable":true,"title_drag_kind":"reorder","title_drag_active":true,"title_drag_col":2,"title_drag_row":1}}}"#,
        )
        .unwrap();
        assert_eq!(pane_changed.kind(), "pane_changed");
        match pane_changed {
            KittwmEvent::PaneChanged(envelope) => {
                assert_eq!(envelope.seq, Some(10));
                let pane = envelope.pane_detail().unwrap();
                assert_eq!(pane.window, "native-1");
                assert!(pane.is_title_drag_active());
                assert_eq!(pane.title_drag_kind(), Some("reorder"));
                assert_eq!(pane.title_drag_cell(), Some((2, 1)));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let input = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":11,"kind":"pane_input_sent","window":"native-1","detail":{"source":"socket","method":"send_key","key":"enter","bytes":1,"sensitive":false}}"#,
        )
        .unwrap();
        assert_eq!(input.kind(), "pane_input_sent");
        match input {
            KittwmEvent::PaneInputSent(envelope) => {
                assert_eq!(envelope.seq, Some(11));
                assert_eq!(envelope.window.as_deref(), Some("native-1"));
                assert_eq!(envelope.detail["source"], "socket");
                assert_eq!(envelope.detail["method"], "send_key");
                assert_eq!(envelope.detail["sensitive"], false);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let frame = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":12,"kind":"pane_frame_presented","window":"native-1","detail":{"renderer":"kitty","format":"rgba","pixel_width":640,"pixel_height":384,"app_bounds":{"x":0,"y":1,"cols":80,"rows":23},"uploaded":true,"skipped_upload":false,"changed_tiles":3,"total_tiles":12,"upload_bytes":4096,"placement_bytes":64,"embed_bytes":512,"elapsed_us":321}}"#,
        )
        .unwrap();
        assert_eq!(frame.kind(), "pane_frame_presented");
        match frame {
            KittwmEvent::PaneFramePresented(envelope) => {
                assert_eq!(envelope.seq, Some(12));
                assert_eq!(envelope.window.as_deref(), Some("native-1"));
                assert_eq!(envelope.detail["renderer"], "kitty");
                assert_eq!(envelope.detail["format"], "rgba");
                assert_eq!(envelope.frame_renderer(), Some("kitty"));
                assert_eq!(envelope.frame_format(), Some("rgba"));
                assert_eq!(envelope.frame_pixel_size(), Some((640, 384)));
                assert_eq!(envelope.frame_app_bounds(), Some((0, 1, 80, 23)));
                assert_eq!(envelope.frame_uploaded(), Some(true));
                assert_eq!(envelope.frame_upload_skipped(), Some(false));
                assert_eq!(envelope.frame_changed_tiles_ratio(), Some((3, 12)));
                assert_eq!(
                    envelope.frame_changed_tiles_label().as_deref(),
                    Some("3/12")
                );
                assert_eq!(envelope.frame_changed_fraction(), Some(0.25));
                assert_eq!(envelope.frame_changed_percent(), Some(25.0));
                assert_eq!(envelope.frame_is_clean(), Some(false));
                assert_eq!(envelope.frame_status_label().as_deref(), Some("3/12"));
                assert_eq!(envelope.frame_chip_label().as_deref(), Some("frame:3/12"));
                assert_eq!(envelope.frame_upload_bytes(), Some(4096));
                assert_eq!(envelope.frame_placement_bytes(), Some(64));
                assert_eq!(envelope.frame_embed_bytes(), Some(512));
                assert_eq!(envelope.frame_total_transport_bytes(), Some(4672));
                assert_eq!(envelope.frame_elapsed_us(), Some(321));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let semantic = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":13,"kind":"semantic_value_changed","window":"native-1","detail":{"component":"settings.name","revision":3,"value":"Grace"}}"#,
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
            ("surface_title_changed", "surface_title_changed"),
            ("surface_bell", "surface_bell"),
            ("surface_clipboard_set", "surface_clipboard_set"),
            ("surface_notification", "surface_notification"),
        ] {
            let line = event_parser_test_event_line(kind);
            let event = KittwmEvent::parse_line(&line).unwrap();
            assert_eq!(event.kind(), expected);
        }

        let line = event_parser_test_event_line("surface_bell");
        assert_eq!(
            line,
            r#"{"kind":"surface_bell","window":"native-1","detail":{}}"#
        );
        assert!(line.capacity() >= line.len());

        let title = KittwmEvent::parse_line(
            r#"{"kind":"surface_title_changed","window":"native-1","detail":{"title":"editor"}}"#,
        )
        .unwrap();
        match title {
            KittwmEvent::SurfaceTitleChanged(envelope) => {
                assert_eq!(envelope.window.as_deref(), Some("native-1"));
                assert_eq!(envelope.detail["title"], "editor");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let bell = KittwmEvent::parse_line(
            r#"{"kind":"surface_bell","window":"native-1","detail":{"visual":true,"audible":false}}"#,
        )
        .unwrap();
        match bell {
            KittwmEvent::SurfaceBell(envelope) => {
                assert_eq!(envelope.detail["visual"], true);
                assert_eq!(envelope.detail["audible"], false);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let clipboard = KittwmEvent::parse_line(
            r#"{"kind":"surface_clipboard_set","window":"native-1","detail":{"selection":"c","payload_base64":"aGVsbG8="}}"#,
        )
        .unwrap();
        match clipboard {
            KittwmEvent::SurfaceClipboardSet(envelope) => {
                assert_eq!(envelope.detail["selection"], "c");
                assert_eq!(envelope.detail["payload_base64"], "aGVsbG8=");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let notification = KittwmEvent::parse_line(
            r#"{"kind":"surface_notification","window":"native-1","detail":{"title":"build","body":"done"}}"#,
        )
        .unwrap();
        match notification {
            KittwmEvent::SurfaceNotification(envelope) => {
                assert_eq!(envelope.detail["title"], "build");
                assert_eq!(envelope.detail["body"], "done");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let unknown =
            KittwmEvent::parse_line(r#"{"kind":"new_future_event","detail":{"x":1}}"#).unwrap();
        assert_eq!(unknown.kind(), "new_future_event");
        assert!(matches!(unknown, KittwmEvent::Unknown { .. }));
        assert!(unknown.envelope().is_none());
        assert_eq!(unknown.unknown_raw().unwrap()["detail"]["x"], 1);
    }

    #[test]
    fn event_envelope_accessors_expose_common_details() {
        let event = KittwmEvent::parse_line(
            r#"{"schema_version":1,"seq":9,"kind":"surface_bell","window":"native-1","detail":{"visual":true,"audible":false,"bytes":12,"delta":-3,"title":"bell"}}"#,
        )
        .unwrap();
        let envelope = event.envelope().unwrap();
        assert_eq!(event.kind(), "surface_bell");
        assert_eq!(envelope.seq, Some(9));
        assert_eq!(envelope.window.as_deref(), Some("native-1"));
        assert_eq!(envelope.detail_str("title"), Some("bell"));
        assert_eq!(envelope.detail_bool("visual"), Some(true));
        assert_eq!(envelope.detail_bool("audible"), Some(false));
        assert_eq!(envelope.detail_u64("bytes"), Some(12));
        assert_eq!(envelope.detail_i64("bytes"), Some(12));
        assert_eq!(envelope.detail_i64("delta"), Some(-3));
        assert_eq!(envelope.detail_u64("delta"), None);
        assert_eq!(envelope.detail_str("missing"), None);
    }

    #[test]
    fn event_capability_denies_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        assert!(matches!(
            client.events_ms(100),
            Err(Error::CapabilityDenied(Capability::SubscribeEvents))
        ));
        assert!(matches!(
            client.events_iter_ms(100),
            Err(Error::CapabilityDenied(Capability::SubscribeEvents))
        ));
    }

    #[test]
    fn event_iter_wraps_bounded_event_batches() {
        let events = vec![
            KittwmEvent::Status(EventEnvelope::default()),
            KittwmEvent::LayoutChanged(EventEnvelope {
                window: Some("native-1".to_string()),
                ..EventEnvelope::default()
            }),
        ];
        let mut iter = KittwmEventIter::from(events);
        assert_eq!(iter.len(), 2);
        assert_eq!(iter.next().unwrap().kind(), "status");
        assert_eq!(iter.next().unwrap().kind(), "layout_changed");
        assert_eq!(iter.next(), None);
    }

    fn event_parser_test_event_line(kind: &str) -> String {
        let mut line = String::with_capacity(
            r#"{"kind":"","window":"native-1","detail":{}}"#.len() + kind.len(),
        );
        line.push_str(r#"{"kind":""#);
        line.push_str(kind);
        line.push_str(r#"","window":"native-1","detail":{}}"#);
        line
    }

    #[cfg(unix)]
    #[test]
    fn chrome_helper_sends_expected_socket_command() {
        let path = PathBuf::from(test_socket_path("kwchrome", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            stream
                .write_all(b"{\"workspace\":\"dev\",\"top_bar_rows\":1,\"tilable_rows\":23}\n")
                .unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let chrome = client.chrome_json().unwrap();
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, "CHROME_JSON");
        assert_eq!(chrome.workspace.as_deref(), Some("dev"));
        assert_eq!(chrome.top_bar_rows_or_zero(), 1);
        assert_eq!(chrome.tilable_rows(), Some(23));
    }

    #[test]
    fn json_payload_request_builds_directly() {
        assert_eq!(
            json_payload_request("RESERVE_CHROME_JSON", "{\"top_bar_rows\":2}"),
            "RESERVE_CHROME_JSON {\"top_bar_rows\":2}"
        );
        assert_eq!(
            json_payload_request("RESTORE_SESSION_JSON", "{\"panes\":[]}"),
            "RESTORE_SESSION_JSON {\"panes\":[]}"
        );
    }

    fn test_socket_path(prefix: &str, pid: u32) -> String {
        let suffix = now_test_nanos() % 1_000_000;
        let mut path =
            String::with_capacity("/tmp/-".len() + prefix.len() + 20 + 1 + 6 + ".sock".len());
        path.push_str("/tmp/");
        path.push_str(prefix);
        path.push('-');
        let _ = write!(path, "{pid}");
        path.push('-');
        let _ = write!(path, "{suffix}");
        path.push_str(".sock");
        path
    }

    #[test]
    fn test_socket_path_builds_directly() {
        let path = test_socket_path("kwchrome", 1234);
        assert!(path.starts_with("/tmp/kwchrome-1234-"), "{path}");
        let reserve = test_socket_path("kwreserve", 5678);
        assert!(reserve.starts_with("/tmp/kwreserve-5678-"), "{reserve}");
        let clip = test_socket_path("kwclip", 9012);
        assert!(clip.starts_with("/tmp/kwclip-9012-"), "{clip}");
        let shortcuts = test_socket_path("kwshortcuts", 3456);
        assert!(
            shortcuts.starts_with("/tmp/kwshortcuts-3456-"),
            "{shortcuts}"
        );
        let help = test_socket_path("kwhc", 7890);
        assert!(help.starts_with("/tmp/kwhc-7890-"), "{help}");
        let replace = test_socket_path("kwreplace", 2468);
        assert!(replace.starts_with("/tmp/kwreplace-2468-"), "{replace}");
        let browser = test_socket_path("kwb", 1357);
        assert!(browser.starts_with("/tmp/kwb-1357-"), "{browser}");
        let session = test_socket_path("kwsession", 9753);
        assert!(session.starts_with("/tmp/kwsession-9753-"), "{session}");
        let events = test_socket_path("kwe", 8642);
        assert!(events.starts_with("/tmp/kwe-8642-"), "{events}");
        let events_iter = test_socket_path("kwei", 7531);
        assert!(events_iter.starts_with("/tmp/kwei-7531-"), "{events_iter}");
        assert!(path.ends_with(".sock"), "{path}");
        assert!(path.capacity() >= path.len());
    }

    #[cfg(unix)]
    #[test]
    fn reserve_chrome_sends_typed_drawable_reservation_request() {
        let path = PathBuf::from(test_socket_path("kwreserve", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            stream.write_all(b"CHROME_RESERVED {}\n").unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let request = ChromeReservationRequest::top_bar(2)
            .gaps(1, 1)
            .owner(" bar ");
        client.reserve_chrome(&request).unwrap();
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(seen.starts_with("RESERVE_CHROME_JSON "), "{seen}");
        assert!(seen.contains("\"top_bar_rows\":2"), "{seen}");
        assert!(seen.contains("\"gap_cols\":1"), "{seen}");
        assert!(seen.contains("\"owner\":\"bar\""), "{seen}");
    }

    #[test]
    fn chrome_reservation_request_owner_drops_blank_values() {
        let request = ChromeReservationRequest::top_bar(1).owner("   ");
        assert_eq!(request.owner, None);
        let request = ChromeReservationRequest::top_bar(1).owner(" panel ");
        assert_eq!(request.owner.as_deref(), Some("panel"));
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_helper_sends_expected_socket_command() {
        let path = PathBuf::from(test_socket_path("kwclip", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            stream
                .write_all(b"{\"allowed\":false,\"available\":false}\n")
                .unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let status = client.clipboard_json().unwrap();
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, "CLIPBOARD_JSON");
        assert!(!status.allowed);
        assert!(!status.has_payload());
    }

    #[cfg(unix)]
    #[test]
    fn shortcuts_helper_sends_expected_socket_command() {
        let path = PathBuf::from(test_socket_path("kwshortcuts", std::process::id()));
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
                    r#"{"schema_version":1,"kind":"kittwm-native-shortcuts","shortcuts":[{"id":"launch_terminal","keys":"C-a Enter / C-a t","description":"launch terminal"},{"id":"toggle_help","keys":"C-a ?","description":"toggle this help"},{"id":"drag_tiled_title","keys":"mouse: drag tiled title","description":"reorder tiled panes"},{"id":"drag_floating_title","keys":"mouse: drag floating title","description":"reposition floating pane"},{"id":"title_markers","keys":"title markers","description":"▶ focus · ◆ active · ⇄ reorder · ↔ resized · ▣ fullscreen · ≡ drag · ▲ top · ● moved"},{"id":"exit_kittwm","keys":"Ctrl-]","description":"exit kittwm"}]}
"#
                    .as_bytes(),
                )
                .unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let catalog = client.shortcuts_json().unwrap();
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, "SHORTCUTS_JSON");
        assert_eq!(catalog.kind.as_deref(), Some("kittwm-native-shortcuts"));
        assert!(catalog
            .shortcuts
            .iter()
            .any(|entry| entry.id == "launch_terminal"));
        assert!(catalog
            .shortcuts
            .iter()
            .any(|entry| entry.id == "toggle_help"));
        assert!(catalog.shortcuts.iter().any(|entry| entry.keys == "Ctrl-]"));
        assert_eq!(catalog.entry("toggle_help").unwrap().keys, "C-a ?");
        let tiled_drag = catalog.tiled_title_drag_shortcut().unwrap();
        assert!(tiled_drag.is_tiled_title_drag_shortcut());
        assert_eq!(tiled_drag.description, "reorder tiled panes");
        let floating_drag = catalog.floating_title_drag_shortcut().unwrap();
        assert!(floating_drag.is_floating_title_drag_shortcut());
        assert_eq!(floating_drag.description, "reposition floating pane");
        assert!(catalog.has_mouse_title_drag_shortcuts());
        let marker = catalog.title_marker_legend().unwrap();
        assert!(marker.is_title_marker_legend());
        assert!(marker.description.contains("◆ active"));
        assert!(marker.description.contains("⇄ reorder"));
        assert!(catalog.has_title_marker_legend());
    }

    #[cfg(unix)]
    #[test]
    fn help_catalog_helper_sends_expected_socket_command() {
        let path = PathBuf::from(test_socket_path("kwhc", std::process::id()));
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
                    b"{\"commands\":[{\"command\":\"PING\",\"category\":\"status\",\"description\":\"ping daemon\"}]}\n",
                )
                .unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let catalog = client.help_catalog().unwrap();
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, "HELP_JSON");
        assert_eq!(catalog.commands[0].command, "PING");
    }

    #[cfg(unix)]
    #[test]
    fn app_lookup_request_builds_directly() {
        assert_eq!(
            app_lookup_request("APPS_FIRST", "Visual Studio Code"),
            "APPS_FIRST Visual Studio Code"
        );
        assert_eq!(
            app_lookup_request("APPS_LAUNCH_FIRST", "Safari"),
            "APPS_LAUNCH_FIRST Safari"
        );
    }

    #[cfg(unix)]
    #[test]
    fn app_discovery_helpers_send_expected_socket_commands() {
        let path = PathBuf::from(test_socket_path("kwa", std::process::id()));
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
            client.app_first(" Visual Studio Code ").unwrap().name,
            "Visual Studio Code"
        );
        assert_eq!(client.app_launch_first(" Safari ").unwrap().pid, 42);
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

    #[test]
    fn app_discovery_helpers_reject_blank_queries_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock");
        assert!(matches!(
            client.app_first("   "),
            Err(Error::Daemon(message)) if message.contains("must be nonempty")
        ));
        assert!(matches!(
            client.app_launch_first(""),
            Err(Error::Daemon(message)) if message.contains("must be nonempty")
        ));
    }

    #[test]
    fn spawn_pty_request_builds_directly() {
        assert_eq!(spawn_pty_request("htop"), "SPAWN_PTY htop");
        assert_eq!(
            spawn_pty_request("kittwm-browser 'https://example.com/a%20b'"),
            "SPAWN_PTY kittwm-browser 'https://example.com/a%20b'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replace_current_closes_env_window_with_direct_request() {
        let path = PathBuf::from(test_socket_path("kwreplace", std::process::id()));
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
                seen.push(request.trim().to_string());
                stream.write_all(b"OK\n").unwrap();
            }
            seen
        });

        env::set_var("KITTWM_WINDOW", "native-9");
        let client = Kittwm::connect_path(&path);
        assert_eq!(
            client
                .replace_current(&WindowSpec {
                    title: None,
                    command: "htop".to_string(),
                })
                .unwrap()
                .trim(),
            "OK"
        );
        env::remove_var("KITTWM_WINDOW");
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, ["SPAWN_PTY htop", "CLOSE_PANE native-9"]);
    }

    #[cfg(unix)]
    #[test]
    fn spawn_surface_sends_browser_as_first_party_browser_app() {
        let path = PathBuf::from(test_socket_path("kwb", std::process::id()));
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

    #[cfg(unix)]
    #[test]
    fn session_helpers_send_expected_socket_commands() {
        let path = PathBuf::from(test_socket_path("kwsession", std::process::id()));
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
                seen.push(command.clone());
                let reply = match command.as_str() {
                    "SESSION_JSON" => {
                        r#"{"schema_version":1,"kind":"kittwm-native-session","layout":"rows","focus":"native-1","panes":[{"index":0,"window":"native-1","title":"shell","command":"bash","weight":1,"focused":true}]}"#
                    }
                    other if other.starts_with("RESTORE_SESSION_JSON ") => {
                        "RESTORE_SESSION_QUEUED command=1"
                    }
                    other => panic!("unexpected command {other}"),
                };
                stream.write_all(reply.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
            seen
        });
        let client = Kittwm::connect_path(&path);
        let session = client.session().unwrap();
        assert_eq!(session.layout, "rows");
        assert_eq!(session.panes[0].command, "bash");
        assert_eq!(
            client.restore_session(&session).unwrap().trim(),
            "RESTORE_SESSION_QUEUED command=1"
        );
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen[0], "SESSION_JSON");
        assert!(seen[1].starts_with("RESTORE_SESSION_JSON {"), "{}", seen[1]);
        assert!(
            seen[1].contains(r#""kind":"kittwm-native-session""#),
            "{}",
            seen[1]
        );
        assert!(seen[1].contains(r#""command":"bash""#), "{}", seen[1]);
    }

    #[test]
    fn session_capabilities_deny_before_io() {
        let read_only = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        let manifest = SessionManifest {
            schema_version: Some(1),
            kind: Some("kittwm-native-session".to_string()),
            layout: "columns".to_string(),
            focus: "-".to_string(),
            panes: vec![SessionPane {
                index: None,
                window: None,
                title: None,
                command: "bash".to_string(),
                weight: 1,
                focused: true,
                floating_dx: Some(0),
                floating_dy: Some(0),
            }],
        };
        assert!(matches!(
            read_only.restore_session(&manifest),
            Err(Error::CapabilityDenied(Capability::CreateWindow))
        ));
        let create_only = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::CreateWindow]));
        assert!(matches!(
            create_only.restore_session(&manifest),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        let no_read = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::CreateWindow]));
        assert!(matches!(
            no_read.session(),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
    }

    #[test]
    fn app_discovery_capabilities_deny_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::SubscribeEvents]));
        assert!(matches!(
            client.help_catalog(),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
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

    #[test]
    fn events_request_builds_directly() {
        assert_eq!(events_request(1), "EVENTS 1");
        assert_eq!(events_request(250), "EVENTS 250");
        assert_eq!(events_request(60_000), "EVENTS 60000");
    }

    #[cfg(unix)]
    #[test]
    fn events_ms_parses_json_lines_until_end() {
        let path = PathBuf::from(test_socket_path("kwe", std::process::id()));
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

    #[cfg(unix)]
    #[test]
    fn events_iter_ms_iterates_bounded_socket_batch() {
        let path = PathBuf::from(test_socket_path("kwei", std::process::id()));
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
                    b"{\"kind\":\"status\",\"seq\":1,\"detail\":{}}\n{\"kind\":\"focus_changed\",\"seq\":2,\"window\":\"native-2\",\"detail\":{}}\nEND\n",
                )
                .unwrap();
            request.trim().to_string()
        });
        let client = Kittwm::connect_path(&path);
        let mut iter = client.events_iter_ms(750).unwrap();
        assert_eq!(iter.len(), 2);
        assert_eq!(iter.next().unwrap().kind(), "status");
        let focus = iter.next().unwrap();
        assert_eq!(focus.kind(), "focus_changed");
        assert_eq!(
            focus.envelope().unwrap().window.as_deref(),
            Some("native-2")
        );
        assert_eq!(iter.next(), None);
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, "EVENTS 750");
    }

    #[test]
    fn surface_token_request_builds_directly() {
        assert_eq!(
            surface_token_request("FOCUS_PANE", "native-2"),
            "FOCUS_PANE native-2"
        );
        assert_eq!(
            surface_token_request("CLOSE_PANE", "focused"),
            "CLOSE_PANE focused"
        );
        assert_eq!(
            surface_token_request("READ_TEXT_JSON", "native-2"),
            "READ_TEXT_JSON native-2"
        );
        assert_eq!(
            surface_token_request("READ_SCROLLBACK_JSON", "native-2"),
            "READ_SCROLLBACK_JSON native-2"
        );
        assert_eq!(
            surface_token_request("SEMANTIC_SNAPSHOT", "native-2"),
            "SEMANTIC_SNAPSHOT native-2"
        );
    }

    #[test]
    fn resize_delta_label_builds_directly() {
        assert_eq!(resize_delta_label(3), "+3");
        assert_eq!(resize_delta_label(0), "+0");
        assert_eq!(resize_delta_label(-2), "-2");
        assert_eq!(resize_delta_label(i16::MIN), "-32768");
    }

    #[test]
    fn nudge_delta_payload_builds_directly() {
        assert_eq!(i16_decimal_len(0), 1);
        assert_eq!(i16_decimal_len(9), 1);
        assert_eq!(i16_decimal_len(10), 2);
        assert_eq!(i16_decimal_len(-1), 2);
        assert_eq!(i16_decimal_len(i16::MIN), 6);
        let payload = nudge_delta_payload(3, -2);
        assert_eq!(payload, "3 -2");
        assert_eq!(payload.capacity(), payload.len());
    }

    #[test]
    fn surface_payload_request_builds_directly() {
        assert_eq!(
            surface_payload_request("RENAME_PANE", "native-2", "Editor Pane"),
            "RENAME_PANE native-2 Editor Pane"
        );
        assert_eq!(
            surface_payload_request("RESIZE_PANE", "native-2", "+3"),
            "RESIZE_PANE native-2 +3"
        );
        assert_eq!(
            surface_payload_request("MOVE_PANE", "native-2", "down"),
            "MOVE_PANE native-2 down"
        );
        assert_eq!(
            surface_payload_request("SEND_TEXT", "native-2", "hello world"),
            "SEND_TEXT native-2 hello world"
        );
        assert_eq!(
            surface_payload_request("SEND_KEY", "native-2", "ctrl-c"),
            "SEND_KEY native-2 ctrl-c"
        );
        assert_eq!(
            surface_payload_request("SEND_BYTES_B64", "native-2", "aGk="),
            "SEND_BYTES_B64 native-2 aGk="
        );
        assert_eq!(
            surface_payload_request("PASTE_BYTES_B64", "native-2", "cGFzdGU="),
            "PASTE_BYTES_B64 native-2 cGFzdGU="
        );
        assert_eq!(
            surface_payload_request("WAIT_TEXT", "native-2", "ready"),
            "WAIT_TEXT native-2 ready"
        );
        assert_eq!(
            surface_payload_request("WAIT_OUTPUT", "native-2", "done"),
            "WAIT_OUTPUT native-2 done"
        );
        assert_eq!(
            surface_payload_request("SEMANTIC_FOCUS", "native-2", "button-1"),
            "SEMANTIC_FOCUS native-2 button-1"
        );
        assert_eq!(
            surface_payload_request("SEMANTIC_PUBLISH", "native-2", "{\"surface\":\"native-2\"}"),
            "SEMANTIC_PUBLISH native-2 {\"surface\":\"native-2\"}"
        );
        assert_eq!(
            surface_payload_request("WAIT_TEXT_JSON", "native-2", "prompt json"),
            "WAIT_TEXT_JSON native-2 prompt json"
        );
        assert_eq!(
            surface_payload_request("WAIT_OUTPUT_JSON", "native-2", "done json"),
            "WAIT_OUTPUT_JSON native-2 done json"
        );
    }

    #[test]
    fn surface_wait_ms_request_builds_directly() {
        assert_eq!(
            surface_wait_ms_request("WAIT_TEXT_MS", "native-1", 250, "ready"),
            "WAIT_TEXT_MS native-1 250 ready"
        );
        assert_eq!(
            surface_wait_ms_request("WAIT_OUTPUT_MS", "native-1", 60_000, "build finished"),
            "WAIT_OUTPUT_MS native-1 60000 build finished"
        );
        assert_eq!(
            surface_wait_ms_request("WAIT_TEXT_JSON_MS", "native-1", 300, "typed json"),
            "WAIT_TEXT_JSON_MS native-1 300 typed json"
        );
        assert_eq!(
            surface_wait_ms_request("WAIT_OUTPUT_JSON_MS", "native-1", 400, "output json"),
            "WAIT_OUTPUT_JSON_MS native-1 400 output json"
        );
    }

    #[test]
    fn surface_mouse_request_builds_directly() {
        assert_eq!(
            surface_mouse_request("native-1", MouseEvent::PressLeft, 7, 9),
            "SEND_MOUSE native-1 press-left 7 9"
        );
        assert_eq!(
            surface_mouse_request("native-2", MouseEvent::ReleaseRight, 12, 34),
            "SEND_MOUSE native-2 release-right 12 34"
        );
    }

    #[test]
    fn surface_action_request_builds_directly() {
        assert_eq!(
            surface_action_request(
                "SEMANTIC_ACTION",
                "native-1",
                "field",
                "set",
                "{\"value\":\"x\"}"
            ),
            "SEMANTIC_ACTION native-1 field set {\"value\":\"x\"}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn surface_focus_close_send_expected_socket_commands() {
        let path = PathBuf::from(test_socket_path("kwfc", std::process::id()));
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
                seen.push(request.trim().to_string());
                stream.write_all(b"OK\n").unwrap();
            }
            seen
        });

        let surface = Kittwm::connect_path(&path).surface("native-2");
        assert_eq!(surface.focus().unwrap().trim(), "OK");
        assert_eq!(surface.close().unwrap().trim(), "OK");
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(seen, ["FOCUS_PANE native-2", "CLOSE_PANE native-2"]);
    }

    #[cfg(unix)]
    #[test]
    fn control_helpers_send_expected_socket_commands() {
        let path = PathBuf::from(test_socket_path("kwc", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..16 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let command = request.trim().to_string();
                seen.push(command);
                stream.write_all(b"OK\n").unwrap();
            }
            seen
        });

        let client = Kittwm::connect_path(&path);
        assert_eq!(client.focus_next().unwrap().trim(), "OK");
        assert_eq!(client.focus_prev().unwrap().trim(), "OK");
        assert_eq!(client.layout(LayoutMode::Columns).unwrap().trim(), "OK");
        assert_eq!(client.layout(LayoutMode::Grid).unwrap().trim(), "OK");
        assert_eq!(
            client
                .split_focused(LayoutMode::Rows, "htop --tree")
                .unwrap()
                .trim(),
            "OK"
        );
        assert_eq!(client.balance_panes().unwrap().trim(), "OK");
        assert_eq!(client.reset_all_positions().unwrap().trim(), "OK");
        assert_eq!(client.reset_all_floating_offsets().unwrap().trim(), "OK");
        let surface = client.surface("native-2");
        assert_eq!(surface.rename(" Editor Pane ").unwrap().trim(), "OK");
        assert_eq!(
            surface.move_pane(MoveDirection::First).unwrap().trim(),
            "OK"
        );
        assert_eq!(surface.move_pane(MoveDirection::Down).unwrap().trim(), "OK");
        assert_eq!(surface.raise().unwrap().trim(), "OK");
        assert_eq!(surface.lower().unwrap().trim(), "OK");
        assert_eq!(surface.nudge(3, -2).unwrap().trim(), "OK");
        assert_eq!(surface.reset_floating_offset().unwrap().trim(), "OK");
        assert_eq!(surface.reset_position().unwrap().trim(), "OK");
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            seen,
            [
                "FOCUS_NEXT",
                "FOCUS_PREV",
                "LAYOUT columns",
                "LAYOUT grid",
                "SPLIT_PANE focused rows htop --tree",
                "BALANCE_PANES",
                "RESET_ALL_PANE_OFFSETS",
                "RESET_ALL_PANE_OFFSETS",
                "RENAME_PANE native-2 Editor Pane",
                "MOVE_PANE native-2 first",
                "MOVE_PANE native-2 down",
                "MOVE_PANE native-2 last",
                "MOVE_PANE native-2 first",
                "NUDGE_PANE native-2 3 -2",
                "RESET_PANE_OFFSET native-2",
                "RESET_PANE_OFFSET native-2"
            ]
        );
    }

    #[test]
    fn rename_helper_validates_title_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock").surface("focused");
        assert!(matches!(
            surface.rename("   "),
            Err(Error::Daemon(message)) if message.contains("nonempty title")
        ));
    }

    #[test]
    fn control_capabilities_deny_helpers_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]));
        assert!(matches!(
            client.focus_next(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.focus_prev(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.layout(LayoutMode::Rows),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.reset_all_positions(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.reset_all_floating_offsets(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.balance_panes(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.surface("focused").move_pane(MoveDirection::Last),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.surface("focused").raise(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.surface("focused").nudge(1, 0),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.surface("focused").reset_floating_offset(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
        assert!(matches!(
            client.surface("focused").reset_position(),
            Err(Error::CapabilityDenied(Capability::ControlWindow))
        ));
    }

    #[test]
    fn control_protocol_labels_match_daemon_vocab() {
        assert_eq!(LayoutMode::Columns.protocol_label(), "columns");
        assert_eq!(LayoutMode::Rows.protocol_label(), "rows");
        assert_eq!(LayoutMode::Grid.protocol_label(), "grid");
        assert_eq!(layout_request(LayoutMode::Columns), "LAYOUT columns");
        assert_eq!(layout_request(LayoutMode::Rows), "LAYOUT rows");
        assert_eq!(layout_request(LayoutMode::Grid), "LAYOUT grid");
        assert_eq!(
            split_pane_request("focused", LayoutMode::Rows, "htop --tree"),
            "SPLIT_PANE focused rows htop --tree"
        );
        assert_eq!(MoveDirection::Left.protocol_label(), "left");
        assert_eq!(MoveDirection::Right.protocol_label(), "right");
        assert_eq!(MoveDirection::Up.protocol_label(), "up");
        assert_eq!(MoveDirection::Down.protocol_label(), "down");
        assert_eq!(MoveDirection::First.protocol_label(), "first");
        assert_eq!(MoveDirection::Last.protocol_label(), "last");
    }

    #[cfg(unix)]
    #[test]
    fn input_helpers_send_expected_socket_commands() {
        let path = PathBuf::from(test_socket_path("kwi", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..10 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let command = request.trim().to_string();
                seen.push(command);
                stream.write_all(b"OK\n").unwrap();
            }
            seen
        });

        let surface = Kittwm::connect_path(&path).surface("native-1");
        assert_eq!(surface.send_text("   ").unwrap().trim(), "OK");
        assert_eq!(surface.send_line("echo hi").unwrap().trim(), "OK");
        assert_eq!(surface.send_key(" ctrl-c ").unwrap().trim(), "OK");
        assert_eq!(surface.send_bytes(b"hi\n\0").unwrap().trim(), "OK");
        assert_eq!(surface.send_bytes_b64(" AQID ").unwrap().trim(), "OK");
        assert_eq!(surface.paste_bytes(b"paste me").unwrap().trim(), "OK");
        assert_eq!(
            surface.paste_bytes_b64(" AP8bWzMxbQ== ").unwrap().trim(),
            "OK"
        );
        assert_eq!(surface.paste_bytes(b"\0\xff\x1b[31m").unwrap().trim(), "OK");
        assert_eq!(
            surface
                .send_mouse(MouseEvent::PressLeft, 7, 9)
                .unwrap()
                .trim(),
            "OK"
        );
        assert_eq!(
            surface
                .send_mouse(MouseEvent::ReleaseRight, 7, 9)
                .unwrap()
                .trim(),
            "OK"
        );
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            seen,
            [
                "SEND_TEXT native-1",
                "SEND_LINE native-1 echo hi",
                "SEND_KEY native-1 ctrl-c",
                "SEND_BYTES_B64 native-1 aGkKAA==",
                "SEND_BYTES_B64 native-1 AQID",
                "PASTE_BYTES_B64 native-1 cGFzdGUgbWU=",
                "PASTE_BYTES_B64 native-1 AP8bWzMxbQ==",
                "PASTE_BYTES_B64 native-1 AP8bWzMxbQ==",
                "SEND_MOUSE native-1 press-left 7 9",
                "SEND_MOUSE native-1 release-right 7 9"
            ]
        );
    }

    #[test]
    fn base64_byte_helpers_validate_payloads_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock").surface("focused");
        assert!(matches!(
            surface.send_bytes_b64(""),
            Err(Error::Daemon(message)) if message.contains("nonempty base64")
        ));
        assert!(matches!(
            surface.paste_bytes_b64("!!!"),
            Err(Error::Daemon(message)) if message.contains("invalid base64")
        ));
    }

    #[test]
    fn text_input_helpers_validate_payloads_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock").surface("focused");
        assert!(matches!(
            surface.send_text(""),
            Err(Error::Daemon(message)) if message.contains("nonempty text")
        ));
        assert!(matches!(
            surface.send_line(""),
            Err(Error::Daemon(message)) if message.contains("nonempty text")
        ));
    }

    #[test]
    fn send_key_helper_validates_token_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock").surface("focused");
        assert!(matches!(
            surface.send_key("   "),
            Err(Error::Daemon(message)) if message.contains("single nonempty token")
        ));
        assert!(matches!(
            surface.send_key("page down"),
            Err(Error::Daemon(message)) if message.contains("single nonempty token")
        ));
    }

    #[test]
    fn input_capabilities_deny_helpers_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::ReadText]))
            .surface("focused");
        assert!(matches!(
            surface.send_bytes(b"x"),
            Err(Error::CapabilityDenied(Capability::SendInput))
        ));
        assert!(matches!(
            surface.paste_bytes(b"x"),
            Err(Error::CapabilityDenied(Capability::SendInput))
        ));
        assert!(matches!(
            surface.send_mouse(MouseEvent::ScrollDown, 1, 2),
            Err(Error::CapabilityDenied(Capability::SendInput))
        ));
    }

    #[test]
    fn mouse_event_serde_matches_daemon_vocab() {
        assert_eq!(
            serde_json::to_string(&MouseEvent::ReleaseLeft).unwrap(),
            "\"release-left\""
        );
        assert_eq!(
            serde_json::from_str::<MouseEvent>("\"release-right\"").unwrap(),
            MouseEvent::ReleaseRight
        );
        assert!(serde_json::from_str::<MouseEvent>("\"release_right\"").is_err());
    }

    #[test]
    fn mouse_event_protocol_labels_match_daemon_vocab() {
        assert_eq!(MouseEvent::PressLeft.protocol_label(), "press-left");
        assert_eq!(MouseEvent::PressMiddle.protocol_label(), "press-middle");
        assert_eq!(MouseEvent::PressRight.protocol_label(), "press-right");
        assert_eq!(MouseEvent::Release.protocol_label(), "release");
        assert_eq!(MouseEvent::ReleaseLeft.protocol_label(), "release-left");
        assert_eq!(MouseEvent::ReleaseMiddle.protocol_label(), "release-middle");
        assert_eq!(MouseEvent::ReleaseRight.protocol_label(), "release-right");
        assert_eq!(MouseEvent::Move.protocol_label(), "move");
        assert_eq!(MouseEvent::MoveLeft.protocol_label(), "move-left");
        assert_eq!(MouseEvent::MoveMiddle.protocol_label(), "move-middle");
        assert_eq!(MouseEvent::MoveRight.protocol_label(), "move-right");
        assert_eq!(MouseEvent::ScrollUp.protocol_label(), "scroll-up");
        assert_eq!(MouseEvent::ScrollDown.protocol_label(), "scroll-down");
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
        let path = PathBuf::from(test_socket_path("kws", std::process::id()));
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
            surface.semantic_focus(" field "),
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

    #[test]
    fn semantic_helpers_validate_tokens_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock").surface("focused");
        assert!(matches!(
            surface.semantic_focus("bad component"),
            Err(Error::Daemon(message)) if message.contains("single nonempty token")
        ));
        assert!(matches!(
            surface.semantic_action("field", "bad action", serde_json::json!({})),
            Err(Error::Daemon(message)) if message.contains("single nonempty token")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn semantic_convenience_helpers_send_expected_commands() {
        let path = PathBuf::from(test_socket_path("kwh", std::process::id()));
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
    fn text_and_scrollback_snapshots_decode_json_shape() {
        let snapshot: TextSnapshot = serde_json::from_str(
            r#"{"window":"native-1","text":"hello\n","cursor_col":2,"cursor_row":0}"#,
        )
        .unwrap();
        assert_eq!(snapshot.window, "native-1");
        assert_eq!(snapshot.text, "hello\n");
        assert_eq!(snapshot.cursor_col, Some(2));

        let scrollback: ScrollbackSnapshot =
            serde_json::from_str(r#"{"window":"native-1","scrollback":"old\nlines\n"}"#).unwrap();
        assert_eq!(scrollback.window, "native-1");
        assert_eq!(scrollback.scrollback, "old\nlines\n");
    }

    #[test]
    fn wait_match_parser_decodes_successful_replies() {
        assert_eq!(
            parse_wait_match("MATCH_TEXT window=native-1 bytes=12\n").unwrap(),
            WaitMatch {
                kind: WaitMatchKind::Text,
                window: "native-1".to_string(),
                bytes: 12,
            }
        );
        assert_eq!(
            parse_wait_match("MATCH_OUTPUT window=focused bytes=64").unwrap(),
            WaitMatch {
                kind: WaitMatchKind::Output,
                window: "focused".to_string(),
                bytes: 64,
            }
        );
        assert!(matches!(
            parse_wait_match("MATCH_TEXT window=native-1"),
            Err(Error::Daemon(_))
        ));
    }

    #[test]
    fn scrollback_and_wait_helpers_deny_before_io() {
        let client = Kittwm::connect_path("/tmp/does-not-exist.sock")
            .with_capabilities(ClientCapabilities::only([Capability::SubscribeEvents]));
        let surface = client.focused_surface();
        assert!(matches!(
            surface.read_scrollback(),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
        assert!(matches!(
            surface.wait_text_ms(100, "ready"),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
        assert!(matches!(
            surface.wait_output_ms(100, "ready"),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
        assert!(matches!(
            surface.wait_text_match_json("ready"),
            Err(Error::CapabilityDenied(Capability::ReadText))
        ));
    }

    #[test]
    fn wait_helpers_validate_needles_before_io() {
        let surface = Kittwm::connect_path("/tmp/does-not-exist.sock").surface("focused");
        assert!(matches!(
            surface.wait_text("   "),
            Err(Error::Daemon(message)) if message.contains("nonempty needle")
        ));
        assert!(matches!(
            surface.wait_output_ms(100, ""),
            Err(Error::Daemon(message)) if message.contains("nonempty needle")
        ));
        assert!(matches!(
            surface.wait_text_match_json("   "),
            Err(Error::Daemon(message)) if message.contains("nonempty needle")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn scrollback_and_wait_helpers_send_expected_commands() {
        let path = PathBuf::from(test_socket_path("kww", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..13 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let command = request.trim().to_string();
                seen.push(command.clone());
                let reply = match command.as_str() {
                    "READ_SCROLLBACK_JSON native-1" => {
                        "{\"window\":\"native-1\",\"scrollback\":\"old\\n\"}"
                    }
                    "WAIT_TEXT_MS native-1 250 ready" => "MATCH_TEXT window=native-1 bytes=12",
                    "WAIT_OUTPUT_MS native-1 500 build finished" => {
                        "MATCH_OUTPUT window=native-1 bytes=64"
                    }
                    "WAIT_TEXT native-1 prompt" => "MATCH_TEXT window=native-1 bytes=10",
                    "WAIT_OUTPUT native-1 done" => "MATCH_OUTPUT window=native-1 bytes=20",
                    "WAIT_TEXT_MS native-1 100 typed" => "MATCH_TEXT window=native-1 bytes=30",
                    "WAIT_OUTPUT_MS native-1 200 typed out" => {
                        "MATCH_OUTPUT window=native-1 bytes=40"
                    }
                    "WAIT_TEXT native-1 prompt2" => "MATCH_TEXT window=native-1 bytes=50",
                    "WAIT_OUTPUT native-1 done2" => "MATCH_OUTPUT window=native-1 bytes=60",
                    "WAIT_TEXT_JSON_MS native-1 300 typed json" => {
                        r#"{"kind":"text","match":"MATCH_TEXT","window":"native-1","bytes":70}"#
                    }
                    "WAIT_OUTPUT_JSON_MS native-1 400 output json" => {
                        r#"{"kind":"output","match":"MATCH_OUTPUT","window":"native-1","bytes":80}"#
                    }
                    "WAIT_TEXT_JSON native-1 prompt json" => {
                        r#"{"kind":"text","match":"MATCH_TEXT","window":"native-1","bytes":90}"#
                    }
                    "WAIT_OUTPUT_JSON native-1 done json" => {
                        r#"{"kind":"output","match":"MATCH_OUTPUT","window":"native-1","bytes":100}"#
                    }
                    other => panic!("unexpected command {other}"),
                };
                stream.write_all(reply.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
            seen
        });

        let surface = Kittwm::connect_path(&path).surface("native-1");
        assert_eq!(surface.read_scrollback().unwrap().scrollback, "old\n");
        assert_eq!(
            surface.wait_text_ms(250, " ready ").unwrap().trim(),
            "MATCH_TEXT window=native-1 bytes=12"
        );
        assert_eq!(
            surface
                .wait_output_ms(500, " build finished ")
                .unwrap()
                .trim(),
            "MATCH_OUTPUT window=native-1 bytes=64"
        );
        assert_eq!(
            surface.wait_text(" prompt ").unwrap().trim(),
            "MATCH_TEXT window=native-1 bytes=10"
        );
        assert_eq!(
            surface.wait_output("done").unwrap().trim(),
            "MATCH_OUTPUT window=native-1 bytes=20"
        );
        assert_eq!(
            surface.wait_text_match_ms(100, "typed").unwrap(),
            WaitMatch {
                kind: WaitMatchKind::Text,
                window: "native-1".to_string(),
                bytes: 30,
            }
        );
        assert_eq!(
            surface.wait_output_match_ms(200, "typed out").unwrap(),
            WaitMatch {
                kind: WaitMatchKind::Output,
                window: "native-1".to_string(),
                bytes: 40,
            }
        );
        assert_eq!(surface.wait_text_match("prompt2").unwrap().bytes, 50);
        assert_eq!(surface.wait_output_match("done2").unwrap().bytes, 60);
        assert_eq!(
            surface.wait_text_match_json_ms(300, "typed json").unwrap(),
            WaitMatch {
                kind: WaitMatchKind::Text,
                window: "native-1".to_string(),
                bytes: 70,
            }
        );
        assert_eq!(
            surface
                .wait_output_match_json_ms(400, "output json")
                .unwrap(),
            WaitMatch {
                kind: WaitMatchKind::Output,
                window: "native-1".to_string(),
                bytes: 80,
            }
        );
        assert_eq!(
            surface.wait_text_match_json(" prompt json ").unwrap().bytes,
            90
        );
        assert_eq!(
            surface.wait_output_match_json("done json").unwrap().bytes,
            100
        );
        let seen = server.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            seen,
            [
                "READ_SCROLLBACK_JSON native-1",
                "WAIT_TEXT_MS native-1 250 ready",
                "WAIT_OUTPUT_MS native-1 500 build finished",
                "WAIT_TEXT native-1 prompt",
                "WAIT_OUTPUT native-1 done",
                "WAIT_TEXT_MS native-1 100 typed",
                "WAIT_OUTPUT_MS native-1 200 typed out",
                "WAIT_TEXT native-1 prompt2",
                "WAIT_OUTPUT native-1 done2",
                "WAIT_TEXT_JSON_MS native-1 300 typed json",
                "WAIT_OUTPUT_JSON_MS native-1 400 output json",
                "WAIT_TEXT_JSON native-1 prompt json",
                "WAIT_OUTPUT_JSON native-1 done json"
            ]
        );
    }
    #[test]
    fn process_snapshot_maps_panes_and_ps_lines() {
        let parsed = parse_ps_process_line(" 12 S 4096 3.5 zsh -l\n").unwrap();
        assert_eq!(parsed.ppid, Some(12));
        assert_eq!(parsed.state.as_deref(), Some("S"));
        assert_eq!(parsed.rss_kib, Some(4096));
        assert_eq!(parsed.cpu_percent, Some(3.5));
        let process_name = parsed.process_name.as_ref().unwrap();
        assert_eq!(process_name, "zsh -l");
        assert_eq!(process_name.capacity(), process_name.len());

        let panes = vec![NativePaneDetail {
            window: "native-1".to_string(),
            title: "shell".to_string(),
            focused: true,
            weight: 1,
            pid: Some(999999),
            command: Some("zsh".to_string()),
            ..NativePaneDetail::default()
        }];
        let snapshot = process_snapshot_from_panes(PathBuf::from("/tmp/kittwm.sock"), &panes);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.processes[0].window, "native-1");
        assert_eq!(snapshot.processes[0].command.as_deref(), Some("zsh"));
    }
}
