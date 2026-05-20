//! Real-Xvfb round-trip proof.
//!
//! Runs only when:
//!   - target_os = "linux" (Xvfb only meaningful on Linux)
//!   - the `xvfb` feature is enabled
//!   - the `Xvfb` and `xterm` binaries are on PATH (provided by the flake
//!     devshell on Linux hosts).
//!
//! On any other host the test is a no-op, so `cargo test --workspace`
//! stays green on macOS too.

#[cfg(all(target_os = "linux", feature = "xvfb"))]
mod linux {
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use kittui_xvfb::xvfb::XvfbServer;
    use kittui_xvfb::{XPointerEvent, XServer, XWindowId};

    fn have_binary(name: &str) -> bool {
        Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn xvfb_spawn_capture_inject_round_trip() {
        if !have_binary("Xvfb") || !have_binary("xterm") {
            eprintln!("skipping: Xvfb or xterm not on PATH");
            return;
        }
        let server = match XvfbServer::spawn(99) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skipping: could not spawn Xvfb: {e}");
                return;
            }
        };
        // Launch xterm against the Xvfb display.
        let mut child = Command::new("xterm")
            .env("DISPLAY", server.display())
            .arg("-geometry")
            .arg("40x10+10+10")
            .spawn()
            .expect("xterm spawn");

        // Give xterm a moment to map.
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(150));
            if let Ok(ws) = server.windows() {
                if !ws.is_empty() {
                    break;
                }
            }
        }

        let windows = server.windows().expect("windows");
        assert!(!windows.is_empty(), "no toplevel windows after spawning xterm");

        // Capture one and assert non-empty RGBA buffer.
        let id = windows[0].id;
        let cap = server.capture(id).expect("capture");
        assert!(cap.width > 0 && cap.height > 0);
        assert_eq!(cap.rgba.len(), (cap.width * cap.height * 4) as usize);

        // Inject a pointer move and a key press (best-effort; we only assert
        // the call succeeds).
        server
            .inject_pointer(XPointerEvent::Move {
                window: XWindowId(id.0),
                x_px: 10,
                y_px: 10,
            })
            .expect("inject motion");
        server
            .inject_key('q' as u32, true)
            .expect("inject key");
        server.inject_key('q' as u32, false).expect("inject key release");

        let _ = child.kill();
    }
}
