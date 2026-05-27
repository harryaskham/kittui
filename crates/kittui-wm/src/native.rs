//! Native kittwm app backends that do not require X11/Quartz windows.
//!
//! These adapters make local processes look like compositor surfaces. The PTY
//! backend turns a shell into a movable/resizable terminal pane; the headless
//! browser backend drives Chrome via the DevTools protocol and captures PNG
//! screenshots. They are intentionally small building blocks: higher layers can
//! wrap them in chrome, tiling, focus, and input policy just like X/Quartz
//! windows.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use kittui::Scene;
use kittui_core::geom::CellSize;
use kittui_ghostty_vt::{
    render_snapshot_preview_png, GhosttyRenderSnapshot, GhosttyVtTerminal, PreviewOptions,
};
use kittui_xvfb::{XButton, XCapture, XPointerEvent, XServer, XWindow, XWindowId};
use kittwm_sdk::{
    ActionKind, ComponentAction, ComponentNode, ComponentRole, ComponentState, ComponentValue,
    SemanticSurfaceSnapshot,
};
use parking_lot::Mutex;
use portable_pty::{
    Child as PtyChild, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem,
};
use serde_json::json;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};
use vte::{Params, Parser, Perform};

const SCROLLBACK_MAX_LINES: usize = 10_000;
const SCROLLBACK_PRUNE_BATCH: usize = 1_024;
const DEFAULT_TERMINAL_SCROLLBACK: usize = 1_000;

fn default_virtual_cell_size() -> CellSize {
    CellSize::default()
}

/// Terminal mouse reporting modes requested by a native PTY application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseReportingModes {
    /// Basic press/release mouse tracking (`CSI ? 1000 h`).
    pub basic: bool,
    /// Button-motion tracking while a button is held (`CSI ? 1002 h`).
    pub button_motion: bool,
    /// All-motion tracking, including hover motion (`CSI ? 1003 h`).
    pub all_motion: bool,
    /// SGR coordinate encoding (`CSI ? 1006 h`).
    pub sgr: bool,
}

/// Surface-level pointer button independent of backend-specific event types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfacePointerButton {
    /// Primary/left button.
    Left,
    /// Middle button.
    Middle,
    /// Secondary/right button.
    Right,
    /// Scroll up wheel step.
    ScrollUp,
    /// Scroll down wheel step.
    ScrollDown,
}

impl SurfacePointerButton {
    fn to_xbutton(self) -> XButton {
        match self {
            Self::Left => XButton::Left,
            Self::Middle => XButton::Middle,
            Self::Right => XButton::Right,
            Self::ScrollUp => XButton::ScrollUp,
            Self::ScrollDown => XButton::ScrollDown,
        }
    }
}

/// Surface-level pointer event in surface-local pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfacePointerEvent {
    /// Move pointer to absolute surface-local pixel coordinates.
    Move {
        /// Surface-local x coordinate in pixels.
        x_px: i32,
        /// Surface-local y coordinate in pixels.
        y_px: i32,
    },
    /// Press a pointer button.
    Press {
        /// Button to press.
        button: SurfacePointerButton,
    },
    /// Release a pointer button.
    Release {
        /// Button to release.
        button: SurfacePointerButton,
    },
}

/// Backend-independent input and capture surface for a kittwm-native app.
pub trait NativeApp {
    /// Human-readable app title.
    fn title(&self) -> String;
    /// Resize the app's logical surface.
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    /// Send UTF-8 text or terminal control bytes to the app.
    fn send_text(&mut self, text: &str) -> Result<()>;
    /// Capture the current app surface.
    fn capture(&mut self) -> Result<NativeFrame>;
}

/// Captured frame from a native app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeFrame {
    /// Raw RGBA pixels.
    Rgba {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// RGBA pixels, width * height * 4 bytes.
        rgba: Vec<u8>,
    },
    /// Encoded PNG bytes.
    Png {
        /// Frame width in pixels, parsed from IHDR.
        width: u32,
        /// Frame height in pixels, parsed from IHDR.
        height: u32,
        /// PNG bytes.
        bytes: Vec<u8>,
    },
}

impl NativeFrame {
    /// Frame width in pixels.
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgba { width, .. } | Self::Png { width, .. } => *width,
        }
    }

    /// Frame height in pixels.
    pub fn height(&self) -> u32 {
        match self {
            Self::Rgba { height, .. } | Self::Png { height, .. } => *height,
        }
    }

    /// Frame dimensions in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width(), self.height())
    }

    /// Stable lowercase frame format label.
    pub fn format(&self) -> &'static str {
        match self {
            Self::Rgba { .. } => "rgba",
            Self::Png { .. } => "png",
        }
    }

    /// Encoded or raw payload length in bytes.
    pub fn payload_len(&self) -> usize {
        match self {
            Self::Rgba { rgba, .. } => rgba.len(),
            Self::Png { bytes, .. } => bytes.len(),
        }
    }

    /// Whether this frame contains raw RGBA pixels.
    pub fn is_rgba(&self) -> bool {
        matches!(self, Self::Rgba { .. })
    }

    /// Whether this frame contains encoded PNG bytes.
    pub fn is_png(&self) -> bool {
        matches!(self, Self::Png { .. })
    }

    /// Convert an RGBA frame into the existing XCapture shape used by the WM.
    pub fn as_xcapture(&self, id: XWindowId) -> Option<XCapture> {
        match self {
            Self::Rgba {
                width,
                height,
                rgba,
            } => Some(XCapture {
                id,
                width: *width,
                height: *height,
                rgba: rgba.clone(),
            }),
            Self::Png { .. } => None,
        }
    }
}

/// Stable identifier for a native surface inside a kittwm session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(String);

impl SurfaceId {
    /// Create a surface id from a caller-provided stable token.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Coarse native surface kind for SDK metadata and capability routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    /// PTY-backed terminal surface.
    Terminal,
    /// Headless browser / DevTools-backed surface.
    Browser,
    /// X11/Xvfb captured window surface.
    X11,
    /// macOS Quartz/SCK captured window surface.
    Quartz,
    /// Kittui scene surface.
    KittuiScene,
    /// Composite surface made from child surfaces.
    Composite,
}

/// Capability flags advertised by a native surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceCapabilities {
    /// Surface can produce frames.
    pub capture: bool,
    /// Surface accepts text/key/mouse input.
    pub input: bool,
    /// Surface accepts exact byte input without UTF-8 conversion.
    pub exact_byte_input: bool,
    /// Surface can receive focus-in/focus-out notifications.
    pub focus_events: bool,
    /// Surface can emit side-effect events through `NativeSurface`.
    pub surface_events: bool,
    /// Surface can be resized.
    pub resize: bool,
    /// Surface exposes a human-readable title.
    pub title: bool,
    /// Surface can serialize restore metadata.
    pub restore: bool,
}

impl SurfaceCapabilities {
    /// Whether this surface can produce frames.
    pub fn can_capture(&self) -> bool {
        self.capture
    }

    /// Whether this surface accepts text/key/mouse input.
    pub fn can_send_text(&self) -> bool {
        self.input
    }

    /// Whether this surface accepts exact byte input without UTF-8 conversion.
    pub fn can_send_bytes(&self) -> bool {
        self.exact_byte_input
    }

    /// Whether this surface can receive focus-in/focus-out notifications.
    pub fn can_receive_focus_events(&self) -> bool {
        self.focus_events
    }

    /// Whether this surface can emit side-effect events.
    pub fn can_emit_surface_events(&self) -> bool {
        self.surface_events
    }

    /// Whether this surface can be resized.
    pub fn can_resize(&self) -> bool {
        self.resize
    }

    /// Whether this surface exposes a human-readable title.
    pub fn has_title(&self) -> bool {
        self.title
    }

    /// Whether this surface can serialize restore metadata.
    pub fn can_restore(&self) -> bool {
        self.restore
    }

    /// Standard capabilities for live terminal-like/native app surfaces.
    pub fn interactive_capture() -> Self {
        Self {
            capture: true,
            input: true,
            exact_byte_input: false,
            focus_events: false,
            surface_events: false,
            resize: true,
            title: true,
            restore: false,
        }
    }

    /// Standard capabilities for captured read-only scene-like surfaces.
    pub fn capture_only() -> Self {
        Self {
            capture: true,
            input: false,
            exact_byte_input: false,
            focus_events: false,
            surface_events: false,
            resize: false,
            title: true,
            restore: false,
        }
    }
}

/// Metadata describing one native surface without including frame bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceMetadata {
    /// Stable surface id.
    pub id: SurfaceId,
    /// Coarse surface kind.
    pub kind: SurfaceKind,
    /// Human-readable title.
    pub title: String,
    /// Advertised capabilities.
    pub capabilities: SurfaceCapabilities,
    /// Last known frame size in pixels, if available.
    pub frame_size: Option<(u32, u32)>,
}

/// Captured surface frame plus metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceFrame {
    /// Metadata captured with the frame.
    pub metadata: SurfaceMetadata,
    /// Native frame payload.
    pub frame: NativeFrame,
}

impl SurfaceFrame {
    /// Captured frame dimensions in pixels.
    pub fn frame_size(&self) -> (u32, u32) {
        self.frame.size()
    }

    /// Stable lowercase captured frame format label.
    pub fn format(&self) -> &'static str {
        self.frame.format()
    }

    /// Encoded or raw captured payload length in bytes.
    pub fn payload_len(&self) -> usize {
        self.frame.payload_len()
    }
}

fn cached_surface_frame(
    mut metadata: SurfaceMetadata,
    cached_frame: &Option<NativeFrame>,
) -> Option<SurfaceFrame> {
    let frame = cached_frame.clone()?;
    metadata.frame_size = Some(frame.size());
    Some(SurfaceFrame { metadata, frame })
}

fn cached_revision_frame(
    revision: u64,
    cached_revision: u64,
    cached_frame: &Option<NativeFrame>,
) -> Option<NativeFrame> {
    if revision == cached_revision {
        cached_frame.clone()
    } else {
        None
    }
}

/// Semantic side effects emitted by a surface while parsing/applying output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceEvent {
    /// Surface title changed.
    TitleChanged(String),
    /// Terminal bell requested.
    Bell {
        /// Whether the shell should show a visual bell affordance.
        visual: bool,
        /// Whether an audible/host bell is appropriate.
        audible: bool,
    },
    /// Surface requested clipboard contents to be set.
    ClipboardSet {
        /// Clipboard selection name, e.g. `c` for clipboard.
        selection: String,
        /// Base64 encoded payload from OSC 52.
        payload_base64: String,
    },
    /// Surface requested a notification.
    Notification {
        /// Notification title.
        title: String,
        /// Notification body.
        body: String,
    },
}

/// Common capture/input/resize/title interface for kittwm-native surfaces.
pub trait NativeSurface {
    /// Return metadata that can be consumed by SDK clients without frame bytes.
    fn metadata(&self) -> SurfaceMetadata;
    /// Resize the logical surface.
    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()>;
    /// Send text bytes to the surface.
    fn send_surface_text(&mut self, text: &str) -> Result<()>;
    /// Send exact bytes to the surface when supported.
    fn send_surface_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            anyhow!("surface does not support non-UTF-8 byte input through this adapter")
        })?;
        self.send_surface_text(text)
    }
    /// Notify the surface that it gained or lost focus.
    fn send_surface_focus(&mut self, _focused: bool) -> Result<()> {
        Ok(())
    }
    /// Send a pointer event in surface-local pixel coordinates when supported.
    fn send_surface_pointer(&mut self, _event: SurfacePointerEvent) -> Result<()> {
        Err(anyhow!(
            "surface does not support pointer input through this adapter"
        ))
    }
    /// Capture a frame and pair it with current metadata.
    fn capture_surface(&mut self) -> Result<SurfaceFrame>;
    /// Drain side-effect events emitted by the surface since the previous drain.
    fn take_surface_events(&mut self) -> Vec<SurfaceEvent> {
        Vec::new()
    }
}

/// Adapter that exposes an X11/Xvfb/XQuartz window as a common native surface.
///
/// The adapter keeps the existing [`XServer`] backend contract as the source of
/// truth for enumeration, capture, input, and window resizing, then translates it
/// into the [`NativeSurface`] shape used by PTY and browser surfaces. XQuartz is
/// an X11 backend under the hood, so it uses the same adapter as Xvfb.
pub struct XWindowSurface {
    server: Box<dyn XServer + Send + Sync>,
    window: XWindow,
    kind: SurfaceKind,
    cell_width: u32,
    cell_height: u32,
}

impl XWindowSurface {
    /// Wrap an X11-family backend window (FakeServer, Xvfb, or XQuartz).
    pub fn x11(
        server: Box<dyn XServer + Send + Sync>,
        window: XWindow,
        cell_width: u32,
        cell_height: u32,
    ) -> Self {
        Self::new(server, window, SurfaceKind::X11, cell_width, cell_height)
    }

    /// Wrap a Quartz capture target with the same surface interface. Quartz can
    /// capture and receive input; resize support depends on the backend target.
    pub fn quartz(
        server: Box<dyn XServer + Send + Sync>,
        window: XWindow,
        cell_width: u32,
        cell_height: u32,
    ) -> Self {
        Self::new(server, window, SurfaceKind::Quartz, cell_width, cell_height)
    }

    /// Wrap a backend window with an explicit surface kind.
    pub fn new(
        server: Box<dyn XServer + Send + Sync>,
        window: XWindow,
        kind: SurfaceKind,
        cell_width: u32,
        cell_height: u32,
    ) -> Self {
        Self {
            server,
            window,
            kind,
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
        }
    }

    fn current_window(&self) -> XWindow {
        self.server
            .windows()
            .ok()
            .and_then(|windows| windows.into_iter().find(|w| w.id == self.window.id))
            .unwrap_or_else(|| self.window.clone())
    }

    fn metadata_for(&self, window: &XWindow, frame_size: Option<(u32, u32)>) -> SurfaceMetadata {
        let mut capabilities = SurfaceCapabilities::interactive_capture();
        if self.kind == SurfaceKind::Quartz {
            capabilities.resize = false;
        }
        SurfaceMetadata {
            id: SurfaceId::new(format!("xwindow:{}", window.id.0)),
            kind: self.kind,
            title: window.title.clone(),
            capabilities,
            frame_size,
        }
    }
}

/// Capture-only adapter that exposes a kittui scene as a native surface.
///
/// This lets runtime/composite code treat first-party kittui render artifacts
/// the same way it treats PTY, browser, X11, or Quartz capture surfaces. The
/// adapter is intentionally immutable: callers that want a different logical
/// size should rebuild the scene so layer geometry and identity stay explicit.
pub struct KittuiSceneSurface {
    id: SurfaceId,
    title: String,
    scene: Scene,
}

impl KittuiSceneSurface {
    /// Wrap a scene with a stable surface id and human-readable title.
    pub fn new(id: impl Into<String>, title: impl Into<String>, scene: Scene) -> Self {
        Self {
            id: SurfaceId::new(id),
            title: title.into(),
            scene,
        }
    }

    /// Borrow the wrapped scene.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    fn frame_size(&self) -> (u32, u32) {
        (self.scene.pixel_width(), self.scene.pixel_height())
    }
}

impl NativeSurface for KittuiSceneSurface {
    fn metadata(&self) -> SurfaceMetadata {
        SurfaceMetadata {
            id: self.id.clone(),
            kind: SurfaceKind::KittuiScene,
            title: self.title.clone(),
            capabilities: SurfaceCapabilities::capture_only(),
            frame_size: Some(self.frame_size()),
        }
    }

    fn resize_surface(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        Err(anyhow!(
            "kittui scene surfaces are immutable; rebuild the scene to resize"
        ))
    }

    fn send_surface_text(&mut self, _text: &str) -> Result<()> {
        Err(anyhow!("kittui scene surfaces do not accept text input"))
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        let rendered = kittui_render_cpu::render_still(&self.scene)
            .map_err(|err| anyhow!("render kittui scene surface: {err:?}"))?;
        let frame = NativeFrame::Png {
            width: rendered.width_px,
            height: rendered.height_px,
            bytes: rendered.png,
        };
        Ok(SurfaceFrame {
            metadata: self.metadata(),
            frame,
        })
    }
}

/// Capture-only adapter for caller-provided RGBA frame streams.
///
/// This is a small bridge for renderers/compositors that already have raw RGBA
/// pixels and need to participate in the same native surface metadata/capture
/// path as PTY, browser, X/Quartz, and kittui scene surfaces.
pub struct RgbaFrameSurface {
    id: SurfaceId,
    title: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl RgbaFrameSurface {
    /// Create a new RGBA frame surface, validating dimensions and payload size.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<Self> {
        validate_rgba_frame(width, height, rgba.len())?;
        Ok(Self {
            id: SurfaceId::new(id),
            title: title.into(),
            width,
            height,
            rgba,
        })
    }

    /// Replace the current RGBA frame, validating dimensions and payload size.
    pub fn update_frame(&mut self, width: u32, height: u32, rgba: Vec<u8>) -> Result<()> {
        validate_rgba_frame(width, height, rgba.len())?;
        self.width = width;
        self.height = height;
        self.rgba = rgba;
        Ok(())
    }

    /// Return the current frame dimensions in pixels.
    pub fn frame_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl NativeSurface for RgbaFrameSurface {
    fn metadata(&self) -> SurfaceMetadata {
        SurfaceMetadata {
            id: self.id.clone(),
            kind: SurfaceKind::Composite,
            title: self.title.clone(),
            capabilities: SurfaceCapabilities::capture_only(),
            frame_size: Some(self.frame_size()),
        }
    }

    fn resize_surface(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        Err(anyhow!(
            "rgba frame surfaces are sized by their producer; update the frame instead"
        ))
    }

    fn send_surface_text(&mut self, _text: &str) -> Result<()> {
        Err(anyhow!("rgba frame surfaces do not accept text input"))
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        Ok(SurfaceFrame {
            metadata: self.metadata(),
            frame: NativeFrame::Rgba {
                width: self.width,
                height: self.height,
                rgba: self.rgba.clone(),
            },
        })
    }
}

/// One positioned RGBA child in a [`CompositeFrameSurface`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeFrameChild {
    /// Left offset in output pixels.
    pub x: u32,
    /// Top offset in output pixels.
    pub y: u32,
    /// Child frame width in pixels.
    pub width: u32,
    /// Child frame height in pixels.
    pub height: u32,
    /// Child RGBA pixels.
    pub rgba: Vec<u8>,
}

/// Capture-only surface that composites positioned RGBA children into one frame.
pub struct CompositeFrameSurface {
    id: SurfaceId,
    title: String,
    width: u32,
    height: u32,
    children: Vec<CompositeFrameChild>,
}

impl CompositeFrameSurface {
    /// Create an empty composite canvas.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        validate_rgba_dimensions(width, height)?;
        Ok(Self {
            id: SurfaceId::new(id),
            title: title.into(),
            width,
            height,
            children: Vec::new(),
        })
    }

    /// Append a positioned RGBA child frame.
    pub fn push_rgba_child(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<()> {
        validate_rgba_frame(width, height, rgba.len())?;
        self.children.push(CompositeFrameChild {
            x,
            y,
            width,
            height,
            rgba,
        });
        Ok(())
    }

    /// Append a positioned child from an existing surface capture.
    ///
    /// Only RGBA captures are accepted. PNG captures are deliberately rejected
    /// so callers choose an explicit decode/raster path instead of silently
    /// dropping or misinterpreting encoded frame bytes.
    pub fn push_surface_frame(&mut self, x: u32, y: u32, frame: &SurfaceFrame) -> Result<()> {
        match &frame.frame {
            NativeFrame::Rgba {
                width,
                height,
                rgba,
            } => self.push_rgba_child(x, y, *width, *height, rgba.clone()),
            NativeFrame::Png { .. } => Err(anyhow!(
                "composite frame surfaces require RGBA child frames; PNG input must be decoded first"
            )),
        }
    }

    /// Capture a child native surface and append its RGBA frame.
    ///
    /// The captured frame is returned so callers can inspect metadata or retain
    /// it for diagnostics. PNG captures are returned only through the error path
    /// from [`push_surface_frame`](Self::push_surface_frame); callers should
    /// decode them explicitly before composing.
    pub fn push_surface_capture<S: NativeSurface + ?Sized>(
        &mut self,
        x: u32,
        y: u32,
        surface: &mut S,
    ) -> Result<SurfaceFrame> {
        let frame = surface.capture_surface()?;
        self.push_surface_frame(x, y, &frame)?;
        Ok(frame)
    }

    /// Remove all children while preserving the canvas metadata.
    pub fn clear_children(&mut self) {
        self.children.clear();
    }

    /// Borrow positioned children in paint order.
    pub fn children(&self) -> &[CompositeFrameChild] {
        &self.children
    }

    fn frame_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn composite_rgba(&self) -> Result<Vec<u8>> {
        let mut out = vec![0u8; rgba_len(self.width, self.height)?];
        for child in &self.children {
            blend_child_rgba(&mut out, self.width, self.height, child)?;
        }
        Ok(out)
    }
}

impl NativeSurface for CompositeFrameSurface {
    fn metadata(&self) -> SurfaceMetadata {
        SurfaceMetadata {
            id: self.id.clone(),
            kind: SurfaceKind::Composite,
            title: self.title.clone(),
            capabilities: SurfaceCapabilities::capture_only(),
            frame_size: Some(self.frame_size()),
        }
    }

    fn resize_surface(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        Err(anyhow!(
            "composite frame surfaces are sized by their producer; rebuild the surface instead"
        ))
    }

    fn send_surface_text(&mut self, _text: &str) -> Result<()> {
        Err(anyhow!("composite frame surfaces do not accept text input"))
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        Ok(SurfaceFrame {
            metadata: self.metadata(),
            frame: NativeFrame::Rgba {
                width: self.width,
                height: self.height,
                rgba: self.composite_rgba()?,
            },
        })
    }
}

fn validate_rgba_frame(width: u32, height: u32, len: usize) -> Result<()> {
    let expected = rgba_len(width, height)?;
    if len != expected {
        return Err(anyhow!(
            "rgba frame payload length {len} does not match {width}x{height}x4 ({expected})"
        ));
    }
    Ok(())
}

fn validate_rgba_dimensions(width: u32, height: u32) -> Result<()> {
    let _ = rgba_len(width, height)?;
    Ok(())
}

fn rgba_len(width: u32, height: u32) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(anyhow!("rgba frame dimensions must be non-zero"));
    }
    usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow!("rgba frame dimensions overflow"))
}

fn blend_child_rgba(
    out: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    child: &CompositeFrameChild,
) -> Result<()> {
    validate_rgba_frame(child.width, child.height, child.rgba.len())?;
    let copy_width = child.width.min(canvas_width.saturating_sub(child.x));
    let copy_height = child.height.min(canvas_height.saturating_sub(child.y));
    if copy_width == 0 || copy_height == 0 {
        return Ok(());
    }
    for row in 0..copy_height {
        for col in 0..copy_width {
            let src_idx = ((row * child.width + col) * 4) as usize;
            let dst_idx = (((child.y + row) * canvas_width + child.x + col) * 4) as usize;
            blend_pixel(
                &mut out[dst_idx..dst_idx + 4],
                &child.rgba[src_idx..src_idx + 4],
            );
        }
    }
    Ok(())
}

