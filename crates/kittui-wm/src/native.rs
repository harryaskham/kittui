//! Native kittwm app backends that do not require X11/Quartz windows.
//!
//! These adapters make local processes look like compositor surfaces. The PTY
//! backend turns a shell into a movable/resizable terminal pane; the headless
//! browser backend drives Chrome via the DevTools protocol and captures PNG
//! screenshots. They are intentionally small building blocks: higher layers can
//! wrap them in chrome, tiling, focus, and input policy just like X/Quartz
//! windows.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use kittui_xvfb::{XCapture, XServer, XWindow, XWindowId};
use parking_lot::Mutex;
use portable_pty::{
    Child as PtyChild, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem,
};
use serde_json::json;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};
use vte::{Params, Parser, Perform};

const SCROLLBACK_MAX_LINES: usize = 10_000;

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
    /// Surface can be resized.
    pub resize: bool,
    /// Surface exposes a human-readable title.
    pub title: bool,
    /// Surface can serialize restore metadata.
    pub restore: bool,
}

impl SurfaceCapabilities {
    /// Standard capabilities for live terminal-like/native app surfaces.
    pub fn interactive_capture() -> Self {
        Self {
            capture: true,
            input: true,
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
    /// Capture a frame and pair it with current metadata.
    fn capture_surface(&mut self) -> Result<SurfaceFrame>;
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
}

/// A nested PTY terminal rendered into an RGBA frame.
pub struct PtyTerminalApp {
    title: String,
    child: Box<dyn PtyChild + Send + Sync>,
    surface: TerminalSurface,
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
        })
    }

    /// Return the terminal grid as plain text for assertions and accessibility.
    pub fn text_snapshot(&self) -> String {
        self.state.lock().text_snapshot()
    }

    /// Return lines that have scrolled off the terminal grid as plain text.
    pub fn scrollback_snapshot(&self) -> String {
        self.state.lock().scrollback_snapshot()
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

    /// Render the current terminal state as an RGBA frame.
    pub fn capture(&mut self) -> Result<NativeFrame> {
        let state = self.state.lock().clone();
        Ok(NativeFrame::Rgba {
            width: u32::from(state.cols) * self.cell_width,
            height: u32::from(state.rows) * self.cell_height,
            rgba: render_terminal_rgba(&state, self.cell_width, self.cell_height),
        })
    }
}

impl PtyTerminalApp {
    /// Spawn a shell command in a real PTY.
    pub fn spawn(command: &str, cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with_env(command, cols, rows, std::iter::empty::<(&str, &str)>())
    }

