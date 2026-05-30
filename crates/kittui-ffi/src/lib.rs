//! kittui-ffi — C ABI for kittui.
//!
//! Exposes a tiny stable C surface that accepts scenes as JSON blobs and
//! returns placement bytes via owned C strings. Every entry point is
//! `catch_unwind`-wrapped to avoid panicking across the FFI boundary.
//!
//! `unsafe` is necessary here because we cross the FFI boundary; it is
//! confined to this crate and audited per-function.

#![warn(missing_docs, rust_2018_idioms)]

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::sync::Mutex;

use base64::Engine;
use kittui::{CellSize, RendererKind, Runtime, Scene, TerminalInfo, Transport};

/// Major version of the FFI ABI. Bumped on any breaking change.
pub const KITTUI_ABI_MAJOR: u32 = 0;
/// Minor version of the FFI ABI. Bumped on additive changes.
pub const KITTUI_ABI_MINOR: u32 = 8;

/// Opaque pointer to a runtime instance. Owned by the caller; freed via
/// [`kittui_runtime_free`].
pub struct KittuiRuntime {
    inner: Runtime,
    last_error: Mutex<Option<CString>>,
}

/// Status code returned by FFI calls.
#[repr(i32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum KittuiStatus {
    /// Success.
    Ok = 0,
    /// Null pointer where a value was required.
    NullPointer = 1,
    /// Failed to parse a JSON scene blob.
    BadScene = 2,
    /// Underlying runtime error.
    Runtime = 3,
    /// Caught a panic before it unwound across FFI.
    Panic = 4,
}

/// Return the ABI version as a packed `(major << 16) | minor` integer.
#[no_mangle]
pub extern "C" fn kittui_abi_version() -> u32 {
    (KITTUI_ABI_MAJOR << 16) | KITTUI_ABI_MINOR
}

/// Construct a runtime. `cache_dir` may be null to use the platform default.
///
/// # Safety
///
/// `cache_dir`, if non-null, must point to a NUL-terminated UTF-8 string for
/// the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn kittui_runtime_new(cache_dir: *const c_char) -> *mut KittuiRuntime {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut builder = Runtime::builder().renderer(RendererKind::Cpu);
        if !cache_dir.is_null() {
            let s = CStr::from_ptr(cache_dir).to_string_lossy().into_owned();
            builder = builder.cache_dir(PathBuf::from(s));
        }
        runtime_ptr_from_builder(builder)
    }));
    result.unwrap_or(ptr::null_mut())
}

/// Construct a runtime from a JSON config blob. Supported fields:
/// `cache_dir`, `renderer`, `transport`, `columns`, `rows`, `cell_width_px`,
/// `cell_height_px`, `supports_kitty`, and `supports_unicode_placeholders`.
///
/// # Safety
///
/// `json` must be null or point to a NUL-terminated UTF-8 string for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn kittui_runtime_new_config(json: *const c_char) -> *mut KittuiRuntime {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if json.is_null() {
            return runtime_ptr_from_builder(Runtime::builder().renderer(RendererKind::Cpu));
        }
        let s = match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        };
        match runtime_from_config_str(s) {
            Ok(runtime) => Box::into_raw(Box::new(KittuiRuntime {
                inner: runtime,
                last_error: Mutex::new(None),
            })),
            Err(_) => ptr::null_mut(),
        }
    }));
    result.unwrap_or(ptr::null_mut())
}

fn runtime_from_config_str(s: &str) -> Result<Runtime, String> {
    let value: serde_json::Value = serde_json::from_str(s).map_err(|e| e.to_string())?;
    let renderer = parse_renderer(
        value
            .get("renderer")
            .and_then(|v| v.as_str())
            .unwrap_or("cpu"),
    )
    .ok_or_else(|| "invalid renderer; expected cpu|gpu|auto".to_string())?;
    let mut builder = Runtime::builder().renderer(renderer);
    if let Some(cache_dir) = value.get("cache_dir").and_then(|v| v.as_str()) {
        builder = builder.cache_dir(PathBuf::from(cache_dir));
    }
    let terminal =
        terminal_from_config(&value).ok_or_else(|| "invalid terminal config values".to_string())?;
    builder = builder.terminal(terminal);
    builder.build().map_err(|e| e.to_string())
}

