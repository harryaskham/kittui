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

    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::display::{CGDirectDisplayID, CGDisplay};
    use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::geometry::{CGPoint, CGRect};
    use kittui_core::geom::PxRect;
    use std::ptr;

    use super::*;

    // ---- Private/public CGWindowList FFI -----------------------------------
    //
    // CGWindowListCopyWindowInfo and CGWindowListCreateImage are both public
    // since 10.5 but the Rust core-graphics crate doesn't expose them yet,
    // so we declare the bare extern "C" symbols ourselves. These live in
    // CoreGraphics.framework and are linked automatically via the
    // core-graphics crate's framework link attribute.
    type CGWindowListOption = u32;
    type CGWindowID = u32;
    type CGWindowImageOption = u32;
    const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: CGWindowListOption = 1 << 0;
    const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: CGWindowListOption = 1 << 4;
    const K_CG_NULL_WINDOW_ID: CGWindowID = 0;
    const K_CG_WINDOW_IMAGE_DEFAULT: CGWindowImageOption = 0;
    const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: CGWindowImageOption = 1 << 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(
            option: CGWindowListOption,
            relative_to_window: CGWindowID,
        ) -> CFArrayRef;
        fn CGWindowListCreateImage(
            screen_bounds: CGRect,
            list_option: CGWindowListOption,
            window_id: CGWindowID,
            image_option: CGWindowImageOption,
        ) -> *mut core::ffi::c_void;
        fn CGImageGetWidth(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetHeight(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetBytesPerRow(image: *mut core::ffi::c_void) -> usize;
        fn CGImageGetDataProvider(image: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn CGDataProviderCopyData(provider: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn CFDataGetLength(data: *mut core::ffi::c_void) -> isize;
        fn CFDataGetBytePtr(data: *mut core::ffi::c_void) -> *const u8;
        fn CFRelease(_: *mut core::ffi::c_void);
    }

    /// What this QuartzServer captures and routes input to.
    #[derive(Clone, Debug)]
    pub enum CaptureTarget {
        /// The main connected display.
        MainDisplay,
        /// A specific connected display by `CGDirectDisplayID`.
        Display(CGDirectDisplayID),
        /// A specific macOS window by `CGWindowID`.
        Window(CGWindowID),
        /// All connected displays, exposed as one logical window per display.
        AllDisplays,
    }

    /// Lightweight descriptor of one macOS app window discovered via
    /// CGWindowListCopyWindowInfo.
    #[derive(Clone, Debug)]
    pub struct MacWindow {
        /// CGWindowID assigned by the WindowServer.
        pub id: CGWindowID,
        /// Owning process id, if reported.
        pub owner_pid: i32,
        /// Window title; empty when the app didn't set one.
        pub title: String,
        /// Owning app's display name, e.g. "Safari".
        pub owner_name: String,
        /// Window bounds in screen coordinates.
        pub bounds: PxRect,
    }

    /// Lightweight descriptor of one connected display.
    #[derive(Clone, Debug)]
    pub struct MacDisplay {
        /// CGDirectDisplayID.
        pub id: CGDirectDisplayID,
        /// Display bounds in global screen coordinates.
        pub bounds: PxRect,
        /// Index within the active-display list (0 = main).
        pub index: usize,
    }

    /// macOS Quartz backend. The capture target picks what `windows()` and
    /// `capture()` return; pointer + key injection always goes through the
    /// global HID event tap.
    pub struct QuartzServer {
        target: CaptureTarget,
    }

    impl QuartzServer {
        /// Bind to the main display (kept for API back-compat).
        pub fn spawn(_width: u32, _height: u32) -> Result<Self, XError> {
            Ok(Self {
                target: CaptureTarget::MainDisplay,
            })
        }

        /// Bind to a specific capture target.
        pub fn with_target(target: CaptureTarget) -> Self {
            Self { target }
        }

        /// Enumerate connected displays.
        pub fn displays() -> Vec<MacDisplay> {
            CGDisplay::active_displays()
                .map(|ids| {
                    ids.into_iter()
                        .enumerate()
                        .map(|(i, id)| {
                            let d = CGDisplay::new(id);
                            let b = d.bounds();
                            MacDisplay {
                                id,
                                bounds: PxRect::new(
                                    b.origin.x as f32,
                                    b.origin.y as f32,
                                    b.size.width as f32,
                                    b.size.height as f32,
                                ),
                                index: i,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        }

        /// Enumerate on-screen, titled application windows.
        pub fn list_app_windows() -> Vec<MacWindow> {
            unsafe {
                let opts = K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY
                    | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
                let array_ref = CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID);
                if array_ref.is_null() {
                    return Vec::new();
                }
                let array: CFArray<CFType> = CFArray::wrap_under_create_rule(array_ref);
                let mut out = Vec::with_capacity(array.len() as usize);
                for item in array.iter() {
                    let dict_ref = item.as_CFTypeRef() as CFDictionaryRef;
                    let dict: CFDictionary<CFString, CFType> =
                        CFDictionary::wrap_under_get_rule(dict_ref);
                    let id = read_u32(&dict, "kCGWindowNumber").unwrap_or(0);
                    if id == 0 {
                        continue;
                    }
                    let owner_pid = read_i32(&dict, "kCGWindowOwnerPID").unwrap_or(-1);
                    let title = read_string(&dict, "kCGWindowName").unwrap_or_default();
                    let owner_name =
                        read_string(&dict, "kCGWindowOwnerName").unwrap_or_default();
                    let bounds = read_bounds(&dict).unwrap_or(PxRect::new(0.0, 0.0, 0.0, 0.0));
                    if title.is_empty() && owner_name.is_empty() {
                        continue;
                    }
                    out.push(MacWindow {
                        id,
                        owner_pid,
                        title,
                        owner_name,
                        bounds,
                    });
                }
                out
            }
        }

        /// Width of the current capture target's primary surface.
        pub fn width(&self) -> u32 {
            self.bounds().2
        }
        /// Height of the current capture target's primary surface.
        pub fn height(&self) -> u32 {
            self.bounds().3
        }
        /// Reserved for v3 headless backend.
        pub fn is_virtual(&self) -> bool {
            false
        }

        fn bounds(&self) -> (i32, i32, u32, u32) {
            match &self.target {
                CaptureTarget::MainDisplay => {
                    let b = CGDisplay::main().bounds();
                    (
                        b.origin.x as i32,
                        b.origin.y as i32,
                        b.size.width as u32,
                        b.size.height as u32,
                    )
                }
                CaptureTarget::Display(id) => {
                    let b = CGDisplay::new(*id).bounds();
                    (
                        b.origin.x as i32,
                        b.origin.y as i32,
                        b.size.width as u32,
                        b.size.height as u32,
                    )
                }
                CaptureTarget::Window(_) | CaptureTarget::AllDisplays => (0, 0, 0, 0),
            }
        }
    }

    fn read_u32(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<u32> {
        let k = CFString::from(key);
        let v = dict.find(&k)?;
        let n = v.downcast::<CFNumber>()?;
        n.to_i32().map(|x| x as u32)
    }

    fn read_i32(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<i32> {
        let k = CFString::from(key);
        let v = dict.find(&k)?;
        let n = v.downcast::<CFNumber>()?;
        n.to_i32()
    }

    fn read_string(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
        let k = CFString::from(key);
        let v = dict.find(&k)?;
        let s = v.downcast::<CFString>()?;
        Some(s.to_string())
    }

    fn read_bounds(dict: &CFDictionary<CFString, CFType>) -> Option<PxRect> {
        let k = CFString::from("kCGWindowBounds");
        let v = dict.find(&k)?;
        let dict_ref = v.as_CFTypeRef() as CFDictionaryRef;
        let inner: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(dict_ref) };
        let x = read_f64(&inner, "X")?;
        let y = read_f64(&inner, "Y")?;
        let w = read_f64(&inner, "Width")?;
        let h = read_f64(&inner, "Height")?;
        Some(PxRect::new(x as f32, y as f32, w as f32, h as f32))
    }

    fn read_f64(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
        let k = CFString::from(key);
        let v = dict.find(&k)?;
        let n = v.downcast::<CFNumber>()?;
        n.to_f64()
    }

    fn capture_cgimage(image: *mut core::ffi::c_void) -> Option<(u32, u32, Vec<u8>)> {
        if image.is_null() {
            return None;
        }
        unsafe {
            let width = CGImageGetWidth(image) as u32;
            let height = CGImageGetHeight(image) as u32;
            let row_bytes = CGImageGetBytesPerRow(image);
            let provider = CGImageGetDataProvider(image);
            if provider.is_null() {
                CFRelease(image);
                return None;
            }
            let data = CGDataProviderCopyData(provider);
            if data.is_null() {
                CFRelease(image);
                return None;
            }
            let len = CFDataGetLength(data) as usize;
            let ptr = CFDataGetBytePtr(data);
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height as usize {
                let off = y * row_bytes;
                if off + (width as usize) * 4 > len {
                    break;
                }
                let row = std::slice::from_raw_parts(ptr.add(off), (width as usize) * 4);
                for px in row.chunks_exact(4) {
                    rgba.push(px[2]);
                    rgba.push(px[1]);
                    rgba.push(px[0]);
                    rgba.push(0xff);
                }
            }
            CFRelease(data);
            CFRelease(image);
            Some((width, height, rgba))
        }
    }

    impl XServer for QuartzServer {
        fn windows(&self) -> Result<Vec<XWindow>, XError> {
            match &self.target {
                CaptureTarget::MainDisplay | CaptureTarget::Display(_) => {
                    let (x, y, w, h) = self.bounds();
                    let id = match &self.target {
                        CaptureTarget::Display(d) => *d,
                        _ => CGDisplay::main().id,
                    };
                    Ok(vec![XWindow {
                        id: XWindowId(id),
                        title: format!("quartz-display-{}", id),
                        rect: PxRect::new(x as f32, y as f32, w as f32, h as f32),
                    }])
                }
                CaptureTarget::Window(window_id) => {
                    // Resolve the window's current bounds from CGWindowList.
                    let win = QuartzServer::list_app_windows()
                        .into_iter()
                        .find(|w| w.id == *window_id)
                        .ok_or_else(|| {
                            XError::Unavailable(format!("window {window_id} not found"))
                        })?;
                    Ok(vec![XWindow {
                        id: XWindowId(*window_id),
                        title: if win.title.is_empty() {
                            win.owner_name.clone()
                        } else {
                            win.title.clone()
                        },
                        rect: win.bounds,
                    }])
                }
                CaptureTarget::AllDisplays => {
                    let displays = QuartzServer::displays();
                    Ok(displays
                        .into_iter()
                        .map(|d| XWindow {
                            id: XWindowId(d.id),
                            title: format!("display-{}", d.index),
                            rect: d.bounds,
                        })
                        .collect())
                }
            }
        }

        fn capture(&self, id: XWindowId) -> Result<XCapture, XError> {
            match &self.target {
                CaptureTarget::MainDisplay | CaptureTarget::Display(_) | CaptureTarget::AllDisplays => {
                    let display_id = match &self.target {
                        CaptureTarget::Display(d) => *d,
                        CaptureTarget::AllDisplays => id.0,
                        _ => CGDisplay::main().id,
                    };
                    let img = CGDisplay::new(display_id).image().ok_or_else(|| {
                        XError::Unavailable("CGDisplayCreateImage returned null".into())
                    })?;
                    let width = img.width() as u32;
                    let height = img.height() as u32;
                    let data = img.data();
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
                        id,
                        width,
                        height,
                        rgba,
                    })
                }
                CaptureTarget::Window(window_id) => {
                    let infinite = CGRect {
                        origin: CGPoint::new(f64::INFINITY, f64::INFINITY),
                        size: core_graphics::geometry::CGSize::new(0.0, 0.0),
                    };
                    let image = unsafe {
                        CGWindowListCreateImage(
                            infinite,
                            0,
                            *window_id,
                            K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING,
                        )
                    };
                    let (width, height, rgba) = capture_cgimage(image).ok_or_else(|| {
                        XError::Unavailable(format!(
                            "CGWindowListCreateImage failed for window {window_id}"
                        ))
                    })?;
                    Ok(XCapture {
                        id,
                        width,
                        height,
                        rgba,
                    })
                }
            }
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
