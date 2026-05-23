//! `kittwm` — the kittui window manager launcher.
//!
//! With no args, opens a kittui-wm session in the current terminal,
//! picking the best available backend (Quartz on macOS, Xvfb on Linux,
//! `FakeServer` otherwise). Survives terminal restoration on
//! SIGINT/HUP/TERM/QUIT via the shared `kittui_cli::session` module.
//!
//! Flags:
//!
//! ```text
//! kittwm              # open a session in the current terminal
//! kittwm --serve      # run only the (in-process today) backend host loop
//! kittwm --attach     # attach to an existing daemon (REPL or -c CMD)
//! kittwm --kill       # send shutdown to the daemon (placeholder; bd-fb5d9d)
//! kittwm --status     # print whether a daemon is running (placeholder)
//! kittwm --backend X  # force a specific backend: fake | quartz | xvfb
//! ```
//!
//! Once the daemon/client split (bd-fb5d9d) lands, `--serve` becomes a
//! `fork + setsid + exec` of the daemon and `kittwm` (no args) attaches
//! to the running socket transparently.
//!
//! The end-goal acceptance criterion is that `kittwm` opens a usable
//! session with an app launcher that can spawn an X11 app (xterm via
//! XQuartz on macOS, xterm via Xvfb on Linux) and route keystrokes into
//! it. See bead bd-a9ec5b.

use std::process::ExitCode;

use anyhow::{anyhow, Result};

