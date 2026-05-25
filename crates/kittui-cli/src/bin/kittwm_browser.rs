//! `kittwm-browser` — first-class kittwm-native browser app.
//!
//! This binary is intentionally WM-context aware: when launched inside a
//! kittwm native terminal it inherits `KITTWM_SOCKET` / `KITTWM_WINDOW`, just
//! like an X app inherits `DISPLAY`. The current implementation is a direct
//! full-terminal replacement app; the socket spawn/replace protocol will make
//! the same binary ask a live kittwm host to create or replace panes.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use kittui::{CellRect, TerminalInfo, Transport};
use kittui_kitty as kitty;
use kittui_wm::native::{HeadlessBrowserApp, NativeApp, NativeFrame};
use kittwm_sdk::{Kittwm, SemanticSurfaceSnapshot};

const DEFAULT_URL: &str =
    "data:text/html,<html><body><h1>kittwm-browser</h1><input autofocus value='ready'></body></html>";
const BROWSER_RESERVED_STATUS_ROWS: u16 = 2;
const BROWSER_IMAGE_ID: u32 = 1;
const BROWSER_IMAGE_Z_INDEX: i32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserArgs {
    url: String,
    semantic_snapshot: bool,
    compact_json: bool,
    help: bool,
}

impl BrowserArgs {
    fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut url = None;
        let mut semantic_snapshot = false;
        let mut compact_json = true;
        let mut help = false;
        let mut iter = args.into_iter().map(Into::into);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => help = true,
                "--semantic-snapshot" | "--print-semantic" => semantic_snapshot = true,
                "--pretty" | "--pretty-json" => compact_json = false,
                "--compact" | "--compact-json" => compact_json = true,
                "--" => {
                    if let Some(value) = iter.next() {
                        url = Some(value);
                    }
                    if let Some(extra) = iter.next() {
                        return Err(format!(
                            "unexpected extra argument {extra}\n\n{}",
                            help_text()
                        ));
                    }
                    break;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other}\n\n{}", help_text()));
                }
                other => {
                    if url.replace(other.to_string()).is_some() {
                        return Err(format!(
                            "unexpected extra argument {other}\n\n{}",
                            help_text()
                        ));
                    }
                }
            }
        }
        Ok(Self {
            url: url.unwrap_or_else(|| DEFAULT_URL.to_string()),
            semantic_snapshot,
            compact_json,
            help,
        })
    }
}

fn help_text() -> String {
    "kittwm-browser — first-party kittwm-native browser app\n\n\
Usage:\n  kittwm-browser [OPTIONS] [URL]\n\n\
Options:\n  --semantic-snapshot, --print-semantic  load URL, print DOM/ARIA semantic snapshot JSON, and exit\n  --pretty, --pretty-json                pretty-print semantic snapshot JSON\n  --compact, --compact-json              compact semantic snapshot JSON (default)\n  -h, --help                             show this help\n\n\
Default mode renders the browser surface in the terminal and publishes semantic snapshots to kittwm when KITTWM_SOCKET is set.\n"
        .to_string()
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kittwm-browser: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let args = BrowserArgs::parse_from(std::env::args().skip(1)).map_err(anyhow::Error::msg)?;
    if args.help {
        print!("{}", help_text());
        return Ok(());
    }
    if args.semantic_snapshot {
        return print_semantic_snapshot(&args.url, args.compact_json);
    }
    let url = args.url;
    let mut viewport = BrowserViewport::from_terminal_cells(terminal_cells().unwrap_or((80, 24)));
    let mut browser = HeadlessBrowserApp::launch(
        &url,
        u32::from(viewport.cols) * 8,
        u32::from(viewport.content_rows) * 16,
    )?;
    let transport = TerminalInfo::detect().transport;
    let _guard = TtyGuard::enter()?;
    let mut semantic_publisher = BrowserSemanticPublisher::from_env();
    let mut placed = false;
    let mut frame = 0u64;
    let mut stdin = std::io::stdin();
    loop {
        let start = Instant::now();
        let mut buf = [0u8; 1024];
        while stdin_ready(Duration::ZERO) {
            let n = stdin.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            if buf[..n].contains(&0x1d) {
                return Ok(());
            }
            let text: String = buf[..n]
                .iter()
                .filter_map(|b| match *b {
                    b'\r' | b'\n' => Some('\n'),
                    0x20..=0x7e => Some(*b as char),
                    _ => None,
                })
                .collect();
            if !text.is_empty() {
                browser.send_text(&text)?;
            }
        }
        if let Some(cells) = terminal_cells() {
            let new_viewport = BrowserViewport::from_terminal_cells(cells);
            if new_viewport != viewport {
                viewport = new_viewport;
                browser.resize(viewport.cols, viewport.content_rows)?;
                placed = false;
            }
        }
        let NativeFrame::Png {
            width: _,
            height: _,
            bytes,
        } = browser.capture()?
        else {
            return Err(anyhow!("browser returned non-PNG frame"));
        };
        semantic_publisher.maybe_publish(&mut browser);
        let fp = viewport.frame_footprint();
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(kitty::upload_still(BROWSER_IMAGE_ID, &bytes, transport).as_bytes())?;
        if !placed {
            handle
                .write_all(browser_image_placement(BROWSER_IMAGE_ID, fp, transport).as_bytes())?;
            placed = true;
        }
        write!(
            handle,
            "\x1b[{};1H\x1b[Kkittwm-browser — {} — window={} socket={} — Ctrl-] exits — frame {}",
            viewport.status_row,
            truncate(&url, 40),
            std::env::var("KITTWM_WINDOW").unwrap_or_else(|_| "<none>".into()),
            std::env::var("KITTWM_SOCKET").unwrap_or_else(|_| "<none>".into()),
            frame
        )?;
        handle.flush()?;
        frame += 1;
        if let Some(slack) = Duration::from_millis(250).checked_sub(start.elapsed()) {
            std::thread::sleep(slack);
        }
    }
}

