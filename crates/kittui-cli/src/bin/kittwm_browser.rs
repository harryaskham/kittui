//! `kittwm-browser` — first-class kittwm-native browser app.
//!
//! This binary is intentionally WM-context aware: when launched inside a
//! kittwm native terminal it inherits `KITTWM_SOCKET` / `KITTWM_WINDOW`, just
//! like an X app inherits `DISPLAY`. The current implementation is a direct
//! full-terminal replacement app; the socket spawn/replace protocol will make
//! the same binary ask a live kittwm host to create or replace panes.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use kittui::{CellRect, TerminalInfo};
use kittui_kitty as kitty;
use kittui_wm::native::{HeadlessBrowserApp, NativeApp, NativeFrame};

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
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| {
        "data:text/html,<html><body><h1>kittwm-browser</h1><input autofocus value='ready'></body></html>".to_string()
    });
    let (mut cols, mut rows) = terminal_cells().unwrap_or((80, 24));
    rows = rows.saturating_sub(2).max(1);
    let mut browser = HeadlessBrowserApp::launch(&url, u32::from(cols) * 8, u32::from(rows) * 16)?;
    let transport = TerminalInfo::detect().transport;
    let _guard = TtyGuard::enter()?;
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
        if let Some((new_cols, new_rows_raw)) = terminal_cells() {
            let new_rows = new_rows_raw.saturating_sub(2).max(1);
            if (new_cols, new_rows) != (cols, rows) {
                cols = new_cols;
                rows = new_rows;
                browser.resize(cols, rows)?;
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
        let fp = CellRect::new(0, 0, cols, rows);
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(kitty::upload_still(1, &bytes, transport).as_bytes())?;
        if !placed {
            handle.write_all(kitty::cursor_move(0, 0, transport).as_bytes())?;
            handle.write_all(kitty::placement_command(1, fp, transport).as_bytes())?;
            handle.write_all(kitty::placeholder_text(1, fp).as_bytes())?;
            placed = true;
        }
        write!(
            handle,
            "\x1b[{};1H\x1b[Kkittwm-browser — {} — window={} socket={} — Ctrl-] exits — frame {}",
            rows + 2,
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
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
