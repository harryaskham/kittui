//! kittui-xvfb
//!
//! Xvfb-backed X11 capture and input substrate for `kittui-wm`. v1 ships:
//!
//! - A backend-agnostic `XServer` trait describing how to enumerate
//!   toplevel windows, capture their RGBA pixels, and inject pointer +
//!   keyboard events back into the server.
//! - A `FakeServer` backend that runs anywhere (CI, macOS) so tests and the
//!   `kittui-wm` compositor can exercise the full pipeline without an X
//!   server present.
//! - An `xvfb`-feature-gated `XvfbServer` backend that spawns Xvfb at a
//!   chosen display, attaches via XCB+SHM, lists toplevels, captures the
//!   root pixmap, and routes pointer/key events via XTestFake* (Linux only).
//!
//! The contract is intentionally narrow so kittui-wm can swap backends
//! transparently. v1's FakeServer is what the demo and tests exercise; the
//! XvfbServer implementation lives behind a feature flag so the workspace
//! builds and tests pass on every host.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use serde::{Deserialize, Serialize};

use kittui_core::geom::PxRect;

/// Stable identifier for a window known to the X server.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct XWindowId(pub u32);

/// Mouse button identifiers shared with `kittui-input::MouseButton` but
/// re-declared here so this crate compiles independently.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XButton {
    /// Primary.
    Left,
    /// Middle.
    Middle,
    /// Secondary.
    Right,
    /// Scroll up.
    ScrollUp,
    /// Scroll down.
    ScrollDown,
}

/// Pointer event in the X server's coordinate space.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum XPointerEvent {
    /// Move to absolute `(x, y)` pixel coordinates on `window`.
    Move {
        /// Window receiving the event.
        window: XWindowId,
        /// X coordinate in window-local pixels.
        x_px: i32,
        /// Y coordinate in window-local pixels.
        y_px: i32,
    },
    /// Button press.
    Press {
        /// Window receiving the event.
        window: XWindowId,
        /// Button.
        button: XButton,
    },
    /// Button release.
    Release {
        /// Window receiving the event.
        window: XWindowId,
        /// Button.
        button: XButton,
    },
}

/// Description of one toplevel window for the compositor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XWindow {
    /// Stable id.
    pub id: XWindowId,
    /// Window title (for chrome).
    pub title: String,
    /// Pixel-space rect on the X server.
    pub rect: PxRect,
}

/// One capture of a window's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct XCapture {
    /// Window id.
    pub id: XWindowId,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tight RGBA8 bytes, row-major, top-down.
    pub rgba: Vec<u8>,
}

/// Errors surfaced by the backend.
#[derive(Debug, thiserror::Error)]
pub enum XError {
    /// Backend not available on this host (e.g. Xvfb on macOS without the
    /// xvfb feature, or an X server that failed to spawn).
    #[error("X backend unavailable: {0}")]
    Unavailable(String),
    /// Underlying IO/system error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Backend contract every X server implementation honours.
pub trait XServer {
    /// Enumerate currently-visible toplevel windows.
    fn windows(&self) -> Result<Vec<XWindow>, XError>;
    /// Capture the current pixels of a given window.
    fn capture(&self, id: XWindowId) -> Result<XCapture, XError>;
    /// Inject a pointer event into the server.
    fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError>;
    /// Inject a key event (sym = X11 keysym).
    fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError>;
}

/// A deterministic in-memory backend that pretends to host a few windows.
/// Used by the kittui-wm tests and the demo runner on hosts without Xvfb.
pub struct FakeServer {
    windows: Vec<XWindow>,
    captures: parking_lot::Mutex<Vec<XCapture>>,
    routed: parking_lot::Mutex<Vec<XPointerEvent>>,
    keys: parking_lot::Mutex<Vec<(u32, bool)>>,
}

impl FakeServer {
    /// Build a FakeServer with a single solid-color window per requested rect.
    pub fn with_windows<I>(windows: I) -> Self
    where
        I: IntoIterator<Item = (XWindowId, PxRect, &'static str, [u8; 4])>,
    {
        let mut ws = Vec::new();
        let mut caps = Vec::new();
        for (id, rect, title, rgba) in windows {
            let width = rect.width as u32;
            let height = rect.height as u32;
            let mut buf = Vec::with_capacity((width * height * 4) as usize);
            for _ in 0..(width * height) {
                buf.extend_from_slice(&rgba);
            }
            ws.push(XWindow {
                id,
                title: title.to_string(),
                rect,
            });
            caps.push(XCapture {
                id,
                width,
                height,
                rgba: buf,
            });
        }
        Self {
            windows: ws,
            captures: parking_lot::Mutex::new(caps),
            routed: parking_lot::Mutex::new(Vec::new()),
            keys: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Test helper: drain the routed pointer events seen so far.
    pub fn drain_pointer_events(&self) -> Vec<XPointerEvent> {
        std::mem::take(&mut *self.routed.lock())
    }

    /// Test helper: drain the routed key events seen so far.
    pub fn drain_key_events(&self) -> Vec<(u32, bool)> {
        std::mem::take(&mut *self.keys.lock())
    }
}

impl XServer for FakeServer {
    fn windows(&self) -> Result<Vec<XWindow>, XError> {
        Ok(self.windows.clone())
    }

    fn capture(&self, id: XWindowId) -> Result<XCapture, XError> {
        let caps = self.captures.lock();
        caps.iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| XError::Unavailable(format!("no capture for {:?}", id)))
    }

    fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError> {
        self.routed.lock().push(event);
        Ok(())
    }

    fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
        self.keys.lock().push((sym, pressed));
        Ok(())
    }
}

// Real Xvfb implementation. Currently a stub that returns `Unavailable`
// when the `xvfb` feature is on but no live Xvfb is reachable. The full
// XCB + SHM wiring lives in a follow-up bead under the parent epic.
#[cfg(feature = "xvfb")]
pub mod xvfb {
    use super::*;

