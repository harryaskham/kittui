use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn panel_scene_json_emits_valid_scene() {
    let output = Command::new(kittui_bin())
        .args([
            "panel",
            "--tone",
            "assistant",
            "-w",
            "20",
            "-h",
            "4",
            "--scene-json",
        ])
        .output()
        .expect("run kittui panel --scene-json");
    assert!(
        output.status.success(),
        "panel failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let scene: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(scene["footprint"]["cols"], 20);
    assert_eq!(scene["footprint"]["rows"], 4);
    assert!(scene["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| layer["label"] == "background"));
}

#[test]
fn panel_animate_includes_animation_metadata() {
    let output = Command::new(kittui_bin())
        .args([
            "panel",
            "--tone",
            "tool",
            "-w",
            "12",
            "-h",
            "3",
            "--animate",
            "--scene-json",
        ])
        .output()
        .expect("run animated kittui panel --scene-json");
    assert!(
        output.status.success(),
        "animated panel failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let scene: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(scene["animation"]["frames"], 180);
    assert_eq!(scene["animation"]["cycle_ms"], 3000);
    assert!(scene["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| layer["label"] == "affordance-panel-animation"));
}