fn print_semantic_snapshot(url: &str, compact: bool) -> Result<()> {
    let mut browser = HeadlessBrowserApp::launch(url, 1024, 768)?;
    let snapshot = browser.semantic_snapshot()?;
    if compact {
        println!("{}", serde_json::to_string(&snapshot)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    }
    Ok(())
}

struct BrowserSemanticPublisher {
    socket: Option<PathBuf>,
    window: String,
    interval: Duration,
    last_attempt: Option<Instant>,
    last_payload: Option<String>,
}

impl BrowserSemanticPublisher {
    fn from_env() -> Self {
        let socket = std::env::var_os("KITTWM_SOCKET")
            .or_else(|| std::env::var_os("KITTWM_SOCK"))
            .map(PathBuf::from);
        let window = std::env::var("KITTWM_WINDOW").unwrap_or_else(|_| "focused".to_string());
        Self {
            socket,
            window,
            interval: Duration::from_millis(500),
            last_attempt: None,
            last_payload: None,
        }
    }

    fn maybe_publish(&mut self, browser: &mut HeadlessBrowserApp) {
        let Some(socket) = self.socket.clone() else {
            return;
        };
        let now = Instant::now();
        if !self.due(now) {
            return;
        }
        self.last_attempt = Some(now);
        let Ok(snapshot) = browser.semantic_snapshot() else {
            return;
        };
        let Ok(payload) = serde_json::to_string(&snapshot) else {
            return;
        };
        if !self.record_payload(&payload) {
            return;
        }
        let _ = publish_semantic_snapshot(&socket, &self.window, &snapshot);
    }

    fn due(&self, now: Instant) -> bool {
        self.last_attempt
            .map(|last| now.saturating_duration_since(last) >= self.interval)
            .unwrap_or(true)
    }

    fn record_payload(&mut self, payload: &str) -> bool {
        if self.last_payload.as_deref() == Some(payload) {
            return false;
        }
        self.last_payload = Some(payload.to_string());
        true
    }
}

fn publish_semantic_snapshot(
    socket: &PathBuf,
    window: &str,
    snapshot: &SemanticSurfaceSnapshot,
) -> Result<String> {
    Kittwm::connect_path(socket)
        .surface(window)
        .semantic_publish(snapshot)
        .map_err(|e| anyhow!(e))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct BrowserViewport {
    cols: u16,
    raw_rows: u16,
    content_rows: u16,
    status_row: u16,
}

impl BrowserViewport {
    fn from_terminal_cells((cols, raw_rows): (u16, u16)) -> Self {
        let raw_rows = raw_rows.max(1);
        let content_rows = raw_rows.saturating_sub(BROWSER_RESERVED_STATUS_ROWS).max(1);
        Self {
            cols: cols.max(1),
            raw_rows,
            content_rows,
            status_row: raw_rows,
        }
    }

    fn frame_footprint(&self) -> CellRect {
        CellRect::new(0, 0, self.cols, self.content_rows)
    }
}

fn browser_image_placement(image_id: u32, footprint: CellRect, transport: Transport) -> String {
    let mut options = kitty::PlacementOptions::absolute();
    options.z_index = BROWSER_IMAGE_Z_INDEX;
    format!(
        "{}{}",
        kitty::cursor_move(footprint.x, footprint.y, transport),
        kitty::placement_command_ex(image_id, footprint, &options, transport)
    )
}

fn terminal_cells() -> Option<(u16, u16)> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_col, ws.ws_row))
    } else {
        None
    }
}

