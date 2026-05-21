//! `kitwm` — the kittui window manager launcher.
//!
//! With no args, opens a kittui-wm session in the current terminal,
//! picking the best available backend (Quartz on macOS, Xvfb on Linux,
//! `FakeServer` otherwise). Survives terminal restoration on
//! SIGINT/HUP/TERM/QUIT via the shared `kittui_cli::session` module.
//!
//! Flags:
//!
//! ```text
//! kitwm              # open a session in the current terminal
//! kitwm --serve      # run only the (in-process today) backend host loop
//! kitwm --attach     # attach to an existing daemon (placeholder; bd-fb5d9d)
//! kitwm --kill       # send shutdown to the daemon (placeholder; bd-fb5d9d)
//! kitwm --status     # print whether a daemon is running (placeholder)
//! kitwm --backend X  # force a specific backend: fake | quartz | xvfb
//! ```
//!
//! Once the daemon/client split (bd-fb5d9d) lands, `--serve` becomes a
//! `fork + setsid + exec` of the daemon and `kitwm` (no args) attaches
//! to the running socket transparently.
//!
//! The end-goal acceptance criterion is that `kitwm` opens a usable
//! session with an app launcher that can spawn an X11 app (xterm via
//! XQuartz on macOS, xterm via Xvfb on Linux) and route keystrokes into
//! it. See bead bd-a9ec5b.

use std::process::ExitCode;

use anyhow::{anyhow, Result};

use kittui::{CellSize, Runtime, TerminalInfo};
use kittui_core::geom::PxRect;
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
use kittui_xvfb::{FakeServer, XServer, XWindowId};

#[derive(Debug, Default)]
struct Cli {
    mode: Mode,
    backend: Option<Backend>,
    pick_window: bool,
    list_windows: bool,
    list_displays: bool,
    capture: Option<String>,
    fps: Option<u32>,
    doctor: bool,
    json: bool,
    record: bool,
    record_frames: Option<u32>,
    record_out: Option<String>,
    record_apng: bool,
    record_delay_ms: Option<u32>,
    bench: bool,
    bench_seconds: Option<u32>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Session,
    Serve,
    Attach,
    Kill,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Fake,
    Quartz,
    Xvfb,
}

fn parse_args() -> Result<Cli> {
    let mut args = std::env::args().skip(1);
    let mut out = Cli::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "doctor" => out.doctor = true,
            "record" => out.record = true,
            "bench" => out.bench = true,
            "--seconds" => {
                let v = args.next().ok_or_else(|| anyhow!("--seconds N"))?;
                out.bench_seconds = Some(v.parse().map_err(|_| anyhow!("--seconds expects integer"))?);
            }
            "--frames" => {
                let v = args.next().ok_or_else(|| anyhow!("--frames N"))?;
                out.record_frames = Some(v.parse().map_err(|_| anyhow!("--frames expects integer"))?);
            }
            "--out" => {
                out.record_out = Some(args.next().ok_or_else(|| anyhow!("--out DIR"))?);
            }
            "--apng" => out.record_apng = true,
            "--delay-ms" => {
                let v = args.next().ok_or_else(|| anyhow!("--delay-ms N"))?;
                out.record_delay_ms = Some(v.parse().map_err(|_| anyhow!("--delay-ms expects integer"))?);
            }
            "--json" => out.json = true,
            "--serve" => out.mode = Mode::Serve,
            "--attach" => out.mode = Mode::Attach,
            "--kill" => out.mode = Mode::Kill,
            "--status" => out.mode = Mode::Status,
            "--backend" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--backend requires a value"))?;
                out.backend = Some(match v.as_str() {
                    "fake" => Backend::Fake,
                    "quartz" => Backend::Quartz,
                    "xvfb" => Backend::Xvfb,
                    other => return Err(anyhow!("unknown backend {other}")),
                });
            }
            "--pick-window" => out.pick_window = true,
            "--list-windows" => out.list_windows = true,
            "--list-displays" => out.list_displays = true,
            "--capture" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--capture requires a target spec"))?;
                out.capture = Some(v);
            }
            "--fps" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--fps requires an integer (1..=240)"))?;
                let n: u32 = v.parse().map_err(|_| anyhow!("--fps expects an integer, got {v:?}"))?;
                out.fps = Some(n);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown arg {other}")),
        }
    }
    Ok(out)
}