use kittui::{CellSize, Runtime, TerminalInfo};
use kittui_core::geom::PxRect;
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
#[cfg(all(target_os = "macos", feature = "quartz"))]
use kittui_xvfb::XServer;
use kittui_xvfb::{FakeServer, XWindowId};

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
    config: bool,
    record: bool,
    record_frames: Option<u32>,
    record_out: Option<String>,
    record_apng: bool,
    record_delay_ms: Option<u32>,
    bench: bool,
    bench_seconds: Option<u32>,
    attach_command: Option<String>,
    launch: bool,
    replace: bool,
    replace_args: Vec<String>,
    launcher_preview: bool,
    launcher_select: Option<usize>,
    launcher_launch_selection: bool,
    launch_args: Vec<String>,
    launch_on_f12: bool,
    launcher_query: Option<String>,
    launcher_overlay: bool,
    no_launcher_overlay: bool,
    apps: bool,
    apps_limit: Option<usize>,
    apps_filter: Option<String>,
    apps_first: bool,
    apps_launch_first: bool,
    keymap: bool,
    keymap_path: Option<String>,
    keymap_check: bool,
    native_terminal: bool,
    native_browser: bool,
    native_url: Option<String>,
    native_out: Option<String>,
    save_session: Option<String>,
    restore_session: Option<String>,
    automation_request: Option<String>,
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
            "config" => out.config = true,
            "record" => out.record = true,
            "bench" => out.bench = true,
            "launch" => {
                out.launch = true;
                out.launch_args = args.by_ref().collect();
                break;
            }
            "replace" => {
                out.replace = true;
                out.replace_args = args.by_ref().collect();
                break;
            }
            "launcher" => out.launcher_preview = true,
            "keymap" => out.keymap = true,
            "apps" => out.apps = true,
            "native-terminal" => out.native_terminal = true,
            "native-browser" => out.native_browser = true,
            "--limit" => {
                let v = args.next().ok_or_else(|| anyhow!("--limit N"))?;
                out.apps_limit = Some(v.parse().map_err(|_| anyhow!("--limit expects integer"))?);
            }
            "--filter" => {
                out.apps_filter = Some(args.next().ok_or_else(|| anyhow!("--filter QUERY"))?);
            }
            "--first" => out.apps_first = true,
            "--launch-first" => out.apps_launch_first = true,
            "--select" => {
                let v = args.next().ok_or_else(|| anyhow!("--select N"))?;
                out.launcher_select =
                    Some(v.parse().map_err(|_| anyhow!("--select expects integer"))?);
            }
            "--launch-selection" => out.launcher_launch_selection = true,
            "--seconds" => {
                let v = args.next().ok_or_else(|| anyhow!("--seconds N"))?;
                out.bench_seconds = Some(
                    v.parse()
                        .map_err(|_| anyhow!("--seconds expects integer"))?,
                );
            }
            "--frames" => {
                let v = args.next().ok_or_else(|| anyhow!("--frames N"))?;
                out.record_frames =
                    Some(v.parse().map_err(|_| anyhow!("--frames expects integer"))?);
            }
            "--out" => {
                let v = args.next().ok_or_else(|| anyhow!("--out PATH"))?;
                out.record_out = Some(v.clone());
                out.native_out = Some(v);
            }
            "--apng" => out.record_apng = true,
            "--delay-ms" => {
                let v = args.next().ok_or_else(|| anyhow!("--delay-ms N"))?;
                out.record_delay_ms = Some(
                    v.parse()
                        .map_err(|_| anyhow!("--delay-ms expects integer"))?,
                );
            }
            "--json" => out.json = true,
            "--keymap" => {
                out.keymap_path = Some(args.next().ok_or_else(|| anyhow!("--keymap PATH"))?);
            }
            "--check" => out.keymap_check = true,
            "-c" | "--command" => {
                out.attach_command = Some(args.next().ok_or_else(|| anyhow!("--command CMD"))?);
            }
            "--serve" => out.mode = Mode::Serve,
            "--attach" => out.mode = Mode::Attach,
            "--launch-on-f12" => out.launch_on_f12 = true,
            "--launcher-query" => {
                out.launcher_query = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--launcher-query QUERY"))?,
                );
            }
            "--url" => {
                out.native_url = Some(args.next().ok_or_else(|| anyhow!("--url URL"))?);
            }
            "--save-session" => {
                out.save_session = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--save-session PATH|-"))?,
                );
            }
            "--restore-session" => {
                out.restore_session = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--restore-session PATH|-"))?,
                );
            }
            "--send-text" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-text WINDOW TEXT"))?;
                let text = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-text WINDOW TEXT"))?;
                out.automation_request = Some(automation_request("SEND_TEXT", &window, &text)?);
            }
            "--send-line" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-line WINDOW TEXT"))?;
                let text = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-line WINDOW TEXT"))?;
                out.automation_request = Some(automation_request("SEND_LINE", &window, &text)?);
            }
            "--send-key" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-key WINDOW KEY"))?;
                let key = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-key WINDOW KEY"))?;
                out.automation_request = Some(automation_request("SEND_KEY", &window, &key)?);
            }
            "--read-text" => {
                let window = args.next().ok_or_else(|| anyhow!("--read-text WINDOW"))?;
                out.automation_request = Some(automation_request("READ_TEXT", &window, "")?);
            }
            "--wait-text" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text WINDOW NEEDLE"))?;
                out.automation_request = Some(automation_request("WAIT_TEXT", &window, &needle)?);
            }
            "--wait-text-ms" => {
                let ms = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-ms MS WINDOW NEEDLE"))?;
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-ms MS WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-ms MS WINDOW NEEDLE"))?;
                out.automation_request = Some(wait_text_ms_request(&ms, &window, &needle)?);
            }
            "--status-json" => out.automation_request = Some("STATUS_JSON".to_string()),
            "--panes" => out.automation_request = Some("PANES".to_string()),
            "--panes-json" => out.automation_request = Some("PANES_JSON".to_string()),
            "--session-json" => out.automation_request = Some("SESSION_JSON".to_string()),
            "--spawn-pty" => {
                let cmd = args.next().ok_or_else(|| anyhow!("--spawn-pty CMD"))?;
                out.automation_request = Some(protocol_payload_request("SPAWN_PTY", &cmd)?);
            }
            "--focus-pane" => {
                let window = args.next().ok_or_else(|| anyhow!("--focus-pane WINDOW"))?;
                out.automation_request = Some(protocol_token_request("FOCUS_PANE", &window)?);
            }
            "--focus-next" => out.automation_request = Some("FOCUS_NEXT".to_string()),
            "--focus-prev" => out.automation_request = Some("FOCUS_PREV".to_string()),
            "--close-pane" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--close-pane WINDOW|focused"))?;
                out.automation_request = Some(protocol_token_request("CLOSE_PANE", &window)?);
            }
            "--layout" => {
                let axis = args
                    .next()
                    .ok_or_else(|| anyhow!("--layout columns|rows"))?;
                out.automation_request = Some(layout_request(&axis)?);
            }
            "--move-pane" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--move-pane WINDOW|focused DIR"))?;
                let direction = args
                    .next()
                    .ok_or_else(|| anyhow!("--move-pane WINDOW|focused DIR"))?;
                out.automation_request = Some(move_pane_request(&window, &direction)?);
            }
            "--resize-pane" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--resize-pane WINDOW|focused AMOUNT"))?;
                let amount = args
                    .next()
                    .ok_or_else(|| anyhow!("--resize-pane WINDOW|focused AMOUNT"))?;
                out.automation_request = Some(resize_pane_request(&window, &amount)?);
            }
            "--balance-panes" => out.automation_request = Some("BALANCE_PANES".to_string()),
            "--rename-pane" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--rename-pane WINDOW TITLE"))?;
                let title = args
                    .next()
                    .ok_or_else(|| anyhow!("--rename-pane WINDOW TITLE"))?;
                out.automation_request = Some(rename_pane_request(&window, &title)?);
            }
            "--launcher-overlay" => out.launcher_overlay = true,
            "--no-launcher-overlay" => out.no_launcher_overlay = true,
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
                let n: u32 = v
                    .parse()
                    .map_err(|_| anyhow!("--fps expects an integer, got {v:?}"))?;
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
        "kittwm — kittui window manager\n\n\
         Usage: kittwm [--serve | --attach | --kill | --status] [--backend fake|quartz|xvfb]\n\n\
         Default: open a kittui-native PTY terminal in the current terminal.\n\
         The child receives KITTWM_SOCKET, KITTWM_DISPLAY, and KITTWM_WINDOW.\n\
         Use --backend fake|quartz|xvfb for capture-backed modes. Ctrl-] exits.\n\n\
         --serve   run as a Unix-socket daemon at $KITTWM_SOCK\n\
                   (default /tmp/kittwm-$USER.sock). Blocks until QUIT or\n\
                   SIGINT/SIGTERM. RAII socket cleanup.\n\
         --attach  connect to a running daemon and open an interactive REPL.\n\
                   Commands: PING STATUS WINDOWS DISPLAYS HELP QUIT.\n\
                   Pass -c/--command CMD for one-shot scripting mode.\n\
         --kill    send QUIT to the running daemon.\n\
         --status  print pid/uptime/sock of the running daemon; exits 1 if\n\
                   no daemon is reachable.\n\
         --save-session PATH|-    write native SESSION_JSON from the running socket.\n\
         --restore-session PATH|- read SESSION_JSON and queue RESTORE_SESSION_JSON.\n\
         --send-text WINDOW TEXT  send text bytes to a native pane.\n\
         --send-line WINDOW TEXT  send text plus newline to a native pane.\n\
         --send-key WINDOW KEY    send a named key (ctrl-c, escape, arrows, ...).\n\
         --read-text WINDOW       print a native pane text snapshot.\n\
         --wait-text WINDOW TEXT  wait until pane text contains TEXT.\n\
         --wait-text-ms MS WINDOW TEXT  wait with explicit millisecond timeout.\n\
         --status-json            print native socket STATUS_JSON.\n\
         --panes                  print native socket PANES listing.\n\
         --panes-json             print native socket PANES_JSON.\n\
         --session-json           print native socket SESSION_JSON.\n\
         --spawn-pty CMD          spawn a native PTY pane.\n\
         --focus-pane WINDOW      focus a pane by token.\n\
         --focus-next / --focus-prev cycle native pane focus.\n\
         --close-pane WINDOW      close a pane (or focused).\n\
         --layout columns|rows    switch native pane layout axis.\n\
         --move-pane WINDOW DIR   move pane left/right/up/down/first/last.\n\
         --resize-pane WINDOW N   resize pane weight (grow/shrink/+N/-N).\n\
         --balance-panes          equalize native pane weights.\n\
         --rename-pane WINDOW TITLE set native pane display title.\n\
         --backend fake|quartz|xvfb force a specific backend.\n\
         --pick-window   (macOS+quartz) live picker over CGWindowList; pick\n\
                         one window, then run a kittwm session capturing only it.\n\
         --list-windows  (macOS+quartz) print every titled CGWindow with id,\n\
                         bounds, owner, title — useful for scripting kittwm.\n\
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
         config          inspect resolved kittwm config env/paths and keymap\n\
                         validation status.\n\
         record          capture N frames from --capture/--backend target and\n\
                         write them as PNG files to --out DIR (default\n\
                         /tmp/kittwm-record-<unix-ts>). Defaults to 30 frames.\n\
                         Pass --apng to emit a single animated PNG at\n\
                         <out>/kittwm.apng (use --delay-ms N for cadence,\n\
                         default 33ms). Never enters raw mode.\n\
         bench           measure capture-pipeline throughput. Runs raw_frames\n\
                         in a tight loop for --seconds N (default 3) against\n\
                         --capture target and prints captures/s + p50/p95/p99\n\
                         latency + MB/s. --json for machine-readable output.\n\
         launch          spawn xterm by default, or run CMD ARGS after\n\
                         'kittwm launch -- CMD ARGS'. Prints pid + argv.\n\
         replace         when inside KITTWM_WINDOW, exec a command in-place.\n\
                         'kittwm replace browser URL' execs kittwm-browser.\n\
         launcher        render a boxed, numbered launcher preview using\n\
                         the same --filter/--limit candidate source. Use\n\
                         --select N to highlight a row and --launch-selection\n\
                         to spawn that selected candidate.\n\
         apps            list launch candidates from PATH and /Applications\n\
                         (macOS). Shows default launcher resolution. Use\n\
                         --limit N to bound output (default 50), --filter\n\
                         QUERY to case-insensitively narrow candidates,\n\
                         --first to print the first match, --launch-first\n\
                         to spawn it (PATH command or macOS app).\n\
         --launch-on-f12 intercept F12 in a running session and spawn\n\
                         KITTWM_LAUNCH_CMD via /bin/sh -c (default: xterm).\n\
                         Footer shows last_launch_pid and log records result.\n\
         --launcher-query QUERY make runtime launch actions pick the first\n\
                         matching PATH/macOS app candidate instead of the\n\
                         fixed KITTWM_LAUNCH_CMD fallback.\n\
         --launcher-overlay open an in-session boxed launcher overlay for\n\
                         launch actions; type filters, Enter launches, Esc closes.\n\
                         Enabled by default. Pass --no-launcher-overlay or\n\
                         set KITTUI_WM_LAUNCHER_OVERLAY=0 to keep immediate-spawn.\n\
         native-terminal run a backend-independent PTY proof: spawn `cat`,\n\
                         type through the PTY, render an RGBA terminal frame.\n\
         native-browser  run a backend-independent headless Chrome proof.\n\
                         Pass --url URL (default: data: page) and --out PNG.\n\
         keymap          print resolved keybinding config. Defaults to the\n\
                         built-in tmux-like Ctrl-A prefix map; pass\n\
                         --keymap PATH to parse and print a custom file.\n\
                         Use --check to validate duplicates/custom actions.\n\
                         Sessions also load --keymap PATH / KITTUI_WM_KEYMAP.\n"
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
    #[cfg(not(any(all(target_os = "macos", feature = "quartz"), feature = "xvfb")))]
    {
        Backend::Fake
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kittwm: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let cli = parse_args()?;
    if let Some(fps) = cli.fps {
        std::env::set_var("KITTUI_WM_FPS", fps.to_string());
    }
    if cli.launch_on_f12 {
        std::env::set_var("KITTUI_WM_LAUNCH_ON_F12", "1");
    }
    if let Some(query) = &cli.launcher_query {
        std::env::set_var("KITTUI_WM_LAUNCH_QUERY", query);
    }
    if cli.launcher_overlay {
        std::env::set_var("KITTUI_WM_LAUNCHER_OVERLAY", "1");
    }
    if cli.no_launcher_overlay {
        std::env::set_var("KITTUI_WM_LAUNCHER_OVERLAY", "0");
    }
    if let Some(path) = &cli.keymap_path {
        std::env::set_var("KITTUI_WM_KEYMAP", path);
    }

    // Inspection flags run cooked, never enter raw mode.
    if cli.doctor {
        return doctor_cmd(cli.json);
    }
    if cli.config {
        return config_cmd(&cli);
    }
    if cli.record {
        return record_cmd(&cli);
    }
    if cli.bench {
        return bench_cmd(&cli);
    }
    if cli.launch {
        return launch_cmd(&cli);
    }
    if cli.replace {
        return replace_cmd(&cli);
    }
    if cli.launcher_preview {
        return launcher_preview_cmd(&cli);
    }
    if cli.keymap {
        return keymap_cmd(&cli);
    }
    if cli.native_terminal {
        return native_terminal_cmd();
    }
    if cli.native_browser {
        return native_browser_cmd(&cli);
    }
    if cli.apps {
        return apps_cmd(&cli);
    }
    if cli.list_windows {
        return list_windows_cmd();
    }
    if cli.list_displays {
        return list_displays_cmd();
    }
    if let Some(path) = &cli.save_session {
        return save_session_cmd(path);
    }
    if let Some(path) = &cli.restore_session {
        return restore_session_cmd(path);
    }
    if let Some(request) = &cli.automation_request {
        return automation_cmd(request);
    }

    match cli.mode {
        Mode::Session => run_session(cli),
        Mode::Serve => serve_cmd(cli),
        Mode::Attach => attach_cmd(cli.attach_command.as_deref()),
        Mode::Kill => kill_cmd(),
        Mode::Status => status_cmd(),
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
    Err(anyhow!(
        "--list-windows requires --features quartz on macOS"
    ))
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn list_displays_cmd() -> Result<()> {
    use kittui_quartz::QuartzServer;
    let displays = QuartzServer::displays();
    println!("{:>3}  {:>10}  bounds", "#", "id");
    for d in displays {
        println!(
            "{:>3}  {:>10}  ({:.0},{:.0}) {:.0}x{:.0}",
            d.index, d.id, d.bounds.origin.0, d.bounds.origin.1, d.bounds.width, d.bounds.height,
        );
    }
    Ok(())
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn list_displays_cmd() -> Result<()> {
    Err(anyhow!(
        "--list-displays requires --features quartz on macOS"
    ))
}

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
    if cli.backend.is_none() && !cli.pick_window && cli.capture.is_none() {
        return kittui_cli::session::run_native_terminal_loop(&runtime);
    }
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
    // Show a tiny gallery so `kittwm` (no args) always renders something
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
    layout.tile(XWindowId(1), PxRect::new(8.0, 16.0, 320.0, 192.0));
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
            "kittwm: capturing window {} ({}: {})",
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

    eprintln!("kittwm: probing macOS Screen Recording permission...");
    let probe = server.windows().and_then(|w| {
        if let Some(first) = w.first() {
            server.capture(first.id).map(|_| ())
        } else {
            Err(kittui_xvfb::XError::Unavailable("no displays".into()))
        }
    });
    if let Err(e) = probe {
        return Err(anyhow!(
            "kittwm could not capture the screen: {e}\n\n  Grant Screen Recording \
             to your terminal under System Settings -> Privacy & Security -> \
             Screen Recording, then quit and relaunch the terminal."
        ));
    }
    eprintln!("kittwm: backend ready. q/Esc to quit.");
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
    println!("\nkittwm --pick-window\n");
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
                    "no Mac window matched 'window:{needle}'; run `kittwm --list-windows` to see candidates"
                )
            })?;
        eprintln!(
            "kittwm: --capture window:{} matched id={} owner={:?} title={:?}",
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

    let log_path =
        std::env::var("KITTUI_WM_LOG").unwrap_or_else(|_| "/tmp/kittui-wm.log".to_string());
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
        buf.push_str(&format!(
            "  \"kitty_graphics_likely\": {},\n",
            kitty_graphics
        ));
        buf.push_str(&format!("  \"display_count\": {},\n", display_count));
        buf.push_str(&format!("  \"log_path\": {:?},\n", log_path));
        buf.push_str(&format!("  \"log_present\": {},\n", log_present));
        buf.push_str(&format!("  \"log_size_bytes\": {}\n", log_size));
        buf.push_str("}\n");
        print!("{buf}");
    } else {
        println!("kittwm doctor");
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
            if kitty_graphics {
                "likely yes"
            } else {
                "unknown"
            }
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
            println!("Hint: SCK + CGEventPost both require Screen Recording + Accessibility");
            println!("      permissions on the terminal hosting kittwm (System Settings >");
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
    let out_dir = cli.record_out.clone().unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("/tmp/kittwm-record-{ts}")
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

    eprintln!("kittwm record: writing {frames_target} frames to {out_dir}");
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
        let path = format!("{out_dir}/kittwm.apng");
        std::fs::write(&path, bytes)?;
        eprintln!("  wrote APNG: {path}");
    }
    let elapsed = started.elapsed();
    eprintln!(
        "kittwm record: done. {} frames in {:.2}s ({:.1} fps). dir={}",
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

    eprintln!("kittwm bench: measuring for {secs}s ...");
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
        if latencies_us.is_empty() {
            return 0;
        }
        let idx = ((latencies_us.len() as f64 - 1.0) * p).round() as usize;
        latencies_us[idx]
    };
    let mean = if latencies_us.is_empty() {
        0
    } else {
        latencies_us.iter().sum::<u64>() / latencies_us.len() as u64
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
        println!("kittwm bench");
        println!("===========");
        println!("  duration       : {:.3} s", wall.as_secs_f32());
        println!("  captures       : {}", iters);
        println!("  captures/s     : {:.1}", captures_per_s);
        println!("  surface        : {}x{} RGBA", first_dims.0, first_dims.1);
        println!(
            "  bytes captured : {:.1} MB",
            total_bytes as f64 / 1_048_576.0
        );
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

fn serve_cmd(_cli: Cli) -> Result<()> {
    use kittui_cli::daemon::{default_socket_path, DaemonServer};
    let path = default_socket_path();
    let server = DaemonServer::bind(path).map_err(|e| anyhow!("kittwm --serve: {e}"))?;
    eprintln!(
        "kittwm: daemon listening on {} (pid={}). Send QUIT or SIGINT to exit.",
        server.path().display(),
        std::process::id()
    );
    // Block until QUIT received or signal.
    use std::sync::atomic::{AtomicBool, Ordering};
    static GOT_SIGNAL: AtomicBool = AtomicBool::new(false);
    extern "C" fn on_signal(_: libc::c_int) {
        GOT_SIGNAL.store(true, Ordering::SeqCst);
    }
    unsafe {
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
    while !server.quit_requested() && !GOT_SIGNAL.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("kittwm: daemon shutting down.");
    Ok(())
}

fn normalize_daemon_command(cmd: &str) -> String {
    let trimmed = cmd.trim();
    let Some((verb, rest)) = trimmed.split_once(char::is_whitespace) else {
        return trimmed.to_ascii_uppercase();
    };
    format!("{} {}", verb.to_ascii_uppercase(), rest.trim_start())
}

fn protocol_token(token: &str, label: &str) -> Result<String> {
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(anyhow!("{label} must be a single nonempty token"));
    }
    Ok(token.to_string())
}

fn protocol_payload_request(verb: &str, payload: &str) -> Result<String> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Err(anyhow!("{verb} requires a nonempty payload"));
    }
    Ok(format!("{} {payload}", verb.trim().to_ascii_uppercase()))
}

