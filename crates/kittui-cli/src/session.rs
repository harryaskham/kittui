//! kittui-wm session loop.
//!
//! Owns the full UI thread for a kittui-wm viewer:
//!
//! - terminal raw mode + alt screen + SGR mouse reporting (RAII-restored
//!   even on SIGINT/HUP/QUIT/TERM)
//! - stdin draining + `kittui_input` parsing
//! - `MultiCompositor`-friendly compose + place pipeline (currently
//!   wired against the single-backend `Compositor` for back-compat;
//!   migrates when the Pump architecture lands)
//! - file-based debug log so the agent and the operator can introspect
//!   without escapes interfering with the live render
//!
//! Both the `kittui_wm_demo` example and the `kitwm` binary call into
//! [`run_loop`].

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use anyhow::Result;

use kittui::Runtime;
use kittui_input::{InputEvent, Key};
use kittui_wm::compositor::{Compositor, Layout};
use kittui_xvfb::XServer;

/// Drive the kittui-wm UI loop until the operator quits.
///
/// `compositor` and `layout` are passed in so callers can wire any
/// `XServer` backend (FakeServer, Xvfb, Quartz, XQuartz, ...) without
/// this module knowing about backends.
pub fn run_loop<S: XServer>(
    runtime: &Runtime,
    compositor: &Compositor<S>,
    layout: &Layout,
) -> Result<()> {
    let dbg = Debugger::open();
    dbg.log("run_loop: enter");
    let _raw_guard = RawMode::enter()?;
    dbg.log("raw mode + alt screen entered");
    install_signal_restore();

    let frame_target = Duration::from_millis(33);
    let mut frame = 0u64;
    let mut input_buf = Vec::<u8>::with_capacity(256);
    let mut stdin = io::stdin();
    let mut last_window_count = 0usize;

    loop {
        let frame_start = Instant::now();

        // Drain any pending stdin BEFORE the expensive compose, so q/Esc
        // takes effect even when a single frame is slow.
        let mut chunk = [0u8; 512];
        while poll_stdin(Duration::ZERO) {
            let n = stdin.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            input_buf.extend_from_slice(&chunk[..n]);
        }
        let mut quit = false;
        while let Some((ev, consumed)) = kittui_input::parse(&input_buf) {
            input_buf.drain(..consumed);
            match &ev {
                InputEvent::Char { ch: 'q', .. }
                | InputEvent::Key {
                    key: Key::Escape, ..
                } => {
                    dbg.log("quit event received during pre-compose drain");
                    quit = true;
                    break;
                }
                InputEvent::MousePress { .. }
                | InputEvent::MouseRelease { .. }
                | InputEvent::MouseMove { .. } => {
                    let _ = compositor.route_pointer(&ev);
                }
                InputEvent::Char { ch, .. } => {
                    dbg.log(&format!("char event: {:?}", ch));
                    let _ = compositor.route_key(&ev);
                }
                InputEvent::Key { key, .. } => {
                    dbg.log(&format!("key event: {:?}", key));
                    let _ = compositor.route_key(&ev);
                }
                _ => {}
            }
            if consumed == 0 {
                break;
            }
        }
        if quit {
            return Ok(());
        }

        // Drive frame. Use the raw RGBA fast path so PNG encode falls
        // out of the per-frame cost. Errors are surfaced inside the chrome
        // footer instead of bailing, so a TCC failure or backend death
        // never leaks the terminal.
        match compositor.raw_frames(layout) {
            Ok(frames) => {
                last_window_count = frames.len();
                if frame % 30 == 0 {
                    dbg.log(&format!(
                        "frame {frame}: {} raw frames",
                        frames.len()
                    ));
                }
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                write!(handle, "\x1b[H\x1b[J")?;
                let mut footer_row = 2u16;
                for f in &frames {
                    let p = runtime.place_raw_frame(
                        f.image_id,
                        &f.rgba,
                        f.width,
                        f.height,
                        f.footprint,
                    );
                    handle.write_all(p.upload.as_bytes())?;
                    write!(
                        handle,
                        "\x1b[{};{}H",
                        f.footprint.y + 1,
                        f.footprint.x + 1
                    )?;
                    handle.write_all(p.placement.as_bytes())?;
                    handle.write_all(p.embed.as_bytes())?;
                    footer_row = footer_row.max(f.footprint.y + f.footprint.rows + 2);
                }
                write!(
                    handle,
                    "\x1b[{};1H\x1b[Kkittui-wm frame {} — {} windows — q to quit (log: {})",
                    footer_row,
                    frame,
                    last_window_count,
                    dbg.path_display()
                )?;
                handle.flush()?;
            }
            Err(e) => {
                let msg = e.to_string();
                dbg.log(&format!("compose err: {msg}"));
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                write!(
                    handle,
                    "\x1b[H\x1b[J\x1b[1mkittui-wm error\x1b[0m\n\n  {}\n\n  q/Esc to quit. On macOS, grant Screen Recording + Accessibility.\n  (log: {})\n",
                    msg,
                    dbg.path_display()
                )?;
                handle.flush()?;
            }
        }

        let elapsed = frame_start.elapsed();
        let remaining = frame_target.checked_sub(elapsed).unwrap_or_default();
        if remaining > Duration::ZERO {
            let mut chunk = [0u8; 512];
            if poll_stdin(remaining) {
                let n = stdin.read(&mut chunk).unwrap_or(0);
                if n > 0 {
                    dbg.log(&format!(
                        "stdin read {n} bytes: {:02x?}",
                        &chunk[..n.min(32)]
                    ));
                    input_buf.extend_from_slice(&chunk[..n]);
                }
            }
        } else {
            dbg.log(&format!(
                "frame {frame} budget blown: {} ms (target {} ms)",
                elapsed.as_millis(),
                frame_target.as_millis()
            ));
        }

        frame = frame.wrapping_add(1);
    }
}