fn print_help() {
    println!(
        "kitwm — kittui window manager\n\n\
         Usage: kitwm [--serve | --attach | --kill | --status] [--backend fake|quartz|xvfb]\n\n\
         Default: open a kittui-wm session in the current terminal, picking the\n\
         best available backend. q or Esc to quit.\n\n\
         --serve   run the in-process backend host (today same as no args; will\n\
                   become 'fork daemon + listen on $KITTUI_WM_DISPLAY socket' in\n\
                   bd-fb5d9d).\n\
         --attach  attach to an existing daemon (placeholder until bd-fb5d9d).\n\
         --kill    send shutdown to the daemon (placeholder).\n\
         --status  print whether a daemon is running (placeholder).\n\
         --backend fake|quartz|xvfb force a specific backend.\n\
         --pick-window   (macOS+quartz) live picker over CGWindowList; pick\n\
                         one window, then run a kitwm session capturing only it.\n\
         --list-windows  (macOS+quartz) print every titled CGWindow with id,\n\
                         bounds, owner, title — useful for scripting kitwm.\n\
         --list-displays (macOS+quartz) print every connected CGDirectDisplayID\n\
                         with bounds + index.\n\
         --capture SPEC  (macOS+quartz) capture a specific source non-interactively:\n\
                         'main'                 = main display (default)\n\
                         'display:<n>'          = nth connected display\n\
                         'window:<substring>'   = first matching app window title/owner\n\
                         'all'                  = all displays as a multi-window session\n\
         --fps N         frame-rate cap for the main loop (1..=240, default 60).\n\
                         Live fps + peak fps render in the chrome footer.\n\
                         Note: on macOS std::thread::sleep granularity is\n\
                         ~10ms so the actual ceiling is roughly half the\n\
                         requested value; raise --fps to push it higher.\n\
\n\
SUBCOMMANDS\n\
         doctor          print a diagnostics report (backends, displays,\n\
                         terminal probe, log status, version). Pass --json\n\
                         for machine-readable output. Never enters raw mode.\n\
         record          capture N frames from --capture/--backend target and\n\
                         write them as PNG files to --out DIR (default\n\
                         /tmp/kitwm-record-<unix-ts>). Defaults to 30 frames.\n\
                         Pass --apng to emit a single animated PNG at\n\
                         <out>/kitwm.apng (use --delay-ms N for cadence,\n\
                         default 33ms). Never enters raw mode.\n\
         bench           measure capture-pipeline throughput. Runs raw_frames\n\
                         in a tight loop for --seconds N (default 3) against\n\
                         --capture target and prints captures/s + p50/p95/p99\n\
                         latency + MB/s. --json for machine-readable output.\n"
    );
}

