use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn scene_for(args: &[&str]) -> serde_json::Value {
    let output = Command::new(kittui_bin())
        .args(args)
        .output()
        .expect("run kittui inline animation command");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn animated_inline_chip_scene_json_reports_loop_and_effect_layer() {
    let scene = scene_for(&[
        "inline",
        "chip",
        "--text",
        "main",
        "--style",
        "glass",
        "--animated",
        "--scene-json",
    ]);
    assert_eq!(scene["animation"]["frames"], 180);
    assert_eq!(scene["animation"]["cycle_ms"], 3000);
    assert_eq!(scene["animation"]["loops"], 0);
    assert!(scene["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "inline-effect-glass-glare" }));
}

#[test]
fn animated_inline_row_scene_json_reports_effect_layer() {
    let scene = scene_for(&[
        "inline",
        "row",
        "--item",
        "chip:main",
        "--item",
        "divider:4",
        "--style",
        "neon",
        "--animated",
        "--fps",
        "30",
        "--frames",
        "90",
        "--scene-json",
    ]);
    assert_eq!(scene["animation"]["frames"], 90);
    assert_eq!(scene["animation"]["cycle_ms"], 3000);
    assert!(scene["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "inline-effect-neon-pulse" }));
}
