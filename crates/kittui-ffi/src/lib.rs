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

use kittui::{RendererKind, Runtime, Scene};

/// Major version of the FFI ABI. Bumped on any breaking change.
pub const KITTUI_ABI_MAJOR: u32 = 0;
/// Minor version of the FFI ABI. Bumped on additive changes.
pub const KITTUI_ABI_MINOR: u32 = 1;

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
        match builder.build() {
            Ok(runtime) => Box::into_raw(Box::new(KittuiRuntime {
                inner: runtime,
                last_error: Mutex::new(None),
            })),
            Err(_) => ptr::null_mut(),
        }
    }));
    result.unwrap_or(ptr::null_mut())
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
        match rt.inner.place(&scene) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_packs_major_minor() {
        let v = kittui_abi_version();
        assert_eq!(v >> 16, KITTUI_ABI_MAJOR);
        assert_eq!(v & 0xffff, KITTUI_ABI_MINOR);
    }
}