fn protocol_token_request(verb: &str, token: &str) -> Result<String> {
    Ok(format!(
        "{} {}",
        verb.trim().to_ascii_uppercase(),
        protocol_token(token, "argument")?
    ))
}

fn automation_request(verb: &str, window: &str, payload: &str) -> Result<String> {
    let window = protocol_token(window, "automation window")?;
    let verb = verb.trim().to_ascii_uppercase();
    if payload.is_empty() {
        Ok(format!("{verb} {window}"))
    } else {
        Ok(format!("{verb} {window} {payload}"))
    }
}

fn layout_request(axis: &str) -> Result<String> {
    let axis = axis.trim().to_ascii_lowercase();
    if !matches!(axis.as_str(), "columns" | "rows") {
        return Err(anyhow!("--layout expects columns or rows"));
    }
    Ok(format!("LAYOUT {axis}"))
}

fn move_pane_request(window: &str, direction: &str) -> Result<String> {
    let window = protocol_token(window, "window")?;
    let direction = direction.trim().to_ascii_lowercase();
    if !matches!(
        direction.as_str(),
        "left" | "right" | "up" | "down" | "first" | "last"
    ) {
        return Err(anyhow!(
            "--move-pane direction expects left|right|up|down|first|last"
        ));
    }
    Ok(format!("MOVE_PANE {window} {direction}"))
}