fn runtime_ptr_from_builder(builder: kittui::RuntimeBuilder) -> *mut KittuiRuntime {
    match builder.build() {
        Ok(runtime) => Box::into_raw(Box::new(KittuiRuntime {
            inner: runtime,
            last_error: Mutex::new(None),
        })),
        Err(_) => ptr::null_mut(),
    }
}

fn parse_renderer(value: &str) -> Option<RendererKind> {
    match value.to_ascii_lowercase().as_str() {
        "cpu" => Some(RendererKind::Cpu),
        "gpu" => Some(RendererKind::Gpu),
        "auto" => Some(RendererKind::Auto),
        _ => None,
    }
}

fn parse_transport(value: &str) -> Option<Transport> {
    match value.to_ascii_lowercase().replace('-', "_").as_str() {
        "direct" => Some(Transport::Direct),
        "tmux" | "tmux_passthrough" => Some(Transport::TmuxPassthrough),
        "file" => Some(Transport::File),
        "memory" | "shm" | "shared" => Some(Transport::Memory),
        _ => None,
    }
}

fn json_u16(value: &serde_json::Value, key: &str) -> Option<Option<u16>> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(v) => v.as_u64().and_then(|n| u16::try_from(n).ok()).map(Some),
    }
}

fn terminal_from_config(value: &serde_json::Value) -> Option<TerminalInfo> {
    let detected = TerminalInfo::detect();
    let transport = value
        .get("transport")
        .and_then(|v| v.as_str())
        .and_then(parse_transport)
        .unwrap_or(detected.transport);
    let width = value
        .get("cell_width_px")
        .and_then(|v| v.as_u64())
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or(detected.cell_size.width_px);
    let height = value
        .get("cell_height_px")
        .and_then(|v| v.as_u64())
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or(detected.cell_size.height_px);
    Some(TerminalInfo::override_with(
        json_u16(value, "columns")?,
        json_u16(value, "rows")?,
        CellSize::new(width, height),
        value
            .get("supports_kitty")
            .and_then(|v| v.as_bool())
            .unwrap_or(detected.supports_kitty),
        value
            .get("supports_unicode_placeholders")
            .and_then(|v| v.as_bool())
            .unwrap_or(detected.supports_unicode_placeholders),
        transport,
    ))
}

/// Free a runtime allocated by [`kittui_runtime_new`].
///
/// # Safety
///
/// The pointer must have been returned by [`kittui_runtime_new`] and not yet
/// freed.
#[no_mangle]
pub unsafe extern "C" fn kittui_runtime_free(runtime: *mut KittuiRuntime) {
    if runtime.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(Box::from_raw(runtime));
    }));
}

/// Render and place a scene. On success, writes a heap-allocated NUL-terminated
/// C string containing the upload+placement+embed bytes into `*out`. The caller
/// must free with [`kittui_string_free`].
///
/// # Safety
///
/// `runtime` must be valid. `scene_json` must be a NUL-terminated UTF-8 string.
/// `out` must point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn kittui_place_json(
    runtime: *mut KittuiRuntime,
    scene_json: *const c_char,
    out: *mut *mut c_char,
) -> KittuiStatus {
    place_json_impl(runtime, scene_json, None, None, out)
}

/// Render/cache a scene JSON document but place it at `(x, y)` terminal cells.
/// The scene's own width/height are preserved for render/cache identity.
///
/// # Safety
///
/// `runtime` must be valid. `scene_json` must be a NUL-terminated UTF-8 string.
/// `out` must point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn kittui_place_json_at(
    runtime: *mut KittuiRuntime,
    scene_json: *const c_char,
    x: u16,
    y: u16,
    out: *mut *mut c_char,
) -> KittuiStatus {
    place_json_impl(runtime, scene_json, Some(x), Some(y), out)
}

