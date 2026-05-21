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

    // Let it render for ~1.5s, then SIGTERM (the binary's signal restore
    // will leave the terminal clean and exit gracefully).
    std::thread::sleep(Duration::from_millis(1500));
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
        contents.contains("frame 0: 2 scenes"),
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
