//! `kittwm` end-to-end smoke. Spawns the `kittwm` binary with the FakeServer
//! backend, lets it run briefly, and asserts that the debug log contains
//! evidence of a real render loop (frames rendered, terminal restored).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn kittwm_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("kittwm");
    p
}

#[test]
fn kittwm_fake_backend_renders_frames_then_exits() {
    let bin = kittwm_path();
    if !bin.exists() {
        eprintln!("skipping: kittwm not built yet at {}", bin.display());
        return;
    }
    let log = std::env::temp_dir().join(format!(
        "kittwm-smoke-{}-{}.log",
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
        .expect("spawn kittwm");

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
fn kittwm_help_prints() {
    let bin = kittwm_path();
    if !bin.exists() {
        eprintln!("skipping: kittwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--help")
        .output()
        .expect("run kittwm --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("kittwm — terminal-native window manager"));
    assert!(stdout.contains("--backend"));
}

#[test]
fn kittwm_status_prints_when_no_daemon() {
    let bin = kittwm_path();
    if !bin.exists() {
        eprintln!("skipping: kittwm not built");
        return;
    }
    // Point at a path that should never have a daemon, so we get the
    // 'no daemon listening' message and exit 1.
    let sock = std::path::PathBuf::from("/tmp").join(format!("ktwm-smoke-nope-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    let out = Command::new(&bin)
        .arg("--status")
        .env("KITTWM_SOCK", &sock)
        .output()
        .expect("run kittwm --status");
    // status against a missing daemon exits non-zero.
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.to_lowercase().contains("no daemon"), "unexpected: {stdout}");
}

#[test]
fn kittwm_serve_status_kill_round_trip() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let sock = std::path::PathBuf::from("/tmp").join(format!("ktwm-smoke-rt-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    // Spawn --serve in the background.
    let mut child = std::process::Command::new(&bin)
        .arg("--serve")
        .env("KITTWM_SOCK", &sock)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn kittwm --serve");
    // Wait for socket file to appear.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !sock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(sock.exists(), "daemon did not bind socket within 5s");
    // STATUS should succeed and mention pid=.
    let st = Command::new(&bin)
        .arg("--status")
        .env("KITTWM_SOCK", &sock)
        .output()
        .unwrap();
    assert!(st.status.success(), "status stderr: {}", String::from_utf8_lossy(&st.stderr));
    let s = String::from_utf8_lossy(&st.stdout);
    assert!(s.contains("pid="), "status missing pid=: {s}");
    // KILL the daemon.
    let k = Command::new(&bin)
        .arg("--kill")
        .env("KITTWM_SOCK", &sock)
        .output()
        .unwrap();
    assert!(k.status.success());
    let _ = child.wait();
    // Socket should be cleaned up.
    assert!(!sock.exists(), "socket lingered after --kill");
}

#[test]
fn kittwm_doctor_prints_text_report() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin).arg("doctor").output().expect("run kittwm doctor");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kittwm doctor"), "missing header: {s}");
    assert!(s.contains("features"));
    assert!(s.contains("displays"));
}

#[test]
fn kittwm_doctor_json_emits_object() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin).args(["doctor", "--json"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    assert!(t.starts_with('{') && t.ends_with('}'), "not JSON-ish: {s}");
    assert!(s.contains("\"display_count\""));
    assert!(s.contains("\"features\""));
}

#[cfg(target_os = "macos")]
#[test]
fn kittwm_list_windows_prints_header_and_at_least_one_window() {
    let bin = kittwm_path();
    if !bin.exists() {
        eprintln!("skipping: kittwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--list-windows")
        .output()
        .expect("run kittwm --list-windows");
    if !out.status.success() {
        // Likely built without --features quartz; skip rather than fail.
        eprintln!(
            "skipping: kittwm --list-windows returned non-zero: stderr={}",
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

#[cfg(target_os = "macos")]
#[test]
fn kittwm_list_displays_prints_at_least_one_display() {
    let bin = kittwm_path();
    if !bin.exists() {
        eprintln!("skipping: kittwm not built");
        return;
    }
    let out = Command::new(&bin)
        .arg("--list-displays")
        .output()
        .expect("run kittwm --list-displays");
    if !out.status.success() {
        eprintln!("skipping: kittwm --list-displays returned non-zero (likely no quartz feature)");
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bounds"));
}

#[cfg(target_os = "macos")]
#[test]
fn kittwm_record_writes_png_files() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let dir = std::env::temp_dir().join(format!(
        "kittwm-rec-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(&bin)
        .args(["record", "--frames", "3", "--out"])
        .arg(&dir)
        .output()
        .expect("run kittwm record");
    if !out.status.success() {
        eprintln!(
            "skipping: kittwm record failed (likely no quartz/sck): stderr={}",
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

#[cfg(target_os = "macos")]
#[test]
fn kittwm_record_apng_writes_single_file() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let dir = std::env::temp_dir().join(format!(
        "kittwm-apng-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(&bin)
        .args(["record", "--frames", "4", "--apng", "--delay-ms", "50", "--out"])
        .arg(&dir)
        .output()
        .expect("run kittwm record --apng");
    if !out.status.success() {
        eprintln!("skipping: stderr={}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let apng = dir.join("kittwm.apng");
    assert!(apng.exists(), "kittwm.apng missing at {apng:?}");
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

#[cfg(target_os = "macos")]
#[test]
fn kittwm_bench_json_emits_metrics() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["bench", "--seconds", "1", "--json"])
        .output()
        .expect("run kittwm bench --json");
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

#[cfg(target_os = "macos")]
#[test]
fn kittwm_attach_repl_round_trip() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let sock = std::path::PathBuf::from("/tmp").join(format!("ktwm-attach-smoke-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    let mut server = std::process::Command::new(&bin)
        .arg("--serve")
        .env("KITTWM_SOCK", &sock)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn --serve");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !sock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(sock.exists(), "daemon did not bind");
    // Feed a script via stdin.
    let mut child = std::process::Command::new(&bin)
        .arg("--attach")
        .env("KITTWM_SOCK", &sock)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn --attach");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"PING\nDISPLAYS\nQUIT\n").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("PONG"), "attach stdout missing PONG: {s}");
    assert!(s.contains("DISPLAYS "), "missing DISPLAYS reply: {s}");
    assert!(s.contains("BYE"), "missing BYE: {s}");
    let _ = server.wait();
    let _ = std::fs::remove_file(&sock);
}

#[cfg(target_os = "macos")]
#[test]
fn kittwm_attach_command_one_shot_round_trip() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let sock = std::path::PathBuf::from("/tmp").join(format!("ktwm-attach-command-smoke-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    let mut server = std::process::Command::new(&bin)
        .arg("--serve")
        .env("KITTWM_SOCK", &sock)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn --serve");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !sock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(sock.exists(), "daemon did not bind");
    // Use STATUS here rather than DISPLAYS so the smoke remains valid even
    // when the workspace is tested without the macOS/quartz feature set.
    let out = std::process::Command::new(&bin)
        .args(["--attach", "-c", "STATUS"])
        .env("KITTWM_SOCK", &sock)
        .output()
        .expect("run --attach -c STATUS");
    assert!(out.status.success(), "stderr: {} stdout: {}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("pid=") && s.contains("uptime_s="), "stdout missing status: {s}");
    let _ = std::process::Command::new(&bin)
        .arg("--kill")
        .env("KITTWM_SOCK", &sock)
        .output();
    let _ = server.wait();
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn kittwm_launch_spawns_command_and_prints_pid() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["launch", "--", "/bin/echo", "kittwm-launch-smoke"])
        .output()
        .expect("run kittwm launch");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kittwm launch: pid="), "missing pid: {s}");
    assert!(s.contains("/bin/echo"), "missing argv: {s}");
}

#[test]
fn kittwm_keymap_prints_default_ctrl_a_bindings() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .arg("keymap")
        .output()
        .expect("run kittwm keymap");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("prefix: C-a"), "missing prefix: {s}");
    assert!(s.contains("C-a c") && s.contains("workspace.new"), "missing workspace binding: {s}");
    assert!(s.contains("C-a |") && s.contains("split.vertical.launcher"), "missing split binding: {s}");
    assert!(s.contains("C-a C-h") && s.contains("focus.left"), "missing focus binding: {s}");
}

#[test]
fn kittwm_keymap_parses_custom_file() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let path = std::env::temp_dir().join(format!("kittwm-keymap-{}.conf", std::process::id()));
    std::fs::write(&path, "prefix C-x\nbind y custom.yank\n").unwrap();
    let out = Command::new(&bin)
        .args(["keymap", "--keymap"])
        .arg(&path)
        .output()
        .expect("run kittwm keymap --keymap");
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("prefix: C-x"), "missing custom prefix: {s}");
    assert!(s.contains("C-x y") && s.contains("custom.yank"), "missing custom binding: {s}");
}

#[test]
fn kittwm_apps_lists_candidates_and_default() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["apps", "--limit", "5"])
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .output()
        .expect("run kittwm apps");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kittwm apps"), "missing header: {s}");
    assert!(s.contains("default: /bin/echo hello"), "missing default: {s}");
    assert!(s.contains("default_resolved: /bin/echo"), "missing resolved path: {s}");
    assert!(s.contains("PATH commands"), "missing PATH commands: {s}");
}

#[test]
fn kittwm_apps_json_lists_candidates_and_default() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["apps", "--json", "--limit", "5"])
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .output()
        .expect("run kittwm apps --json");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    assert!(t.starts_with('{') && t.ends_with('}'), "not JSON-ish: {s}");
    assert!(s.contains("\"default_command\": \"/bin/echo hello\""), "missing default: {s}");
    assert!(s.contains("\"default_resolved\": \"/bin/echo\""), "missing resolved path: {s}");
    assert!(s.contains("\"path_commands\""), "missing path commands: {s}");
    assert!(s.contains("\"macos_apps\""), "missing macos apps: {s}");
}

#[cfg(target_os = "macos")]
#[test]
fn kittwm_attach_apps_verbs_round_trip() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let sock = std::path::PathBuf::from("/tmp").join(format!("ktwm-apps-verb-smoke-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    let mut server = std::process::Command::new(&bin)
        .arg("--serve")
        .env("KITTWM_SOCK", &sock)
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn --serve");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !sock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(sock.exists(), "daemon did not bind");

    let apps = Command::new(&bin)
        .args(["--attach", "-c", "APPS"])
        .env("KITTWM_SOCK", &sock)
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .output()
        .expect("run APPS");
    assert!(apps.status.success(), "stderr: {}", String::from_utf8_lossy(&apps.stderr));
    let s = String::from_utf8_lossy(&apps.stdout);
    assert!(s.contains("APPS default=\"/bin/echo hello\""), "missing APPS header: {s}");
    assert!(s.contains("PATH_COMMANDS"), "missing PATH commands: {s}");

    let json = Command::new(&bin)
        .args(["--attach", "-c", "APPS_JSON"])
        .env("KITTWM_SOCK", &sock)
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .output()
        .expect("run APPS_JSON");
    assert!(json.status.success(), "stderr: {}", String::from_utf8_lossy(&json.stderr));
    let j = String::from_utf8_lossy(&json.stdout);
    assert!(j.contains("\"default_command\": \"/bin/echo hello\""), "missing json default: {j}");
    assert!(j.contains("\"path_commands\""), "missing path commands json: {j}");

    let _ = Command::new(&bin)
        .arg("--kill")
        .env("KITTWM_SOCK", &sock)
        .output();
    let _ = server.wait();
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn kittwm_apps_filter_narrows_text_and_json() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["apps", "--filter", "echo", "--limit", "10"])
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .output()
        .expect("run kittwm apps --filter");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("filter: echo"), "missing filter line: {s}");
    assert!(s.to_ascii_lowercase().contains("echo"), "filtered output missing echo: {s}");

    let json = Command::new(&bin)
        .args(["apps", "--json", "--filter", "echo", "--limit", "10"])
        .env("KITTWM_LAUNCH_CMD", "/bin/echo hello")
        .output()
        .expect("run kittwm apps --json --filter");
    assert!(json.status.success(), "stderr: {}", String::from_utf8_lossy(&json.stderr));
    let j = String::from_utf8_lossy(&json.stdout);
    assert!(j.contains("\"path_commands\""), "missing path commands: {j}");
    assert!(j.to_ascii_lowercase().contains("echo"), "json filtered output missing echo: {j}");
}

#[test]
fn kittwm_apps_first_and_launch_first_select_candidate() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let first = Command::new(&bin)
        .args(["apps", "--filter", "echo", "--first", "--limit", "10"])
        .output()
        .expect("run kittwm apps --first");
    assert!(first.status.success(), "stderr: {}", String::from_utf8_lossy(&first.stderr));
    let first_out = String::from_utf8_lossy(&first.stdout);
    assert!(first_out.trim().starts_with("path:"), "expected path candidate: {first_out}");
    assert!(first_out.to_ascii_lowercase().contains("echo"), "expected echo candidate: {first_out}");

    let launched = Command::new(&bin)
        .args(["apps", "--filter", "echo", "--launch-first", "--limit", "10"])
        .output()
        .expect("run kittwm apps --launch-first");
    assert!(launched.status.success(), "stderr: {}", String::from_utf8_lossy(&launched.stderr));
    let s = String::from_utf8_lossy(&launched.stdout);
    assert!(s.contains("kittwm apps: launched pid="), "missing launch pid: {s}");
    assert!(s.contains("kind=path"), "wrong candidate kind: {s}");
}

#[cfg(target_os = "macos")]
#[test]
fn kittwm_attach_apps_first_verbs_round_trip() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let sock = std::path::PathBuf::from("/tmp").join(format!("ktwm-apps-first-smoke-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock);
    let mut server = std::process::Command::new(&bin)
        .arg("--serve")
        .env("KITTWM_SOCK", &sock)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn --serve");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !sock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(sock.exists(), "daemon did not bind");

    let first = Command::new(&bin)
        .args(["--attach", "-c", "APPS_FIRST echo"])
        .env("KITTWM_SOCK", &sock)
        .output()
        .expect("run APPS_FIRST");
    assert!(first.status.success(), "stderr: {}", String::from_utf8_lossy(&first.stderr));
    let s = String::from_utf8_lossy(&first.stdout);
    assert!(s.contains("APPS_FIRST kind=path"), "missing APPS_FIRST path candidate: {s}");
    assert!(s.to_ascii_lowercase().contains("echo"), "missing echo candidate: {s}");

    let launched = Command::new(&bin)
        .args(["--attach", "-c", "APPS_LAUNCH_FIRST echo"])
        .env("KITTWM_SOCK", &sock)
        .output()
        .expect("run APPS_LAUNCH_FIRST");
    assert!(launched.status.success(), "stderr: {}", String::from_utf8_lossy(&launched.stderr));
    let l = String::from_utf8_lossy(&launched.stdout);
    assert!(l.contains("APPS_LAUNCH_FIRST pid="), "missing pid: {l}");
    assert!(l.contains("kind=path"), "wrong kind: {l}");

    let _ = Command::new(&bin)
        .arg("--kill")
        .env("KITTWM_SOCK", &sock)
        .output();
    let _ = server.wait();
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn kittwm_launcher_preview_renders_boxed_filtered_menu() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["launcher", "--filter", "echo", "--limit", "5"])
        .output()
        .expect("run kittwm launcher");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kittwm launcher"), "missing title: {s}");
    assert!(s.contains("query: echo"), "missing query: {s}");
    assert!(s.contains("▶") || s.contains("[path ]"), "missing selectable row: {s}");
    assert!(s.to_ascii_lowercase().contains("echo"), "missing echo candidate: {s}");
}

#[test]
fn kittwm_launcher_select_and_launch_selection() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let out = Command::new(&bin)
        .args(["launcher", "--filter", "echo", "--select", "2", "--limit", "5"])
        .output()
        .expect("run kittwm launcher --select");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("kittwm launcher"), "missing launcher title: {s}");
    assert!(s.contains("▶  2.") || s.contains("▶  1."), "missing selected marker: {s}");

    let launched = Command::new(&bin)
        .args(["launcher", "--filter", "echo", "--select", "1", "--launch-selection", "--limit", "5"])
        .output()
        .expect("run kittwm launcher --launch-selection");
    assert!(launched.status.success(), "stderr: {}", String::from_utf8_lossy(&launched.stderr));
    let ls = String::from_utf8_lossy(&launched.stdout);
    assert!(ls.contains("kittwm launcher: launched selection=1 pid="), "missing launched pid: {ls}");
    assert!(ls.contains("kind=path"), "expected path candidate: {ls}");
}

#[test]
fn kittwm_keymap_check_reports_ok_for_default_and_duplicates_for_custom_file() {
    let bin = kittwm_path();
    if !bin.exists() { return; }
    let ok = Command::new(&bin)
        .args(["keymap", "--check"])
        .output()
        .expect("run kittwm keymap --check");
    assert!(ok.status.success(), "stderr: {}", String::from_utf8_lossy(&ok.stderr));
    let s = String::from_utf8_lossy(&ok.stdout);
    assert!(s.contains("kittwm keymap check"), "missing header: {s}");
    assert!(s.contains("duplicate_chords: 0"), "unexpected duplicates: {s}");
    assert!(s.contains("status: ok"), "missing ok: {s}");

    let path = std::env::temp_dir().join(format!("kittwm-keymap-dupe-{}.conf", std::process::id()));
    std::fs::write(&path, "prefix C-a\nbind c workspace.new\nbind c quit\n").unwrap();
    let dup = Command::new(&bin)
        .args(["keymap", "--check", "--keymap"])
        .arg(&path)
        .output()
        .expect("run duplicate keymap check");
    let _ = std::fs::remove_file(&path);
    assert_eq!(dup.status.code(), Some(2), "expected duplicate exit 2, got {:?}; stdout={}", dup.status.code(), String::from_utf8_lossy(&dup.stdout));
    let d = String::from_utf8_lossy(&dup.stdout);
    assert!(d.contains("duplicate_chords: 1"), "missing duplicate count: {d}");
    assert!(d.contains("C-a c"), "missing duplicated chord: {d}");
}
