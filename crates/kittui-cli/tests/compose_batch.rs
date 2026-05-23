use std::io::Write;
use std::process::{Command, Stdio};

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn scene_json(cols: u16, rows: u16) -> Vec<u8> {
    let output = Command::new(kittui_bin())
        .args([
            "box",
            "-w",
            &cols.to_string(),
            "-h",
            &rows.to_string(),
            "--scene-json",
        ])
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
    let a: serde_json::Value = serde_json::from_slice(&scene_json(2, 1)).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&scene_json(3, 1)).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a, b])).unwrap();
    let mut compose = Command::new(kittui_bin())
        .args(["compose", "-", "--dry-run", "--json"])
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
fn compose_batch_rejects_placement_overrides() {
    let a: serde_json::Value = serde_json::from_slice(&scene_json(2, 1)).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a])).unwrap();
    let mut compose = Command::new(kittui_bin())
        .args(["compose", "-", "--x", "5", "--dry-run", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui compose batch with override");
    compose.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = compose.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only supported for single Scene"));
}
