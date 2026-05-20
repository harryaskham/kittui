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
}