fn blend_pixel(dst: &mut [u8], src: &[u8]) {
    let sa = u32::from(src[3]);
    if sa == 0 {
        return;
    }
    if sa == 255 || dst[3] == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let da = u32::from(dst[3]);
    let inv_sa = 255 - sa;
    let out_a = sa + (da * inv_sa + 127) / 255;
    if out_a == 0 {
        dst.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for channel in 0..3 {
        let sc = u32::from(src[channel]);
        let dc = u32::from(dst[channel]);
        let premul = sc * sa + (dc * da * inv_sa + 127) / 255;
        dst[channel] = ((premul + out_a / 2) / out_a).min(255) as u8;
    }
    dst[3] = out_a.min(255) as u8;
}

impl NativeSurface for XWindowSurface {
    fn metadata(&self) -> SurfaceMetadata {
        let window = self.current_window();
        self.metadata_for(
            &window,
            Some((
                window.rect.width.max(0.0) as u32,
                window.rect.height.max(0.0) as u32,
            )),
        )
    }

    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()> {
        let width = (cols as u32).saturating_mul(self.cell_width).max(1);
        let height = (rows as u32).saturating_mul(self.cell_height).max(1);
        self.server
            .resize_window(self.window.id, width, height)
            .with_context(|| format!("resize X window {:?} to {width}x{height}", self.window.id))?;
        self.window.rect.width = width as f32;
        self.window.rect.height = height as f32;
        Ok(())
    }

    fn send_surface_text(&mut self, text: &str) -> Result<()> {
        for ch in text.chars() {
            let sym = ch as u32;
            self.server
                .inject_key(sym, true)
                .with_context(|| format!("press keysym {sym} for X window {:?}", self.window.id))?;
            self.server.inject_key(sym, false).with_context(|| {
                format!("release keysym {sym} for X window {:?}", self.window.id)
            })?;
        }
        Ok(())
    }

    fn send_surface_pointer(&mut self, event: SurfacePointerEvent) -> Result<()> {
        let event = match event {
            SurfacePointerEvent::Move { x_px, y_px } => XPointerEvent::Move {
                window: self.window.id,
                x_px,
                y_px,
            },
            SurfacePointerEvent::Press { button } => XPointerEvent::Press {
                window: self.window.id,
                button: button.to_xbutton(),
            },
            SurfacePointerEvent::Release { button } => XPointerEvent::Release {
                window: self.window.id,
                button: button.to_xbutton(),
            },
        };
        self.server
            .inject_pointer(event)
            .with_context(|| format!("send pointer event to X window {:?}", self.window.id))
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        let capture = self
            .server
            .capture(self.window.id)
            .with_context(|| format!("capture X window {:?}", self.window.id))?;
        let window = self.current_window();
        self.window = window.clone();
        let frame = NativeFrame::Rgba {
            width: capture.width,
            height: capture.height,
            rgba: capture.rgba,
        };
        let metadata = self.metadata_for(&window, Some((frame.width(), frame.height())));
        Ok(SurfaceFrame { metadata, frame })
    }
}

/// Reusable terminal surface engine for a PTY-backed kittwm-native app.
///
/// `TerminalSurface` owns terminal parsing, PTY read/write, host responses,
/// readback snapshots, resize state, and RGBA rendering. Higher-level window
/// adapters such as [`PtyTerminalApp`] keep process lifecycle and policy while
/// delegating terminal behavior here.
pub struct TerminalSurface {
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    state: Arc<Mutex<TerminalState>>,
    _reader: JoinHandle<()>,
    cell_width: u32,
    cell_height: u32,
    cached_revision: u64,
    cached_frame: Option<NativeFrame>,
    cached_text_snapshot: Mutex<Option<(u64, String)>>,
    cached_scrollback_snapshot: Mutex<Option<(u64, String)>>,
}

/// A nested PTY terminal rendered into an RGBA frame.
pub struct PtyTerminalApp {
    title: String,
    child: Box<dyn PtyChild + Send + Sync>,
    surface: TerminalSurface,
}

/// A nested PTY terminal parsed by libghostty-vt and rendered into PNG frames.
pub struct GhosttyTerminalApp {
    title: String,
    child: Box<dyn PtyChild + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    output: mpsc::Receiver<Vec<u8>>,
    terminal: GhosttyVtTerminal,
    preview_options: PreviewOptions,
    cols: u16,
    rows: u16,
    last_text_snapshot: String,
    last_png_frame: Option<NativeFrame>,
}

impl TerminalSurface {
    /// Attach to an already-spawned PTY master and start parsing output.
    pub fn from_master(
        master: Box<dyn MasterPty + Send>,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<Self> {
        let mut reader = master.try_clone_reader().context("clone PTY reader")?;
        let writer = Arc::new(Mutex::new(master.take_writer().context("take PTY writer")?));
        let state = Arc::new(Mutex::new(TerminalState::new(cols, rows)));
        let reader_state = state.clone();
        let reader_writer = writer.clone();
        let join = std::thread::spawn(move || {
            let mut parser = Parser::new();
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = reader.read(&mut buf) else { break };
                if n == 0 {
                    break;
                }
                let responses = {
                    let mut state = reader_state.lock();
                    parser.advance(&mut *state, &buf[..n]);
                    state.bump_revision();
                    state.take_pending_responses()
                };
                if !responses.is_empty() {
                    let mut writer = reader_writer.lock();
                    if writer.write_all(&responses).is_err() || writer.flush().is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Self {
            master,
            writer,
            state,
            _reader: join,
            cell_width,
            cell_height,
            cached_revision: 0,
            cached_frame: None,
            cached_text_snapshot: Mutex::new(None),
            cached_scrollback_snapshot: Mutex::new(None),
        })
    }

    /// Return the terminal grid as plain text for assertions and accessibility.
    pub fn text_snapshot(&self) -> String {
        let state = self.state.lock();
        cached_terminal_snapshot(
            &state,
            &self.cached_text_snapshot,
            TerminalState::text_snapshot,
        )
    }

    /// Return lines that have scrolled off the terminal grid as plain text.
    pub fn scrollback_snapshot(&self) -> String {
        let state = self.state.lock();
        cached_terminal_snapshot(
            &state,
            &self.cached_scrollback_snapshot,
            TerminalState::scrollback_snapshot,
        )
    }

    /// Drain host-terminal OSC/control sequences requested by the nested app.
    pub fn take_host_sequences(&self) -> Vec<u8> {
        self.state.lock().take_pending_host_sequences()
    }

    /// Drain semantic surface events emitted by the nested app.
    pub fn take_surface_events(&self) -> Vec<SurfaceEvent> {
        self.state.lock().take_pending_surface_events()
    }

    /// Return the current zero-based cursor `(col, row)` in the terminal grid.
    pub fn cursor_position(&self) -> (u16, u16) {
        let state = self.state.lock();
        (state.cursor_col, state.cursor_row)
    }

    /// Whether the terminal cursor should be visible.
    pub fn cursor_visible(&self) -> bool {
        self.state.lock().cursor_visible
    }

    /// Whether the terminal application has enabled bracketed paste mode.
    pub fn bracketed_paste_enabled(&self) -> bool {
        self.state.lock().bracketed_paste
    }

    /// Whether the terminal application has enabled focus in/out reporting.
    pub fn focus_reporting_enabled(&self) -> bool {
        self.state.lock().focus_reporting
    }

    /// Whether the terminal application has enabled application cursor-key mode.
    pub fn application_cursor_keys_enabled(&self) -> bool {
        self.state.lock().application_cursor_keys
    }

    /// Mouse reporting modes requested by the terminal application.
    pub fn mouse_reporting_modes(&self) -> MouseReportingModes {
        self.state.lock().mouse_modes
    }

    /// Runtime title reported by the nested terminal, if any.
    pub fn title(&self) -> Option<String> {
        self.state.lock().title.clone()
    }

    /// Resize the terminal surface and backing PTY.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: cols.saturating_mul(self.cell_width.min(u32::from(u16::MAX)) as u16),
            pixel_height: rows.saturating_mul(self.cell_height.min(u32::from(u16::MAX)) as u16),
        })?;
        self.state.lock().resize(cols, rows);
        self.cached_frame = None;
        Ok(())
    }

    /// Send UTF-8 text bytes to the terminal surface.
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_bytes(text.as_bytes())
    }

    /// Send raw bytes to the PTY, preserving control sequences.
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock();
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Send terminal focus-in/focus-out bytes when focus reporting is enabled.
    pub fn send_focus(&mut self, focused: bool) -> Result<()> {
        if !self.focus_reporting_enabled() {
            return Ok(());
        }
        self.send_bytes(if focused { b"\x1b[I" } else { b"\x1b[O" })
    }

    /// Render the current terminal state as an RGBA frame.
    pub fn capture(&mut self) -> Result<NativeFrame> {
        let state = self.state.lock();
        if let Some(frame) =
            cached_revision_frame(state.revision, self.cached_revision, &self.cached_frame)
        {
            return Ok(frame);
        }
        let render_state = state.render_state();
        let revision = state.revision;
        drop(state);
        let frame = NativeFrame::Rgba {
            width: u32::from(render_state.cols) * self.cell_width,
            height: u32::from(render_state.rows) * self.cell_height,
            rgba: render_terminal_render_state_rgba(
                &render_state,
                self.cell_width,
                self.cell_height,
            ),
        };
        self.cached_revision = revision;
        self.cached_frame = Some(frame.clone());
        Ok(frame)
    }
}

fn ghostty_snapshot_text(snapshot: &GhosttyRenderSnapshot) -> String {
    let mut out = String::with_capacity(
        snapshot
            .cells
            .iter()
            .map(|row| row.iter().map(|cell| cell.text.len()).sum::<usize>() + 1)
            .sum::<usize>()
            .saturating_sub(1),
    );
    for (row_idx, row) in snapshot.cells.iter().enumerate() {
        if row_idx > 0 {
            out.push('\n');
        }
        for cell in row {
            out.push_str(&cell.text);
        }
    }
    out
}

fn should_invalidate_ghostty_png_cache_after_text_refresh(had_output: bool) -> bool {
    had_output
}

impl GhosttyTerminalApp {
    /// Spawn a shell command in a PTY rendered by libghostty-vt.
    pub fn spawn(command: &str, cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with_env(command, cols, rows, std::iter::empty::<(&str, &str)>())
    }

    /// Spawn a shell command in a PTY rendered by libghostty-vt with extra environment variables.
    pub fn spawn_with_env<'a, I, K, V>(command: &str, cols: u16, rows: u16, envs: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr> + 'a,
        V: AsRef<std::ffi::OsStr> + 'a,
    {
        Self::spawn_with_env_and_preview(command, cols, rows, envs, PreviewOptions::default())
    }

    /// Spawn a shell command in a PTY rendered by libghostty-vt with explicit preview styling.
    pub fn spawn_with_env_and_preview<'a, I, K, V>(
        command: &str,
        cols: u16,
        rows: u16,
        envs: I,
        preview_options: PreviewOptions,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr> + 'a,
        V: AsRef<std::ffi::OsStr> + 'a,
    {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let cell = default_virtual_cell_size();
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: cols.saturating_mul(cell.width_px),
                pixel_height: rows.saturating_mul(cell.height_px),
            })
            .context("open libghostty PTY")?;
        let mut builder = CommandBuilder::new(default_pty_shell());
        builder.arg("-lc");
        builder.arg(command);
        for (key, value) in envs {
            builder.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(builder)
            .context("spawn libghostty PTY child")?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("clone libghostty PTY reader")?;
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .context("take libghostty PTY writer")?,
        ));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = reader.read(&mut buf) else { break };
                if n == 0 {
                    break;
                }
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            title: command.to_string(),
            child,
            master: pair.master,
            writer,
            output: rx,
            terminal: GhosttyVtTerminal::new(cols, rows, DEFAULT_TERMINAL_SCROLLBACK)?,
            preview_options,
            cols,
            rows,
            last_text_snapshot: String::new(),
            last_png_frame: None,
        })
    }

    fn drain_output(&mut self) -> bool {
        let mut drained = false;
        while let Ok(bytes) = self.output.try_recv() {
            self.terminal.write(bytes);
            drained = true;
        }
        drained
    }

    /// Return the latest text snapshot captured from libghostty-vt.
    pub fn text_snapshot(&self) -> String {
        self.last_text_snapshot.clone()
    }

    /// Refresh the text snapshot without rendering a PNG frame.
    pub fn refresh_text_snapshot(&mut self) -> Result<bool> {
        let had_output = self.drain_output();
        if should_invalidate_ghostty_png_cache_after_text_refresh(had_output) {
            self.last_png_frame = None;
        }
        if !had_output && !self.last_text_snapshot.is_empty() {
            return Ok(false);
        }
        let snapshot = self.terminal.render_snapshot()?;
        self.last_text_snapshot = ghostty_snapshot_text(&snapshot);
        Ok(true)
    }

    /// Return whether bracketed paste is known to be enabled.
    pub fn bracketed_paste_enabled(&self) -> bool {
        false
    }

    /// Return whether application cursor keys are known to be enabled.
    pub fn application_cursor_keys_enabled(&self) -> bool {
        false
    }

    /// Mouse reporting modes requested by the terminal application.
    pub fn mouse_reporting_modes(&self) -> MouseReportingModes {
        MouseReportingModes::default()
    }

    /// Return the PTY child process id when the backend exposes one.
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Whether the PTY child has exited.
    pub fn exited(&mut self) -> Result<Option<u32>> {
        Ok(self.child.try_wait()?.map(|status| status.exit_code()))
    }

    /// Terminate the PTY child process.
    pub fn terminate(&mut self) -> Result<()> {
        self.child.kill()?;
        Ok(())
    }

    /// Send raw bytes to the PTY, preserving control sequences.
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock();
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }
}

impl NativeApp for GhosttyTerminalApp {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.resize_surface(cols, rows)
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_surface_text(text)
    }

    fn capture(&mut self) -> Result<NativeFrame> {
        Ok(self.capture_surface()?.frame)
    }
}

impl NativeSurface for GhosttyTerminalApp {
    fn metadata(&self) -> SurfaceMetadata {
        let mut capabilities = SurfaceCapabilities::interactive_capture();
        capabilities.exact_byte_input = true;
        SurfaceMetadata {
            id: SurfaceId::new(format!(
                "ghostty-pty:{}",
                self.process_id()
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )),
            kind: SurfaceKind::Terminal,
            title: self.title.clone(),
            capabilities,
            frame_size: Some((
                u32::from(self.cols) * u32::from(default_virtual_cell_size().width_px),
                u32::from(self.rows) * u32::from(default_virtual_cell_size().height_px),
            )),
        }
    }

    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let cell = default_virtual_cell_size();
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: cols.saturating_mul(cell.width_px),
            pixel_height: rows.saturating_mul(cell.height_px),
        })?;
        // libghostty-vt does not expose a resize wrapper yet; recreate the
        // backing terminal so future output uses the new virtual app size.
        self.terminal = GhosttyVtTerminal::new(cols, rows, DEFAULT_TERMINAL_SCROLLBACK)?;
        self.cols = cols;
        self.rows = rows;
        self.last_text_snapshot.clear();
        self.last_png_frame = None;
        Ok(())
    }

    fn send_surface_text(&mut self, text: &str) -> Result<()> {
        self.send_bytes(text.as_bytes())
    }

    fn send_surface_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.send_bytes(bytes)
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        let had_output = self.drain_output();
        if !had_output {
            if let Some(frame) = cached_surface_frame(self.metadata(), &self.last_png_frame) {
                return Ok(frame);
            }
        }
        let snapshot = self.terminal.render_snapshot()?;
        self.last_text_snapshot = ghostty_snapshot_text(&snapshot);
        let png = render_snapshot_preview_png(&snapshot, &self.preview_options)?;
        let frame = NativeFrame::Png {
            width: u32::from(snapshot.cols) * u32::from(default_virtual_cell_size().width_px),
            height: u32::from(snapshot.rows) * u32::from(default_virtual_cell_size().height_px),
            bytes: png,
        };
        self.last_png_frame = Some(frame.clone());
        let mut metadata = self.metadata();
        metadata.frame_size = Some(frame.size());
        Ok(SurfaceFrame { metadata, frame })
    }
}

fn cached_terminal_snapshot(
    state: &TerminalState,
    cache: &Mutex<Option<(u64, String)>>,
    build: fn(&TerminalState) -> String,
) -> String {
    let revision = state.revision;
    if let Some((cached_revision, cached)) = cache.lock().as_ref() {
        if *cached_revision == revision {
            return cached.clone();
        }
    }
    let snapshot = build(state);
    *cache.lock() = Some((revision, snapshot.clone()));
    snapshot
}

impl PtyTerminalApp {
    /// Spawn a shell command in a real PTY.
    pub fn spawn(command: &str, cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with_env(command, cols, rows, std::iter::empty::<(&str, &str)>())
    }

    /// Spawn a program directly in a real PTY without invoking a shell.
    pub fn spawn_program(program: &str, args: &[&str], cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_program_with_env(
            program,
            args,
            cols,
            rows,
            std::iter::empty::<(&str, &str)>(),
        )
    }

    /// Spawn a program directly in a real PTY with extra environment variables.
    pub fn spawn_program_with_env<'a, I, K, V>(
        program: &str,
        args: &[&str],
        cols: u16,
        rows: u16,
        envs: I,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr> + 'a,
        V: AsRef<std::ffi::OsStr> + 'a,
    {
        let cell = default_virtual_cell_size();
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: cols.saturating_mul(cell.width_px),
                pixel_height: rows.saturating_mul(cell.height_px),
            })
            .context("open PTY")?;
        let mut builder = CommandBuilder::new(program);
        for arg in args {
            builder.arg(arg);
        }
        for (key, value) in envs {
            builder.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(builder)
            .with_context(|| format!("spawn PTY child program {program}"))?;
        drop(pair.slave);
        let surface = TerminalSurface::from_master(
            pair.master,
            cols,
            rows,
            u32::from(cell.width_px),
            u32::from(cell.height_px),
        )?;
        Ok(Self {
            title: program.to_string(),
            child,
            surface,
        })
    }

    /// Spawn a shell command in a real PTY with extra environment variables.
    pub fn spawn_with_env<'a, I, K, V>(command: &str, cols: u16, rows: u16, envs: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr> + 'a,
        V: AsRef<std::ffi::OsStr> + 'a,
    {
        let cell = default_virtual_cell_size();
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: cols.saturating_mul(cell.width_px),
                pixel_height: rows.saturating_mul(cell.height_px),
            })
            .context("open PTY")?;
        let mut builder = CommandBuilder::new(default_pty_shell());
        builder.arg("-lc");
        builder.arg(command);
        for (key, value) in envs {
            builder.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(builder)
            .context("spawn PTY child")?;
        drop(pair.slave);
        let surface = TerminalSurface::from_master(
            pair.master,
            cols,
            rows,
            u32::from(cell.width_px),
            u32::from(cell.height_px),
        )?;
        Ok(Self {
            title: command.to_string(),
            child,
            surface,
        })
    }

    /// Return the terminal grid as plain text for assertions and accessibility.
    pub fn text_snapshot(&self) -> String {
        self.surface.text_snapshot()
    }

    /// Return lines that have scrolled off the terminal grid as plain text.
    pub fn scrollback_snapshot(&self) -> String {
        self.surface.scrollback_snapshot()
    }

    /// Drain host-terminal OSC/control sequences requested by the nested app.
    pub fn take_host_sequences(&self) -> Vec<u8> {
        self.surface.take_host_sequences()
    }

    /// Drain semantic surface events emitted by the nested app.
    pub fn take_surface_events(&self) -> Vec<SurfaceEvent> {
        self.surface.take_surface_events()
    }

    /// Return the current zero-based cursor `(col, row)` in the terminal grid.
    pub fn cursor_position(&self) -> (u16, u16) {
        self.surface.cursor_position()
    }

    /// Whether the terminal cursor should be visible.
    pub fn cursor_visible(&self) -> bool {
        self.surface.cursor_visible()
    }

    /// Whether the terminal application has enabled bracketed paste mode.
    pub fn bracketed_paste_enabled(&self) -> bool {
        self.surface.bracketed_paste_enabled()
    }

    /// Whether the terminal application has enabled focus in/out reporting.
    pub fn focus_reporting_enabled(&self) -> bool {
        self.surface.focus_reporting_enabled()
    }

    /// Whether the terminal application has enabled application cursor-key mode.
    pub fn application_cursor_keys_enabled(&self) -> bool {
        self.surface.application_cursor_keys_enabled()
    }

    /// Mouse reporting modes requested by the terminal application.
    pub fn mouse_reporting_modes(&self) -> MouseReportingModes {
        self.surface.mouse_reporting_modes()
    }

    /// Return the PTY child process id when the backend exposes one.
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Whether the PTY child has exited.
    pub fn exited(&mut self) -> Result<Option<u32>> {
        Ok(self.child.try_wait()?.map(|status| status.exit_code()))
    }

    /// Terminate the PTY child process.
    pub fn terminate(&mut self) -> Result<()> {
        self.child.kill()?;
        Ok(())
    }

    /// Send raw bytes to the PTY, preserving control sequences.
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.surface.send_bytes(bytes)
    }
}

impl NativeApp for PtyTerminalApp {
    fn title(&self) -> String {
        self.surface.title().unwrap_or_else(|| self.title.clone())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.resize_surface(cols, rows)
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_surface_text(text)
    }

    fn capture(&mut self) -> Result<NativeFrame> {
        Ok(self.capture_surface()?.frame)
    }
}

impl NativeSurface for PtyTerminalApp {
    fn metadata(&self) -> SurfaceMetadata {
        let mut capabilities = SurfaceCapabilities::interactive_capture();
        capabilities.exact_byte_input = true;
        capabilities.focus_events = true;
        capabilities.surface_events = true;
        SurfaceMetadata {
            id: SurfaceId::new(format!(
                "pty:{}",
                self.process_id()
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )),
            kind: SurfaceKind::Terminal,
            title: self.surface.title().unwrap_or_else(|| self.title.clone()),
            capabilities,
            frame_size: None,
        }
    }

    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.surface.resize(cols, rows)
    }

    fn send_surface_text(&mut self, text: &str) -> Result<()> {
        self.surface.send_text(text)
    }

    fn send_surface_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.surface.send_bytes(bytes)
    }

    fn send_surface_focus(&mut self, focused: bool) -> Result<()> {
        self.surface.send_focus(focused)
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        let frame = self.surface.capture()?;
        let frame_size = match &frame {
            NativeFrame::Rgba { width, height, .. } | NativeFrame::Png { width, height, .. } => {
                Some((*width, *height))
            }
        };
        let mut metadata = self.metadata();
        metadata.frame_size = frame_size;
        Ok(SurfaceFrame { metadata, frame })
    }

    fn take_surface_events(&mut self) -> Vec<SurfaceEvent> {
        self.surface.take_surface_events()
    }
}

#[derive(Clone)]
struct TerminalState {
    cols: u16,
    rows: u16,
    cursor_col: u16,
    cursor_row: u16,
    saved_cursor_col: u16,
    saved_cursor_row: u16,
    cursor_visible: bool,
    origin_mode: bool,
    auto_wrap: bool,
    application_cursor_keys: bool,
    insert_mode: bool,
    dec_special_graphics: bool,
    scroll_top: u16,
    scroll_bottom: u16,
    cells: Vec<TerminalCell>,
    current_style: TerminalStyle,
    scrollback: Vec<String>,
    pending_responses: Vec<u8>,
    pending_host_sequences: Vec<u8>,
    pending_surface_events: Vec<SurfaceEvent>,
    alt_screen: Option<AlternateScreen>,
    bracketed_paste: bool,
    focus_reporting: bool,
    mouse_modes: MouseReportingModes,
    title: Option<String>,
    revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalCell {
    ch: char,
    style: TerminalStyle,
}

struct TerminalRenderState {
    cols: u16,
    rows: u16,
    cursor_col: u16,
    cursor_row: u16,
    cursor_visible: bool,
    cells: Vec<TerminalCell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalStyle {
    fg: Option<TerminalColor>,
    bg: Option<TerminalColor>,
    bold: bool,
    reverse: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalColor(u8, u8, u8);

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            reverse: false,
        }
    }
}

impl TerminalCell {
    fn blank(style: TerminalStyle) -> Self {
        Self { ch: ' ', style }
    }
}

#[derive(Clone)]
struct AlternateScreen {
    normal_cells: Vec<TerminalCell>,
    normal_cursor_col: u16,
    normal_cursor_row: u16,
    normal_scroll_top: u16,
    normal_scroll_bottom: u16,
}

