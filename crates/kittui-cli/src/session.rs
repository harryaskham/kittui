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
    let opts = RunOptions {
        fps: std::env::var("KITTUI_WM_FPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
        launch_on_f12: std::env::var("KITTUI_WM_LAUNCH_ON_F12")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
    };
    run_loop_with(runtime, compositor, layout, opts)
}

/// Tunable runtime options for the kitwm session loop.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Target frames per second. Capped at 240 to keep terminal output sane.
    pub fps: u32,
    /// If true, intercept F12 and spawn the launcher command instead of
    /// forwarding it to the focused backend window.
    pub launch_on_f12: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self { fps: 60, launch_on_f12: false }
    }
}

pub fn run_loop_with<S: XServer>(
    runtime: &Runtime,
    compositor: &Compositor<S>,
    layout: &Layout,
    opts: RunOptions,
) -> Result<()> {
    let dbg = Debugger::open();
    dbg.log(&format!(
        "run_loop: enter fps={} launch_on_f12={}",
        opts.fps, opts.launch_on_f12
    ));
    let _raw_guard = RawMode::enter()?;
    dbg.log("raw mode + alt screen entered");
    install_signal_restore();

    let fps = opts.fps.clamp(1, 240);
    let frame_target = Duration::from_micros(1_000_000 / fps as u64);
    // Live fps tracking: instantaneous over the last 30 frames + peak.
    let mut fps_window_start = std::time::Instant::now();
    let mut fps_window_frames = 0u32;
    let mut live_fps: f32 = 0.0;
    let mut peak_fps: f32 = 0.0;
    let mut frame = 0u64;
    let mut input_buf = Vec::<u8>::with_capacity(256);
    let mut stdin = io::stdin();
    let mut last_window_count = 0usize;
    let mut last_launch_pid: Option<u32> = None;
    // Per-window placement memo: (image_id, footprint) -> placement+embed.
    // We only re-emit placement+placeholder when the footprint or image_id
    // changes. Kitty atomically replaces the image at the same id on each
    // raw RGBA upload, so the picture updates without us clearing the screen.
    let mut last_placed: std::collections::HashMap<u32, (kittui::CellRect, String, String)> =
        std::collections::HashMap::new();
    // Set of window image-ids seen on the previous frame so we can delete
    // ones that disappear without redrawing the whole screen.
    let mut prev_window_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();

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
                InputEvent::Key { key: Key::F(12), .. } if opts.launch_on_f12 => {
                    match spawn_launcher_command() {
                        Ok(pid) => {
                            last_launch_pid = Some(pid);
                            dbg.log(&format!("launcher F12 spawned pid={pid}"));
                        }
                        Err(e) => {
                            dbg.log(&format!("launcher F12 failed: {e}"));
                        }
                    }
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
                // Track which windows are present this frame so we can
                // delete the ones that have disappeared.
                let mut current_ids: std::collections::HashSet<u32> =
                    std::collections::HashSet::with_capacity(frames.len());
                let mut footer_row = 2u16;
                for f in &frames {
                    current_ids.insert(f.image_id);
                    // Detect whether placement (footprint) changed since
                    // last frame for this id. If so, emit a kitty
                    // 'delete by id' first so the placeholder grid is
                    // cleared from its old cells, then place fresh.
                    let footprint_changed = last_placed
                        .get(&f.image_id)
                        .map(|(prev_fp, _, _)| prev_fp != &f.footprint)
                        .unwrap_or(true);
                    if footprint_changed {
                        if last_placed.contains_key(&f.image_id) {
                            handle.write_all(
                                runtime.unplace(f.image_id).as_bytes(),
                            )?;
                        }
                    }
                    let p = runtime.place_raw_frame(
                        f.image_id,
                        &f.rgba,
                        f.width,
                        f.height,
                        f.footprint,
                    );
                    // Always re-upload (kitty atomically replaces the
                    // image at the same id; no flicker because no clear).
                    handle.write_all(p.upload.as_bytes())?;
                    if footprint_changed {
                        write!(
                            handle,
                            "\x1b[{};{}H",
                            f.footprint.y + 1,
                            f.footprint.x + 1
                        )?;
                        handle.write_all(p.placement.as_bytes())?;
                        handle.write_all(p.embed.as_bytes())?;
                        last_placed.insert(
                            f.image_id,
                            (
                                f.footprint,
                                p.placement.clone(),
                                p.embed.clone(),
                            ),
                        );
                    }
                    footer_row = footer_row.max(f.footprint.y + f.footprint.rows + 2);
                }
                // Delete any window that disappeared since last frame.
                for old_id in prev_window_ids.difference(&current_ids) {
                    handle.write_all(runtime.unplace(*old_id).as_bytes())?;
                    last_placed.remove(old_id);
                }
                prev_window_ids = current_ids;
                let launch_note = last_launch_pid
                    .map(|pid| format!(" — last launch pid={pid}"))
                    .unwrap_or_default();
                write!(
                    handle,
                    "\x1b[{};1H\x1b[Kkittui-wm frame {} — {} windows — {:.0} fps (peak {:.0}, cap {}){} — q to quit (log: {})",
                    footer_row,
                    frame,
                    last_window_count,
                    live_fps,
                    peak_fps,
                    fps,
                    launch_note,
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
            // Brief stdin poll with a 1ms cap so even on a fd that returns
            // ready immediately we don't spin. Skip entirely when the
            // frame budget is already small.
            let poll_budget = remaining.min(Duration::from_millis(1));
            if poll_budget >= Duration::from_micros(500) && poll_stdin(poll_budget) {
                let n = stdin.read(&mut chunk).unwrap_or(0);
                if n > 0 {
                    dbg.log(&format!(
                        "stdin read {n} bytes: {:02x?}",
                        &chunk[..n.min(32)]
                    ));
                    input_buf.extend_from_slice(&chunk[..n]);
                }
            }
            // Sleep out the rest of the frame budget so --fps actually caps,
            // but never longer than ~16ms at a stretch (preserves Ctrl-C
            // responsiveness on a backgrounded stdin).
            loop {
                let used = frame_start.elapsed();
                let Some(slack) = frame_target.checked_sub(used) else { break };
                if slack < Duration::from_micros(500) { break; }
                std::thread::sleep(slack);
            }
        } else {
            dbg.log(&format!(
                "frame {frame} budget blown: {} ms (target {} ms)",
                elapsed.as_millis(),
                frame_target.as_millis()
            ));
        }

        frame = frame.wrapping_add(1);
        fps_window_frames += 1;
        let elapsed_w = fps_window_start.elapsed();
        if fps_window_frames >= 30 || elapsed_w >= Duration::from_millis(500) {
            let secs = elapsed_w.as_secs_f32().max(1e-6);
            live_fps = fps_window_frames as f32 / secs;
            if live_fps > peak_fps {
                peak_fps = live_fps;
            }
            fps_window_frames = 0;
            fps_window_start = std::time::Instant::now();
        }
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

/// Return the shell command used by the F12 launcher.
///
/// Defaults to `xterm`; set `KITWM_LAUNCH_CMD` to override, e.g.
/// `KITWM_LAUNCH_CMD='open -a Terminal'` or `'/bin/sleep 10'`.
pub fn launcher_command() -> String {
    std::env::var("KITWM_LAUNCH_CMD").unwrap_or_else(|_| "xterm".to_string())
}

fn spawn_launcher_command() -> Result<u32> {
    let cmd = launcher_command();
    let child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(child.id())
}

#[cfg(test)]
mod launcher_tests {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn launcher_command_defaults_to_xterm() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KITWM_LAUNCH_CMD");
        assert_eq!(super::launcher_command(), "xterm");
    }

    #[test]
    fn launcher_command_honors_env_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITWM_LAUNCH_CMD", "/bin/sleep 1");
        assert_eq!(super::launcher_command(), "/bin/sleep 1");
        std::env::remove_var("KITWM_LAUNCH_CMD");
    }
}