fn resize_pane_request(window: &str, amount: &str) -> Result<String> {
    let window = protocol_token(window, "window")?;
    let amount = protocol_token(amount, "resize amount")?;
    Ok(format!("RESIZE_PANE {window} {amount}"))
}

fn rename_pane_request(window: &str, title: &str) -> Result<String> {
    let window = protocol_token(window, "window")?;
    protocol_payload_request("RENAME_PANE", &format!("{window} {title}"))
}

fn wait_text_ms_request(ms: &str, window: &str, needle: &str) -> Result<String> {
    let parsed = ms
        .trim()
        .parse::<u64>()
        .map_err(|_| anyhow!("--wait-text-ms expects integer milliseconds"))?;
    if parsed == 0 || parsed > 60_000 {
        return Err(anyhow!("--wait-text-ms must be in 1..=60000"));
    }
    automation_request("WAIT_TEXT_MS", window, &format!("{parsed} {needle}"))
}

fn automation_cmd(request: &str) -> Result<()> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let reply = client_request_multi(&path, request).map_err(|e| {
        anyhow!(
            "could not send automation request to {}: {e}",
            path.display()
        )
    })?;
    print!("{reply}");
    if !reply.ends_with('\n') {
        println!();
    }
    if reply.starts_with("ERR ") {
        std::process::exit(2);
    }
    Ok(())
}