fn pick_backend(forced: Option<Backend>) -> Backend {
    if let Some(b) = forced {
        return b;
    }
    // Auto-pick by host. Order matters: prefer the richest available
    // backend (Quartz on macOS with sck, Xvfb on Linux), falling back
    // to FakeServer everywhere.
    #[cfg(all(target_os = "macos", feature = "quartz"))]
    {
        return Backend::Quartz;
    }
    #[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
    {
        return Backend::Xvfb;
    }
    #[cfg(not(any(
        all(target_os = "macos", feature = "quartz"),
        feature = "xvfb"
    )))]
    {
        Backend::Fake
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kitwm: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let cli = parse_args()?;
    if let Some(fps) = cli.fps {
        std::env::set_var("KITTUI_WM_FPS", fps.to_string());
    }

    // Inspection flags run cooked, never enter raw mode.
    if cli.doctor {
        return doctor_cmd(cli.json);
    }
    if cli.record {
        return record_cmd(&cli);
    }
    if cli.bench {
        return bench_cmd(&cli);
    }
    if cli.list_windows {
        return list_windows_cmd();
    }
    if cli.list_displays {
        return list_displays_cmd();
    }

    match cli.mode {
        Mode::Session | Mode::Serve => run_session(cli),
        Mode::Attach => Err(anyhow!(
            "--attach is a placeholder until the kittui-wm daemon lands (bd-fb5d9d). \
             Run `kitwm` with no args to open a session in this terminal."
        )),
        Mode::Kill => Err(anyhow!(
            "--kill is a placeholder until the kittui-wm daemon lands (bd-fb5d9d)."
        )),
        Mode::Status => {
            println!("kitwm: no daemon yet (bd-fb5d9d). Single-session in-process mode is active.");
            Ok(())
        }
    }
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn list_windows_cmd() -> Result<()> {
    use kittui_quartz::QuartzServer;
    let wins = QuartzServer::list_app_windows();
    println!("{:>8}  {:<24}  {:<48}  bounds", "id", "owner", "title");
    for w in wins {
        println!(
            "{:>8}  {:<24}  {:<48}  ({:.0},{:.0}) {:.0}x{:.0}",
            w.id,
            truncate(&w.owner_name, 24),
            truncate(&w.title, 48),
            w.bounds.origin.0,
            w.bounds.origin.1,
            w.bounds.width,
            w.bounds.height,
        );
    }
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn list_windows_cmd() -> Result<()> {
    Err(anyhow!("--list-windows requires --features quartz on macOS"))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn list_displays_cmd() -> Result<()> {
    use kittui_quartz::QuartzServer;
    let displays = QuartzServer::displays();
    println!("{:>3}  {:>10}  bounds", "#", "id");
    for d in displays {
        println!(
            "{:>3}  {:>10}  ({:.0},{:.0}) {:.0}x{:.0}",
            d.index,
            d.id,
            d.bounds.origin.0,
            d.bounds.origin.1,
            d.bounds.width,
            d.bounds.height,
        );
    }
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn list_displays_cmd() -> Result<()> {
    Err(anyhow!("--list-displays requires --features quartz on macOS"))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn run_session(cli: Cli) -> Result<()> {
    let cell = CellSize::default();
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;
    let backend = pick_backend(cli.backend);

    match backend {
        Backend::Fake => run_with_fake(&runtime, cell),
        #[cfg(all(target_os = "macos", feature = "quartz"))]
        Backend::Quartz => run_with_quartz(&runtime, cell, cli.pick_window, cli.capture.as_deref()),
        #[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
        Backend::Xvfb => run_with_xvfb(&runtime, cell),
        #[cfg(not(all(target_os = "macos", feature = "quartz")))]
        Backend::Quartz => Err(anyhow!(
            "Quartz backend requires --features quartz on macOS"
        )),
        #[cfg(not(feature = "xvfb"))]
        Backend::Xvfb => Err(anyhow!("Xvfb backend requires --features xvfb on Linux")),
    }
}

fn run_with_fake(runtime: &Runtime, cell: CellSize) -> Result<()> {
    // Show a tiny gallery so `kitwm` (no args) always renders something
    // visible on any host, even without Quartz/Xvfb permissions.
    let server = FakeServer::with_windows(vec![
        (
            XWindowId(1),
            PxRect::new(8.0, 16.0, 256.0, 160.0),
            "welcome",
            [0x00, 0xd8, 0xff, 0xff],
        ),
        (
            XWindowId(2),
            PxRect::new(320.0, 16.0, 256.0, 160.0),
            "press q to quit",
            [0xb4, 0x8c, 0xff, 0xff],
        ),
    ]);
    let compositor = Compositor::new(server, cell);
    compositor.set_mode(XWindowId(1), WindowMode::Tiled);
    let mut layout = Layout::all_floating();
    layout.tile(
        XWindowId(1),
        PxRect::new(8.0, 16.0, 320.0, 192.0),
    );
    kittui_cli::session::run_loop(runtime, &compositor, &layout)
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn run_with_quartz(
    runtime: &Runtime,
    cell: CellSize,
    pick_window: bool,
    capture: Option<&str>,
) -> Result<()> {
    use kittui_quartz::{CaptureTarget, QuartzServer};

    let target = if pick_window {
        let windows = QuartzServer::list_app_windows();
        let chosen = prompt_pick(&windows)?;
        eprintln!(
            "kitwm: capturing window {} ({}: {})",
            chosen.id, chosen.owner_name, chosen.title
        );
        CaptureTarget::Window(chosen.id)
    } else if let Some(spec) = capture {
        resolve_capture_spec(spec)?
    } else {
        CaptureTarget::MainDisplay
    };

    let mut server = QuartzServer::with_target(target);
    let max_w = 80u32 * cell.width_px as u32 * 2;
    let max_h = 24u32 * cell.height_px as u32 * 2;
    server.set_max_size(Some((max_w, max_h)));

    eprintln!("kitwm: probing macOS Screen Recording permission...");
    let probe = server.windows().and_then(|w| {
        if let Some(first) = w.first() {
            server.capture(first.id).map(|_| ())
        } else {
            Err(kittui_xvfb::XError::Unavailable("no displays".into()))
        }
    });
    if let Err(e) = probe {
        return Err(anyhow!(
            "kitwm could not capture the screen: {e}\n\n  Grant Screen Recording \
             to your terminal under System Settings -> Privacy & Security -> \
             Screen Recording, then quit and relaunch the terminal."
        ));
    }
    eprintln!("kitwm: backend ready. q/Esc to quit.");
    std::thread::sleep(std::time::Duration::from_millis(600));

    let compositor = Compositor::new(server, cell);
    let mut layout = Layout::all_floating();
    if let Ok(ws) = compositor.server().windows() {
        if let Some(w) = ws.first() {
            layout.tile(
                w.id,
                PxRect::new(
                    0.0,
                    0.0,
                    80.0 * cell.width_px as f32,
                    24.0 * cell.height_px as f32,
                ),
            );
            compositor.set_mode(w.id, WindowMode::Tiled);
        }
    }
    kittui_cli::session::run_loop(runtime, &compositor, &layout)
}

#[cfg(all(not(all(target_os = "macos", feature = "quartz")), feature = "xvfb"))]
fn run_with_xvfb(runtime: &Runtime, cell: CellSize) -> Result<()> {
    let display: u32 = std::env::var("KITTUI_WM_DISPLAY")
        .ok()
        .and_then(|s| s.trim_start_matches(':').parse().ok())
        .unwrap_or(99);
    let server = kittui_xvfb::xvfb::XvfbServer::spawn(display)
        .map_err(|e| anyhow!("XvfbServer::spawn: {e}"))?;
    let compositor = Compositor::new(server, cell);
    let layout = Layout::all_floating();
    kittui_cli::session::run_loop(runtime, &compositor, &layout)
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn prompt_pick(windows: &[kittui_quartz::MacWindow]) -> Result<kittui_quartz::MacWindow> {
    use std::io::{BufRead, Write};
    if windows.is_empty() {
        return Err(anyhow!(
            "no macOS app windows visible via CGWindowList; nothing to pick"
        ));
    }
    println!("\nkitwm --pick-window\n");
    for (i, w) in windows.iter().enumerate() {
        println!(
            "  [{:>2}]  {:<24}  {:<48}  ({:.0},{:.0}) {:.0}x{:.0}",
            i,
            truncate(&w.owner_name, 24),
            truncate(&w.title, 48),
            w.bounds.origin.0,
            w.bounds.origin.1,
            w.bounds.width,
            w.bounds.height,
        );
    }
    print!("\nNumber to capture (Enter to cancel): ");
    std::io::stdout().flush().ok();
    let stdin = std::io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).ok();
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("cancelled by operator"));
    }
    let idx: usize = trimmed
        .parse()
        .map_err(|_| anyhow!("expected a number, got {trimmed:?}"))?;
    windows
        .get(idx)
        .cloned()
        .ok_or_else(|| anyhow!("out of range; pick 0..{}", windows.len()))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn resolve_capture_spec(spec: &str) -> Result<kittui_quartz::CaptureTarget> {
    use kittui_quartz::{CaptureTarget, QuartzServer};
    if spec == "main" {
        return Ok(CaptureTarget::MainDisplay);
    }
    if spec == "all" {
        return Ok(CaptureTarget::AllDisplays);
    }
    if let Some(n) = spec.strip_prefix("display:") {
        let idx: usize = n
            .parse()
            .map_err(|_| anyhow!("display:N expects an integer, got {n:?}"))?;
        let displays = QuartzServer::displays();
        let chosen = displays
            .get(idx)
            .ok_or_else(|| anyhow!("display index {idx} out of range (0..{})", displays.len()))?;
        return Ok(CaptureTarget::Display(chosen.id));
    }
    if let Some(needle) = spec.strip_prefix("window:") {
        let needle_lc = needle.to_ascii_lowercase();
        let windows = QuartzServer::list_app_windows();
        let chosen = windows
            .iter()
            .find(|w| {
                w.title.to_ascii_lowercase().contains(&needle_lc)
                    || w.owner_name.to_ascii_lowercase().contains(&needle_lc)
            })
            .ok_or_else(|| {
                anyhow!(
                    "no Mac window matched 'window:{needle}'; run `kitwm --list-windows` to see candidates"
                )
            })?;
        eprintln!(
            "kitwm: --capture window:{} matched id={} owner={:?} title={:?}",
            needle, chosen.id, chosen.owner_name, chosen.title
        );
        return Ok(CaptureTarget::Window(chosen.id));
    }
    Err(anyhow!(
        "unknown --capture spec {spec:?}. Use: main | all | display:<n> | window:<substr>"
    ))
}

fn doctor_cmd(json: bool) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();

    let feat_sck = cfg!(feature = "sck");
    let feat_quartz = cfg!(feature = "quartz");
    let feat_xvfb = cfg!(feature = "xvfb");

    let log_path = std::env::var("KITTUI_WM_LOG")
        .unwrap_or_else(|_| "/tmp/kittui-wm.log".to_string());
    let log_meta = std::fs::metadata(&log_path).ok();
    let log_size = log_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let log_present = log_meta.is_some();

    #[cfg(all(target_os = "macos", feature = "quartz"))]
    let displays = kittui_quartz::QuartzServer::displays();
    #[cfg(not(all(target_os = "macos", feature = "quartz")))]
    let displays: Vec<()> = Vec::new();
    let display_count = displays.len();

    // Kitty graphics: presence of TERM=xterm-kitty or KITTY_WINDOW_ID env.
    let kitty_graphics = term.contains("kitty")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
        || term_program.to_ascii_lowercase().contains("ghostty")
        || term_program.to_ascii_lowercase().contains("wezterm");

    if json {
        let mut buf = String::new();
        buf.push_str("{\n");
        buf.push_str(&format!("  \"version\": {:?},\n", version));
        buf.push_str(&format!("  \"os\": {:?},\n", os));
        buf.push_str(&format!("  \"arch\": {:?},\n", arch));
        buf.push_str(&format!(
            "  \"features\": {{\"sck\": {}, \"quartz\": {}, \"xvfb\": {}}},\n",
            feat_sck, feat_quartz, feat_xvfb
        ));
        buf.push_str(&format!("  \"term\": {:?},\n", term));
        buf.push_str(&format!("  \"colorterm\": {:?},\n", colorterm));
        buf.push_str(&format!("  \"term_program\": {:?},\n", term_program));
        buf.push_str(&format!("  \"kitty_graphics_likely\": {},\n", kitty_graphics));
        buf.push_str(&format!("  \"display_count\": {},\n", display_count));
        buf.push_str(&format!("  \"log_path\": {:?},\n", log_path));
        buf.push_str(&format!("  \"log_present\": {},\n", log_present));
        buf.push_str(&format!("  \"log_size_bytes\": {}\n", log_size));
        buf.push_str("}\n");
        print!("{buf}");
    } else {
        println!("kitwm doctor");
        println!("============");
        println!("  version        : {version}");
        println!("  os / arch      : {os} / {arch}");
        println!(
            "  features       : sck={} quartz={} xvfb={}",
            feat_sck, feat_quartz, feat_xvfb
        );
        println!("  TERM           : {term}");
        println!("  COLORTERM      : {colorterm}");
        println!("  TERM_PROGRAM   : {term_program}");
        println!(
            "  kitty graphics : {}",
            if kitty_graphics { "likely yes" } else { "unknown" }
        );
        println!("  displays       : {display_count}");
        println!(
            "  log            : {} ({}{})",
            log_path,
            if log_present { "present" } else { "missing" },
            if log_present {
                format!(", {log_size} bytes")
            } else {
                String::new()
            }
        );
        if cfg!(target_os = "macos") {
            println!();
            println!(
                "Hint: SCK + CGEventPost both require Screen Recording + Accessibility"
            );
            println!(
                "      permissions on the terminal hosting kitwm (System Settings >"
            );
            println!("      Privacy & Security).");
        }
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn record_cmd(cli: &Cli) -> Result<()> {
    use kittui_quartz::QuartzServer;
    use kittui_render_cpu::Pixmap;
    use kittui_wm::compositor::{Compositor, Layout};

    let frames_target = cli.record_frames.unwrap_or(30);
    let out_dir = cli
        .record_out
        .clone()
        .unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("/tmp/kitwm-record-{ts}")
        });
    std::fs::create_dir_all(&out_dir)?;

    // Resolve capture spec (reuses --capture/--pick-window logic).
    let target = if cli.pick_window {
        let windows = QuartzServer::list_app_windows();
        let chosen = prompt_pick(&windows)?;
        kittui_quartz::CaptureTarget::Window(chosen.id)
    } else if let Some(spec) = cli.capture.as_deref() {
        resolve_capture_spec(spec)?
    } else {
        kittui_quartz::CaptureTarget::MainDisplay
    };

    let server = QuartzServer::with_target(target);
    let cell = kittui::CellSize::new(9, 18);
    let compositor = Compositor::new(server, cell);
    let layout = Layout::all_floating();

    eprintln!("kitwm record: writing {frames_target} frames to {out_dir}");
    let apng_mode = cli.record_apng;
    let delay_ms = cli.record_delay_ms.unwrap_or(33);
    let mut apng_frames: Vec<Pixmap> = Vec::new();
    let started = std::time::Instant::now();
    for i in 0..frames_target {
        let frames = compositor
            .raw_frames(&layout)
            .map_err(|e| anyhow!("capture failed at frame {i}: {e}"))?;
        for (j, f) in frames.iter().enumerate() {
            let mut pm = Pixmap::new(f.width, f.height);
            pm.data_mut().copy_from_slice(&f.rgba);
            if apng_mode {
                // Only record window 0 into the apng (apng requires uniform
                // width/height across frames). Multi-window apng would need
                // composition into a single canvas; left for a future bead.
                if j == 0 {
                    apng_frames.push(pm);
                }
            } else {
                let png = kittui_render_cpu::encode_png(&pm);
                let path = format!("{out_dir}/frame-{:05}-win{}.png", i, j);
                std::fs::write(&path, png)?;
            }
        }
        if i % 10 == 0 {
            eprintln!("  frame {i}/{frames_target}");
        }
    }
    if apng_mode {
        if apng_frames.is_empty() {
            return Err(anyhow!("no frames captured; nothing to write"));
        }
        // Normalize: APNG demands all frames share width/height. Pad or
        // truncate frames whose dims don't match the first.
        let (w, h) = (apng_frames[0].width(), apng_frames[0].height());
        apng_frames.retain(|p| p.width() == w && p.height() == h);
        let delays: Vec<u32> = vec![delay_ms; apng_frames.len()];
        let bytes = kittui_render_cpu::encode_apng(&apng_frames, &delays, 0);
        let path = format!("{out_dir}/kitwm.apng");
        std::fs::write(&path, bytes)?;
        eprintln!("  wrote APNG: {path}");
    }
    let elapsed = started.elapsed();
    eprintln!(
        "kitwm record: done. {} frames in {:.2}s ({:.1} fps). dir={}",
        frames_target,
        elapsed.as_secs_f32(),
        frames_target as f32 / elapsed.as_secs_f32(),
        out_dir
    );
    println!("{out_dir}");
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn record_cmd(_cli: &Cli) -> Result<()> {
    Err(anyhow!("record requires --features quartz on macOS"))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn bench_cmd(cli: &Cli) -> Result<()> {
    use kittui_quartz::QuartzServer;
    use kittui_wm::compositor::{Compositor, Layout};

    let secs = cli.bench_seconds.unwrap_or(3).max(1);
    let target = if let Some(spec) = cli.capture.as_deref() {
        resolve_capture_spec(spec)?
    } else {
        kittui_quartz::CaptureTarget::MainDisplay
    };
    let server = QuartzServer::with_target(target);
    let cell = kittui::CellSize::new(9, 18);
    let compositor = Compositor::new(server, cell);
    let layout = Layout::all_floating();

    eprintln!("kitwm bench: measuring for {secs}s ...");
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(secs as u64);
    let mut latencies_us: Vec<u64> = Vec::with_capacity(4096);
    let mut total_bytes: u64 = 0;
    let mut iters: u64 = 0;
    let mut first_dims = (0u32, 0u32);
    while std::time::Instant::now() < deadline {
        let t0 = std::time::Instant::now();
        let frames = compositor
            .raw_frames(&layout)
            .map_err(|e| anyhow!("capture failed: {e}"))?;
        let dt = t0.elapsed();
        latencies_us.push(dt.as_micros() as u64);
        for f in &frames {
            total_bytes += f.rgba.len() as u64;
            if iters == 0 {
                first_dims = (f.width, f.height);
            }
        }
        iters += 1;
    }
    let wall = started.elapsed();
    latencies_us.sort_unstable();
    let pct = |p: f64| -> u64 {
        if latencies_us.is_empty() { return 0; }
        let idx = ((latencies_us.len() as f64 - 1.0) * p).round() as usize;
        latencies_us[idx]
    };
    let mean = if latencies_us.is_empty() {
        0
    } else {
        (latencies_us.iter().sum::<u64>() / latencies_us.len() as u64)
    };
    let captures_per_s = iters as f64 / wall.as_secs_f32() as f64;
    let mb_per_s = (total_bytes as f64 / 1_048_576.0) / wall.as_secs_f32() as f64;

    if cli.json {
        println!(
            "{{\"captures\": {}, \"wall_s\": {:.3}, \"captures_per_s\": {:.2}, \
             \"mean_us\": {}, \"p50_us\": {}, \"p95_us\": {}, \"p99_us\": {}, \"max_us\": {}, \
             \"bytes\": {}, \"mb_per_s\": {:.2}, \"width\": {}, \"height\": {}}}",
            iters,
            wall.as_secs_f32(),
            captures_per_s,
            mean,
            pct(0.50),
            pct(0.95),
            pct(0.99),
            latencies_us.last().copied().unwrap_or(0),
            total_bytes,
            mb_per_s,
            first_dims.0,
            first_dims.1,
        );
    } else {
        println!("kitwm bench");
        println!("===========");
        println!("  duration       : {:.3} s", wall.as_secs_f32());
        println!("  captures       : {}", iters);
        println!("  captures/s     : {:.1}", captures_per_s);
        println!("  surface        : {}x{} RGBA", first_dims.0, first_dims.1);
        println!("  bytes captured : {:.1} MB", total_bytes as f64 / 1_048_576.0);
        println!("  throughput     : {:.1} MB/s", mb_per_s);
        println!("  mean latency   : {:.2} ms", mean as f64 / 1000.0);
        println!("  p50 latency    : {:.2} ms", pct(0.50) as f64 / 1000.0);
        println!("  p95 latency    : {:.2} ms", pct(0.95) as f64 / 1000.0);
        println!("  p99 latency    : {:.2} ms", pct(0.99) as f64 / 1000.0);
        println!(
            "  max latency    : {:.2} ms",
            latencies_us.last().copied().unwrap_or(0) as f64 / 1000.0
        );
    }
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn bench_cmd(_cli: &Cli) -> Result<()> {
    Err(anyhow!("bench requires --features quartz on macOS"))
}