/// Render/place a JSON array of scenes in one FFI round-trip. On success,
/// writes a heap-allocated string containing concatenated
/// `upload + placement + embed` bytes for the whole batch.
///
/// # Safety
///
/// `runtime` must be valid. `scenes_json` must be a NUL-terminated UTF-8
/// string containing a JSON array of scenes. `out` must point to writable
/// storage.
#[no_mangle]
pub unsafe extern "C" fn kittui_place_many_json(
    runtime: *mut KittuiRuntime,
    scenes_json: *const c_char,
    out: *mut *mut c_char,
) -> KittuiStatus {
    place_many_json_impl(runtime, scenes_json, None, None, out)
}

/// Render/place a JSON array of scenes in one FFI round-trip with a runtime
/// group origin. The batch's minimum x/y is remapped to `x`/`y` while
/// preserving relative offsets.
///
/// # Safety
///
/// `runtime` must be valid. `scenes_json` must be a NUL-terminated UTF-8
/// string containing a JSON array of scenes. `out` must point to writable
/// storage.
#[no_mangle]
pub unsafe extern "C" fn kittui_place_many_json_at(
    runtime: *mut KittuiRuntime,
    scenes_json: *const c_char,
    x: u16,
    y: u16,
    out: *mut *mut c_char,
) -> KittuiStatus {
    place_many_json_impl(runtime, scenes_json, Some(x), Some(y), out)
}

/// Render/place a JSON array of scenes at a runtime group origin and return a
/// JSON object with separated upload, placement, and embed channels.
///
/// # Safety
///
/// `runtime` must be valid. `scenes_json` must be a NUL-terminated UTF-8
/// string containing a JSON array of scenes. `out` must point to writable
/// storage.
#[no_mangle]
pub unsafe extern "C" fn kittui_place_many_json_channels(
    runtime: *mut KittuiRuntime,
    scenes_json: *const c_char,
    x: u16,
    y: u16,
    out: *mut *mut c_char,
) -> KittuiStatus {
    place_many_json_impl_with(runtime, scenes_json, out, |rt, scenes| {
        let batch = rt.inner.place_batch_at_origin(scenes, x, y)?;
        Ok(batch_json(&batch))
    })
}

unsafe fn place_many_json_impl(
    runtime: *mut KittuiRuntime,
    scenes_json: *const c_char,
    x: Option<u16>,
    y: Option<u16>,
    out: *mut *mut c_char,
) -> KittuiStatus {
    place_many_json_impl_with(runtime, scenes_json, out, |rt, scenes| {
        let batch = match (x, y) {
            (Some(x), Some(y)) => rt.inner.place_batch_at_origin(scenes, x, y),
            _ => rt.inner.place_batch(scenes),
        }?;
        let mut bytes = String::new();
        bytes.push_str(&batch.upload);
        bytes.push_str(&batch.placement);
        bytes.push_str(&batch.embed);
        Ok(bytes)
    })
}

unsafe fn place_many_json_impl_with<F>(
    runtime: *mut KittuiRuntime,
    scenes_json: *const c_char,
    out: *mut *mut c_char,
    f: F,
) -> KittuiStatus
where
    F: FnOnce(&KittuiRuntime, &[Scene]) -> Result<String, kittui::KittuiError>,
{
    if runtime.is_null() || scenes_json.is_null() || out.is_null() {
        return KittuiStatus::NullPointer;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let json = match CStr::from_ptr(scenes_json).to_str() {
            Ok(s) => s,
            Err(_) => return KittuiStatus::BadScene,
        };
        let scenes: Vec<Scene> = match serde_json::from_str(json) {
            Ok(s) => s,
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                return KittuiStatus::BadScene;
            }
        };
        match f(rt, &scenes) {
            Ok(bytes) => match CString::new(bytes) {
                Ok(c) => {
                    *out = c.into_raw();
                    KittuiStatus::Ok
                }
                Err(_) => KittuiStatus::Runtime,
            },
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                KittuiStatus::Runtime
            }
        }
    }));
    result.unwrap_or(KittuiStatus::Panic)
}

fn batch_json(batch: &kittui::BatchPlacement) -> String {
    serde_json::json!({
        "count": batch.image_ids.len(),
        "image_ids": batch.image_ids.iter().map(|id| format!("0x{id:08x}")).collect::<Vec<_>>(),
        "footprints": batch.footprints,
        "upload_bytes": batch.upload.len(),
        "placement_bytes": batch.placement.len(),
        "embed_bytes": batch.embed.len(),
        "upload": batch.upload,
        "placement": batch.placement,
        "embed": batch.embed,
    })
    .to_string()
}

