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
    /// Resize a window or capture target to the requested pixel size.
    ///
    /// Backends that cannot control the target size keep the default
    /// `Unavailable` implementation. Surface adapters use this hook for the
    /// common resize path while still allowing read-only capture backends.
    fn resize_window(&self, id: XWindowId, width: u32, height: u32) -> Result<(), XError> {
        let _ = (id, width, height);
        Err(XError::Unavailable(
            "backend does not support window resize".into(),
        ))
    }
    /// Inject a pointer event into the server.
    fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError>;
    /// Inject a key event (sym = X11 keysym).
    fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError>;
}

/// A deterministic in-memory backend that pretends to host a few windows.
/// Used by the kittui-wm tests and the demo runner on hosts without Xvfb.
pub struct FakeServer {
    windows: parking_lot::Mutex<Vec<XWindow>>,
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
            windows: parking_lot::Mutex::new(ws),
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
        Ok(self.windows.lock().clone())
    }

    fn capture(&self, id: XWindowId) -> Result<XCapture, XError> {
        let caps = self.captures.lock();
        caps.iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| XError::Unavailable(format!("no capture for {:?}", id)))
    }

    fn resize_window(&self, id: XWindowId, width: u32, height: u32) -> Result<(), XError> {
        if width == 0 || height == 0 {
            return Err(XError::Unavailable(
                "window resize requires non-zero size".into(),
            ));
        }
        let mut windows = self.windows.lock();
        if let Some(window) = windows.iter_mut().find(|w| w.id == id) {
            window.rect.width = width as f32;
            window.rect.height = height as f32;
        }
        let mut captures = self.captures.lock();
        let capture = captures
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| XError::Unavailable(format!("no capture for {:?}", id)))?;
        let pixel = capture.rgba.get(0..4).unwrap_or(&[0, 0, 0, 0xff]).to_vec();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            rgba.extend_from_slice(&pixel);
        }
        capture.width = width;
        capture.height = height;
        capture.rgba = rgba;
        Ok(())
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