impl TerminalState {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            saved_cursor_col: 0,
            saved_cursor_row: 0,
            cursor_visible: true,
            origin_mode: false,
            auto_wrap: true,
            application_cursor_keys: false,
            insert_mode: false,
            dec_special_graphics: false,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            cells: vec![
                TerminalCell::blank(TerminalStyle::default());
                usize::from(cols) * usize::from(rows)
            ],
            current_style: TerminalStyle::default(),
            scrollback: Vec::new(),
            pending_responses: Vec::new(),
            pending_host_sequences: Vec::new(),
            pending_surface_events: Vec::new(),
            alt_screen: None,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_modes: MouseReportingModes::default(),
            title: None,
            revision: 0,
        }
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn render_state(&self) -> TerminalRenderState {
        TerminalRenderState {
            cols: self.cols,
            rows: self.rows,
            cursor_col: self.cursor_col,
            cursor_row: self.cursor_row,
            cursor_visible: self.cursor_visible,
            cells: self.cells.clone(),
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let old = self.clone();
        *self = Self::new(cols, rows);
        self.revision = old.revision.wrapping_add(1);
        self.title = old.title.clone();
        self.scrollback = old.scrollback.clone();
        self.pending_responses = old.pending_responses;
        self.pending_host_sequences = old.pending_host_sequences;
        self.pending_surface_events = old.pending_surface_events;
        self.current_style = old.current_style;
        self.cursor_visible = old.cursor_visible;
        self.origin_mode = old.origin_mode;
        self.auto_wrap = old.auto_wrap;
        self.application_cursor_keys = old.application_cursor_keys;
        self.insert_mode = old.insert_mode;
        self.dec_special_graphics = old.dec_special_graphics;
        self.bracketed_paste = old.bracketed_paste;
        self.focus_reporting = old.focus_reporting;
        self.mouse_modes = old.mouse_modes;
        self.cells = resize_cells(&old.cells, old.cols, old.rows, cols, rows);
        self.alt_screen = old.alt_screen.map(|alt| AlternateScreen {
            normal_cells: resize_cells(&alt.normal_cells, old.cols, old.rows, cols, rows),
            normal_cursor_col: alt.normal_cursor_col.min(cols.saturating_sub(1)),
            normal_cursor_row: alt.normal_cursor_row.min(rows.saturating_sub(1)),
            normal_scroll_top: alt.normal_scroll_top.min(rows.saturating_sub(1)),
            normal_scroll_bottom: alt.normal_scroll_bottom.min(rows.saturating_sub(1)),
        });
        self.cursor_col = old.cursor_col.min(cols.saturating_sub(1));
        self.cursor_row = old.cursor_row.min(rows.saturating_sub(1));
        self.saved_cursor_col = old.saved_cursor_col.min(cols.saturating_sub(1));
        self.saved_cursor_row = old.saved_cursor_row.min(rows.saturating_sub(1));
        self.scroll_top = old.scroll_top.min(rows.saturating_sub(1));
        self.scroll_bottom = old.scroll_bottom.min(rows.saturating_sub(1));
        if self.scroll_top >= self.scroll_bottom {
            self.reset_scroll_region();
        }
    }

    fn text_snapshot(&self) -> String {
        let mut out = String::with_capacity(
            usize::from(self.rows).saturating_mul(usize::from(self.cols).saturating_add(1)),
        );
        for row in 0..self.rows {
            self.append_line_snapshot(row, &mut out);
            out.push('\n');
        }
        out
    }

    fn scrollback_snapshot(&self) -> String {
        if self.scrollback.is_empty() {
            return String::new();
        }
        let mut out = String::with_capacity(
            self.scrollback
                .iter()
                .map(|line| line.len().saturating_add(1))
                .sum(),
        );
        for line in &self.scrollback {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn take_pending_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_responses)
    }

    fn take_pending_host_sequences(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_host_sequences)
    }

    fn take_pending_surface_events(&mut self) -> Vec<SurfaceEvent> {
        std::mem::take(&mut self.pending_surface_events)
    }

    fn queue_response(&mut self, bytes: impl AsRef<[u8]>) {
        self.pending_responses.extend_from_slice(bytes.as_ref());
    }

    fn queue_host_sequence(&mut self, bytes: impl AsRef<[u8]>) {
        self.pending_host_sequences
            .extend_from_slice(bytes.as_ref());
    }

    fn queue_surface_event(&mut self, event: SurfaceEvent) {
        self.pending_surface_events.push(event);
    }

    fn line_snapshot(&self, row: u16) -> String {
        let mut out = String::new();
        self.append_line_snapshot(row, &mut out);
        out
    }

    fn append_line_snapshot(&self, row: u16, out: &mut String) {
        let start = usize::from(row) * usize::from(self.cols);
        let end = start + usize::from(self.cols);
        let cells = &self.cells[start..end];
        let visible_end = cells
            .iter()
            .rposition(|cell| cell.ch != ' ')
            .map(|idx| idx + 1)
            .unwrap_or(0);
        out.extend(cells[..visible_end].iter().map(|cell| cell.ch));
    }

    fn push_scrollback_line(&mut self, line: String) {
        self.scrollback.push(line);
        if self.scrollback.len() > SCROLLBACK_MAX_LINES {
            let overflow = self.scrollback.len() - SCROLLBACK_MAX_LINES;
            let prune = overflow
                .max(SCROLLBACK_PRUNE_BATCH)
                .min(self.scrollback.len());
            self.scrollback.drain(0..prune);
        }
    }

    fn put_at(&mut self, col: u16, row: u16, ch: char) {
        if col < self.cols && row < self.rows {
            let idx = usize::from(row) * usize::from(self.cols) + usize::from(col);
            self.cells[idx] = TerminalCell {
                ch,
                style: self.current_style,
            };
        }
    }

    fn put_cell_at(&mut self, col: u16, row: u16, cell: TerminalCell) {
        if col < self.cols && row < self.rows {
            let idx = usize::from(row) * usize::from(self.cols) + usize::from(col);
            self.cells[idx] = cell;
        }
    }

    fn get_cell_at(&self, col: u16, row: u16) -> TerminalCell {
        if col < self.cols && row < self.rows {
            self.cells[usize::from(row) * usize::from(self.cols) + usize::from(col)]
        } else {
            TerminalCell::blank(TerminalStyle::default())
        }
    }

    fn newline(&mut self) {
        self.cursor_col = 0;
        self.index();
    }

    fn scroll_region_up(&mut self, top: u16, bottom: u16) {
        if top >= bottom || bottom >= self.rows {
            return;
        }
        if top == 0 && bottom == self.rows.saturating_sub(1) && self.alt_screen.is_none() {
            self.push_scrollback_line(self.line_snapshot(0));
        }
        let cols = usize::from(self.cols);
        let start = usize::from(top) * cols;
        let end = (usize::from(bottom) + 1) * cols;
        self.cells.copy_within(start + cols..end, start);
        let clear_start = usize::from(bottom) * cols;
        for cell in &mut self.cells[clear_start..clear_start + cols] {
            *cell = TerminalCell::blank(self.current_style);
        }
    }

    fn scroll_region_down(&mut self, top: u16, bottom: u16) {
        if top >= bottom || bottom >= self.rows {
            return;
        }
        let cols = usize::from(self.cols);
        let start = usize::from(top) * cols;
        let end = usize::from(bottom) * cols;
        self.cells.copy_within(start..end, start + cols);
        for cell in &mut self.cells[start..start + cols] {
            *cell = TerminalCell::blank(self.current_style);
        }
    }

    fn index(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_region_up(self.scroll_top, self.scroll_bottom);
        } else if self.cursor_row + 1 >= self.rows {
            self.scroll_region_up(0, self.rows.saturating_sub(1));
        } else {
            self.cursor_row += 1;
        }
    }

    fn next_line(&mut self) {
        self.carriage_return();
        self.index();
    }

    fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_region_down(self.scroll_top, self.scroll_bottom);
        } else {
            self.cursor_row = self.cursor_row.saturating_sub(1);
        }
    }

    fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    fn tab(&mut self) {
        let next = ((self.cursor_col / 8) + 1) * 8;
        self.cursor_col = next.min(self.cols.saturating_sub(1));
    }

    fn put_char(&mut self, ch: char) {
        let ch = if self.dec_special_graphics {
            dec_special_graphics_char(ch)
        } else {
            ch
        };
        if self.cursor_col >= self.cols {
            if self.auto_wrap {
                self.newline();
            } else {
                self.cursor_col = self.cols.saturating_sub(1);
            }
        }
        if self.insert_mode {
            self.insert_chars(1);
        }
        self.put_at(self.cursor_col, self.cursor_row, ch);
        if self.auto_wrap || self.cursor_col + 1 < self.cols {
            self.cursor_col += 1;
        }
    }

    fn handle_osc(&mut self, params: &[&[u8]]) {
        let Some(kind) = params
            .first()
            .and_then(|param| std::str::from_utf8(param).ok())
        else {
            return;
        };
        match kind {
            "0" | "1" | "2" => self.set_title_from_osc(params),
            "9" => self.notification_from_osc9(params),
            "52" => self.forward_osc52_clipboard(params),
            "777" => self.notification_from_osc777(params),
            _ => {}
        }
    }

    fn set_title_from_osc(&mut self, params: &[&[u8]]) {
        let title = join_osc_utf8_params(params.get(1..).unwrap_or_default());
        if !title.is_empty() {
            self.title = Some(title.clone());
            self.queue_surface_event(SurfaceEvent::TitleChanged(title));
        }
    }

    fn notification_from_osc9(&mut self, params: &[&[u8]]) {
        let body = join_osc_utf8_params(params.get(1..).unwrap_or_default());
        if !body.is_empty() {
            self.queue_surface_event(SurfaceEvent::Notification {
                title: self.title.clone().unwrap_or_else(|| "kittwm".to_string()),
                body,
            });
        }
    }

    fn notification_from_osc777(&mut self, params: &[&[u8]]) {
        let Some(kind) = params
            .get(1)
            .and_then(|part| std::str::from_utf8(part).ok())
        else {
            return;
        };
        if kind != "notify" {
            return;
        }
        let title = params
            .get(2)
            .and_then(|part| std::str::from_utf8(part).ok())
            .unwrap_or("kittwm")
            .to_string();
        let body = join_osc_utf8_params(params.get(3..).unwrap_or_default());
        self.queue_surface_event(SurfaceEvent::Notification { title, body });
    }

    fn forward_osc52_clipboard(&mut self, params: &[&[u8]]) {
        let selector = params
            .get(1)
            .and_then(|param| std::str::from_utf8(param).ok())
            .filter(|selector| !selector.is_empty())
            .unwrap_or("c");
        if !selector
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-' | b'_'))
        {
            return;
        }
        let payload = join_osc_utf8_params(params.get(2..).unwrap_or_default());
        if payload.is_empty() || payload == "?" {
            return;
        }
        if payload.len() > 1_048_576 {
            return;
        }
        if base64::engine::general_purpose::STANDARD
            .decode(payload.as_bytes())
            .is_err()
        {
            return;
        }
        self.queue_surface_event(SurfaceEvent::ClipboardSet {
            selection: selector.to_string(),
            payload_base64: payload.clone(),
        });
        self.queue_host_sequence(format!("\x1b]52;{selector};{payload}\x07"));
    }

    fn clear_line_range(&mut self, start: u16, end_inclusive: u16) {
        for col in start..=end_inclusive.min(self.cols.saturating_sub(1)) {
            self.put_at(col, self.cursor_row, ' ');
        }
    }

    fn clear_screen_range(&mut self, start_row: u16, start_col: u16, end_row: u16, end_col: u16) {
        for row in start_row..=end_row.min(self.rows.saturating_sub(1)) {
            let first_col = if row == start_row { start_col } else { 0 };
            let last_col = if row == end_row {
                end_col.min(self.cols.saturating_sub(1))
            } else {
                self.cols.saturating_sub(1)
            };
            for col in first_col..=last_col {
                self.put_at(col, row, ' ');
            }
        }
    }

    fn insert_chars(&mut self, count: u16) {
        if self.cursor_col >= self.cols {
            return;
        }
        let count = count.min(self.cols - self.cursor_col);
        for col in (self.cursor_col..self.cols - count).rev() {
            self.put_cell_at(
                col + count,
                self.cursor_row,
                self.get_cell_at(col, self.cursor_row),
            );
        }
        self.clear_line_range(self.cursor_col, self.cursor_col + count - 1);
    }

    fn delete_chars(&mut self, count: u16) {
        if self.cursor_col >= self.cols {
            return;
        }
        let count = count.min(self.cols - self.cursor_col);
        for col in self.cursor_col + count..self.cols {
            self.put_cell_at(
                col - count,
                self.cursor_row,
                self.get_cell_at(col, self.cursor_row),
            );
        }
        self.clear_line_range(self.cols - count, self.cols.saturating_sub(1));
    }

    fn erase_chars(&mut self, count: u16) {
        if self.cursor_col >= self.cols {
            return;
        }
        let end = (self.cursor_col + count.saturating_sub(1)).min(self.cols.saturating_sub(1));
        self.clear_line_range(self.cursor_col, end);
    }

    fn insert_lines(&mut self, count: u16) {
        if self.cursor_row >= self.rows {
            return;
        }
        let count = count.min(self.rows - self.cursor_row);
        let cols = usize::from(self.cols);
        let start = usize::from(self.cursor_row) * cols;
        let shift = usize::from(count) * cols;
        let end = self.cells.len().saturating_sub(shift);
        self.cells.copy_within(start..end, start + shift);
        for cell in &mut self.cells[start..start + shift] {
            *cell = TerminalCell::blank(self.current_style);
        }
    }

    fn delete_lines(&mut self, count: u16) {
        if self.cursor_row >= self.rows {
            return;
        }
        let count = count.min(self.rows - self.cursor_row);
        let cols = usize::from(self.cols);
        let start = usize::from(self.cursor_row) * cols;
        let shift = usize::from(count) * cols;
        self.cells.copy_within(start + shift.., start);
        let clear_start = self.cells.len().saturating_sub(shift);
        for cell in &mut self.cells[clear_start..] {
            *cell = TerminalCell::blank(self.current_style);
        }
    }

    fn enter_alternate_screen(&mut self) {
        if self.alt_screen.is_some() {
            self.cells.fill(TerminalCell::blank(self.current_style));
            self.cursor_col = 0;
            self.cursor_row = 0;
            return;
        }
        let normal_cells = std::mem::replace(
            &mut self.cells,
            vec![
                TerminalCell::blank(self.current_style);
                usize::from(self.cols) * usize::from(self.rows)
            ],
        );
        self.alt_screen = Some(AlternateScreen {
            normal_cells,
            normal_cursor_col: self.cursor_col,
            normal_cursor_row: self.cursor_row,
            normal_scroll_top: self.scroll_top,
            normal_scroll_bottom: self.scroll_bottom,
        });
        self.cursor_col = 0;
        self.cursor_row = 0;
        self.reset_scroll_region();
    }

    fn leave_alternate_screen(&mut self) {
        if let Some(alt) = self.alt_screen.take() {
            self.cells = alt.normal_cells;
            self.cursor_col = alt.normal_cursor_col.min(self.cols.saturating_sub(1));
            self.cursor_row = alt.normal_cursor_row.min(self.rows.saturating_sub(1));
            self.scroll_top = alt.normal_scroll_top.min(self.rows.saturating_sub(1));
            self.scroll_bottom = alt.normal_scroll_bottom.min(self.rows.saturating_sub(1));
            if self.scroll_top >= self.scroll_bottom {
                self.reset_scroll_region();
            }
        }
    }

    fn reset_scroll_region(&mut self) {
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
    }

    fn set_scroll_region(&mut self, params: &Params) {
        let mut iter = params.iter();
        let top = iter.next().and_then(|p| p.first().copied()).unwrap_or(1) as u16;
        let bottom = iter
            .next()
            .and_then(|p| p.first().copied())
            .unwrap_or(self.rows) as u16;
        if top == 0 && bottom == 0 {
            self.reset_scroll_region();
            return;
        }
        let top = top
            .max(1)
            .saturating_sub(1)
            .min(self.rows.saturating_sub(1));
        let bottom = bottom
            .max(1)
            .saturating_sub(1)
            .min(self.rows.saturating_sub(1));
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.cursor_col = 0;
            self.cursor_row = 0;
        }
    }

    fn set_origin_mode(&mut self, enabled: bool) {
        self.origin_mode = enabled;
        self.cursor_col = 0;
        self.cursor_row = if enabled { self.scroll_top } else { 0 };
    }

    fn address_cursor(&mut self, row: u16, col: u16) {
        self.cursor_col = col.saturating_sub(1).min(self.cols.saturating_sub(1));
        if self.origin_mode {
            let relative = row.saturating_sub(1);
            self.cursor_row = self
                .scroll_top
                .saturating_add(relative)
                .min(self.scroll_bottom)
                .min(self.rows.saturating_sub(1));
        } else {
            self.cursor_row = row.saturating_sub(1).min(self.rows.saturating_sub(1));
        }
    }

    fn save_cursor(&mut self) {
        self.saved_cursor_col = self.cursor_col;
        self.saved_cursor_row = self.cursor_row;
    }

    fn restore_cursor(&mut self) {
        self.cursor_col = self.saved_cursor_col.min(self.cols.saturating_sub(1));
        self.cursor_row = self.saved_cursor_row.min(self.rows.saturating_sub(1));
    }

    fn set_mouse_mode(&mut self, mode: u16, enabled: bool) {
        match mode {
            1000 => self.mouse_modes.basic = enabled,
            1002 => self.mouse_modes.button_motion = enabled,
            1003 => self.mouse_modes.all_motion = enabled,
            1006 => self.mouse_modes.sgr = enabled,
            _ => {}
        }
    }

    fn reset_modes(&mut self) {
        self.cursor_col = 0;
        self.cursor_row = 0;
        self.saved_cursor_col = 0;
        self.saved_cursor_row = 0;
        self.cursor_visible = true;
        self.origin_mode = false;
        self.auto_wrap = true;
        self.application_cursor_keys = false;
        self.insert_mode = false;
        self.dec_special_graphics = false;
        self.current_style = TerminalStyle::default();
        self.bracketed_paste = false;
        self.focus_reporting = false;
        self.mouse_modes = MouseReportingModes::default();
        self.reset_scroll_region();
    }

    fn soft_reset(&mut self) {
        self.reset_modes();
    }

    fn full_reset(&mut self) {
        self.reset_modes();
        self.alt_screen = None;
        self.cells.fill(TerminalCell::blank(self.current_style));
        self.scrollback.clear();
    }

    fn device_status_report(&mut self, mode: u16) {
        match mode {
            5 => self.queue_response(b"\x1b[0n"),
            6 => self.queue_response(format!(
                "\x1b[{};{}R",
                self.cursor_row.saturating_add(1),
                self.cursor_col.saturating_add(1)
            )),
            _ => {}
        }
    }

    fn apply_sgr(&mut self, params: &Params) {
        if params.is_empty() {
            self.current_style = TerminalStyle::default();
            return;
        }
        let values = params
            .iter()
            .flat_map(|param| param.iter().copied())
            .collect::<Vec<_>>();
        let mut idx = 0;
        while idx < values.len() {
            let value = values[idx];
            match value {
                0 => self.current_style = TerminalStyle::default(),
                1 => self.current_style.bold = true,
                22 => self.current_style.bold = false,
                7 => self.current_style.reverse = true,
                27 => self.current_style.reverse = false,
                30..=37 => self.current_style.fg = ansi_color(value as u8, false),
                39 => self.current_style.fg = None,
                40..=47 => self.current_style.bg = ansi_color((value - 10) as u8, false),
                49 => self.current_style.bg = None,
                90..=97 => self.current_style.fg = ansi_color((value - 60) as u8, true),
                100..=107 => self.current_style.bg = ansi_color((value - 70) as u8, true),
                38 | 48 => {
                    if let Some((color, consumed)) = parse_extended_sgr_color(&values[idx + 1..]) {
                        if value == 38 {
                            self.current_style.fg = Some(color);
                        } else {
                            self.current_style.bg = Some(color);
                        }
                        idx += consumed;
                    }
                }
                _ => {}
            }
            idx += 1;
        }
    }
}

fn parse_extended_sgr_color(values: &[u16]) -> Option<(TerminalColor, usize)> {
    match values.first().copied()? {
        5 => {
            let index = values.get(1).copied()? as u8;
            Some((ansi_256_color(index), 2))
        }
        2 => {
            let r = values.get(1).copied()?.min(255) as u8;
            let g = values.get(2).copied()?.min(255) as u8;
            let b = values.get(3).copied()?.min(255) as u8;
            Some((TerminalColor(r, g, b), 4))
        }
        _ => None,
    }
}

fn ansi_256_color(index: u8) -> TerminalColor {
    match index {
        0..=7 => ansi_color(30 + index, false).unwrap_or(TerminalColor(0xd7, 0xf8, 0xff)),
        8..=15 => ansi_color(30 + (index - 8), true).unwrap_or(TerminalColor(0xd7, 0xf8, 0xff)),
        16..=231 => {
            let n = index - 16;
            let r = n / 36;
            let g = (n / 6) % 6;
            let b = n % 6;
            TerminalColor(
                color_cube_component(r),
                color_cube_component(g),
                color_cube_component(b),
            )
        }
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            TerminalColor(level, level, level)
        }
    }
}

fn color_cube_component(value: u8) -> u8 {
    if value == 0 {
        0
    } else {
        55 + value * 40
    }
}

fn ansi_color(code: u8, bright: bool) -> Option<TerminalColor> {
    let palette = if bright {
        [
            TerminalColor(0x66, 0x6a, 0x73),
            TerminalColor(0xff, 0x6b, 0x6b),
            TerminalColor(0x69, 0xdb, 0x7c),
            TerminalColor(0xff, 0xd4, 0x3b),
            TerminalColor(0x4d, 0xab, 0xf7),
            TerminalColor(0xda, 0x77, 0xf2),
            TerminalColor(0x3b, 0xf0, 0xe4),
            TerminalColor(0xff, 0xff, 0xff),
        ]
    } else {
        [
            TerminalColor(0x1b, 0x1f, 0x2a),
            TerminalColor(0xe0, 0x31, 0x31),
            TerminalColor(0x2f, 0x9e, 0x44),
            TerminalColor(0xf0, 0x8c, 0x00),
            TerminalColor(0x19, 0x71, 0xc2),
            TerminalColor(0xae, 0x3e, 0xc9),
            TerminalColor(0x0c, 0x85, 0x99),
            TerminalColor(0xd7, 0xf8, 0xff),
        ]
    };
    palette.get(usize::from(code.saturating_sub(30))).copied()
}

fn dec_special_graphics_char(ch: char) -> char {
    match ch {
        '_' => ' ',
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => ch,
    }
}

fn join_osc_utf8_params(parts: &[&[u8]]) -> String {
    let mut out = String::new();
    for part in parts {
        let Ok(text) = std::str::from_utf8(part) else {
            continue;
        };
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(text);
    }
    out
}

fn resize_cells(
    old: &[TerminalCell],
    old_cols: u16,
    old_rows: u16,
    cols: u16,
    rows: u16,
) -> Vec<TerminalCell> {
    let mut cells =
        vec![TerminalCell::blank(TerminalStyle::default()); usize::from(cols) * usize::from(rows)];
    let copy_rows = rows.min(old_rows);
    let copy_cols = cols.min(old_cols);
    for row in 0..copy_rows {
        for col in 0..copy_cols {
            let old_idx = usize::from(row) * usize::from(old_cols) + usize::from(col);
            let new_idx = usize::from(row) * usize::from(cols) + usize::from(col);
            if let Some(ch) = old.get(old_idx) {
                cells[new_idx] = *ch;
            }
        }
    }
    cells
}

