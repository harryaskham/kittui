use std::io::Write;
use std::process::{Command, Stdio};

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn box_scene_json_pipes_into_compose_stdin() {
    let scene = Command::new(kittui_bin())
        .args(["box", "-w", "4", "-h", "2", "--scene-json"])
        .output()
        .expect("run kittui box --scene-json");
    assert!(
        scene.status.success(),
        "box failed: {}",
        String::from_utf8_lossy(&scene.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&scene.stdout).expect("valid scene json");
    assert_eq!(parsed["footprint"]["cols"], 4);
    assert_eq!(parsed["footprint"]["rows"], 2);

    let mut compose = Command::new(kittui_bin())
        .args(["compose", "-", "--dry-run", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui compose -");
    compose
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&scene.stdout)
        .expect("write scene to stdin");
    let composed = compose.wait_with_output().expect("wait for compose");
    assert!(
        composed.status.success(),
        "compose failed: {}",
        String::from_utf8_lossy(&composed.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&composed.stdout).expect("valid dry-run json");
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["footprint"]["cols"], 4);
    assert_eq!(payload["footprint"]["rows"], 2);
}
