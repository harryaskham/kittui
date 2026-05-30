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

    #![allow(dead_code, unsafe_code, unused_imports)]

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
        /// Optional `(max_width_px, max_height_px)` cap to downscale the
        /// capture before it returns. Avoids encoding huge PNGs of Retina
        /// displays when the terminal only needs a few hundred pixels.
        max_size: Option<(u32, u32)>,
    }

    impl QuartzServer {
        /// Bind to the main display (kept for API back-compat).
        pub fn spawn(_width: u32, _height: u32) -> Result<Self, XError> {
            Ok(Self {
                target: CaptureTarget::MainDisplay,
                max_size: None,
            })
        }

        /// Bind to a specific capture target.
        pub fn with_target(target: CaptureTarget) -> Self {
            Self {
                target,
                max_size: None,
            }
        }

        /// Set a maximum output size; captures will be downscaled by the
        /// ScreenCaptureKit configuration to fit within this box. Calling
        /// with a different value invalidates the cached SCStream so the
        /// new size takes effect immediately.
        pub fn set_max_size(&mut self, max_size: Option<(u32, u32)>) {
            self.max_size = max_size;
            #[cfg(feature = "sck")]
            sck::invalidate_all();
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

        #[cfg(not(feature = "sck"))]
        fn capture_legacy(&self, id: XWindowId) -> Result<XCapture, XError> {
            // Pre-macOS-12.3 path. On macOS 14+ this returns a black frame;
            // use --features sck for the supported ScreenCaptureKit path.
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
                    let row_bytes = img.bytes_per_row();
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
                    Ok(XCapture { id, width, height, rgba })
                }
                CaptureTarget::Window(window_id) => {
                    let infinite = CGRect {
                        origin: CGPoint::new(f64::INFINITY, f64::INFINITY),
                        size: core_graphics::geometry::CGSize::new(0.0, 0.0),
                    };
                    let image = unsafe {
                        CGWindowListCreateImage(infinite, 0, *window_id, K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING)
                    };
                    let (width, height, rgba) = capture_cgimage(image).ok_or_else(|| {
                        XError::Unavailable(format!(
                            "CGWindowListCreateImage failed for window {window_id}"
                        ))
                    })?;
                    Ok(XCapture { id, width, height, rgba })
                }
            }
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

        /// Global-screen-space origin to add to a local pointer event for
        /// the configured capture target. Used by `inject_pointer` so a
        /// click inside a secondary-display capture lands on the correct
        /// display, not the main one.
        pub(crate) fn global_origin_for(&self, ev: &XPointerEvent) -> (f64, f64) {
            match &self.target {
                CaptureTarget::MainDisplay => (0.0, 0.0),
                CaptureTarget::Display(id) => {
                    let b = CGDisplay::new(*id).bounds();
                    (b.origin.x, b.origin.y)
                }
                CaptureTarget::AllDisplays => {
                    let id = match ev {
                        XPointerEvent::Move { window, .. }
                        | XPointerEvent::Press { window, .. }
                        | XPointerEvent::Release { window, .. } => window.0,
                    };
                    let b = CGDisplay::new(id).bounds();
                    (b.origin.x, b.origin.y)
                }
                CaptureTarget::Window(window_id) => QuartzServer::list_app_windows()
                    .iter()
                    .find(|w| w.id == *window_id)
                    .map(|w| (w.bounds.origin.0 as f64, w.bounds.origin.1 as f64))
                    .unwrap_or((0.0, 0.0)),
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
            // ScreenCaptureKit path: required on macOS 14+ where
            // CGDisplayCreateImage silently returns a black frame.
            #[cfg(feature = "sck")]
            {
                match &self.target {
                    CaptureTarget::MainDisplay | CaptureTarget::Display(_) | CaptureTarget::AllDisplays => {
                        let display_id = match &self.target {
                            CaptureTarget::Display(d) => *d,
                            CaptureTarget::AllDisplays => id.0,
                            _ => CGDisplay::main().id,
                        };
                        sck::capture_display(display_id, self.max_size).map(|(w, h, rgba)| {
                            XCapture {
                                id,
                                width: w,
                                height: h,
                                rgba,
                            }
                        })
                    }
                    CaptureTarget::Window(window_id) => {
                        sck::capture_window(*window_id, self.max_size).map(|(w, h, rgba)| {
                            XCapture {
                                id,
                                width: w,
                                height: h,
                                rgba,
                            }
                        })
                    }
                }
            }

            #[cfg(not(feature = "sck"))]
            {
                self.capture_legacy(id)
            }
        }

        fn inject_pointer(&self, event: XPointerEvent) -> Result<(), XError> {
            // Translate local (source-pixel) coordinates into the global
            // screen layout that CGEventPost expects. For Display/AllDisplays
            // targets we add the target display's bounds.origin; for Window
            // targets we add the window's screen bounds.origin reported by
            // CGWindowList. For MainDisplay the origin is (0,0) so the
            // addition is a no-op.
            let (origin_x, origin_y) = self.global_origin_for(&event);
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|_| XError::Unavailable("CGEventSource::new failed".into()))?;
            let cg_event = match event {
                XPointerEvent::Move { x_px, y_px, .. } => CGEvent::new_mouse_event(
                    source,
                    CGEventType::MouseMoved,
                    CGPoint::new(
                        (x_px as f64) + origin_x,
                        (y_px as f64) + origin_y,
                    ),
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

    #[cfg(feature = "sck")]
    pub(crate) mod sck {
        //! ScreenCaptureKit capture path. Lazy-spawns one `SCStream` per
        //! display/window target on first call, then synchronously returns
        //! the most recent BGRA frame converted to tight RGBA.

        use super::*;
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        use std::time::{Duration, Instant};

        use core_media_rs::cm_sample_buffer::CMSampleBuffer;
        use core_video_rs::cv_pixel_buffer::lock::LockTrait;
        use screencapturekit::shareable_content::SCShareableContent;
        use screencapturekit::stream::{
            configuration::SCStreamConfiguration, content_filter::SCContentFilter,
            output_trait::SCStreamOutputTrait, output_type::SCStreamOutputType, SCStream,
        };

        /// Captured frame payload: (width, height, RGBA bytes).
        type FrameData = (u32, u32, Vec<u8>);

        #[derive(Clone)]
        struct FrameSlot(std::sync::Arc<Mutex<Option<FrameData>>>);

        impl FrameSlot {
            fn new() -> Self {
                Self(std::sync::Arc::new(Mutex::new(None)))
            }
            fn store(&self, w: u32, h: u32, rgba: Vec<u8>) {
                *self.0.lock().unwrap() = Some((w, h, rgba));
            }
            fn load(&self) -> Option<FrameData> {
                self.0.lock().unwrap().clone()
            }
        }

        struct Capturer {
            slot: FrameSlot,
        }

        impl SCStreamOutputTrait for Capturer {
            fn did_output_sample_buffer(
                &self,
                sample: CMSampleBuffer,
                of_type: SCStreamOutputType,
            ) {
                if of_type != SCStreamOutputType::Screen {
                    return;
                }
                let Ok(pixel_buffer) = sample.get_pixel_buffer() else { return };
                let w = pixel_buffer.get_width();
                let h = pixel_buffer.get_height();
                let row_bytes = pixel_buffer.get_bytes_per_row() as usize;
                let Ok(guard) = pixel_buffer.lock() else { return };
                let bytes = guard.as_slice();
                let row_pixels = (w as usize).min(row_bytes / 4);
                let mut rgba = Vec::with_capacity(row_pixels * (h as usize) * 4);
                for y in 0..h as usize {
                    let row_start = y * row_bytes;
                    if row_start + row_pixels * 4 > bytes.len() {
                        break;
                    }
                    let row = &bytes[row_start..row_start + row_pixels * 4];
                    for px in row.chunks_exact(4) {
                        // ScreenCaptureKit defaults to BGRA pixel format.
                        rgba.push(px[2]);
                        rgba.push(px[1]);
                        rgba.push(px[0]);
                        rgba.push(0xff);
                    }
                }
                self.slot.store(row_pixels as u32, h, rgba);
            }
        }

        struct ActiveStream {
            _stream: SCStream,
            slot: FrameSlot,
        }

        type StreamMap = Mutex<HashMap<u64, ActiveStream>>;

        fn streams() -> &'static StreamMap {
            static MAP: OnceLock<StreamMap> = OnceLock::new();
            MAP.get_or_init(|| Mutex::new(HashMap::new()))
        }

        fn key_display(id: u32) -> u64 {
            ((1u64) << 32) | (id as u64)
        }
        fn key_window(id: u32) -> u64 {
            ((2u64) << 32) | (id as u64)
        }

        pub(crate) fn invalidate_all() {
            streams().lock().unwrap().clear();
        }

        pub(crate) fn capture_display(
            display_id: u32,
            max_size: Option<(u32, u32)>,
        ) -> Result<(u32, u32, Vec<u8>), XError> {
            let key = key_display(display_id);
            ensure_display_stream(display_id, key, max_size)?;
            wait_for_frame(key)
        }

        pub(crate) fn capture_window(
            window_id: u32,
            max_size: Option<(u32, u32)>,
        ) -> Result<(u32, u32, Vec<u8>), XError> {
            let key = key_window(window_id);
            ensure_window_stream(window_id, key, max_size)?;
            wait_for_frame(key)
        }

        fn ensure_display_stream(
            display_id: u32,
            key: u64,
            max_size: Option<(u32, u32)>,
        ) -> Result<(), XError> {
            let mut map = streams().lock().unwrap();
            if map.contains_key(&key) {
                return Ok(());
            }
            let content = SCShareableContent::get()
                .map_err(|e| XError::Unavailable(format!("SCShareableContent::get: {e:?}")))?;
            let display = content
                .displays()
                .into_iter()
                .find(|d| d.display_id() == display_id)
                .ok_or_else(|| XError::Unavailable(format!("display {display_id} not in SCShareableContent")))?;
            let bounds = CGDisplay::new(display_id).bounds();
            let (out_w, out_h) = clamp_to_max(
                bounds.size.width as u32,
                bounds.size.height as u32,
                max_size,
            );
            let config = SCStreamConfiguration::new()
                .set_width(out_w)
                .map_err(|e| XError::Unavailable(format!("set_width: {e:?}")))?
                .set_height(out_h)
                .map_err(|e| XError::Unavailable(format!("set_height: {e:?}")))?;
            let filter = SCContentFilter::new().with_display_excluding_windows(&display, &[]);
            let slot = FrameSlot::new();
            let mut stream = SCStream::new(&filter, &config);
            stream.add_output_handler(
                Capturer { slot: slot.clone() },
                SCStreamOutputType::Screen,
            );
            stream
                .start_capture()
                .map_err(|e| XError::Unavailable(format!("start_capture: {e:?}")))?;
            map.insert(key, ActiveStream { _stream: stream, slot });
            Ok(())
        }

        fn ensure_window_stream(
            window_id: u32,
            key: u64,
            max_size: Option<(u32, u32)>,
        ) -> Result<(), XError> {
            let mut map = streams().lock().unwrap();
            if map.contains_key(&key) {
                return Ok(());
            }
            let content = SCShareableContent::get()
                .map_err(|e| XError::Unavailable(format!("SCShareableContent::get: {e:?}")))?;
            let window = content
                .windows()
                .into_iter()
                .find(|w| w.window_id() == window_id)
                .ok_or_else(|| XError::Unavailable(format!("window {window_id} not in SCShareableContent")))?;
            let frame = window.get_frame();
            let (out_w, out_h) = clamp_to_max(
                frame.size.width as u32,
                frame.size.height as u32,
                max_size,
            );
            let config = SCStreamConfiguration::new()
                .set_width(out_w)
                .map_err(|e| XError::Unavailable(format!("set_width: {e:?}")))?
                .set_height(out_h)
                .map_err(|e| XError::Unavailable(format!("set_height: {e:?}")))?;
            let filter = SCContentFilter::new().with_desktop_independent_window(&window);
            let slot = FrameSlot::new();
            let mut stream = SCStream::new(&filter, &config);
            stream.add_output_handler(
                Capturer { slot: slot.clone() },
                SCStreamOutputType::Screen,
            );
            stream
                .start_capture()
                .map_err(|e| XError::Unavailable(format!("start_capture: {e:?}")))?;
            map.insert(key, ActiveStream { _stream: stream, slot });
            Ok(())
        }

        fn wait_for_frame(key: u64) -> Result<(u32, u32, Vec<u8>), XError> {
            // ScreenCaptureKit needs a brief settle on first capture.
            let deadline = Instant::now() + Duration::from_millis(800);
            loop {
                if let Some(stream) = streams().lock().unwrap().get(&key) {
                    if let Some(frame) = stream.slot.load() {
                        return Ok(frame);
                    }
                }
                if Instant::now() >= deadline {
                    return Err(XError::Unavailable(
                        "no ScreenCaptureKit frame received within 800ms".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(16));
            }
        }

        fn clamp_to_max(w: u32, h: u32, max: Option<(u32, u32)>) -> (u32, u32) {
            let Some((mw, mh)) = max else { return (w, h) };
            if w <= mw && h <= mh {
                return (w, h);
            }
            let sw = mw as f64 / w as f64;
            let sh = mh as f64 / h as f64;
            let s = sw.min(sh);
            ((w as f64 * s).round() as u32, (h as f64 * s).round() as u32)
        }
    }
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
pub use imp::{CaptureTarget, MacDisplay, MacWindow, QuartzServer};

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