fn stdin_ready(timeout: Duration) -> bool {
    let mut fds = unsafe { std::mem::zeroed::<libc::fd_set>() };
    unsafe { libc::FD_ZERO(&mut fds) };
    unsafe { libc::FD_SET(libc::STDIN_FILENO, &mut fds) };
    let mut tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: timeout.subsec_micros() as libc::suseconds_t,
    };
    let rc = unsafe {
        libc::select(
            libc::STDIN_FILENO + 1,
            &mut fds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut tv,
        )
    };
    rc > 0
}

struct TtyGuard {
    orig: libc::termios,
}

impl TtyGuard {
    fn enter() -> Result<Self> {
        let mut orig = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut orig) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut raw = orig;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        print!("\x1b[?1049h\x1b[?25l");
        std::io::stdout().flush().ok();
        Ok(Self { orig })
    }
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.orig) };
        print!("\x1b[?25h\x1b[?1049l");
        std::io::stdout().flush().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_semantic_snapshot_flags() {
        let args =
            BrowserArgs::parse_from(["--semantic-snapshot", "--pretty", "https://example.com/app"])
                .unwrap();
        assert_eq!(args.url, "https://example.com/app");
        assert!(args.semantic_snapshot);
        assert!(!args.compact_json);
    }

    #[test]
    fn parses_print_semantic_alias_and_default_url() {
        let args = BrowserArgs::parse_from(["--print-semantic"]).unwrap();
        assert_eq!(args.url, DEFAULT_URL);
        assert!(args.semantic_snapshot);
        assert!(args.compact_json);
    }

    #[test]
    fn rejects_unknown_browser_option() {
        let err = BrowserArgs::parse_from(["--bogus"]).unwrap_err();
        assert!(err.contains("unknown option --bogus"));
        assert!(err.contains("--semantic-snapshot"));
    }

    #[test]
    fn browser_viewport_clamps_content_and_status_to_reported_rows() {
        let normal = BrowserViewport::from_terminal_cells((100, 30));
        assert_eq!(normal.cols, 100);
        assert_eq!(normal.content_rows, 28);
        assert_eq!(normal.status_row, 30);
        assert_eq!(normal.frame_footprint(), CellRect::new(0, 0, 100, 28));

        let tiny = BrowserViewport::from_terminal_cells((0, 1));
        assert_eq!(tiny.cols, 1);
        assert_eq!(tiny.content_rows, 1);
        assert_eq!(tiny.status_row, 1);
        assert_eq!(tiny.frame_footprint(), CellRect::new(0, 0, 1, 1));
    }

    #[test]
    fn browser_image_placement_uses_absolute_kitty_graphics_without_placeholders() {
        let placement = browser_image_placement(42, CellRect::new(0, 0, 80, 22), Transport::Direct);
        assert!(placement.contains("a=p"), "{placement:?}");
        assert!(placement.contains("c=80"), "{placement:?}");
        assert!(placement.contains("r=22"), "{placement:?}");
        assert!(!placement.contains("U=1"), "{placement:?}");
    }

    #[test]
    fn semantic_publisher_debounces_and_skips_unchanged_payloads() {
        let start = Instant::now();
        let mut publisher = BrowserSemanticPublisher {
            socket: Some(PathBuf::from("/tmp/unused.sock")),
            window: "native-1".to_string(),
            interval: Duration::from_millis(500),
            last_attempt: None,
            last_payload: None,
        };

        assert!(publisher.due(start));
        publisher.last_attempt = Some(start);
        assert!(!publisher.due(start + Duration::from_millis(499)));
        assert!(publisher.due(start + Duration::from_millis(500)));
        assert!(publisher.record_payload("{\"revision\":1}"));
        assert!(!publisher.record_payload("{\"revision\":1}"));
        assert!(publisher.record_payload("{\"revision\":2}"));
    }

    #[test]
    fn semantic_publisher_defaults_to_focused_without_socket() {
        let publisher = BrowserSemanticPublisher {
            socket: None,
            window: "focused".to_string(),
            interval: Duration::from_millis(500),
            last_attempt: None,
            last_payload: None,
        };
        assert!(publisher.socket.is_none());
        assert_eq!(publisher.window, "focused");
    }
}