// Real Xvfb implementation, behind the `xvfb` feature. Uses x11rb (pure-Rust
// XCB) so no `unsafe` is needed in this crate. The backend spawns an Xvfb
// child process, attaches to its display, enumerates toplevels, captures
// window pixels via `GetImage`, and injects pointer/key events via XTest.
/// Real Xvfb/XQuartz backend. Pure-Rust XCB via `x11rb`, no `unsafe`.
/// Enabled with the `xvfb` or `xquartz` cargo feature.
#[cfg(any(feature = "xvfb", feature = "xquartz"))]
pub mod xvfb {
    use super::*;

    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::Duration;

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConfigureWindowAux, ConnectionExt as _, GetImageReply, ImageFormat,
        Window as XWindow32,
    };
    use x11rb::protocol::xtest::ConnectionExt as XTestExt;
    use x11rb::rust_connection::RustConnection;

    /// A live Xvfb server connected via XCB. Owns the child process so it is
    /// torn down when this struct drops.
    pub struct XvfbServer {
        conn: RustConnection,
        screen_root: XWindow32,
        _xvfb: Option<Child>,
        display: String,
    }

    impl XvfbServer {
        /// Spawn `Xvfb :<display>` and connect to it.
        pub fn spawn(display: u32) -> Result<Self, XError> {
            let display_str = format!(":{display}");
            let xvfb = Command::new("Xvfb")
                .arg(&display_str)
                .args(["-screen", "0", "1024x768x24"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| XError::Unavailable(format!("failed to spawn Xvfb: {e}")))?;
            // Wait for the X socket to come up.
            let mut last_err = None;
            for _ in 0..30 {
                thread::sleep(Duration::from_millis(100));
                match RustConnection::connect(Some(&display_str)) {
                    Ok((conn, screen_num)) => {
                        let screen = &conn.setup().roots[screen_num];
                        let root = screen.root;
                        return Ok(Self {
                            conn,
                            screen_root: root,
                            _xvfb: Some(xvfb),
                            display: display_str,
                        });
                    }
                    Err(e) => last_err = Some(e),
                }
            }
            Err(XError::Unavailable(format!(
                "could not connect to Xvfb at {display_str}: {last_err:?}"
            )))
        }

        /// Attach to an already-running X server on the given DISPLAY.
        pub fn attach(display: &str) -> Result<Self, XError> {
            let (conn, screen_num) = RustConnection::connect(Some(display))
                .map_err(|e| XError::Unavailable(format!("connect {display}: {e}")))?;
            let screen = &conn.setup().roots[screen_num];
            let root = screen.root;
            Ok(Self {
                conn,
                screen_root: root,
                _xvfb: None,
                display: display.to_string(),
            })
        }

        /// The DISPLAY string this backend is using.
        pub fn display(&self) -> &str {
            &self.display
        }
    }

    impl XServer for XvfbServer {
        fn windows(&self) -> Result<Vec<XWindow>, XError> {
            let tree = self
                .conn
                .query_tree(self.screen_root)
                .map_err(|e| XError::Unavailable(format!("query_tree: {e}")))?
                .reply()
                .map_err(|e| XError::Unavailable(format!("query_tree reply: {e}")))?;
            let mut out = Vec::new();
            for child in tree.children.iter().copied() {
                let attrs = match self.conn.get_window_attributes(child) {
                    Ok(c) => match c.reply() {
                        Ok(a) => a,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
                if attrs.map_state != x11rb::protocol::xproto::MapState::VIEWABLE {
                    continue;
                }
                let geom = match self.conn.get_geometry(child) {
                    Ok(c) => match c.reply() {
                        Ok(g) => g,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
                let title = read_title(&self.conn, child).unwrap_or_default();
                out.push(XWindow {
                    id: XWindowId(child),
                    title,
                    rect: PxRect::new(
                        geom.x as f32,
                        geom.y as f32,
                        geom.width as f32,
                        geom.height as f32,
                    ),
                });
            }
            Ok(out)
        }

        fn capture(&self, id: XWindowId) -> Result<XCapture, XError> {
            let geom = self
                .conn
                .get_geometry(id.0)
                .map_err(|e| XError::Unavailable(format!("get_geometry: {e}")))?
                .reply()
                .map_err(|e| XError::Unavailable(format!("get_geometry reply: {e}")))?;
            let img: GetImageReply = self
                .conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    id.0,
                    0,
                    0,
                    geom.width,
                    geom.height,
                    !0,
                )
                .map_err(|e| XError::Unavailable(format!("get_image: {e}")))?
                .reply()
                .map_err(|e| XError::Unavailable(format!("get_image reply: {e}")))?;
            // Xvfb returns BGRA (or BGR for 24-bit depth). Convert to RGBA.
            let mut rgba = Vec::with_capacity((geom.width as usize) * (geom.height as usize) * 4);
            for chunk in img.data.chunks_exact(4) {
                rgba.push(chunk[2]);
                rgba.push(chunk[1]);
                rgba.push(chunk[0]);
                rgba.push(0xff);
            }
            Ok(XCapture {
                id,
                width: geom.width as u32,
                height: geom.height as u32,
                rgba,
            })
        }

        fn resize_window(&self, id: XWindowId, width: u32, height: u32) -> Result<(), XError> {
            if width == 0 || height == 0 {
                return Err(XError::Unavailable(
                    "window resize requires non-zero size".into(),
                ));
            }
            self.conn
                .configure_window(id.0, &ConfigureWindowAux::new().width(width).height(height))
                .map_err(|e| XError::Unavailable(format!("configure_window: {e}")))?;
            self.conn
                .flush()
                .map_err(|e| XError::Unavailable(format!("flush: {e}")))?;
            Ok(())
        }

        fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError> {
            match event {
                XPointerEvent::Move { window, x_px, y_px } => {
                    self.conn
                        .xtest_fake_input(
                            x11rb::protocol::xproto::MOTION_NOTIFY_EVENT,
                            0,
                            x11rb::CURRENT_TIME,
                            window.0,
                            x_px as i16,
                            y_px as i16,
                            0,
                        )
                        .map_err(|e| {
                            XError::Unavailable(format!("xtest_fake_input motion: {e}"))
                        })?;
                }
                XPointerEvent::Press { window, button } => {
                    self.conn
                        .xtest_fake_input(
                            x11rb::protocol::xproto::BUTTON_PRESS_EVENT,
                            xbutton_to_code(button),
                            x11rb::CURRENT_TIME,
                            window.0,
                            0,
                            0,
                            0,
                        )
                        .map_err(|e| XError::Unavailable(format!("xtest_fake_input press: {e}")))?;
                }
                XPointerEvent::Release { window, button } => {
                    self.conn
                        .xtest_fake_input(
                            x11rb::protocol::xproto::BUTTON_RELEASE_EVENT,
                            xbutton_to_code(button),
                            x11rb::CURRENT_TIME,
                            window.0,
                            0,
                            0,
                            0,
                        )
                        .map_err(|e| {
                            XError::Unavailable(format!("xtest_fake_input release: {e}"))
                        })?;
                }
            }
            self.conn
                .flush()
                .map_err(|e| XError::Unavailable(format!("flush: {e}")))?;
            Ok(())
        }

        fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
            // Best-effort: convert keysym -> keycode via the server. For
            // characters we use the lower 8 bits as a keycode hint; correct
            // mapping needs Xkb which lands in v2.
            let code = (sym & 0xff) as u8;
            let kind = if pressed {
                x11rb::protocol::xproto::KEY_PRESS_EVENT
            } else {
                x11rb::protocol::xproto::KEY_RELEASE_EVENT
            };
            self.conn
                .xtest_fake_input(kind, code, x11rb::CURRENT_TIME, self.screen_root, 0, 0, 0)
                .map_err(|e| XError::Unavailable(format!("xtest_fake_input key: {e}")))?;
            self.conn
                .flush()
                .map_err(|e| XError::Unavailable(format!("flush: {e}")))?;
            Ok(())
        }
    }

    fn xbutton_to_code(b: XButton) -> u8 {
        match b {
            XButton::Left => 1,
            XButton::Middle => 2,
            XButton::Right => 3,
            XButton::ScrollUp => 4,
            XButton::ScrollDown => 5,
        }
    }

    fn read_title(conn: &RustConnection, w: XWindow32) -> Option<String> {
        let cookie = conn
            .get_property(false, w, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
            .ok()?;
        let reply = cookie.reply().ok()?;
        String::from_utf8(reply.value).ok()
    }
}

/// XQuartz-backed X11 server for macOS.
///
/// This is a thin wrapper around the same x11rb/XServer implementation used
/// by [`xvfb::XvfbServer`]. `spawn` starts a private XQuartz display in
/// `-nolisten tcp` mode and then attaches to it; `attach` connects to an
/// already-running XQuartz/X11 display.
#[cfg(all(target_os = "macos", feature = "xquartz"))]
pub mod xquartz {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    /// A live XQuartz display plus x11rb capture/input adapter.
    pub struct XQuartzServer {
        inner: crate::xvfb::XvfbServer,
        _xquartz: Option<Child>,
    }

    impl XQuartzServer {
        /// Spawn `Xquartz :<display> -nolisten tcp` and connect to it.
        pub fn spawn(display: u32) -> Result<Self, XError> {
            let bin = find_xquartz().ok_or_else(|| {
                XError::Unavailable(
                    "XQuartz not found; install it or set KITTUI_XQUARTZ_BIN".into(),
                )
            })?;
            let display_str = format!(":{display}");
            let child = Command::new(&bin)
                .arg(&display_str)
                .args(["-nolisten", "tcp"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| XError::Unavailable(format!("spawn {}: {e}", bin.display())))?;
            let started = Instant::now();
            loop {
                match crate::xvfb::XvfbServer::attach(&display_str) {
                    Ok(inner) => {
                        return Ok(Self {
                            inner,
                            _xquartz: Some(child),
                        })
                    }
                    Err(e) if started.elapsed() < Duration::from_secs(10) => {
                        let _ = e;
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        /// Attach to an already-running XQuartz/X11 display.
        pub fn attach(display: &str) -> Result<Self, XError> {
            Ok(Self {
                inner: crate::xvfb::XvfbServer::attach(display)?,
                _xquartz: None,
            })
        }

        /// DISPLAY string for this server.
        pub fn display(&self) -> &str {
            self.inner.display()
        }
    }

    impl XServer for XQuartzServer {
        fn windows(&self) -> Result<Vec<XWindow>, XError> {
            self.inner.windows()
        }

        fn capture(&self, id: XWindowId) -> Result<XCapture, XError> {
            self.inner.capture(id)
        }

        fn resize_window(&self, id: XWindowId, width: u32, height: u32) -> Result<(), XError> {
            self.inner.resize_window(id, width, height)
        }

        fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError> {
            self.inner.inject_pointer(event)
        }

        fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
            self.inner.inject_key(sym, pressed)
        }
    }

    fn find_xquartz() -> Option<PathBuf> {
        std::env::var_os("KITTUI_XQUARTZ_BIN")
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(|| {
                first_existing(&[
                    "/opt/X11/bin/Xquartz",
                    "/Applications/Utilities/XQuartz.app/Contents/MacOS/X11.bin",
                    "/Library/Apple/System/Library/CoreServices/X11.app/Contents/MacOS/X11.bin",
                ])
            })
    }

    fn first_existing(paths: &[&str]) -> Option<PathBuf> {
        paths.iter().map(PathBuf::from).find(|p| p.exists())
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
