use std::io::Write;
use std::process::{Command, Stdio};

fn base64_decode(input: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u8;
    for byte in input.bytes().filter(|b| *b != b'=') {
        let val = TABLE.iter().position(|v| *v == byte).expect("base64 char") as u32;
        buf = (buf << 6) | val;
        bits += 6;
        while bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    out
}

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn temp_path(name: &str, ext: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{name}-{}-{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        ext
    ))
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

fn animated_scene_json() -> Vec<u8> {
    let output = Command::new(kittui_bin())
        .args([
            "box",
            "-w",
            "4",
            "-h",
            "2",
            "--animated",
            "--frames",
            "3",
            "--fps",
            "3",
            "--scene-json",
        ])
        .output()
        .expect("run animated kittui box --scene-json");
    assert!(
        output.status.success(),
        "animated scene failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn render_stdin_writes_png_file() {
    let path = temp_path("kittui-render-command", "png");
    let manifest_path = temp_path("kittui-render-command", "json");
    let mut render = Command::new(kittui_bin())
        .args([
            "render",
            "-",
            "--out",
            path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
        ])
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
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&manifest_path);
    assert_eq!(manifest["output"], path.display().to_string());
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

#[test]
fn render_single_animated_scene_writes_frame_directory() {
    let out_dir = temp_path("kittui-render-animation", "dir");
    let manifest_path = out_dir.join("manifest.json");
    let mut render = Command::new(kittui_bin())
        .args([
            "render",
            "-",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render animation");
    render
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&animated_scene_json())
        .unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "render animation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for idx in 0..3 {
        let png = std::fs::read(out_dir.join(format!("frame-{idx:05}.png"))).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["frames"], 3);
    assert_eq!(manifest["files"][0]["delay_ms"], 333);
    assert!(manifest["files"][0]["output"]
        .as_str()
        .unwrap()
        .ends_with("frame-00000.png"));
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn render_scene_array_writes_png_directory() {
    let out_dir = temp_path("kittui-render-batch", "dir");
    let manifest_path = out_dir.join("manifest.json");
    let a: serde_json::Value = serde_json::from_slice(&scene_json()).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&scene_json()).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a, b])).unwrap();
    let mut render = Command::new(kittui_bin())
        .args([
            "render",
            "-",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render batch");
    render.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "render batch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for idx in 0..2 {
        let png = std::fs::read(out_dir.join(format!("scene-{idx:05}.png"))).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!png.windows(2).any(|window| window == b"\x1b_"));
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["count"], 2);
    assert!(manifest["files"][0]["output"]
        .as_str()
        .unwrap()
        .ends_with("scene-00000.png"));
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn render_batch_dry_run_json_bytes_include_png_base64() {
    let out_dir = temp_path("kittui-render-batch-json-bytes", "dir");
    let a: serde_json::Value = serde_json::from_slice(&scene_json()).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a])).unwrap();
    let mut render = Command::new(kittui_bin())
        .args([
            "--json",
            "--json-bytes",
            "--dry-run",
            "render",
            "-",
            "--out-dir",
            out_dir.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render batch json bytes");
    render.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "render batch json bytes failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let encoded = payload["files"][0]["png_base64"].as_str().unwrap();
    let decoded = base64_decode(encoded);
    assert!(decoded.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(!out_dir.exists(), "dry-run should not create output dir");
}

#[test]
fn render_batch_dry_run_json_reports_manifest() {
    let out_dir = temp_path("kittui-render-batch-json", "dir");
    let a: serde_json::Value = serde_json::from_slice(&scene_json()).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a])).unwrap();
    let mut render = Command::new(kittui_bin())
        .args([
            "--json",
            "--dry-run",
            "render",
            "-",
            "--out-dir",
            out_dir.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render batch json");
    render.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "render batch json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["count"], 1);
    assert_eq!(payload["files"][0]["index"], 0);
    assert!(payload["files"][0]["output"]
        .as_str()
        .unwrap()
        .ends_with("scene-00000.png"));
    assert!(!out_dir.exists(), "dry-run should not create output dir");
}

#[test]
fn render_scene_array_requires_out_dir() {
    let a: serde_json::Value = serde_json::from_slice(&scene_json()).unwrap();
    let batch = serde_json::to_vec(&serde_json::json!([a])).unwrap();
    let mut render = Command::new(kittui_bin())
        .args(["render", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui render batch without out-dir");
    render.stdin.as_mut().unwrap().write_all(&batch).unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--out-dir"));
}