fn save_session_cmd(path_arg: &str) -> Result<()> {
    use kittui_cli::daemon::{client_request, default_socket_path};
    let path = default_socket_path();
    let reply = client_request(&path, "SESSION_JSON")
        .map_err(|e| anyhow!("could not read SESSION_JSON from {}: {e}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&reply)
        .map_err(|e| anyhow!("daemon returned invalid SESSION_JSON: {e}"))?;
    let pretty = serde_json::to_string_pretty(&value)?;
    if path_arg == "-" {
        println!("{pretty}");
    } else {
        std::fs::write(path_arg, format!("{pretty}\n"))?;
    }
    Ok(())
}

fn restore_session_cmd(path_arg: &str) -> Result<()> {
    use kittui_cli::daemon::{client_request, default_socket_path};
    use std::io::Read as _;
    let mut input = String::new();
    if path_arg == "-" {
        std::io::stdin().read_to_string(&mut input)?;
    } else {
        input = std::fs::read_to_string(path_arg)?;
    }
    let request = restore_session_request(&input)?;
    let path = default_socket_path();
    let reply = client_request(&path, &request)
        .map_err(|e| anyhow!("could not queue restore on {}: {e}", path.display()))?;
    print!("{reply}");
    if !reply.ends_with('\n') {
        println!();
    }
    if reply.starts_with("ERR ") {
        std::process::exit(2);
    }
    Ok(())
}

fn restore_session_request(input: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| anyhow!("--restore-session expects valid SESSION_JSON: {e}"))?;
    Ok(format!(
        "RESTORE_SESSION_JSON {}",
        serde_json::to_string(&value)?
    ))
}

fn status_cmd() -> Result<()> {
    use kittui_cli::daemon::{client_request, default_socket_path};
    let path = default_socket_path();
    match client_request(&path, "STATUS") {
        Ok(reply) => {
            print!("kittwm daemon: {reply}");
            Ok(())
        }
        Err(_) => {
            println!(
                "kittwm: no daemon listening on {} (try `kittwm --serve` to start one).",
                path.display()
            );
            std::process::exit(1);
        }
    }
}

fn kill_cmd() -> Result<()> {
    use kittui_cli::daemon::{client_request, default_socket_path};
    let path = default_socket_path();
    match client_request(&path, "QUIT") {
        Ok(reply) => {
            print!("{reply}");
            Ok(())
        }
        Err(e) => Err(anyhow!("no daemon to kill at {}: {e}", path.display())),
    }
}

fn attach_cmd(command: Option<&str>) -> Result<()> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    use std::io::{BufRead, Write};
    let path = default_socket_path();
    // Probe first so we fail fast if no daemon.
    let probe = client_request_multi(&path, "PING")
        .map_err(|e| anyhow!("no daemon at {}: {e}", path.display()))?;
    if let Some(command) = command {
        let reply = client_request_multi(&path, &normalize_daemon_command(command))?;
        print!("{reply}");
        if !reply.ends_with('\n') {
            println!();
        }
        if reply.starts_with("ERR ") {
            std::process::exit(2);
        }
        return Ok(());
    }
    eprintln!(
        "kittwm --attach: connected to {} ({})",
        path.display(),
        probe.trim()
    );
    eprintln!(
        "Commands: PING STATUS PANES SPAWN <argv> WINDOWS DISPLAYS HELP QUIT (Ctrl-D to detach)"
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    loop {
        {
            let mut w = stdout.lock();
            write!(w, "kittwm> ")?;
            w.flush()?;
        }
        let mut line = String::new();
        let n = stdin.lock().read_line(&mut line)?;
        if n == 0 {
            eprintln!();
            break;
        }
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }
        if cmd.eq_ignore_ascii_case("detach") || cmd.eq_ignore_ascii_case("exit") {
            break;
        }
        match client_request_multi(&path, &normalize_daemon_command(cmd)) {
            Ok(reply) => {
                print!("{reply}");
                if !reply.ends_with('\n') {
                    println!();
                }
            }
            Err(e) => {
                eprintln!("(daemon error: {e})");
                // Daemon likely died — exit.
                if let Err(_) = client_request_multi(&path, "PING") {
                    eprintln!("daemon unreachable; detaching.");
                    break;
                }
            }
        }
        if cmd.eq_ignore_ascii_case("QUIT") {
            break;
        }
    }
    Ok(())
}

fn launch_cmd(cli: &Cli) -> Result<()> {
    let mut argv: Vec<String> = cli.launch_args.clone();
    if argv.first().map(|s| s.as_str()) == Some("--") {
        argv.remove(0);
    }
    if argv.is_empty() {
        argv.push("xterm".to_string());
    }
    let program = argv[0].clone();
    let args = &argv[1..];
    let child = std::process::Command::new(&program)
        .args(args)
        .spawn()
        .map_err(|e| anyhow!("launch {:?}: {e}", argv))?;
    println!("kittwm launch: pid={} argv={:?}", child.id(), argv);
    Ok(())
}

fn keymap_cmd(cli: &Cli) -> Result<()> {
    let km = if let Some(path) = &cli.keymap_path {
        kittui_cli::keymap::Keymap::load(std::path::Path::new(path))?
    } else {
        kittui_cli::keymap::default_keymap()
    };
    if cli.keymap_check {
        return keymap_check_cmd(&km);
    }
    print!("{}", km.render_table());
    Ok(())
}

