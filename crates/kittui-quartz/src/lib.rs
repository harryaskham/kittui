//! kittui-quartz
//!
//! macOS Quartz/CoreGraphics backend for kittui-wm. Implements the same
//! `XServer` trait as `kittui-xvfb` so the WM compositor stays portable: on
//! Linux you wire in `kittui-xvfb::xvfb::XvfbServer`, on macOS you wire in
//! `kittui-quartz::QuartzServer`.
//!
//! v1 scope (supported public API only):
//!
//! - Capture: `CGDisplayCreateImage` of the main display, decoded to RGBA.
//! - Pointer injection: `CGEventCreateMouseEvent` + `CGEventPost(HID)`.
//! - Key injection: `CGEventCreateKeyboardEvent` + `CGEventPost(HID)`.
//!
//! Headless virtual displays via `CGVirtualDisplayCreate*` (private API)
//! were considered for v1 and explicitly rejected: those symbols are not
//! exported from the public CoreGraphics TBD on Apple Silicon, so even
//! `extern "C"` linkage fails at link time. The hooks needed for v2 are
//! documented in docs/wm.md.
//!
//! On hosts where the `quartz` feature is disabled or `target_os != "macos"`,
//! every entry point returns `XError::Unavailable`, keeping the workspace
//! portable.

#![warn(missing_docs, rust_2018_idioms)]

pub use kittui_xvfb::{XButton, XCapture, XError, XPointerEvent, XServer, XWindow, XWindowId};

#[cfg(all(target_os = "macos", feature = "quartz"))]
mod imp {
    //! macOS-only Quartz implementation.

    #![allow(unsafe_code)]

    use core_graphics::display::{CGDirectDisplayID, CGDisplay};
    use core_graphics::event::{
        CGEvent, CGEventTapLocation, CGEventType, CGMouseButton,
    };
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::geometry::CGPoint;
    use kittui_core::geom::PxRect;
    use std::ptr;

    use super::*;

    /// macOS Quartz backend. v1 mirrors the main display and drives
    /// pointer/keyboard injection through the public CGEvent API.
    pub struct QuartzServer {
        display_id: CGDirectDisplayID,
        width: u32,
        height: u32,
    }

    impl QuartzServer {
        /// Bind to the main display. `width` and `height` are accepted for
        /// API symmetry with `XvfbServer::spawn` but ignored — capture uses
        /// the live main display's dimensions.
        pub fn spawn(_width: u32, _height: u32) -> Result<Self, XError> {
            let main = CGDisplay::main();
            let bounds = main.bounds();
            Ok(Self {
                display_id: main.id,
                width: bounds.size.width as u32,
                height: bounds.size.height as u32,
            })
        }

        /// Width of the captured display.
        pub fn width(&self) -> u32 {
            self.width
        }
        /// Height of the captured display.
        pub fn height(&self) -> u32 {
            self.height
        }
        /// Always `false` in v1; reserved for a future headless backend.
        pub fn is_virtual(&self) -> bool {
            false
        }
    }

    impl XServer for QuartzServer {
        fn windows(&self) -> Result<Vec<XWindow>, XError> {
            // v1 treats the display as a single window. Multi-window
            // enumeration via CGWindowListCopyWindowInfo lands in v2.
            Ok(vec![XWindow {
                id: XWindowId(self.display_id as u32),
                title: "quartz-main-display".into(),
                rect: PxRect::new(0.0, 0.0, self.width as f32, self.height as f32),
            }])
        }

        fn capture(&self, _id: XWindowId) -> Result<XCapture, XError> {
            let img = CGDisplay::new(self.display_id)
                .image()
                .ok_or_else(|| XError::Unavailable("CGDisplayCreateImage returned null".into()))?;
            let width = img.width() as u32;
            let height = img.height() as u32;
            let data = img.data();
            // CGImage data is BGRA, premultiplied, 4-byte aligned per row
            // (bytes_per_row may exceed width*4). Convert to tight RGBA.
            let row_bytes = img.bytes_per_row() as usize;
            let bytes = data.bytes();
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height as usize {
                let row = &bytes[y * row_bytes..y * row_bytes + (width as usize) * 4];
                for px in row.chunks_exact(4) {
                    rgba.push(px[2]);
                    rgba.push(px[1]);
                    rgba.push(px[0]);
                    rgba.push(0xff);
                }
            }
            Ok(XCapture {
                id: XWindowId(self.display_id as u32),
                width,
                height,
                rgba,
            })
        }

        fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError> {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|_| XError::Unavailable("CGEventSource::new failed".into()))?;
            let cg_event = match event {
                XPointerEvent::Move { x_px, y_px, .. } => CGEvent::new_mouse_event(
                    source,
                    CGEventType::MouseMoved,
                    CGPoint::new(x_px as f64, y_px as f64),
                    CGMouseButton::Left,
                )
                .map_err(|_| XError::Unavailable("CGEvent move failed".into()))?,
                XPointerEvent::Press { button, .. } => {
                    let (ty, btn) = press_for(button, true);
                    let pt = current_pointer();
                    CGEvent::new_mouse_event(source, ty, pt, btn)
                        .map_err(|_| XError::Unavailable("CGEvent press failed".into()))?
                }
                XPointerEvent::Release { button, .. } => {
                    let (ty, btn) = press_for(button, false);
                    let pt = current_pointer();
                    CGEvent::new_mouse_event(source, ty, pt, btn)
                        .map_err(|_| XError::Unavailable("CGEvent release failed".into()))?
                }
            };
            cg_event.post(CGEventTapLocation::HID);
            Ok(())
        }

        fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|_| XError::Unavailable("CGEventSource::new failed".into()))?;
            // sym is treated as a virtual keycode hint. Full keysym->keycode
            // mapping needs the Carbon HIToolbox UCKeyTranslate machinery
            // and lands in v2. For ASCII chars we set the unicode string
            // payload so the system synthesises the correct keypress.
            let ev = CGEvent::new_keyboard_event(source, sym as u16, pressed)
                .map_err(|_| XError::Unavailable("CGEvent key failed".into()))?;
            if pressed {
                if let Some(ch) = char::from_u32(sym) {
                    let mut buf = [0u16; 2];
                    let encoded = ch.encode_utf16(&mut buf);
                    ev.set_string_from_utf16_unchecked(encoded);
                }
            }
            ev.post(CGEventTapLocation::HID);
            Ok(())
        }
    }

    fn press_for(b: XButton, pressed: bool) -> (CGEventType, CGMouseButton) {
        match (b, pressed) {
            (XButton::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
            (XButton::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
            (XButton::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
            (XButton::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            (XButton::Middle, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (XButton::Middle, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
            // Scroll events use scroll-wheel API; v1 maps them to button events.
            (XButton::ScrollUp, _) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (XButton::ScrollDown, _) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
        }
    }

    fn current_pointer() -> CGPoint {
        unsafe {
            extern "C" {
                fn CGEventCreate(source: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
                fn CGEventGetLocation(event: *mut core::ffi::c_void) -> CGPoint;
                fn CFRelease(_: *mut core::ffi::c_void);
            }
            let ev = CGEventCreate(ptr::null_mut());
            if ev.is_null() {
                return CGPoint::new(0.0, 0.0);
            }
            let p = CGEventGetLocation(ev);
            CFRelease(ev);
            p
        }
    }
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
pub use imp::QuartzServer;

/// Stub used when the crate is built on a non-macOS host or without the
/// `quartz` feature. Returns `XError::Unavailable` from every entry point so
/// kittui-wm can still try to construct a backend portably and fall back to
/// `FakeServer` when this returns unavailable.
#[cfg(not(all(target_os = "macos", feature = "quartz")))]
pub struct QuartzServer;

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
impl QuartzServer {
    /// Always returns `XError::Unavailable` when the `quartz` feature is off.
    pub fn spawn(_width: u32, _height: u32) -> Result<Self, XError> {
        Err(XError::Unavailable(
            "kittui-quartz: rebuild with --features quartz on macOS".into(),
        ))
    }
    /// Stub.
    pub fn width(&self) -> u32 {
        0
    }
    /// Stub.
    pub fn height(&self) -> u32 {
        0
    }
    /// Stub.
    pub fn is_virtual(&self) -> bool {
        false
    }
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
impl XServer for QuartzServer {
    fn windows(&self) -> Result<Vec<XWindow>, XError> {
        Err(XError::Unavailable("quartz feature disabled".into()))
    }
    fn capture(&self, _id: XWindowId) -> Result<XCapture, XError> {
        Err(XError::Unavailable("quartz feature disabled".into()))
    }
    fn inject_pointer(&self, _event: XPointerEvent) -> Result<(), XError> {
        Err(XError::Unavailable("quartz feature disabled".into()))
    }
    fn inject_key(&self, _sym: u32, _pressed: bool) -> Result<(), XError> {
        Err(XError::Unavailable("quartz feature disabled".into()))
    }
}
