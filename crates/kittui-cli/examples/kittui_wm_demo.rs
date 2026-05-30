//! kittui-wm v1 demo — finished feature.
//!
//! Build either way:
//!
//! ```sh
//! # macOS / any host: drives the FakeServer (two solid-color windows).
//! cargo run --release -p kittui-cli --example kittui_wm_demo
//!
//! # Linux + flake devshell: spawns Xvfb, runs a real X app, composites it
//! # into the kitty terminal, and routes pointer/key events back.
//! nix develop
//! cargo run --release -p kittui-cli --example kittui_wm_demo --features xvfb
//! ```
//!
//! The demo runs a 30-fps composite loop. Press `q` to quit. While the
//! `xvfb` feature is on, kittui-wm forwards every kitty SGR mouse event and
//! every printable keystroke into the live X server via XTestFake*, so a
//! click on a chrome window actually clicks the underlying X app and a
//! keystroke actually types into it.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use anyhow::Result;

use kittui::{CellSize, Runtime, TerminalInfo};
use kittui_input::{InputEvent, Key};
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
use kittui_xvfb::XServer;

#[cfg(feature = "xvfb")]
mod live {
    use super::*;
    use kittui_xvfb::xvfb::XvfbServer;
    use std::process::{Child, Command};

    pub struct LiveSession {
        pub server: XvfbServer,
        pub _app: Option<Child>,
    }

    impl LiveSession {
        pub fn spawn(display: u32, app: Option<&[&str]>) -> Result<Self> {
            let server = XvfbServer::spawn(display)
                .map_err(|e| anyhow::anyhow!("Xvfb spawn failed: {e}"))?;
            let _app = if let Some(argv) = app {
                let mut cmd = Command::new(argv[0]);
                cmd.args(&argv[1..]);
                cmd.env("DISPLAY", server.display());
                let child = cmd
                    .spawn()
                    .map_err(|e| anyhow::anyhow!("spawn {:?}: {e}", argv))?;
                Some(child)
            } else {
                None
            };
            // Give the X app a moment to map.
            std::thread::sleep(std::time::Duration::from_millis(400));
            Ok(Self { server, _app })
        }
    }
}

fn main() -> Result<()> {
    let cell = CellSize::default();
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;

    // Backend precedence: --features quartz wins on macOS; --features xvfb
    // wins on Linux; otherwise the FakeServer keeps everything portable.
    #[cfg(all(target_os = "macos", feature = "quartz"))]
    {
        let width: u32 = std::env::var("KITTUI_WM_WIDTH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1280);
        let height: u32 = std::env::var("KITTUI_WM_HEIGHT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(800);
        let mut server = kittui_quartz::QuartzServer::spawn(width, height)
            .map_err(|e| anyhow::anyhow!("QuartzServer::spawn failed: {e}"))?;
        // Downscale on the GPU side so the kittui PNG encode doesn't have to
        // chew through 2880x1800 every frame. The terminal only needs a few
        // hundred pixels for 80x24 cells.
        let cap_w = (80u32) * cell.width_px as u32 * 2;
        let cap_h = (24u32) * cell.height_px as u32 * 2;
        server.set_max_size(Some((cap_w, cap_h)));

        // Probe TCC permissions before flipping the terminal into raw mode.
        // CGDisplayCreateImage requires Screen Recording; CGEventPost
        // requires Accessibility for cross-app delivery. We probe both and
        // print plain-English instructions on failure so the operator never
        // sees a leaked terminal.
        eprintln!("kittui-wm: probing macOS permissions...");
        if let Err(e) = server.windows().and_then(|w| {
            if let Some(first) = w.first() {
                server.capture(first.id).map(|_| ())
            } else {
                Err(kittui_xvfb::XError::Unavailable("no displays".into()))
            }
        }) {
            eprintln!(
                "kittui-wm: capture probe failed: {e}\n\n  \
                 macOS may have just prompted you for Screen Recording \
                 permission. Grant it under System Settings → Privacy & \
                 Security → Screen Recording (you may need to quit and \
                 restart the terminal), then re-run this demo.\n"
            );
            return Ok(());
        }
        eprintln!("kittui-wm: permissions OK, entering visual loop. q/Esc to quit.");
        std::thread::sleep(std::time::Duration::from_millis(800));

        let compositor = Compositor::new(server, cell);
        // Pin the captured display to a friendly 80x24 cell footprint so
        // the demo always fits in a reasonable terminal. The X11/Xvfb path
        // doesn't need this because each X window is already cell-sized.
        let mut layout = Layout::all_floating();
        if let Ok(windows) = compositor.server().windows() {
            if let Some(w) = windows.first() {
                layout.tile(
                    w.id,
                    kittui_core::geom::PxRect::new(
                        0.0,
                        0.0,
                        80.0 * cell.width_px as f32,
                        24.0 * cell.height_px as f32,
                    ),
                );
                compositor.set_mode(w.id, WindowMode::Tiled);
            }
        }
        run_loop(&runtime, &compositor, &layout)
    }

    #[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
    {
        let display: u32 = std::env::var("KITTUI_WM_DISPLAY")
            .ok()
            .and_then(|s| s.trim_start_matches(':').parse().ok())
            .unwrap_or(99);
        let app_cmd: Vec<String> = std::env::var("KITTUI_WM_APP")
            .ok()
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_else(|| vec!["xterm".into(), "-geometry".into(), "60x16+20+20".into()]);
        let app_argv: Vec<&str> = app_cmd.iter().map(|s| s.as_str()).collect();
        let session = live::LiveSession::spawn(display, Some(&app_argv))?;
        let compositor = Compositor::new(session.server, cell);
        let layout = Layout::all_floating();
        return run_loop(&runtime, &compositor, &layout);
    }

    #[cfg(not(any(
        all(target_os = "macos", feature = "quartz"),
        feature = "xvfb"
    )))]
    {
        let server = FakeServer::with_windows(vec![
            (
                XWindowId(1),
                PxRect::new(8.0, 16.0, 256.0, 160.0),
                "alpha",
                [0xff, 0x40, 0x40, 0xff],
            ),
            (
                XWindowId(2),
                PxRect::new(320.0, 64.0, 256.0, 160.0),
                "beta",
                [0x40, 0xc0, 0xff, 0xff],
            ),
        ]);
        let compositor = Compositor::new(server, cell);
        let mut layout = Layout::all_floating();
        layout.tile(XWindowId(1), PxRect::new(8.0, 16.0, 320.0, 192.0));
        compositor.set_mode(XWindowId(1), WindowMode::Tiled);
        return run_loop(&runtime, &compositor, &layout);
    }
}

