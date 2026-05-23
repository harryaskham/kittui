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
use kittui_xvfb::{XCapture, XWindowId};
use parking_lot::Mutex;
use portable_pty::{
    Child as PtyChild, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem,
};
use serde_json::json;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};
use vte::{Params, Parser, Perform};

const SCROLLBACK_MAX_LINES: usize = 10_000;

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

/// A nested PTY terminal rendered into an RGBA frame.
pub struct PtyTerminalApp {
    title: String,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn PtyChild + Send + Sync>,
    writer: Box<dyn Write + Send>,
    state: Arc<Mutex<TerminalState>>,
    _reader: JoinHandle<()>,
    cell_width: u32,
    cell_height: u32,
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
        let mut builder = CommandBuilder::new("/bin/sh");
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
        let mut reader = pair.master.try_clone_reader().context("clone PTY reader")?;
        let writer = pair.master.take_writer().context("take PTY writer")?;
        let state = Arc::new(Mutex::new(TerminalState::new(cols, rows)));
        let reader_state = state.clone();
        let join = std::thread::spawn(move || {
            let mut parser = Parser::new();
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = reader.read(&mut buf) else { break };
                if n == 0 {
                    break;
                }
                let mut state = reader_state.lock();
                parser.advance(&mut *state, &buf[..n]);
            }
        });
        Ok(Self {
            title: command.to_string(),
            master: pair.master,
            child,
            writer,
            state,
            _reader: join,
            cell_width: 8,
            cell_height: 16,
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

    /// Return the current zero-based cursor `(col, row)` in the terminal grid.
    pub fn cursor_position(&self) -> (u16, u16) {
        let state = self.state.lock();
        (state.cursor_col, state.cursor_row)
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
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }
}

impl NativeApp for PtyTerminalApp {
    fn title(&self) -> String {
        self.state
            .lock()
            .title
            .clone()
            .unwrap_or_else(|| self.title.clone())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: cols.saturating_mul(self.cell_width.min(u32::from(u16::MAX)) as u16),
            pixel_height: rows.saturating_mul(self.cell_height.min(u32::from(u16::MAX)) as u16),
        })?;
        self.state.lock().resize(cols, rows);
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    fn capture(&mut self) -> Result<NativeFrame> {
        let state = self.state.lock().clone();
        Ok(NativeFrame::Rgba {
            width: u32::from(state.cols) * self.cell_width,
            height: u32::from(state.rows) * self.cell_height,
            rgba: render_terminal_rgba(&state, self.cell_width, self.cell_height),
        })
    }
}

#[derive(Clone)]
struct TerminalState {
    cols: u16,
    rows: u16,
    cursor_col: u16,
    cursor_row: u16,
    cells: Vec<char>,
    scrollback: Vec<String>,
    alt_screen: Option<AlternateScreen>,
    title: Option<String>,
}

#[derive(Clone)]
struct AlternateScreen {
    normal_cells: Vec<char>,
    normal_cursor_col: u16,
    normal_cursor_row: u16,
}

impl TerminalState {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            cells: vec![' '; usize::from(cols) * usize::from(rows)],
            scrollback: Vec::new(),
            alt_screen: None,
            title: None,
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let old = self.clone();
        *self = Self::new(cols, rows);
        self.title = old.title.clone();
        self.scrollback = old.scrollback.clone();
        self.cells = resize_cells(&old.cells, old.cols, old.rows, cols, rows);
        self.alt_screen = old.alt_screen.map(|alt| AlternateScreen {
            normal_cells: resize_cells(&alt.normal_cells, old.cols, old.rows, cols, rows),
            normal_cursor_col: alt.normal_cursor_col.min(cols.saturating_sub(1)),
            normal_cursor_row: alt.normal_cursor_row.min(rows.saturating_sub(1)),
        });
        self.cursor_col = old.cursor_col.min(cols.saturating_sub(1));
        self.cursor_row = old.cursor_row.min(rows.saturating_sub(1));
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

