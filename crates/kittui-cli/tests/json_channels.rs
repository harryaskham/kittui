use std::process::Command;

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn json_bytes_exposes_channel_strings_and_compact_json_omits_them() {
    let compact = Command::new(kittui_bin())
        .args(["box", "-w", "2", "-h", "1", "--json"])
        .output()
        .expect("run compact json");
    assert!(
        compact.status.success(),
        "compact failed: {}",
        String::from_utf8_lossy(&compact.stderr)
    );
    let compact_payload: serde_json::Value = serde_json::from_slice(&compact.stdout).unwrap();
    assert!(compact_payload.get("upload").is_none());
    assert!(compact_payload.get("placement").is_none());
    assert!(compact_payload["embed"]
        .as_str()
        .unwrap()
        .contains('\u{10EEEE}'));

    let verbose = Command::new(kittui_bin())
        .args(["box", "-w", "2", "-h", "1", "--json", "--json-bytes"])
        .output()
        .expect("run json bytes");
    assert!(
        verbose.status.success(),
        "verbose failed: {}",
        String::from_utf8_lossy(&verbose.stderr)
    );
    let verbose_payload: serde_json::Value = serde_json::from_slice(&verbose.stdout).unwrap();
    assert!(verbose_payload["upload"]
        .as_str()
        .unwrap()
        .contains("\u{1b}_G"));
    assert!(verbose_payload["placement"]
        .as_str()
        .unwrap()
        .contains("\u{1b}["));
    assert!(verbose_payload["embed"]
        .as_str()
        .unwrap()
        .contains('\u{10EEEE}'));
    assert_eq!(
        verbose_payload["upload_bytes"],
        verbose_payload["upload"].as_str().unwrap().len()
    );
    assert_eq!(
        verbose_payload["placement_bytes"],
        verbose_payload["placement"].as_str().unwrap().len()
    );
}
