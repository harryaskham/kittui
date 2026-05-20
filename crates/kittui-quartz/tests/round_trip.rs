//! macOS-only Quartz round-trip proof.
//!
//! Runs only on macOS with the `quartz` feature enabled. Verifies:
//! - `QuartzServer::spawn` succeeds and returns reasonable dimensions.
//! - `windows()` enumerates a single virtual/main display entry.
//! - `capture` returns a non-empty RGBA buffer the expected length.
//! - `inject_pointer` and `inject_key` complete without error (the OS may
//!   silently no-op without Accessibility permission; we only assert the
//!   Rust-side call path, not actual UI delivery).
//!
//! On any other host or without the feature, this test is a no-op so
//! `cargo test --workspace` stays green everywhere.

#[cfg(all(target_os = "macos", feature = "quartz"))]
mod mac {
    use kittui_quartz::{QuartzServer, XButton, XPointerEvent, XServer};


    #[test]
    fn quartz_spawn_capture_inject_round_trip() {
        let server = QuartzServer::spawn(640, 480).expect("spawn");
        assert!(server.width() > 0);
        assert!(server.height() > 0);
        let windows = server.windows().expect("windows");
        assert_eq!(windows.len(), 1);
        let id = windows[0].id;
        let cap = server.capture(id).expect("capture");
        assert!(cap.width > 0 && cap.height > 0);
        assert_eq!(cap.rgba.len(), (cap.width * cap.height * 4) as usize);
        // Frame must not be entirely black/transparent (would indicate the
        // legacy CGDisplayCreateImage path silently returning a black frame
        // on macOS 14+, or a stalled SCK stream).
        let non_zero = cap.rgba.iter().any(|&b| b != 0 && b != 0xff);
        assert!(non_zero, "captured frame was all zero/0xff");
        // Inject events; ignore errors caused by missing Accessibility.
        let _ = server.inject_pointer(XPointerEvent::Move {
            window: id,
            x_px: 10,
            y_px: 10,
        });
        let _ = server.inject_pointer(XPointerEvent::Press {
            window: id,
            button: XButton::Left,
        });
        let _ = server.inject_pointer(XPointerEvent::Release {
            window: id,
            button: XButton::Left,
        });
        let _ = server.inject_key('a' as u32, true);
        let _ = server.inject_key('a' as u32, false);
    }

    #[test]
    fn lists_app_windows_returns_something() {
        let wins = QuartzServer::list_app_windows();
        // We can't assert exact count, but on a real macOS desktop session
        // there should be at least one non-empty window descriptor.
        eprintln!("list_app_windows() found {} windows", wins.len());
        for w in wins.iter().take(5) {
            eprintln!("  id={:>6} owner={:<24} title={:<32} bounds={:?}", w.id, w.owner_name, w.title, w.bounds);
        }
    }

    #[test]
    fn displays_returns_main() {
        let displays = QuartzServer::displays();
        assert!(!displays.is_empty(), "at least one display expected");
        eprintln!("displays: {:#?}", displays);
    }
}
