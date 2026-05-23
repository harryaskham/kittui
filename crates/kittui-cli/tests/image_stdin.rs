use std::io::Write;
use std::process::{Command, Stdio};

fn kittui_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kittui")
}

fn tiny_png() -> &'static [u8] {
    // 1x1 transparent PNG.
    &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, b'I',
        b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ]
}

#[test]
fn image_reads_png_bytes_from_stdin() {
    let mut child = Command::new(kittui_bin())
        .args([
            "image",
            "--src",
            "-",
            "-w",
            "1",
            "-h",
            "1",
            "--dry-run",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kittui image --src -");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(tiny_png())
        .expect("write png stdin");
    let output = child.wait_with_output().expect("wait for image command");
    assert!(
        output.status.success(),
        "image stdin failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["footprint"]["cols"], 1);
    assert_eq!(payload["footprint"]["rows"], 1);
    assert!(payload["upload_bytes"].as_u64().unwrap() > 0);
}