// Demo render loop keeps last_error/last_window_count across loop iterations and
// resets them on success; the reset writes are not always read before the next
// overwrite, which is intentional for this example.
#[allow(unused_assignments)]
fn run_loop<S: XServer>(
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
    let mut last_error: Option<String> = None;
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
                | InputEvent::Key { key: Key::Escape, .. } => {
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

        // Drive frame. Don't bail on compose errors — surface them inside
        // the chrome footer so the user can see the cause (missing TCC
        // permission, X server gone, etc.) without leaking the terminal.
        match compositor.compose_with_layout(layout) {
            Ok(scenes) => {
                last_error = None;
                last_window_count = scenes.len();
                if frame.is_multiple_of(30) {
                    dbg.log(&format!(
                        "frame {frame}: {} scenes, first footprint {:?}",
                        scenes.len(),
                        scenes.first().map(|s| (s.footprint.x, s.footprint.y, s.footprint.cols, s.footprint.rows))
                    ));
                    // Sample a few pixels from the first scene's image layer
                    // to detect 'all-zero' frames.
                    if let Some(scene) = scenes.first() {
                        if let Some(layer) = scene.layers.first() {
                            if let kittui_core::node::Node::Image {
                                src: kittui_core::node::ImageRef::Bytes { bytes },
                                ..
                            } = &layer.root
                            {
                                dbg.log(&format!(
                                    "  image bytes len {}, first 16 bytes {:02x?}",
                                    bytes.len(),
                                    &bytes[..bytes.len().min(16)]
                                ));
                            }
                        }
                    }
                }
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                write!(handle, "\x1b[H\x1b[J")?;
                for scene in &scenes {
                    let p = runtime.place(scene)?;
                    handle.write_all(p.upload.as_bytes())?;
                    write!(
                        handle,
                        "\x1b[{};{}H",
                        scene.footprint.y + 1,
                        scene.footprint.x + 1
                    )?;
                    handle.write_all(p.placement.as_bytes())?;
                    handle.write_all(p.embed.as_bytes())?;
                }
                let footer_row = scenes
                    .iter()
                    .map(|s| s.footprint.y + s.footprint.rows + 2)
                    .max()
                    .unwrap_or(2);
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
                last_error = Some(e.to_string());
                dbg.log(&format!("compose err: {}", last_error.as_deref().unwrap()));
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                write!(
                    handle,
                    "\x1b[H\x1b[J\x1b[1mkittui-wm error\x1b[0m\n\n  {}\n\n  q/Esc to quit. On macOS, grant Screen Recording + Accessibility.\n  (log: {})\n",
                    last_error.as_deref().unwrap_or("unknown"),
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

/// Append-only log for the visual demo. Stderr is invisible inside the
/// alt screen, so we mirror everything to a file at $KITTUI_WM_LOG
/// (default `/tmp/kittui-wm.log`).
struct Debugger {
    file: std::sync::Mutex<Option<std::fs::File>>,
    path: String,
}

impl Debugger {
    fn open() -> Self {
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
                chrono_like_now(),
                std::process::id()
            );
        }
        Self {
            file: std::sync::Mutex::new(file),
            path,
        }
    }

    fn log(&self, line: &str) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {}", chrono_like_now(), line);
                let _ = f.flush();
            }
        }
    }

    fn path_display(&self) -> &str {
        &self.path
    }
}

fn chrono_like_now() -> String {
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
fn poll_stdin(_: Duration) -> bool {
    std::thread::sleep(_);
    false
}