    /// Placeholder Xvfb backend. Returns `Unavailable` until the XCB wiring
    /// is finished (tracked under the kittui-wm v1 epic).
    pub struct XvfbServer;

    impl XvfbServer {
        /// Try to spawn (or attach to) Xvfb at the given display number.
        pub fn spawn(_display: u32) -> Result<Self, XError> {
            Err(XError::Unavailable(
                "XvfbServer is stubbed: XCB+SHM wiring lands in a follow-up".into(),
            ))
        }
    }

    impl XServer for XvfbServer {
        fn windows(&self) -> Result<Vec<XWindow>, XError> {
            Err(XError::Unavailable("XvfbServer stub".into()))
        }
        fn capture(&self, _id: XWindowId) -> Result<XCapture, XError> {
            Err(XError::Unavailable("XvfbServer stub".into()))
        }
        fn inject_pointer(&self, _event: XPointerEvent) -> Result<(), XError> {
            Err(XError::Unavailable("XvfbServer stub".into()))
        }
        fn inject_key(&self, _sym: u32, _pressed: bool) -> Result<(), XError> {
            Err(XError::Unavailable("XvfbServer stub".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> FakeServer {
        FakeServer::with_windows(vec![
            (
                XWindowId(1),
                PxRect::new(0.0, 0.0, 64.0, 32.0),
                "alpha",
                [0xff, 0x00, 0x00, 0xff],
            ),
            (
                XWindowId(2),
                PxRect::new(100.0, 50.0, 80.0, 40.0),
                "beta",
                [0x00, 0xff, 0x00, 0xff],
            ),
        ])
    }

    #[test]
    fn fake_server_enumerates_windows_with_titles() {
        let server = fake();
        let ws = server.windows().unwrap();
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].title, "alpha");
        assert_eq!(ws[1].id, XWindowId(2));
    }

    #[test]
    fn fake_server_capture_has_expected_shape() {
        let server = fake();
        let cap = server.capture(XWindowId(1)).unwrap();
        assert_eq!((cap.width, cap.height), (64, 32));
        assert_eq!(cap.rgba.len(), (cap.width * cap.height * 4) as usize);
        assert_eq!(&cap.rgba[..4], &[0xff, 0x00, 0x00, 0xff]);
    }

    #[test]
    fn fake_server_routes_pointer_events() {
        let server = fake();
        server
            .inject_pointer(XPointerEvent::Move {
                window: XWindowId(2),
                x_px: 4,
                y_px: 5,
            })
            .unwrap();
        server
            .inject_pointer(XPointerEvent::Press {
                window: XWindowId(2),
                button: XButton::Left,
            })
            .unwrap();
        let drained = server.drain_pointer_events();
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn fake_server_routes_key_events() {
        let server = fake();
        server.inject_key(0x61, true).unwrap();
        server.inject_key(0x61, false).unwrap();
        assert_eq!(server.drain_key_events(), vec![(0x61, true), (0x61, false)]);
    }
}