impl Perform for TerminalState {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.carriage_return(),
            b'\t' => self.tab(),
            0x07 => self.queue_surface_event(SurfaceEvent::Bell {
                visual: true,
                audible: true,
            }),
            0x08 => self.cursor_col = self.cursor_col.saturating_sub(1),
            0x84 => self.index(),
            0x85 => self.next_line(),
            0x8d => self.reverse_index(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.handle_osc(params);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let first_raw = params
            .iter()
            .next()
            .and_then(|p| p.first().copied())
            .unwrap_or(0) as u16;
        let first_count = if first_raw == 0 { 1 } else { first_raw };
        let is_dec_private = intermediates.contains(&b'?');
        let has_alt_screen_mode = params.iter().any(|param| {
            param
                .first()
                .copied()
                .is_some_and(|mode| matches!(mode, 47 | 1047 | 1049))
        });
        let has_bracketed_paste_mode = params
            .iter()
            .any(|param| param.first().copied() == Some(2004));
        let has_focus_reporting_mode = params
            .iter()
            .any(|param| param.first().copied() == Some(1004));
        let has_cursor_visibility_mode = params
            .iter()
            .any(|param| param.first().copied() == Some(25));
        let has_application_cursor_mode =
            params.iter().any(|param| param.first().copied() == Some(1));
        let has_origin_mode = params.iter().any(|param| param.first().copied() == Some(6));
        let has_autowrap_mode = params.iter().any(|param| param.first().copied() == Some(7));
        let mouse_modes = params
            .iter()
            .filter_map(|param| param.first().copied())
            .filter(|mode| matches!(mode, 1000 | 1002 | 1003 | 1006))
            .collect::<Vec<_>>();
        match action {
            '@' => self.insert_chars(first_count),
            'A' => self.cursor_row = self.cursor_row.saturating_sub(first_count),
            'B' => {
                self.cursor_row = (self.cursor_row + first_count).min(self.rows.saturating_sub(1))
            }
            'C' | 'a' => {
                self.cursor_col = (self.cursor_col + first_count).min(self.cols.saturating_sub(1))
            }
            'D' => self.cursor_col = self.cursor_col.saturating_sub(first_count),
            'E' => {
                self.cursor_row = (self.cursor_row + first_count).min(self.rows.saturating_sub(1));
                self.cursor_col = 0;
            }
            'F' => {
                self.cursor_row = self.cursor_row.saturating_sub(first_count);
                self.cursor_col = 0;
            }
            'G' => {
                self.cursor_col = first_count
                    .saturating_sub(1)
                    .min(self.cols.saturating_sub(1))
            }
            'd' => {
                self.cursor_row = first_count
                    .saturating_sub(1)
                    .min(self.rows.saturating_sub(1))
            }
            'e' => {
                self.cursor_row = (self.cursor_row + first_count).min(self.rows.saturating_sub(1))
            }
            'H' | 'f' => {
                let mut iter = params.iter();
                let row = iter.next().and_then(|p| p.first().copied()).unwrap_or(1) as u16;
                let col = iter.next().and_then(|p| p.first().copied()).unwrap_or(1) as u16;
                self.address_cursor(row, col);
            }
            'h' if !is_dec_private && first_raw == 4 => self.insert_mode = true,
            'h' if is_dec_private && has_alt_screen_mode => self.enter_alternate_screen(),
            'h' if is_dec_private && has_application_cursor_mode => {
                self.application_cursor_keys = true
            }
            'h' if is_dec_private && has_bracketed_paste_mode => self.bracketed_paste = true,
            'h' if is_dec_private && has_focus_reporting_mode => self.focus_reporting = true,
            'h' if is_dec_private && has_cursor_visibility_mode => self.cursor_visible = true,
            'h' if is_dec_private && has_origin_mode => self.set_origin_mode(true),
            'h' if is_dec_private && has_autowrap_mode => self.auto_wrap = true,
            'h' if is_dec_private && !mouse_modes.is_empty() => {
                for mode in mouse_modes {
                    self.set_mouse_mode(mode, true);
                }
            }
            'J' => match first_raw {
                0 => self.clear_screen_range(
                    self.cursor_row,
                    self.cursor_col,
                    self.rows.saturating_sub(1),
                    self.cols.saturating_sub(1),
                ),
                1 => self.clear_screen_range(0, 0, self.cursor_row, self.cursor_col),
                2 => self.cells.fill(TerminalCell::blank(self.current_style)),
                _ => {}
            },
            'K' => match first_raw {
                0 => self.clear_line_range(self.cursor_col, self.cols.saturating_sub(1)),
                1 => self.clear_line_range(0, self.cursor_col),
                2 => self.clear_line_range(0, self.cols.saturating_sub(1)),
                _ => {}
            },
            'L' => self.insert_lines(first_count),
            'l' if !is_dec_private && first_raw == 4 => self.insert_mode = false,
            'l' if is_dec_private && has_alt_screen_mode => self.leave_alternate_screen(),
            'l' if is_dec_private && has_application_cursor_mode => {
                self.application_cursor_keys = false
            }
            'l' if is_dec_private && has_bracketed_paste_mode => self.bracketed_paste = false,
            'l' if is_dec_private && has_focus_reporting_mode => self.focus_reporting = false,
            'l' if is_dec_private && has_cursor_visibility_mode => self.cursor_visible = false,
            'l' if is_dec_private && has_origin_mode => self.set_origin_mode(false),
            'l' if is_dec_private && has_autowrap_mode => self.auto_wrap = false,
            'l' if is_dec_private && !mouse_modes.is_empty() => {
                for mode in mouse_modes {
                    self.set_mouse_mode(mode, false);
                }
            }
            'M' => self.delete_lines(first_count),
            'm' => self.apply_sgr(params),
            'n' if !is_dec_private => self.device_status_report(first_raw),
            'p' if intermediates.contains(&b'!') => self.soft_reset(),
            'P' => self.delete_chars(first_count),
            'r' => self.set_scroll_region(params),
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'X' => self.erase_chars(first_count),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if intermediates == [b'('] {
            match byte {
                b'0' => self.dec_special_graphics = true,
                b'B' => self.dec_special_graphics = false,
                _ => {}
            }
            return;
        }
        match byte {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => self.index(),
            b'E' => self.next_line(),
            b'M' => self.reverse_index(),
            b'c' => self.full_reset(),
            _ => {}
        }
    }
}

#[cfg(test)]
fn render_terminal_rgba(state: &TerminalState, cell_w: u32, cell_h: u32) -> Vec<u8> {
    render_terminal_render_state_rgba(&state.render_state(), cell_w, cell_h)
}

fn render_terminal_render_state_rgba(
    state: &TerminalRenderState,
    cell_w: u32,
    cell_h: u32,
) -> Vec<u8> {
    let width = u32::from(state.cols) * cell_w;
    let height = u32::from(state.rows) * cell_h;
    let default_bg = default_terminal_bg_color();
    let mut rgba = vec![0; (width as usize) * (height as usize) * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[0] = default_bg.0;
        px[1] = default_bg.1;
        px[2] = default_bg.2;
        px[3] = 0xff;
    }
    let cols = usize::from(state.cols);
    if cols == 0 {
        return rgba;
    }
    for (row, cells) in state.cells.chunks(cols).enumerate() {
        let row = row as u16;
        for (col, cell) in cells.iter().enumerate() {
            if is_blank_default_terminal_cell(cell) {
                continue;
            }
            let col = col as u16;
            let (fg, bg) = terminal_cell_colors(cell.style);
            if should_fill_terminal_cell_background(bg, default_bg) {
                fill_cell_background(&mut rgba, width, col, row, cell_w, cell_h, bg);
            }
            if cell.ch == ' ' {
                continue;
            }
            draw_terminal_glyph(&mut rgba, width, col, row, cell_w, cell_h, cell.ch, fg);
        }
    }
    if state.cursor_visible {
        draw_terminal_cursor(&mut rgba, width, state, cell_w, cell_h);
    }
    rgba
}

fn draw_terminal_cursor(
    rgba: &mut [u8],
    width: u32,
    state: &TerminalRenderState,
    cell_w: u32,
    cell_h: u32,
) {
    if state.cursor_col >= state.cols || state.cursor_row >= state.rows {
        return;
    }
    let idx =
        usize::from(state.cursor_row) * usize::from(state.cols) + usize::from(state.cursor_col);
    let cell = state
        .cells
        .get(idx)
        .copied()
        .unwrap_or_else(|| TerminalCell::blank(TerminalStyle::default()));
    let (fg, bg) = terminal_cell_colors(cell.style);
    let cursor = if cell.ch == ' ' { fg } else { bg };
    let x0 = u32::from(state.cursor_col) * cell_w;
    let y0 = u32::from(state.cursor_row) * cell_h;
    let start_y = cell_h.saturating_sub(3);
    for y in start_y..cell_h {
        let mut idx = rgba_pixel_index(width, x0, y0 + y);
        for _ in 0..cell_w {
            set_rgba_pixel_at_index(rgba, idx, cursor);
            idx += 4;
        }
    }
}

fn is_blank_default_terminal_cell(cell: &TerminalCell) -> bool {
    cell.ch == ' ' && cell.style == TerminalStyle::default()
}

fn default_terminal_bg_color() -> TerminalColor {
    TerminalColor(0x08, 0x0d, 0x14)
}

fn should_fill_terminal_cell_background(bg: TerminalColor, default_bg: TerminalColor) -> bool {
    bg != default_bg
}

fn terminal_cell_colors(style: TerminalStyle) -> (TerminalColor, TerminalColor) {
    let mut fg = style.fg.unwrap_or(TerminalColor(0xd7, 0xf8, 0xff));
    let mut bg = style.bg.unwrap_or(default_terminal_bg_color());
    if style.bold && style.fg.is_some() {
        fg = brighten_color(fg);
    }
    if style.reverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    (fg, bg)
}

fn brighten_color(color: TerminalColor) -> TerminalColor {
    TerminalColor(
        color.0.saturating_add(0x30),
        color.1.saturating_add(0x30),
        color.2.saturating_add(0x30),
    )
}

fn fill_cell_background(
    rgba: &mut [u8],
    width: u32,
    col: u16,
    row: u16,
    cell_w: u32,
    cell_h: u32,
    color: TerminalColor,
) {
    let x0 = u32::from(col) * cell_w;
    let y0 = u32::from(row) * cell_h;
    for y in 0..cell_h {
        let mut idx = rgba_pixel_index(width, x0, y0 + y);
        for _ in 0..cell_w {
            set_rgba_pixel_at_index(rgba, idx, color);
            idx += 4;
        }
    }
}

fn draw_terminal_glyph(
    rgba: &mut [u8],
    width: u32,
    col: u16,
    row: u16,
    cell_w: u32,
    cell_h: u32,
    ch: char,
    color: TerminalColor,
) {
    if draw_box_drawing_glyph(rgba, width, col, row, cell_w, cell_h, ch, color) {
        return;
    }
    if draw_terminal_font_glyph(rgba, width, col, row, cell_w, cell_h, ch, color) {
        return;
    }
    let bitmap = terminal_bitmap_glyph(ch);
    let x0 = u32::from(col) * cell_w;
    let y0 = u32::from(row) * cell_h;
    let scale_x = (cell_w.saturating_sub(2) / 5).max(1);
    let scale_y = (cell_h.saturating_sub(2) / 7).max(1);
    let glyph_w = scale_x * 5;
    let glyph_h = scale_y * 7;
    let left = 1 + cell_w.saturating_sub(glyph_w) / 2;
    let top = 1 + cell_h.saturating_sub(glyph_h) / 2;
    for (gy, bits) in bitmap.iter().enumerate() {
        for gx in 0..5u32 {
            if bits & (1 << (4 - gx)) == 0 {
                continue;
            }
            for sy in 0..scale_y {
                for sx in 0..scale_x {
                    let cell_x = left + gx * scale_x + sx;
                    let cell_y = top + gy as u32 * scale_y + sy;
                    if cell_x >= cell_w || cell_y >= cell_h {
                        continue;
                    }
                    set_rgba_pixel_in_bounds(rgba, width, x0 + cell_x, y0 + cell_y, color);
                }
            }
        }
    }
}

static TERMINAL_FONT: OnceLock<Option<TerminalFont>> = OnceLock::new();
const TERMINAL_FONT_GLYPH_CACHE_MAX: usize = 4096;
static TERMINAL_FONT_GLYPHS: OnceLock<
    Mutex<std::collections::HashMap<(char, u32), TerminalFontGlyph>>,
> = OnceLock::new();

#[derive(Clone)]
struct TerminalFontGlyph {
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
}

struct TerminalFont {
    font: fontdue::Font,
}

fn terminal_font() -> Option<&'static TerminalFont> {
    TERMINAL_FONT
        .get_or_init(|| load_terminal_font().ok())
        .as_ref()
}

fn load_terminal_font() -> Result<TerminalFont> {
    let path = discover_terminal_font_path().context("discover terminal font")?;
    let bytes = std::fs::read(&path).with_context(|| format!("read font {}", path.display()))?;
    let settings = fontdue::FontSettings {
        collection_index: 0,
        scale: 40.0,
        load_substitutions: true,
    };
    let font = fontdue::Font::from_bytes(bytes, settings)
        .map_err(|err| anyhow!("parse font {}: {err}", path.display()))?;
    Ok(TerminalFont { font })
}

fn discover_terminal_font_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KITTUI_TERMINAL_FONT") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    for root in terminal_font_roots() {
        if !root.exists() {
            continue;
        }
        if let Some(path) = find_fira_code_font(&root, 4) {
            return Some(path);
        }
    }
    None
}

fn terminal_font_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join("Library/Fonts"));
        roots.push(home.join(".local/share/fonts"));
        roots.push(home.join(".fonts"));
    }
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        roots.push(xdg_data_home.join("fonts"));
    }
    roots.extend([
        PathBuf::from("/opt/homebrew/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/fonts"),
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts"),
    ]);
    roots.dedup();
    roots
}

fn find_fira_code_font(root: &Path, depth: usize) -> Option<PathBuf> {
    let mut candidates = Vec::<(u8, PathBuf)>::new();
    collect_fira_code_fonts(root, depth, &mut candidates);
    candidates
        .into_iter()
        .min_by(|(a_score, a_path), (b_score, b_path)| {
            a_score.cmp(b_score).then_with(|| a_path.cmp(b_path))
        })
        .map(|(_, path)| path)
}

fn collect_fira_code_fonts(root: &Path, depth: usize, candidates: &mut Vec<(u8, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
            continue;
        }
        if let Some(score) = fira_code_font_score(&path) {
            candidates.push((score, path));
        }
    }
    if depth == 0 {
        return;
    }
    dirs.sort();
    for dir in dirs {
        collect_fira_code_fonts(&dir, depth - 1, candidates);
    }
}

fn fira_code_font_score(path: &Path) -> Option<u8> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let is_font = name.ends_with(".ttf") || name.ends_with(".otf");
    let normalized = normalize_font_filename(&name);
    if !is_font || !normalized.contains("firacode") || !normalized.contains("regular") {
        return None;
    }
    if normalized.contains("nerd") && normalized.contains("mono") {
        Some(0)
    } else if normalized.contains("nerd") {
        Some(1)
    } else {
        Some(2)
    }
}

fn normalize_font_filename(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn draw_terminal_font_glyph(
    rgba: &mut [u8],
    width: u32,
    col: u16,
    row: u16,
    cell_w: u32,
    cell_h: u32,
    ch: char,
    color: TerminalColor,
) -> bool {
    let Some(font) = terminal_font() else {
        return false;
    };
    let px = (cell_h as f32 * 0.82).max(6.0);
    let Some(glyph) = cached_terminal_font_glyph(font, ch, px) else {
        return false;
    };
    let metrics = &glyph.metrics;
    let bitmap = &glyph.bitmap;
    let x0 = i32::from(col) * cell_w as i32;
    let y0 = i32::from(row) * cell_h as i32;
    let left = ((cell_w as i32 - metrics.width as i32) / 2).max(0) + metrics.xmin;
    let baseline = (cell_h as f32 * 0.78) as i32;
    let top = baseline - metrics.height as i32 - metrics.ymin;
    for gy in 0..metrics.height {
        for gx in 0..metrics.width {
            let alpha = bitmap[gy * metrics.width + gx];
            if alpha == 0 {
                continue;
            }
            let px = x0 + left + gx as i32;
            let py = y0 + top + gy as i32;
            if px < x0 || py < y0 || px >= x0 + cell_w as i32 || py >= y0 + cell_h as i32 {
                continue;
            }
            blend_rgba_pixel_in_bounds(rgba, width, px as u32, py as u32, color, alpha);
        }
    }
    true
}

fn cached_terminal_font_glyph(font: &TerminalFont, ch: char, px: f32) -> Option<TerminalFontGlyph> {
    let px_key = (px * 64.0).round().max(0.0) as u32;
    let cache = TERMINAL_FONT_GLYPHS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(glyph) = cache.lock().get(&(ch, px_key)).cloned() {
        return Some(glyph);
    }
    let (metrics, bitmap) = font.font.rasterize(ch, px);
    if metrics.width == 0 || metrics.height == 0 || bitmap.is_empty() {
        return None;
    }
    let glyph = TerminalFontGlyph { metrics, bitmap };
    let mut cache = cache.lock();
    prune_terminal_font_glyph_cache_if_full(&mut cache);
    cache.insert((ch, px_key), glyph.clone());
    Some(glyph)
}

fn prune_terminal_font_glyph_cache_if_full(
    cache: &mut std::collections::HashMap<(char, u32), TerminalFontGlyph>,
) -> bool {
    if !terminal_font_glyph_cache_should_prune(cache.len()) {
        return false;
    }
    cache.clear();
    true
}

fn terminal_font_glyph_cache_should_prune(len: usize) -> bool {
    len >= TERMINAL_FONT_GLYPH_CACHE_MAX
}

#[cfg(test)]
fn clear_terminal_font_glyph_cache_for_tests() {
    if let Some(cache) = TERMINAL_FONT_GLYPHS.get() {
        cache.lock().clear();
    }
}

#[cfg(test)]
fn terminal_font_glyph_cache_len_for_tests() -> usize {
    TERMINAL_FONT_GLYPHS
        .get()
        .map(|cache| cache.lock().len())
        .unwrap_or(0)
}

fn rgba_pixel_index(width: u32, x: u32, y: u32) -> usize {
    ((y * width + x) as usize) * 4
}

#[cfg(test)]
fn blend_rgba_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: TerminalColor, alpha: u8) {
    let idx = rgba_pixel_index(width, x, y);
    if idx + 3 >= rgba.len() {
        return;
    }
    blend_rgba_pixel_at_index(rgba, idx, color, alpha);
}

fn blend_rgba_pixel_in_bounds(
    rgba: &mut [u8],
    width: u32,
    x: u32,
    y: u32,
    color: TerminalColor,
    alpha: u8,
) {
    let idx = rgba_pixel_index(width, x, y);
    blend_rgba_pixel_at_index(rgba, idx, color, alpha);
}

fn blend_rgba_pixel_at_index(rgba: &mut [u8], idx: usize, color: TerminalColor, alpha: u8) {
    let a = u16::from(alpha);
    let inv = 255u16.saturating_sub(a);
    rgba[idx] = ((u16::from(color.0) * a + u16::from(rgba[idx]) * inv) / 255) as u8;
    rgba[idx + 1] = ((u16::from(color.1) * a + u16::from(rgba[idx + 1]) * inv) / 255) as u8;
    rgba[idx + 2] = ((u16::from(color.2) * a + u16::from(rgba[idx + 2]) * inv) / 255) as u8;
    rgba[idx + 3] = 0xff;
}

#[cfg(test)]
fn set_rgba_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: TerminalColor) {
    let idx = rgba_pixel_index(width, x, y);
    if idx + 3 >= rgba.len() {
        return;
    }
    set_rgba_pixel_at_index(rgba, idx, color);
}

fn set_rgba_pixel_in_bounds(rgba: &mut [u8], width: u32, x: u32, y: u32, color: TerminalColor) {
    let idx = rgba_pixel_index(width, x, y);
    set_rgba_pixel_at_index(rgba, idx, color);
}

fn set_rgba_pixel_at_index(rgba: &mut [u8], idx: usize, color: TerminalColor) {
    rgba[idx] = color.0;
    rgba[idx + 1] = color.1;
    rgba[idx + 2] = color.2;
    rgba[idx + 3] = 0xff;
}

fn draw_box_drawing_glyph(
    rgba: &mut [u8],
    width: u32,
    col: u16,
    row: u16,
    cell_w: u32,
    cell_h: u32,
    ch: char,
    color: TerminalColor,
) -> bool {
    let (left, right, up, down) = match ch {
        '─' => (true, true, false, false),
        '│' => (false, false, true, true),
        '┌' => (false, true, false, true),
        '┐' => (true, false, false, true),
        '└' => (false, true, true, false),
        '┘' => (true, false, true, false),
        '├' => (false, true, true, true),
        '┤' => (true, false, true, true),
        '┬' => (true, true, false, true),
        '┴' => (true, true, true, false),
        '┼' => (true, true, true, true),
        _ => return false,
    };
    let x0 = u32::from(col) * cell_w;
    let y0 = u32::from(row) * cell_h;
    let cx = cell_w / 2;
    let cy = cell_h / 2;
    let thickness = (cell_w.min(cell_h) / 8).max(1);
    let mut draw_rect = |x_start: u32, y_start: u32, w: u32, h: u32| {
        for y in y_start..y_start.saturating_add(h).min(cell_h) {
            for x in x_start..x_start.saturating_add(w).min(cell_w) {
                set_rgba_pixel_in_bounds(rgba, width, x0 + x, y0 + y, color);
            }
        }
    };
    if left {
        draw_rect(0, cy.saturating_sub(thickness / 2), cx + 1, thickness);
    }
    if right {
        draw_rect(cx, cy.saturating_sub(thickness / 2), cell_w - cx, thickness);
    }
    if up {
        draw_rect(cx.saturating_sub(thickness / 2), 0, thickness, cy + 1);
    }
    if down {
        draw_rect(cx.saturating_sub(thickness / 2), cy, thickness, cell_h - cy);
    }
    true
}

fn terminal_bitmap_glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ',' => [0, 0, 0, 0, 0, 0b01100, 0b01000],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        ';' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01000, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '\\' => [
            0b10000, 0b01000, 0b01000, 0b00100, 0b00010, 0b00010, 0b00001,
        ],
        '|' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        '$' => [
            0b00100, 0b01111, 0b10100, 0b01110, 0b00101, 0b11110, 0b00100,
        ],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        '@' => [
            0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01110,
        ],
        '*' => [0, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0],
        '=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
        '>' => [
            0b10000, 0b01000, 0b00100, 0b00010, 0b00100, 0b01000, 0b10000,
        ],
        '<' => [
            0b00001, 0b00010, 0b00100, 0b01000, 0b00100, 0b00010, 0b00001,
        ],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '[' => [
            0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
        ],
        ']' => [
            0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
        ],
        '{' => [
            0b00110, 0b01000, 0b01000, 0b10000, 0b01000, 0b01000, 0b00110,
        ],
        '}' => [
            0b01100, 0b00010, 0b00010, 0b00001, 0b00010, 0b00010, 0b01100,
        ],
        '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        '\'' => [0b00100, 0b00100, 0b01000, 0, 0, 0, 0],
        '"' => [0b01010, 0b01010, 0, 0, 0, 0, 0],
        '`' => [0b01000, 0b00100, 0, 0, 0, 0, 0],
        '~' => [0, 0, 0b01000, 0b10101, 0b00010, 0, 0],
        '%' => [
            0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011,
        ],
        '&' => [
            0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
        ],
        '^' => [0b00100, 0b01010, 0b10001, 0, 0, 0, 0],
        _ => [
            0b11111, 0b10001, 0b00010, 0b00100, 0b01000, 0b10001, 0b11111,
        ],
    }
}

/// Browser arrow-key direction for DevTools key dispatch.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BrowserArrowKey {
    /// ArrowUp.
    Up,
    /// ArrowDown.
    Down,
    /// ArrowLeft.
    Left,
    /// ArrowRight.
    Right,
}

impl BrowserArrowKey {
    fn key_fields(self) -> (&'static str, &'static str, u32) {
        match self {
            BrowserArrowKey::Up => ("ArrowUp", "ArrowUp", 38),
            BrowserArrowKey::Down => ("ArrowDown", "ArrowDown", 40),
            BrowserArrowKey::Left => ("ArrowLeft", "ArrowLeft", 37),
            BrowserArrowKey::Right => ("ArrowRight", "ArrowRight", 39),
        }
    }
}

/// Browser page-navigation key for DevTools key dispatch.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BrowserPageKey {
    /// PageUp.
    Up,
    /// PageDown.
    Down,
}

impl BrowserPageKey {
    fn key_fields(self) -> (&'static str, &'static str, u32) {
        match self {
            BrowserPageKey::Up => ("PageUp", "PageUp", 33),
            BrowserPageKey::Down => ("PageDown", "PageDown", 34),
        }
    }
}

/// Headless Chrome/Chromium native app driven via Chrome DevTools Protocol.
pub struct HeadlessBrowserApp {
    child: Child,
    socket: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    next_id: u64,
    title: String,
    width: u32,
    height: u32,
}