unsafe fn place_json_impl(
    runtime: *mut KittuiRuntime,
    scene_json: *const c_char,
    x: Option<u16>,
    y: Option<u16>,
    out: *mut *mut c_char,
) -> KittuiStatus {
    if runtime.is_null() || scene_json.is_null() || out.is_null() {
        return KittuiStatus::NullPointer;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let json = match CStr::from_ptr(scene_json).to_str() {
            Ok(s) => s,
            Err(_) => return KittuiStatus::BadScene,
        };
        let scene: Scene = match serde_json::from_str(json) {
            Ok(s) => s,
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                return KittuiStatus::BadScene;
            }
        };
        let footprint = kittui::CellRect::new(
            x.unwrap_or(scene.footprint.x),
            y.unwrap_or(scene.footprint.y),
            scene.footprint.cols,
            scene.footprint.rows,
        );
        match rt.inner.place_at(&scene, footprint) {
            Ok(placement) => {
                let bytes = placement.to_bytes();
                match CString::new(bytes) {
                    Ok(c) => {
                        *out = c.into_raw();
                        KittuiStatus::Ok
                    }
                    Err(_) => KittuiStatus::Runtime,
                }
            }
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                KittuiStatus::Runtime
            }
        }
    }));
    result.unwrap_or(KittuiStatus::Panic)
}

/// Free a string returned by the FFI.
///
/// # Safety
///
/// `ptr` must have been returned by an FFI call that allocates strings.
#[no_mangle]
pub unsafe extern "C" fn kittui_string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(CString::from_raw(ptr));
    }));
}

/// Free a byte buffer returned by the FFI alongside a length.
///
/// # Safety
///
/// `ptr` must have been allocated by an FFI call that returns sized buffers.
#[no_mangle]
pub unsafe extern "C" fn kittui_bytes_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // Reconstruct and drop the boxed slice.
        let _ = Vec::from_raw_parts(ptr, len, len);
    }));
}

/// Verify that the loaded library implements the requested ABI major.
/// Returns 1 if compatible, 0 otherwise.
#[no_mangle]
pub extern "C" fn kittui_abi_version_check(required_major: u32) -> i32 {
    if required_major == KITTUI_ABI_MAJOR {
        1
    } else {
        0
    }
}

/// Read the last error string set on this runtime, if any. Returns an owned
/// C string (free via [`kittui_string_free`]) or NULL.
///
/// # Safety
///
/// `runtime` must be a valid pointer returned by [`kittui_runtime_new`].
#[no_mangle]
pub unsafe extern "C" fn kittui_last_error(runtime: *mut KittuiRuntime) -> *mut c_char {
    if runtime.is_null() {
        return ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let guard = rt.last_error.lock().unwrap();
        guard
            .as_ref()
            .and_then(|s| CString::new(s.to_bytes()).ok())
            .map(|c| c.into_raw())
            .unwrap_or(ptr::null_mut())
    }));
    result.unwrap_or(ptr::null_mut())
}

/// Unplace (delete) an image by id. Returns an owned C string with the
/// generated delete escape, or NULL on error.
///
/// # Safety
///
/// `runtime` must be a valid pointer returned by [`kittui_runtime_new`].
#[no_mangle]
pub unsafe extern "C" fn kittui_unplace(runtime: *mut KittuiRuntime, image_id: u32) -> *mut c_char {
    if runtime.is_null() {
        return ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let bytes = rt.inner.unplace(image_id);
        CString::new(bytes)
            .ok()
            .map(|c| c.into_raw())
            .unwrap_or(ptr::null_mut())
    }));
    result.unwrap_or(ptr::null_mut())
}

