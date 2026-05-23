use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn place_command_reemits_existing_image_id_as_json_channels() {
    let output = Command::new(kittui_bin())
        .args([
            "place",
            "--id",
            "0x1234",
            "--x",
            "2",
            "--y",
            "3",
            "--cols",
            "4",
            "--rows",
            "2",
            "--json",
            "--json-bytes",
        ])
        .output()
        .expect("run kittui place");
    assert!(
        output.status.success(),
        "place failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["image_id"], "0x00001234");
    assert_eq!(payload["upload"], "");
    assert_eq!(payload["upload_bytes"], 0);
    assert_eq!(payload["footprint"]["x"], 2);
    assert_eq!(payload["footprint"]["y"], 3);
    assert_eq!(payload["footprint"]["cols"], 4);
    assert_eq!(payload["footprint"]["rows"], 2);
    assert!(payload["placement"]
        .as_str()
        .unwrap()
        .contains("\u{1b}[4;3H"));
    assert!(payload["embed"].as_str().unwrap().contains('\u{10EEEE}'));
}

#[test]
fn place_command_can_emit_only_embed_channel() {
    let output = Command::new(kittui_bin())
        .args([
            "place",
            "--id",
            "4660",
            "--x",
            "0",
            "--y",
            "0",
            "--cols",
            "1",
            "--rows",
            "1",
            "--embed-only",
        ])
        .output()
        .expect("run kittui place --embed-only");
    assert!(
        output.status.success(),
        "place embed-only failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains('\u{10EEEE}'), "{text:?}");
    assert!(!text.contains("\u{1b}_G"), "{text:?}");
}