    fn line_snapshot(&self, row: u16) -> String {
        let start = usize::from(row) * usize::from(self.cols);
        let end = start + usize::from(self.cols);
        self.cells[start..end]
            .iter()
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
            self.cells[idx] = ch;
        }
    }

    fn get_at(&self, col: u16, row: u16) -> char {
        if col < self.cols && row < self.rows {
            self.cells[usize::from(row) * usize::from(self.cols) + usize::from(col)]
        } else {
            ' '
        }
    }

    fn newline(&mut self) {
        self.cursor_col = 0;
        if self.cursor_row + 1 >= self.rows {
            if self.alt_screen.is_none() {
                self.push_scrollback_line(self.line_snapshot(0));
            }
            let cols = usize::from(self.cols);
            self.cells.copy_within(cols.., 0);
            let start = self.cells.len().saturating_sub(cols);
            for cell in &mut self.cells[start..] {
                *cell = ' ';
            }
        } else {
            self.cursor_row += 1;
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
        if self.cursor_col >= self.cols {
            self.newline();
        }
        self.put_at(self.cursor_col, self.cursor_row, ch);
        self.cursor_col += 1;
    }

    fn set_title_from_osc(&mut self, params: &[&[u8]]) {
        let Some(kind) = params
            .first()
            .and_then(|param| std::str::from_utf8(param).ok())
        else {
            return;
        };
        if !matches!(kind, "0" | "1" | "2") {
            return;
        }
        let title = params
            .get(1..)
            .unwrap_or_default()
            .iter()
            .flat_map(|part| std::str::from_utf8(part).ok())
            .collect::<Vec<_>>()
            .join(";");
        if !title.is_empty() {
            self.title = Some(title);
        }
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
            self.put_at(
                col + count,
                self.cursor_row,
                self.get_at(col, self.cursor_row),
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
            self.put_at(
                col - count,
                self.cursor_row,
                self.get_at(col, self.cursor_row),
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
            *cell = ' ';
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
            *cell = ' ';
        }
    }

    fn enter_alternate_screen(&mut self) {
        if self.alt_screen.is_some() {
            self.cells.fill(' ');
            self.cursor_col = 0;
            self.cursor_row = 0;
            return;
        }
        let normal_cells = std::mem::replace(
            &mut self.cells,
            vec![' '; usize::from(self.cols) * usize::from(self.rows)],
        );
        self.alt_screen = Some(AlternateScreen {
            normal_cells,
            normal_cursor_col: self.cursor_col,
            normal_cursor_row: self.cursor_row,
        });
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    fn leave_alternate_screen(&mut self) {
        if let Some(alt) = self.alt_screen.take() {
            self.cells = alt.normal_cells;
            self.cursor_col = alt.normal_cursor_col.min(self.cols.saturating_sub(1));
            self.cursor_row = alt.normal_cursor_row.min(self.rows.saturating_sub(1));
        }
    }
}

fn resize_cells(old: &[char], old_cols: u16, old_rows: u16, cols: u16, rows: u16) -> Vec<char> {
    let mut cells = vec![' '; usize::from(cols) * usize::from(rows)];
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
            0x08 => self.cursor_col = self.cursor_col.saturating_sub(1),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.set_title_from_osc(params);
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
                self.cursor_row = row.saturating_sub(1).min(self.rows.saturating_sub(1));
                self.cursor_col = col.saturating_sub(1).min(self.cols.saturating_sub(1));
            }
            'h' if is_dec_private && has_alt_screen_mode => self.enter_alternate_screen(),
            'J' => match first_raw {
                0 => self.clear_screen_range(
                    self.cursor_row,
                    self.cursor_col,
                    self.rows.saturating_sub(1),
                    self.cols.saturating_sub(1),
                ),
                1 => self.clear_screen_range(0, 0, self.cursor_row, self.cursor_col),
                2 => self.cells.fill(' '),
                _ => {}
            },
            'K' => match first_raw {
                0 => self.clear_line_range(self.cursor_col, self.cols.saturating_sub(1)),
                1 => self.clear_line_range(0, self.cursor_col),
                2 => self.clear_line_range(0, self.cols.saturating_sub(1)),
                _ => {}
            },
            'L' => self.insert_lines(first_count),
            'l' if is_dec_private && has_alt_screen_mode => self.leave_alternate_screen(),
            'M' => self.delete_lines(first_count),
            'P' => self.delete_chars(first_count),
            'X' => self.erase_chars(first_count),
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
            let ch = state.get_at(col, row);
            if ch == ' ' {
                continue;
            }
            draw_pseudo_glyph(&mut rgba, width, col, row, cell_w, cell_h, ch);
        }
    }
    rgba
}

fn draw_pseudo_glyph(
    rgba: &mut [u8],
    width: u32,
    col: u16,
    row: u16,
    cell_w: u32,
    cell_h: u32,
    ch: char,
) {
    let seed = ch as u32;
    let x0 = u32::from(col) * cell_w;
    let y0 = u32::from(row) * cell_h;
    for y in 2..cell_h.saturating_sub(2) {
        for x in 1..cell_w.saturating_sub(1) {
            let stroke = x == 1
                || x == cell_w.saturating_sub(2)
                || y == 2
                || y == cell_h.saturating_sub(3)
                || ((seed.rotate_left((x % 7) + 1) ^ y) & 0x3) == 0;
            if stroke {
                let idx = (((y0 + y) * width + (x0 + x)) as usize) * 4;
                rgba[idx] = 0xd7;
                rgba[idx + 1] = 0xf8;
                rgba[idx + 2] = 0xff;
                rgba[idx + 3] = 0xff;
            }
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
        self.width = u32::from(cols) * 8;
        self.height = u32::from(rows) * 16;
        self.cdp(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": self.width, "height": self.height, "deviceScaleFactor": 1, "mobile": false}),
        )?;
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        for ch in text.chars() {
            self.cdp(
                "Input.dispatchKeyEvent",
                json!({"type": "char", "text": ch.to_string()}),
            )?;
        }
        Ok(())
    }

    fn capture(&mut self) -> Result<NativeFrame> {
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
        Ok(NativeFrame::Png {
            width,
            height,
            bytes,
        })
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