/// Probe the runtime's current renderer/transport status. Returns an owned
/// JSON C string, or NULL on error.
///
/// # Safety
///
/// `runtime` must be a valid pointer returned by [`kittui_runtime_new`].
#[no_mangle]
pub unsafe extern "C" fn kittui_probe_json(runtime: *mut KittuiRuntime) -> *mut c_char {
    if runtime.is_null() {
        return ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let payload = serde_json::json!({
            "abi_major": KITTUI_ABI_MAJOR,
            "abi_minor": KITTUI_ABI_MINOR,
            "version": env!("CARGO_PKG_VERSION"),
            "renderer": format!("{:?}", rt.inner.renderer_kind()),
            "transport": format!("{:?}", rt.inner.transport()),
        });
        let s = serde_json::to_string(&payload).ok();
        s.and_then(|s| CString::new(s).ok())
            .map(|c| c.into_raw())
            .unwrap_or(ptr::null_mut())
    }));
    result.unwrap_or(ptr::null_mut())
}

/// Configure runtime fields on a live runtime. Accepts a JSON blob with any
/// of `{ "renderer": "cpu"|"gpu"|"auto", "transport": "direct"|"tmux"|"file"|"memory" }`.
///
/// Returns `Ok` on success, or sets `last_error` and returns a non-Ok status.
///
/// # Safety
///
/// `runtime` and `json` must be valid pointers; `json` must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn kittui_runtime_configure(
    runtime: *mut KittuiRuntime,
    json: *const c_char,
) -> KittuiStatus {
    if runtime.is_null() || json.is_null() {
        return KittuiStatus::NullPointer;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &mut *runtime;
        let s = match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return KittuiStatus::BadScene,
        };
        match runtime_from_config_str(s) {
            Ok(runtime) => {
                rt.inner = runtime;
                *rt.last_error.lock().unwrap() = None;
                KittuiStatus::Ok
            }
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e).ok();
                KittuiStatus::BadScene
            }
        }
    }));
    result.unwrap_or(KittuiStatus::Panic)
}

/// Render a scene to PNG bytes with explicit length. Returns Ok
/// and writes `(ptr, len)` for the caller to free via [`kittui_bytes_free`].
///
/// # Safety
///
/// `runtime`, `scene_json`, `out_ptr`, and `out_len` must be valid pointers;
/// `scene_json` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn kittui_render_json(
    runtime: *mut KittuiRuntime,
    scene_json: *const c_char,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> KittuiStatus {
    if runtime.is_null() || scene_json.is_null() || out_ptr.is_null() || out_len.is_null() {
        return KittuiStatus::NullPointer;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let json = match CStr::from_ptr(scene_json).to_str() {
            Ok(s) => s,
            Err(_) => return KittuiStatus::BadScene,
        };
        let scene: Scene = match serde_json::from_str(json) {
            Ok(s) => s,
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                return KittuiStatus::BadScene;
            }
        };
        match rt.inner.render_png(&scene) {
            Ok(bytes) => {
                let mut boxed = bytes.into_boxed_slice();
                *out_len = boxed.len();
                *out_ptr = boxed.as_mut_ptr();
                std::mem::forget(boxed);
                KittuiStatus::Ok
            }
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                KittuiStatus::Runtime
            }
        }
    }));
    result.unwrap_or(KittuiStatus::Panic)
}

