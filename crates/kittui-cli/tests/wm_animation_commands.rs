use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn wm_chrome_animated_scene_json_uses_standard_defaults() {
    let output = Command::new(kittui_bin())
        .args([
            "wm-chrome",
            "-w",
            "20",
            "-h",
            "4",
            "--title",
            "logs",
            "--focused",
            "--animated",
            "--scene-json",
        ])
        .output()
        .expect("run kittui wm-chrome --animated --scene-json");
    assert!(
        output.status.success(),
        "wm-chrome failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let scene: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(scene["footprint"]["cols"], 20);
    assert_eq!(scene["footprint"]["rows"], 4);
    assert_eq!(scene["animation"]["frames"], 180);
    assert_eq!(scene["animation"]["cycle_ms"], 3000);
    assert!(scene["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|layer| layer["label"] == "wm-chrome-animation"));
}

#[test]
fn wm_chrome_animated_scene_json_honors_fps_and_frames() {
    let output = Command::new(kittui_bin())
        .args([
            "wm-chrome",
            "-w",
            "18",
            "-h",
            "3",
            "--animated",
            "--fps",
            "30",
            "--frames",
            "90",
            "--scene-json",
        ])
        .output()
        .expect("run kittui wm-chrome custom animation --scene-json");
    assert!(
        output.status.success(),
        "wm-chrome custom animation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let scene: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(scene["animation"]["frames"], 90);
    assert_eq!(scene["animation"]["cycle_ms"], 3000);
}

#[test]
fn wm_session_animated_scene_json_batch_labels_each_scene() {
    let manifest_path = std::env::temp_dir().join(format!(
        "kittui-wm-session-animation-{}-{}.json",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    std::fs::write(
        &manifest_path,
        r#"{
            "layout": "columns",
            "panes": [
                {"title": "shell", "focused": true},
                {"title": "logs", "floating": true}
            ]
        }"#,
    )
    .unwrap();

    let output = Command::new(kittui_bin())
        .arg("wm-session")
        .arg(&manifest_path)
        .args(["-w", "30", "-h", "6", "--animated", "--scene-json"])
        .output()
        .expect("run kittui wm-session --animated --scene-json");
    let _ = std::fs::remove_file(&manifest_path);
    assert!(
        output.status.success(),
        "wm-session failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let scenes: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let scenes = scenes.as_array().unwrap();
    assert_eq!(scenes.len(), 2);
    for scene in scenes {
        assert_eq!(scene["animation"]["frames"], 180);
        assert_eq!(scene["animation"]["cycle_ms"], 3000);
        assert!(scene["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["label"] == "wm-session-animation"));
    }
}
