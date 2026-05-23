use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn delete_command_emits_image_delete_json_bytes() {
    let output = Command::new(kittui_bin())
        .args(["delete", "--id", "0x1234", "--json", "--json-bytes"])
        .output()
        .expect("run kittui delete");
    assert!(
        output.status.success(),
        "delete failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["image_id"], "0x00001234");
    assert!(payload["placement_id"].is_null());
    assert!(payload["delete"].as_str().unwrap().contains("\u{1b}_Ga=d"));
    assert!(payload["delete"].as_str().unwrap().contains("i=4660"));
}

#[test]
fn delete_command_can_target_one_placement() {
    let output = Command::new(kittui_bin())
        .args([
            "delete",
            "--id",
            "4660",
            "--placement-id",
            "7",
            "--json",
            "--json-bytes",
        ])
        .output()
        .expect("run kittui delete placement");
    assert!(
        output.status.success(),
        "delete placement failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["image_id"], "0x00001234");
    assert_eq!(payload["placement_id"], 7);
    let delete = payload["delete"].as_str().unwrap();
    assert!(delete.contains("i=4660"), "{delete:?}");
    assert!(delete.contains("p=7"), "{delete:?}");
}