fn keymap_check_cmd(km: &kittui_cli::keymap::Keymap) -> Result<()> {
    let mut seen = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut custom = Vec::<String>::new();
    for binding in &km.bindings {
        let chord = binding.chord_string();
        seen.entry(chord)
            .or_default()
            .push(binding.action.to_string());
        if binding.action.to_string().contains('.')
            && !matches!(
                binding.action,
                kittui_cli::keymap::Action::WorkspaceNew
                    | kittui_cli::keymap::Action::WorkspaceNext
                    | kittui_cli::keymap::Action::WorkspacePrev
                    | kittui_cli::keymap::Action::SplitVerticalLauncher
                    | kittui_cli::keymap::Action::SplitHorizontalLauncher
                    | kittui_cli::keymap::Action::FullscreenToggle
                    | kittui_cli::keymap::Action::FloatToggle
                    | kittui_cli::keymap::Action::ToggleSplit
                    | kittui_cli::keymap::Action::BalanceWindows
                    | kittui_cli::keymap::Action::FocusLeft
                    | kittui_cli::keymap::Action::FocusRight
                    | kittui_cli::keymap::Action::FocusUp
                    | kittui_cli::keymap::Action::FocusDown
                    | kittui_cli::keymap::Action::SwapLeft
                    | kittui_cli::keymap::Action::SwapRight
                    | kittui_cli::keymap::Action::SwapUp
                    | kittui_cli::keymap::Action::SwapDown
            )
        {
            if matches!(binding.action, kittui_cli::keymap::Action::Custom(_)) {
                custom.push(binding.action.to_string());
            }
        }
    }
    let duplicates: Vec<_> = seen
        .iter()
        .filter(|(_, actions)| actions.len() > 1)
        .collect();
    println!("kittwm keymap check");
    println!("==================");
    println!(
        "prefix: {}",
        km.prefix
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!("bindings: {}", km.bindings.len());
    println!("duplicate_chords: {}", duplicates.len());
    for (chord, actions) in duplicates {
        println!("  {chord}: {}", actions.join(", "));
    }
    println!("custom_actions: {}", custom.len());
    for action in custom {
        println!("  {action}");
    }
    if !seen.iter().any(|(_, actions)| actions.len() > 1) {
        println!("status: ok");
        Ok(())
    } else {
        println!("status: duplicate chords found");
        std::process::exit(2);
    }
}

fn apps_cmd(cli: &Cli) -> Result<()> {
    let limit = cli.apps_limit.unwrap_or(50);
    let default_cmd = kittui_cli::session::launcher_command();
    let default_prog = default_cmd.split_whitespace().next().unwrap_or("xterm");
    let default_path = find_on_path(default_prog);
    let query = cli.apps_filter.as_deref();
    let path_cmds = filter_candidates(path_commands(5000), query, limit);
    #[cfg(target_os = "macos")]
    let mac_apps = filter_candidates(macos_apps(5000), query, limit);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();
    if cli.apps_first || cli.apps_launch_first {
        let selected = first_app_candidate(&path_cmds, &mac_apps)
            .ok_or_else(|| anyhow!("no app candidates matched"))?;
        if cli.apps_launch_first {
            let pid = launch_app_candidate(&selected)?;
            println!(
                "kittwm apps: launched pid={} kind={} name={}",
                pid, selected.kind, selected.name
            );
        } else {
            println!("{}:{}", selected.kind, selected.name);
        }
        return Ok(());
    }
    if cli.json {
        println!(
            "{{\"default_command\": {:?}, \"default_resolved\": {}, \"path_commands\": [{}], \"macos_apps\": [{}]}}",
            default_cmd,
            default_path
                .as_ref()
                .map(|p| format!("{:?}", p.display().to_string()))
                .unwrap_or_else(|| "null".to_string()),
            json_string_array(&path_cmds),
            json_string_array(&mac_apps),
        );
        return Ok(());
    }
    println!("kittwm apps");
    println!("==========");
    println!("default: {default_cmd}");
    println!(
        "default_resolved: {}",
        default_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<not found on PATH>".to_string())
    );
    println!();
    if let Some(q) = query {
        println!("filter: {q}");
    }
    println!("PATH commands (first {limit}):");
    for cmd in &path_cmds {
        println!("  {cmd}");
    }
    #[cfg(target_os = "macos")]
    {
        println!();
        println!("macOS applications (first {limit}):");
        for app in &mac_apps {
            println!("  {app}");
        }
    }
    Ok(())
}

fn find_on_path(program: &str) -> Option<std::path::PathBuf> {
    if program.contains('/') {
        let p = std::path::PathBuf::from(program);
        return p.exists().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(program);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn path_commands(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(read) = std::fs::read_dir(dir) else {
                continue;
            };
            for ent in read.flatten() {
                let path = ent.path();
                if !path.is_file() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                out.insert(name.to_string());
                if out.len() >= limit {
                    break;
                }
            }
            if out.len() >= limit {
                break;
            }
        }
    }
    out.into_iter().take(limit).collect()
}

#[cfg(target_os = "macos")]
fn macos_apps(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for root in ["/Applications", "/System/Applications"] {
        let Ok(read) = std::fs::read_dir(root) else {
            continue;
        };
        for ent in read.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("app") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            out.insert(name.trim_end_matches(".app").to_string());
            if out.len() >= limit {
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.into_iter().take(limit).collect()
}

fn json_string_array(items: &[String]) -> String {
    items
        .iter()
        .map(|s| format!("{:?}", s))
        .collect::<Vec<_>>()
        .join(", ")
}

fn filter_candidates(items: Vec<String>, query: Option<&str>, limit: usize) -> Vec<String> {
    let Some(query) = query else {
        return items.into_iter().take(limit).collect();
    };
    let q = query.to_ascii_lowercase();
    let mut scored: Vec<(u8, String)> = items
        .into_iter()
        .filter_map(|item| candidate_match_score(&item, &q).map(|score| (score, item)))
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| a_score.cmp(b_score).then_with(|| a.cmp(b)));
    scored
        .into_iter()
        .map(|(_, item)| item)
        .take(limit)
        .collect()
}

fn candidate_match_score(item: &str, lower_query: &str) -> Option<u8> {
    let lower_item = item.to_ascii_lowercase();
    if lower_item == lower_query {
        Some(0)
    } else if lower_item.starts_with(lower_query) {
        Some(1)
    } else if lower_item.contains(lower_query) {
        Some(2)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct AppCandidate {
    kind: &'static str,
    name: String,
}

fn first_app_candidate(path_cmds: &[String], mac_apps: &[String]) -> Option<AppCandidate> {
    path_cmds
        .first()
        .map(|name| AppCandidate {
            kind: "path",
            name: name.clone(),
        })
        .or_else(|| {
            mac_apps.first().map(|name| AppCandidate {
                kind: "macos",
                name: name.clone(),
            })
        })
}

fn launch_app_candidate(candidate: &AppCandidate) -> Result<u32> {
    let mut cmd = if candidate.kind == "macos" {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(&candidate.name);
        c
    } else {
        std::process::Command::new(&candidate.name)
    };
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            anyhow!(
                "launch candidate {}:{}: {e}",
                candidate.kind,
                candidate.name
            )
        })?;
    Ok(child.id())
}

fn replace_cmd(cli: &Cli) -> Result<()> {
    match resolve_replace_action(&cli.replace_args, std::env::var("KITTWM_WINDOW").is_ok())? {
        ReplaceAction::Spawn { request } => {
            let sock = std::env::var("KITTWM_SOCKET").unwrap_or_else(|_| "<unset>".to_string());
            let path = std::path::PathBuf::from(sock.clone());
            let reply = kittui_cli::daemon::client_request(&path, &request)?;
            print!("{reply}");
            Ok(())
        }
        ReplaceAction::Exec { argv } => exec_replace_argv(&argv),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplaceAction {
    Spawn { request: String },
    Exec { argv: Vec<String> },
}

fn resolve_replace_action(args: &[String], in_window: bool) -> Result<ReplaceAction> {
    if args.is_empty() {
        return Err(anyhow!("usage: kittwm replace <command|browser> [args...]"));
    }
    let argv = resolve_replace_argv(args);
    if in_window {
        Ok(ReplaceAction::Exec { argv })
    } else {
        Ok(ReplaceAction::Spawn {
            request: format!("SPAWN {}", argv_to_shell_words(&argv)),
        })
    }
}

fn resolve_replace_argv(args: &[String]) -> Vec<String> {
    let mut argv = args.to_vec();
    if argv.first().is_some_and(|arg| arg == "browser") {
        argv[0] = "kittwm-browser".to_string();
    }
    argv
}

fn exec_replace_argv(argv: &[String]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&argv[0]).args(&argv[1..]).exec();
        Err(anyhow!("exec {:?}: {err}", argv))
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn argv_to_shell_words(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_/.:".contains(c))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn launcher_preview_cmd(cli: &Cli) -> Result<()> {
    let limit = cli.apps_limit.unwrap_or(8).min(20);
    let query = cli.apps_filter.as_deref().unwrap_or("");
    let path_cmds = filter_candidates(path_commands(5000), cli.apps_filter.as_deref(), limit);
    #[cfg(target_os = "macos")]
    let mac_app_candidates = filter_candidates(macos_apps(5000), cli.apps_filter.as_deref(), limit);
    #[cfg(not(target_os = "macos"))]
    let mac_app_candidates: Vec<String> = Vec::new();
    let mut candidates: Vec<AppCandidate> = path_cmds
        .into_iter()
        .map(|name| AppCandidate { kind: "path", name })
        .chain(mac_app_candidates.into_iter().map(|name| AppCandidate {
            kind: "macos",
            name,
        }))
        .take(limit)
        .collect();
    if candidates.is_empty() {
        candidates.push(AppCandidate {
            kind: "none",
            name: "<no matches>".to_string(),
        });
    }
    let mut selected = cli.launcher_select.unwrap_or(1);
    if selected == 0 {
        selected = 1;
    }
    if selected > candidates.len() {
        selected = candidates.len();
    }
    let selected_idx = selected - 1;
    if cli.launcher_launch_selection {
        let candidate = &candidates[selected_idx];
        if candidate.kind == "none" {
            return Err(anyhow!("no launcher candidate selected"));
        }
        let pid = launch_app_candidate(candidate)?;
        println!(
            "kittwm launcher: launched selection={} pid={} kind={} name={}",
            selected, pid, candidate.kind, candidate.name
        );
        return Ok(());
    }

    let width = 62usize;
    println!("┌{}┐", "─".repeat(width));
    println!("│{:^width$}│", "kittwm launcher", width = width);
    println!("├{}┤", "─".repeat(width));
    println!("│ query: {:<qwidth$}│", query, qwidth = width - 8);
    println!("├{}┤", "─".repeat(width));
    for (idx, cand) in candidates.iter().enumerate() {
        let marker = if idx == selected_idx { "▶" } else { " " };
        let text = format!("{marker} {:>2}. [{:<5}] {}", idx + 1, cand.kind, cand.name);
        println!("│{:<width$}│", truncate(&text, width), width = width);
    }
    println!("├{}┤", "─".repeat(width));
    println!(
        "│ {:<w$}│",
        "Enter launches selection · Esc closes · type filters",
        w = width - 1
    );
    println!("└{}┘", "─".repeat(width));
    Ok(())
}

fn native_terminal_cmd() -> Result<()> {
    use kittui_wm::native::{NativeApp, NativeFrame, PtyTerminalApp};

    let mut term = PtyTerminalApp::spawn("cat", 40, 6)?;
    term.send_text("hello from kittwm native pty\n")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline
        && !term
            .text_snapshot()
            .contains("hello from kittwm native pty")
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let text = term.text_snapshot();
    let frame = term.capture()?;
    let NativeFrame::Rgba {
        width,
        height,
        rgba,
    } = frame
    else {
        return Err(anyhow!("native terminal returned non-RGBA frame"));
    };
    println!("kittwm native-terminal");
    println!("=======================");
    println!(
        "text_contains_hello: {}",
        text.contains("hello from kittwm native pty")
    );
    println!("frame: {width}x{height} rgba_bytes={}", rgba.len());
    print!("{text}");
    Ok(())
}

fn native_browser_cmd(cli: &Cli) -> Result<()> {
    use kittui_wm::native::{HeadlessBrowserApp, NativeApp, NativeFrame};

    let url = cli.native_url.as_deref().unwrap_or(
        "data:text/html,<html><body><h1>kittwm native browser</h1><input autofocus value='ready'></body></html>",
    );
    let mut browser = HeadlessBrowserApp::launch(url, 640, 360)?;
    browser.send_text(" typed")?;
    browser.click(20, 20)?;
    let frame = browser.capture()?;
    let NativeFrame::Png {
        width,
        height,
        bytes,
    } = frame
    else {
        return Err(anyhow!("native browser returned non-PNG frame"));
    };
    let out = cli
        .native_out
        .clone()
        .unwrap_or_else(|| format!("/tmp/kittwm-native-browser-{}.png", std::process::id()));
    std::fs::write(&out, &bytes)?;
    println!("kittwm native-browser");
    println!("======================");
    println!("url: {url}");
    println!(
        "screenshot: {width}x{height} bytes={} path={out}",
        bytes.len()
    );
    Ok(())
}

fn config_cmd(_cli: &Cli) -> Result<()> {
    let keymap_path = std::env::var("KITTUI_WM_KEYMAP").ok();
    let keymap = if let Some(path) = &keymap_path {
        kittui_cli::keymap::Keymap::load(std::path::Path::new(path))?
    } else {
        kittui_cli::keymap::default_keymap()
    };
    let duplicates = keymap_duplicate_count(&keymap);
    println!("kittwm config");
    println!("============");
    println!(
        "KITTUI_WM_KEYMAP       : {}",
        keymap_path.as_deref().unwrap_or("<default>")
    );
    println!(
        "KITTUI_WM_LAUNCH_CMD   : {}",
        std::env::var("KITTUI_WM_LAUNCH_CMD").unwrap_or_else(|_| "<default: xterm>".to_string())
    );
    println!(
        "KITTUI_WM_LAUNCH_QUERY : {}",
        std::env::var("KITTUI_WM_LAUNCH_QUERY").unwrap_or_else(|_| "<unset>".to_string())
    );
    println!(
        "KITTUI_WM_LAUNCHER_OVERLAY: {}",
        std::env::var("KITTUI_WM_LAUNCHER_OVERLAY").unwrap_or_else(|_| "<unset>".to_string())
    );
    println!(
        "prefix                 : {}",
        keymap
            .prefix
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!("bindings               : {}", keymap.bindings.len());
    println!("duplicate_chords       : {duplicates}");
    println!(
        "status                 : {}",
        if duplicates == 0 {
            "ok"
        } else {
            "duplicate chords found"
        }
    );
    Ok(())
}

fn keymap_duplicate_count(km: &kittui_cli::keymap::Keymap) -> usize {
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for binding in &km.bindings {
        *seen.entry(binding.chord_string()).or_default() += 1;
    }
    seen.values().filter(|&&n| n > 1).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn replace_browser_maps_to_kittwm_browser_for_exec() {
        let action = resolve_replace_action(&args(&["browser", "https://example.com"]), true)
            .expect("replace action");
        assert_eq!(
            action,
            ReplaceAction::Exec {
                argv: args(&["kittwm-browser", "https://example.com"])
            }
        );
    }

    #[test]
    fn replace_browser_maps_to_kittwm_browser_for_spawn_request() {
        let action = resolve_replace_action(&args(&["browser", "https://example.com/a b"]), false)
            .expect("replace action");
        assert_eq!(
            action,
            ReplaceAction::Spawn {
                request: "SPAWN kittwm-browser 'https://example.com/a b'".to_string()
            }
        );
    }

    #[test]
    fn replace_requires_a_command() {
        let err = resolve_replace_action(&[], true).unwrap_err();
        assert!(err.to_string().contains("usage: kittwm replace"), "{err}");
    }

    #[test]
    fn argv_to_shell_words_quotes_single_quotes() {
        let shell = argv_to_shell_words(&args(&["echo", "Bob's pane"]));
        assert_eq!(shell, "echo 'Bob'\\''s pane'");
    }

    #[test]
    fn normalize_daemon_command_uppercases_only_verb() {
        assert_eq!(normalize_daemon_command("status"), "STATUS");
        assert_eq!(
            normalize_daemon_command("spawn printf MixedCase"),
            "SPAWN printf MixedCase"
        );
        assert_eq!(
            normalize_daemon_command("apps_first Safari"),
            "APPS_FIRST Safari"
        );
    }

    #[test]
    fn pane_control_requests_validate_and_preserve_payloads() {
        assert_eq!(
            protocol_payload_request("spawn_pty", "htop").unwrap(),
            "SPAWN_PTY htop"
        );
        assert_eq!(
            protocol_token_request("focus_pane", "native-2").unwrap(),
            "FOCUS_PANE native-2"
        );
        assert_eq!(layout_request("ROWS").unwrap(), "LAYOUT rows");
        assert_eq!(
            move_pane_request("focused", "LAST").unwrap(),
            "MOVE_PANE focused last"
        );
        assert_eq!(
            resize_pane_request("focused", "+2").unwrap(),
            "RESIZE_PANE focused +2"
        );
        assert_eq!(
            rename_pane_request("native-2", "Editor Pane").unwrap(),
            "RENAME_PANE native-2 Editor Pane"
        );
        assert!(layout_request("diagonal").is_err());
        assert!(move_pane_request("bad window", "last").is_err());
    }

    #[test]
    fn normalize_daemon_command_preserves_json_inspection_verbs() {
        assert_eq!(normalize_daemon_command("status_json"), "STATUS_JSON");
        assert_eq!(normalize_daemon_command("panes_json"), "PANES_JSON");
        assert_eq!(normalize_daemon_command("session_json"), "SESSION_JSON");
    }

    #[test]
    fn automation_request_preserves_payload_case_and_spaces() {
        assert_eq!(
            automation_request("send_line", "focused", "echo Mixed Case").unwrap(),
            "SEND_LINE focused echo Mixed Case"
        );
        assert_eq!(
            automation_request("read_text", "native-2", "").unwrap(),
            "READ_TEXT native-2"
        );
        assert_eq!(
            wait_text_ms_request("2500", "focused", "Ready Now").unwrap(),
            "WAIT_TEXT_MS focused 2500 Ready Now"
        );
        assert!(wait_text_ms_request("0", "focused", "ready").is_err());
        assert!(automation_request("SEND_KEY", "bad window", "ctrl-c").is_err());
    }

    #[test]
    fn restore_session_request_compacts_pretty_json() {
        let request = restore_session_request(
            r#"{
              "layout": "rows",
              "panes": [
                { "command": "htop", "title": "htop", "weight": 2, "focused": true }
              ]
            }"#,
        )
        .unwrap();
        assert!(request.starts_with("RESTORE_SESSION_JSON {"), "{request}");
        assert!(!request.contains('\n'), "{request}");
        assert!(request.contains(r#""command":"htop""#), "{request}");
    }

    #[test]
    fn restore_session_request_rejects_invalid_json() {
        let err = restore_session_request("not json").unwrap_err();
        assert!(
            err.to_string()
                .contains("--restore-session expects valid SESSION_JSON"),
            "{err}"
        );
    }
}