    /// Spawn a shell command in a real PTY with extra environment variables.
    pub fn spawn_with_env<'a, I, K, V>(command: &str, cols: u16, rows: u16, envs: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr> + 'a,
        V: AsRef<std::ffi::OsStr> + 'a,
    {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: cols.saturating_mul(8),
                pixel_height: rows.saturating_mul(16),
            })
            .context("open PTY")?;
        let shell = std::env::var("KITTWM_PTY_SHELL").unwrap_or_else(|_| {
            std::env::var("SHELL").unwrap_or_else(|_| {
                if std::path::Path::new("/bin/sh").exists() {
                    "/bin/sh".to_string()
                } else {
                    "sh".to_string()
                }
            })
        });
        let mut builder = CommandBuilder::new(shell);
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
        let surface = TerminalSurface::from_master(pair.master, cols, rows, 8, 16)?;
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
        SurfaceMetadata {
            id: SurfaceId::new(format!(
                "pty:{}",
                self.process_id()
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )),
            kind: SurfaceKind::Terminal,
            title: self.surface.title().unwrap_or_else(|| self.title.clone()),
            capabilities: SurfaceCapabilities::interactive_capture(),
            frame_size: None,
        }
    }

    fn resize_surface(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.surface.resize(cols, rows)
    }

    fn send_surface_text(&mut self, text: &str) -> Result<()> {
        self.surface.send_text(text)
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalCell {
    ch: char,
    style: TerminalStyle,
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
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let old = self.clone();
        *self = Self::new(cols, rows);
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
        let mut out = String::new();
        for row in 0..self.rows {
            out.push_str(&self.line_snapshot(row));
            out.push('\n');
        }
        out
    }

    fn scrollback_snapshot(&self) -> String {
        if self.scrollback.is_empty() {
            return String::new();
        }
        let mut out = self.scrollback.join("\n");
        out.push('\n');
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
        let start = usize::from(row) * usize::from(self.cols);
        let end = start + usize::from(self.cols);
        self.cells[start..end]
            .iter()
            .map(|cell| cell.ch)
            .collect::<String>()
            .trim_end()
            .into()
    }

    fn push_scrollback_line(&mut self, line: String) {
        self.scrollback.push(line);
        if self.scrollback.len() > SCROLLBACK_MAX_LINES {
            let overflow = self.scrollback.len() - SCROLLBACK_MAX_LINES;
            self.scrollback.drain(0..overflow);
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
        let title = params
            .get(1..)
            .unwrap_or_default()
            .iter()
            .flat_map(|part| std::str::from_utf8(part).ok())
            .collect::<Vec<_>>()
            .join(";");
        if !title.is_empty() {
            self.title = Some(title.clone());
            self.queue_surface_event(SurfaceEvent::TitleChanged(title));
        }
    }

    fn notification_from_osc9(&mut self, params: &[&[u8]]) {
        let body = params
            .get(1..)
            .unwrap_or_default()
            .iter()
            .flat_map(|part| std::str::from_utf8(part).ok())
            .collect::<Vec<_>>()
            .join(";");
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
        let body = params
            .get(3..)
            .unwrap_or_default()
            .iter()
            .flat_map(|part| std::str::from_utf8(part).ok())
            .collect::<Vec<_>>()
            .join(";");
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
        let payload = params
            .get(2..)
            .unwrap_or_default()
            .iter()
            .flat_map(|part| std::str::from_utf8(part).ok())
            .collect::<Vec<_>>()
            .join(";");
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

fn render_terminal_rgba(state: &TerminalState, cell_w: u32, cell_h: u32) -> Vec<u8> {
    let width = u32::from(state.cols) * cell_w;
    let height = u32::from(state.rows) * cell_h;
    let mut rgba = vec![0x0b; (width as usize) * (height as usize) * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[0] = 0x08;
        px[1] = 0x0d;
        px[2] = 0x14;
        px[3] = 0xff;
    }
    for row in 0..state.rows {
        for col in 0..state.cols {
            let cell = state.get_cell_at(col, row);
            let (fg, bg) = terminal_cell_colors(cell.style);
            fill_cell_background(&mut rgba, width, col, row, cell_w, cell_h, bg);
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
    state: &TerminalState,
    cell_w: u32,
    cell_h: u32,
) {
    if state.cursor_col >= state.cols || state.cursor_row >= state.rows {
        return;
    }
    let cell = state.get_cell_at(state.cursor_col, state.cursor_row);
    let (fg, bg) = terminal_cell_colors(cell.style);
    let cursor = if cell.ch == ' ' { fg } else { bg };
    let x0 = u32::from(state.cursor_col) * cell_w;
    let y0 = u32::from(state.cursor_row) * cell_h;
    let start_y = cell_h.saturating_sub(3);
    for y in start_y..cell_h {
        for x in 0..cell_w {
            let idx = (((y0 + y) * width + (x0 + x)) as usize) * 4;
            rgba[idx] = cursor.0;
            rgba[idx + 1] = cursor.1;
            rgba[idx + 2] = cursor.2;
            rgba[idx + 3] = 0xff;
        }
    }
}

fn terminal_cell_colors(style: TerminalStyle) -> (TerminalColor, TerminalColor) {
    let mut fg = style.fg.unwrap_or(TerminalColor(0xd7, 0xf8, 0xff));
    let mut bg = style.bg.unwrap_or(TerminalColor(0x08, 0x0d, 0x14));
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
        for x in 0..cell_w {
            let idx = (((y0 + y) * width + (x0 + x)) as usize) * 4;
            rgba[idx] = color.0;
            rgba[idx + 1] = color.1;
            rgba[idx + 2] = color.2;
            rgba[idx + 3] = 0xff;
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
                    let px = x0 + left + gx * scale_x + sx;
                    let py = y0 + top + gy as u32 * scale_y + sy;
                    set_rgba_pixel(rgba, width, px, py, color);
                }
            }
        }
    }
}

fn set_rgba_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: TerminalColor) {
    let idx = ((y * width + x) as usize) * 4;
    if idx + 3 >= rgba.len() {
        return;
    }
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
                set_rgba_pixel(rgba, width, x0 + x, y0 + y, color);
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
    fn pty_terminal_advertises_native_surface_metadata() {
        let mut term =
            PtyTerminalApp::spawn("printf surface-ready", 40, 6).expect("spawn pty surface probe");
        let metadata = NativeSurface::metadata(&term);
        assert!(metadata.id.as_str().starts_with("pty:"));
        assert_eq!(metadata.kind, SurfaceKind::Terminal);
        assert!(metadata.capabilities.capture);
        assert!(metadata.capabilities.input);
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
