use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn test_cache_dir(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "kittui-compose-batch-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create isolated kittui cache dir");
    dir
}

fn scene_json(cols: u16, rows: u16, cache_dir: &Path) -> Vec<u8> {
    let output = Command::new(kittui_bin())
        .args([
            "box",
            "-w",
            &cols.to_string(),
            "-h",
            &rows.to_string(),
            "--scene-json",
        ])
        .env("KITTUI_CACHE_DIR", cache_dir)
        .output()
        .expect("run kittui box --scene-json");
    assert!(
        output.status.success(),
        "scene failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn compose_stdin_accepts_scene_arrays_for_dry_run_json() {
    let cache_dir = test_cache_dir("dry-run");
    let a: serde_json::Value = serde_json::from_slice(&scene_json(2, 1, &cache_dir)).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&scene_json(3, 1, &cache_dir)).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a, b])).unwrap();
    let mut compose = Command::new(kittui_bin())
        .args(["compose", "-", "--dry-run", "--json"])
        .env("KITTUI_CACHE_DIR", &cache_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui compose batch");
    compose.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = compose.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "compose batch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["count"], 2);
    assert_eq!(payload["footprints"].as_array().unwrap().len(), 2);
    assert!(payload["upload_bytes"].as_u64().unwrap() > 0);
}

#[test]
fn compose_batch_accepts_placement_origin_overrides() {
    let cache_dir = test_cache_dir("origin-overrides");
    let mut a: serde_json::Value = serde_json::from_slice(&scene_json(2, 1, &cache_dir)).unwrap();
    let mut b: serde_json::Value = serde_json::from_slice(&scene_json(3, 1, &cache_dir)).unwrap();
    a["footprint"]["x"] = serde_json::json!(2);
    a["footprint"]["y"] = serde_json::json!(4);
    b["footprint"]["x"] = serde_json::json!(7);
    b["footprint"]["y"] = serde_json::json!(6);
    let batch = serde_json::to_vec(&serde_json::json!([a, b])).unwrap();
    let mut compose = Command::new(kittui_bin())
        .args([
            "compose",
            "-",
            "--x",
            "10",
            "--y",
            "20",
            "--dry-run",
            "--json",
        ])
        .env("KITTUI_CACHE_DIR", &cache_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui compose batch with override");
    compose.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = compose.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "compose batch override failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let footprints = payload["footprints"].as_array().unwrap();
    assert_eq!(footprints[0]["x"], 10);
    assert_eq!(footprints[0]["y"], 20);
    assert_eq!(footprints[1]["x"], 15);
    assert_eq!(footprints[1]["y"], 22);
}
