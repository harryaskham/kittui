use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn scene_for(args: &[&str]) -> serde_json::Value {
    let output = Command::new(kittui_bin())
        .args(args)
        .output()
        .expect("run kittui affordance command");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn chip_scene_json_uses_requested_footprint() {
    let scene = scene_for(&[
        "chip",
        "-w",
        "10",
        "--bg",
        "#001122",
        "--border",
        "#00d8ff",
        "--scene-json",
    ]);
    assert_eq!(scene["footprint"]["cols"], 10);
    assert_eq!(scene["footprint"]["rows"], 1);
    assert!(scene["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| layer["label"] == "border"));
}

#[test]
fn divider_and_title_bar_emit_background_layers() {
    let divider = scene_for(&[
        "divider",
        "-w",
        "12",
        "--left",
        "#001122",
        "--right",
        "#00d8ff",
        "--scene-json",
    ]);
    assert_eq!(divider["footprint"]["rows"], 1);
    assert!(divider["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| layer["label"] == "background"));

    let title = scene_for(&[
        "title-bar",
        "-w",
        "14",
        "-h",
        "2",
        "--left",
        "#001122",
        "--right",
        "#00d8ff",
        "--scene-json",
    ]);
    assert_eq!(title["footprint"]["cols"], 14);
    assert_eq!(title["footprint"]["rows"], 2);
    assert!(title["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| layer["label"] == "background"));
}
