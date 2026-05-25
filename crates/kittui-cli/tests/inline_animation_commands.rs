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
fn top_level_affordance_scene_json_reports_animation_contract() {
    let chip = scene_for(&[
        "chip",
        "-w",
        "10",
        "--bg",
        "#001122",
        "--border",
        "#00d8ff",
        "--animated",
        "--scene-json",
    ]);
    assert_eq!(chip["animation"]["frames"], 180);
    assert_eq!(chip["animation"]["cycle_ms"], 3000);
    assert!(chip["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "affordance-chip-animation" }));

    let divider = scene_for(&[
        "divider",
        "-w",
        "12",
        "--left",
        "#001122",
        "--right",
        "#00d8ff",
        "--animated",
        "--fps",
        "30",
        "--frames",
        "90",
        "--scene-json",
    ]);
    assert_eq!(divider["animation"]["frames"], 90);
    assert_eq!(divider["animation"]["cycle_ms"], 3000);
    assert!(divider["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "affordance-divider-animation" }));
}

#[test]
fn top_level_panel_and_title_bar_scene_json_report_animation_contract() {
    let panel = scene_for(&["panel", "-w", "12", "-h", "3", "--animated", "--scene-json"]);
    assert_eq!(panel["animation"]["frames"], 180);
    assert!(panel["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "affordance-panel-animation" }));

    let title = scene_for(&[
        "title-bar",
        "-w",
        "12",
        "-h",
        "1",
        "--left",
        "#001122",
        "--right",
        "#00d8ff",
        "--animated",
        "--scene-json",
    ]);
    assert_eq!(title["animation"]["cycle_ms"], 3000);
    assert!(title["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "affordance-title-bar-animation" }));
}

#[test]
fn primitive_scene_json_reports_animation_contract() {
    let bx = scene_for(&["box", "-w", "8", "-h", "2", "--animated", "--scene-json"]);
    assert_eq!(bx["animation"]["frames"], 180);
    assert!(bx["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "primitive-box-animation" }));

    let gradient = scene_for(&[
        "gradient",
        "-w",
        "8",
        "--animated",
        "--fps",
        "20",
        "--frames",
        "60",
        "--scene-json",
    ]);
    assert_eq!(gradient["animation"]["frames"], 60);
    assert_eq!(gradient["animation"]["cycle_ms"], 3000);
    assert!(gradient["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "primitive-gradient-animation" }));

    let glow = scene_for(&["glow", "-w", "8", "-h", "2", "--animated", "--scene-json"]);
    assert_eq!(glow["animation"]["cycle_ms"], 3000);
    assert!(glow["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| { layer["label"] == "primitive-glow-animation" }));
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
