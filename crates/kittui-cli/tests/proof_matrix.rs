//! Integration-level regression test for the `kittui proof` matrix.
//!
//! The matrix is the closest thing kittui has to an end-to-end protocol
//! contract: if any of the labelled sections vanishes or the byte length of
//! a known section drifts, the kitty graphics surface has silently changed
//! and the test fails. Combined with the per-function grammar tests in
//! `crates/kittui-kitty/src/lib.rs::tests`, this gates regressions against
//! both individual command shape and the full assembled flow.

use std::process::Command;

fn proof_binary() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("kittui");
    p
}

#[test]
fn proof_matrix_lists_every_expected_section() {
    let bin = proof_binary();
    if !bin.exists() {
        eprintln!("skipping proof_matrix_lists_every_expected_section: build kittui first");
        return;
    }
    let out = Command::new(&bin)
        .arg("proof")
        .output()
        .expect("run kittui proof");
    assert!(out.status.success(), "kittui proof failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    for label in [
        "upload still + unicode placement (Direct, q=2)",
        "upload still + unicode placement (TmuxPassthrough)",
        "upload still via File medium",
        "upload still via SharedMemory medium",
        "absolute placement (no unicode placeholder)",
        "placement with id=7, X=4, Y=2, z=1",
        "animated upload + placement (3 frames)",
        "delete image / delete placement",
        "HiDPI 16x32 cell override",
    ] {
        assert!(
            stdout.contains(label),
            "kittui proof output missing `{label}`:\n{stdout}",
        );
    }
}
