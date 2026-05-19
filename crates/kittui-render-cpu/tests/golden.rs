//! Golden snapshot harness for the CPU renderer.
//!
//! Each fixture is a (scene_json, expected.png) pair under
//! `tests/golden/`. The test renders the scene through the CPU oracle
//! and asserts byte equality against the committed PNG. Run with
//! `KITTUI_REFRESH_GOLDENS=1` to overwrite the committed PNG with the
//! freshly produced bytes (used after intentional renderer changes).
//!
//! Goldens are intentionally tiny (a few cells) so the committed bytes
//! stay small.

use std::fs;
use std::path::{Path, PathBuf};

use kittui_core::Scene;
use kittui_render_cpu::render_still;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn list_fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let dir = fixtures_dir();
    if !dir.is_dir() {
        return out;
    }
    for entry in fs::read_dir(&dir).expect("read goldens dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn refresh_mode() -> bool {
    std::env::var("KITTUI_REFRESH_GOLDENS")
        .ok()
        .filter(|v| !v.is_empty())
        .is_some()
}

#[test]
fn cpu_renderer_matches_committed_goldens() {
    let mut checked = 0usize;
    let mut mismatched: Vec<String> = Vec::new();
    for json_path in list_fixtures() {
        let png_path = json_path.with_extension("png");
        let scene: Scene = serde_json::from_slice(&fs::read(&json_path).unwrap())
            .unwrap_or_else(|e| panic!("parse {}: {e}", json_path.display()));
        let frame = render_still(&scene).expect("render");
        if refresh_mode() || !png_path.exists() {
            fs::write(&png_path, &frame.png).unwrap();
            checked += 1;
            continue;
        }
        let expected = fs::read(&png_path).unwrap();
        if expected != frame.png {
            mismatched.push(format!(
                "{}: produced {} bytes, expected {} bytes",
                png_path.display(),
                frame.png.len(),
                expected.len()
            ));
        }
        checked += 1;
    }
    assert!(checked > 0, "no golden fixtures found under tests/golden/");
    assert!(
        mismatched.is_empty(),
        "golden mismatches (set KITTUI_REFRESH_GOLDENS=1 to refresh): {:#?}",
        mismatched
    );
}
