//! ABI snapshot check.
//!
//! Asserts that the symbols listed in the committed `kittui.h` header
//! are all exported by the cdylib. Catches accidental ABI breaks: any
//! removal or rename trips this test. Additive changes (new symbols)
//! pass — they should be reflected in `kittui.h` in the same commit,
//! and the minor version bumped.

use std::fs;
use std::path::Path;

#[test]
fn header_lists_only_exported_symbols() {
    let header =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("kittui.h")).unwrap();
    let expected = [
        "kittui_abi_version",
        "kittui_abi_version_check",
        "kittui_runtime_new",
        "kittui_runtime_new_config",
        "kittui_runtime_free",
        "kittui_runtime_configure",
        "kittui_place_json",
        "kittui_place_json_at",
        "kittui_place_many_json",
        "kittui_place_many_json_at",
        "kittui_place_many_json_channels",
        "kittui_render_json",
        "kittui_unplace",
        "kittui_probe_json",
        "kittui_last_error",
        "kittui_string_free",
        "kittui_bytes_free",
    ];
    for sym in expected {
        assert!(
            header.contains(sym),
            "kittui.h is missing declaration of {sym}; ABI snapshot is out of sync"
        );
    }
}

#[test]
fn abi_version_constants_match_header() {
    let header =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("kittui.h")).unwrap();
    assert!(header.contains("#define KITTUI_ABI_MAJOR 0"));
    assert!(header.contains("#define KITTUI_ABI_MINOR 7"));
    assert_eq!(kittui_ffi::KITTUI_ABI_MAJOR, 0);
    assert_eq!(kittui_ffi::KITTUI_ABI_MINOR, 7);
}

#[test]
fn ffi_round_trip_place_render_unplace_and_probe() {
    use std::ffi::{CStr, CString};
    use std::ptr;

    let cache_dir = std::env::temp_dir().join(format!(
        "kittui-ffi-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&cache_dir).unwrap();
    let c_cache = CString::new(cache_dir.to_string_lossy().as_bytes()).unwrap();

    unsafe {
        // abi check
        assert_eq!(kittui_ffi::kittui_abi_version_check(0), 1);
        assert_eq!(kittui_ffi::kittui_abi_version_check(99), 0);

        // construct runtime
        let rt = kittui_ffi::kittui_runtime_new(c_cache.as_ptr());
        assert!(!rt.is_null());

        // probe -> JSON
        let probe = kittui_ffi::kittui_probe_json(rt);
        assert!(!probe.is_null());
        let probe_s = CStr::from_ptr(probe).to_str().unwrap().to_owned();
        assert!(probe_s.contains("abi_major"));
        kittui_ffi::kittui_string_free(probe);

        // place a tiny scene via place_json
        let scene = serde_json::json!({
            "footprint": { "x": 0, "y": 0, "cols": 2, "rows": 1 },
            "cell_size": { "width_px": 8, "height_px": 16 },
            "layers": [{
                "label": null,
                "root": {
                    "kind": "rect",
                    "rect": { "origin": [0.0, 0.0], "width": 16.0, "height": 16.0 },
                    "fill": { "kind": "solid", "color": [0, 216, 255, 255] },
                    "stroke": null,
                    "corners": { "tl": 0.0, "tr": 0.0, "bl": 0.0, "br": 0.0 }
                }
            }],
            "animation": null
        });
        let scene_c = CString::new(scene.to_string()).unwrap();
        let mut out: *mut std::ffi::c_char = ptr::null_mut();
        let status = kittui_ffi::kittui_place_json(rt, scene_c.as_ptr(), &mut out);
        assert_eq!(status as i32, kittui_ffi::KittuiStatus::Ok as i32);
        assert!(!out.is_null());
        kittui_ffi::kittui_string_free(out);

        // render_json with explicit length
        let mut buf: *mut u8 = ptr::null_mut();
        let mut len: usize = 0;
        let st2 = kittui_ffi::kittui_render_json(rt, scene_c.as_ptr(), &mut buf, &mut len);
        assert_eq!(st2 as i32, kittui_ffi::KittuiStatus::Ok as i32);
        assert!(!buf.is_null());
        assert!(len > 0);
        kittui_ffi::kittui_bytes_free(buf, len);

        // unplace
        let unp = kittui_ffi::kittui_unplace(rt, 0x12345678);
        assert!(!unp.is_null());
        kittui_ffi::kittui_string_free(unp);

        // last_error should be NULL after success
        let err = kittui_ffi::kittui_last_error(rt);
        if !err.is_null() {
            kittui_ffi::kittui_string_free(err);
        }

        kittui_ffi::kittui_runtime_free(rt);
    }
}
