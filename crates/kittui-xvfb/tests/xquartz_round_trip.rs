//! XQuartz-backed X11 proof harness for macOS.
//!
//! This is intentionally a skip-when-unavailable integration test: CI and
//! many developer Macs do not have XQuartz installed, but when it is present
//! this exercises the same pure-Rust XCB backend against a real XQuartz display
//! and a real `xterm` window.

#[cfg(all(target_os = "macos", feature = "xquartz"))]
mod mac {
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use kittui_xvfb::xquartz::XQuartzServer;
    use kittui_xvfb::XServer;

    #[test]
    fn xquartz_spawn_xterm_capture_round_trip() {
        let Some(xterm) = find_xterm() else {
            eprintln!("skipping: xterm binary not found");
            return;
        };

        let display_num = 70 + (std::process::id() % 1000);
        let display = format!(":{display_num}");
        let server = match XQuartzServer::spawn(display_num) {
            Ok(server) => server,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };

        let mut app = match Command::new(&xterm)
            .env("DISPLAY", server.display())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("skipping: failed to spawn {}: {e}", xterm.display());
                return;
            }
        };

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut captured = false;
        while Instant::now() < deadline {
            if let Ok(windows) = server.windows() {
                for w in windows {
                    if let Ok(cap) = server.capture(w.id) {
                        if cap.width > 0 && cap.height > 0 && !cap.rgba.is_empty() {
                            captured = true;
                            break;
                        }
                    }
                }
            }
            if captured {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        cleanup(&mut app);
        assert!(captured, "no non-empty xterm capture from XQuartz display {display}");
    }

    fn find_xterm() -> Option<PathBuf> {
        first_existing(&["/opt/X11/bin/xterm"]).or_else(|| find_on_path("xterm"))
    }

    fn first_existing(paths: &[&str]) -> Option<PathBuf> {
        paths.iter().map(PathBuf::from).find(|p| p.exists())
    }

    fn find_on_path(name: &str) -> Option<PathBuf> {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|p| p.exists())
        })
    }

    fn cleanup(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(not(all(target_os = "macos", feature = "xquartz")))]
#[test]
fn xquartz_round_trip_requires_macos_xquartz_feature() {
    eprintln!("skipping: requires macOS with --features xquartz");
}