/// Render a JSON array of scenes and return a JSON manifest with base64 PNGs.
///
/// # Safety
///
/// `runtime` must be valid. `scenes_json` must be a NUL-terminated JSON array
/// of scenes. `out` must point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn kittui_render_many_json(
    runtime: *mut KittuiRuntime,
    scenes_json: *const c_char,
    out: *mut *mut c_char,
) -> KittuiStatus {
    if runtime.is_null() || scenes_json.is_null() || out.is_null() {
        return KittuiStatus::NullPointer;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = &*runtime;
        let json = match CStr::from_ptr(scenes_json).to_str() {
            Ok(s) => s,
            Err(_) => return KittuiStatus::BadScene,
        };
        let scenes: Vec<Scene> = match serde_json::from_str(json) {
            Ok(s) => s,
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                return KittuiStatus::BadScene;
            }
        };
        match rt.inner.render_many_png(&scenes) {
            Ok(pngs) => {
                let images = scenes
                    .iter()
                    .zip(pngs.iter())
                    .enumerate()
                    .map(|(index, (scene, png))| {
                        serde_json::json!({
                            "index": index,
                            "bytes": png.len(),
                            "footprint": scene.footprint,
                            "png_base64": base64::engine::general_purpose::STANDARD.encode(png),
                        })
                    })
                    .collect::<Vec<_>>();
                let payload = serde_json::json!({
                    "count": images.len(),
                    "images": images,
                })
                .to_string();
                match CString::new(payload) {
                    Ok(c) => {
                        *out = c.into_raw();
                        KittuiStatus::Ok
                    }
                    Err(_) => KittuiStatus::Runtime,
                }
            }
            Err(e) => {
                *rt.last_error.lock().unwrap() = CString::new(e.to_string()).ok();
                KittuiStatus::Runtime
            }
        }
    }));
    result.unwrap_or(KittuiStatus::Panic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "kittui-ffi-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn scene_json() -> CString {
        let scene = kittui::scene::builders::simple_solid_box(2, 1, "#00d8ff");
        CString::new(serde_json::to_string(&scene).unwrap()).unwrap()
    }

    unsafe fn owned_string(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        kittui_string_free(ptr);
        s
    }

    #[test]
    fn abi_version_packs_major_minor() {
        let v = kittui_abi_version();
        assert_eq!(v >> 16, KITTUI_ABI_MAJOR);
        assert_eq!(v & 0xffff, KITTUI_ABI_MINOR);
    }

    #[test]
    fn runtime_new_config_sets_transport_and_places_scene() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true, "columns": 80, "rows": 24}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let probe = owned_string(kittui_probe_json(runtime));
            assert!(probe.contains("\"transport\":\"Direct\""), "{probe}");
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_json(runtime, scene_json().as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::Ok);
            let bytes = owned_string(out);
            assert!(bytes.contains("\x1b_G"), "{bytes:?}");
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn place_many_json_batches_scene_array() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let scene_a: serde_json::Value =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            let scene_b: serde_json::Value =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            let scenes =
                CString::new(serde_json::to_string(&vec![scene_a, scene_b]).unwrap()).unwrap();
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_many_json(runtime, scenes.as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::Ok);
            let bytes = owned_string(out);
            assert!(bytes.contains("\x1b_G"), "{bytes:?}");
            assert!(bytes.len() > 32, "{bytes:?}");
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn place_many_json_at_places_batch_at_origin() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut scene_a: kittui::Scene =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            let mut scene_b: kittui::Scene =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            scene_a.footprint.x = 2;
            scene_a.footprint.y = 4;
            scene_b.footprint.x = 7;
            scene_b.footprint.y = 6;
            let scenes =
                CString::new(serde_json::to_string(&vec![scene_a, scene_b]).unwrap()).unwrap();
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_many_json_at(runtime, scenes.as_ptr(), 10, 20, &mut out);
            assert_eq!(status, KittuiStatus::Ok);
            let bytes = owned_string(out);
            assert!(bytes.contains("\x1b[21;11H"), "{bytes:?}");
            assert!(bytes.contains("\x1b[23;16H"), "{bytes:?}");
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn place_many_json_channels_returns_metadata_and_bytes() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut scene_a: kittui::Scene =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            let mut scene_b: kittui::Scene =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            scene_a.footprint.x = 2;
            scene_a.footprint.y = 4;
            scene_b.footprint.x = 7;
            scene_b.footprint.y = 6;
            let scenes =
                CString::new(serde_json::to_string(&vec![scene_a, scene_b]).unwrap()).unwrap();
            let mut out: *mut c_char = std::ptr::null_mut();
            let status =
                kittui_place_many_json_channels(runtime, scenes.as_ptr(), 10, 20, &mut out);
            assert_eq!(status, KittuiStatus::Ok);
            let json = owned_string(out);
            let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(payload["count"], 2);
            assert_eq!(payload["footprints"][0]["x"], 10);
            assert_eq!(payload["footprints"][1]["x"], 15);
            assert!(payload["upload_bytes"].as_u64().unwrap() > 0);
            assert!(payload["placement"]
                .as_str()
                .unwrap()
                .contains("\x1b[21;11H"));
            assert!(!payload["embed"].as_str().unwrap().is_empty());
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn place_many_json_rejects_non_array_input() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_many_json(runtime, scene_json().as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::BadScene);
            assert!(out.is_null());
            let err = owned_string(kittui_last_error(runtime));
            assert!(
                err.contains("invalid type") || err.contains("sequence"),
                "{err}"
            );
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn place_json_at_overrides_terminal_position() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_json_at(runtime, scene_json().as_ptr(), 5, 6, &mut out);
            assert_eq!(status, KittuiStatus::Ok);
            let bytes = owned_string(out);
            assert!(bytes.contains("\x1b[7;6H"), "{bytes:?}");
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn runtime_configure_rebuilds_live_runtime() {
        let first = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        let second = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "tmux", "supports_kitty": false, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(first.as_ptr());
            assert!(!runtime.is_null());
            let status = kittui_runtime_configure(runtime, second.as_ptr());
            assert_eq!(status, KittuiStatus::Ok);
            let probe = owned_string(kittui_probe_json(runtime));
            assert!(probe.contains("TmuxPassthrough"), "{probe}");
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_json(runtime, scene_json().as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::Runtime);
            assert!(out.is_null());
            let err = owned_string(kittui_last_error(runtime));
            assert!(err.contains("unsupported terminal"), "{err}");
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn runtime_configure_rejects_bad_json() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        let bad = CString::new(r#"{"renderer":"bogus"}"#).unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let status = kittui_runtime_configure(runtime, bad.as_ptr());
            assert_eq!(status, KittuiStatus::BadScene);
            let err = owned_string(kittui_last_error(runtime));
            assert!(err.contains("invalid renderer"), "{err}");
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn render_json_returns_png_bytes_without_terminal_support() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": false, "supports_unicode_placeholders": false}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut out: *mut u8 = std::ptr::null_mut();
            let mut len = 0usize;
            let status = kittui_render_json(runtime, scene_json().as_ptr(), &mut out, &mut len);
            assert_eq!(status, KittuiStatus::Ok);
            assert!(len > 8);
            let bytes = std::slice::from_raw_parts(out, len);
            assert!(
                bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "{:02x?}",
                &bytes[..8]
            );
            assert!(!bytes.windows(2).any(|window| window == b"\x1b_"));
            kittui_bytes_free(out, len);
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn render_many_json_returns_base64_png_manifest() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": false, "supports_unicode_placeholders": false}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let scene_a: serde_json::Value =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            let scene_b: serde_json::Value =
                serde_json::from_str(scene_json().to_str().unwrap()).unwrap();
            let scenes =
                CString::new(serde_json::to_string(&vec![scene_a, scene_b]).unwrap()).unwrap();
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_render_many_json(runtime, scenes.as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::Ok);
            let json = owned_string(out);
            let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(payload["count"], 2);
            let first = payload["images"][0]["png_base64"].as_str().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(first)
                .unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(payload["images"][0]["index"], 0);
            assert!(payload["images"][0]["bytes"].as_u64().unwrap() > 8);
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn render_many_json_rejects_non_array_input() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": true, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_render_many_json(runtime, scene_json().as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::BadScene);
            assert!(out.is_null());
            let err = owned_string(kittui_last_error(runtime));
            assert!(
                err.contains("invalid type") || err.contains("sequence"),
                "{err}"
            );
            kittui_runtime_free(runtime);
        }
    }

    #[test]
    fn runtime_new_config_can_disable_terminal_support() {
        let config = CString::new(format!(
            r#"{{"cache_dir": {:?}, "renderer": "cpu", "transport": "direct", "supports_kitty": false, "supports_unicode_placeholders": true}}"#,
            tempdir().display().to_string()
        ))
        .unwrap();
        unsafe {
            let runtime = kittui_runtime_new_config(config.as_ptr());
            assert!(!runtime.is_null());
            let mut out: *mut c_char = std::ptr::null_mut();
            let status = kittui_place_json(runtime, scene_json().as_ptr(), &mut out);
            assert_eq!(status, KittuiStatus::Runtime);
            assert!(out.is_null());
            let err = owned_string(kittui_last_error(runtime));
            assert!(err.contains("unsupported terminal"), "{err}");
            kittui_runtime_free(runtime);
        }
    }
}
