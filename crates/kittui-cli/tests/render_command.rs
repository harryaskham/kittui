use std::io::Write;
use std::process::{Command, Stdio};

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn scene_json() -> Vec<u8> {
    let output = Command::new(kittui_bin())
        .args(["box", "-w", "4", "-h", "2", "--scene-json"])
        .output()
        .expect("run kittui box --scene-json");
    assert!(
        output.status.success(),
        "scene failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn render_stdin_writes_png_file() {
    let path = std::env::temp_dir().join(format!(
        "kittui-render-command-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut render = Command::new(kittui_bin())
        .args(["render", "-", "--out", path.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render");
    render
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&scene_json())
        .unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let png = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(!png.windows(2).any(|window| window == b"\x1b_"));
}

#[test]
fn render_json_reports_metadata_without_writing_on_dry_run() {
    let mut render = Command::new(kittui_bin())
        .args(["--json", "--dry-run", "render", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render json");
    render
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&scene_json())
        .unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "render json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dry_run"], true);
    assert!(payload["bytes"].as_u64().unwrap() > 8);
    assert_eq!(payload["footprint"]["cols"], 4);
}
