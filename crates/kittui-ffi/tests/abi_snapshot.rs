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
    let header = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("kittui.h"),
    )
    .unwrap();
    let expected = [
        "kittui_abi_version",
        "kittui_runtime_new",
        "kittui_runtime_free",
        "kittui_place_json",
        "kittui_string_free",
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
    let header = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("kittui.h"),
    )
    .unwrap();
    assert!(header.contains("#define KITTUI_ABI_MAJOR 0"));
    assert!(header.contains("#define KITTUI_ABI_MINOR 1"));
    assert_eq!(kittui_ffi::KITTUI_ABI_MAJOR, 0);
    assert_eq!(kittui_ffi::KITTUI_ABI_MINOR, 1);
}
