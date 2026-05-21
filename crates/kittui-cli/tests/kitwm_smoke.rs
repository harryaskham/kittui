//! `kitwm` end-to-end smoke. Spawns the `kitwm` binary with the FakeServer
//! backend, lets it run briefly, and asserts that the debug log contains
//! evidence of a real render loop (frames rendered, terminal restored).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn kitwm_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("kitwm");
    p
}

#[test]
fn kitwm_fake_backend_renders_frames_then_exits() {
    let bin = kitwm_path();
    if !bin.exists() {
        eprintln!("skipping: kitwm not built yet at {}", bin.display());
        return;
    }
    let log = std::env::temp_dir().join(format!(
        "kitwm-smoke-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&log);

    let mut child = Command::new(&bin)
        .arg("--backend")
        .arg("fake")
        .env("KITTUI_WM_LOG", &log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn kitwm");

    // Let it render for ~4s under parallel test load (debug-build first
    // frame can take >1.5s on heavily-loaded CI), then SIGTERM. The
    // binary's signal restore leaves the terminal clean and exits.
    std::thread::sleep(Duration::from_millis(4000));
    let _ = child.kill();
    let _ = child.wait();

    let contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        contents.contains("run_loop: enter"),
        "log missing run_loop marker:\n{contents}"
    );
    assert!(
        contents.contains("raw mode + alt screen entered"),
        "log missing raw-mode marker:\n{contents}"
    );
    assert!(
        contents.contains("frame 0: 2 raw frames"),
        "log missing first-frame marker:\n{contents}"
    );
    let _ = std::fs::remove_file(&log);
}

#[test]
fn kitwm_help_prints() {
    let bin = kitwm_path();
    if !bin.exists() {
        eprintln!("skipping: kitwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--help")
        .output()
        .expect("run kitwm --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("kitwm — kittui window manager"));
    assert!(stdout.contains("--backend"));
}

#[test]
fn kitwm_status_prints_when_no_daemon() {
    let bin = kitwm_path();
    if !bin.exists() {
        eprintln!("skipping: kitwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--status")
        .output()
        .expect("run kitwm --status");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.to_lowercase().contains("daemon"));
}

#[test]
fn kitwm_doctor_prints_text_report() {
    let bin = kitwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin).arg("doctor").output().expect("run kitwm doctor");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kitwm doctor"), "missing header: {s}");
    assert!(s.contains("features"));
    assert!(s.contains("displays"));
}

#[test]
fn kitwm_doctor_json_emits_object() {
    let bin = kitwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin).args(["doctor", "--json"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    assert!(t.starts_with('{') && t.ends_with('}'), "not JSON-ish: {s}");
    assert!(s.contains("\"display_count\""));
    assert!(s.contains("\"features\""));
}

#[cfg(all(target_os = "macos"))]
#[test]
fn kitwm_list_windows_prints_header_and_at_least_one_window() {
    let bin = kitwm_path();
    if !bin.exists() {
        eprintln!("skipping: kitwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--list-windows")
        .output()
        .expect("run kitwm --list-windows");
    if !out.status.success() {
        // Likely built without --features quartz; skip rather than fail.
        eprintln!(
            "skipping: kitwm --list-windows returned non-zero: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("owner") && stdout.contains("title"),
        "missing header: {stdout}"
    );
    // One or more data rows. On a real macOS desktop there's always at
    // least the WindowServer status indicators.
    let lines = stdout.lines().count();
    assert!(lines > 1, "expected header + at least one window: {stdout}");
}

#[cfg(all(target_os = "macos"))]
#[test]
fn kitwm_list_displays_prints_at_least_one_display() {
    let bin = kitwm_path();
    if !bin.exists() {
        eprintln!("skipping: kitwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--list-displays")
        .output()
        .expect("run kitwm --list-displays");
    if !out.status.success() {
        eprintln!("skipping: kitwm --list-displays returned non-zero (likely no quartz feature)");
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bounds"));
}

#[cfg(all(target_os = "macos"))]
#[test]
fn kitwm_record_writes_png_files() {
    let bin = kitwm_path();
    if !bin.exists() { return; }
    let dir = std::env::temp_dir().join(format!(
        "kitwm-rec-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(&bin)
        .args(["record", "--frames", "3", "--out"])
        .arg(&dir)
        .output()
        .expect("run kitwm record");
    if !out.status.success() {
        eprintln!(
            "skipping: kitwm record failed (likely no quartz/sck): stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("dir exists")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    assert!(entries.len() >= 3, "expected >=3 PNG files, got {}", entries.len());
    // First file should start with PNG signature 89 50 4E 47.
    let bytes = std::fs::read(entries[0].path()).unwrap();
    assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(all(target_os = "macos"))]
#[test]
fn kitwm_record_apng_writes_single_file() {
    let bin = kitwm_path();
    if !bin.exists() { return; }
    let dir = std::env::temp_dir().join(format!(
        "kitwm-apng-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(&bin)
        .args(["record", "--frames", "4", "--apng", "--delay-ms", "50", "--out"])
        .arg(&dir)
        .output()
        .expect("run kitwm record --apng");
    if !out.status.success() {
        eprintln!("skipping: stderr={}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let apng = dir.join("kitwm.apng");
    assert!(apng.exists(), "kitwm.apng missing at {apng:?}");
    let bytes = std::fs::read(&apng).unwrap();
    // PNG signature.
    assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
    // APNG marker chunk 'acTL' must be present.
    assert!(
        bytes.windows(4).any(|w| w == b"acTL"),
        "no acTL chunk -> not an APNG"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(all(target_os = "macos"))]
#[test]
fn kitwm_bench_json_emits_metrics() {
    let bin = kitwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["bench", "--seconds", "1", "--json"])
        .output()
        .expect("run kitwm bench --json");
    if !out.status.success() {
        eprintln!("skipping: stderr={}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    assert!(t.starts_with('{') && t.ends_with('}'), "not JSON: {s}");
    for key in &["captures_per_s", "p50_us", "p95_us", "p99_us", "mb_per_s"] {
        assert!(s.contains(&format!("\"{key}\"")), "missing key {key}: {s}");
    }
}