impl HeadlessBrowserApp {
    /// Launch Chrome headless and navigate to `url`.
    pub fn launch(url: &str, width: u32, height: u32) -> Result<Self> {
        let chrome = find_chrome().ok_or_else(|| anyhow!("Chrome/Chromium binary not found"))?;
        let user_data_dir = std::env::temp_dir().join(format!(
            "kittui-headless-chrome-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&user_data_dir)?;
        let mut child = Command::new(chrome)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--hide-scrollbars")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", user_data_dir.display()))
            .arg(format!("--window-size={width},{height}"))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn headless Chrome")?;
        let port = read_devtools_port(&mut child)?;
        let target = create_target(port, url)?;
        let ws_url = target
            .get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("/json/new response missing webSocketDebuggerUrl"))?;
        let (mut socket, _) = connect(ws_url).context("connect DevTools websocket")?;
        cdp_send_raw(&mut socket, 1, "Page.enable", json!({}))?;
        cdp_send_raw(
            &mut socket,
            2,
            "Emulation.setDeviceMetricsOverride",
            json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
        )?;
        Ok(Self {
            child,
            socket,
            next_id: 3,
            title: url.to_string(),
            width,
            height,
        })
    }

    /// Extract a best-effort DOM/ARIA semantic snapshot from the page.
    ///
    /// This augments, but does not replace, the screenshot path: opaque content
    /// such as canvas/video remains visible as pixels even when this extractor
    /// returns only a small or empty component tree.
    pub fn semantic_snapshot(&mut self) -> Result<SemanticSurfaceSnapshot> {
        let expression = browser_semantic_extractor_script();
        let value = self.cdp(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true, "awaitPromise": false}),
        )?;
        let extracted = value
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        browser_semantic_snapshot_from_value(
            format!("browser:{}", self.child.id()),
            self.title(),
            extracted,
        )
    }

    /// Route semantic focus to a DOM/ARIA component id from the latest browser
    /// semantic snapshot.
    pub fn semantic_focus(&mut self, component_id: &str) -> Result<()> {
        self.semantic_action(component_id, "focus", json!({}))
    }

    /// Route a semantic action to a DOM/ARIA component through DevTools.
    ///
    /// Component ids are resolved by rerunning the same DOM candidate/id logic
    /// used by [`HeadlessBrowserApp::semantic_snapshot`]. If the element is no
    /// longer present, the page reports a stale-component error so callers can
    /// refresh their snapshot.
    pub fn semantic_action(
        &mut self,
        component_id: &str,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        let expression = browser_semantic_action_script(component_id, action, payload)?;
        let value = self.cdp(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true, "awaitPromise": false}),
        )?;
        let result = value
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .unwrap_or_else(|| json!({"ok": false, "error": "missing-result"}));
        if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(())
        } else {
            let error = result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("semantic action failed");
            Err(anyhow!(
                "browser semantic action {action} on {component_id}: {error}"
            ))
        }
    }

    /// Dispatch a Backspace key press/release to the focused page element.
    pub fn send_backspace(&mut self) -> Result<()> {
        self.dispatch_browser_key("Backspace", "Backspace", 8)
    }

    /// Dispatch a Shift+Backspace key press/release to the focused page element.
    pub fn send_shift_backspace(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers(
            "Backspace",
            "Backspace",
            8,
            BROWSER_SHIFT_MODIFIER,
        )
    }

    /// Dispatch a Ctrl+Backspace key press/release to the focused page element.
    pub fn send_ctrl_backspace(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Backspace", "Backspace", 8, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch an Alt+Backspace key press/release to the focused page element.
    pub fn send_alt_backspace(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Backspace", "Backspace", 8, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch a Tab key press/release to the focused page element.
    pub fn send_tab(&mut self) -> Result<()> {
        self.dispatch_browser_key("Tab", "Tab", 9)
    }

    /// Dispatch a Shift+Tab key press/release to the focused page element.
    pub fn send_shift_tab(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Tab", "Tab", 9, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch an Enter key press/release to the focused page element.
    pub fn send_enter(&mut self) -> Result<()> {
        self.dispatch_browser_key("Enter", "Enter", 13)
    }

    /// Dispatch a Shift+Enter key press/release to the focused page element.
    pub fn send_shift_enter(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Enter", "Enter", 13, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Ctrl+Enter key press/release to the focused page element.
    pub fn send_ctrl_enter(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Enter", "Enter", 13, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch an Alt+Enter key press/release to the focused page element.
    pub fn send_alt_enter(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Enter", "Enter", 13, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch an Escape key press/release to the focused page element.
    pub fn send_escape(&mut self) -> Result<()> {
        self.dispatch_browser_key("Escape", "Escape", 27)
    }

    /// Dispatch an Insert key press/release to the focused page element.
    pub fn send_insert(&mut self) -> Result<()> {
        self.dispatch_browser_key("Insert", "Insert", 45)
    }

    /// Dispatch a Delete key press/release to the focused page element.
    pub fn send_delete(&mut self) -> Result<()> {
        self.dispatch_browser_key("Delete", "Delete", 46)
    }

    /// Dispatch a Shift+Insert key press/release to the focused page element.
    pub fn send_shift_insert(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Insert", "Insert", 45, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Shift+Delete key press/release to the focused page element.
    pub fn send_shift_delete(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Delete", "Delete", 46, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Ctrl+Insert key press/release to the focused page element.
    pub fn send_ctrl_insert(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Insert", "Insert", 45, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch a Ctrl+Delete key press/release to the focused page element.
    pub fn send_ctrl_delete(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Delete", "Delete", 46, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch an Alt+Insert key press/release to the focused page element.
    pub fn send_alt_insert(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Insert", "Insert", 45, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch an Alt+Delete key press/release to the focused page element.
    pub fn send_alt_delete(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Delete", "Delete", 46, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch a Home key press/release to the focused page element.
    pub fn send_home(&mut self) -> Result<()> {
        self.dispatch_browser_key("Home", "Home", 36)
    }

    /// Dispatch an End key press/release to the focused page element.
    pub fn send_end(&mut self) -> Result<()> {
        self.dispatch_browser_key("End", "End", 35)
    }

    /// Dispatch a Shift+Home key press/release to the focused page element.
    pub fn send_shift_home(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Home", "Home", 36, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Shift+End key press/release to the focused page element.
    pub fn send_shift_end(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("End", "End", 35, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Ctrl+Home key press/release to the focused page element.
    pub fn send_ctrl_home(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Home", "Home", 36, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch a Ctrl+End key press/release to the focused page element.
    pub fn send_ctrl_end(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("End", "End", 35, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch an Alt+Home key press/release to the focused page element.
    pub fn send_alt_home(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("Home", "Home", 36, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch an Alt+End key press/release to the focused page element.
    pub fn send_alt_end(&mut self) -> Result<()> {
        self.dispatch_browser_key_with_modifiers("End", "End", 35, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch an arrow-key press/release to the focused page element.
    pub fn send_arrow_key(&mut self, direction: BrowserArrowKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key(key, code, key_code)
    }

    /// Dispatch a Shift+Arrow key press/release to the focused page element.
    pub fn send_shift_arrow_key(&mut self, direction: BrowserArrowKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key_with_modifiers(key, code, key_code, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Ctrl+Arrow key press/release to the focused page element.
    pub fn send_ctrl_arrow_key(&mut self, direction: BrowserArrowKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key_with_modifiers(key, code, key_code, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch an Alt+Arrow key press/release to the focused page element.
    pub fn send_alt_arrow_key(&mut self, direction: BrowserArrowKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key_with_modifiers(key, code, key_code, BROWSER_ALT_MODIFIER)
    }

    /// Dispatch a PageUp/PageDown key press/release to the focused page element.
    pub fn send_page_key(&mut self, direction: BrowserPageKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key(key, code, key_code)
    }

    /// Dispatch a Shift+PageUp/PageDown key press/release to the focused page element.
    pub fn send_shift_page_key(&mut self, direction: BrowserPageKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key_with_modifiers(key, code, key_code, BROWSER_SHIFT_MODIFIER)
    }

    /// Dispatch a Ctrl+PageUp/PageDown key press/release to the focused page element.
    pub fn send_ctrl_page_key(&mut self, direction: BrowserPageKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key_with_modifiers(key, code, key_code, BROWSER_CTRL_MODIFIER)
    }

    /// Dispatch an Alt+PageUp/PageDown key press/release to the focused page element.
    pub fn send_alt_page_key(&mut self, direction: BrowserPageKey) -> Result<()> {
        let (key, code, key_code) = direction.key_fields();
        self.dispatch_browser_key_with_modifiers(key, code, key_code, BROWSER_ALT_MODIFIER)
    }

    fn dispatch_browser_key(&mut self, key: &str, code: &str, key_code: u32) -> Result<()> {
        let params = browser_key_event_params(key, code, key_code);
        self.dispatch_browser_key_params(params)
    }

    fn dispatch_browser_key_with_modifiers(
        &mut self,
        key: &str,
        code: &str,
        key_code: u32,
        modifiers: u32,
    ) -> Result<()> {
        let params = browser_key_event_params_with_modifiers(key, code, key_code, modifiers);
        self.dispatch_browser_key_params(params)
    }

    fn dispatch_browser_key_params(&mut self, params: serde_json::Value) -> Result<()> {
        let mut down = params.clone();
        down["type"] = json!("keyDown");
        self.cdp("Input.dispatchKeyEvent", down)?;
        let mut up = params;
        up["type"] = json!("keyUp");
        self.cdp("Input.dispatchKeyEvent", up)?;
        Ok(())
    }

    /// Dispatch a mouse click at CSS-pixel coordinates.
    pub fn click(&mut self, x: i32, y: i32) -> Result<()> {
        self.cdp(
            "Input.dispatchMouseEvent",
            json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1}),
        )?;
        self.cdp(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1}),
        )?;
        Ok(())
    }

    fn cdp(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        cdp_send_raw(&mut self.socket, id, method, params)
    }
}

const BROWSER_ALT_MODIFIER: u32 = 1;
const BROWSER_CTRL_MODIFIER: u32 = 2;
const BROWSER_SHIFT_MODIFIER: u32 = 8;

fn browser_key_event_params(key: &str, code: &str, key_code: u32) -> serde_json::Value {
    browser_key_event_params_with_modifiers(key, code, key_code, 0)
}

fn browser_key_event_params_with_modifiers(
    key: &str,
    code: &str,
    key_code: u32,
    modifiers: u32,
) -> serde_json::Value {
    json!({
        "key": key,
        "code": code,
        "windowsVirtualKeyCode": key_code,
        "nativeVirtualKeyCode": key_code,
        "modifiers": modifiers,
    })
}

fn browser_semantic_extractor_script() -> &'static str {
    r#"(() => {
  const nodes = [];
  const seen = new Set();
  const roleOf = (el) => {
    const explicit = (el.getAttribute('role') || '').toLowerCase();
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'button' || ['button', 'submit', 'reset'].includes(type)) return 'button';
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return 'listbox';
    if (tag === 'progress' || tag === 'meter') return 'progressbar';
    if (type === 'checkbox') return 'checkbox';
    if (type === 'radio') return 'radio';
    if (type === 'range') return 'slider';
    if (['text', 'search', 'email', 'url', 'tel', 'password', 'number'].includes(type)) return 'textbox';
    if (el.isContentEditable) return 'textbox';
    if (/^h[1-6]$/.test(tag) || tag === 'label') return 'label';
    return '';
  };
  const text = (el) => (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
  const byIdText = (id) => document.getElementById(id)?.innerText?.trim() || '';
  const nameOf = (el) => {
    const labelled = (el.getAttribute('aria-labelledby') || '').split(/\s+/).filter(Boolean).map(byIdText).filter(Boolean).join(' ');
    if (labelled) return labelled;
    const aria = el.getAttribute('aria-label');
    if (aria) return aria.trim();
    if (el.id) {
      const label = document.querySelector(`label[for="${CSS.escape(el.id)}"]`);
      if (label) return text(label);
    }
    const wrapped = el.closest('label');
    if (wrapped) return text(wrapped);
    if (el.alt) return el.alt;
    return text(el) || el.getAttribute('title') || el.getAttribute('placeholder') || '';
  };
  const idOf = (el, role, index) => el.id ? `dom:${el.id}` : `dom:${role || el.tagName.toLowerCase()}:${index}`;
  const candidates = document.querySelectorAll('button,a[href],input,textarea,select,progress,meter,label,h1,h2,h3,h4,h5,h6,[role],[contenteditable="true"],canvas,video');
  candidates.forEach((el, index) => {
    const rect = el.getBoundingClientRect();
    const visible = rect.width > 0 && rect.height > 0 && getComputedStyle(el).visibility !== 'hidden' && getComputedStyle(el).display !== 'none';
    if (!visible) return;
    const role = roleOf(el) || (['CANVAS', 'VIDEO'].includes(el.tagName) ? 'pixel_region' : '');
    if (!role) return;
    const id = idOf(el, role, index);
    if (seen.has(id)) return;
    seen.add(id);
    const type = (el.getAttribute('type') || '').toLowerCase();
    nodes.push({
      id, role, tag: el.tagName.toLowerCase(), type,
      label: nameOf(el), text: text(el),
      value: type === 'password' ? null : ('value' in el ? el.value : null),
      checked: !!el.checked, selected: !!el.selected, disabled: !!el.disabled,
      focusable: typeof el.focus === 'function' && (el.tabIndex >= 0 || ['A','BUTTON','INPUT','TEXTAREA','SELECT'].includes(el.tagName) || el.isContentEditable),
      sensitive: type === 'password', href: el.href || null,
      x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height)
    });
  });
  return { title: document.title || location.href, nodes };
})()"#
}

fn browser_semantic_action_script(
    component_id: &str,
    action: &str,
    payload: serde_json::Value,
) -> Result<String> {
    let action = match action {
        "focus" | "activate" | "toggle" | "set_value" | "insert_text" | "select" | "scroll" => {
            action
        }
        other => return Err(anyhow!("unsupported browser semantic action {other}")),
    };
    let component = serde_json::to_string(component_id)?;
    let action_json = serde_json::to_string(action)?;
    let payload_json = serde_json::to_string(&payload)?;
    Ok(format!(
        r#"(() => {{
  const targetId = {component};
  const action = {action_json};
  const payload = {payload_json};
  const roleOf = (el) => {{
    const explicit = (el.getAttribute('role') || '').toLowerCase();
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'button' || ['button', 'submit', 'reset'].includes(type)) return 'button';
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return 'listbox';
    if (tag === 'progress' || tag === 'meter') return 'progressbar';
    if (type === 'checkbox') return 'checkbox';
    if (type === 'radio') return 'radio';
    if (type === 'range') return 'slider';
    if (['text', 'search', 'email', 'url', 'tel', 'password', 'number'].includes(type)) return 'textbox';
    if (el.isContentEditable) return 'textbox';
    if (/^h[1-6]$/.test(tag) || tag === 'label') return 'label';
    return '';
  }};
  const idOf = (el, role, index) => el.id ? `dom:${{el.id}}` : `dom:${{role || el.tagName.toLowerCase()}}:${{index}}`;
  const candidates = document.querySelectorAll('button,a[href],input,textarea,select,progress,meter,label,h1,h2,h3,h4,h5,h6,[role],[contenteditable="true"],canvas,video');
  let el = null;
  candidates.forEach((candidate, index) => {{
    if (el) return;
    const role = roleOf(candidate) || (['CANVAS', 'VIDEO'].includes(candidate.tagName) ? 'pixel_region' : '');
    if (role && idOf(candidate, role, index) === targetId) el = candidate;
  }});
  if (!el) return {{ok:false, error:'stale-component'}};
  const dispatchValue = () => {{
    el.dispatchEvent(new Event('input', {{bubbles:true}}));
    el.dispatchEvent(new Event('change', {{bubbles:true}}));
  }};
  if (action === 'focus') {{ el.focus(); return {{ok:true}}; }}
  if (action === 'activate' || action === 'toggle') {{ el.focus(); el.click(); return {{ok:true}}; }}
  if (action === 'set_value') {{
    const value = payload.value ?? payload.text ?? '';
    el.focus();
    if ('value' in el) el.value = String(value);
    else if (el.isContentEditable) el.textContent = String(value);
    else return {{ok:false, error:'not-editable'}};
    dispatchValue();
    return {{ok:true}};
  }}
  if (action === 'insert_text') {{
    const text = String(payload.text ?? payload.value ?? '');
    el.focus();
    if (document.execCommand && document.execCommand('insertText', false, text)) return {{ok:true}};
    if ('value' in el) {{ el.value = (el.value || '') + text; dispatchValue(); return {{ok:true}}; }}
    if (el.isContentEditable) {{ el.textContent = (el.textContent || '') + text; dispatchValue(); return {{ok:true}}; }}
    return {{ok:false, error:'not-editable'}};
  }}
  if (action === 'select') {{
    const value = payload.value ?? payload.id ?? payload.option;
    el.focus();
    if ('value' in el && value != null) {{ el.value = String(value); dispatchValue(); return {{ok:true}}; }}
    el.click();
    return {{ok:true}};
  }}
  if (action === 'scroll') {{ el.scrollIntoView({{block:'center', inline:'center'}}); return {{ok:true}}; }}
  return {{ok:false, error:'unsupported-action'}};
}})()"#
    ))
}

fn browser_semantic_snapshot_from_value(
    surface: impl Into<String>,
    title: impl Into<String>,
    value: serde_json::Value,
) -> Result<SemanticSurfaceSnapshot> {
    let surface = surface.into();
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title.into());
    let mut children = Vec::new();
    if let Some(nodes) = value.get("nodes").and_then(|v| v.as_array()) {
        for node in nodes {
            if let Some(component) = browser_component_from_value(node) {
                children.push(component);
            }
        }
    }
    let root = ComponentNode::new(format!("{surface}.root"), ComponentRole::Group)
        .labeled(title)
        .children(children);
    Ok(SemanticSurfaceSnapshot::new(surface, 1, root))
}

fn browser_component_from_value(value: &serde_json::Value) -> Option<ComponentNode> {
    let id = value.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let role_text = value.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let role = browser_component_role(role_text, value.get("tag").and_then(|v| v.as_str()));
    let mut state = ComponentState {
        focusable: value
            .get("focusable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        disabled: value
            .get("disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        checked: value
            .get("checked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        selected: value
            .get("selected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        sensitive: value
            .get("sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        ..ComponentState::default()
    };
    state.focused = false;
    let mut component = ComponentNode::new(id, role)
        .state(state)
        .actions(browser_actions_for_role(role_text));
    if let Some(label) = value
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        component = component.labeled(label.to_string());
    }
    if let Some(description) = value
        .get("href")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        component.description = Some(description.to_string());
    }
    if let Some(component_value) = browser_component_value(value, role_text, state.sensitive) {
        component = component.valued(component_value);
    }
    Some(component)
}

fn browser_component_role(role: &str, tag: Option<&str>) -> ComponentRole {
    match role {
        "button" => ComponentRole::Button,
        "checkbox" => ComponentRole::Checkbox,
        "radio" => ComponentRole::Radio,
        "textbox" | "searchbox" => ComponentRole::TextInput,
        "listbox" | "combobox" => ComponentRole::SelectList,
        "slider" => ComponentRole::Slider,
        "progressbar" => ComponentRole::Progress,
        "menu" | "menubar" => ComponentRole::Menu,
        "table" | "grid" | "treegrid" => ComponentRole::Table,
        "label" | "heading" => ComponentRole::Label,
        "link" => ComponentRole::Link,
        "pixel_region" => ComponentRole::Canvas,
        _ if matches!(tag, Some("label")) => ComponentRole::Label,
        _ => ComponentRole::Custom(format!("browser.{role}")),
    }
}

fn browser_component_value(
    value: &serde_json::Value,
    role: &str,
    sensitive: bool,
) -> Option<ComponentValue> {
    if sensitive {
        return None;
    }
    match role {
        "checkbox" | "radio" => value
            .get("checked")
            .and_then(|v| v.as_bool())
            .map(ComponentValue::Bool),
        "slider" | "progressbar" => value
            .get("value")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok())
            .map(ComponentValue::Number),
        "textbox" | "searchbox" => value
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| ComponentValue::Text(s.to_string())),
        _ => value
            .get("text")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| ComponentValue::Text(s.to_string())),
    }
}

fn browser_actions_for_role(role: &str) -> Vec<ComponentAction> {
    match role {
        "button" | "link" => vec![
            ComponentAction::new("focus", ActionKind::Focus),
            ComponentAction::new("activate", ActionKind::Activate),
        ],
        "checkbox" => vec![
            ComponentAction::new("focus", ActionKind::Focus),
            ComponentAction::new("toggle", ActionKind::Toggle),
        ],
        "radio" | "listbox" | "combobox" => vec![
            ComponentAction::new("focus", ActionKind::Focus),
            ComponentAction::new("select", ActionKind::Select),
        ],
        "textbox" | "searchbox" => vec![
            ComponentAction::new("focus", ActionKind::Focus),
            ComponentAction::new("set_value", ActionKind::SetValue),
            ComponentAction::new("insert_text", ActionKind::InsertText),
        ],
        "slider" => vec![
            ComponentAction::new("focus", ActionKind::Focus),
            ComponentAction::new("set_value", ActionKind::SetValue),
        ],
        _ => Vec::new(),
    }
}

fn browser_surface_metadata(pid: u32, title: String, width: u32, height: u32) -> SurfaceMetadata {
    SurfaceMetadata {
        id: SurfaceId::new(format!("browser:{pid}")),
        kind: SurfaceKind::Browser,
        title,
        capabilities: SurfaceCapabilities::interactive_capture(),
        frame_size: Some((width, height)),
    }
}

impl NativeSurface for HeadlessBrowserApp {
    fn metadata(&self) -> SurfaceMetadata {
        browser_surface_metadata(self.child.id(), self.title(), self.width, self.height)
    }

    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.width = u32::from(cols) * 8;
        self.height = u32::from(rows) * 16;
        self.cdp(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": self.width, "height": self.height, "deviceScaleFactor": 1, "mobile": false}),
        )?;
        Ok(())
    }

    fn send_surface_text(&mut self, text: &str) -> Result<()> {
        for ch in text.chars() {
            self.cdp(
                "Input.dispatchKeyEvent",
                json!({"type": "char", "text": ch.to_string()}),
            )?;
        }
        Ok(())
    }

    fn capture_surface(&mut self) -> Result<SurfaceFrame> {
        let value = self.cdp(
            "Page.captureScreenshot",
            json!({"format": "png", "captureBeyondViewport": false}),
        )?;
        let b64 = value
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("captureScreenshot response missing data"))?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
        let (width, height) = png_dimensions(&bytes)?;
        let frame = NativeFrame::Png {
            width,
            height,
            bytes,
        };
        let mut metadata = self.metadata();
        metadata.frame_size = Some((width, height));
        Ok(SurfaceFrame { metadata, frame })
    }
}

impl Drop for HeadlessBrowserApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl NativeApp for HeadlessBrowserApp {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.resize_surface(cols, rows)
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_surface_text(text)
    }

    fn capture(&mut self) -> Result<NativeFrame> {
        Ok(self.capture_surface()?.frame)
    }
}

fn cdp_send_raw(
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    socket.send(Message::Text(
        json!({"id": id, "method": method, "params": params}).to_string(),
    ))?;
    loop {
        let msg = socket.read()?;
        let Message::Text(text) = msg else { continue };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(|v| v.as_u64()) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(anyhow!("CDP {method} failed: {error}"));
        }
        return Ok(value.get("result").cloned().unwrap_or_else(|| json!({})));
    }
}

fn read_devtools_port(child: &mut Child) -> Result<u16> {
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Chrome stderr unavailable"))?;
    let started = Instant::now();
    let mut buf = Vec::new();
    while started.elapsed() < Duration::from_secs(10) {
        let mut byte = [0u8; 1];
        match stderr.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(port) = parse_devtools_port(&text) {
                    return Ok(port);
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(anyhow!("Chrome did not print DevTools listening port"))
}

fn parse_devtools_port(text: &str) -> Option<u16> {
    let marker = "DevTools listening on ws://";
    let idx = text.find(marker)? + marker.len();
    let after = &text[idx..];
    let colon = after.find(':')?;
    let after_colon = &after[colon + 1..];
    let end = after_colon.find('/')?;
    after_colon[..end].parse().ok()
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn create_target(port: u16, url: &str) -> Result<serde_json::Value> {
    let endpoint = format!("http://127.0.0.1:{port}/json/new?{}", percent_encode(url));
    let text = ureq::put(&endpoint).call()?.into_string()?;
    Ok(serde_json::from_str(&text)?)
}

fn default_pty_shell() -> String {
    if let Ok(shell) = std::env::var("KITTWM_PTY_SHELL") {
        if !shell.trim().is_empty() {
            return shell;
        }
    }
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.trim().is_empty() && std::path::Path::new(&shell).exists() {
            return shell;
        }
    }
    find_on_path("sh")
        .or_else(|| find_on_path("bash"))
        .or_else(|| {
            if std::path::Path::new("/bin/sh").exists() {
                Some("/bin/sh".to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "sh".to_string())
}

fn find_chrome() -> Option<String> {
    let candidates = [
        std::env::var("KITTUI_CHROME").ok(),
        Some("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string()),
        Some("/Applications/Chromium.app/Contents/MacOS/Chromium".to_string()),
        find_on_path("google-chrome"),
        find_on_path("chromium"),
        find_on_path("chromium-browser"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| std::path::Path::new(p).exists())
}

fn find_on_path(name: &str) -> Option<String> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string())
    })
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != PNG_SIG || &bytes[12..16] != b"IHDR" {
        return Err(anyhow!("not a PNG with IHDR"));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghostty_text_refresh_invalidates_png_cache_only_after_output() {
        assert!(should_invalidate_ghostty_png_cache_after_text_refresh(true));
        assert!(!should_invalidate_ghostty_png_cache_after_text_refresh(
            false
        ));
    }

    #[test]
    fn ghostty_snapshot_text_joins_cell_rows() {
        let snapshot = GhosttyRenderSnapshot {
            cols: 2,
            rows: 2,
            cursor_x: 0,
            cursor_y: 0,
            cells: vec![
                vec![ghostty_text_cell("a"), ghostty_text_cell("b")],
                vec![ghostty_text_cell("c"), ghostty_text_cell("d")],
            ],
        };

        assert_eq!(ghostty_snapshot_text(&snapshot), "ab\ncd");
    }

    #[test]
    fn ghostty_snapshot_text_appends_variable_width_cells_directly() {
        let snapshot = GhosttyRenderSnapshot {
            cols: 3,
            rows: 2,
            cursor_x: 0,
            cursor_y: 0,
            cells: vec![
                vec![
                    ghostty_text_cell("wide"),
                    ghostty_text_cell(""),
                    ghostty_text_cell("🙂"),
                ],
                vec![
                    ghostty_text_cell("x"),
                    ghostty_text_cell("y"),
                    ghostty_text_cell("z"),
                ],
            ],
        };

        assert_eq!(ghostty_snapshot_text(&snapshot), "wide🙂\nxyz");
    }

    fn ghostty_text_cell(text: &str) -> kittui_ghostty_vt::GhosttyCellSnapshot {
        kittui_ghostty_vt::GhosttyCellSnapshot {
            text: text.to_string(),
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: 0,
        }
    }

    #[test]
    fn cached_revision_frame_requires_matching_revision() {
        let cached = Some(NativeFrame::Rgba {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        });

        assert!(cached_revision_frame(7, 7, &cached).is_some());
        assert!(cached_revision_frame(8, 7, &cached).is_none());
        assert!(cached_revision_frame(7, 7, &None).is_none());
    }

    #[test]
    fn terminal_state_resize_bumps_revision() {
        let mut state = TerminalState::new(2, 2);
        assert_eq!(state.revision, 0);
        state.bump_revision();
        assert_eq!(state.revision, 1);
        state.resize(3, 2);
        assert_eq!(state.revision, 2);
    }

    #[test]
    fn terminal_render_state_excludes_scrollback_and_pending_buffers() {
        let mut state = TerminalState::new(2, 1);
        state.cells[0].ch = 'x';
        state.push_scrollback_line("history".to_string());
        state.queue_response(b"reply");
        state.queue_host_sequence(b"host");
        let render = state.render_state();
        assert_eq!(render.cols, 2);
        assert_eq!(render.rows, 1);
        assert_eq!(render.cells[0].ch, 'x');
        assert_eq!(state.scrollback_snapshot(), "history\n");
    }

    #[test]
    fn terminal_snapshot_cache_reuses_revision_and_refreshes_on_change() {
        let mut state = TerminalState::new(2, 1);
        state.cells[0].ch = 'a';
        let cache = Mutex::new(None);
        let first = cached_terminal_snapshot(&state, &cache, TerminalState::text_snapshot);
        assert_eq!(first, "a\n");

        state.cells[0].ch = 'b';
        let stale = cached_terminal_snapshot(&state, &cache, TerminalState::text_snapshot);
        assert_eq!(stale, first, "unchanged revision should reuse cached text");

        state.bump_revision();
        let refreshed = cached_terminal_snapshot(&state, &cache, TerminalState::text_snapshot);
        assert_eq!(refreshed, "b\n");
    }

    #[test]
    fn terminal_snapshot_cache_keeps_scrollback_empty_fast_path() {
        let state = TerminalState::new(2, 1);
        let cache = Mutex::new(None);
        assert_eq!(
            cached_terminal_snapshot(&state, &cache, TerminalState::scrollback_snapshot),
            ""
        );
        assert_eq!(
            cache.lock().as_ref().map(|(revision, _)| *revision),
            Some(0)
        );
    }

    #[test]
    fn terminal_scrollback_snapshot_appends_lines_directly() {
        let mut state = TerminalState::new(2, 1);
        state.push_scrollback_line("one".to_string());
        state.push_scrollback_line("two".to_string());
        assert_eq!(state.scrollback_snapshot(), "one\ntwo\n");
    }

    #[test]
    fn terminal_text_snapshot_appends_trimmed_rows_directly() {
        let mut state = TerminalState::new(4, 2);
        state.cells[0].ch = 'a';
        state.cells[1].ch = 'b';
        state.cells[4].ch = 'c';
        assert_eq!(state.line_snapshot(0), "ab");
        assert_eq!(state.text_snapshot(), "ab\nc\n");
    }

    #[test]
    fn cached_surface_frame_reuses_payload_and_updates_metadata_size() {
        let metadata = SurfaceMetadata {
            id: SurfaceId::new("ghostty-test"),
            kind: SurfaceKind::Terminal,
            title: "ghostty-test".to_string(),
            capabilities: SurfaceCapabilities::interactive_capture(),
            frame_size: None,
        };
        let cached = Some(NativeFrame::Png {
            width: 11,
            height: 7,
            bytes: vec![1, 2, 3],
        });

        let frame = cached_surface_frame(metadata, &cached).expect("cached frame");

        assert_eq!(frame.metadata.frame_size, Some((11, 7)));
        assert_eq!(frame.frame.payload_len(), 3);
        assert!(cached_surface_frame(frame.metadata, &None).is_none());
    }

    #[test]
    fn native_frame_reports_dimensions() {
        let frame = NativeFrame::Rgba {
            width: 3,
            height: 2,
            rgba: vec![0; 24],
        };
        assert_eq!(frame.width(), 3);
        assert_eq!(frame.height(), 2);
    }

    #[test]
    fn pty_terminal_echo_round_trip_and_capture() {
        let mut term = PtyTerminalApp::spawn("cat", 40, 6).expect("spawn pty cat");
        term.send_text("hello from pty\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !term.text_snapshot().contains("hello from pty") {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = term.text_snapshot();
        assert!(text.contains("hello from pty"), "snapshot was:\n{text}");
        let frame = term.capture().unwrap();
        let NativeFrame::Rgba {
            width,
            height,
            rgba,
        } = frame
        else {
            panic!("expected RGBA")
        };
        assert_eq!((width, height), (320, 96));
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        assert!(rgba.chunks_exact(4).any(|px| px[0] == 0xd7));
    }

    #[test]
    fn terminal_state_reports_cursor_position() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(20, 4);
        parser.advance(&mut state, b"abc\nxy");
        assert_eq!((state.cursor_col, state.cursor_row), (2, 1));
    }

    #[test]
    fn terminal_state_expands_tabs_to_next_stop() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(16, 2);
        parser.advance(&mut state, b"a\tb");
        let text = state.text_snapshot();
        assert!(text.starts_with("a       b"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_honors_additional_cursor_csi_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(12, 4);
        parser.advance(&mut state, b"x\x1b[6Gy\x1b[2dz\x1b[2Ew\x1b[1Fk\x1b[2an");
        let text = state.text_snapshot();
        assert!(
            text.starts_with("x    y\n      z\nk  n\nw"),
            "snapshot was:\n{text}"
        );
    }

    #[test]
    fn terminal_state_honors_dec_autowrap_mode() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(4, 2);
        parser.advance(&mut state, b"abcdE");
        let text = state.text_snapshot();
        assert!(text.starts_with("abcd\nE"), "snapshot was:\n{text}");

        let mut state = TerminalState::new(4, 2);
        parser.advance(&mut state, b"\x1b[?7labcdE");
        let text = state.text_snapshot();
        assert!(text.starts_with("abcE\n"), "snapshot was:\n{text}");
        assert!(!state.auto_wrap);

        parser.advance(&mut state, b"\x1b[?7hFG");
        let text = state.text_snapshot();
        assert!(text.starts_with("abcF\nG"), "snapshot was:\n{text}");
        assert!(state.auto_wrap);
    }

    #[test]
    fn terminal_state_honors_scroll_region() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 5);
        parser.advance(&mut state, b"header\nbody1\nbody2\nbody3\nfooter");
        parser.advance(&mut state, b"\x1b[2;4r\x1b[4;1H\x1b[2Knew\n");
        let text = state.text_snapshot();
        assert!(
            text.starts_with("header\nbody2\nnew\n\nfooter"),
            "snapshot was:\n{text}"
        );
    }

    #[test]
    fn terminal_state_resets_scroll_region() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 4);
        parser.advance(&mut state, b"top\nmid1\nmid2\nbot");
        parser.advance(
            &mut state,
            b"\x1b[2;3r\x1b[3;1H\x1b[2KX\n\x1b[r\x1b[4;1H\x1b[2KY\n",
        );
        let text = state.text_snapshot();
        assert!(text.starts_with("X\n\nY"), "snapshot was:\n{text}");
        assert_eq!(state.scroll_top, 0);
        assert_eq!(state.scroll_bottom, 3);
    }

    #[test]
    fn terminal_state_honors_dec_special_graphics() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"\x1b(0lqqk\nxx\n\x1b(Bxq");
        let text = state.text_snapshot();
        assert!(text.starts_with("┌──┐\n││\nxq"), "snapshot was:\n{text}");
        assert!(!state.dec_special_graphics);
    }

    #[test]
    fn terminal_state_honors_insert_mode() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(5, 1);
        parser.advance(&mut state, b"abcde\x1b[1;3HX");
        assert_eq!(state.text_snapshot(), "abXde\n");
        assert!(!state.insert_mode);

        let mut state = TerminalState::new(5, 1);
        parser.advance(&mut state, b"abcde\x1b[1;3H\x1b[4hX");
        assert_eq!(state.text_snapshot(), "abXcd\n");
        assert!(state.insert_mode);
        parser.advance(&mut state, b"\x1b[4lY");
        assert_eq!(state.text_snapshot(), "abXYd\n");
        assert!(!state.insert_mode);
    }

    #[test]
    fn terminal_state_tracks_application_cursor_key_mode() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 3);
        assert!(!state.application_cursor_keys);
        parser.advance(&mut state, b"\x1b[?1h");
        assert!(state.application_cursor_keys);
        parser.advance(&mut state, b"\x1b[?1l");
        assert!(!state.application_cursor_keys);
    }

    #[test]
    fn terminal_state_queues_device_status_responses() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"\x1b[5n");
        assert_eq!(state.take_pending_responses(), b"\x1b[0n");

        parser.advance(&mut state, b"\x1b[2;4H\x1b[6n");
        assert_eq!(state.take_pending_responses(), b"\x1b[2;4R");
        assert!(state.take_pending_responses().is_empty());
    }

    #[test]
    fn terminal_state_honors_full_and_soft_reset_controls() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 3);
        parser.advance(
            &mut state,
            b"\x1b[31mabc\nscrolled\nmore\n\x1b[?7l\x1b[?25l\x1b[2;3r\x1bc",
        );
        assert_eq!(state.text_snapshot(), "\n\n\n");
        assert_eq!(state.scrollback_snapshot(), "");
        assert_eq!(state.current_style, TerminalStyle::default());
        assert!(state.auto_wrap);
        assert!(state.cursor_visible);
        assert_eq!((state.cursor_col, state.cursor_row), (0, 0));
        assert_eq!((state.scroll_top, state.scroll_bottom), (0, 2));

        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"abc\x1b[31m\x1b[?7l\x1b[?25l\x1b[2;3r\x1b[!p");
        assert!(state.text_snapshot().starts_with("abc"));
        assert_eq!(state.current_style, TerminalStyle::default());
        assert!(state.auto_wrap);
        assert!(state.cursor_visible);
        assert_eq!((state.cursor_col, state.cursor_row), (0, 0));
        assert_eq!((state.scroll_top, state.scroll_bottom), (0, 2));
    }

    #[test]
    fn terminal_state_honors_index_and_next_line_controls() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"ab\x1bEcd");
        let text = state.text_snapshot();
        assert!(text.starts_with("ab\ncd"), "snapshot was:\n{text}");

        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"ab\x1bDcd");
        let text = state.text_snapshot();
        assert!(text.starts_with("ab\n  cd"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_honors_reverse_index_in_scroll_region() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 5);
        parser.advance(&mut state, b"header\nbody1\nbody2\nbody3\nfooter");
        parser.advance(&mut state, b"\x1b[2;4r\x1b[2;1H\x1bMtop");
        let text = state.text_snapshot();
        assert!(
            text.starts_with("header\ntop\nbody1\nbody2\nfooter"),
            "snapshot was:\n{text}"
        );
    }

    #[test]
    fn terminal_state_honors_origin_mode_with_scroll_region() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 5);
        parser.advance(&mut state, b"header\nbody1\nbody2\nbody3\nfooter");
        parser.advance(&mut state, b"\x1b[2;4r\x1b[?6h\x1b[1;1H\x1b[2KORIGIN");
        let text = state.text_snapshot();
        assert!(
            text.starts_with("header\nORIGIN\nbody2\nbody3\nfooter"),
            "snapshot was:\n{text}"
        );
        assert!(state.origin_mode);
    }

    #[test]
    fn terminal_state_origin_mode_disable_restores_absolute_addressing() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 5);
        parser.advance(&mut state, b"header\nbody1\nbody2\nbody3\nfooter");
        parser.advance(&mut state, b"\x1b[2;4r\x1b[?6h\x1b[1;1HR\x1b[?6l\x1b[1;1HA");
        let text = state.text_snapshot();
        assert!(text.starts_with("Aeader\nRody1"), "snapshot was:\n{text}");
        assert!(!state.origin_mode);
    }

    #[test]
    fn terminal_state_honors_cursor_save_restore_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(12, 3);
        parser.advance(&mut state, b"ab\x1b7\x1b[3;6HXY\x1b8c");
        let text = state.text_snapshot();
        assert!(text.starts_with("abc\n\n     XY"), "snapshot was:\n{text}");

        let mut state = TerminalState::new(12, 3);
        parser.advance(&mut state, b"ab\x1b[s\x1b[3;6HXY\x1b[uC");
        let text = state.text_snapshot();
        assert!(text.starts_with("abC\n\n     XY"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_tracks_sgr_cell_colors() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"\x1b[31mR\x1b[44mB\x1b[0mD");
        assert_eq!(state.get_cell_at(0, 0).ch, 'R');
        assert_eq!(
            state.get_cell_at(0, 0).style.fg,
            Some(TerminalColor(0xe0, 0x31, 0x31))
        );
        assert_eq!(
            state.get_cell_at(1, 0).style.bg,
            Some(TerminalColor(0x19, 0x71, 0xc2))
        );
        assert_eq!(state.get_cell_at(2, 0).style, TerminalStyle::default());
        assert!(state.text_snapshot().starts_with("RBD"));
    }

    #[test]
    fn terminal_font_discovery_honors_env_and_fira_regular_names() {
        let root = std::env::temp_dir().join(format!(
            "kittui-font-test-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let nested = root.join("share/fonts/truetype");
        std::fs::create_dir_all(&nested).unwrap();
        let fira = nested.join("FiraCode-Regular.ttf");
        std::fs::write(&fira, b"not a real font").unwrap();
        assert_eq!(find_fira_code_font(&root, 4), Some(fira.clone()));

        let old = std::env::var_os("KITTUI_TERMINAL_FONT");
        std::env::set_var("KITTUI_TERMINAL_FONT", &fira);
        assert_eq!(discover_terminal_font_path(), Some(fira));
        if let Some(old) = old {
            std::env::set_var("KITTUI_TERMINAL_FONT", old);
        } else {
            std::env::remove_var("KITTUI_TERMINAL_FONT");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_font_roots_include_user_font_locations() {
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        let home = std::env::temp_dir().join(format!(
            "kittui-font-home-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let xdg = home.join("xdg-data");
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &xdg);
        let roots = terminal_font_roots();
        assert!(roots.contains(&home.join("Library/Fonts")));
        assert!(roots.contains(&home.join(".local/share/fonts")));
        assert!(roots.contains(&home.join(".fonts")));
        assert!(roots.contains(&xdg.join("fonts")));
        assert!(roots.contains(&PathBuf::from("/opt/homebrew/share/fonts")));
        if let Some(old) = old_home {
            std::env::set_var("HOME", old);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(old) = old_xdg {
            std::env::set_var("XDG_DATA_HOME", old);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
        }
    }

    #[test]
    fn terminal_font_score_matches_spaced_fira_code_nerd_names() {
        let root = std::env::temp_dir().join(format!(
            "kittui-spaced-nerd-font-test-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let spaced = root.join("Fira Code Regular Nerd Font Complete Mono.otf");
        let hyphenated = root.join("Fira-Code-Regular-Nerd-Font.ttf");
        let plain = root.join("Fira Code Regular.ttf");
        std::fs::write(&spaced, b"spaced").unwrap();
        std::fs::write(&hyphenated, b"hyphenated").unwrap();
        std::fs::write(&plain, b"plain").unwrap();
        assert_eq!(fira_code_font_score(&spaced), Some(0));
        assert_eq!(fira_code_font_score(&hyphenated), Some(1));
        assert_eq!(fira_code_font_score(&plain), Some(2));
        assert_eq!(find_fira_code_font(&root, 1), Some(spaced));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_font_discovery_prefers_fira_code_nerd_font() {
        let root = std::env::temp_dir().join(format!(
            "kittui-nerd-font-test-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let nested = root.join("share/fonts/truetype");
        std::fs::create_dir_all(&nested).unwrap();
        let regular = nested.join("FiraCode-Regular.ttf");
        let nerd = nested.join("FiraCodeNerdFont-Regular.ttf");
        let nerd_mono = nested.join("FiraCodeNerdFontMono-Regular.ttf");
        std::fs::write(&regular, b"regular").unwrap();
        std::fs::write(&nerd, b"nerd").unwrap();
        std::fs::write(&nerd_mono, b"nerd mono").unwrap();

        assert_eq!(fira_code_font_score(&regular), Some(2));
        assert_eq!(fira_code_font_score(&nerd), Some(1));
        assert_eq!(fira_code_font_score(&nerd_mono), Some(0));
        assert_eq!(find_fira_code_font(&root, 4), Some(nerd_mono));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_renderer_draws_readable_bitmap_glyphs() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(2, 1);
        parser.advance(&mut state, b"AZ");
        let rgba = render_terminal_rgba(&state, 8, 16);
        let fg = [0xd7, 0xf8, 0xff, 0xff];
        let bg = [0x08, 0x0d, 0x14, 0xff];
        let first_cell_fg = rgba
            .chunks_exact(4)
            .take(8 * 16)
            .filter(|px| **px == fg)
            .count();
        let first_cell_bg = rgba
            .chunks_exact(4)
            .take(8 * 16)
            .filter(|px| **px == bg)
            .count();
        assert!(first_cell_fg > 8, "glyph should draw foreground pixels");
        assert!(
            first_cell_bg > first_cell_fg,
            "glyph should not be a filled box"
        );
    }

    #[test]
    fn terminal_bitmap_glyph_clips_tiny_cells_before_in_bounds_writes() {
        let mut rgba = vec![0; 4];
        draw_terminal_glyph(&mut rgba, 1, 0, 0, 1, 1, 'A', TerminalColor(1, 2, 3));
        assert_eq!(rgba.len(), 4);
    }

    #[test]
    fn terminal_renderer_uses_sgr_foreground_and_background() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(2, 1);
        parser.advance(&mut state, b"\x1b[31;44mA");
        let rgba = render_terminal_rgba(&state, 8, 16);
        assert!(rgba
            .chunks_exact(4)
            .any(|px| px == [0xe0, 0x31, 0x31, 0xff]));
        assert!(rgba
            .chunks_exact(4)
            .any(|px| px == [0x19, 0x71, 0xc2, 0xff]));
    }

    #[test]
    fn terminal_renderer_skips_default_background_fills_only() {
        let default_bg = default_terminal_bg_color();
        assert!(!should_fill_terminal_cell_background(
            default_bg, default_bg
        ));
        assert!(should_fill_terminal_cell_background(
            TerminalColor(0x19, 0x71, 0xc2),
            default_bg
        ));
        assert!(should_fill_terminal_cell_background(
            TerminalColor(0xd7, 0xf8, 0xff),
            default_bg
        ));
    }

    #[test]
    fn terminal_renderer_handles_zero_columns_without_chunk_panic() {
        let state = TerminalState::new(0, 1);
        assert!(render_terminal_rgba(&state, 8, 16).is_empty());
    }

    #[test]
    fn terminal_renderer_fast_paths_only_blank_default_cells() {
        let default_cell = TerminalCell::blank(TerminalStyle::default());
        assert!(is_blank_default_terminal_cell(&default_cell));
        let mut styled_blank = default_cell;
        styled_blank.style.bg = Some(TerminalColor(0x19, 0x71, 0xc2));
        assert!(!is_blank_default_terminal_cell(&styled_blank));
        let mut glyph = default_cell;
        glyph.ch = 'x';
        assert!(!is_blank_default_terminal_cell(&glyph));
    }

    #[test]
    fn rgba_pixel_index_matches_row_major_layout() {
        assert_eq!(rgba_pixel_index(10, 0, 0), 0);
        assert_eq!(rgba_pixel_index(10, 3, 0), 12);
        assert_eq!(rgba_pixel_index(10, 0, 2), 80);
        assert_eq!(rgba_pixel_index(10, 3, 2), 92);
    }

    #[test]
    fn checked_and_in_bounds_blend_match_for_valid_pixel() {
        let color = TerminalColor(200, 100, 50);
        let mut checked = vec![10, 20, 30, 255];
        let mut in_bounds = checked.clone();
        blend_rgba_pixel(&mut checked, 1, 0, 0, color, 128);
        blend_rgba_pixel_in_bounds(&mut in_bounds, 1, 0, 0, color, 128);
        assert_eq!(checked, in_bounds);
    }

    #[test]
    fn checked_and_in_bounds_set_pixel_match_for_valid_pixel() {
        let color = TerminalColor(3, 4, 5);
        let mut checked = vec![0; 16];
        let mut in_bounds = checked.clone();
        set_rgba_pixel(&mut checked, 2, 1, 1, color);
        set_rgba_pixel_in_bounds(&mut in_bounds, 2, 1, 1, color);
        assert_eq!(checked, in_bounds);
        assert_eq!(&checked[12..16], &[3, 4, 5, 255]);
    }

    #[test]
    fn terminal_font_glyph_cache_reuses_character_size_entries() {
        clear_terminal_font_glyph_cache_for_tests();
        let Some(font) = terminal_font() else {
            return;
        };
        let Some(first) = cached_terminal_font_glyph(font, 'A', 13.0) else {
            return;
        };
        let second = cached_terminal_font_glyph(font, 'A', 13.0).expect("cached glyph");
        assert_eq!(first.metrics.width, second.metrics.width);
        assert_eq!(first.bitmap, second.bitmap);
        assert_eq!(terminal_font_glyph_cache_len_for_tests(), 1);
        let _ = cached_terminal_font_glyph(font, 'B', 13.0);
        assert!(terminal_font_glyph_cache_len_for_tests() >= 1);
    }

    #[test]
    fn terminal_font_glyph_cache_prunes_at_cap() {
        assert!(!terminal_font_glyph_cache_should_prune(
            TERMINAL_FONT_GLYPH_CACHE_MAX - 1
        ));
        assert!(terminal_font_glyph_cache_should_prune(
            TERMINAL_FONT_GLYPH_CACHE_MAX
        ));
        assert!(terminal_font_glyph_cache_should_prune(
            TERMINAL_FONT_GLYPH_CACHE_MAX + 1
        ));
    }

    #[test]
    fn terminal_renderer_direct_cell_iteration_preserves_styled_backgrounds() {
        let mut state = TerminalState::new(2, 1);
        state.cursor_visible = false;
        state.cells[1].style.bg = Some(TerminalColor(0x19, 0x71, 0xc2));
        let rgba = render_terminal_rgba(&state, 2, 1);
        assert_eq!(&rgba[0..4], &[0x08, 0x0d, 0x14, 0xff]);
        assert_eq!(&rgba[8..12], &[0x19, 0x71, 0xc2, 0xff]);
    }

    #[test]
    fn fill_cell_background_writes_contiguous_cell_rows() {
        let mut rgba = vec![0; 4 * 4 * 2];
        fill_cell_background(&mut rgba, 4, 1, 0, 2, 2, TerminalColor(7, 8, 9));
        assert_eq!(&rgba[8..12], &[7, 8, 9, 255]);
        assert_eq!(&rgba[12..16], &[7, 8, 9, 255]);
        assert_eq!(&rgba[24..28], &[7, 8, 9, 255]);
        assert_eq!(&rgba[28..32], &[7, 8, 9, 255]);
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(&rgba[20..24], &[0, 0, 0, 0]);
    }

    #[test]
    fn terminal_cursor_draws_contiguous_bottom_rows() {
        let mut state = TerminalState::new(1, 1);
        state.cursor_visible = true;
        let rgba = render_terminal_rgba(&state, 4, 4);
        let bg = [0x08, 0x0d, 0x14, 0xff];
        let fg = [0xd7, 0xf8, 0xff, 0xff];
        assert_eq!(&rgba[0..4], &bg);
        for px in rgba[16..].chunks_exact(4) {
            assert_eq!(px, fg);
        }
    }

    #[test]
    fn terminal_state_tracks_extended_sgr_colors() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        parser.advance(
            &mut state,
            b"\x1b[38;5;196mX\x1b[48:2:1:2:3mY\x1b[38:2:4:5:6mZ",
        );
        assert_eq!(
            state.get_cell_at(0, 0).style.fg,
            Some(TerminalColor(255, 0, 0))
        );
        assert_eq!(
            state.get_cell_at(1, 0).style.bg,
            Some(TerminalColor(1, 2, 3))
        );
        assert_eq!(
            state.get_cell_at(2, 0).style.fg,
            Some(TerminalColor(4, 5, 6))
        );
        assert!(state.text_snapshot().starts_with("XYZ"));
    }

    #[test]
    fn terminal_renderer_uses_extended_sgr_colors() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(2, 1);
        parser.advance(&mut state, b"\x1b[38;2;9;8;7;48;5;21mT");
        let rgba = render_terminal_rgba(&state, 8, 16);
        assert!(rgba.chunks_exact(4).any(|px| px == [9, 8, 7, 0xff]));
        assert!(rgba.chunks_exact(4).any(|px| px == [0, 0, 255, 0xff]));
    }

    #[test]
    fn terminal_state_tracks_bracketed_paste_mode() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        assert!(!state.bracketed_paste);
        parser.advance(&mut state, b"\x1b[?2004h");
        assert!(state.bracketed_paste);
        parser.advance(&mut state, b"\x1b[?2004l");
        assert!(!state.bracketed_paste);
    }

    #[test]
    fn terminal_state_tracks_focus_reporting_mode() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        assert!(!state.focus_reporting);
        parser.advance(&mut state, b"\x1b[?1004h");
        assert!(state.focus_reporting);
        parser.advance(&mut state, b"\x1b[?1004l");
        assert!(!state.focus_reporting);
    }

    #[test]
    fn terminal_state_tracks_mouse_reporting_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        assert_eq!(state.mouse_modes, MouseReportingModes::default());
        parser.advance(&mut state, b"\x1b[?1000;1002;1003;1006h");
        assert_eq!(
            state.mouse_modes,
            MouseReportingModes {
                basic: true,
                button_motion: true,
                all_motion: true,
                sgr: true,
            }
        );
        parser.advance(&mut state, b"\x1b[?1002;1006l");
        assert!(state.mouse_modes.basic);
        assert!(!state.mouse_modes.button_motion);
        assert!(state.mouse_modes.all_motion);
        assert!(!state.mouse_modes.sgr);
    }

    #[test]
    fn terminal_state_tracks_cursor_visibility_mode() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        assert!(state.cursor_visible);
        parser.advance(&mut state, b"\x1b[?25l");
        assert!(!state.cursor_visible);
        parser.advance(&mut state, b"\x1b[?25h");
        assert!(state.cursor_visible);
    }

    #[test]
    fn terminal_renderer_draws_visible_cursor() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(2, 1);
        parser.advance(&mut state, b"A");
        let visible = render_terminal_rgba(&state, 8, 16);
        state.cursor_visible = false;
        let hidden = render_terminal_rgba(&state, 8, 16);
        assert_ne!(visible, hidden);
        assert!(visible
            .chunks_exact(4)
            .any(|px| px == [0xd7, 0xf8, 0xff, 0xff]));
    }

    #[test]
    fn terminal_state_captures_scrollback_on_scroll() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"one\ntwo\nthree");
        assert_eq!(state.scrollback_snapshot(), "one\n");
        let text = state.text_snapshot();
        assert!(text.starts_with("two\nthree"), "snapshot was:\n{text}");
    }

    #[test]
    fn join_osc_utf8_params_appends_without_intermediate_vec() {
        assert_eq!(join_osc_utf8_params(&[b"hello", b"world"]), "hello;world");
        assert_eq!(
            join_osc_utf8_params(&[b"hello", &[0xff], b"world"]),
            "hello;world"
        );
        assert_eq!(join_osc_utf8_params(&[]), "");
    }

    #[test]
    fn terminal_state_batches_scrollback_pruning_after_overflow() {
        let mut state = TerminalState::new(8, 2);
        for idx in 0..=SCROLLBACK_MAX_LINES {
            state.push_scrollback_line(format!("line-{idx}"));
        }
        assert!(state.scrollback.len() <= SCROLLBACK_MAX_LINES);
        assert_eq!(
            state.scrollback.len(),
            SCROLLBACK_MAX_LINES + 1 - SCROLLBACK_PRUNE_BATCH
        );
        assert_eq!(
            state.scrollback.first().map(String::as_str),
            Some("line-1024")
        );
        assert_eq!(
            state.scrollback.last().map(String::as_str),
            Some("line-10000")
        );
    }

    #[test]
    fn terminal_state_does_not_capture_alt_screen_scrollback() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"normal\x1b[?1049hone\ntwo\nthree");
        assert_eq!(state.scrollback_snapshot(), "");
        parser.advance(&mut state, b"\x1b[?1049l");
        assert_eq!(state.scrollback_snapshot(), "");
    }

    #[test]
    fn terminal_state_honors_alternate_screen_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(12, 3);
        parser.advance(&mut state, b"shell$ \x1b[?1049htui\x1b[2;1Hview");
        let text = state.text_snapshot();
        assert!(text.starts_with("tui\nview"), "snapshot was:\n{text}");
        assert!(!text.contains("shell$"), "snapshot was:\n{text}");

        parser.advance(&mut state, b"\x1b[?1049l!");
        let text = state.text_snapshot();
        assert!(text.starts_with("shell$ !"), "snapshot was:\n{text}");
        assert!(!text.contains("tui"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_resizes_saved_alternate_screen_buffer() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"normal\x1b[?1049halt");
        state.resize(12, 3);
        parser.advance(&mut state, b"\x1b[?1049l");
        let text = state.text_snapshot();
        assert!(text.starts_with("normal"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_honors_edit_character_csi_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(10, 2);
        parser.advance(&mut state, b"abcdef\r\x1b[3C\x1b[2@XY");
        let text = state.text_snapshot();
        assert!(text.starts_with("abcXYdef"), "snapshot was:\n{text}");

        let mut state = TerminalState::new(10, 2);
        parser.advance(&mut state, b"abcdef\r\x1b[2C\x1b[2P");
        let text = state.text_snapshot();
        assert!(text.starts_with("abef"), "snapshot was:\n{text}");

        let mut state = TerminalState::new(10, 2);
        parser.advance(&mut state, b"abcdef\r\x1b[2C\x1b[3X");
        let text = state.text_snapshot();
        assert!(text.starts_with("ab   f"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_honors_edit_line_csi_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 4);
        parser.advance(&mut state, b"one\ntwo\nthree\x1b[2;1H\x1b[L");
        let text = state.text_snapshot();
        assert!(
            text.starts_with("one\n\ntwo\nthree"),
            "snapshot was:\n{text}"
        );

        let mut state = TerminalState::new(8, 4);
        parser.advance(&mut state, b"one\ntwo\nthree\x1b[2;1H\x1b[M");
        let text = state.text_snapshot();
        assert!(text.starts_with("one\nthree"), "snapshot was:\n{text}");
    }

    #[test]
    fn terminal_state_honors_erase_line_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"abcdef\r\x1b[3C\x1b[1K");
        assert!(state.text_snapshot().starts_with("    ef"));

        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"abcdef\r\x1b[2C\x1b[0K");
        assert!(state.text_snapshot().starts_with("ab"));

        let mut state = TerminalState::new(8, 2);
        parser.advance(&mut state, b"abcdef\r\x1b[2K");
        assert_eq!(state.text_snapshot().lines().next().unwrap_or(""), "");
    }

    #[test]
    fn terminal_state_honors_erase_display_modes() {
        let mut parser = Parser::new();
        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"11111111\n22222222\n33333333\x1b[2;4H\x1b[0J");
        let text = state.text_snapshot();
        assert!(text.starts_with("11111111\n222"), "snapshot was:\n{text}");
        assert!(text.contains("\n\n"), "snapshot was:\n{text}");

        let mut state = TerminalState::new(8, 3);
        parser.advance(&mut state, b"11111111\n22222222\n33333333\x1b[2;4H\x1b[1J");
        let text = state.text_snapshot();
        assert!(
            text.starts_with("\n    2222\n33333333"),
            "snapshot was:\n{text}"
        );
    }

    #[test]
    fn pty_terminal_captures_osc_window_title() {
        let term = PtyTerminalApp::spawn("printf '\\033]2;editor pane title\\007'", 40, 4)
            .expect("spawn pty title probe");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && term.title() != "editor pane title" {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(term.title(), "editor pane title");
    }

    #[test]
    fn terminal_state_preserves_osc_title_across_resize() {
        let mut state = TerminalState::new(10, 2);
        state.osc_dispatch(&[b"0", b"build", b"pane"], true);
        assert_eq!(state.title.as_deref(), Some("build;pane"));
        state.resize(20, 4);
        assert_eq!(state.title.as_deref(), Some("build;pane"));
    }

    #[test]
    fn terminal_state_forwards_osc52_clipboard_writes() {
        let mut state = TerminalState::new(10, 2);
        state.osc_dispatch(&[b"52", b"c", b"aGVsbG8="], true);
        assert_eq!(
            state.take_pending_surface_events(),
            vec![SurfaceEvent::ClipboardSet {
                selection: "c".to_string(),
                payload_base64: "aGVsbG8=".to_string(),
            }]
        );
        assert_eq!(
            state.take_pending_host_sequences(),
            b"\x1b]52;c;aGVsbG8=\x07"
        );
        assert!(state.take_pending_host_sequences().is_empty());
    }

    #[test]
    fn terminal_state_reports_bell_title_and_notification_events() {
        let mut state = TerminalState::new(10, 2);
        state.execute(0x07);
        state.osc_dispatch(&[b"2", b"editor"], true);
        state.osc_dispatch(&[b"9", b"build finished"], true);
        state.osc_dispatch(&[b"777", b"notify", b"cargo", b"tests passed"], true);
        assert_eq!(
            state.take_pending_surface_events(),
            vec![
                SurfaceEvent::Bell {
                    visual: true,
                    audible: true,
                },
                SurfaceEvent::TitleChanged("editor".to_string()),
                SurfaceEvent::Notification {
                    title: "editor".to_string(),
                    body: "build finished".to_string(),
                },
                SurfaceEvent::Notification {
                    title: "cargo".to_string(),
                    body: "tests passed".to_string(),
                },
            ]
        );
    }

    #[test]
    fn terminal_state_ignores_osc52_queries_and_invalid_payloads() {
        let mut state = TerminalState::new(10, 2);
        state.osc_dispatch(&[b"52", b"c", b"?"], true);
        state.osc_dispatch(&[b"52", b"c", b"not base64!!!"], true);
        state.osc_dispatch(&[b"52", b"bad;selector", b"aGVsbG8="], true);
        assert!(state.take_pending_host_sequences().is_empty());
    }

    #[test]
    fn native_frame_and_surface_frame_helpers_report_non_payload_metadata() {
        let rgba = NativeFrame::Rgba {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert_eq!(rgba.size(), (2, 1));
        assert_eq!(rgba.format(), "rgba");
        assert_eq!(rgba.payload_len(), 8);
        assert!(rgba.is_rgba());
        assert!(!rgba.is_png());

        let png = NativeFrame::Png {
            width: 3,
            height: 4,
            bytes: b"png".to_vec(),
        };
        assert_eq!(png.size(), (3, 4));
        assert_eq!(png.format(), "png");
        assert_eq!(png.payload_len(), 3);
        assert!(png.is_png());
        assert!(!png.is_rgba());

        let frame = SurfaceFrame {
            metadata: SurfaceMetadata {
                id: SurfaceId::new("frame:1"),
                kind: SurfaceKind::Composite,
                title: "frame".to_string(),
                capabilities: SurfaceCapabilities::capture_only(),
                frame_size: Some((3, 4)),
            },
            frame: png,
        };
        assert_eq!(frame.frame_size(), (3, 4));
        assert_eq!(frame.format(), "png");
        assert_eq!(frame.payload_len(), 3);
    }

    #[test]
    fn kittui_scene_surface_adapts_scene_capture_metadata() {
        let footprint = kittui::CellRect::new(0, 0, 4, 3);
        let cell_size = kittui::CellSize::new(8, 10);
        let scene = kittui::scene::scene(
            footprint,
            cell_size,
            vec![kittui::scene::background_solid(
                footprint,
                cell_size,
                kittui::Rgba::rgb(0x12, 0x34, 0x56),
            )],
        );
        let mut surface = KittuiSceneSurface::new("scene:settings", "settings", scene);

        let metadata = NativeSurface::metadata(&surface);
        assert_eq!(metadata.id.as_str(), "scene:settings");
        assert_eq!(metadata.kind, SurfaceKind::KittuiScene);
        assert_eq!(metadata.title, "settings");
        assert!(metadata.capabilities.capture);
        assert!(!metadata.capabilities.input);
        assert!(!metadata.capabilities.exact_byte_input);
        assert!(!metadata.capabilities.focus_events);
        assert!(!metadata.capabilities.surface_events);
        assert!(!metadata.capabilities.resize);
        assert_eq!(metadata.frame_size, Some((32, 30)));
        assert!(NativeSurface::resize_surface(&mut surface, 8, 6).is_err());
        assert!(NativeSurface::send_surface_text(&mut surface, "ignored").is_err());

        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        assert_eq!(frame.metadata.kind, SurfaceKind::KittuiScene);
        assert_eq!(frame.metadata.frame_size, Some((32, 30)));
        match frame.frame {
            NativeFrame::Png {
                width,
                height,
                bytes,
            } => {
                assert_eq!((width, height), (32, 30));
                assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            }
            other => panic!("expected PNG frame, got {other:?}"),
        }
    }

    #[test]
    fn rgba_frame_surface_validates_updates_and_captures() {
        let mut surface = RgbaFrameSurface::new(
            "rgba:1",
            "composited frame",
            2,
            1,
            vec![0x11, 0x22, 0x33, 0xff, 0x44, 0x55, 0x66, 0xff],
        )
        .unwrap();
        let metadata = NativeSurface::metadata(&surface);
        assert_eq!(metadata.id.as_str(), "rgba:1");
        assert_eq!(metadata.kind, SurfaceKind::Composite);
        assert_eq!(metadata.title, "composited frame");
        assert!(metadata.capabilities.capture);
        assert!(!metadata.capabilities.input);
        assert!(!metadata.capabilities.exact_byte_input);
        assert!(!metadata.capabilities.focus_events);
        assert!(!metadata.capabilities.surface_events);
        assert!(!metadata.capabilities.resize);
        assert_eq!(metadata.frame_size, Some((2, 1)));
        assert!(NativeSurface::resize_surface(&mut surface, 4, 4).is_err());
        assert!(NativeSurface::send_surface_text(&mut surface, "ignored").is_err());

        surface
            .update_frame(1, 2, vec![0xaa, 0xbb, 0xcc, 0xff, 0x01, 0x02, 0x03, 0xff])
            .unwrap();
        assert_eq!(surface.frame_size(), (1, 2));
        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        assert_eq!(frame.metadata.frame_size, Some((1, 2)));
        assert!(matches!(
            frame.frame,
            NativeFrame::Rgba {
                width: 1,
                height: 2,
                ..
            }
        ));
    }

    #[test]
    fn rgba_frame_surface_rejects_invalid_payloads() {
        assert!(RgbaFrameSurface::new("bad", "bad", 0, 1, vec![]).is_err());
        assert!(RgbaFrameSurface::new("bad", "bad", 2, 2, vec![0; 15]).is_err());
        let mut surface = RgbaFrameSurface::new("ok", "ok", 1, 1, vec![0; 4]).unwrap();
        assert!(surface.update_frame(1, 2, vec![0; 4]).is_err());
    }

    #[test]
    fn composite_frame_surface_composes_rgba_children() {
        let mut surface = CompositeFrameSurface::new("composite:1", "preview", 3, 2).unwrap();
        surface
            .push_rgba_child(
                0,
                0,
                2,
                1,
                vec![0xff, 0x00, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff],
            )
            .unwrap();
        surface
            .push_rgba_child(
                1,
                1,
                2,
                1,
                vec![0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0xff],
            )
            .unwrap();

        let metadata = NativeSurface::metadata(&surface);
        assert_eq!(metadata.id.as_str(), "composite:1");
        assert_eq!(metadata.kind, SurfaceKind::Composite);
        assert_eq!(metadata.title, "preview");
        assert!(metadata.capabilities.capture);
        assert!(!metadata.capabilities.input);
        assert!(!metadata.capabilities.exact_byte_input);
        assert!(!metadata.capabilities.focus_events);
        assert!(!metadata.capabilities.surface_events);
        assert!(!metadata.capabilities.resize);
        assert_eq!(metadata.frame_size, Some((3, 2)));
        assert_eq!(surface.children().len(), 2);
        assert!(NativeSurface::resize_surface(&mut surface, 4, 4).is_err());
        assert!(NativeSurface::send_surface_text(&mut surface, "ignored").is_err());

        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        match frame.frame {
            NativeFrame::Rgba {
                width,
                height,
                rgba,
            } => {
                assert_eq!((width, height), (3, 2));
                assert_eq!(&rgba[0..4], &[0xff, 0x00, 0x00, 0xff]);
                assert_eq!(&rgba[4..8], &[0x00, 0xff, 0x00, 0xff]);
                assert_eq!(&rgba[16..20], &[0x00, 0x00, 0xff, 0xff]);
                assert_eq!(&rgba[20..24], &[0xff, 0xff, 0x00, 0xff]);
            }
            other => panic!("expected RGBA frame, got {other:?}"),
        }
    }

    #[test]
    fn composite_frame_surface_blends_and_clips_children() {
        let mut surface = CompositeFrameSurface::new("composite:blend", "blend", 1, 1).unwrap();
        surface
            .push_rgba_child(0, 0, 1, 1, vec![0x00, 0x00, 0xff, 0xff])
            .unwrap();
        surface
            .push_rgba_child(
                0,
                0,
                2,
                1,
                vec![0xff, 0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0xff],
            )
            .unwrap();
        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        match frame.frame {
            NativeFrame::Rgba { rgba, .. } => {
                assert_eq!(rgba[3], 0xff);
                assert!(rgba[0] >= 127 && rgba[0] <= 129, "{rgba:?}");
                assert!(rgba[2] >= 126 && rgba[2] <= 128, "{rgba:?}");
            }
            other => panic!("expected RGBA frame, got {other:?}"),
        }
        surface.clear_children();
        assert!(surface.children().is_empty());
    }

    #[test]
    fn composite_frame_surface_ingests_rgba_surface_frames() {
        let child = SurfaceFrame {
            metadata: SurfaceMetadata {
                id: SurfaceId::new("rgba:child"),
                kind: SurfaceKind::Composite,
                title: "child".to_string(),
                capabilities: SurfaceCapabilities::capture_only(),
                frame_size: Some((1, 1)),
            },
            frame: NativeFrame::Rgba {
                width: 1,
                height: 1,
                rgba: vec![0x22, 0x33, 0x44, 0xff],
            },
        };
        let mut surface = CompositeFrameSurface::new("composite:frame", "frame", 2, 1).unwrap();
        surface.push_surface_frame(1, 0, &child).unwrap();
        assert_eq!(surface.children().len(), 1);
        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        match frame.frame {
            NativeFrame::Rgba { rgba, .. } => {
                assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
                assert_eq!(&rgba[4..8], &[0x22, 0x33, 0x44, 0xff]);
            }
            other => panic!("expected RGBA frame, got {other:?}"),
        }
    }

    #[test]
    fn composite_frame_surface_captures_rgba_child_surfaces() {
        let mut child =
            RgbaFrameSurface::new("rgba:child", "child", 1, 1, vec![0x90, 0x80, 0x70, 0xff])
                .unwrap();
        let mut surface = CompositeFrameSurface::new("composite:capture", "capture", 2, 1).unwrap();
        let captured = surface.push_surface_capture(1, 0, &mut child).unwrap();
        assert_eq!(captured.metadata.id.as_str(), "rgba:child");
        assert_eq!(surface.children().len(), 1);
        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        match frame.frame {
            NativeFrame::Rgba { rgba, .. } => {
                assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
                assert_eq!(&rgba[4..8], &[0x90, 0x80, 0x70, 0xff]);
            }
            other => panic!("expected RGBA frame, got {other:?}"),
        }
    }

    #[test]
    fn composite_frame_surface_rejects_png_child_surface_captures() {
        let footprint = kittui::CellRect::new(0, 0, 1, 1);
        let cell_size = kittui::CellSize::new(1, 1);
        let scene = kittui::scene::scene(
            footprint,
            cell_size,
            vec![kittui::scene::background_solid(
                footprint,
                cell_size,
                kittui::Rgba::rgb(0, 0, 0),
            )],
        );
        let mut child = KittuiSceneSurface::new("scene:child", "scene", scene);
        let mut surface = CompositeFrameSurface::new("composite:capture", "capture", 2, 1).unwrap();
        let err = surface.push_surface_capture(0, 0, &mut child).unwrap_err();
        assert!(
            err.to_string().contains("PNG input must be decoded"),
            "{err}"
        );
        assert!(surface.children().is_empty());
    }

    #[test]
    fn composite_frame_surface_rejects_png_surface_frames() {
        let child = SurfaceFrame {
            metadata: SurfaceMetadata {
                id: SurfaceId::new("scene:child"),
                kind: SurfaceKind::KittuiScene,
                title: "scene".to_string(),
                capabilities: SurfaceCapabilities::capture_only(),
                frame_size: Some((1, 1)),
            },
            frame: NativeFrame::Png {
                width: 1,
                height: 1,
                bytes: b"not actually png".to_vec(),
            },
        };
        let mut surface = CompositeFrameSurface::new("composite:frame", "frame", 2, 1).unwrap();
        let err = surface.push_surface_frame(0, 0, &child).unwrap_err();
        assert!(
            err.to_string().contains("PNG input must be decoded"),
            "{err}"
        );
    }

    #[test]
    fn composite_frame_surface_rejects_invalid_inputs() {
        assert!(CompositeFrameSurface::new("bad", "bad", 0, 1).is_err());
        let mut surface = CompositeFrameSurface::new("ok", "ok", 2, 2).unwrap();
        assert!(surface.push_rgba_child(0, 0, 1, 1, vec![0; 3]).is_err());
    }

    #[test]
    fn xwindow_surface_routes_surface_pointer_events() {
        struct RecordingServer {
            window: XWindow,
            events: Arc<parking_lot::Mutex<Vec<XPointerEvent>>>,
        }

        impl XServer for RecordingServer {
            fn windows(&self) -> std::result::Result<Vec<XWindow>, kittui_xvfb::XError> {
                Ok(vec![self.window.clone()])
            }

            fn capture(&self, id: XWindowId) -> std::result::Result<XCapture, kittui_xvfb::XError> {
                Ok(XCapture {
                    id,
                    width: 1,
                    height: 1,
                    rgba: vec![0, 0, 0, 0xff],
                })
            }

            fn inject_pointer(
                &self,
                event: XPointerEvent,
            ) -> std::result::Result<(), kittui_xvfb::XError> {
                self.events.lock().push(event);
                Ok(())
            }

            fn inject_key(
                &self,
                _sym: u32,
                _pressed: bool,
            ) -> std::result::Result<(), kittui_xvfb::XError> {
                Ok(())
            }
        }

        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let window = XWindow {
            id: XWindowId(11),
            rect: kittui_core::geom::PxRect::new(0.0, 0.0, 10.0, 10.0),
            title: "xterm".to_string(),
        };
        let server = RecordingServer {
            window: window.clone(),
            events: events.clone(),
        };
        let mut surface = XWindowSurface::x11(Box::new(server), window, 8, 16);

        NativeSurface::send_surface_pointer(
            &mut surface,
            SurfacePointerEvent::Move { x_px: 3, y_px: 4 },
        )
        .unwrap();
        NativeSurface::send_surface_pointer(
            &mut surface,
            SurfacePointerEvent::Press {
                button: SurfacePointerButton::Left,
            },
        )
        .unwrap();
        NativeSurface::send_surface_pointer(
            &mut surface,
            SurfacePointerEvent::Release {
                button: SurfacePointerButton::ScrollDown,
            },
        )
        .unwrap();

        assert_eq!(
            &*events.lock(),
            &[
                XPointerEvent::Move {
                    window: XWindowId(11),
                    x_px: 3,
                    y_px: 4,
                },
                XPointerEvent::Press {
                    window: XWindowId(11),
                    button: XButton::Left,
                },
                XPointerEvent::Release {
                    window: XWindowId(11),
                    button: XButton::ScrollDown,
                },
            ]
        );
    }

    #[test]
    fn capture_only_surface_pointer_hook_reports_unsupported() {
        let mut surface =
            RgbaFrameSurface::new("rgba:pointer", "pointer", 1, 1, vec![0; 4]).unwrap();
        let err = NativeSurface::send_surface_pointer(
            &mut surface,
            SurfacePointerEvent::Move { x_px: 0, y_px: 0 },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("does not support pointer input"),
            "{err}"
        );
    }

    #[test]
    fn xwindow_surface_adapts_xserver_capture_input_and_resize() {
        let window = (
            XWindowId(7),
            kittui_core::geom::PxRect::new(0.0, 0.0, 16.0, 8.0),
            "xterm",
            [0x11, 0x22, 0x33, 0xff],
        );
        let server = kittui_xvfb::FakeServer::with_windows(vec![window]);
        let x_window = server.windows().unwrap().pop().unwrap();
        let mut surface = XWindowSurface::x11(Box::new(server), x_window, 8, 16);

        let metadata = NativeSurface::metadata(&surface);
        assert_eq!(metadata.id.as_str(), "xwindow:7");
        assert_eq!(metadata.kind, SurfaceKind::X11);
        assert!(metadata.capabilities.capture);
        assert!(metadata.capabilities.input);
        assert!(metadata.capabilities.resize);
        assert_eq!(metadata.title, "xterm");

        NativeSurface::send_surface_text(&mut surface, "a").unwrap();
        NativeSurface::resize_surface(&mut surface, 4, 3).unwrap();
        let frame = NativeSurface::capture_surface(&mut surface).unwrap();
        assert_eq!(frame.metadata.frame_size, Some((32, 48)));
        assert!(matches!(
            frame.frame,
            NativeFrame::Rgba {
                width: 32,
                height: 48,
                ..
            }
        ));
    }

    #[test]
    fn native_surface_focus_hook_sends_pty_focus_reports_when_enabled() {
        let mut term = PtyTerminalApp::spawn("printf '\\033[?1004h'; cat -v", 40, 6)
            .expect("spawn pty focus probe");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !term.focus_reporting_enabled() {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(term.focus_reporting_enabled());
        NativeSurface::send_surface_focus(&mut term, true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !term.text_snapshot().contains("^[[I") {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = term.text_snapshot();
        assert!(text.contains("^[[I"), "snapshot was:\n{text}");
    }

    #[test]
    fn native_surface_focus_hook_ignores_pty_focus_reports_when_disabled() {
        let mut term = PtyTerminalApp::spawn("cat -v", 40, 6).expect("spawn pty focus probe");
        assert!(!term.focus_reporting_enabled());
        NativeSurface::send_surface_focus(&mut term, true).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let text = term.text_snapshot();
        assert!(!text.contains("^[[I"), "snapshot was:\n{text}");
    }

    #[test]
    fn capture_only_surface_focus_hook_defaults_to_noop() {
        let mut surface = RgbaFrameSurface::new("rgba:empty", "empty", 1, 1, vec![0; 4]).unwrap();
        NativeSurface::send_surface_focus(&mut surface, true).unwrap();
        assert!(NativeSurface::take_surface_events(&mut surface).is_empty());
    }

    #[test]
    fn native_surface_exact_byte_hook_preserves_pty_bytes() {
        let mut term = PtyTerminalApp::spawn("cat", 40, 6).expect("spawn pty cat");
        NativeSurface::send_surface_bytes(&mut term, b"raw\x1b[7m-bytes\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !term.text_snapshot().contains("raw-bytes") {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = term.text_snapshot();
        assert!(text.contains("raw-bytes"), "snapshot was:\n{text}");
    }

    #[test]
    fn native_surface_default_byte_hook_rejects_non_utf8() {
        let mut window = XWindowSurface::x11(
            Box::new(kittui_xvfb::FakeServer::with_windows(vec![(
                XWindowId(9),
                kittui_core::geom::PxRect::new(0.0, 0.0, 8.0, 8.0),
                "xterm",
                [0, 0, 0, 0xff],
            )])),
            kittui_xvfb::XWindow {
                id: XWindowId(9),
                rect: kittui_core::geom::PxRect::new(0.0, 0.0, 8.0, 8.0),
                title: "xterm".to_string(),
            },
            8,
            16,
        );
        let err = NativeSurface::send_surface_bytes(&mut window, b"hi\xff").unwrap_err();
        assert!(err.to_string().contains("non-UTF-8 byte input"), "{err}");
    }

    #[test]
    fn capture_only_surface_byte_hook_reports_unsupported_text_input() {
        let mut surface = RgbaFrameSurface::new("rgba:empty", "empty", 1, 1, vec![0; 4]).unwrap();
        let err = NativeSurface::send_surface_bytes(&mut surface, b"hello").unwrap_err();
        assert!(
            err.to_string().contains("do not accept text input"),
            "{err}"
        );
    }

    #[test]
    fn native_surface_trait_drains_pty_surface_events() {
        let mut term = PtyTerminalApp::spawn("printf '\\033]2;trait title\\007'", 40, 4)
            .expect("spawn pty surface event probe");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && term.title() != "trait title" {
            std::thread::sleep(Duration::from_millis(20));
        }
        let events = NativeSurface::take_surface_events(&mut term);
        assert!(
            events
                .iter()
                .any(|event| event == &SurfaceEvent::TitleChanged("trait title".to_string())),
            "events were {events:?}"
        );
        assert!(NativeSurface::take_surface_events(&mut term).is_empty());
    }

    #[test]
    fn capture_only_native_surfaces_default_to_no_surface_events() {
        let mut surface = RgbaFrameSurface::new("rgba:empty", "empty", 1, 1, vec![0; 4]).unwrap();
        assert!(NativeSurface::take_surface_events(&mut surface).is_empty());
    }

    #[test]
    fn surface_capability_accessors_reflect_pty_and_capture_only_metadata() {
        let term = PtyTerminalApp::spawn("printf caps", 20, 3).expect("spawn pty caps probe");
        let pty = NativeSurface::metadata(&term).capabilities;
        assert!(pty.can_capture());
        assert!(pty.can_send_text());
        assert!(pty.can_send_bytes());
        assert!(pty.can_receive_focus_events());
        assert!(pty.can_emit_surface_events());
        assert!(pty.can_resize());
        assert!(pty.has_title());
        assert!(!pty.can_restore());

        let surface = RgbaFrameSurface::new("rgba:caps", "caps", 1, 1, vec![0; 4]).unwrap();
        let capture = NativeSurface::metadata(&surface).capabilities;
        assert!(capture.can_capture());
        assert!(!capture.can_send_text());
        assert!(!capture.can_send_bytes());
        assert!(!capture.can_receive_focus_events());
        assert!(!capture.can_emit_surface_events());
        assert!(!capture.can_resize());
        assert!(capture.has_title());
        assert!(!capture.can_restore());
    }

    #[test]
    fn pty_terminal_advertises_native_surface_metadata() {
        let mut term =
            PtyTerminalApp::spawn("printf surface-ready", 40, 6).expect("spawn pty surface probe");
        let metadata = NativeSurface::metadata(&term);
        assert!(metadata.id.as_str().starts_with("pty:"));
        assert_eq!(metadata.kind, SurfaceKind::Terminal);
        assert!(metadata.capabilities.capture);
        assert!(metadata.capabilities.input);
        assert!(metadata.capabilities.exact_byte_input);
        assert!(metadata.capabilities.focus_events);
        assert!(metadata.capabilities.surface_events);
        assert!(metadata.capabilities.resize);
        assert!(metadata.capabilities.title);
        assert_eq!(metadata.frame_size, None);

        let frame = NativeSurface::capture_surface(&mut term).unwrap();
        assert_eq!(frame.metadata.kind, SurfaceKind::Terminal);
        assert_eq!(frame.metadata.frame_size, Some((320, 96)));
        assert!(matches!(frame.frame, NativeFrame::Rgba { .. }));
    }

    #[test]
    fn pty_terminal_injects_kittwm_environment() {
        let term = PtyTerminalApp::spawn_with_env(
            "printf \"$KITTWM_WINDOW/$KITTWM_SOCKET\"",
            60,
            4,
            [
                ("KITTWM_WINDOW", "native-1"),
                ("KITTWM_SOCKET", "/tmp/kittwm-test.sock"),
            ],
        )
        .expect("spawn pty env probe");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !term.text_snapshot().contains("native-1") {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = term.text_snapshot();
        assert!(
            text.contains("native-1//tmp/kittwm-test.sock"),
            "snapshot was:\n{text}"
        );
    }

    #[test]
    fn parses_chrome_devtools_port() {
        assert_eq!(
            parse_devtools_port(
                "noise\nDevTools listening on ws://127.0.0.1:54321/devtools/browser/abc\n"
            ),
            Some(54321)
        );
    }

    #[test]
    fn browser_semantic_snapshot_maps_common_dom_controls() {
        let value = json!({
            "title": "Settings",
            "nodes": [
                {"id":"dom:save","role":"button","label":"Save","focusable":true,"disabled":false},
                {"id":"dom:name","role":"textbox","label":"Name","value":"Ada","focusable":true},
                {"id":"dom:subscribe","role":"checkbox","label":"Subscribe","checked":true,"focusable":true},
                {"id":"dom:home","role":"link","label":"Home","href":"https://example.test/","focusable":true},
                {"id":"dom:secret","role":"textbox","label":"Password","value":"hidden","sensitive":true,"focusable":true},
                {"id":"dom:canvas:1","role":"pixel_region","label":"Chart"}
            ]
        });
        let snapshot =
            browser_semantic_snapshot_from_value("browser:42", "fallback", value).unwrap();
        assert_eq!(snapshot.surface, "browser:42");
        assert_eq!(snapshot.root.label.as_deref(), Some("Settings"));
        assert_eq!(snapshot.root.children.len(), 6);
        assert_eq!(snapshot.root.children[0].role, ComponentRole::Button);
        assert_eq!(snapshot.root.children[1].role, ComponentRole::TextInput);
        assert_eq!(
            snapshot.root.children[1].value,
            Some(ComponentValue::Text("Ada".to_string()))
        );
        assert_eq!(snapshot.root.children[2].role, ComponentRole::Checkbox);
        assert!(snapshot.root.children[2].state.checked);
        assert_eq!(snapshot.root.children[3].role, ComponentRole::Link);
        assert_eq!(
            snapshot.root.children[3].description.as_deref(),
            Some("https://example.test/")
        );
        assert!(snapshot.root.children[4].state.sensitive);
        assert!(snapshot.root.children[4].value.is_none());
        assert_eq!(snapshot.root.children[5].role, ComponentRole::Canvas);
    }

    #[test]
    fn browser_semantic_action_script_routes_focus_and_value_actions() {
        let focus = browser_semantic_action_script("dom:name", "focus", json!({})).unwrap();
        assert!(focus.contains("const targetId = \"dom:name\""));
        assert!(focus.contains("const action = \"focus\""));
        assert!(focus.contains("stale-component"));

        let set_value =
            browser_semantic_action_script("dom:name", "set_value", json!({"value":"Ada"}))
                .unwrap();
        assert!(set_value.contains("const action = \"set_value\""));
        assert!(set_value.contains("\"value\":\"Ada\""));
        assert!(set_value.contains("dispatchValue"));
        assert!(browser_semantic_action_script("dom:name", "delete", json!({})).is_err());
    }

    #[test]
    fn browser_key_event_params_map_editing_and_page_keys() {
        let params = browser_key_event_params("Enter", "Enter", 13);
        assert_eq!(params["key"], "Enter");
        assert_eq!(params["code"], "Enter");
        assert_eq!(params["windowsVirtualKeyCode"], 13);
        assert_eq!(params["nativeVirtualKeyCode"], 13);
        let ctrl_backspace = browser_key_event_params_with_modifiers(
            "Backspace",
            "Backspace",
            8,
            BROWSER_CTRL_MODIFIER,
        );
        assert_eq!(ctrl_backspace["key"], "Backspace");
        assert_eq!(ctrl_backspace["code"], "Backspace");
        assert_eq!(ctrl_backspace["windowsVirtualKeyCode"], 8);
        assert_eq!(ctrl_backspace["nativeVirtualKeyCode"], 8);
        assert_eq!(ctrl_backspace["modifiers"], BROWSER_CTRL_MODIFIER);
        let shift_backspace = browser_key_event_params_with_modifiers(
            "Backspace",
            "Backspace",
            8,
            BROWSER_SHIFT_MODIFIER,
        );
        assert_eq!(shift_backspace["key"], "Backspace");
        assert_eq!(shift_backspace["code"], "Backspace");
        assert_eq!(shift_backspace["windowsVirtualKeyCode"], 8);
        assert_eq!(shift_backspace["nativeVirtualKeyCode"], 8);
        assert_eq!(shift_backspace["modifiers"], BROWSER_SHIFT_MODIFIER);
        let alt_backspace = browser_key_event_params_with_modifiers(
            "Backspace",
            "Backspace",
            8,
            BROWSER_ALT_MODIFIER,
        );
        assert_eq!(alt_backspace["key"], "Backspace");
        assert_eq!(alt_backspace["code"], "Backspace");
        assert_eq!(alt_backspace["windowsVirtualKeyCode"], 8);
        assert_eq!(alt_backspace["nativeVirtualKeyCode"], 8);
        assert_eq!(alt_backspace["modifiers"], BROWSER_ALT_MODIFIER);
        let shift_enter =
            browser_key_event_params_with_modifiers("Enter", "Enter", 13, BROWSER_SHIFT_MODIFIER);
        assert_eq!(shift_enter["key"], "Enter");
        assert_eq!(shift_enter["code"], "Enter");
        assert_eq!(shift_enter["windowsVirtualKeyCode"], 13);
        assert_eq!(shift_enter["nativeVirtualKeyCode"], 13);
        assert_eq!(shift_enter["modifiers"], BROWSER_SHIFT_MODIFIER);
        let ctrl_enter =
            browser_key_event_params_with_modifiers("Enter", "Enter", 13, BROWSER_CTRL_MODIFIER);
        assert_eq!(ctrl_enter["key"], "Enter");
        assert_eq!(ctrl_enter["code"], "Enter");
        assert_eq!(ctrl_enter["windowsVirtualKeyCode"], 13);
        assert_eq!(ctrl_enter["nativeVirtualKeyCode"], 13);
        assert_eq!(ctrl_enter["modifiers"], BROWSER_CTRL_MODIFIER);
        let alt_enter =
            browser_key_event_params_with_modifiers("Enter", "Enter", 13, BROWSER_ALT_MODIFIER);
        assert_eq!(alt_enter["key"], "Enter");
        assert_eq!(alt_enter["code"], "Enter");
        assert_eq!(alt_enter["windowsVirtualKeyCode"], 13);
        assert_eq!(alt_enter["nativeVirtualKeyCode"], 13);
        assert_eq!(alt_enter["modifiers"], BROWSER_ALT_MODIFIER);
        let (arrow_key, arrow_code, arrow_code_num) = BrowserArrowKey::Left.key_fields();
        let shift_arrow = browser_key_event_params_with_modifiers(
            arrow_key,
            arrow_code,
            arrow_code_num,
            BROWSER_SHIFT_MODIFIER,
        );
        assert_eq!(shift_arrow["key"], "ArrowLeft");
        assert_eq!(shift_arrow["code"], "ArrowLeft");
        assert_eq!(shift_arrow["windowsVirtualKeyCode"], 37);
        assert_eq!(shift_arrow["nativeVirtualKeyCode"], 37);
        assert_eq!(shift_arrow["modifiers"], BROWSER_SHIFT_MODIFIER);
        let ctrl_arrow = browser_key_event_params_with_modifiers(
            arrow_key,
            arrow_code,
            arrow_code_num,
            BROWSER_CTRL_MODIFIER,
        );
        assert_eq!(ctrl_arrow["key"], "ArrowLeft");
        assert_eq!(ctrl_arrow["code"], "ArrowLeft");
        assert_eq!(ctrl_arrow["windowsVirtualKeyCode"], 37);
        assert_eq!(ctrl_arrow["nativeVirtualKeyCode"], 37);
        assert_eq!(ctrl_arrow["modifiers"], BROWSER_CTRL_MODIFIER);
        let alt_arrow = browser_key_event_params_with_modifiers(
            arrow_key,
            arrow_code,
            arrow_code_num,
            BROWSER_ALT_MODIFIER,
        );
        assert_eq!(alt_arrow["key"], "ArrowLeft");
        assert_eq!(alt_arrow["code"], "ArrowLeft");
        assert_eq!(alt_arrow["windowsVirtualKeyCode"], 37);
        assert_eq!(alt_arrow["nativeVirtualKeyCode"], 37);
        assert_eq!(alt_arrow["modifiers"], BROWSER_ALT_MODIFIER);
        let escape = browser_key_event_params("Escape", "Escape", 27);
        assert_eq!(escape["key"], "Escape");
        assert_eq!(escape["code"], "Escape");
        assert_eq!(escape["windowsVirtualKeyCode"], 27);
        assert_eq!(escape["nativeVirtualKeyCode"], 27);
        let insert = browser_key_event_params("Insert", "Insert", 45);
        assert_eq!(insert["key"], "Insert");
        assert_eq!(insert["code"], "Insert");
        assert_eq!(insert["windowsVirtualKeyCode"], 45);
        assert_eq!(insert["nativeVirtualKeyCode"], 45);
        let delete = browser_key_event_params("Delete", "Delete", 46);
        assert_eq!(delete["key"], "Delete");
        assert_eq!(delete["code"], "Delete");
        assert_eq!(delete["windowsVirtualKeyCode"], 46);
        assert_eq!(delete["nativeVirtualKeyCode"], 46);
        let shift_insert =
            browser_key_event_params_with_modifiers("Insert", "Insert", 45, BROWSER_SHIFT_MODIFIER);
        assert_eq!(shift_insert["key"], "Insert");
        assert_eq!(shift_insert["code"], "Insert");
        assert_eq!(shift_insert["windowsVirtualKeyCode"], 45);
        assert_eq!(shift_insert["nativeVirtualKeyCode"], 45);
        assert_eq!(shift_insert["modifiers"], BROWSER_SHIFT_MODIFIER);
        let shift_delete =
            browser_key_event_params_with_modifiers("Delete", "Delete", 46, BROWSER_SHIFT_MODIFIER);
        assert_eq!(shift_delete["key"], "Delete");
        assert_eq!(shift_delete["code"], "Delete");
        assert_eq!(shift_delete["windowsVirtualKeyCode"], 46);
        assert_eq!(shift_delete["nativeVirtualKeyCode"], 46);
        assert_eq!(shift_delete["modifiers"], BROWSER_SHIFT_MODIFIER);
        let ctrl_insert =
            browser_key_event_params_with_modifiers("Insert", "Insert", 45, BROWSER_CTRL_MODIFIER);
        assert_eq!(ctrl_insert["key"], "Insert");
        assert_eq!(ctrl_insert["code"], "Insert");
        assert_eq!(ctrl_insert["windowsVirtualKeyCode"], 45);
        assert_eq!(ctrl_insert["nativeVirtualKeyCode"], 45);
        assert_eq!(ctrl_insert["modifiers"], BROWSER_CTRL_MODIFIER);
        let ctrl_delete =
            browser_key_event_params_with_modifiers("Delete", "Delete", 46, BROWSER_CTRL_MODIFIER);
        assert_eq!(ctrl_delete["key"], "Delete");
        assert_eq!(ctrl_delete["code"], "Delete");
        assert_eq!(ctrl_delete["windowsVirtualKeyCode"], 46);
        assert_eq!(ctrl_delete["nativeVirtualKeyCode"], 46);
        assert_eq!(ctrl_delete["modifiers"], BROWSER_CTRL_MODIFIER);
        let alt_insert =
            browser_key_event_params_with_modifiers("Insert", "Insert", 45, BROWSER_ALT_MODIFIER);
        assert_eq!(alt_insert["key"], "Insert");
        assert_eq!(alt_insert["code"], "Insert");
        assert_eq!(alt_insert["windowsVirtualKeyCode"], 45);
        assert_eq!(alt_insert["nativeVirtualKeyCode"], 45);
        assert_eq!(alt_insert["modifiers"], BROWSER_ALT_MODIFIER);
        let alt_delete =
            browser_key_event_params_with_modifiers("Delete", "Delete", 46, BROWSER_ALT_MODIFIER);
        assert_eq!(alt_delete["key"], "Delete");
        assert_eq!(alt_delete["code"], "Delete");
        assert_eq!(alt_delete["windowsVirtualKeyCode"], 46);
        assert_eq!(alt_delete["nativeVirtualKeyCode"], 46);
        assert_eq!(alt_delete["modifiers"], BROWSER_ALT_MODIFIER);
        let home = browser_key_event_params("Home", "Home", 36);
        assert_eq!(home["key"], "Home");
        assert_eq!(home["code"], "Home");
        assert_eq!(home["windowsVirtualKeyCode"], 36);
        assert_eq!(home["nativeVirtualKeyCode"], 36);
        let end = browser_key_event_params("End", "End", 35);
        assert_eq!(end["key"], "End");
        assert_eq!(end["code"], "End");
        assert_eq!(end["windowsVirtualKeyCode"], 35);
        assert_eq!(end["nativeVirtualKeyCode"], 35);
        let shift_home =
            browser_key_event_params_with_modifiers("Home", "Home", 36, BROWSER_SHIFT_MODIFIER);
        assert_eq!(shift_home["key"], "Home");
        assert_eq!(shift_home["code"], "Home");
        assert_eq!(shift_home["windowsVirtualKeyCode"], 36);
        assert_eq!(shift_home["nativeVirtualKeyCode"], 36);
        assert_eq!(shift_home["modifiers"], BROWSER_SHIFT_MODIFIER);
        let shift_end =
            browser_key_event_params_with_modifiers("End", "End", 35, BROWSER_SHIFT_MODIFIER);
        assert_eq!(shift_end["key"], "End");
        assert_eq!(shift_end["code"], "End");
        assert_eq!(shift_end["windowsVirtualKeyCode"], 35);
        assert_eq!(shift_end["nativeVirtualKeyCode"], 35);
        assert_eq!(shift_end["modifiers"], BROWSER_SHIFT_MODIFIER);
        let ctrl_home =
            browser_key_event_params_with_modifiers("Home", "Home", 36, BROWSER_CTRL_MODIFIER);
        assert_eq!(ctrl_home["key"], "Home");
        assert_eq!(ctrl_home["code"], "Home");
        assert_eq!(ctrl_home["windowsVirtualKeyCode"], 36);
        assert_eq!(ctrl_home["nativeVirtualKeyCode"], 36);
        assert_eq!(ctrl_home["modifiers"], BROWSER_CTRL_MODIFIER);
        let ctrl_end =
            browser_key_event_params_with_modifiers("End", "End", 35, BROWSER_CTRL_MODIFIER);
        assert_eq!(ctrl_end["key"], "End");
        assert_eq!(ctrl_end["code"], "End");
        assert_eq!(ctrl_end["windowsVirtualKeyCode"], 35);
        assert_eq!(ctrl_end["nativeVirtualKeyCode"], 35);
        assert_eq!(ctrl_end["modifiers"], BROWSER_CTRL_MODIFIER);
        let alt_home =
            browser_key_event_params_with_modifiers("Home", "Home", 36, BROWSER_ALT_MODIFIER);
        assert_eq!(alt_home["key"], "Home");
        assert_eq!(alt_home["code"], "Home");
        assert_eq!(alt_home["windowsVirtualKeyCode"], 36);
        assert_eq!(alt_home["nativeVirtualKeyCode"], 36);
        assert_eq!(alt_home["modifiers"], BROWSER_ALT_MODIFIER);
        let alt_end =
            browser_key_event_params_with_modifiers("End", "End", 35, BROWSER_ALT_MODIFIER);
        assert_eq!(alt_end["key"], "End");
        assert_eq!(alt_end["code"], "End");
        assert_eq!(alt_end["windowsVirtualKeyCode"], 35);
        assert_eq!(alt_end["nativeVirtualKeyCode"], 35);
        assert_eq!(alt_end["modifiers"], BROWSER_ALT_MODIFIER);
        let (page_up_key, page_up_code, page_up_code_num) = BrowserPageKey::Up.key_fields();
        let page_up = browser_key_event_params(page_up_key, page_up_code, page_up_code_num);
        assert_eq!(page_up["key"], "PageUp");
        assert_eq!(page_up["code"], "PageUp");
        assert_eq!(page_up["windowsVirtualKeyCode"], 33);
        assert_eq!(page_up["nativeVirtualKeyCode"], 33);
        let shift_page_up = browser_key_event_params_with_modifiers(
            page_up_key,
            page_up_code,
            page_up_code_num,
            BROWSER_SHIFT_MODIFIER,
        );
        assert_eq!(shift_page_up["key"], "PageUp");
        assert_eq!(shift_page_up["code"], "PageUp");
        assert_eq!(shift_page_up["windowsVirtualKeyCode"], 33);
        assert_eq!(shift_page_up["nativeVirtualKeyCode"], 33);
        assert_eq!(shift_page_up["modifiers"], BROWSER_SHIFT_MODIFIER);
        let (page_down_key, page_down_code, page_down_code_num) = BrowserPageKey::Down.key_fields();
        let page_down = browser_key_event_params(page_down_key, page_down_code, page_down_code_num);
        assert_eq!(page_down["key"], "PageDown");
        assert_eq!(page_down["code"], "PageDown");
        assert_eq!(page_down["windowsVirtualKeyCode"], 34);
        assert_eq!(page_down["nativeVirtualKeyCode"], 34);
        let shift_page_down = browser_key_event_params_with_modifiers(
            page_down_key,
            page_down_code,
            page_down_code_num,
            BROWSER_SHIFT_MODIFIER,
        );
        assert_eq!(shift_page_down["key"], "PageDown");
        assert_eq!(shift_page_down["code"], "PageDown");
        assert_eq!(shift_page_down["windowsVirtualKeyCode"], 34);
        assert_eq!(shift_page_down["nativeVirtualKeyCode"], 34);
        assert_eq!(shift_page_down["modifiers"], BROWSER_SHIFT_MODIFIER);
        let ctrl_page_up = browser_key_event_params_with_modifiers(
            page_up_key,
            page_up_code,
            page_up_code_num,
            BROWSER_CTRL_MODIFIER,
        );
        assert_eq!(ctrl_page_up["key"], "PageUp");
        assert_eq!(ctrl_page_up["code"], "PageUp");
        assert_eq!(ctrl_page_up["windowsVirtualKeyCode"], 33);
        assert_eq!(ctrl_page_up["nativeVirtualKeyCode"], 33);
        assert_eq!(ctrl_page_up["modifiers"], BROWSER_CTRL_MODIFIER);
        let ctrl_page_down = browser_key_event_params_with_modifiers(
            page_down_key,
            page_down_code,
            page_down_code_num,
            BROWSER_CTRL_MODIFIER,
        );
        assert_eq!(ctrl_page_down["key"], "PageDown");
        assert_eq!(ctrl_page_down["code"], "PageDown");
        assert_eq!(ctrl_page_down["windowsVirtualKeyCode"], 34);
        assert_eq!(ctrl_page_down["nativeVirtualKeyCode"], 34);
        assert_eq!(ctrl_page_down["modifiers"], BROWSER_CTRL_MODIFIER);
        let alt_page_up = browser_key_event_params_with_modifiers(
            page_up_key,
            page_up_code,
            page_up_code_num,
            BROWSER_ALT_MODIFIER,
        );
        assert_eq!(alt_page_up["key"], "PageUp");
        assert_eq!(alt_page_up["code"], "PageUp");
        assert_eq!(alt_page_up["windowsVirtualKeyCode"], 33);
        assert_eq!(alt_page_up["nativeVirtualKeyCode"], 33);
        assert_eq!(alt_page_up["modifiers"], BROWSER_ALT_MODIFIER);
        let alt_page_down = browser_key_event_params_with_modifiers(
            page_down_key,
            page_down_code,
            page_down_code_num,
            BROWSER_ALT_MODIFIER,
        );
        assert_eq!(alt_page_down["key"], "PageDown");
        assert_eq!(alt_page_down["code"], "PageDown");
        assert_eq!(alt_page_down["windowsVirtualKeyCode"], 34);
        assert_eq!(alt_page_down["nativeVirtualKeyCode"], 34);
        assert_eq!(alt_page_down["modifiers"], BROWSER_ALT_MODIFIER);
        let shift_tab =
            browser_key_event_params_with_modifiers("Tab", "Tab", 9, BROWSER_SHIFT_MODIFIER);
        assert_eq!(shift_tab["key"], "Tab");
        assert_eq!(shift_tab["code"], "Tab");
        assert_eq!(shift_tab["windowsVirtualKeyCode"], 9);
        assert_eq!(shift_tab["nativeVirtualKeyCode"], 9);
        assert_eq!(shift_tab["modifiers"], BROWSER_SHIFT_MODIFIER);
    }

    #[test]
    fn browser_semantic_snapshot_falls_back_to_empty_root_for_opaque_pages() {
        let snapshot = browser_semantic_snapshot_from_value(
            "browser:7",
            "Opaque Canvas",
            json!({"nodes": []}),
        )
        .unwrap();
        assert_eq!(snapshot.root.label.as_deref(), Some("Opaque Canvas"));
        assert!(snapshot.root.children.is_empty());
    }

    #[test]
    fn browser_surface_metadata_uses_devtools_dimensions() {
        let metadata = browser_surface_metadata(4242, "data:text/html,hi".to_string(), 320, 200);
        assert_eq!(metadata.id.as_str(), "browser:4242");
        assert_eq!(metadata.kind, SurfaceKind::Browser);
        assert_eq!(metadata.title, "data:text/html,hi");
        assert_eq!(metadata.frame_size, Some((320, 200)));
        assert!(metadata.capabilities.capture);
        assert!(metadata.capabilities.input);
        assert!(metadata.capabilities.resize);
        assert!(metadata.capabilities.title);
    }

    #[test]
    fn headless_browser_data_url_screenshot_when_chrome_available() {
        if find_chrome().is_none() {
            eprintln!("skipping: Chrome/Chromium not found");
            return;
        }
        let mut app = HeadlessBrowserApp::launch(
            "data:text/html,<html><body><button autofocus>hi</button></body></html>",
            320,
            200,
        )
        .expect("launch headless browser");
        app.send_text("abc").unwrap();
        app.click(10, 10).unwrap();
        let frame = app.capture().unwrap();
        let NativeFrame::Png {
            width,
            height,
            bytes,
        } = frame
        else {
            panic!("expected PNG")
        };
        assert_eq!((width, height), (320, 200));
        assert!(bytes.starts_with(b"\x89PNG"));
    }
}