/// Append-only log for the kittui-wm session. Stderr is invisible inside
/// the alt screen, so we mirror everything to a file at $KITTUI_WM_LOG
/// (default `/tmp/kittui-wm.log`).
pub struct Debugger {
    file: std::sync::Mutex<Option<std::fs::File>>,
    path: String,
}

impl Debugger {
    /// Open the log file (truncating on each session).
    pub fn open() -> Self {
        let path = std::env::var("KITTUI_WM_LOG")
            .unwrap_or_else(|_| "/tmp/kittui-wm.log".to_string());
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .ok();
        if let Some(mut f) = file.as_ref() {
            use std::io::Write;
            let _ = writeln!(
                f,
                "kittui-wm log {} (pid {})",
                clock(),
                std::process::id()
            );
        }
        Self {
            file: std::sync::Mutex::new(file),
            path,
        }
    }

    /// Append a single log line.
    pub fn log(&self, line: &str) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {}", clock(), line);
                let _ = f.flush();
            }
        }
    }

    /// Path the log was opened at.
    pub fn path_display(&self) -> &str {
        &self.path
    }
}

fn clock() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

// --- raw mode + SGR mouse reporting guard ----------------------------------

struct RawMode;

impl RawMode {
    fn enter() -> Result<Self> {
        let mut out = io::stdout();
        // Alt screen + hide cursor, then SGR mouse + motion + focus reporting.
        out.write_all(
            b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h",
        )?;
        out.flush()?;
        #[cfg(unix)]
        unsafe {
            use libc::*;
            let mut term: termios = std::mem::zeroed();
            tcgetattr(STDIN_FILENO, &mut term);
            let mut raw = term;
            raw.c_lflag &= !(ICANON | ECHO);
            raw.c_cc[VMIN] = 0;
            raw.c_cc[VTIME] = 0;
            tcsetattr(STDIN_FILENO, TCSANOW, &raw);
            ORIG_TERM = Some(term);
            RAW_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    #[cfg(unix)]
    {
        use std::sync::atomic::Ordering;
        if !RAW_ACTIVE.swap(false, Ordering::SeqCst) {
            return;
        }
    }
    let mut out = io::stdout();
    let _ = out.write_all(
        b"\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l",
    );
    let _ = out.flush();
    #[cfg(unix)]
    unsafe {
        use libc::*;
        let orig = std::ptr::addr_of_mut!(ORIG_TERM);
        if let Some(t) = (*orig).take() {
            tcsetattr(STDIN_FILENO, TCSANOW, &t);
        }
    }
}

static RAW_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
static mut ORIG_TERM: Option<libc::termios> = None;

#[cfg(unix)]
fn install_signal_restore() {
    use libc::*;
    extern "C" fn handler(_sig: i32) {
        restore_terminal();
        std::process::exit(130);
    }
    unsafe {
        for sig in [SIGINT, SIGTERM, SIGHUP, SIGQUIT] {
            signal(sig, handler as *const () as sighandler_t);
        }
    }
}

#[cfg(not(unix))]
fn install_signal_restore() {}

#[cfg(unix)]
fn poll_stdin(timeout: Duration) -> bool {
    use libc::*;
    unsafe {
        let mut fds = pollfd {
            fd: STDIN_FILENO,
            events: POLLIN,
            revents: 0,
        };
        let ms: i32 = timeout.as_millis().min(i32::MAX as u128) as i32;
        let n = poll(&mut fds, 1, ms);
        n > 0 && fds.revents & POLLIN != 0
    }
}

#[cfg(not(unix))]
fn poll_stdin(timeout: Duration) -> bool {
    std::thread::sleep(timeout);
    false
}
