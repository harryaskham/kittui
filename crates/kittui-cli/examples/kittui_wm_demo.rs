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
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use kittui::{CellSize, Runtime, TerminalInfo};
use kittui_core::geom::PxRect;
use kittui_input::{InputEvent, Key, MouseButton, Modifiers};
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
use kittui_xvfb::{FakeServer, XServer, XWindowId};

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
        let server = kittui_quartz::QuartzServer::spawn(width, height)
            .map_err(|e| anyhow::anyhow!("QuartzServer::spawn failed: {e}"))?;
        let compositor = Compositor::new(server, cell);
        let layout = Layout::all_floating();
        return run_loop(&runtime, &compositor, &layout);
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

fn run_loop<S: XServer>(
    runtime: &Runtime,
    compositor: &Compositor<S>,
    layout: &Layout,
) -> Result<()> {
    // Enable raw mode + SGR mouse reporting so we can stream pointer events.
    let _raw_guard = RawMode::enter()?;

    let frame_target = Duration::from_millis(33);
    let mut frame = 0u64;
    let mut input_buf = Vec::<u8>::with_capacity(256);
    let mut stdin = io::stdin();

    loop {
        let frame_start = Instant::now();

        // Drive frame.
        let scenes = compositor
            .compose_with_layout(layout)
            .map_err(|e| anyhow::anyhow!("compose: {e}"))?;
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        write!(handle, "\x1b[2J\x1b[H")?;
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
        write!(handle, "\x1b[{};1H", scenes.iter().map(|s| s.footprint.y + s.footprint.rows + 2).max().unwrap_or(2))?;
        write!(
            handle,
            "kittui-wm frame {} — {} windows — q to quit",
            frame,
            scenes.len()
        )?;
        handle.flush()?;
        drop(handle);

        // Pump stdin without blocking and dispatch events.
        let elapsed = frame_start.elapsed();
        let remaining = frame_target.checked_sub(elapsed).unwrap_or_default();
        if remaining > Duration::ZERO {
            // Best-effort non-blocking read using a short timeout via select().
            let mut chunk = [0u8; 64];
            if poll_stdin(remaining) {
                let n = stdin.read(&mut chunk).unwrap_or(0);
                input_buf.extend_from_slice(&chunk[..n]);
            }
        }

        while let Some((ev, consumed)) = kittui_input::parse(&input_buf) {
            input_buf.drain(..consumed);
            match &ev {
                InputEvent::Char { ch: 'q', .. }
                | InputEvent::Key {
                    key: Key::Escape, ..
                } => return Ok(()),
                InputEvent::MousePress { .. }
                | InputEvent::MouseRelease { .. }
                | InputEvent::MouseMove { .. } => {
                    let _ = compositor.route_pointer(&ev);
                }
                InputEvent::Char { .. } | InputEvent::Key { .. } => {
                    let _ = compositor.route_key(&ev);
                }
                _ => {}
            }
        }

        frame = frame.wrapping_add(1);
    }
}

// --- raw mode + SGR mouse reporting guard ----------------------------------

struct RawMode;

impl RawMode {
    fn enter() -> Result<Self> {
        // Enable SGR mouse + motion + focus reporting.
        let mut out = io::stdout();
        out.write_all(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h")?;
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
        }
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = out
            .write_all(b"\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
        let _ = out.flush();
        #[cfg(unix)]
        unsafe {
            use libc::*;
            if let Some(t) = ORIG_TERM.take() {
                tcsetattr(STDIN_FILENO, TCSANOW, &t);
            }
        }
    }
}

#[cfg(unix)]
static mut ORIG_TERM: Option<libc::termios> = None;

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
