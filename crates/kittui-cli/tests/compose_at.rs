use std::io::Write;
use std::process::{Command, Stdio};

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

#[test]
fn compose_stdin_can_override_terminal_position() {
    let scene = Command::new(kittui_bin())
        .args(["box", "-w", "4", "-h", "2", "--scene-json"])
        .output()
        .expect("run kittui box --scene-json");
    assert!(
        scene.status.success(),
        "scene failed: {}",
        String::from_utf8_lossy(&scene.stderr)
    );

    let mut compose = Command::new(kittui_bin())
        .args([
            "compose",
            "-",
            "--x",
            "5",
            "--y",
            "6",
            "--dry-run",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui compose - --x --y");
    compose
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&scene.stdout)
        .unwrap();
    let output = compose.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "compose failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["footprint"]["x"], 5);
    assert_eq!(payload["footprint"]["y"], 6);
    assert_eq!(payload["footprint"]["cols"], 4);
    assert_eq!(payload["footprint"]["rows"], 2);
}
