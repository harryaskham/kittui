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

use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use base64::Engine;

use kittui::{
    CellRect, CellSize, Corners, Layer, Node, Paint, PxRect as KittuiPxRect, Rgba, Runtime, Scene,
    Stroke, TerminalInfo, TransportDiagnostics,
};
use kittui_cli::update::{self as cli_update, UpdateAction, UpdateOptions};
use kittui_core::geom::PxRect;
use kittui_core::terminal::{
    read_kitty_response, KittyResponseReadConfig, KittyResponseReadStatus,
};
use kittui_kitty::{parse_response, query_capabilities, KittyResponseStatus};
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
#[cfg(all(target_os = "macos", feature = "quartz"))]
use kittui_xvfb::XServer;
use kittui_xvfb::{FakeServer, XWindowId};
use kittwm_sdk::{default_config_path as default_kittwm_config_path, KittwmConfig};

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
    doctor_scene_json: bool,
    doctor_kitty: bool,
    probe_kitty: bool,
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
    launcher_scene_json: bool,
    launcher_kitty: bool,
    launcher_select: Option<usize>,
    launcher_launch_selection: bool,
    launch_args: Vec<String>,
    launch_on_f12: bool,
    launcher_query: Option<String>,
    launcher_overlay: bool,
    no_launcher_overlay: bool,
    apps: bool,
    apps_scene_json: bool,
    apps_kitty: bool,
    apps_limit: Option<usize>,
    apps_filter: Option<String>,
    apps_first: bool,
    apps_launch_first: bool,
    apps_force_fallback: bool,
    remote_host: Option<String>,
    status_scene_json: bool,
    status_kitty: bool,
    chrome_scene_json: bool,
    chrome_kitty: bool,
    keymap: bool,
    keymap_scene_json: bool,
    keymap_kitty: bool,
    config_scene_json: bool,
    config_kitty: bool,
    shortcuts: bool,
    shortcuts_json: bool,
    shortcuts_scene_json: bool,
    shortcuts_kitty: bool,
    help_topic: Option<String>,
    help_scene_topic: Option<String>,
    help_kitty_topic: Option<String>,
    info: bool,
    info_scene_json: bool,
    info_kitty: bool,
    quickstart: bool,
    quickstart_scene_json: bool,
    quickstart_kitty: bool,
    examples: bool,
    examples_scene_json: bool,
    examples_kitty: bool,
    cheat: bool,
    cheat_scene_json: bool,
    cheat_kitty: bool,
    commands: bool,
    commands_json: bool,
    commands_scene_json: bool,
    commands_kitty: bool,
    architecture_json: bool,
    architecture_scene_json: bool,
    architecture_kitty: bool,
    native_surfaces: bool,
    native_surfaces_json: bool,
    native_surfaces_scene_json: bool,
    native_surfaces_kitty: bool,
    panes_scene_json: bool,
    panes_kitty: bool,
    events_scene_json: Option<u64>,
    events_kitty: Option<u64>,
    showcase_scene_json: bool,
    showcase_metrics_json: bool,
    showcase_composition_json: bool,
    tui_smoke_json: bool,
    update: Option<UpdateOptions>,
    mcp: bool,
    completions: Option<String>,
    log_command: Option<LogCommand>,
    keymap_path: Option<String>,
    keymap_check: bool,
    native_terminal: bool,
    native_browser: bool,
    native_url: Option<String>,
    native_out: Option<String>,
    save_session: Option<String>,
    restore_session: Option<String>,
    session_scene_json: bool,
    session_kitty: bool,
    semantic_publish: Option<(String, String)>,
    automation_request: Option<String>,
    remote_help: bool,
    remote_doctor_graphical: bool,
    remote_listing_filter: Option<String>,
    remote_listing_force_fallback: bool,
    remote_terminal_args: Option<Vec<String>>,
    socket: Option<String>,
    display: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogCommand {
    Path,
    Tail { follow: bool },
}

fn parse_args() -> Result<Cli> {
    let mut args = std::env::args().skip(1);
    let mut out = Cli::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "doctor" => out.doctor = true,
            "doctor-scene-json" => out.doctor_scene_json = true,
            "doctor-kitty" | "doctor-graphics" => out.doctor_kitty = true,
            "config" => out.config = true,
            "config-scene-json" => out.config_scene_json = true,
            "config-kitty" | "config-graphics" => out.config_kitty = true,
            "record" => out.record = true,
            "bench" => out.bench = true,
            "launch" => {
                out.launch = true;
                out.launch_args = args.by_ref().collect();
                break;
            }
            "log" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.log_command = Some(parse_log_command(&argv)?);
                break;
            }
            "replace" => {
                out.replace = true;
                out.replace_args = args.by_ref().collect();
                break;
            }
            "launcher" => out.launcher_preview = true,
            "launcher-scene-json" => out.launcher_scene_json = true,
            "launcher-kitty" | "launcher-graphics" => out.launcher_kitty = true,
            "start" => out.mode = lifecycle_alias_mode("start")?,
            "stop" => out.mode = lifecycle_alias_mode("stop")?,
            "keymap" => out.keymap = true,
            "keymap-scene-json" => out.keymap_scene_json = true,
            "keymap-kitty" | "keymap-graphics" => out.keymap_kitty = true,
            "shortcuts" => out.shortcuts = true,
            "shortcuts-json" => out.shortcuts_json = true,
            "shortcuts-scene-json" => out.shortcuts_scene_json = true,
            "shortcuts-kitty" | "shortcuts-graphics" => out.shortcuts_kitty = true,
            "help" => {
                out.help_topic = Some(args.next().unwrap_or_else(|| "topics".to_string()));
                if let Some(extra) = args.next() {
                    return Err(extra_help_topic_error("help", &extra));
                }
                break;
            }
            "help-scene-json" => {
                out.help_scene_topic = Some(args.next().unwrap_or_else(|| "topics".to_string()));
                if let Some(extra) = args.next() {
                    return Err(extra_help_topic_error("help-scene-json", &extra));
                }
                break;
            }
            "help-kitty" | "help-graphics" => {
                out.help_kitty_topic = Some(args.next().unwrap_or_else(|| "topics".to_string()));
                if let Some(extra) = args.next() {
                    return Err(extra_help_topic_error("help-kitty", &extra));
                }
                break;
            }
            "info" => out.info = true,
            "info-scene-json" => out.info_scene_json = true,
            "info-kitty" | "info-graphics" => out.info_kitty = true,
            "panes-scene-json" => out.panes_scene_json = true,
            "panes-kitty" | "panes-graphics" => out.panes_kitty = true,
            "quickstart" => out.quickstart = true,
            "quickstart-scene-json" => out.quickstart_scene_json = true,
            "quickstart-kitty" | "quickstart-graphics" => out.quickstart_kitty = true,
            "examples" => out.examples = true,
            "examples-scene-json" => out.examples_scene_json = true,
            "examples-kitty" | "examples-graphics" => out.examples_kitty = true,
            "cheat" | "cheatsheet" | "cheat-sheet" => out.cheat = true,
            "cheat-scene-json" | "cheatsheet-scene-json" | "cheat-sheet-scene-json" => {
                out.cheat_scene_json = true
            }
            "cheat-kitty"
            | "cheat-graphics"
            | "cheatsheet-kitty"
            | "cheatsheet-graphics"
            | "cheat-sheet-kitty"
            | "cheat-sheet-graphics" => out.cheat_kitty = true,
            "commands" => out.commands = true,
            "commands-json" => out.commands_json = true,
            "commands-scene-json" => out.commands_scene_json = true,
            "commands-kitty" | "commands-graphics" => out.commands_kitty = true,
            "architecture-json" | "platform-contract-json" => out.architecture_json = true,
            "architecture-scene-json" | "platform-contract-scene-json" => {
                out.architecture_scene_json = true
            }
            "architecture-kitty"
            | "architecture-graphics"
            | "platform-contract-kitty"
            | "platform-contract-graphics" => out.architecture_kitty = true,
            "native-surfaces" | "surface-coverage" => out.native_surfaces = true,
            "native-surfaces-json" | "surface-coverage-json" => out.native_surfaces_json = true,
            "native-surfaces-scene-json" | "surface-coverage-scene-json" => {
                out.native_surfaces_scene_json = true
            }
            "native-surfaces-kitty"
            | "native-surfaces-graphics"
            | "surface-coverage-kitty"
            | "surface-coverage-graphics" => out.native_surfaces_kitty = true,
            "showcase-scene-json" | "shell-scene-json" => out.showcase_scene_json = true,
            "showcase-metrics-json" | "shell-metrics-json" => out.showcase_metrics_json = true,
            "showcase-composition-json" | "shell-composition-json" => {
                out.showcase_composition_json = true
            }
            "tui-smoke-json" | "terminal-smoke-json" => out.tui_smoke_json = true,
            "update" => {
                let mut options = parse_update_options(&mut args)?;
                options.json |= out.json;
                out.update = Some(options);
                break;
            }
            "mcp" => {
                out.mcp = true;
                break;
            }
            "completions" => {
                out.completions = Some(args.next().ok_or_else(missing_completion_shell_error)?);
                if let Some(extra) = args.next() {
                    return Err(extra_completion_shell_error(&extra));
                }
                break;
            }
            "status" => {
                out.automation_request =
                    parse_inspection_alias("status", args.next(), args.next())?;
                out.mode = Mode::Status;
                break;
            }
            "status-scene-json" => {
                out.status_scene_json = true;
                break;
            }
            "status-kitty" | "status-graphics" => {
                out.status_kitty = true;
                break;
            }
            "chrome-scene-json" => {
                out.chrome_scene_json = true;
                break;
            }
            "chrome-kitty" | "chrome-graphics" => {
                out.chrome_kitty = true;
                break;
            }
            "session-scene-json" => {
                out.session_scene_json = true;
                break;
            }
            "session-kitty" | "session-graphics" => {
                out.session_kitty = true;
                break;
            }
            "panes" => {
                out.automation_request = parse_inspection_alias("panes", args.next(), args.next())?;
                break;
            }
            "panes-json" => {
                out.automation_request =
                    parse_inspection_alias("panes-json", args.next(), args.next())?;
                break;
            }
            "events" => {
                out.automation_request =
                    parse_inspection_alias("events", args.next(), args.next())?;
                break;
            }
            "events-scene-json" => {
                out.events_scene_json = Some(parse_optional_events_ms(args.next())?);
                if let Some(extra) = args.next() {
                    return Err(anyhow!(
                        "kittwm events-scene-json accepts at most one timeout, got {extra:?}"
                    ));
                }
                break;
            }
            "events-kitty" | "events-graphics" => {
                out.events_kitty = Some(parse_optional_events_ms(args.next())?);
                if let Some(extra) = args.next() {
                    return Err(anyhow!(
                        "kittwm events-kitty accepts at most one timeout, got {extra:?}"
                    ));
                }
                break;
            }
            "spawn" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request = Some(spawn_alias_request(&argv)?);
                break;
            }
            "split" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request = Some(split_alias_request(&argv)?);
                break;
            }
            "read" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request = Some(read_alias_request(false, &argv)?);
                break;
            }
            "read-json" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request = Some(read_alias_request(true, &argv)?);
                break;
            }
            "type" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request =
                    Some(default_window_payload_alias("SEND_TEXT", "type", &argv)?);
                break;
            }
            "line" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request =
                    Some(default_window_payload_alias("SEND_LINE", "line", &argv)?);
                break;
            }
            "paste" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request = Some(default_window_payload_alias(
                    "PASTE_BYTES_B64",
                    "paste",
                    &argv,
                )?);
                break;
            }
            "key" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request =
                    Some(default_window_payload_alias("SEND_KEY", "key", &argv)?);
                break;
            }
            "wait" => {
                let argv = args.by_ref().collect::<Vec<_>>();
                out.automation_request =
                    Some(default_window_payload_alias("WAIT_OUTPUT", "wait", &argv)?);
                break;
            }
            "focus" | "close" | "layout" | "move" | "raise" | "lower" | "nudge"
            | "reset-position" | "reset-offset" | "reset-positions" | "reset-offsets"
            | "resize" | "balance" | "reset-weights" | "reset-weight" | "rename" => {
                out.automation_request = Some(parse_pane_control_alias(a.as_str(), args.by_ref())?);
                break;
            }
            "remote" => {
                let host = args.next().ok_or_else(remote_alias_missing_host_error)?;
                let action = args.next().unwrap_or_else(|| "doctor".to_string());
                let rest = args.by_ref().collect::<Vec<_>>();
                out.remote_host = Some(host);
                parse_remote_alias_action(&mut out, &action, &rest)?;
                break;
            }
            "apps" => out.apps = true,
            "apps-scene-json" => out.apps_scene_json = true,
            "apps-kitty" | "apps-graphics" => out.apps_kitty = true,
            "windows" => out.list_windows = true,
            "displays" => out.list_displays = true,
            "native-terminal" => out.native_terminal = true,
            "native-browser" => out.native_browser = true,
            "--socket" => {
                out.socket = Some(args.next().ok_or_else(|| anyhow!("--socket PATH"))?);
            }
            "--display" => {
                out.display = Some(args.next().ok_or_else(|| anyhow!("--display DISPLAY"))?);
            }
            "--limit" => {
                let v = args.next().ok_or_else(missing_limit_error)?;
                out.apps_limit = Some(parse_limit_value(&v)?);
            }
            "--filter" => {
                out.apps_filter = Some(args.next().ok_or_else(missing_filter_error)?);
            }
            "--remote" | "--host" => {
                out.remote_host = Some(args.next().ok_or_else(|| anyhow!("--remote HOST"))?);
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
            "--probe-kitty" => out.probe_kitty = true,
            "--keymap" => {
                out.keymap_path = Some(args.next().ok_or_else(|| anyhow!("--keymap PATH"))?);
            }
            "--check" => out.keymap_check = true,
            "--shortcuts" => out.shortcuts = true,
            "--shortcuts-json" => out.shortcuts_json = true,
            "--shortcuts-scene-json" => out.shortcuts_scene_json = true,
            "--shortcuts-kitty" | "--shortcuts-graphics" => out.shortcuts_kitty = true,
            "--showcase-scene-json" | "--shell-scene-json" => out.showcase_scene_json = true,
            "--showcase-metrics-json" | "--shell-metrics-json" => out.showcase_metrics_json = true,
            "--showcase-composition-json" | "--shell-composition-json" => {
                out.showcase_composition_json = true
            }
            "--tui-smoke-json" | "--terminal-smoke-json" => out.tui_smoke_json = true,
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
                out.automation_request = Some(text_payload_request(
                    "SEND_TEXT",
                    &window,
                    &text,
                    "--send-text",
                )?);
            }
            "--send-line" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-line WINDOW TEXT"))?;
                let text = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-line WINDOW TEXT"))?;
                out.automation_request = Some(text_payload_request(
                    "SEND_LINE",
                    &window,
                    &text,
                    "--send-line",
                )?);
            }
            "--send-key" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-key WINDOW KEY"))?;
                let key = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-key WINDOW KEY"))?;
                out.automation_request = Some(send_key_request(&window, &key)?);
            }
            "--send-mouse" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-mouse WINDOW EVENT COL ROW"))?;
                let event = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-mouse WINDOW EVENT COL ROW"))?;
                let col = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-mouse WINDOW EVENT COL ROW"))?;
                let row = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-mouse WINDOW EVENT COL ROW"))?;
                out.automation_request = Some(send_mouse_request(&window, &event, &col, &row)?);
            }
            "--send-bytes-b64" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-bytes-b64 WINDOW BASE64"))?;
                let encoded = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-bytes-b64 WINDOW BASE64"))?;
                out.automation_request = Some(send_bytes_b64_request(&window, &encoded)?);
            }
            "--paste-bytes-b64" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--paste-bytes-b64 WINDOW BASE64"))?;
                let encoded = args
                    .next()
                    .ok_or_else(|| anyhow!("--paste-bytes-b64 WINDOW BASE64"))?;
                out.automation_request = Some(paste_bytes_b64_request(&window, &encoded)?);
            }
            "--send-file" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-file WINDOW PATH|-"))?;
                let path = args
                    .next()
                    .ok_or_else(|| anyhow!("--send-file WINDOW PATH|-"))?;
                out.automation_request = Some(send_file_request(&window, &path)?);
            }
            "--paste-file" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--paste-file WINDOW PATH|-"))?;
                let path = args
                    .next()
                    .ok_or_else(|| anyhow!("--paste-file WINDOW PATH|-"))?;
                out.automation_request = Some(paste_file_request(&window, &path)?);
            }
            "--read-text" => {
                let window = args.next().ok_or_else(|| anyhow!("--read-text WINDOW"))?;
                out.automation_request = Some(automation_request("READ_TEXT", &window, "")?);
            }
            "--read-text-json" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--read-text-json WINDOW"))?;
                out.automation_request = Some(automation_request("READ_TEXT_JSON", &window, "")?);
            }
            "--read-scrollback" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--read-scrollback WINDOW"))?;
                out.automation_request = Some(automation_request("READ_SCROLLBACK", &window, "")?);
            }
            "--read-scrollback-json" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--read-scrollback-json WINDOW"))?;
                out.automation_request =
                    Some(automation_request("READ_SCROLLBACK_JSON", &window, "")?);
            }
            "--semantic-snapshot" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--semantic-snapshot WINDOW|focused"))?;
                out.automation_request = Some(semantic_snapshot_request(&window)?);
            }
            "--semantic-publish" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--semantic-publish WINDOW|focused JSON_OR_PATH|-"))?;
                let json = args
                    .next()
                    .ok_or_else(|| anyhow!("--semantic-publish WINDOW|focused JSON_OR_PATH|-"))?;
                out.semantic_publish = Some((window, json));
            }
            "--semantic-action" => {
                let window = args.next().ok_or_else(|| {
                    anyhow!("--semantic-action WINDOW|focused COMPONENT ACTION JSON")
                })?;
                let component = args.next().ok_or_else(|| {
                    anyhow!("--semantic-action WINDOW|focused COMPONENT ACTION JSON")
                })?;
                let action = args.next().ok_or_else(|| {
                    anyhow!("--semantic-action WINDOW|focused COMPONENT ACTION JSON")
                })?;
                let payload = args.next().ok_or_else(|| {
                    anyhow!("--semantic-action WINDOW|focused COMPONENT ACTION JSON")
                })?;
                out.automation_request = Some(semantic_action_request(
                    &window, &component, &action, &payload,
                )?);
            }
            "--semantic-focus" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--semantic-focus WINDOW|focused COMPONENT"))?;
                let component = args
                    .next()
                    .ok_or_else(|| anyhow!("--semantic-focus WINDOW|focused COMPONENT"))?;
                out.automation_request = Some(semantic_focus_request(&window, &component)?);
            }
            "--wait-text" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text WINDOW NEEDLE"))?;
                out.automation_request = Some(wait_request("WAIT_TEXT", &window, &needle)?);
            }
            "--wait-text-json" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-json WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-json WINDOW NEEDLE"))?;
                out.automation_request = Some(wait_request("WAIT_TEXT_JSON", &window, &needle)?);
            }
            "--wait-output" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output WINDOW NEEDLE"))?;
                out.automation_request = Some(wait_request("WAIT_OUTPUT", &window, &needle)?);
            }
            "--wait-output-json" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-json WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-json WINDOW NEEDLE"))?;
                out.automation_request = Some(wait_request("WAIT_OUTPUT_JSON", &window, &needle)?);
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
                out.automation_request =
                    Some(wait_ms_request("WAIT_TEXT_MS", &ms, &window, &needle)?);
            }
            "--wait-text-json-ms" => {
                let ms = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-json-ms MS WINDOW NEEDLE"))?;
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-json-ms MS WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-text-json-ms MS WINDOW NEEDLE"))?;
                out.automation_request =
                    Some(wait_ms_request("WAIT_TEXT_JSON_MS", &ms, &window, &needle)?);
            }
            "--wait-output-ms" => {
                let ms = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-ms MS WINDOW NEEDLE"))?;
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-ms MS WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-ms MS WINDOW NEEDLE"))?;
                out.automation_request =
                    Some(wait_ms_request("WAIT_OUTPUT_MS", &ms, &window, &needle)?);
            }
            "--wait-output-json-ms" => {
                let ms = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-json-ms MS WINDOW NEEDLE"))?;
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-json-ms MS WINDOW NEEDLE"))?;
                let needle = args
                    .next()
                    .ok_or_else(|| anyhow!("--wait-output-json-ms MS WINDOW NEEDLE"))?;
                out.automation_request = Some(wait_ms_request(
                    "WAIT_OUTPUT_JSON_MS",
                    &ms,
                    &window,
                    &needle,
                )?);
            }
            "--status-json" => out.automation_request = Some("STATUS_JSON".to_string()),
            "--status-scene-json" => out.status_scene_json = true,
            "--status-kitty" | "--status-graphics" => out.status_kitty = true,
            "--help-json" => out.automation_request = Some("HELP_JSON".to_string()),
            "--chrome-json" => out.automation_request = Some("CHROME_JSON".to_string()),
            "--chrome-scene-json" => out.chrome_scene_json = true,
            "--chrome-kitty" | "--chrome-graphics" => out.chrome_kitty = true,
            "--clipboard-json" => out.automation_request = Some("CLIPBOARD_JSON".to_string()),
            "--panes" => out.automation_request = Some("PANES".to_string()),
            "--panes-json" => out.automation_request = Some("PANES_JSON".to_string()),
            "--panes-scene-json" => out.panes_scene_json = true,
            "--panes-kitty" | "--panes-graphics" => out.panes_kitty = true,
            "--session-json" => out.automation_request = Some("SESSION_JSON".to_string()),
            "--session-scene-json" => out.session_scene_json = true,
            "--session-kitty" | "--session-graphics" => out.session_kitty = true,
            "--events" => out.automation_request = Some("EVENTS".to_string()),
            "--events-ms" => {
                let ms = args.next().ok_or_else(|| anyhow!("--events-ms MS"))?;
                out.automation_request = Some(events_request(&ms)?);
            }
            "--events-scene-json" => {
                let ms = args
                    .next()
                    .ok_or_else(|| anyhow!("--events-scene-json MS"))?;
                out.events_scene_json = Some(parse_optional_events_ms(Some(ms))?);
            }
            "--events-kitty" | "--events-graphics" => {
                let ms = args.next().ok_or_else(|| anyhow!("--events-kitty MS"))?;
                out.events_kitty = Some(parse_optional_events_ms(Some(ms))?);
            }
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
                    .ok_or_else(|| anyhow!("--layout columns|rows|grid"))?;
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
            "--nudge-pane" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--nudge-pane WINDOW|focused DX DY"))?;
                let dx = args
                    .next()
                    .ok_or_else(|| anyhow!("--nudge-pane WINDOW|focused DX DY"))?;
                let dy = args
                    .next()
                    .ok_or_else(|| anyhow!("--nudge-pane WINDOW|focused DX DY"))?;
                out.automation_request = Some(nudge_pane_request(&window, &dx, &dy)?);
            }
            "--reset-pane-offset" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--reset-pane-offset WINDOW|focused"))?;
                out.automation_request = Some(reset_pane_offset_request(&window)?);
            }
            "--reset-all-pane-offsets" => {
                out.automation_request = Some("RESET_ALL_PANE_OFFSETS".to_string());
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
            "--balance-panes" | "--reset-pane-weights" => {
                out.automation_request = Some("BALANCE_PANES".to_string())
            }
            "--rename-pane" => {
                let window = args
                    .next()
                    .ok_or_else(|| anyhow!("--rename-pane WINDOW TITLE"))?;
                let title = args
                    .next()
                    .ok_or_else(|| anyhow!("--rename-pane WINDOW TITLE"))?;
                out.automation_request = Some(rename_pane_request(&window, &title)?);
            }
            "--apps-json" => out.automation_request = Some("APPS_JSON".to_string()),
            "--apps-first" => {
                let query = args.next().ok_or_else(|| anyhow!("--apps-first QUERY"))?;
                out.automation_request = Some(protocol_payload_request("APPS_FIRST", &query)?);
            }
            "--apps-launch-first" => {
                let query = args
                    .next()
                    .ok_or_else(|| anyhow!("--apps-launch-first QUERY"))?;
                out.automation_request =
                    Some(protocol_payload_request("APPS_LAUNCH_FIRST", &query)?);
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
            other => return Err(friendly_unknown_command_error(other)),
        }
    }
    validate_socket_target_flags(&out)?;
    Ok(out)
}

fn parse_update_options(args: &mut impl Iterator<Item = String>) -> Result<UpdateOptions> {
    let mut options = UpdateOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--status" | "status" => options.action = UpdateAction::Status,
            "--check" | "check" => options.action = UpdateAction::Check,
            "--run" | "run" => options.action = UpdateAction::Run,
            "--json" => options.json = true,
            "--repository" => {
                options.repository = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--repository OWNER/REPO"))?,
                );
            }
            "--install-dir" => {
                options.install_dir = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--install-dir PATH"))?
                        .into(),
                );
            }
            other => return Err(anyhow!("unknown update option {other:?}")),
        }
    }
    Ok(options)
}

fn validate_socket_target_flags(cli: &Cli) -> Result<()> {
    if cli.socket.is_some() && cli.display.is_some() {
        return Err(anyhow!("--socket and --display are mutually exclusive"));
    }
    Ok(())
}

fn apply_socket_target_flags(cli: &Cli) {
    if let Some(socket) = &cli.socket {
        std::env::set_var("KITTWM_SOCKET", socket);
        std::env::set_var("KITTWM_SOCK", socket);
    }
    if let Some(display) = &cli.display {
        std::env::set_var("KITTUI_WM_DISPLAY", display);
        std::env::set_var("KITTWM_DISPLAY", display);
    }
}

fn kittwm_help_text() -> String {
    let mut out = String::with_capacity(8192);
    out.push_str("kittwm — terminal-native window manager\n\n");
    out.push_str(&kittwm_help_command_tree_text());
    out.push_str(
        r#"
DAILY DRIVER BASICS
  Quickstart:      kittwm quickstart
  Examples:        kittwm examples
  Cheat sheet:     kittwm cheat
  Start:           kittwm        (or: kittwm start)
  New terminal:    press C-a Enter inside kittwm
  Float/full:      press C-a t / C-a f inside kittwm
  Toggle split:    press C-a e inside kittwm
  Launcher:        press C-a g inside kittwm
  Help overlay:    press C-a ? inside kittwm
  Exit:            press Ctrl-]
  Shortcut list:   kittwm shortcuts        (JSON: kittwm --shortcuts-json)
  Scene artifact:  kittwm showcase-scene-json
  Perf metrics:    kittwm showcase-metrics-json
  Composition:     kittwm showcase-composition-json
  Architecture:    kittwm architecture-json
  Native surfaces: kittwm native-surfaces     (JSON: kittwm native-surfaces-json)
  TUI smoke:       kittwm tui-smoke-json
  Old startup:     KITTWM_STARTUP_TERMINAL=1 kittwm

COMMON INSPECTION
  info                Friendly one-screen status/panes/chrome overview
  --status-json       Current WM status JSON
  --panes             Human-readable pane list
  --panes-json        Pane list + geometry/cursor/mode metadata JSON
  --chrome-json       Top-bar/workspace reservation JSON
  --help-json         Machine-readable socket command catalog
  --events-ms MS      Bounded JSON-lines event stream

PANE CONTROL
  spawn CMD [ARGS...]         Spawn a terminal pane
  split [WINDOW] columns|rows|grid CMD [ARGS...]
                              Spawn next to focused/window and set split axis
  focus WINDOW                Alias for --focus-pane WINDOW
  close [WINDOW]              Alias for --close-pane (default focused)
  layout columns|rows|grid         Alias for --layout
  move [WINDOW] DIR           Alias for --move-pane (default focused)
  raise [WINDOW]              Raise pane to top of floating/stack order
  lower [WINDOW]              Lower pane to bottom of floating/stack order
  nudge [WINDOW] DX DY        Nudge floating pane by cell delta
  reset-position [WINDOW]     Reset floating pane to generated position
  reset-positions             Reset all floating panes to generated positions
  resize [WINDOW] AMOUNT      Alias for --resize-pane (default focused)
  balance                     Alias for --balance-panes
  reset-weights               Alias for --balance-panes
  rename WINDOW TITLE         Alias for --rename-pane
  --spawn-pty CMD             Spawn a terminal pane
  --focus-pane WINDOW         Focus pane by id, or use focused
  --focus-next | --focus-prev Cycle focus
  --close-pane WINDOW         Close pane; last pane returns to empty workspace
  --layout columns|rows|grid       Change tiling axis/grid
  --move-pane WINDOW DIR      DIR: left/right/up/down/first/last
  --nudge-pane WINDOW DX DY   Nudge floating pane by cell delta
  --reset-pane-offset WINDOW  Reset floating pane offset
  --reset-all-pane-offsets    Reset every floating pane offset
  --resize-pane WINDOW N      N: grow/shrink/+N/-N
  --balance-panes             Equalize pane weights
  --reset-pane-weights        Alias for --balance-panes
  --rename-pane WINDOW TITLE  Set pane display title

INPUT AND AUTOMATION
  type [WINDOW] TEXT               Send text bytes (default window: focused)
  line [WINDOW] TEXT               Send text plus newline
  paste [WINDOW] TEXT              Paste text via bracketed paste
  key [WINDOW] KEY                 Send a named key
  read [WINDOW]                    Read text (default window: focused)
  read-json [WINDOW]               Read text JSON
  wait [WINDOW] TEXT               Wait for text or scrollback
  --send-text WINDOW TEXT          Send text bytes
  --send-line WINDOW TEXT          Send text plus newline
  --send-key WINDOW KEY            KEY: ctrl-c, shift/alt/ctrl arrows, insert/delete, home/end/page, shift-tab, f5..f12, arrows
  --send-mouse WINDOW EVENT C R    Send terminal mouse event
  --send-bytes-b64 WINDOW BASE64   Send arbitrary bytes
  --paste-bytes-b64 WINDOW BASE64  Paste arbitrary bytes
  --send-file WINDOW PATH|-        Read bytes from file/stdin and send
  --paste-file WINDOW PATH|-       Paste bytes; respects bracketed paste
  --read-text WINDOW               Text snapshot
  --read-text-json WINDOW          Text snapshot JSON
  --read-scrollback WINDOW         Scrollback text
  --read-scrollback-json WINDOW    Scrollback JSON
  --wait-text WINDOW TEXT          Wait for visible text
  --wait-output WINDOW TEXT        Wait for visible text or scrollback
  --wait-text-json WINDOW TEXT     JSON wait match
  --wait-output-json WINDOW TEXT   JSON wait match over text+scrollback
  Add -ms after wait verb for explicit timeout, e.g. --wait-output-json-ms 5000 focused Ready

APPS AND LAUNCHING
  apps [--filter QUERY] [--limit N] [--first] [--launch-first]
  remote HOST [help|doctor|status|check|x11|gui|graphical|wayland|forward|fallback|kittwm|desktop|wm|list|apps|applications|programs|software|app|application|program|select|pick|launch|open|run|start|windows|win|displays|monitors|screens|terminal|term|cmd|command|exec|shell|sh|login|ssh|console|tty]
                            Friendly pooled-SSH aliases for remote workflows
  apps --remote HOST [--filter QUERY] [--limit N] [--first|--launch-first]
                            List/launch remote candidates via pooled SSH;
                            uses remote kittwm when installed, else PATH fallback
  windows --remote HOST [QUERY]
                            List/filter remote windows via pooled SSH; delegates
                            to remote kittwm when available, else best-effort fallback
  displays --remote HOST [QUERY]
                            List/filter remote displays via pooled SSH; delegates
                            to remote kittwm when available, else best-effort fallback
  apps-kitty [--filter QUERY] [--limit N]
                            Render launcher candidates with kitty graphics
  --apps-json              App discovery catalog JSON
  --apps-first QUERY       Print first matching app candidate
  --apps-launch-first Q    Launch first matching app candidate
  launcher                 Render launcher preview; use --select N / --launch-selection
  launcher-kitty           Render launcher preview with kitty graphics
  launch -- CMD ARGS       Spawn command through backend launcher
  replace CMD ARGS         Exec command in current KITTWM_WINDOW

SESSIONS AND SEMANTICS
  --session-json                 Print persistence manifest
  --save-session PATH|-          Save SESSION_JSON
  --restore-session PATH|-       Restore panes from manifest
  --semantic-snapshot WINDOW     Semantic component snapshot JSON
  --semantic-publish WINDOW JSON_OR_PATH|-
  --semantic-action WINDOW COMPONENT ACTION JSON
  --semantic-focus WINDOW COMPONENT

DIAGNOSTICS AND BACKENDS
  doctor [--json] [--probe-kitty]   Diagnostics; kitty probing is opt-in
  doctor --remote HOST              Check remote kittwm availability and SSH forwarding hints
  config [--keymap PATH] [--check]  Config/keymap inspection
  config-scene-json [--keymap PATH] Emit config readiness as a kittui Scene
  config-kitty [--keymap PATH]      Render config readiness with kitty graphics
  keymap [--keymap PATH] [--check]  Print resolved keymap
  keymap-scene-json [--keymap PATH] Emit resolved keymap as a kittui Scene
  keymap-kitty [--keymap PATH]      Render resolved keymap with kitty graphics
  record / bench                    Capture-pipeline tooling
  native-terminal / native-browser  Backend-independent proofs
  --backend fake|quartz|xvfb        Force capture backend
  --pick-window / --list-windows / --list-displays / --capture SPEC

EXAMPLES
  kittwm
  kittwm quickstart
  kittwm commands
  kittwm info
  kittwm --panes
  kittwm spawn htop
  kittwm read-json focused
  kittwm --wait-output-json-ms 10000 focused 'build finished'
  kittwm --save-session session.json
  kittwm --restore-session session.json

FIRST-PARTY HELPERS
  kittwm-launch --browser https://example.com
  kittwm-terminal --events-ms 1000
  kittwm-top --json
  kittwm-bar --reserve --kitty
  kittwm-browser --semantic-snapshot https://example.com

For complete socket verbs: kittwm --help-json
For interactive key chords: kittwm shortcuts
"#,
    );
    out
}

fn kittwm_help_command_tree_text() -> String {
    let entries = local_command_entries();
    let mut out = String::with_capacity(entries.len().saturating_mul(64).saturating_add(256));
    out.push_str("USAGE\n");
    out.push_str("  kittwm                         Start the WM in this terminal (empty workspace + top bar)\n");
    out.push_str("  kittwm --socket PATH COMMAND   Target a running WM socket for one command\n");
    out.push_str("  kittwm --display :N COMMAND    Target a DISPLAY-like kittwm socket token\n");
    out.push_str("  kittwm --help                  Show this overview\n");
    out.push_str("\nCOMMAND TREE (derived from kittwm parser catalog)\n");
    let mut categories = Vec::new();
    for entry in entries {
        if !categories.contains(&entry.category) {
            categories.push(entry.category);
        }
    }
    for category in categories {
        out.push('\n');
        for ch in category.chars() {
            out.push(ch.to_ascii_uppercase());
        }
        out.push('\n');
        for entry in entries.iter().filter(|entry| entry.category == category) {
            let _ = writeln!(out, "  kittwm {:32} {}", entry.command, entry.description);
        }
    }
    out
}

fn print_help() {
    print!("{}", kittwm_help_text());
}

fn help_topic_cmd(topic: &str) -> Result<()> {
    print!("{}", help_topic_text(topic)?);
    Ok(())
}

fn help_topic_graphical_cmd(topic: &str, kitty: bool) -> Result<()> {
    let text = help_topic_text(topic)?;
    let scene = help_topic_scene(topic, text);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn help_topic_scene(topic: &str, text: &str) -> Scene {
    help_topic_scene_for_cols(topic, text, info_scene_cols())
}

fn help_topic_scene_for_cols(topic: &str, text: &str, cols: u16) -> Scene {
    let content_lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let rows = help_topic_scene_rows(content_lines.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let heading = content_lines.first().copied().unwrap_or(topic).trim();
    let topic_label = truncate(topic, 32);
    let heading_label = truncate(heading, 64);
    let command_count = content_lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("kittwm")
                || trimmed.starts_with("--")
                || trimmed.contains(" WINDOW")
        })
        .count();
    let mut layers = vec![
        Layer {
            label: Some(help_topic_backdrop_label(
                &topic_label,
                content_lines.len(),
                command_count,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(help_topic_heading_label(&topic_label, &heading_label)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, line) in content_lines.iter().skip(1).take(20).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let trimmed = line.trim();
        let row_label = truncate(trimmed, 80);
        layers.push(Layer {
            label: Some(help_topic_row_label(&topic_label, idx, &row_label)),
            root: Node::Rect {
                rect: info_indicator_rect(width, y),
                fill: Paint::Solid {
                    color: if trimmed.starts_with("--") || trimmed.starts_with("kittwm") {
                        Rgba::rgba(163, 190, 140, 255)
                    } else {
                        Rgba::rgba(136, 192, 208, 255)
                    },
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn help_topic_row_label(topic_label: &str, idx: usize, row_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-help-topic-row::".len() + topic_label.len() + 20 + row_label.len(),
    );
    label.push_str("kittwm-help-topic-row:");
    label.push_str(topic_label);
    label.push(':');
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(row_label);
    label
}

fn help_topic_heading_label(topic_label: &str, heading_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-help-topic-heading::".len() + topic_label.len() + heading_label.len(),
    );
    label.push_str("kittwm-help-topic-heading:");
    label.push_str(topic_label);
    label.push(':');
    label.push_str(heading_label);
    label
}

fn help_topic_backdrop_label(topic_label: &str, line_count: usize, command_count: usize) -> String {
    let mut label = String::with_capacity(
        "kittwm-help-topic-backdrop::lines=:commands=".len() + topic_label.len() + 20 + 20,
    );
    label.push_str("kittwm-help-topic-backdrop:");
    label.push_str(topic_label);
    label.push_str(":lines=");
    let _ = write!(label, "{line_count}");
    label.push_str(":commands=");
    let _ = write!(label, "{command_count}");
    label
}

fn help_topic_scene_rows(line_count: usize) -> u16 {
    let rows = line_count.saturating_add(4).min(u16::MAX as usize) as u16;
    rows.clamp(8, 30)
}

fn help_topic_text(topic: &str) -> Result<&'static str> {
    match topic {
        "topics" | "topic" | "list" => Ok("kittwm help topics\n\
             ==================\n\n\
             start    launch modes, startup environment, and first terminal\n\
             panes    pane lifecycle, focus, layout, movement, and sessions\n\
             input    text, key, mouse, bytes, paste, and semantic actions\n\
             inspect  status, panes, chrome, shortcuts, help, and text reads\n\
             session  save/restore session manifests\n\
             events   bounded event streams and typed SDK event helpers\n\
             apps     app discovery and launch helpers\n\
             ssh      pooled SSH workflows for remote apps, displays, and terminals\n\
             log      debug log path and tailing workflows\n\
             completions shell completion setup\n\n\
             Daily guides:\n\
             kittwm quickstart    first-run daily-driver checklist\n\
             kittwm examples      copy-paste workflows\n\
             kittwm cheat         compact keys and commands\n\n\
             Usage: kittwm help <topic>\n"),
        "start" | "startup" => Ok("kittwm help start\n\
             =================\n\n\
             kittwm                         start clean native workspace\n\
             KITTWM_STARTUP_TERMINAL=1      start old immediate terminal\n\
             KITTWM_WORKSPACE=<label>       override displayed/reported label\n\
             KITTWM_NATIVE_RENDERER=terminal use ANSI renderer\n\
             KITTWM_NATIVE_RENDERER=kitty    use kitty graphics renderer\n\
             KITTWM_NATIVE_CHROME_RENDERER=affordance-scene\n\
                                            opt into kittui scene chrome\n\
             Ctrl-A Enter                   launch terminal from empty workspace\n\
             Ctrl-A t                       toggle floating mode\n\
             Ctrl-A f                       toggle fullscreen\n\
             Ctrl-A e                       toggle current split vertical/horizontal\n\
             Ctrl-]                         exit kittwm\n\
             KITTWM_WORKSPACE=dev kittwm    start with a named workspace\n\
             KITTWM_NATIVE_RENDERER=kitty kittwm\n\
                                            start with kitty graphics renderer\n\
             KITTWM_NATIVE_CHROME_RENDERER=affordance-scene kittwm\n\
                                            start with kittui scene chrome\n"),
        "panes" | "pane" => Ok("kittwm help panes
\
             =================

\
             --spawn-pty CMD                spawn a native PTY pane
\
             split [WINDOW] columns|rows|grid CMD [ARGS...]
\
                                            spawn next to target and set split axis
\
             focus WINDOW                   focus window or focused token
\
             close [WINDOW]                 close pane (default focused)
\
             layout columns|rows|grid            switch layout axis
\
             move [WINDOW] DIR              move pane (default focused)
\
             nudge [WINDOW] DX DY           nudge floating pane by cell delta
\
             reset-position [WINDOW]        reset floating pane offset
\
             reset-positions                reset all floating pane offsets
\
             resize [WINDOW] AMOUNT         resize pane weight (default focused)
\
             balance                        equalize weights
\
             reset-weights                  reset pane weights
\
             rename WINDOW TITLE            set display title
\
             --focus-pane WINDOW            focus window or focused token
\
             --focus-next / --focus-prev    cycle focus
\
             --close-pane WINDOW            close pane; last pane returns empty
\
             --layout columns|rows|grid          switch layout axis
\
             --move-pane WINDOW DIR         left/right/up/down/first/last
\
             --nudge-pane WINDOW DX DY      nudge floating pane by cell delta
\
             --reset-pane-offset WINDOW     reset floating pane offset
\
             --reset-all-pane-offsets       reset all floating pane offsets
\
             --resize-pane WINDOW AMOUNT    grow/shrink/+N/-N pane weight
\
             --balance-panes                equalize weights
\
             --reset-pane-weights           reset pane weights
\
             --rename-pane WINDOW TITLE     set display title
\
             kittwm spawn htop              create a new PTY pane
\
             kittwm split focused columns htop
\
                                            split beside focused pane
\
             kittwm focus next              cycle focus forward
\
             kittwm balance                 equalize pane weights

\
             Socket equivalents include SPAWN_PTY, SPLIT_PANE, FOCUS_PANE,
\
             CLOSE_PANE, LAYOUT, MOVE_PANE, RESIZE_PANE, BALANCE_PANES,
\
             and RENAME_PANE.
"),
        "logs" | "log" => Ok("kittwm help log\n\
             ================\n\n\
             kittwm writes debug logs to KITTUI_WM_LOG when set, otherwise /tmp/kittui-wm.log.\n\
             Open kittwm in one terminal, then watch it from another:\n\n\
             kittwm log path              print the active log path\n\
             kittwm log tail              print recent log lines\n\
             kittwm log tail -f           follow the log, like tail -f\n\
             KITTUI_WM_LOG=/tmp/demo.log kittwm\n\
                                           start with a per-session log file\n"),
        "ssh" | "remote" | "remotes" => Ok("kittwm help ssh\n\
             ===============\n\n\
             If the remote has kittwm installed, run kittwm there and it uses\n\
             that host's desktop, displays, terminal size, and graphics context.\n\
             When the remote does not have kittwm, use local kittwm helpers that\n\
             auto-detect remote capabilities and forward over pooled SSH.\n\n\
             kittwm apps --remote HOST\n\
                                           list remote app candidates; delegates to remote kittwm if present\n\
             kittwm apps --remote HOST --filter firefox --launch-first\n\
                                           launch first remote app match through pooled SSH\n\
             kittwm --list-windows --remote HOST\n\
                                           list remote windows/displays when supported\n\
             kittwm remote HOST help       host-specific SSH quick reference\n\
             kittwm remote HOST status     check remote kittwm availability\n\
             kittwm remote HOST status --x11\n\
                                           check trusted X11 forwarding for remote app launch\n\
             kittwm remote HOST status --wayland\n\
                                           alias for the graphical forwarding check\n\
             kittwm remote HOST x11        short alias for the graphical forwarding check\n\
             kittwm remote HOST gui        alias for remote HOST x11\n\
             kittwm remote HOST graphical  alias for remote HOST x11\n\
             kittwm remote HOST wayland    alias for remote HOST graphical\n\
             kittwm remote HOST forwarding alias for remote HOST x11\n\
             kittwm remote HOST forward    short alias for remote HOST forwarding\n\
             kittwm remote HOST            friendly alias for remote doctor\n\
             kittwm remote HOST kittwm     open remote kittwm in a pooled SSH pane\n\
             kittwm remote HOST desktop    alias for remote HOST kittwm\n\
             kittwm remote HOST wm         short alias for remote HOST kittwm\n\
             kittwm remote HOST list       list remote app candidates\n\
             kittwm remote HOST list apps firefox\n\
                                           list remote app matches using a natural alias\n\
             kittwm remote HOST list app firefox\n\
                                           singular alias for remote HOST list apps\n\
             kittwm remote HOST list windows firefox\n\
                                           list remote windows matching a query\n\
             kittwm remote HOST list windows firefox --json\n\
                                           structured remote window matches\n\
             kittwm remote HOST list windows firefox --fallback\n\
                                           skip remote kittwm and force platform fallback listing\n\
             kittwm remote HOST win firefox\n\
                                           short alias for remote window listing\n\
             kittwm remote HOST monitors retina\n\
                                           alias for remote display listing\n\
             kittwm remote HOST monitors retina --fallback\n\
                                           skip remote kittwm and force platform fallback listing\n\
             kittwm remote HOST screens retina\n\
                                           alias for remote display listing\n\
             kittwm remote HOST apps firefox\n\
                                           list remote app matches using a positional query\n\
             kittwm remote HOST apps firefox --json\n\
                                           structured remote app matches with counts\n\
             kittwm remote HOST apps firefox --fallback\n\
                                           skip remote kittwm and force pooled-SSH fallback discovery\n\
             kittwm remote HOST fallback apps firefox\n\
                                           front-door alias for forcing pooled-SSH fallback discovery\n\
             kittwm remote HOST fallback launch firefox\n\
                                           front-door alias for forcing pooled-SSH fallback launch\n\
             kittwm remote HOST fallback open firefox\n\
                                           natural alias for forced fallback launch\n\
             kittwm remote HOST fallback run firefox\n\
                                           natural alias for forced fallback launch\n\
             kittwm remote HOST fallback start firefox\n\
                                           natural alias for forced fallback launch\n\
             kittwm remote HOST fallback windows firefox\n\
                                           front-door alias for forcing platform fallback window listing\n\
             kittwm remote HOST fallback displays retina\n\
                                           front-door alias for forcing platform fallback display listing\n\
             kittwm remote HOST applications firefox\n\
                                           natural alias for remote HOST apps\n\
             kittwm remote HOST programs firefox\n\
                                           program-style alias for remote HOST apps\n\
             kittwm remote HOST software firefox\n\
                                           software-style alias for remote HOST apps\n\
             kittwm remote HOST app firefox\n\
                                           select the first remote app match\n\
             kittwm remote HOST app firefox --json\n\
                                           structured first remote app match\n\
             kittwm remote HOST application firefox --json\n\
                                           structured alias for selecting the first remote app match\n\
             kittwm remote HOST program firefox --json\n\
                                           structured program-style alias for the first remote app match\n\
             kittwm remote HOST select firefox
\
                                           natural alias for selecting the first remote app match
\
             kittwm remote HOST pick firefox --json
\
                                           structured alias for selecting the first remote app match
\
             kittwm remote HOST launch firefox\n\
                                           shortest alias for launching the first remote app match\n\
             kittwm remote HOST launch firefox --fallback\n\
                                           skip remote kittwm and force pooled-SSH fallback launch\n\
             kittwm remote HOST open firefox\n\
                                           natural alias for remote HOST launch\n\
             kittwm remote HOST run firefox\n\
                                           natural alias for remote HOST launch\n\
             kittwm remote HOST start firefox
\
                                           natural alias for remote HOST launch
\
             kittwm remote HOST apps firefox --launch-first\n\
                                           explicit remote app launch path\n\
             kittwm remote HOST launch firefox --json\n\
                                           structured remote app launch result or error\n\
             kittwm remote HOST shell      open a pooled SSH login shell pane\n\
             kittwm remote HOST sh         short alias for the same pooled SSH shell pane\n\
             kittwm remote HOST login      natural alias for the same pooled SSH shell pane\n\
             kittwm remote HOST ssh        alias for the same pooled SSH shell pane\n\
             kittwm remote HOST console    natural alias for the same pooled SSH pane\n\
             kittwm remote HOST tty        short alias for the same pooled SSH pane\n\
             kittwm remote HOST terminal htop\n\
                                           shortest alias for kittwm-terminal --remote HOST htop\n\
             kittwm remote HOST term htop  short alias for remote HOST terminal htop\n\
             kittwm remote HOST cmd htop   command-style alias for remote HOST terminal htop\n\
             kittwm remote HOST command htop\n\
                                           command-style alias for remote HOST terminal htop\n\
             kittwm remote HOST exec htop  exec-style alias for remote HOST terminal htop\n\
             kittwm doctor --remote HOST\n\
                                           check remote kittwm availability and suggested path\n\
             kittwm-terminal --remote HOST --title HOST\n\
                                           open a local kittwm pane running a remote login shell\n\
             kittwm-terminal --remote HOST -- htop\n\
                                           open a local pane running a remote command\n\n\
             SSH pooling uses ControlMaster=auto and ControlPersist=10m, so\n\
             repeated remote app/list/terminal commands reuse the same connection.\n"),
        "completions" | "completion" => Ok("kittwm help completions\n\
             ========================\n\n\
             kittwm completions bash      print Bash completion script\n\
             kittwm completions zsh       print Zsh completion script\n\
             kittwm completions fish      print Fish completion script\n\
             kittwm completions bash >> ~/.bashrc\n\
                                          install Bash completions for future shells\n\
             kittwm completions zsh >> ~/.zshrc\n\
                                          install Zsh completions for future shells\n\
             mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish\n\
                                          install Fish completions\n"),
        "input" => Ok("kittwm help input\n\
             =================\n\n\
             type [WINDOW] TEXT             short alias for --send-text\n\
             line [WINDOW] TEXT             short alias for --send-line\n\
             paste [WINDOW] TEXT            paste text with bracketed-paste support\n\
             key [WINDOW] KEY               short alias for --send-key\n\
             --send-text WINDOW TEXT        send text bytes\n\
             --send-line WINDOW TEXT        send text plus newline\n\
             --send-key WINDOW KEY          send named key (ctrl-c, shift/alt/ctrl arrows, insert/delete, home/end/page, shift-tab, f5..f12, arrows)\n\
             --send-mouse WINDOW EVENT C R  send terminal mouse event if app enabled it\n\
             --send-bytes-b64 WINDOW B64    send exact bytes\n\
             --paste-bytes-b64 WINDOW B64   paste exact bytes\n\
             --send-file WINDOW PATH|-      send bytes from file/stdin\n\
             --paste-file WINDOW PATH|-     paste bytes with bracketed-paste support\n\
             --semantic-action WINDOW COMPONENT ACTION JSON\n\
                                            invoke semantic action\n\
             --semantic-focus WINDOW COMPONENT request semantic focus\n\
             kittwm line focused 'echo ready'\n\
                                            send a command line to focused pane\n\
             kittwm paste focused 'multi-line text'\n\
                                            bracketed-paste text into focused pane\n\
             kittwm key focused ctrl-c      send an interrupt key\n\
             kittwm --semantic-action focused button-1 press '{}'\n\
                                            invoke a semantic component action\n"),
        "inspect" | "inspection" => Ok("kittwm help inspect\n\
             ===================\n\n\
             status                         daemon STATUS (alias for --status)\n\
             panes / panes-json             pane listing / structured panes\n\
             events [ms]                    bounded event stream\n\
             --status-json                  STATUS_JSON snapshot\n\
             --chrome-json                  CHROME_JSON workspace/chrome metadata\n\
             --shortcuts / --shortcuts-json shortcut catalog\n\
             --help-json                    HELP_JSON command catalog\n\
             --read-text[-json] WINDOW      text snapshot\n\
             --read-scrollback[-json] WINDOW scrollback snapshot\n\
             --semantic-snapshot WINDOW     semantic component snapshot\n\
             --apps-json                    app discovery catalog\n\
             kittwm-top --json              first-party process snapshot helper\n\
             kittwm-browser --semantic-snapshot URL\n\
                                            first-party browser semantic snapshot\n"),
        "session" | "sessions" => Ok("kittwm help session\n\
             ===================\n\n\
             --save-session PATH|-          write SESSION_JSON manifest\n\
             --restore-session PATH|-       queue RESTORE_SESSION_JSON\n\
             --session-json                 print current SESSION_JSON\n\
             kittwm --save-session session.json\n\
                                            save current layout to a file\n\
             kittwm --restore-session session.json\n\
                                            restore layout from a file\n\
             kittwm --save-session -        write session JSON to stdout\n\n\
             Session manifests store layout axis, focus, pane order, titles,\n\
             commands, and weights. Restore replaces the native pane set.\n"),
        "events" | "event" => Ok("kittwm help events\n\
             ==================\n\n\
             --events                       stream bounded EVENTS output\n\
             --events-ms MS                 stream EVENTS for explicit timeout\n\
             kittwm-terminal --events-ms 1000\n\
                                            first-party text event helper\n\
             kittwm-terminal --events-scene-json 1000\n\
                                            first-party event card JSON\n\n\
             EVENTS starts with status, then pane/focus/layout/input/frame,\n\
             semantic, and surface side-effect event envelopes, ending with END.\n"),
        "apps" | "app" | "applications" | "application" | "programs" | "program" | "software" => Ok("kittwm help apps\n\
             ================\n\n\
             apps                           list launch candidates\n\
             remote HOST                    check remote kittwm availability\n\
             remote HOST status --x11      check graphical forwarding for app launch\n\
             remote HOST status --wayland  alias for graphical forwarding check\n\
             remote HOST x11               short alias for graphical forwarding check\n\
             remote HOST gui               alias for remote HOST x11\n\
             remote HOST graphical         alias for remote HOST x11\n\
             remote HOST wayland           alias for remote HOST graphical\n\
             remote HOST forwarding        alias for remote HOST x11\n\
             remote HOST forward           short alias for remote HOST forwarding\n\
             remote HOST kittwm            open remote kittwm in a pooled SSH pane\n\
             remote HOST desktop           alias for remote HOST kittwm\n\
             remote HOST wm                short alias for remote HOST kittwm\n\
             remote HOST list              list remote app candidates\n\
             remote HOST list apps QUERY    list remote app matches with a natural alias\n\
             remote HOST list app QUERY     singular alias for remote HOST list apps\n\
             remote HOST list windows QUERY list remote windows matching a query\n\
             remote HOST list windows QUERY --json\n\
                                            structured remote window matches\n\
             remote HOST list windows QUERY --fallback\n\
                                            skip remote kittwm and force platform fallback listing\n\
             remote HOST list win QUERY     short alias for remote HOST list windows\n\
             remote HOST list displays QUERY\n\
                                            list remote displays matching a query\n\
             remote HOST list monitors QUERY\n\
                                            alias for remote HOST list displays\n\
             remote HOST list monitors QUERY --fallback\n\
                                            skip remote kittwm and force platform fallback listing\n\
             remote HOST list screens QUERY\n\
                                            alias for remote HOST list displays\n\
             remote HOST apps QUERY         list remote app matches with a positional query\n\
             remote HOST apps QUERY --json  structured remote app matches with counts\n\
             remote HOST apps QUERY --fallback\n\
                                            skip remote kittwm and force pooled-SSH fallback discovery\n\
             remote HOST fallback apps QUERY force pooled-SSH fallback app discovery\n\
             remote HOST fallback launch QUERY\n\
                                            force pooled-SSH fallback app launch\n\
             remote HOST fallback open QUERY alias for fallback launch\n\
             remote HOST fallback run QUERY  alias for fallback launch\n\
             remote HOST fallback start QUERY\n\
                                            alias for fallback launch\n\
             remote HOST fallback windows QUERY\n\
                                            force platform fallback window listing\n\
             remote HOST fallback displays QUERY\n\
                                            force platform fallback display listing\n\
             remote HOST applications QUERY alias for remote HOST apps QUERY\n\
             remote HOST programs QUERY     alias for remote HOST apps QUERY\n\
             remote HOST software QUERY     alias for remote HOST apps QUERY\n\
             remote HOST app QUERY          select the first remote app match\n\
             remote HOST app QUERY --json   structured first remote app match\n\
             remote HOST application QUERY --json\n\
                                            structured alias for remote HOST app QUERY\n\
             remote HOST program QUERY --json\n\
                                            structured alias for remote HOST app QUERY\n\
             remote HOST select QUERY       alias for remote HOST app QUERY
\
             remote HOST pick QUERY --json  structured alias for remote HOST app QUERY
\
             remote HOST launch QUERY       shortest alias for remote app launch\n\
             remote HOST launch QUERY --fallback\n\
                                            skip remote kittwm and force pooled-SSH fallback launch\n\
             remote HOST open QUERY         natural alias for remote app launch\n\
             remote HOST run QUERY          natural alias for remote app launch\n\
             remote HOST start QUERY        natural alias for remote app launch
\
             remote HOST apps QUERY --launch-first\n\
                                            explicit first remote match launch alias\n\
             remote HOST launch QUERY --json\n\
                                            structured launch result or error\n\
             remote HOST terminal CMD       open remote command in a pooled SSH pane\n\
             remote HOST term CMD           short alias for remote HOST terminal CMD\n\
             remote HOST cmd CMD            command-style alias for remote HOST terminal CMD\n\
             remote HOST command CMD        command-style alias for remote HOST terminal CMD\n\
             remote HOST exec CMD           exec-style alias for remote HOST terminal CMD\n\
             remote HOST sh CMD             shell-style alias for remote HOST terminal CMD\n\
             remote HOST login CMD         login-shell alias for remote HOST terminal CMD\n\
             remote HOST console CMD        alias for remote HOST terminal CMD\n\
             remote HOST tty CMD            alias for remote HOST terminal CMD\n\
             apps --remote HOST             list remote candidates via pooled SSH\n\
             apps --remote HOST --filter QUERY --launch-first\n\
                                            launch first remote match; uses remote kittwm when present\n\
             windows --remote HOST          list remote windows via pooled SSH\n\
             displays --remote HOST         list remote displays via pooled SSH\n\
             --apps-json                    APPS_JSON catalog\n\
             --apps-first QUERY             first matching app candidate\n\
             --apps-launch-first QUERY      launch first matching candidate\n\
             launcher [--filter Q] [--limit N]\n\
                                            boxed launcher preview\n\
             kittwm-launch                  first-party SDK launcher helper\n\
             kittwm-launch --browser URL    launch first-party browser helper\n\
             kittwm-terminal --title logs -- tail -f /tmp/app.log\n\
                                            launch a titled terminal helper\n\
             kittwm-top                     first-party SDK process viewer\n\
             kittwm-bar --kitty --reserve   kitty-native top bar chrome app; reserves drawable row\n\
             kittwm-bar --release           clear the bar chrome reservation\n"),
        other => Err(friendly_unknown_help_topic_error(other)),
    }
}

fn known_help_topics() -> &'static [&'static str] {
    &[
        "topics",
        "start",
        "panes",
        "input",
        "inspect",
        "session",
        "events",
        "apps",
        "ssh",
        "log",
        "completions",
    ]
}

fn known_kittwm_commands() -> &'static [&'static str] {
    &[
        "quickstart",
        "quickstart-scene-json",
        "quickstart-kitty",
        "quickstart-graphics",
        "examples-scene-json",
        "examples-kitty",
        "examples-graphics",
        "cheat-scene-json",
        "cheat-kitty",
        "cheat-graphics",
        "info",
        "help",
        "help-scene-json",
        "help-kitty",
        "help-graphics",
        "status",
        "status-scene-json",
        "status-kitty",
        "status-graphics",
        "chrome-scene-json",
        "chrome-kitty",
        "chrome-graphics",
        "session-scene-json",
        "session-kitty",
        "session-graphics",
        "panes",
        "panes-json",
        "events",
        "spawn",
        "read",
        "read-json",
        "type",
        "line",
        "key",
        "wait",
        "focus",
        "close",
        "layout",
        "move",
        "resize",
        "balance",
        "rename",
        "apps",
        "remote",
        "ssh",
        "apps-scene-json",
        "apps-kitty",
        "apps-graphics",
        "launcher-scene-json",
        "launcher-kitty",
        "launcher-graphics",
        "shortcuts",
        "shortcuts-json",
        "architecture-json",
        "architecture-scene-json",
        "architecture-kitty",
        "architecture-graphics",
        "native-surfaces",
        "native-surfaces-json",
        "native-surfaces-scene-json",
        "native-surfaces-kitty",
        "native-surfaces-graphics",
        "doctor",
        "config",
        "config-scene-json",
        "config-kitty",
        "config-graphics",
        "keymap",
        "keymap-scene-json",
        "keymap-kitty",
        "keymap-graphics",
        "completions",
    ]
}

fn friendly_unknown_command_error(command: &str) -> anyhow::Error {
    let suggestion = closest_command(command, known_kittwm_commands());
    let mut msg = String::with_capacity(command.len().saturating_add(128));
    let _ = write!(msg, "unknown kittwm command or flag {command:?}.");
    if let Some(suggestion) = suggestion {
        msg.push_str("\n\nDid you mean?\n  kittwm ");
        msg.push_str(suggestion);
    }
    msg.push_str(
        "\n\nStart here:\n  kittwm quickstart\n  kittwm examples\n  kittwm cheat\n  kittwm --help\n  kittwm help topics\n",
    );
    anyhow!(msg)
}

fn extra_help_topic_error(command: &str, extra: &str) -> anyhow::Error {
    anyhow!(
        "kittwm {command} accepts at most one topic, got {extra:?}\ntry: kittwm {command} panes\nhelp: kittwm help topics"
    )
}

fn friendly_unknown_help_topic_error(topic: &str) -> anyhow::Error {
    let suggestion = closest_command(topic, known_help_topics());
    let mut msg = String::with_capacity(topic.len().saturating_add(96));
    let _ = write!(msg, "unknown kittwm help topic {topic:?}.");
    if let Some(suggestion) = suggestion {
        msg.push_str("\n\nDid you mean?\n  kittwm help ");
        msg.push_str(suggestion);
    }
    msg.push_str(
        "\n\nAvailable topics:\n  kittwm help topics\n  kittwm help panes\n  kittwm help input\n  kittwm help inspect\n  kittwm help log\n  kittwm help completions\n  kittwm quickstart\n",
    );
    anyhow!(msg)
}

fn closest_command<'a>(input: &str, commands: &'a [&'a str]) -> Option<&'a str> {
    let normalized = input.trim_start_matches('-').to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if let Some(command) = commands
        .iter()
        .copied()
        .find(|command| *command == normalized)
    {
        return Some(command);
    }
    if let Some(command) = commands
        .iter()
        .copied()
        .find(|command| command.starts_with(&normalized) || normalized.starts_with(*command))
    {
        return Some(command);
    }
    commands
        .iter()
        .copied()
        .filter_map(|command| {
            let distance = levenshtein_distance(&normalized, command);
            (distance <= 3).then_some((distance, command))
        })
        .min_by_key(|(distance, command)| (*distance, command.len()))
        .map(|(_, command)| command)
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ac) in a.bytes().enumerate() {
        curr[0] = i + 1;
        for (j, bc) in b.bytes().enumerate() {
            let cost = usize::from(ac != bc);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

fn lifecycle_alias_mode(alias: &str) -> Result<Mode> {
    match alias {
        "start" => Ok(Mode::Session),
        "stop" => Ok(Mode::Kill),
        other => Err(anyhow!("unknown lifecycle alias {other:?}")),
    }
}

fn parse_limit_value(value: &str) -> Result<usize> {
    value.parse().map_err(|_| limit_parse_error(value))
}

fn missing_limit_error() -> anyhow::Error {
    anyhow!("--limit requires an integer\ntry: kittwm apps --limit 10\nhelp: kittwm help apps")
}

fn limit_parse_error(value: &str) -> anyhow::Error {
    anyhow!("--limit expects integer, got {value:?}\ntry: kittwm apps --limit 10\nhelp: kittwm help apps")
}

fn missing_filter_error() -> anyhow::Error {
    anyhow!("--filter requires a query\ntry: kittwm apps --filter terminal\nhelp: kittwm help apps")
}

fn remote_alias_missing_host_error() -> anyhow::Error {
    anyhow!(
        "kittwm remote requires HOST\ntry: kittwm remote buildbox doctor\nhelp: kittwm help ssh"
    )
}

fn parse_remote_alias_action(out: &mut Cli, action: &str, rest: &[String]) -> Result<()> {
    match action {
        "doctor" | "status" | "check" => {
            out.doctor = true;
            parse_remote_doctor_flags(out, rest)
        }
        "x11" | "gui" | "graphical" | "wayland" | "forwarding" | "forward" => {
            out.doctor = true;
            out.remote_doctor_graphical = true;
            parse_remote_doctor_flags(out, rest)
        }
        "help" | "usage" => ensure_empty_remote_help_args(rest, || {
            out.remote_help = true;
        }),
        "apps" | "applications" | "programs" | "software" => {
            out.apps = true;
            parse_remote_apps_flags(out, rest)
        }
        "app" | "application" | "program" | "select" | "pick" => {
            out.apps = true;
            out.apps_first = true;
            parse_remote_apps_flags(out, rest)
        }
        "list" | "ls" => parse_remote_list_alias(out, rest),
        "fallback" => parse_remote_fallback_alias(out, rest),
        "launch" | "open" | "run" | "start" => parse_remote_launch_alias(out, rest),
        "kittwm" | "desktop" | "wm" => {
            out.remote_terminal_args = Some(remote_kittwm_alias_args(
                out.remote_host.as_deref().unwrap_or("HOST"),
                rest,
            ));
            Ok(())
        }
        "windows" | "window" | "wins" | "win" => {
            parse_remote_listing_alias(out, RemoteListingKind::Windows, rest)
        }
        "displays" | "display" | "monitors" | "monitor" | "screens" | "screen" => {
            parse_remote_listing_alias(out, RemoteListingKind::Displays, rest)
        }
        "terminal" | "term" | "cmd" | "command" | "exec" | "shell" | "sh" | "login" | "ssh"
        | "console" | "tty" => {
            out.remote_terminal_args = Some(remote_terminal_alias_args(
                out.remote_host.as_deref().unwrap_or("HOST"),
                rest,
            ));
            Ok(())
        }
        flag if flag.starts_with('-') => {
            out.doctor = true;
            let mut flags = Vec::with_capacity(rest.len() + 1);
            flags.push(flag.to_string());
            flags.extend(rest.iter().cloned());
            parse_remote_doctor_flags(out, &flags)
        }
        other => Err(anyhow!(
            "unknown remote action {other:?}\ntry: kittwm remote HOST help | doctor | status | x11 | gui | wayland | kittwm | list | apps | app | select | launch | windows | displays | terminal | shell\nhelp: kittwm help ssh"
        )),
    }
}

fn ensure_empty_remote_help_args(rest: &[String], apply: impl FnOnce()) -> Result<()> {
    if let Some(extra) = rest.first() {
        return Err(anyhow!(
            "kittwm remote HOST help accepts no extra argument {extra:?}\ntry: kittwm remote HOST help"
        ));
    }
    apply();
    Ok(())
}

fn parse_remote_fallback_alias(out: &mut Cli, rest: &[String]) -> Result<()> {
    let Some((action, rest)) = rest.split_first() else {
        return Err(anyhow!(
            "kittwm remote HOST fallback requires apps|launch|windows|displays\ntry: kittwm remote HOST fallback apps firefox\nhelp: kittwm help ssh"
        ));
    };
    out.apps_force_fallback = true;
    out.remote_listing_force_fallback = true;
    match action.as_str() {
        "apps" | "applications" | "programs" | "software" => {
            out.apps = true;
            parse_remote_apps_flags(out, rest)
        }
        "app" | "application" | "program" | "select" | "pick" => {
            out.apps = true;
            out.apps_first = true;
            parse_remote_apps_flags(out, rest)
        }
        "launch" | "open" | "run" | "start" => parse_remote_launch_alias(out, rest),
        "list" | "ls" => parse_remote_list_alias(out, rest),
        "windows" | "window" | "wins" | "win" => {
            parse_remote_listing_alias(out, RemoteListingKind::Windows, rest)
        }
        "displays" | "display" | "monitors" | "monitor" | "screens" | "screen" => {
            parse_remote_listing_alias(out, RemoteListingKind::Displays, rest)
        }
        other => Err(anyhow!(
            "unknown remote fallback target {other:?}\ntry: kittwm remote HOST fallback apps firefox | fallback launch firefox | fallback open firefox | fallback windows firefox | fallback displays retina\nhelp: kittwm help ssh"
        )),
    }
}

fn remote_terminal_alias_args(host: &str, rest: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(rest.len() + 4);
    args.push("--remote".to_string());
    args.push(host.to_string());
    if !remote_alias_args_include_title(rest) {
        args.push("--title".to_string());
        args.push(remote_terminal_alias_title(host, rest));
    }
    args.extend(rest.iter().cloned());
    args
}

fn remote_terminal_alias_title(host: &str, rest: &[String]) -> String {
    let command = rest
        .split(|arg| arg == "--")
        .next_back()
        .unwrap_or(rest)
        .iter()
        .find(|arg| !arg.starts_with('-'))
        .map(String::as_str);
    command.map_or_else(|| host.to_string(), |command| format!("{host}: {command}"))
}

fn remote_kittwm_alias_args(host: &str, rest: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(rest.len() + 6);
    args.push("--remote".to_string());
    args.push(host.to_string());
    if !remote_alias_args_include_title(rest) {
        args.push("--title".to_string());
        args.push(host.to_string());
    }
    args.push("--".to_string());
    args.push("kittwm".to_string());
    args.extend(rest.iter().cloned());
    args
}

fn remote_alias_args_include_title(rest: &[String]) -> bool {
    rest.iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| arg == "--title")
}

fn parse_remote_doctor_flags(out: &mut Cli, flags: &[String]) -> Result<()> {
    for flag in flags {
        match flag.as_str() {
            "--json" => out.json = true,
            "--x11" | "--gui" | "--graphical" | "--wayland" | "--forwarding" | "--forward" => {
                out.remote_doctor_graphical = true
            }
            other => {
                return Err(anyhow!(
                    "unknown remote doctor flag {other:?}\ntry: kittwm remote HOST doctor --json | --x11 | --wayland | --forward\nhelp: kittwm help ssh"
                ))
            }
        }
    }
    Ok(())
}

fn parse_remote_listing_alias(
    out: &mut Cli,
    kind: RemoteListingKind,
    args: &[String],
) -> Result<()> {
    let query = remote_listing_query(args, out)?;
    match kind {
        RemoteListingKind::Windows => out.list_windows = true,
        RemoteListingKind::Displays => out.list_displays = true,
    }
    out.remote_listing_filter = query;
    Ok(())
}

fn remote_listing_query(args: &[String], out: &mut Cli) -> Result<Option<String>> {
    if args.is_empty() {
        return Ok(None);
    }
    let mut terms = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--filter" | "--query" => terms.push(iter.next().ok_or_else(missing_filter_error)?.clone()),
            "--json" => out.json = true,
            "--fallback" => out.remote_listing_force_fallback = true,
            "--" => {
                terms.extend(iter.cloned());
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(anyhow!(
                    "unknown remote listing flag {flag:?}\ntry: kittwm remote HOST windows firefox\nhelp: kittwm help ssh"
                ))
            }
            term => terms.push(term.to_string()),
        }
    }
    if terms.is_empty() {
        Ok(None)
    } else {
        Ok(Some(terms.join(" ")))
    }
}

fn parse_remote_list_alias(out: &mut Cli, args: &[String]) -> Result<()> {
    let Some(kind) = args.first().map(String::as_str) else {
        out.apps = true;
        return Ok(());
    };
    match kind {
        "apps" | "applications" | "programs" | "software" | "app" | "application" | "program" => {
            out.apps = true;
            parse_remote_apps_flags(out, &args[1..])
        }
        "windows" | "window" | "wins" | "win" => {
            parse_remote_listing_alias(out, RemoteListingKind::Windows, &args[1..])
        }
        "displays" | "display" | "monitors" | "monitor" | "screens" | "screen" => {
            parse_remote_listing_alias(out, RemoteListingKind::Displays, &args[1..])
        }
        "terminals" | "terminal" | "terms" | "term" => Err(anyhow!(
            "remote terminal listing is not supported yet\ntry: kittwm remote HOST terminal\nhelp: kittwm help ssh"
        )),
        other if other.starts_with('-') => {
            out.apps = true;
            parse_remote_apps_flags(out, args)
        }
        _ => {
            out.apps = true;
            parse_remote_apps_flags(out, args)
        }
    }
}

fn parse_remote_launch_alias(out: &mut Cli, args: &[String]) -> Result<()> {
    let mut terms = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => out.json = true,
            "--filter" => terms.push(iter.next().ok_or_else(missing_filter_error)?.clone()),
            "--limit" => {
                let value = iter.next().ok_or_else(missing_limit_error)?;
                out.apps_limit = Some(parse_limit_value(value)?);
            }
            "--fallback" => out.apps_force_fallback = true,
            "--" => {
                terms.extend(iter.cloned());
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(anyhow!(
                    "unknown remote launch flag {flag:?}\ntry: kittwm remote HOST launch firefox\nhelp: kittwm help ssh"
                ))
            }
            term => terms.push(term.to_string()),
        }
    }
    if terms.is_empty() {
        return Err(anyhow!(
            "kittwm remote HOST launch requires an app query\ntry: kittwm remote buildbox launch firefox\nhelp: kittwm help ssh"
        ));
    }
    out.apps = true;
    out.apps_filter = Some(terms.join(" "));
    out.apps_launch_first = true;
    Ok(())
}

fn parse_remote_apps_flags(out: &mut Cli, flags: &[String]) -> Result<()> {
    let mut query_terms = Vec::new();
    let mut iter = flags.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--json" => out.json = true,
            "--filter" => query_terms.push(iter.next().ok_or_else(missing_filter_error)?.clone()),
            "--limit" => {
                let value = iter.next().ok_or_else(missing_limit_error)?;
                out.apps_limit = Some(parse_limit_value(value)?);
            }
            "--first" => out.apps_first = true,
            "--launch-first" => out.apps_launch_first = true,
            "--fallback" => out.apps_force_fallback = true,
            "--" => {
                query_terms.extend(iter.cloned());
                break;
            }
            other if other.starts_with('-') => {
                return Err(anyhow!(
                    "unknown remote apps flag {other:?}\ntry: kittwm remote HOST apps firefox\nhelp: kittwm help ssh"
                ))
            }
            term => query_terms.push(term.to_string()),
        }
    }
    if !query_terms.is_empty() {
        out.apps_filter = Some(query_terms.join(" "));
    }
    Ok(())
}

fn debug_log_path() -> String {
    std::env::var("KITTUI_WM_LOG").unwrap_or_else(|_| "/tmp/kittui-wm.log".to_string())
}

fn parse_log_command(argv: &[String]) -> Result<LogCommand> {
    match argv {
        [] => Ok(LogCommand::Path),
        [cmd] if cmd == "path" => Ok(LogCommand::Path),
        [cmd] if cmd == "tail" => Ok(LogCommand::Tail { follow: false }),
        [cmd, flag] if cmd == "tail" && (flag == "-f" || flag == "--follow") => {
            Ok(LogCommand::Tail { follow: true })
        }
        _ => Err(log_usage_error()),
    }
}

fn log_usage_error() -> anyhow::Error {
    anyhow!(
        "usage: kittwm log path | kittwm log tail [-f]\ntry: kittwm log tail -f\nhelp: kittwm help log"
    )
}

fn log_cmd(command: LogCommand) -> Result<()> {
    let path = debug_log_path();
    match command {
        LogCommand::Path => {
            println!("{path}");
            Ok(())
        }
        LogCommand::Tail { follow } => {
            let mut cmd = std::process::Command::new("tail");
            cmd.arg("-n").arg("100");
            if follow {
                cmd.arg("-f");
            }
            let status = cmd.arg(&path).status()?;
            if status.success() {
                Ok(())
            } else {
                Err(anyhow!("tail exited with status {status}"))
            }
        }
    }
}

fn spawn_alias_request(argv: &[String]) -> Result<String> {
    if argv.is_empty() {
        return Err(anyhow!("usage: kittwm spawn CMD [ARGS...]"));
    }
    protocol_payload_request("SPAWN_PTY", &argv_to_shell_words(argv))
}

fn split_alias_request(argv: &[String]) -> Result<String> {
    if argv.is_empty() {
        return Err(anyhow!(
            "usage: kittwm split [WINDOW] columns|rows|grid CMD [ARGS...]"
        ));
    }
    let (window, axis, command_argv) = if matches!(argv[0].as_str(), "columns" | "rows" | "grid") {
        ("focused", argv[0].as_str(), &argv[1..])
    } else if argv.len() >= 2 && matches!(argv[1].as_str(), "columns" | "rows" | "grid") {
        (argv[0].as_str(), argv[1].as_str(), &argv[2..])
    } else {
        return Err(anyhow!(
            "usage: kittwm split [WINDOW] columns|rows|grid CMD [ARGS...]"
        ));
    };
    if command_argv.is_empty() {
        return Err(anyhow!(
            "usage: kittwm split [WINDOW] columns|rows|grid CMD [ARGS...]"
        ));
    }
    split_pane_request(window, axis, &argv_to_shell_words(command_argv))
}

fn read_alias_request(json: bool, argv: &[String]) -> Result<String> {
    let window = match argv {
        [] => "focused",
        [window] => window.as_str(),
        _ => return Err(anyhow!("usage: kittwm read[-json] [WINDOW]")),
    };
    automation_request(
        if json { "READ_TEXT_JSON" } else { "READ_TEXT" },
        window,
        "",
    )
}

fn default_window_payload_alias(verb: &str, label: &str, argv: &[String]) -> Result<String> {
    let (window, payload) = match argv {
        [payload] => ("focused", payload.as_str()),
        [window, payload] => (window.as_str(), payload.as_str()),
        [] => return Err(anyhow!("usage: kittwm {label} [WINDOW] VALUE")),
        _ => return Err(anyhow!("usage: kittwm {label} [WINDOW] VALUE")),
    };
    let normalized_verb = verb.trim().to_ascii_uppercase();
    if normalized_verb.starts_with("WAIT_") {
        wait_request(verb, window, payload)
    } else if normalized_verb == "SEND_KEY" {
        send_key_request(window, payload)
    } else if normalized_verb == "PASTE_BYTES_B64" {
        paste_text_request(window, payload, label)
    } else {
        text_payload_request(verb, window, payload, label)
    }
}

fn parse_pane_control_alias(alias: &str, mut args: impl Iterator<Item = String>) -> Result<String> {
    let mut next = || args.next();
    let request = match alias {
        "focus" => {
            let window = next().ok_or_else(|| anyhow!("kittwm focus WINDOW"))?;
            protocol_token_request("FOCUS_PANE", &window)?
        }
        "close" => {
            let window = next().unwrap_or_else(|| "focused".to_string());
            protocol_token_request("CLOSE_PANE", &window)?
        }
        "layout" => {
            let axis = next().ok_or_else(|| anyhow!("kittwm layout columns|rows|grid"))?;
            layout_request(&axis)?
        }
        "move" => {
            let first = next().ok_or_else(|| anyhow!("kittwm move [WINDOW] DIR"))?;
            let second = next();
            let (window, direction) = match second {
                Some(direction) => (first, direction),
                None => ("focused".to_string(), first),
            };
            move_pane_request(&window, &direction)?
        }
        "raise" => {
            let window = next().unwrap_or_else(|| "focused".to_string());
            move_pane_request(&window, "last")?
        }
        "lower" => {
            let window = next().unwrap_or_else(|| "focused".to_string());
            move_pane_request(&window, "first")?
        }
        "nudge" => {
            let first = next().ok_or_else(|| anyhow!("kittwm nudge [WINDOW] DX DY"))?;
            let second = next().ok_or_else(|| anyhow!("kittwm nudge [WINDOW] DX DY"))?;
            let third = next();
            let (window, dx, dy) = match third {
                Some(dy) => (first, second, dy),
                None => ("focused".to_string(), first, second),
            };
            nudge_pane_request(&window, &dx, &dy)?
        }
        "reset-position" | "reset-offset" => {
            let window = next().unwrap_or_else(|| "focused".to_string());
            reset_pane_offset_request(&window)?
        }
        "reset-positions" | "reset-offsets" => "RESET_ALL_PANE_OFFSETS".to_string(),
        "resize" => {
            let first = next().ok_or_else(|| anyhow!("kittwm resize [WINDOW] AMOUNT"))?;
            let second = next();
            let (window, amount) = match second {
                Some(amount) => (first, amount),
                None => ("focused".to_string(), first),
            };
            resize_pane_request(&window, &amount)?
        }
        "balance" | "reset-weights" | "reset-weight" => "BALANCE_PANES".to_string(),
        "rename" => {
            let window = next().ok_or_else(|| anyhow!("kittwm rename WINDOW TITLE"))?;
            let title = next().ok_or_else(|| anyhow!("kittwm rename WINDOW TITLE"))?;
            rename_pane_request(&window, &title)?
        }
        _ => return Err(anyhow!("unknown pane control alias {alias:?}")),
    };
    if let Some(extra) = next() {
        return Err(anyhow!(
            "kittwm {alias} got unexpected extra argument {extra:?}"
        ));
    }
    Ok(request)
}

fn parse_inspection_alias(
    alias: &str,
    arg: Option<String>,
    extra: Option<String>,
) -> Result<Option<String>> {
    match alias {
        "status" => {
            if let Some(arg) = arg {
                return Err(anyhow!(
                    "kittwm status does not accept an argument, got {arg:?}"
                ));
            }
            Ok(None)
        }
        "panes" => {
            if let Some(arg) = arg {
                return Err(anyhow!(
                    "kittwm panes does not accept an argument, got {arg:?}"
                ));
            }
            Ok(Some("PANES".to_string()))
        }
        "panes-json" => {
            if let Some(arg) = arg {
                return Err(anyhow!(
                    "kittwm panes-json does not accept an argument, got {arg:?}"
                ));
            }
            Ok(Some("PANES_JSON".to_string()))
        }
        "events" => {
            if let Some(extra) = extra {
                return Err(anyhow!(
                    "kittwm events accepts at most one millisecond timeout, got {extra:?}"
                ));
            }
            Ok(Some(match arg {
                Some(ms) => events_request(&ms)?,
                None => "EVENTS".to_string(),
            }))
        }
        _ => Err(anyhow!("unknown inspection alias {alias:?}")),
    }
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
    #[cfg(target_os = "linux")]
    {
        return Backend::Xvfb;
    }
    #[cfg(not(any(all(target_os = "macos", feature = "quartz"), target_os = "linux")))]
    {
        Backend::Fake
    }
}

fn main() -> ExitCode {
    cli_update::maybe_apply_staged_update("kittwm");
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !panic_payload_is_broken_pipe(info.payload()) {
            default_panic_hook(info);
        }
    }));
    match std::panic::catch_unwind(real_main) {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(e)) => {
            let message = fatal_error_log_line(&e);
            log_kittwm_process_event(&message);
            eprintln!("kittwm: {e}");
            ExitCode::from(1)
        }
        Err(payload) if panic_payload_is_broken_pipe(payload.as_ref()) => ExitCode::SUCCESS,
        Err(payload) => {
            let message = panic_log_line(&panic_payload_message(payload.as_ref()));
            log_kittwm_process_event(&message);
            ExitCode::from(101)
        }
    }
}

fn fatal_error_log_line(error: &anyhow::Error) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("fatal error: ".len() + 96);
    out.push_str("fatal error: ");
    let _ = write!(out, "{error:#}");
    out
}

fn panic_log_line(message: &str) -> String {
    let mut out = String::with_capacity("panic: ".len() + message.len());
    out.push_str("panic: ");
    out.push_str(message);
    out
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
        .to_string()
}

fn panic_payload_is_broken_pipe(payload: &(dyn std::any::Any + Send)) -> bool {
    let message = panic_payload_message(payload);
    message.contains("Broken pipe") || message.contains("os error 32")
}

fn log_kittwm_process_event(message: &str) {
    let path = debug_log_path();
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(file, "[{}] {}", kittwm_log_clock(), message);
    let _ = file.flush();
}

fn kittwm_log_clock() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    kittwm_log_clock_line(now.as_secs(), now.subsec_millis())
}

fn kittwm_log_clock_line(secs: u64, millis: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(24);
    let _ = write!(out, "{secs}");
    out.push('.');
    let _ = write!(out, "{millis:03}");
    out
}

fn real_main() -> Result<()> {
    let cli = parse_args()?;
    apply_socket_target_flags(&cli);
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
    if cli.remote_help {
        return remote_help_cmd(cli.remote_host.as_deref().unwrap_or("HOST"));
    }
    if let Some(args) = &cli.remote_terminal_args {
        return remote_terminal_alias_cmd(args);
    }
    if cli.doctor || cli.doctor_scene_json || cli.doctor_kitty {
        if let Some(host) = cli.remote_host.as_deref() {
            if cli.doctor_scene_json || cli.doctor_kitty {
                return Err(anyhow!(
                    "remote doctor supports text/json output; run `ssh {host} kittwm doctor-kitty` when remote kittwm is installed"
                ));
            }
            return remote_doctor_cmd(host, cli.json, cli.remote_doctor_graphical);
        }
        return doctor_cmd(
            cli.json,
            cli.doctor_scene_json,
            cli.doctor_kitty,
            cli.probe_kitty || kitty_probe_env_enabled(),
        );
    }
    if cli.config || cli.config_scene_json || cli.config_kitty {
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
    if let Some(command) = cli.log_command {
        return log_cmd(command);
    }
    if cli.replace {
        return replace_cmd(&cli);
    }
    if cli.launcher_preview || cli.launcher_scene_json || cli.launcher_kitty {
        return launcher_preview_cmd(&cli);
    }
    if cli.keymap || cli.keymap_scene_json || cli.keymap_kitty {
        return keymap_cmd(&cli);
    }
    if let Some(topic) = &cli.help_scene_topic {
        return help_topic_graphical_cmd(topic, false);
    }
    if let Some(topic) = &cli.help_kitty_topic {
        return help_topic_graphical_cmd(topic, true);
    }
    if let Some(topic) = &cli.help_topic {
        return help_topic_cmd(topic);
    }
    if cli.info || cli.info_scene_json || cli.info_kitty {
        return info_cmd(cli.info_scene_json, cli.info_kitty);
    }
    if cli.panes_scene_json || cli.panes_kitty {
        return panes_graphical_cmd(cli.panes_kitty);
    }
    if let Some(ms) = cli.events_scene_json {
        return events_graphical_cmd(ms, false);
    }
    if let Some(ms) = cli.events_kitty {
        return events_graphical_cmd(ms, true);
    }
    if cli.quickstart || cli.quickstart_scene_json || cli.quickstart_kitty {
        return quickstart_cmd(cli.quickstart_scene_json, cli.quickstart_kitty);
    }
    if cli.examples || cli.examples_scene_json || cli.examples_kitty {
        return examples_cmd(cli.examples_scene_json, cli.examples_kitty);
    }
    if cli.cheat || cli.cheat_scene_json || cli.cheat_kitty {
        return cheat_cmd(cli.cheat_scene_json, cli.cheat_kitty);
    }
    if cli.commands {
        return commands_cmd();
    }
    if cli.commands_json {
        return commands_json_cmd();
    }
    if cli.commands_scene_json || cli.commands_kitty {
        return commands_graphical_cmd(cli.commands_kitty);
    }
    if cli.architecture_json {
        return architecture_json_cmd();
    }
    if cli.architecture_scene_json || cli.architecture_kitty {
        return architecture_graphical_cmd(cli.architecture_kitty);
    }
    if cli.native_surfaces {
        return native_surfaces_cmd();
    }
    if cli.native_surfaces_json {
        return native_surfaces_json_cmd();
    }
    if cli.native_surfaces_scene_json || cli.native_surfaces_kitty {
        return native_surfaces_graphical_cmd(cli.native_surfaces_kitty);
    }
    if cli.showcase_scene_json {
        return showcase_scene_json_cmd();
    }
    if cli.showcase_metrics_json {
        return showcase_metrics_json_cmd();
    }
    if cli.showcase_composition_json {
        return showcase_composition_json_cmd();
    }
    if cli.tui_smoke_json {
        return tui_smoke_json_cmd();
    }
    if let Some(options) = &cli.update {
        return cli_update::run_update_command("kittwm", options);
    }
    if cli.mcp {
        return cli_update::serve_update_mcp("kittwm");
    }
    if let Some(shell) = &cli.completions {
        return completions_cmd(shell);
    }
    if cli.shortcuts {
        return shortcuts_cmd();
    }
    if cli.shortcuts_json {
        return shortcuts_json_cmd();
    }
    if cli.shortcuts_scene_json {
        return shortcuts_scene_json_cmd();
    }
    if cli.shortcuts_kitty {
        return shortcuts_kitty_cmd();
    }
    if cli.native_terminal {
        return native_terminal_cmd();
    }
    if cli.native_browser {
        return native_browser_cmd(&cli);
    }
    if cli.apps || cli.apps_scene_json || cli.apps_kitty {
        return apps_cmd(&cli);
    }
    if cli.list_windows {
        if let Some(host) = cli.remote_host.as_deref() {
            return remote_listing_cmd(
                RemoteListingKind::Windows,
                host,
                cli.remote_listing_filter.as_deref(),
                cli.json,
                cli.remote_listing_force_fallback,
            );
        }
        return list_windows_cmd();
    }
    if cli.list_displays {
        if let Some(host) = cli.remote_host.as_deref() {
            return remote_listing_cmd(
                RemoteListingKind::Displays,
                host,
                cli.remote_listing_filter.as_deref(),
                cli.json,
                cli.remote_listing_force_fallback,
            );
        }
        return list_displays_cmd();
    }
    if let Some(path) = &cli.save_session {
        return save_session_cmd(path);
    }
    if let Some(path) = &cli.restore_session {
        return restore_session_cmd(path);
    }
    if let Some((window, json)) = &cli.semantic_publish {
        return semantic_publish_cmd(window, json);
    }
    if cli.status_scene_json || cli.status_kitty {
        return status_graphical_cmd(cli.status_kitty);
    }
    if cli.chrome_scene_json || cli.chrome_kitty {
        return chrome_graphical_cmd(cli.chrome_kitty);
    }
    if cli.session_scene_json || cli.session_kitty {
        return session_graphical_cmd(cli.session_kitty);
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
    if n == 0 {
        return String::new();
    }
    let mut chars = s.chars();
    let mut out = String::with_capacity(n);
    for _ in 0..n {
        let Some(ch) = chars.next() else {
            return out;
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
        out
    } else {
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
        #[cfg(target_os = "linux")]
        Backend::Xvfb => run_with_xvfb(&runtime, cell),
        #[cfg(not(all(target_os = "macos", feature = "quartz")))]
        Backend::Quartz => Err(anyhow!(
            "Quartz backend requires --features quartz on macOS"
        )),
        #[cfg(not(target_os = "linux"))]
        Backend::Xvfb => Err(anyhow!(
            "Xvfb backend is enabled by default on Linux targets"
        )),
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

#[cfg(target_os = "linux")]
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
        let windows = QuartzServer::list_app_windows();
        let chosen = windows
            .iter()
            .find(|w| {
                ascii_casefold_contains(&w.title, needle)
                    || ascii_casefold_contains(&w.owner_name, needle)
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

fn write_stdout_or_ignore_broken_pipe(bytes: &[u8]) -> Result<()> {
    match std::io::stdout().write_all(bytes) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn remote_help_cmd(host: &str) -> Result<()> {
    println!("kittwm remote {host} help");
    println!("====================");
    println!("If remote kittwm is installed and healthy:");
    println!("  ssh {host} kittwm");
    println!("  kittwm remote {host} kittwm");
    println!("  kittwm remote {host} desktop");
    println!("  ssh {host} kittwm doctor");
    println!();
    println!("Local kittwm pooled-SSH helpers:");
    println!("  kittwm remote {host} status");
    println!("  kittwm remote {host} status --x11");
    println!("  kittwm remote {host} x11");
    println!("  kittwm remote {host} graphical");
    println!("  kittwm remote {host} wayland");
    println!("  kittwm remote {host} forwarding");
    println!("  kittwm remote {host} doctor");
    println!("  kittwm remote {host} list");
    println!("  kittwm remote {host} list apps firefox");
    println!("  kittwm remote {host} launch firefox");
    println!("  kittwm remote {host} list windows firefox");
    println!("  kittwm remote {host} list displays retina");
    println!("  kittwm remote {host} shell");
    println!("  kittwm remote {host} ssh");
    println!("  kittwm remote {host} terminal htop");
    println!();
    println!("Connections reuse ControlMaster=auto and ControlPersist=10m.");
    println!("Remote app launch requests trusted X11 forwarding with ssh -Y.");
    Ok(())
}

fn remote_terminal_alias_cmd(args: &[String]) -> Result<()> {
    let status = std::process::Command::new("kittwm-terminal")
        .args(args)
        .status()
        .map_err(|e| anyhow!("run kittwm-terminal {:?}: {e}", args))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("kittwm-terminal {:?} exited with {status}", args))
    }
}

fn remote_doctor_cmd(host: &str, json: bool, graphical: bool) -> Result<()> {
    let env = [
        (
            "KITTWM_REMOTE_DOCTOR_JSON".to_string(),
            if json { "1" } else { "0" }.to_string(),
        ),
        (
            "KITTWM_REMOTE_DOCTOR_GRAPHICAL".to_string(),
            if graphical { "1" } else { "0" }.to_string(),
        ),
        ("KITTWM_REMOTE_TARGET".to_string(), host.to_string()),
    ];
    let args = if graphical {
        pooled_ssh_args_with_forwarding(host, &env, remote_doctor_script(), true)?
    } else {
        pooled_ssh_args(host, &env, remote_doctor_script())?
    };
    let status = std::process::Command::new("ssh")
        .args(&args)
        .status()
        .map_err(|e| anyhow!("ssh remote doctor {host}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("ssh remote doctor {host} exited with {status}"))
    }
}

fn remote_doctor_script() -> &'static str {
    r#"host=$(hostname 2>/dev/null || printf unknown)
command_host=${KITTWM_REMOTE_TARGET:-$host}
term=${TERM:-}
term_program=${TERM_PROGRAM:-}
ssh_tty=${SSH_TTY:-}
display=${DISPLAY:-}
wayland_display=${WAYLAND_DISPLAY:-}
graphical=${KITTWM_REMOTE_DOCTOR_GRAPHICAL:-0}
size=$(stty size 2>/dev/null || true)
kittwm_path=$(command -v kittwm 2>/dev/null || true)
waypipe_path=$(command -v waypipe 2>/dev/null || true)
json_string() {
    printf '%s' "$1" | awk '{ gsub(/\\/, "\\\\"); gsub(/\"/, "\\\""); printf "\"%s\"", $0 }'
}
if [ -n "$kittwm_path" ]; then
    if kittwm doctor --json >/dev/null 2>&1; then
        kittwm_healthy=true
        kittwm_startup_check="kittwm doctor --json succeeded"
    else
        kittwm_healthy=false
        kittwm_startup_check="kittwm doctor --json failed"
    fi
else
    kittwm_healthy=false
    kittwm_startup_check="kittwm not found"
fi
if [ "${KITTWM_REMOTE_DOCTOR_JSON:-0}" = "1" ]; then
    printf '{"host":%s,"target_host":%s,"term":%s,"term_program":%s,"ssh_tty":%s,"stty_size":%s,"graphical_check":%s,"display":%s,"wayland_display":%s,"x11_forwarding_available":%s,"waypipe_available":%s,"waypipe_path":%s,"kittwm_available":%s,"kittwm_healthy":%s,"kittwm_path":%s,"startup_check":%s,"local_commands":[%s,%s,%s,%s,%s,%s,%s],"fallback_commands":[%s,%s,%s,%s,%s,%s,%s,%s],"terminal_commands":[%s,%s]}\n' \
        "$(json_string "$host")" "$(json_string "$command_host")" "$(json_string "$term")" "$(json_string "$term_program")" "$(json_string "$ssh_tty")" "$(json_string "$size")" "$([ "$graphical" = "1" ] && printf true || printf false)" "$(json_string "$display")" "$(json_string "$wayland_display")" "$([ -n "$display" ] && printf true || printf false)" "$([ -n "$waypipe_path" ] && printf true || printf false)" "$(json_string "$waypipe_path")" "$([ -n "$kittwm_path" ] && printf true || printf false)" "$kittwm_healthy" "$(json_string "$kittwm_path")" "$(json_string "$kittwm_startup_check")" \
        "$(json_string "kittwm remote $command_host kittwm")" "$(json_string "kittwm remote $command_host graphical")" "$(json_string "kittwm remote $command_host list")" "$(json_string "kittwm remote $command_host list apps firefox")" "$(json_string "kittwm remote $command_host launch firefox")" "$(json_string "kittwm remote $command_host list windows")" "$(json_string "kittwm remote $command_host list displays")" \
        "$(json_string "kittwm remote $command_host fallback apps firefox")" "$(json_string "kittwm remote $command_host fallback launch firefox")" "$(json_string "kittwm remote $command_host fallback windows firefox")" "$(json_string "kittwm remote $command_host fallback displays retina")" "$(json_string "kittwm remote $command_host apps firefox --fallback")" "$(json_string "kittwm remote $command_host launch firefox --fallback")" "$(json_string "kittwm remote $command_host list windows --fallback")" "$(json_string "kittwm remote $command_host list displays --fallback")" \
        "$(json_string "kittwm remote $command_host shell")" "$(json_string "kittwm remote $command_host terminal htop")"
    exit 0
fi
printf 'kittwm remote doctor\n=====================\n'
printf 'host           : %s\n' "$host"
printf 'TERM           : %s\n' "$term"
printf 'TERM_PROGRAM   : %s\n' "$term_program"
printf 'SSH_TTY        : %s\n' "$ssh_tty"
printf 'stty size      : %s\n' "${size:-unknown}"
if [ "$graphical" = "1" ]; then
    printf 'graphical check: requested trusted X11 forwarding\n'
    printf 'DISPLAY        : %s\n' "${display:-unset}"
    printf 'WAYLAND_DISPLAY: %s\n' "${wayland_display:-unset}"
    if [ -n "$display" ]; then
        printf 'X11 forwarding : available\n'
    else
        printf 'X11 forwarding : unavailable; check ssh -Y, sshd X11Forwarding, and local X server\n'
    fi
    if [ -n "$waypipe_path" ]; then
        printf 'waypipe        : %s\n' "$waypipe_path"
    else
        printf 'waypipe        : not found\n'
    fi
fi
if [ -n "$kittwm_path" ]; then
    printf 'remote kittwm  : %s\n' "$kittwm_path"
    printf 'startup check  : %s\n' "$kittwm_startup_check"
    if [ "$kittwm_healthy" = "true" ]; then
        printf 'suggestion     : run kittwm on the remote for remote desktop/window context\n'
    else
        printf 'suggestion     : use local pooled-SSH forwarding until remote kittwm packaging is fixed\n'
    fi
else
    printf 'remote kittwm  : not found\n'
    printf 'suggestion     : use local kittwm pooled-SSH forwarding commands\n'
fi
printf 'target host    : %s\n' "$command_host"
printf 'local commands : kittwm remote %s kittwm\n' "$command_host"
printf '               : kittwm remote %s graphical\n' "$command_host"
printf '               : kittwm remote %s list\n' "$command_host"
printf '               : kittwm remote %s list apps firefox\n' "$command_host"
printf '               : kittwm remote %s launch firefox\n' "$command_host"
printf '               : kittwm remote %s list windows\n' "$command_host"
printf '               : kittwm remote %s list displays\n' "$command_host"
printf 'fallback cmds  : kittwm remote %s fallback apps firefox\n' "$command_host"
printf '               : kittwm remote %s fallback launch firefox\n' "$command_host"
printf '               : kittwm remote %s fallback windows firefox\n' "$command_host"
printf '               : kittwm remote %s fallback displays retina\n' "$command_host"
printf '               : kittwm remote %s apps firefox --fallback\n' "$command_host"
printf '               : kittwm remote %s launch firefox --fallback\n' "$command_host"
printf '               : kittwm remote %s list windows --fallback\n' "$command_host"
printf '               : kittwm remote %s list displays --fallback\n' "$command_host"
printf 'terminal cmds  : kittwm remote %s shell\n' "$command_host"
printf '               : kittwm remote %s terminal htop\n' "$command_host"
"#
}

fn doctor_cmd(json: bool, scene_json: bool, kitty: bool, probe_kitty: bool) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let executable = doctor_executable_provenance(std::env::current_exe().ok());

    let feat_sck = cfg!(all(target_os = "macos", feature = "sck"));
    let feat_quartz = cfg!(all(target_os = "macos", feature = "quartz"));
    let feat_xvfb = cfg!(target_os = "linux");

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

    let terminal_info = TerminalInfo::detect();
    let display_tuning = kittui_cli::session::native_display_tuning();
    let mut transport_diagnostics = TransportDiagnostics::detect(&terminal_info);
    if probe_kitty {
        transport_diagnostics = run_kitty_doctor_probe(&terminal_info, transport_diagnostics);
    }
    let kitty_graphics = transport_diagnostics.supports_kitty;

    if scene_json || kitty {
        let scene = doctor_scene(
            &transport_diagnostics,
            log_present,
            display_count as u64,
            &display_tuning,
        );
        if scene_json {
            let mut out = serde_json::to_string(&scene)?;
            out.push('\n');
            write_stdout_or_ignore_broken_pipe(out.as_bytes())?;
        } else {
            let runtime = Runtime::builder().terminal(terminal_info).build()?;
            let options =
                kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Decoration);
            let placement = runtime.place_at_with_options(&scene, scene.footprint, &options)?;
            write_stdout_or_ignore_broken_pipe(placement.to_bytes().as_bytes())?;
        }
    } else if json {
        let mut buf = String::new();
        buf.push_str("{\n");
        let _ = writeln!(buf, "  \"version\": {version:?},");
        let _ = writeln!(
            buf,
            "  \"executable_path\": {},",
            serde_json::to_string(&executable.path)?
        );
        let _ = writeln!(
            buf,
            "  \"executable_realpath\": {},",
            serde_json::to_string(&executable.realpath)?
        );
        let _ = writeln!(buf, "  \"os\": {os:?},");
        let _ = writeln!(buf, "  \"arch\": {arch:?},");
        let _ = writeln!(
            buf,
            "  \"features\": {{\"sck\": {feat_sck}, \"quartz\": {feat_quartz}, \"xvfb\": {feat_xvfb}}},"
        );
        let _ = writeln!(buf, "  \"term\": {term:?},");
        let _ = writeln!(buf, "  \"colorterm\": {colorterm:?},");
        let _ = writeln!(buf, "  \"term_program\": {term_program:?},");
        let _ = writeln!(buf, "  \"kitty_graphics_likely\": {kitty_graphics},");
        let _ = writeln!(buf, "  \"display_count\": {display_count},");
        let _ = writeln!(
            buf,
            "  \"transport_diagnostics\": {},",
            serde_json::to_string(&transport_diagnostics)?
        );
        let _ = writeln!(
            buf,
            "  \"display_tuning\": {},",
            serde_json::to_string(&display_tuning)?
        );
        let _ = writeln!(buf, "  \"log_path\": {log_path:?},");
        let _ = writeln!(buf, "  \"log_present\": {log_present},");
        let _ = writeln!(buf, "  \"log_size_bytes\": {log_size}");
        buf.push_str("}\n");
        write_stdout_or_ignore_broken_pipe(buf.as_bytes())?;
    } else {
        let mut out = String::new();
        out.push_str("kittwm doctor\n");
        out.push_str("============\n");
        let _ = writeln!(out, "  version        : {version}");
        append_doctor_executable_rows(&mut out, &executable);
        let _ = writeln!(out, "  os / arch      : {os} / {arch}");
        let _ = writeln!(
            out,
            "  features       : sck={} quartz={} xvfb={}",
            feat_sck, feat_quartz, feat_xvfb
        );
        let _ = writeln!(out, "  TERM           : {term}");
        let _ = writeln!(out, "  COLORTERM      : {colorterm}");
        let _ = writeln!(out, "  TERM_PROGRAM   : {term_program}");
        let _ = writeln!(
            out,
            "  kitty graphics : {}",
            if kitty_graphics {
                "likely yes"
            } else {
                "unknown"
            }
        );
        let _ = writeln!(
            out,
            "  transport      : {:?} (compression={:?}, tmux={}, remote={})",
            transport_diagnostics.selected_transport,
            transport_diagnostics.compression_mode,
            transport_diagnostics.tmux,
            transport_diagnostics.remote
        );
        if let Some(source) = &transport_diagnostics.override_source {
            let _ = writeln!(out, "  transport set  : {source}");
        }
        let text_cols = doctor_text_cols();
        if let Some(reason) = &transport_diagnostics.fallback_reason {
            append_doctor_wrapped_row(&mut out, "  transport note : ", reason, text_cols);
        }
        let _ = writeln!(
            out,
            "  kitty probe    : {}",
            if transport_diagnostics.probe_attempted {
                transport_diagnostics
                    .probe_status
                    .as_deref()
                    .unwrap_or("attempted")
            } else {
                "not attempted"
            }
        );
        if let Some(supported) = transport_diagnostics.probe_supports_kitty {
            let _ = writeln!(out, "  probe support  : {supported}");
        }
        if let Some(elapsed) = transport_diagnostics.probe_elapsed_ms {
            let _ = writeln!(out, "  probe elapsed  : {elapsed} ms");
        }
        if let Some(error) = &transport_diagnostics.probe_error {
            let _ = writeln!(out, "  probe note     : {error}");
        }
        let _ = writeln!(out, "  display hidpi  : {}", display_tuning.hidpi_enabled);
        let _ = writeln!(
            out,
            "  display cell   : {}x{} px",
            display_tuning.cell_width_px, display_tuning.cell_height_px
        );
        let _ = writeln!(
            out,
            "  tile gap       : {} px ({} cols / {} rows)",
            display_tuning.tile_gap_px, display_tuning.tile_gap_cols, display_tuning.tile_gap_rows
        );
        let _ = writeln!(
            out,
            "  header/footer  : {} px -> {} rows / {} px -> {} rows",
            display_tuning.header_gap_px,
            display_tuning.header_gap_rows,
            display_tuning.footer_gap_px,
            display_tuning.footer_gap_rows
        );
        let _ = writeln!(out, "  displays       : {display_count}");
        append_doctor_log_row(&mut out, &log_path, log_present, log_size);
        out.push_str(&doctor_daily_driver_text(
            &transport_diagnostics,
            log_present,
        ));
        if cfg!(target_os = "macos") {
            out.push('\n');
            out.push_str("Hint: SCK + CGEventPost both require Screen Recording + Accessibility\n");
            out.push_str("      permissions on the terminal hosting kittwm (System Settings >\n");
            out.push_str("      Privacy & Security).\n");
        }
        write_stdout_or_ignore_broken_pipe(out.as_bytes())?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DoctorExecutableProvenance {
    path: Option<String>,
    realpath: Option<String>,
}

fn doctor_executable_provenance(path: Option<std::path::PathBuf>) -> DoctorExecutableProvenance {
    let path_string = path.as_ref().map(|path| path.display().to_string());
    let realpath = path
        .as_ref()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .map(|path| path.display().to_string());
    DoctorExecutableProvenance {
        path: path_string,
        realpath,
    }
}

fn append_doctor_executable_rows(out: &mut String, executable: &DoctorExecutableProvenance) {
    let path = executable.path.as_deref().unwrap_or("-");
    let _ = writeln!(out, "  executable     : {path}");
    if let Some(realpath) = executable
        .realpath
        .as_deref()
        .filter(|realpath| *realpath != path)
    {
        let _ = writeln!(out, "  executable real: {realpath}");
    }
}

fn append_doctor_log_row(out: &mut String, log_path: &str, log_present: bool, log_size: u64) {
    out.push_str("  log            : ");
    out.push_str(log_path);
    if log_present {
        let _ = writeln!(out, " (present, {log_size} bytes)");
    } else {
        out.push_str(" (missing)\n");
    }
}

fn doctor_scene(
    transport: &TransportDiagnostics,
    log_present: bool,
    display_count: u64,
    display_tuning: &kittui_cli::session::NativeDisplayTuning,
) -> Scene {
    let cols = doctor_scene_cols();
    let rows = 7;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let readiness = if transport.tmux {
        "tmux-terminal-fallback"
    } else if transport.supports_kitty {
        "kitty-ready"
    } else {
        "graphics-unknown"
    };
    let log_state = if log_present {
        "log-present"
    } else {
        "log-missing"
    };
    let display_label = doctor_display_tuning_label(display_tuning);
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers: vec![
            Layer {
                label: Some(doctor_backdrop_label(readiness)),
                root: Node::Rect {
                    rect: KittuiPxRect::new(0.0, 0.0, width, height),
                    fill: Paint::Solid {
                        color: Rgba::rgba(7, 17, 31, 238),
                    },
                    stroke: Some(Stroke::inside(
                        1.5,
                        Paint::Solid {
                            color: Rgba::rgba(136, 192, 208, 255),
                        },
                    )),
                    corners: Corners::uniform(8.0),
                },
            },
            Layer {
                label: Some(doctor_heading_label(
                    transport.selected_transport,
                    transport.compression_mode,
                )),
                root: Node::Rect {
                    rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                    fill: Paint::Solid {
                        color: Rgba::rgba(94, 129, 172, 210),
                    },
                    stroke: None,
                    corners: Corners {
                        tl: 8.0,
                        tr: 8.0,
                        bl: 0.0,
                        br: 0.0,
                    },
                },
            },
            Layer {
                label: Some(doctor_readiness_label(
                    readiness,
                    transport.tmux,
                    transport.remote,
                    display_count,
                    log_state,
                )),
                root: Node::Rect {
                    rect: doctor_readiness_rect(width, cell),
                    fill: Paint::Solid {
                        color: if transport.supports_kitty {
                            Rgba::rgba(163, 190, 140, 255)
                        } else {
                            Rgba::rgba(235, 203, 139, 255)
                        },
                    },
                    stroke: None,
                    corners: Corners::uniform(1.0),
                },
            },
            Layer {
                label: Some(doctor_display_label(&display_label)),
                root: Node::Rect {
                    rect: doctor_detail_rect(width, cell, 3.4),
                    fill: Paint::Solid {
                        color: Rgba::rgba(180, 142, 173, 255),
                    },
                    stroke: None,
                    corners: Corners::uniform(1.0),
                },
            },
        ],
        animation: None,
    }
}

fn doctor_readiness_rect(width: f32, cell: CellSize) -> KittuiPxRect {
    doctor_detail_rect(width, cell, 2.2)
}

fn doctor_detail_rect(width: f32, cell: CellSize, row: f32) -> KittuiPxRect {
    let inset = (width * 0.12).min(10.0).floor().max(0.0);
    let available = (width - inset * 2.0).max(1.0).min(width.max(1.0));
    KittuiPxRect::new(
        inset.min((width - 1.0).max(0.0)),
        cell.height_px as f32 * row,
        available,
        2.0,
    )
}

fn doctor_display_label(display_label: &str) -> String {
    let mut label = String::with_capacity("kittwm-doctor-display:".len() + display_label.len());
    label.push_str("kittwm-doctor-display:");
    label.push_str(display_label);
    label
}

fn doctor_readiness_label(
    readiness: &str,
    tmux: bool,
    remote: bool,
    display_count: u64,
    log_state: &str,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-doctor-readiness::tmux=:remote=:displays=:".len()
            + readiness.len()
            + 5
            + 5
            + 20
            + log_state.len(),
    );
    label.push_str("kittwm-doctor-readiness:");
    label.push_str(readiness);
    label.push_str(":tmux=");
    let _ = write!(label, "{tmux}");
    label.push_str(":remote=");
    let _ = write!(label, "{remote}");
    label.push_str(":displays=");
    let _ = write!(label, "{display_count}");
    label.push(':');
    label.push_str(log_state);
    label
}

fn doctor_heading_label(
    selected_transport: impl std::fmt::Debug,
    compression_mode: impl std::fmt::Debug,
) -> String {
    let mut label =
        String::with_capacity("kittwm-doctor-heading:transport=:compression=".len() + 32);
    label.push_str("kittwm-doctor-heading:transport=");
    let _ = write!(label, "{selected_transport:?}");
    label.push_str(":compression=");
    let _ = write!(label, "{compression_mode:?}");
    label
}

fn doctor_backdrop_label(readiness: &str) -> String {
    let mut label = String::with_capacity("kittwm-doctor-backdrop:".len() + readiness.len());
    label.push_str("kittwm-doctor-backdrop:");
    label.push_str(readiness);
    label
}

fn doctor_display_tuning_label(
    display_tuning: &kittui_cli::session::NativeDisplayTuning,
) -> String {
    let mut out = String::with_capacity(96);
    let _ = write!(
        out,
        "hidpi={}:cell={}x{}:tile_gap={}px={}x{}:header_gap={}px={}:footer_gap={}px={}",
        display_tuning.hidpi_enabled,
        display_tuning.cell_width_px,
        display_tuning.cell_height_px,
        display_tuning.tile_gap_px,
        display_tuning.tile_gap_cols,
        display_tuning.tile_gap_rows,
        display_tuning.header_gap_px,
        display_tuning.header_gap_rows,
        display_tuning.footer_gap_px,
        display_tuning.footer_gap_rows
    );
    out
}

fn doctor_scene_cols() -> u16 {
    let detected = TerminalInfo::detect().columns;
    doctor_scene_cols_from_sources(
        std::env::var("KITTWM_DOCTOR_COLS")
            .or_else(|_| std::env::var("COLUMNS"))
            .ok()
            .as_deref(),
        detected,
    )
}

fn graphical_scene_cols_from_sources(
    value: Option<&str>,
    detected_cols: Option<u16>,
    default_cols: u16,
    max_cols: u16,
) -> u16 {
    value
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|cols| *cols > 0)
        .or_else(|| detected_cols.filter(|cols| *cols > 0))
        .map(|cols| cols.min(max_cols.max(1)))
        .unwrap_or(default_cols.min(max_cols.max(1)).max(1))
}

fn doctor_scene_cols_from_sources(value: Option<&str>, detected_cols: Option<u16>) -> u16 {
    graphical_scene_cols_from_sources(value, detected_cols, 64, 120)
}

fn doctor_text_cols() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(100)
        .clamp(40, 160)
}

fn append_doctor_wrapped_row(out: &mut String, label: &str, value: &str, cols: usize) {
    let continuation = " ".repeat(label.chars().count());
    let width = cols.saturating_sub(label.chars().count()).max(12);
    let mut line = String::new();
    let mut first_line = true;
    let flush_line = |out: &mut String, line: &mut String, first_line: &mut bool| {
        if line.is_empty() {
            return;
        }
        if *first_line {
            out.push_str(label);
            *first_line = false;
        } else {
            out.push_str(&continuation);
        }
        out.push_str(line);
        out.push('\n');
        line.clear();
    };
    for word in value.split_whitespace() {
        let word_len = word.chars().count();
        if line.is_empty() {
            line.push_str(word);
        } else if line.chars().count() + 1 + word_len <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            flush_line(out, &mut line, &mut first_line);
            line.push_str(word);
        }
    }
    flush_line(out, &mut line, &mut first_line);
    if first_line {
        out.push_str(label);
        out.push('\n');
    }
}

fn doctor_daily_driver_text(transport: &TransportDiagnostics, log_present: bool) -> String {
    use kittui_cli::daemon::default_socket_path;
    let socket_path = default_socket_path();
    let socket_reachable = std::os::unix::net::UnixStream::connect(&socket_path).is_ok();
    let renderer_hint = if transport.tmux {
        "tmux detected: kittwm defaults to the pure terminal renderer; override with KITTWM_NATIVE_RENDERER=kitty only when you want graphics passthrough."
    } else if transport.supports_kitty {
        "kitty graphics likely available; if rendering is slow or remote, try KITTWM_NATIVE_RENDERER=terminal."
    } else {
        "kitty graphics not confirmed; kittwm can run with KITTWM_NATIVE_RENDERER=terminal for a stable ANSI path."
    };
    let socket_hint = doctor_socket_hint(&socket_path, socket_reachable);
    let log_hint = if log_present {
        "log file exists; use `tail -f ${KITTUI_WM_LOG:-/tmp/kittui-wm.log}` while iterating."
    } else {
        "log file missing so far; start kittwm once to create it, or set KITTUI_WM_LOG for a custom path."
    };
    let mut out = String::from("\nDaily driver readiness\n");
    let cols = doctor_text_cols();
    append_doctor_wrapped_row(&mut out, "  renderer        : ", renderer_hint, cols);
    append_doctor_wrapped_row(&mut out, "  socket          : ", &socket_hint, cols);
    append_doctor_wrapped_row(
        &mut out,
        "  next steps      : ",
        "run `kittwm quickstart`, `kittwm examples`, or `kittwm help panes` for copy-paste workflows.",
        cols,
    );
    append_doctor_wrapped_row(&mut out, "  log hint        : ", log_hint, cols);
    out
}

fn doctor_socket_hint(socket_path: &std::path::Path, socket_reachable: bool) -> String {
    let mut out = String::with_capacity(160);
    if socket_reachable {
        out.push_str("running WM detected at ");
        let _ = write!(out, "{}", socket_path.display());
        out.push_str("; inspect it with `kittwm info`, `kittwm panes`, or `kittwm events 1000`.");
    } else {
        out.push_str("no running WM socket at ");
        let _ = write!(out, "{}", socket_path.display());
        out.push_str("; start one with `kittwm`, then inspect with `kittwm info`.");
    }
    out
}

fn kitty_probe_env_enabled() -> bool {
    matches!(
        std::env::var("KITTUI_KITTY_PROBE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn run_kitty_doctor_probe(
    terminal_info: &TerminalInfo,
    diagnostics: TransportDiagnostics,
) -> TransportDiagnostics {
    match run_kitty_doctor_probe_inner(terminal_info) {
        Ok(probe) => diagnostics.with_probe(
            probe.status,
            probe.supports_kitty,
            probe.error,
            Some(probe.elapsed_ms),
        ),
        Err(err) => diagnostics.with_probe("error", None, Some(err.to_string()), None),
    }
}

struct KittyDoctorProbe {
    status: String,
    supports_kitty: Option<bool>,
    error: Option<String>,
    elapsed_ms: u64,
}

fn kitty_probe_matched_status(status: &KittyResponseStatus) -> String {
    let mut out = String::with_capacity("matched:".len() + 32);
    out.push_str("matched:");
    let _ = write!(out, "{status:?}");
    out
}

fn run_kitty_doctor_probe_inner(terminal_info: &TerminalInfo) -> Result<KittyDoctorProbe> {
    let query_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.as_millis() & 0xffff_ffff) as u32)
        .unwrap_or(1)
        .max(1);
    let query = query_capabilities(query_id, terminal_info.transport);
    {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(query.as_bytes())?;
        stdout.flush()?;
    }
    let _guard = NonblockingStdinGuard::enter()?;
    let mut stdin = std::io::stdin().lock();
    let read = read_kitty_response(
        &mut stdin,
        KittyResponseReadConfig {
            timeout: Duration::from_millis(500),
            max_bytes: 16 * 1024,
            poll_interval: Duration::from_millis(5),
        },
        |text| {
            parse_response(text)
                .map(|response| response.image_id == Some(query_id))
                .unwrap_or(false)
        },
    )?;
    match read.status {
        KittyResponseReadStatus::Matched => match parse_response(&read.response) {
            Ok(response) => {
                let supports_kitty = match response.status {
                    KittyResponseStatus::Capability(_) | KittyResponseStatus::Ok => Some(true),
                    KittyResponseStatus::Error(_) => Some(false),
                    KittyResponseStatus::Other(_) => None,
                };
                Ok(KittyDoctorProbe {
                    status: kitty_probe_matched_status(&response.status),
                    supports_kitty,
                    error: None,
                    elapsed_ms: read.elapsed_ms,
                })
            }
            Err(err) => Ok(KittyDoctorProbe {
                status: "parse_error".to_string(),
                supports_kitty: None,
                error: Some(err.to_string()),
                elapsed_ms: read.elapsed_ms,
            }),
        },
        KittyResponseReadStatus::Timeout => Ok(KittyDoctorProbe {
            status: "timeout".to_string(),
            supports_kitty: None,
            error: Some("no matching kitty response before timeout".to_string()),
            elapsed_ms: read.elapsed_ms,
        }),
        KittyResponseReadStatus::Eof => Ok(KittyDoctorProbe {
            status: "eof".to_string(),
            supports_kitty: None,
            error: Some("stdin reached EOF while probing".to_string()),
            elapsed_ms: read.elapsed_ms,
        }),
        KittyResponseReadStatus::ByteLimitExceeded => Ok(KittyDoctorProbe {
            status: "byte_limit_exceeded".to_string(),
            supports_kitty: None,
            error: Some("probe response exceeded byte limit".to_string()),
            elapsed_ms: read.elapsed_ms,
        }),
    }
}

#[cfg(unix)]
struct NonblockingStdinGuard {
    fd: i32,
    old_flags: i32,
}

#[cfg(unix)]
impl NonblockingStdinGuard {
    fn enter() -> Result<Self> {
        let fd = libc::STDIN_FILENO;
        let old_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if old_flags < 0 {
            return Err(anyhow!("fcntl(F_GETFL) failed"));
        }
        let new_flags = old_flags | libc::O_NONBLOCK;
        if unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) } < 0 {
            return Err(anyhow!("fcntl(F_SETFL O_NONBLOCK) failed"));
        }
        Ok(Self { fd, old_flags })
    }
}

#[cfg(unix)]
impl Drop for NonblockingStdinGuard {
    fn drop(&mut self) {
        let _ = unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.old_flags) };
    }
}

#[cfg(not(unix))]
struct NonblockingStdinGuard;

#[cfg(not(unix))]
impl NonblockingStdinGuard {
    fn enter() -> Result<Self> {
        Err(anyhow!(
            "kitty probe response reading is currently Unix-only"
        ))
    }
}

fn kittwm_record_default_out_dir(ts: u64) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("/tmp/kittwm-record-".len() + 20);
    out.push_str("/tmp/kittwm-record-");
    let _ = write!(out, "{ts}");
    out
}

fn kittwm_record_frame_path(out_dir: &str, frame: u32, window: usize) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(out_dir.len() + "/frame--win.png".len() + 5 + 20);
    out.push_str(out_dir);
    out.push_str("/frame-");
    let _ = write!(out, "{frame:05}");
    out.push_str("-win");
    let _ = write!(out, "{window}");
    out.push_str(".png");
    out
}

fn kittwm_record_apng_path(out_dir: &str) -> String {
    let mut out = String::with_capacity(out_dir.len() + "/kittwm.apng".len());
    out.push_str(out_dir);
    out.push_str("/kittwm.apng");
    out
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
        kittwm_record_default_out_dir(ts)
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
                let path = kittwm_record_frame_path(&out_dir, i, j);
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
        let path = kittwm_record_apng_path(&out_dir);
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
        return ascii_uppercase_string(trimmed);
    };
    let rest = rest.trim_start();
    let mut out = String::with_capacity(verb.len() + 1 + rest.len());
    push_ascii_uppercase(&mut out, verb);
    out.push(' ');
    out.push_str(rest);
    out
}

fn ascii_uppercase_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    push_ascii_uppercase(&mut out, value);
    out
}

fn push_ascii_uppercase(out: &mut String, value: &str) {
    for ch in value.chars() {
        out.push(ch.to_ascii_uppercase());
    }
}

fn protocol_token(token: &str, label: &str) -> Result<String> {
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(anyhow!("{label} must be a single nonempty token"));
    }
    Ok(token.to_string())
}

fn protocol_payload_request(verb: &str, payload: &str) -> Result<String> {
    let verb = verb.trim();
    let payload = payload.trim();
    if payload.is_empty() {
        return Err(anyhow!("{verb} requires a nonempty payload"));
    }
    let mut out = String::with_capacity(verb.len() + 1 + payload.len());
    push_ascii_uppercase(&mut out, verb);
    out.push(' ');
    out.push_str(payload);
    Ok(out)
}

fn protocol_token_request(verb: &str, token: &str) -> Result<String> {
    let verb = verb.trim();
    let token = protocol_token(token, "argument")?;
    let mut out = String::with_capacity(verb.len() + 1 + token.len());
    push_ascii_uppercase(&mut out, verb);
    out.push(' ');
    out.push_str(&token);
    Ok(out)
}

fn automation_request(verb: &str, window: &str, payload: &str) -> Result<String> {
    let verb = verb.trim();
    let window = protocol_token(window, "automation window")?;
    let mut out = String::with_capacity(
        verb.len()
            .saturating_add(1)
            .saturating_add(window.len())
            .saturating_add(if payload.is_empty() {
                0
            } else {
                1 + payload.len()
            }),
    );
    push_ascii_uppercase(&mut out, verb);
    out.push(' ');
    out.push_str(&window);
    if !payload.is_empty() {
        out.push(' ');
        out.push_str(payload);
    }
    Ok(out)
}

fn text_payload_request(verb: &str, window: &str, text: &str, label: &str) -> Result<String> {
    if text.is_empty() {
        return Err(anyhow!("{label} text must be nonempty"));
    }
    automation_request(verb, window, text)
}

fn paste_text_request(window: &str, text: &str, label: &str) -> Result<String> {
    if text.is_empty() {
        return Err(anyhow!("{label} text must be nonempty"));
    }
    paste_bytes_request(window, text.as_bytes())
}

fn semantic_snapshot_request(window: &str) -> Result<String> {
    automation_request("SEMANTIC_SNAPSHOT", window, "")
}

fn semantic_focus_request(window: &str, component: &str) -> Result<String> {
    automation_request(
        "SEMANTIC_FOCUS",
        window,
        &protocol_token(component, "semantic component")?,
    )
}

fn semantic_publish_request(window: &str, input: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| anyhow!("--semantic-publish expects valid snapshot JSON: {e}"))?;
    automation_request("SEMANTIC_PUBLISH", window, &serde_json::to_string(&value)?)
}

fn semantic_action_request(
    window: &str,
    component: &str,
    action: &str,
    payload: &str,
) -> Result<String> {
    let window = protocol_token(window, "automation window")?;
    let component = protocol_token(component, "semantic component")?;
    let action = protocol_token(action, "semantic action")?;
    serde_json::from_str::<serde_json::Value>(payload)
        .map_err(|_| anyhow!("--semantic-action JSON payload must be valid JSON"))?;
    let mut out = String::with_capacity(
        "SEMANTIC_ACTION".len() + 4 + window.len() + component.len() + action.len() + payload.len(),
    );
    out.push_str("SEMANTIC_ACTION ");
    out.push_str(&window);
    out.push(' ');
    out.push_str(&component);
    out.push(' ');
    out.push_str(&action);
    out.push(' ');
    out.push_str(payload);
    Ok(out)
}

fn send_key_request(window: &str, key: &str) -> Result<String> {
    let key = protocol_token(key, "--send-key KEY")?;
    automation_request("SEND_KEY", window, &key)
}

fn send_mouse_request(window: &str, event: &str, col: &str, row: &str) -> Result<String> {
    let window = protocol_token(window, "automation window")?;
    let event = event.trim();
    if !matches!(
        event,
        "press-left"
            | "press-middle"
            | "press-right"
            | "release"
            | "release-left"
            | "release-middle"
            | "release-right"
            | "move"
            | "move-left"
            | "move-middle"
            | "move-right"
            | "scroll-up"
            | "scroll-down"
    ) {
        return Err(anyhow!("--send-mouse event must be press-left|press-middle|press-right|release|release-left|release-middle|release-right|move|move-left|move-middle|move-right|scroll-up|scroll-down"));
    }
    let col = col
        .trim()
        .parse::<u16>()
        .map_err(|_| anyhow!("--send-mouse COL must be an integer"))?;
    let row = row
        .trim()
        .parse::<u16>()
        .map_err(|_| anyhow!("--send-mouse ROW must be an integer"))?;
    if col == 0 || row == 0 {
        return Err(anyhow!(
            "--send-mouse COL and ROW are 1-indexed and must be positive"
        ));
    }
    let mut out =
        String::with_capacity("SEND_MOUSE   65535 65535".len() + window.len() + event.len());
    let _ = write!(out, "SEND_MOUSE {window} {event} {col} {row}");
    Ok(out)
}

fn validated_base64_arg<'a>(encoded: &'a str, label: &str) -> Result<&'a str> {
    let encoded = encoded.trim();
    if encoded.is_empty() {
        return Err(anyhow!("{label} BASE64 must be nonempty"));
    }
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| anyhow!("{label} BASE64 must be valid base64: {err}"))?;
    Ok(encoded)
}

fn send_bytes_b64_request(window: &str, encoded: &str) -> Result<String> {
    let encoded = validated_base64_arg(encoded, "--send-bytes-b64")?;
    automation_request("SEND_BYTES_B64", window, encoded)
}

fn paste_bytes_b64_request(window: &str, encoded: &str) -> Result<String> {
    let encoded = validated_base64_arg(encoded, "--paste-bytes-b64")?;
    automation_request("PASTE_BYTES_B64", window, encoded)
}

fn send_bytes_request(window: &str, bytes: &[u8]) -> Result<String> {
    encoded_bytes_request("SEND_BYTES_B64", window, bytes)
}

fn paste_bytes_request(window: &str, bytes: &[u8]) -> Result<String> {
    encoded_bytes_request("PASTE_BYTES_B64", window, bytes)
}

fn encoded_bytes_request(verb: &str, window: &str, bytes: &[u8]) -> Result<String> {
    automation_request(
        verb,
        window,
        &base64::engine::general_purpose::STANDARD.encode(bytes),
    )
}

fn send_file_request(window: &str, path: &str) -> Result<String> {
    file_bytes_request(window, path, send_bytes_request)
}

fn paste_file_request(window: &str, path: &str) -> Result<String> {
    file_bytes_request(window, path, paste_bytes_request)
}

fn file_bytes_request(
    window: &str,
    path: &str,
    build: fn(window: &str, bytes: &[u8]) -> Result<String>,
) -> Result<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    if path == "-" {
        std::io::stdin().read_to_end(&mut bytes)?;
    } else {
        bytes = std::fs::read(path)?;
    }
    build(window, &bytes)
}

fn split_pane_request(window: &str, axis: &str, command: &str) -> Result<String> {
    let window = protocol_token(window, "SPLIT_PANE window")?;
    let axis = axis.trim().to_ascii_lowercase();
    if !matches!(axis.as_str(), "columns" | "rows" | "grid") {
        return Err(anyhow!("SPLIT_PANE axis must be columns|rows|grid"));
    }
    let command = command.trim();
    if command.is_empty() {
        return Err(anyhow!("SPLIT_PANE requires command"));
    }
    let mut out = String::with_capacity(
        "SPLIT_PANE ".len() + window.len() + 1 + axis.len() + 1 + command.len(),
    );
    out.push_str("SPLIT_PANE ");
    out.push_str(&window);
    out.push(' ');
    out.push_str(&axis);
    out.push(' ');
    out.push_str(command);
    Ok(out)
}

fn layout_request(axis: &str) -> Result<String> {
    let axis = axis.trim().to_ascii_lowercase();
    if !matches!(axis.as_str(), "columns" | "rows" | "grid") {
        return Err(anyhow!("--layout expects columns, rows, or grid"));
    }
    let mut out = String::with_capacity("LAYOUT ".len() + axis.len());
    out.push_str("LAYOUT ");
    out.push_str(&axis);
    Ok(out)
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
    let mut out = String::with_capacity("MOVE_PANE  ".len() + window.len() + direction.len());
    out.push_str("MOVE_PANE ");
    out.push_str(&window);
    out.push(' ');
    out.push_str(&direction);
    Ok(out)
}

fn nudge_parse_context(axis: &str, raw: &str) -> String {
    use std::fmt::Write as _;

    let mut out =
        String::with_capacity("nudge  must be an i16: ".len() + axis.len() + raw.len() + 2);
    out.push_str("nudge ");
    out.push_str(axis);
    out.push_str(" must be an i16: ");
    let _ = write!(out, "{raw:?}");
    out
}

fn nudge_pane_request(window: &str, dx: &str, dy: &str) -> Result<String> {
    let window = protocol_token(window, "window")?;
    let dx = dx
        .trim()
        .parse::<i16>()
        .with_context(|| nudge_parse_context("dx", dx))?;
    let dy = dy
        .trim()
        .parse::<i16>()
        .with_context(|| nudge_parse_context("dy", dy))?;
    if dx == 0 && dy == 0 {
        return Err(anyhow!("nudge delta must move at least one axis"));
    }
    let mut out = String::with_capacity("NUDGE_PANE   ".len() + window.len() + 16);
    out.push_str("NUDGE_PANE ");
    out.push_str(&window);
    out.push(' ');
    let _ = write!(out, "{dx} {dy}");
    Ok(out)
}

fn reset_pane_offset_request(window: &str) -> Result<String> {
    protocol_token_request("RESET_PANE_OFFSET", window)
}

fn resize_pane_request(window: &str, amount: &str) -> Result<String> {
    let window = protocol_token(window, "window")?;
    let amount = protocol_token(amount, "resize amount")?;
    let mut out = String::with_capacity("RESIZE_PANE  ".len() + window.len() + amount.len());
    out.push_str("RESIZE_PANE ");
    out.push_str(&window);
    out.push(' ');
    out.push_str(&amount);
    Ok(out)
}

fn rename_pane_request(window: &str, title: &str) -> Result<String> {
    let window = protocol_token(window, "window")?;
    let title = title.trim();
    if title.is_empty() {
        return Err(anyhow!("--rename-pane TITLE must be nonempty"));
    }
    let mut out = String::with_capacity("RENAME_PANE  ".len() + window.len() + title.len());
    out.push_str("RENAME_PANE ");
    out.push_str(&window);
    out.push(' ');
    out.push_str(title);
    Ok(out)
}

fn parse_optional_events_ms(ms: Option<String>) -> Result<u64> {
    match ms {
        Some(ms) => parse_events_ms_value(&ms),
        None => Ok(1000),
    }
}

fn parse_events_ms_value(ms: &str) -> Result<u64> {
    let parsed: u64 = ms
        .parse()
        .map_err(|_| anyhow!("events timeout expects integer milliseconds"))?;
    if parsed == 0 || parsed > 60_000 {
        return Err(anyhow!("events timeout expects 1..=60000"));
    }
    Ok(parsed)
}

fn events_request(ms: &str) -> Result<String> {
    let parsed = parse_events_ms_value(ms)
        .map_err(|_| anyhow!("--events-ms expects integer milliseconds in 1..=60000"))?;
    Ok(events_request_millis(parsed))
}

fn events_request_millis(ms: u64) -> String {
    let ms_text = ms.to_string();
    let mut out = String::with_capacity("EVENTS ".len() + ms_text.len());
    out.push_str("EVENTS ");
    out.push_str(&ms_text);
    out
}

fn wait_needle(needle: &str, verb: &str) -> Result<String> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Err(anyhow!("{verb} needle must be nonempty"));
    }
    Ok(needle.to_string())
}

fn wait_request(verb: &str, window: &str, needle: &str) -> Result<String> {
    let needle = wait_needle(needle, verb)?;
    automation_request(verb, window, &needle)
}

fn wait_ms_request(verb: &str, ms: &str, window: &str, needle: &str) -> Result<String> {
    let verb = verb.trim();
    let parsed = ms
        .trim()
        .parse::<u64>()
        .map_err(|_| anyhow!("{verb} expects integer milliseconds"))?;
    if parsed == 0 || parsed > 60_000 {
        return Err(anyhow!("{verb} must be in 1..=60000"));
    }
    let window = protocol_token(window, "automation window")?;
    let needle = wait_needle(needle, verb)?;
    let mut out = String::with_capacity(
        verb.len()
            .saturating_add(2)
            .saturating_add(window.len())
            .saturating_add("60000".len())
            .saturating_add(needle.len()),
    );
    push_ascii_uppercase(&mut out, verb);
    let _ = write!(out, " {window} {parsed} {needle}");
    Ok(out)
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

fn save_session_json_file_text(pretty: &str) -> String {
    let mut out = String::with_capacity(pretty.len() + 1);
    out.push_str(pretty);
    out.push('\n');
    out
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
        std::fs::write(path_arg, save_session_json_file_text(&pretty))?;
    }
    Ok(())
}

fn read_json_arg(path_arg: &str) -> Result<String> {
    use std::io::Read as _;
    let mut input = String::new();
    if path_arg == "-" {
        std::io::stdin().read_to_string(&mut input)?;
    } else if std::path::Path::new(path_arg).exists() {
        input = std::fs::read_to_string(path_arg)?;
    } else {
        input = path_arg.to_string();
    }
    Ok(input)
}

fn restore_session_cmd(path_arg: &str) -> Result<()> {
    use kittui_cli::daemon::{client_request, default_socket_path};
    let input = read_json_arg(path_arg)?;
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
    let compact = serde_json::to_string(&value)?;
    let mut out = String::with_capacity("RESTORE_SESSION_JSON ".len() + compact.len());
    out.push_str("RESTORE_SESSION_JSON ");
    out.push_str(&compact);
    Ok(out)
}

fn semantic_publish_cmd(window: &str, json_arg: &str) -> Result<()> {
    use kittui_cli::daemon::{client_request, default_socket_path};
    let input = read_json_arg(json_arg)?;
    let request = semantic_publish_request(window, &input)?;
    let path = default_socket_path();
    let reply = client_request(&path, &request).map_err(|e| {
        anyhow!(
            "could not publish semantic snapshot on {}: {e}",
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

#[derive(Clone, Copy, Debug)]
struct LocalCommandEntry {
    command: &'static str,
    category: &'static str,
    description: &'static str,
}

fn local_command_entries() -> &'static [LocalCommandEntry] {
    &[
        LocalCommandEntry {
            command: "start",
            category: "lifecycle",
            description: "explicit start alias for the same default foreground session",
        },
        LocalCommandEntry {
            command: "stop",
            category: "lifecycle",
            description: "stop a socket daemon (alias for --kill)",
        },
        LocalCommandEntry {
            command: "quickstart",
            category: "help",
            description: "first-run daily-driver checklist",
        },
        LocalCommandEntry {
            command: "quickstart-scene-json",
            category: "help",
            description: "first-run checklist kittui scene",
        },
        LocalCommandEntry {
            command: "quickstart-kitty",
            category: "help",
            description: "first-run checklist kitty graphics",
        },
        LocalCommandEntry {
            command: "cheat",
            category: "help",
            description: "compact daily reference",
        },
        LocalCommandEntry {
            command: "cheat-scene-json",
            category: "help",
            description: "compact daily reference kittui scene",
        },
        LocalCommandEntry {
            command: "cheat-kitty",
            category: "help",
            description: "compact daily reference kitty graphics",
        },
        LocalCommandEntry {
            command: "log path",
            category: "diagnostics",
            description: "print debug log path",
        },
        LocalCommandEntry {
            command: "log tail [-f]",
            category: "diagnostics",
            description: "tail debug log",
        },
        LocalCommandEntry {
            command: "examples",
            category: "help",
            description: "copy-paste workflows",
        },
        LocalCommandEntry {
            command: "examples-scene-json",
            category: "help",
            description: "copy-paste workflows kittui scene",
        },
        LocalCommandEntry {
            command: "examples-kitty",
            category: "help",
            description: "copy-paste workflows kitty graphics",
        },
        LocalCommandEntry {
            command: "commands",
            category: "help",
            description: "grouped local command catalog",
        },
        LocalCommandEntry {
            command: "commands-json",
            category: "help",
            description: "machine-readable local command catalog",
        },
        LocalCommandEntry {
            command: "commands-scene-json",
            category: "help",
            description: "emit local command catalog as a kittui Scene",
        },
        LocalCommandEntry {
            command: "commands-kitty",
            category: "help",
            description: "render local command catalog with kitty graphics",
        },
        LocalCommandEntry {
            command: "architecture-json",
            category: "diagnostics",
            description: "WM architecture/separation contract JSON",
        },
        LocalCommandEntry {
            command: "architecture-scene-json",
            category: "diagnostics",
            description: "WM architecture contract kittui scene",
        },
        LocalCommandEntry {
            command: "architecture-kitty",
            category: "diagnostics",
            description: "render architecture contract with kitty graphics",
        },
        LocalCommandEntry {
            command: "showcase-scene-json",
            category: "diagnostics",
            description: "representative graphical WM scene artifact",
        },
        LocalCommandEntry {
            command: "showcase-metrics-json",
            category: "diagnostics",
            description: "scene/layer/pixel metrics for the showcase artifact",
        },
        LocalCommandEntry {
            command: "showcase-composition-json",
            category: "diagnostics",
            description: "ordered app/chrome/overlay composition graph",
        },
        LocalCommandEntry {
            command: "tui-smoke-json",
            category: "diagnostics",
            description: "terminal/TUI conformance smoke matrix",
        },
        LocalCommandEntry {
            command: "native-surfaces",
            category: "diagnostics",
            description: "first-party SDK/kitty-native surface coverage",
        },
        LocalCommandEntry {
            command: "native-surfaces-json",
            category: "diagnostics",
            description: "first-party SDK/kitty-native surface coverage JSON",
        },
        LocalCommandEntry {
            command: "native-surfaces-scene-json",
            category: "diagnostics",
            description: "first-party native surface coverage kittui scene",
        },
        LocalCommandEntry {
            command: "native-surfaces-kitty",
            category: "diagnostics",
            description: "first-party native surface coverage kitty graphics",
        },
        LocalCommandEntry {
            command: "completions SHELL",
            category: "help",
            description: "shell completions for bash, zsh, or fish",
        },
        LocalCommandEntry {
            command: "update [--status|--check]",
            category: "lifecycle",
            description: "self-update from GitHub release assets",
        },
        LocalCommandEntry {
            command: "mcp",
            category: "lifecycle",
            description: "expose shared update tools over MCP stdio",
        },
        LocalCommandEntry {
            command: "help <topic>",
            category: "help",
            description: "focused topic help",
        },
        LocalCommandEntry {
            command: "help topics",
            category: "help",
            description: "list focused help topics",
        },
        LocalCommandEntry {
            command: "help completions",
            category: "help",
            description: "shell completion setup help",
        },
        LocalCommandEntry {
            command: "help-scene-json [topic]",
            category: "help",
            description: "focused topic help kittui scene",
        },
        LocalCommandEntry {
            command: "help-kitty [topic]",
            category: "help",
            description: "render focused topic help with kitty graphics",
        },
        LocalCommandEntry {
            command: "shortcuts",
            category: "help",
            description: "interactive key chord list",
        },
        LocalCommandEntry {
            command: "shortcuts-json",
            category: "help",
            description: "interactive key chord list JSON",
        },
        LocalCommandEntry {
            command: "shortcuts-scene-json",
            category: "help",
            description: "interactive key chord list kittui scene",
        },
        LocalCommandEntry {
            command: "shortcuts-kitty",
            category: "help",
            description: "render shortcut list with kitty graphics",
        },
        LocalCommandEntry {
            command: "info",
            category: "inspect",
            description: "friendly running-WM overview",
        },
        LocalCommandEntry {
            command: "status",
            category: "inspect",
            description: "daemon status",
        },
        LocalCommandEntry {
            command: "status-scene-json",
            category: "inspect",
            description: "daemon status kittui scene",
        },
        LocalCommandEntry {
            command: "status-kitty",
            category: "inspect",
            description: "render daemon status with kitty graphics",
        },
        LocalCommandEntry {
            command: "chrome-scene-json",
            category: "inspect",
            description: "chrome reservation kittui scene",
        },
        LocalCommandEntry {
            command: "chrome-kitty",
            category: "inspect",
            description: "chrome reservation kitty graphics",
        },
        LocalCommandEntry {
            command: "panes",
            category: "inspect",
            description: "human-readable pane list",
        },
        LocalCommandEntry {
            command: "panes-json",
            category: "inspect",
            description: "structured pane list",
        },
        LocalCommandEntry {
            command: "events [ms]",
            category: "inspect",
            description: "bounded event stream",
        },
        LocalCommandEntry {
            command: "spawn CMD [ARGS...]",
            category: "action",
            description: "spawn a terminal pane",
        },
        LocalCommandEntry {
            command: "split [WINDOW] columns|rows|grid CMD [ARGS...]",
            category: "action",
            description: "spawn next to a target pane and set split axis",
        },
        LocalCommandEntry {
            command: "read [WINDOW]",
            category: "action",
            description: "read pane text",
        },
        LocalCommandEntry {
            command: "read-json [WINDOW]",
            category: "action",
            description: "read pane text JSON",
        },
        LocalCommandEntry {
            command: "type [WINDOW] TEXT",
            category: "action",
            description: "send text",
        },
        LocalCommandEntry {
            command: "line [WINDOW] TEXT",
            category: "action",
            description: "send line",
        },
        LocalCommandEntry {
            command: "paste [WINDOW] TEXT",
            category: "action",
            description: "paste text",
        },
        LocalCommandEntry {
            command: "key [WINDOW] KEY",
            category: "action",
            description: "send key",
        },
        LocalCommandEntry {
            command: "wait [WINDOW] TEXT",
            category: "action",
            description: "wait for output",
        },
        LocalCommandEntry {
            command: "focus WINDOW",
            category: "panes",
            description: "focus a pane",
        },
        LocalCommandEntry {
            command: "close [WINDOW]",
            category: "panes",
            description: "close a pane",
        },
        LocalCommandEntry {
            command: "layout columns|rows|grid",
            category: "panes",
            description: "change layout axis",
        },
        LocalCommandEntry {
            command: "move [WINDOW] DIR",
            category: "panes",
            description: "move pane",
        },
        LocalCommandEntry {
            command: "nudge [WINDOW] DX DY",
            category: "panes",
            description: "nudge floating pane",
        },
        LocalCommandEntry {
            command: "reset-position [WINDOW]",
            category: "panes",
            description: "reset floating pane position",
        },
        LocalCommandEntry {
            command: "reset-positions",
            category: "panes",
            description: "reset all floating pane positions",
        },
        LocalCommandEntry {
            command: "resize [WINDOW] N",
            category: "panes",
            description: "resize pane weight",
        },
        LocalCommandEntry {
            command: "balance",
            category: "panes",
            description: "equalize pane weights",
        },
        LocalCommandEntry {
            command: "reset-weights",
            category: "panes",
            description: "reset pane weights",
        },
        LocalCommandEntry {
            command: "rename WINDOW TITLE",
            category: "panes",
            description: "set pane title",
        },
        LocalCommandEntry {
            command: "apps",
            category: "apps",
            description: "list launch candidates",
        },
        LocalCommandEntry {
            command: "remote HOST",
            category: "remote",
            description: "friendly alias for remote doctor",
        },
        LocalCommandEntry {
            command: "remote HOST help",
            category: "remote",
            description: "host-specific SSH quick reference",
        },
        LocalCommandEntry {
            command: "remote HOST status",
            category: "remote",
            description: "check remote kittwm availability",
        },
        LocalCommandEntry {
            command: "remote HOST x11",
            category: "remote",
            description: "check trusted X11 forwarding for remote app launch",
        },
        LocalCommandEntry {
            command: "remote HOST graphical",
            category: "remote",
            description: "alias for remote HOST x11",
        },
        LocalCommandEntry {
            command: "remote HOST gui",
            category: "remote",
            description: "alias for remote HOST x11",
        },
        LocalCommandEntry {
            command: "remote HOST wayland",
            category: "remote",
            description: "alias for remote HOST graphical",
        },
        LocalCommandEntry {
            command: "remote HOST forwarding",
            category: "remote",
            description: "alias for remote HOST x11",
        },
        LocalCommandEntry {
            command: "remote HOST forward",
            category: "remote",
            description: "short alias for remote HOST forwarding",
        },
        LocalCommandEntry {
            command: "remote HOST kittwm",
            category: "remote",
            description: "open remote kittwm in a pooled SSH terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST desktop",
            category: "remote",
            description: "alias for remote HOST kittwm",
        },
        LocalCommandEntry {
            command: "remote HOST wm",
            category: "remote",
            description: "short alias for remote HOST kittwm",
        },
        LocalCommandEntry {
            command: "remote HOST list",
            category: "remote",
            description: "list remote app candidates",
        },
        LocalCommandEntry {
            command: "remote HOST list apps QUERY",
            category: "remote",
            description: "list remote app matches with a natural alias",
        },
        LocalCommandEntry {
            command: "remote HOST list windows",
            category: "remote",
            description: "list remote windows through pooled SSH",
        },
        LocalCommandEntry {
            command: "remote HOST list windows --json",
            category: "remote",
            description: "list remote windows as JSON",
        },
        LocalCommandEntry {
            command: "remote HOST list windows --fallback",
            category: "remote",
            description: "force pooled-SSH fallback window listing",
        },
        LocalCommandEntry {
            command: "remote HOST list win",
            category: "remote",
            description: "short alias for remote window listing",
        },
        LocalCommandEntry {
            command: "remote HOST win QUERY",
            category: "remote",
            description: "short alias for remote window listing",
        },
        LocalCommandEntry {
            command: "remote HOST list displays",
            category: "remote",
            description: "list remote displays through pooled SSH",
        },
        LocalCommandEntry {
            command: "remote HOST list displays --fallback",
            category: "remote",
            description: "force pooled-SSH fallback display listing",
        },
        LocalCommandEntry {
            command: "remote HOST list monitors",
            category: "remote",
            description: "alias for remote display listing",
        },
        LocalCommandEntry {
            command: "remote HOST monitors QUERY",
            category: "remote",
            description: "alias for remote display listing",
        },
        LocalCommandEntry {
            command: "remote HOST list screens",
            category: "remote",
            description: "alias for remote display listing",
        },
        LocalCommandEntry {
            command: "remote HOST screens QUERY",
            category: "remote",
            description: "alias for remote display listing",
        },
        LocalCommandEntry {
            command: "remote HOST apps QUERY",
            category: "remote",
            description: "list remote app matches with a positional query",
        },
        LocalCommandEntry {
            command: "remote HOST apps QUERY --json",
            category: "remote",
            description: "list remote app matches as JSON",
        },
        LocalCommandEntry {
            command: "remote HOST apps QUERY --fallback",
            category: "remote",
            description: "force pooled-SSH fallback app discovery",
        },
        LocalCommandEntry {
            command: "remote HOST fallback apps QUERY",
            category: "remote",
            description: "front-door alias for fallback app discovery",
        },
        LocalCommandEntry {
            command: "remote HOST fallback launch QUERY",
            category: "remote",
            description: "front-door alias for fallback app launch",
        },
        LocalCommandEntry {
            command: "remote HOST fallback open QUERY",
            category: "remote",
            description: "natural alias for fallback app launch",
        },
        LocalCommandEntry {
            command: "remote HOST fallback run QUERY",
            category: "remote",
            description: "natural alias for fallback app launch",
        },
        LocalCommandEntry {
            command: "remote HOST fallback start QUERY",
            category: "remote",
            description: "natural alias for fallback app launch",
        },
        LocalCommandEntry {
            command: "remote HOST fallback windows QUERY",
            category: "remote",
            description: "front-door alias for fallback window listing",
        },
        LocalCommandEntry {
            command: "remote HOST fallback displays QUERY",
            category: "remote",
            description: "front-door alias for fallback display listing",
        },
        LocalCommandEntry {
            command: "remote HOST applications QUERY",
            category: "remote",
            description: "alias for remote app matches",
        },
        LocalCommandEntry {
            command: "remote HOST programs QUERY",
            category: "remote",
            description: "program-style alias for remote app matches",
        },
        LocalCommandEntry {
            command: "remote HOST software QUERY",
            category: "remote",
            description: "software-style alias for remote app matches",
        },
        LocalCommandEntry {
            command: "remote HOST app QUERY",
            category: "remote",
            description: "select the first remote app match",
        },
        LocalCommandEntry {
            command: "remote HOST app QUERY --json",
            category: "remote",
            description: "structured first remote app match",
        },
        LocalCommandEntry {
            command: "remote HOST application QUERY --json",
            category: "remote",
            description: "structured alias for first remote app match",
        },
        LocalCommandEntry {
            command: "remote HOST program QUERY --json",
            category: "remote",
            description: "structured program-style alias for first remote app match",
        },
        LocalCommandEntry {
            command: "remote HOST select QUERY",
            category: "remote",
            description: "alias for first remote app match",
        },
        LocalCommandEntry {
            command: "remote HOST pick QUERY --json",
            category: "remote",
            description: "structured alias for first remote app match",
        },
        LocalCommandEntry {
            command: "remote HOST launch QUERY",
            category: "remote",
            description: "shortest alias for remote app launch",
        },
        LocalCommandEntry {
            command: "remote HOST launch QUERY --fallback",
            category: "remote",
            description: "force pooled-SSH fallback app launch",
        },
        LocalCommandEntry {
            command: "remote HOST open QUERY",
            category: "remote",
            description: "natural alias for remote app launch",
        },
        LocalCommandEntry {
            command: "remote HOST run QUERY",
            category: "remote",
            description: "natural alias for remote app launch",
        },
        LocalCommandEntry {
            command: "remote HOST start QUERY",
            category: "remote",
            description: "natural alias for remote app launch",
        },
        LocalCommandEntry {
            command: "remote HOST apps QUERY --launch-first",
            category: "remote",
            description: "explicit alias for remote app launch",
        },
        LocalCommandEntry {
            command: "remote HOST launch QUERY --json",
            category: "remote",
            description: "structured remote app launch result or error",
        },
        LocalCommandEntry {
            command: "remote HOST shell",
            category: "remote",
            description: "open a pooled SSH login shell pane",
        },
        LocalCommandEntry {
            command: "remote HOST sh",
            category: "remote",
            description: "short alias for a pooled SSH login shell pane",
        },
        LocalCommandEntry {
            command: "remote HOST login",
            category: "remote",
            description: "natural alias for a pooled SSH login shell pane",
        },
        LocalCommandEntry {
            command: "remote HOST ssh",
            category: "remote",
            description: "alias for a pooled SSH login shell pane",
        },
        LocalCommandEntry {
            command: "remote HOST terminal CMD",
            category: "remote",
            description: "friendly alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST term CMD",
            category: "remote",
            description: "short alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST cmd CMD",
            category: "remote",
            description: "command-style alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST command CMD",
            category: "remote",
            description: "command-style alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST exec CMD",
            category: "remote",
            description: "exec-style alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST sh CMD",
            category: "remote",
            description: "shell-style alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST login CMD",
            category: "remote",
            description: "login-shell alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST console CMD",
            category: "remote",
            description: "natural alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "remote HOST tty CMD",
            category: "remote",
            description: "short alias for remote terminal pane",
        },
        LocalCommandEntry {
            command: "apps --remote HOST",
            category: "remote",
            description: "list remote app candidates via pooled SSH",
        },
        LocalCommandEntry {
            command: "apps --remote HOST --filter QUERY --launch-first",
            category: "remote",
            description: "launch first remote app match via pooled SSH",
        },
        LocalCommandEntry {
            command: "doctor --remote HOST",
            category: "remote",
            description: "check remote kittwm availability and forwarding hints",
        },
        LocalCommandEntry {
            command: "--list-windows --remote HOST",
            category: "remote",
            description: "list remote windows when supported",
        },
        LocalCommandEntry {
            command: "--list-displays --remote HOST",
            category: "remote",
            description: "list remote displays when supported",
        },
        LocalCommandEntry {
            command: "kittwm-terminal --remote HOST",
            category: "remote",
            description: "open a local pane running a pooled SSH terminal",
        },
        LocalCommandEntry {
            command: "apps-scene-json",
            category: "apps",
            description: "launcher candidates kittui scene",
        },
        LocalCommandEntry {
            command: "apps-kitty",
            category: "apps",
            description: "launcher candidates kitty graphics",
        },
        LocalCommandEntry {
            command: "launcher",
            category: "apps",
            description: "launcher preview",
        },
        LocalCommandEntry {
            command: "launcher-scene-json",
            category: "apps",
            description: "launcher preview kittui scene",
        },
        LocalCommandEntry {
            command: "launcher-kitty",
            category: "apps",
            description: "launcher preview kitty graphics",
        },
        LocalCommandEntry {
            command: "launch -- CMD",
            category: "apps",
            description: "backend launcher",
        },
        LocalCommandEntry {
            command: "--save-session PATH",
            category: "session",
            description: "save session manifest",
        },
        LocalCommandEntry {
            command: "--restore-session PATH",
            category: "session",
            description: "restore session manifest",
        },
        LocalCommandEntry {
            command: "session-scene-json",
            category: "session",
            description: "session manifest kittui scene",
        },
        LocalCommandEntry {
            command: "session-kitty",
            category: "session",
            description: "session manifest kitty graphics",
        },
        LocalCommandEntry {
            command: "doctor",
            category: "diagnostics",
            description: "diagnostics and readiness hints",
        },
        LocalCommandEntry {
            command: "config",
            category: "diagnostics",
            description: "config and keymap inspection",
        },
        LocalCommandEntry {
            command: "config-scene-json",
            category: "diagnostics",
            description: "config readiness kittui scene",
        },
        LocalCommandEntry {
            command: "config-kitty",
            category: "diagnostics",
            description: "config readiness kitty graphics",
        },
        LocalCommandEntry {
            command: "keymap",
            category: "diagnostics",
            description: "resolved keybindings",
        },
        LocalCommandEntry {
            command: "keymap-scene-json",
            category: "diagnostics",
            description: "resolved keybindings kittui scene",
        },
        LocalCommandEntry {
            command: "keymap-kitty",
            category: "diagnostics",
            description: "resolved keybindings kitty graphics",
        },
    ]
}

fn commands_text() -> String {
    let entries = local_command_entries();
    let mut out = String::with_capacity(entries.len().saturating_mul(48).saturating_add(96));
    out.push_str("kittwm commands — local CLI catalog\n");
    let mut categories = Vec::new();
    for entry in entries {
        if !categories.contains(&entry.category) {
            categories.push(entry.category);
        }
    }
    for category in categories {
        out.push('\n');
        for ch in category.chars() {
            out.push(ch.to_ascii_uppercase());
        }
        out.push('\n');
        for entry in entries.iter().filter(|entry| entry.category == category) {
            let _ = writeln!(out, "  {:28} {}", entry.command, entry.description);
        }
    }
    out.push_str(
        "\nFor socket verbs from a running WM: kittwm --help-json\nDaily workflows: kittwm examples | kittwm cheat | kittwm help topics\n",
    );
    out
}

fn commands_json_text() -> String {
    let commands: Vec<_> = local_command_entries()
        .iter()
        .map(|entry| {
            serde_json::json!({
                "command": entry.command,
                "category": entry.category,
                "description": entry.description,
            })
        })
        .collect();
    let value = serde_json::json!({
        "schema_version": 1,
        "kind": "kittwm-local-commands",
        "commands": commands,
    });
    let mut out = value.to_string();
    out.push('\n');
    out
}

fn commands_cmd() -> Result<()> {
    print!("{}", commands_text());
    Ok(())
}

fn commands_json_cmd() -> Result<()> {
    print!("{}", commands_json_text());
    Ok(())
}

fn commands_graphical_cmd(kitty: bool) -> Result<()> {
    let scene = commands_scene();
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn commands_scene() -> Scene {
    let cols = info_scene_cols();
    let entries = local_command_entries();
    let rows = commands_scene_rows(entries.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let mut by_category = std::collections::BTreeMap::<&str, usize>::new();
    for entry in entries {
        *by_category.entry(entry.category).or_default() += 1;
    }
    let summary = command_category_summary_label(&by_category);
    let mut layers = vec![
        Layer {
            label: Some(commands_backdrop_label(entries.len(), &summary)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(163, 190, 140, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-commands-heading:local-command-catalog".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, entry) in entries.iter().take(20).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(commands_scene_row_label(
                entry.category,
                entry.command,
                entry.description,
            )),
            root: Node::Rect {
                rect: commands_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(163, 190, 140, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn commands_scene_row_label(category: &str, command: &str, description: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-command-row::"
            .len()
            .saturating_add(category.len())
            .saturating_add(command.len())
            .saturating_add(description.len()),
    );
    label.push_str("kittwm-command-row:");
    label.push_str(category);
    label.push(':');
    label.push_str(command);
    label.push(':');
    label.push_str(description);
    label
}

fn commands_backdrop_label(command_count: usize, summary: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-commands-backdrop:count=:categories=".len() + 20 + summary.len(),
    );
    label.push_str("kittwm-commands-backdrop:count=");
    let _ = write!(label, "{command_count}");
    label.push_str(":categories=");
    label.push_str(summary);
    label
}

fn command_category_summary_label(by_category: &std::collections::BTreeMap<&str, usize>) -> String {
    let mut summary = String::with_capacity(by_category.len().saturating_mul(16));
    for (category, count) in by_category {
        if !summary.is_empty() {
            summary.push(',');
        }
        summary.push_str(category);
        summary.push('=');
        summary.push_str(&count.to_string());
    }
    summary
}

fn commands_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn commands_scene_rows(entry_count: usize) -> u16 {
    entry_count.saturating_add(5).clamp(8, 28) as u16
}

fn architecture_contract_json_text() -> String {
    let mut out = serde_json::to_string(&kittwm_sdk::ArchitectureContract::current())
        .expect("architecture contract serializes");
    out.push('\n');
    out
}
fn architecture_json_cmd() -> Result<()> {
    print!("{}", architecture_contract_json_text());
    Ok(())
}

fn architecture_graphical_cmd(kitty: bool) -> Result<()> {
    let contract = kittwm_sdk::ArchitectureContract::current();
    let scene = architecture_scene(&contract);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn architecture_scene(contract: &kittwm_sdk::ArchitectureContract) -> Scene {
    let cols = info_scene_cols();
    let rows = architecture_scene_rows(
        contract.layers.len(),
        contract.composition_order.len(),
        contract.first_party_native_surfaces.len(),
    );
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let mut layers = vec![
        Layer {
            label: Some(architecture_backdrop_label(
                contract.layers.len(),
                contract.composition_order.len(),
                contract.first_party_native_surfaces.len(),
                contract.schema_version,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(180, 142, 173, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(architecture_scene_heading_label(&contract.kind)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    let mut row = 2usize;
    for layer in contract.layers.iter().take(8) {
        let y = row as f32 * cell.height_px as f32;
        layers.push(Layer {
            label: Some(architecture_layer_label(
                &layer.id,
                &layer.owner,
                layer.responsibilities.len(),
                layer.must_not.len(),
                layer.native_contracts.len(),
            )),
            root: Node::Rect {
                rect: architecture_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(136, 192, 208, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
        row += 1;
    }
    for plane in contract.composition_order.iter().take(6) {
        let y = row as f32 * cell.height_px as f32;
        layers.push(Layer {
            label: Some(architecture_plane_label(&plane.plane, plane.z_index)),
            root: Node::Rect {
                rect: architecture_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(235, 203, 139, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
        row += 1;
    }
    for surface in contract.first_party_native_surfaces.iter().take(6) {
        let y = row as f32 * cell.height_px as f32;
        layers.push(Layer {
            label: Some(architecture_surface_label(
                &surface.name,
                &surface.surface_kind,
                surface.sdk_backed,
                surface.kitty_graphics_native,
                &surface.kittui_entry,
            )),
            root: Node::Rect {
                rect: architecture_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(163, 190, 140, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
        row += 1;
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn architecture_backdrop_label(
    layer_count: usize,
    plane_count: usize,
    surface_count: usize,
    schema_version: u32,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-architecture-backdrop:layers=:planes=:surfaces=:schema=".len() + 20 * 4,
    );
    label.push_str("kittwm-architecture-backdrop:layers=");
    let _ = write!(label, "{layer_count}");
    label.push_str(":planes=");
    let _ = write!(label, "{plane_count}");
    label.push_str(":surfaces=");
    let _ = write!(label, "{surface_count}");
    label.push_str(":schema=");
    let _ = write!(label, "{schema_version}");
    label
}

fn architecture_surface_label(
    name: &str,
    surface_kind: &str,
    sdk_backed: bool,
    kitty_graphics_native: bool,
    kittui_entry: &str,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-architecture-surface::kind=:sdk=:kitty=:kittui=".len()
            + name.len()
            + surface_kind.len()
            + 5
            + 5
            + kittui_entry.len(),
    );
    label.push_str("kittwm-architecture-surface:");
    label.push_str(name);
    label.push_str(":kind=");
    label.push_str(surface_kind);
    label.push_str(":sdk=");
    let _ = write!(label, "{sdk_backed}");
    label.push_str(":kitty=");
    let _ = write!(label, "{kitty_graphics_native}");
    label.push_str(":kittui=");
    label.push_str(kittui_entry);
    label
}

fn architecture_plane_label(plane: &str, z_index: i32) -> String {
    let mut label = String::with_capacity("kittwm-architecture-plane::z=".len() + plane.len() + 12);
    label.push_str("kittwm-architecture-plane:");
    label.push_str(plane);
    label.push_str(":z=");
    let _ = write!(label, "{z_index}");
    label
}

fn architecture_layer_label(
    id: &str,
    owner: &str,
    responsibilities: usize,
    must_not: usize,
    native_contracts: usize,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-architecture-layer::owner=:responsibilities=:must_not=:native_contracts="
            .len()
            .saturating_add(id.len())
            .saturating_add(owner.len())
            .saturating_add(60),
    );
    label.push_str("kittwm-architecture-layer:");
    label.push_str(id);
    label.push_str(":owner=");
    label.push_str(owner);
    label.push_str(":responsibilities=");
    let _ = write!(label, "{responsibilities}");
    label.push_str(":must_not=");
    let _ = write!(label, "{must_not}");
    label.push_str(":native_contracts=");
    let _ = write!(label, "{native_contracts}");
    label
}

fn architecture_scene_heading_label(kind: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-architecture-heading:"
            .len()
            .saturating_add(kind.len()),
    );
    label.push_str("kittwm-architecture-heading:");
    label.push_str(kind);
    label
}

fn architecture_scene_rows(layer_count: usize, plane_count: usize, surface_count: usize) -> u16 {
    let rows = layer_count
        .saturating_add(plane_count)
        .saturating_add(surface_count)
        .saturating_add(6)
        .min(u16::MAX as usize) as u16;
    rows.clamp(10, 30)
}

fn architecture_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn native_surface_row_label(
    idx: usize,
    name: &str,
    surface_kind: &str,
    ready: bool,
    sdk_backed: bool,
    kitty_graphics_native: bool,
    plane: &str,
    z_index: &str,
    kittui_entry: &str,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-native-surface-row::kind=:ready=:sdk=:kitty=:plane=:z=:kittui=".len()
            + 20
            + name.len()
            + surface_kind.len()
            + 5
            + 5
            + 5
            + plane.len()
            + z_index.len()
            + kittui_entry.len(),
    );
    label.push_str("kittwm-native-surface-row:");
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(name);
    label.push_str(":kind=");
    label.push_str(surface_kind);
    label.push_str(":ready=");
    let _ = write!(label, "{ready}");
    label.push_str(":sdk=");
    let _ = write!(label, "{sdk_backed}");
    label.push_str(":kitty=");
    let _ = write!(label, "{kitty_graphics_native}");
    label.push_str(":plane=");
    label.push_str(plane);
    label.push_str(":z=");
    label.push_str(z_index);
    label.push_str(":kittui=");
    label.push_str(kittui_entry);
    label
}

fn native_surfaces_backdrop_label(surface_count: usize, all_ready: bool) -> String {
    let mut label =
        String::with_capacity("kittwm-native-surfaces-backdrop:count=:all_ready=".len() + 20 + 5);
    label.push_str("kittwm-native-surfaces-backdrop:count=");
    let _ = write!(label, "{surface_count}");
    label.push_str(":all_ready=");
    let _ = write!(label, "{all_ready}");
    label
}

fn native_surfaces_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn native_surfaces_scene_rows(surface_count: usize) -> u16 {
    surface_count.saturating_add(5).clamp(8, 22) as u16
}

fn native_surfaces_json_text() -> String {
    let contract = kittwm_sdk::ArchitectureContract::current();
    let surfaces = contract.first_party_native_surfaces.clone();
    let all_ready = surfaces.iter().all(|surface| surface.is_native_ready());
    let mut out = serde_json::json!({
        "schema_version": contract.schema_version,
        "kind": "kittwm-native-surface-coverage",
        "all_ready": all_ready,
        "surfaces": surfaces,
    })
    .to_string();
    out.push('\n');
    out
}

fn native_surfaces_json_cmd() -> Result<()> {
    print!("{}", native_surfaces_json_text());
    Ok(())
}

fn native_surfaces_text() -> String {
    let contract = kittwm_sdk::ArchitectureContract::current();
    let mut out = String::from("kittwm native surfaces — SDK + kitty graphics coverage\n");
    out.push_str("all ready: ");
    out.push_str(if contract.all_native_surfaces_ready() {
        "yes"
    } else {
        "no"
    });
    out.push_str("\n\n");
    for surface in &contract.first_party_native_surfaces {
        let _ = writeln!(
            out,
            "  {:16} kind:{:<9} sdk:{:<38} kitty:{}",
            surface.name,
            surface.surface_kind,
            surface.sdk_entry,
            if surface.kitty_graphics_native {
                "yes"
            } else {
                "no"
            }
        );
        out.push_str("    kittui: ");
        out.push_str(&surface.kittui_entry);
        out.push('\n');
    }
    out
}

fn native_surfaces_cmd() -> Result<()> {
    print!("{}", native_surfaces_text());
    Ok(())
}

fn native_surfaces_graphical_cmd(kitty: bool) -> Result<()> {
    let contract = kittwm_sdk::ArchitectureContract::current();
    let scene = native_surfaces_scene(&contract);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn native_surfaces_scene(contract: &kittwm_sdk::ArchitectureContract) -> Scene {
    let cols = info_scene_cols();
    let surfaces = &contract.first_party_native_surfaces;
    let rows = native_surfaces_scene_rows(surfaces.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let all_ready = contract.all_native_surfaces_ready();
    let mut layers = vec![
        Layer {
            label: Some(native_surfaces_backdrop_label(surfaces.len(), all_ready)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(163, 190, 140, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-native-surfaces-heading:sdk-kittui-kitty-coverage".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, surface) in surfaces.iter().take(16).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let plane = surface.composition_plane().unwrap_or("unknown");
        let z_index = surface
            .z_index(contract)
            .map(|z| z.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        layers.push(Layer {
            label: Some(native_surface_row_label(
                idx,
                &surface.name,
                &surface.surface_kind,
                surface.is_native_ready(),
                surface.sdk_backed,
                surface.kitty_graphics_native,
                plane,
                &z_index,
                &surface.kittui_entry,
            )),
            root: Node::Rect {
                rect: native_surfaces_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: if surface.is_native_ready() {
                        Rgba::rgba(163, 190, 140, 255)
                    } else {
                        Rgba::rgba(191, 97, 106, 255)
                    },
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

static COMPLETION_WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();

fn completion_words() -> &'static [&'static str] {
    COMPLETION_WORDS.get_or_init(|| {
        let mut words = local_command_entries()
            .iter()
            .filter_map(|entry| entry.command.split_whitespace().next())
            .collect::<Vec<_>>();
        words.extend([
            "--help",
            "--socket",
            "--display",
            "--remote",
            "--host",
            "--filter",
            "--limit",
            "--first",
            "--launch-first",
            "--x11",
            "--gui",
            "--graphical",
            "--wayland",
            "--forward",
            "help",
            "doctor",
            "status",
            "check",
            "x11",
            "gui",
            "graphical",
            "wayland",
            "forwarding",
            "forward",
            "list",
            "apps",
            "applications",
            "programs",
            "software",
            "app",
            "application",
            "program",
            "select",
            "pick",
            "launch",
            "open",
            "run",
            "start",
            "fallback",
            "kittwm",
            "desktop",
            "wm",
            "windows",
            "window",
            "wins",
            "win",
            "displays",
            "monitors",
            "monitor",
            "screens",
            "screen",
            "terminal",
            "term",
            "cmd",
            "command",
            "exec",
            "shell",
            "sh",
            "login",
            "ssh",
            "console",
            "tty",
            "--status-json",
            "--help-json",
            "--panes",
            "--panes-json",
            "--session-json",
            "--events",
            "--events-ms",
            "--shortcuts",
            "--shortcuts-json",
            "--read-text-json",
            "--nudge-pane",
            "--reset-pane-offset",
            "--reset-all-pane-offsets",
            "--reset-pane-weights",
            "--wait-output-json-ms",
        ]);
        words.sort_unstable();
        words.dedup();
        words
    })
}

fn missing_completion_shell_error() -> anyhow::Error {
    anyhow!(
        "kittwm completions requires a shell: bash, zsh, or fish\ntry: kittwm completions bash\nhelp: kittwm help completions"
    )
}

fn extra_completion_shell_error(extra: &str) -> anyhow::Error {
    anyhow!(
        "kittwm completions accepts one shell, got {extra:?}; expected bash, zsh, or fish\ntry: kittwm completions bash\nhelp: kittwm help completions"
    )
}

fn completions_text(shell: &str) -> Result<String> {
    match shell {
        "bash" => Ok(bash_completions_text()),
        "zsh" => Ok(zsh_completions_text()),
        "fish" => Ok(fish_completions_text()),
        other => Err(anyhow!(
            "unsupported completion shell {other:?}; expected bash, zsh, or fish\ntry: kittwm completions bash\nhelp: kittwm help completions"
        )),
    }
}

fn push_completion_words(out: &mut String) {
    for (idx, word) in completion_words().iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        out.push_str(word);
    }
}

fn bash_completions_text() -> String {
    let mut out = String::with_capacity(512);
    out.push_str(
        "_kittwm() {\n  local cur=\"${COMP_WORDS[COMP_CWORD]}\"\n  COMPREPLY=( $(compgen -W '",
    );
    push_completion_words(&mut out);
    out.push_str("' -- \"$cur\") )\n}\ncomplete -F _kittwm kittwm\n");
    out
}

fn zsh_completions_text() -> String {
    let mut out = String::with_capacity(512);
    out.push_str("#compdef kittwm\n_arguments '1:command:(");
    push_completion_words(&mut out);
    out.push_str(")' '*::arg:->args'\n");
    out
}

fn fish_completions_text() -> String {
    const PREFIX: &str = "complete -c kittwm -f -a '";
    const SUFFIX: &str = "'\n";
    let words = completion_words();
    let capacity = words
        .iter()
        .map(|word| PREFIX.len() + word.len() + SUFFIX.len())
        .sum();
    let mut out = String::with_capacity(capacity);
    for word in words {
        out.push_str(PREFIX);
        out.push_str(&word);
        out.push_str(SUFFIX);
    }
    out
}

fn completions_cmd(shell: &str) -> Result<()> {
    print!("{}", completions_text(shell)?);
    Ok(())
}

fn quickstart_text() -> &'static str {
    r#"kittwm quickstart — daily-driver path

1. Start the WM
   kittwm
   # equivalent: kittwm start

2. Inside kittwm
   C-a Enter           open a terminal pane
   C-a t               toggle floating mode
   C-a f               toggle fullscreen
   C-a e               toggle current split vertical/horizontal
   C-a g               open launcher
   C-a ?               show the shortcut overlay
   C-a Tab             focus next pane
   C-a x               close focused pane
   Ctrl-]              exit kittwm

3. From another shell, inspect the running WM
   kittwm info
   kittwm panes
   kittwm events 1000

4. Do common pane work without long flags
   kittwm spawn htop
   kittwm read-json focused
   kittwm type focused 'echo hello'
   kittwm line focused 'cargo test'
   kittwm paste focused 'multi-line text'
   kittwm key focused ctrl-c
   kittwm --paste-bytes-b64 focused cGFzdGUgbWU=
   kittwm wait focused 'finished'

5. Manage panes
   kittwm focus native-2
   kittwm close focused
   kittwm layout rows
   kittwm balance

6. Save and restore a working layout
   kittwm --save-session session.json
   kittwm --restore-session session.json

7. Work across SSH hosts
   kittwm remote buildbox
   kittwm remote buildbox list apps firefox
   kittwm remote buildbox launch firefox
   kittwm remote buildbox list windows firefox
   kittwm remote buildbox list displays retina
   kittwm remote buildbox terminal htop

8. Try first-party helpers when you need richer views
   kittwm-launch --browser https://example.com
   kittwm-terminal --events-ms 1000
   kittwm-top --json
   kittwm-bar --reserve --kitty
   kittwm-browser --semantic-snapshot https://example.com

More help
   kittwm --help
   kittwm examples
   kittwm cheat
   kittwm help topics
   kittwm help panes
   kittwm help input
   kittwm help completions
   kittwm completions bash >> ~/.bashrc
   kittwm completions zsh >> ~/.zshrc
   mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish
   kittwm shortcuts
"#
}

fn quickstart_cmd(scene_json: bool, kitty: bool) -> Result<()> {
    daily_help_cmd("quickstart", quickstart_text(), scene_json, kitty)
}

fn examples_text() -> &'static str {
    r#"kittwm examples — copy/paste workflows

START
  kittwm
  KITTWM_WORKSPACE=dev kittwm
  KITTWM_STARTUP_TERMINAL=1 kittwm

SHELL SETUP
  kittwm completions bash >> ~/.bashrc
  kittwm completions zsh >> ~/.zshrc
  mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish

INSPECT
  kittwm info
  kittwm status
  kittwm panes
  kittwm panes-json
  kittwm events 1000
  kittwm --chrome-json

SPAWN AND TYPE
  kittwm spawn htop
  kittwm split columns htop
  kittwm split native-1 rows bash -lc 'cargo test'
  kittwm spawn bash -lc 'cargo test'
  kittwm type focused 'echo hello'
  kittwm line focused 'cargo test -p kittui-cli'
  kittwm paste focused 'multi-line text'
  kittwm key focused ctrl-c

BYTES AND PASTE
  kittwm --send-bytes-b64 focused aGkKAA==
  kittwm --paste-bytes-b64 focused cGFzdGUgbWU=
  kittwm --send-file focused ./input.bin
  kittwm --paste-file focused -

READ AND WAIT
  kittwm read focused
  kittwm read-json focused
  kittwm --read-scrollback-json focused
  kittwm wait focused 'Finished'
  kittwm --wait-output-json-ms 10000 focused 'build finished'

CONTROL PANES
  kittwm focus native-2
  kittwm close
  kittwm layout rows
  kittwm layout grid
  kittwm move last
  kittwm resize focused +2
  kittwm balance
  kittwm rename focused editor

SESSION
  kittwm --save-session session.json
  kittwm --restore-session session.json

FIRST-PARTY HELPERS
  kittwm-launch --browser https://example.com
  kittwm-terminal --events-ms 1000
  kittwm-top --json
  kittwm-bar --reserve --kitty
  kittwm-browser --semantic-snapshot https://example.com

SSH / REMOTE HOSTS
  kittwm remote buildbox
  kittwm remote buildbox list apps firefox
  kittwm remote buildbox launch firefox
  kittwm remote buildbox list windows firefox
  kittwm remote buildbox list displays retina
  kittwm remote buildbox terminal htop

HELP
  kittwm quickstart
  kittwm help topics
  kittwm help panes
  kittwm shortcuts
  kittwm --help-json
"#
}

fn examples_cmd(scene_json: bool, kitty: bool) -> Result<()> {
    daily_help_cmd("examples", examples_text(), scene_json, kitty)
}

fn cheat_text() -> &'static str {
    r#"kittwm cheat — daily keys + commands

IN SESSION
  C-a Enter    terminal     C-a g launcher   C-a ? help
  C-a t float  C-a f full   C-a e split-toggle
  C-a % split columns       C-a - split rows  C-a x close
  C-a Tab focus next        C-a b balance     C-a +/- resize

INSPECT
  kittwm info               kittwm panes      kittwm events 1000
  kittwm --chrome-json      kittwm shortcuts  kittwm --help-json

PANE CONTROL
  kittwm spawn htop         kittwm focus native-2
  kittwm close              kittwm layout rows
  kittwm move last          kittwm nudge focused 3 -2
  kittwm reset-position     kittwm resize focused +2
  kittwm balance            kittwm rename focused editor

AUTOMATION
  kittwm type focused 'echo hi'
  kittwm line focused 'cargo test'
  kittwm paste focused 'multi-line text'
  kittwm read-json focused
  kittwm wait focused 'Finished'

HELPERS
  kittwm-launch --browser URL   kittwm-terminal --events-ms 1000
  kittwm-top --json             kittwm-bar --reserve --kitty
  kittwm-browser --semantic-snapshot URL

SSH
  kittwm doctor --remote HOST   kittwm apps --remote HOST
  kittwm --list-windows --remote HOST
  kittwm remote HOST shell       kittwm remote HOST terminal htop

MORE
  kittwm quickstart         kittwm examples    kittwm help panes
  kittwm help completions   kittwm completions bash >> ~/.bashrc
  kittwm completions zsh >> ~/.zshrc
  mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish
"#
}

fn cheat_cmd(scene_json: bool, kitty: bool) -> Result<()> {
    daily_help_cmd("cheat", cheat_text(), scene_json, kitty)
}

fn daily_help_cmd(kind: &str, text: &str, scene_json: bool, kitty: bool) -> Result<()> {
    if scene_json || kitty {
        let scene = daily_help_scene(kind, text);
        return print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration);
    }
    print!("{text}");
    Ok(())
}

fn daily_help_scene(kind: &str, text: &str) -> Scene {
    let cols = info_scene_cols();
    let content_lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let rows = daily_help_scene_rows(content_lines.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let heading = content_lines.first().copied().unwrap_or(kind);
    let kind_label = truncate(kind, 32);
    let heading_label = truncate(heading, 64);
    let command_count = content_lines
        .iter()
        .filter(|line| line.trim_start().starts_with("kittwm "))
        .count();
    let mut layers = vec![
        Layer {
            label: Some(daily_help_backdrop_label(
                &kind_label,
                content_lines.len(),
                command_count,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(daily_help_heading_label(&kind_label, &heading_label)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, line) in content_lines.iter().skip(1).take(20).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let trimmed = line.trim();
        let row_label = truncate(trimmed, 80);
        layers.push(Layer {
            label: Some(daily_help_row_label(&kind_label, idx, &row_label)),
            root: Node::Rect {
                rect: daily_help_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: if trimmed.starts_with("kittwm ") {
                        Rgba::rgba(163, 190, 140, 255)
                    } else {
                        Rgba::rgba(136, 192, 208, 255)
                    },
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn info_cmd(scene_json: bool, kitty: bool) -> Result<()> {
    let (path, status, chrome, panes) = load_info_snapshot()?;
    if scene_json || kitty {
        let scene = info_scene(&path, &status, &chrome, &panes);
        if scene_json {
            println!("{}", serde_json::to_string(&scene)?);
        } else {
            let runtime = Runtime::builder()
                .terminal(TerminalInfo::detect())
                .build()?;
            let options =
                kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Decoration);
            let placement = runtime.place_at_with_options(&scene, scene.footprint, &options)?;
            print!("{}", placement.to_bytes());
        }
    } else {
        print!("{}", format_info_output(&path, &status, &chrome, &panes));
    }
    Ok(())
}

fn load_info_snapshot() -> Result<(
    std::path::PathBuf,
    serde_json::Value,
    serde_json::Value,
    serde_json::Value,
)> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let status = match client_request_multi(&path, "STATUS_JSON") {
        Ok(reply) => reply,
        Err(err) => {
            println!(
                "kittwm: no running WM reachable at {}\n\nStart one with:\n  kittwm\n\nThen inspect it with:\n  kittwm info\n  kittwm --panes\n",
                path.display()
            );
            return Err(anyhow!("connect {}: {err}", path.display()));
        }
    };
    let chrome = client_request_multi(&path, "CHROME_JSON").unwrap_or_else(|_| "{}".to_string());
    let panes = client_request_multi(&path, "PANES_JSON").unwrap_or_else(|_| "{}".to_string());
    Ok((
        path,
        serde_json::from_str(&status)?,
        serde_json::from_str(&chrome)?,
        serde_json::from_str(&panes)?,
    ))
}

fn kittwm_scene_workspace_from(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|workspace| !workspace.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn format_info_output(
    socket: &std::path::Path,
    status: &serde_json::Value,
    chrome: &serde_json::Value,
    panes: &serde_json::Value,
) -> String {
    use std::fmt::Write as _;

    let workspace =
        kittwm_scene_workspace_from(chrome.get("workspace").or_else(|| status.get("workspace")));
    let top_bar_rows = chrome
        .get("top_bar_rows")
        .and_then(serde_json::Value::as_u64)
        .map(|rows| rows.to_string())
        .unwrap_or_else(|| "-".to_string());
    let tilable_rows = chrome
        .get("tilable_rows")
        .and_then(serde_json::Value::as_u64)
        .map(|rows| rows.to_string())
        .unwrap_or_else(|| "-".to_string());
    let pane_count = status
        .get("panes")
        .or_else(|| panes.get("panes"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let focus = status
        .get("focus")
        .or_else(|| panes.get("focus"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let layout = status
        .get("layout")
        .or_else(|| panes.get("layout"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let pane_details = panes
        .get("panes_detail")
        .or_else(|| status.get("panes_detail"))
        .and_then(serde_json::Value::as_array);

    let socket = socket.display().to_string();
    let mut out = String::with_capacity(
        "kittwm info\n  socket: \n  workspace: \n  chrome: top_bar_rows= tilable_rows=\n  panes:  focus= layout=\n".len()
            + socket.len()
            + workspace.len()
            + focus.len()
            + layout.len()
            + 48,
    );
    out.push_str("kittwm info\n  socket: ");
    out.push_str(&socket);
    out.push_str("\n  workspace: ");
    out.push_str(&workspace);
    out.push_str("\n  chrome: top_bar_rows=");
    let _ = write!(out, "{top_bar_rows}");
    out.push_str(" tilable_rows=");
    let _ = write!(out, "{tilable_rows}");
    out.push_str("\n  panes: ");
    let _ = write!(out, "{pane_count}");
    out.push_str(" focus=");
    out.push_str(focus);
    out.push_str(" layout=");
    out.push_str(layout);
    out.push('\n');
    if let Some(details) = pane_details {
        if !details.is_empty() {
            out.push_str("\nPanes\n");
            for pane in details {
                let window = pane
                    .get("window")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let title = pane
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let focused = if pane
                    .get("focused")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    "*"
                } else {
                    " "
                };
                out.push_str("  ");
                out.push_str(focused);
                out.push(' ');
                out.push_str(window);
                out.push_str("  ");
                out.push_str(title);
                if let (Some(x), Some(y), Some(cols), Some(rows)) = (
                    pane.get("x").and_then(serde_json::Value::as_u64),
                    pane.get("y").and_then(serde_json::Value::as_u64),
                    pane.get("cols").and_then(serde_json::Value::as_u64),
                    pane.get("rows").and_then(serde_json::Value::as_u64),
                ) {
                    let _ = write!(out, " {x},{y} {cols}x{rows}");
                }
                out.push('\n');
            }
        }
    }
    out.push_str(
        "\nNext\n  kittwm help panes\n  kittwm --read-text-json focused\n  kittwm --spawn-pty 'htop'\n",
    );
    out
}

fn info_scene(
    socket: &std::path::Path,
    status: &serde_json::Value,
    chrome: &serde_json::Value,
    panes: &serde_json::Value,
) -> Scene {
    let cols = info_scene_cols();
    let pane_count = status
        .get("panes")
        .or_else(|| panes.get("panes"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let rows = info_scene_rows(pane_count);
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let workspace =
        kittwm_scene_workspace_from(chrome.get("workspace").or_else(|| status.get("workspace")));
    let focus = status
        .get("focus")
        .or_else(|| panes.get("focus"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let layout = status
        .get("layout")
        .or_else(|| panes.get("layout"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let top_bar_rows = chrome
        .get("top_bar_rows")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let tilable_rows = chrome
        .get("tilable_rows")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let workspace_label = truncate(&workspace, 32);
    let socket_label = truncate(&socket.display().to_string(), 48);
    let focus_label = truncate(focus, 32);
    let layout_label = truncate(layout, 32);
    let mut layers = vec![
        Layer {
            label: Some(info_backdrop_label(&workspace_label, pane_count)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(info_heading_label(
                &socket_label,
                &focus_label,
                &layout_label,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
        Layer {
            label: Some(info_chrome_label(top_bar_rows, tilable_rows)),
            root: Node::Rect {
                rect: info_indicator_rect(width, cell.height_px as f32 * 2.0),
                fill: Paint::Solid {
                    color: Rgba::rgba(163, 190, 140, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        },
    ];
    if let Some(details) = panes
        .get("panes_detail")
        .or_else(|| status.get("panes_detail"))
        .and_then(serde_json::Value::as_array)
    {
        for (idx, pane) in details.iter().take(12).enumerate() {
            let window = pane
                .get("window")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-");
            let title = pane
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-");
            let focused = pane
                .get("focused")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let window_label = truncate(window, 32);
            let title_label = truncate(title, 48);
            let y = (idx as f32 + 3.0) * cell.height_px as f32;
            layers.push(Layer {
                label: Some(info_pane_label(&window_label, focused, &title_label)),
                root: Node::Rect {
                    rect: info_indicator_rect(width, y),
                    fill: Paint::Solid {
                        color: if focused {
                            Rgba::rgba(235, 203, 139, 255)
                        } else {
                            Rgba::rgba(136, 192, 208, 255)
                        },
                    },
                    stroke: None,
                    corners: Corners::uniform(1.0),
                },
            });
        }
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn info_heading_label(socket_label: &str, focus_label: &str, layout_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-info-heading:socket=:focus=:layout="
            .len()
            .saturating_add(socket_label.len())
            .saturating_add(focus_label.len())
            .saturating_add(layout_label.len()),
    );
    label.push_str("kittwm-info-heading:socket=");
    label.push_str(socket_label);
    label.push_str(":focus=");
    label.push_str(focus_label);
    label.push_str(":layout=");
    label.push_str(layout_label);
    label
}

fn info_pane_label(window_label: &str, focused: bool, title_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-info-pane::focused=:title="
            .len()
            .saturating_add(window_label.len())
            .saturating_add(title_label.len())
            .saturating_add(5),
    );
    label.push_str("kittwm-info-pane:");
    label.push_str(window_label);
    label.push_str(":focused=");
    let _ = write!(label, "{focused}");
    label.push_str(":title=");
    label.push_str(title_label);
    label
}

fn info_chrome_label(top_bar_rows: u64, tilable_rows: u64) -> String {
    let mut label =
        String::with_capacity("kittwm-info-chrome:top_bar_rows=:tilable_rows=".len() + 20 + 20);
    label.push_str("kittwm-info-chrome:top_bar_rows=");
    let _ = write!(label, "{top_bar_rows}");
    label.push_str(":tilable_rows=");
    let _ = write!(label, "{tilable_rows}");
    label
}

fn info_backdrop_label(workspace_label: &str, pane_count: u64) -> String {
    let mut label = String::with_capacity(
        "kittwm-info-backdrop:workspace=:panes="
            .len()
            .saturating_add(workspace_label.len())
            .saturating_add(20),
    );
    label.push_str("kittwm-info-backdrop:workspace=");
    label.push_str(workspace_label);
    label.push_str(":panes=");
    let _ = write!(label, "{pane_count}");
    label
}

fn info_scene_rows(pane_count: u64) -> u16 {
    pane_count.saturating_add(5).clamp(5, 18) as u16
}

fn daily_help_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn daily_help_row_label(kind_label: &str, idx: usize, row_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-daily-help-row::".len() + kind_label.len() + 20 + row_label.len(),
    );
    label.push_str("kittwm-daily-help-row:");
    label.push_str(kind_label);
    label.push(':');
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(row_label);
    label
}

fn daily_help_heading_label(kind_label: &str, heading_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-daily-help-heading::".len() + kind_label.len() + heading_label.len(),
    );
    label.push_str("kittwm-daily-help-heading:");
    label.push_str(kind_label);
    label.push(':');
    label.push_str(heading_label);
    label
}

fn daily_help_backdrop_label(kind_label: &str, line_count: usize, command_count: usize) -> String {
    let mut label = String::with_capacity(
        "kittwm-daily-help-backdrop::lines=:commands=".len() + kind_label.len() + 20 + 20,
    );
    label.push_str("kittwm-daily-help-backdrop:");
    label.push_str(kind_label);
    label.push_str(":lines=");
    let _ = write!(label, "{line_count}");
    label.push_str(":commands=");
    let _ = write!(label, "{command_count}");
    label
}

fn daily_help_scene_rows(line_count: usize) -> u16 {
    line_count.saturating_add(4).clamp(8, 30) as u16
}

fn info_indicator_rect(width: f32, y: f32) -> KittuiPxRect {
    let inset = (width * 0.12).min(10.0).floor().max(0.0);
    let x = inset.min((width - 1.0).max(0.0));
    let available = (width - x * 2.0).max(1.0).min(width.max(1.0));
    KittuiPxRect::new(x, y, available, 2.0)
}

fn info_scene_cols() -> u16 {
    let detected = TerminalInfo::detect().columns;
    info_scene_cols_from_sources(
        std::env::var("KITTWM_INFO_COLS")
            .or_else(|_| std::env::var("COLUMNS"))
            .ok()
            .as_deref(),
        detected,
    )
}

fn info_scene_cols_from_sources(value: Option<&str>, detected_cols: Option<u16>) -> u16 {
    graphical_scene_cols_from_sources(value, detected_cols, 72, 140)
}

fn panes_graphical_cmd(kitty: bool) -> Result<()> {
    let panes = load_panes_snapshot()?;
    let scene = panes_scene(&panes);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn events_graphical_cmd(ms: u64, kitty: bool) -> Result<()> {
    let events = load_events_snapshot(ms)?;
    let scene = events_scene(ms, &events);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn kittwm_z_index(role: kittwm_sdk::SurfacePlacementRole) -> i32 {
    kittwm_sdk::ArchitectureContract::current()
        .z_index_for_role(role)
        .expect("current kittwm architecture contract defines all placement roles")
}

fn kittwm_scene_placement_options(
    role: kittwm_sdk::SurfacePlacementRole,
) -> kittui_kitty::PlacementOptions {
    kittui_kitty::PlacementOptions::absolute().with_z_index(kittwm_z_index(role))
}

fn print_scene_or_kitty(
    scene: &Scene,
    kitty: bool,
    role: kittwm_sdk::SurfacePlacementRole,
) -> Result<()> {
    if kitty {
        let runtime = Runtime::builder()
            .terminal(TerminalInfo::detect())
            .build()?;
        let options = kittwm_scene_placement_options(role);
        let placement = runtime.place_at_with_options(scene, scene.footprint, &options)?;
        print!("{}", placement.to_bytes());
    } else {
        println!("{}", serde_json::to_string(scene)?);
    }
    Ok(())
}

fn load_panes_snapshot() -> Result<serde_json::Value> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let panes = client_request_multi(&path, "PANES_JSON")
        .map_err(|err| anyhow!("connect {}: {err}", path.display()))?;
    Ok(serde_json::from_str(&panes)?)
}

fn load_events_snapshot(ms: u64) -> Result<Vec<String>> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let request = events_request_millis(ms);
    let reply = client_request_multi(&path, &request)
        .map_err(|err| anyhow!("connect {}: {err}", path.display()))?;
    Ok(event_kinds_from_lines(&reply))
}

fn event_kinds_from_lines(lines: &str) -> Vec<String> {
    lines
        .lines()
        .filter(|line| *line != "END" && !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| value.get("kind")?.as_str().map(str::to_string))
        .collect()
}

fn events_scene(ms: u64, kinds: &[String]) -> Scene {
    events_scene_for_cols(ms, kinds, info_scene_cols())
}

fn events_scene_for_cols(ms: u64, kinds: &[String], cols: u16) -> Scene {
    let rows = events_scene_rows(kinds.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let summary = events_summary_label(kinds);
    let mut layers = vec![
        Layer {
            label: Some(events_backdrop_label(kinds.len(), ms)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(180, 142, 173, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(events_heading_label(&summary)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, kind) in kinds.iter().take(12).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let kind_label = truncate(kind, 48);
        layers.push(Layer {
            label: Some(events_row_label(idx, &kind_label)),
            root: Node::Rect {
                rect: info_indicator_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(235, 203, 139, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn events_backdrop_label(count: usize, ms: u64) -> String {
    let mut label = String::with_capacity("kittwm-events-backdrop:count=:ms=".len() + 40);
    label.push_str("kittwm-events-backdrop:count=");
    let _ = write!(label, "{count}");
    label.push_str(":ms=");
    let _ = write!(label, "{ms}");
    label
}

fn events_row_label(idx: usize, kind_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-event-row::"
            .len()
            .saturating_add(kind_label.len())
            .saturating_add(20),
    );
    label.push_str("kittwm-event-row:");
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(kind_label);
    label
}

fn events_heading_label(summary: &str) -> String {
    let mut label =
        String::with_capacity("kittwm-events-heading:".len().saturating_add(summary.len()));
    label.push_str("kittwm-events-heading:");
    label.push_str(summary);
    label
}

fn events_summary_label(kinds: &[String]) -> String {
    let mut summary = String::with_capacity(kinds.len().min(6).saturating_mul(33));
    for kind in kinds.iter().take(6) {
        if !summary.is_empty() {
            summary.push(',');
        }
        summary.push_str(&truncate(kind, 32));
    }
    summary
}

fn events_scene_rows(kind_count: usize) -> u16 {
    let rows = kind_count.saturating_add(4).min(u16::MAX as usize) as u16;
    rows.clamp(5, 18)
}

fn panes_scene(panes: &serde_json::Value) -> Scene {
    panes_scene_for_cols(panes, info_scene_cols())
}

fn panes_scene_for_cols(panes: &serde_json::Value, cols: u16) -> Scene {
    let details = panes
        .get("panes_detail")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let pane_count = panes
        .get("panes")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(details.len() as u64);
    let focus = panes
        .get("focus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let layout = panes
        .get("layout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let focus_label = truncate(focus, 32);
    let layout_label = truncate(layout, 32);
    let rows = panes_scene_rows(details.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let mut layers = vec![
        Layer {
            label: Some(panes_backdrop_label(
                pane_count,
                &focus_label,
                &layout_label,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-panes-heading:panes".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, pane) in details.iter().take(12).enumerate() {
        let window = pane
            .get("window")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-");
        let title = pane
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-");
        let focused = pane
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let app_cols = pane
            .get("app_cols")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let app_rows = pane
            .get("app_rows")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let window_label = truncate(window, 32);
        let title_label = truncate(title, 48);
        layers.push(Layer {
            label: Some(panes_scene_row_label(
                &window_label,
                focused,
                &title_label,
                app_cols,
                app_rows,
            )),
            root: Node::Rect {
                rect: info_indicator_rect(width, y),
                fill: Paint::Solid {
                    color: if focused {
                        Rgba::rgba(235, 203, 139, 255)
                    } else {
                        Rgba::rgba(136, 192, 208, 255)
                    },
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn panes_backdrop_label(pane_count: u64, focus_label: &str, layout_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-panes-backdrop:panes=:focus=:layout="
            .len()
            .saturating_add(focus_label.len())
            .saturating_add(layout_label.len())
            .saturating_add(20),
    );
    label.push_str("kittwm-panes-backdrop:panes=");
    let _ = write!(label, "{pane_count}");
    label.push_str(":focus=");
    label.push_str(focus_label);
    label.push_str(":layout=");
    label.push_str(layout_label);
    label
}

fn panes_scene_row_label(
    window_label: &str,
    focused: bool,
    title_label: &str,
    app_cols: u64,
    app_rows: u64,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-pane-row::focused=:title=:app=x"
            .len()
            .saturating_add(window_label.len())
            .saturating_add(5)
            .saturating_add(title_label.len())
            .saturating_add(20)
            .saturating_add(20),
    );
    label.push_str("kittwm-pane-row:");
    label.push_str(window_label);
    label.push_str(":focused=");
    let _ = write!(label, "{focused}");
    label.push_str(":title=");
    label.push_str(title_label);
    label.push_str(":app=");
    let _ = write!(label, "{app_cols}");
    label.push('x');
    let _ = write!(label, "{app_rows}");
    label
}

fn panes_scene_rows(detail_count: usize) -> u16 {
    let rows = detail_count.saturating_add(4).min(u16::MAX as usize) as u16;
    rows.clamp(5, 18)
}

fn session_graphical_cmd(kitty: bool) -> Result<()> {
    let session = load_session_snapshot()?;
    let scene = session_scene(&session);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn load_session_snapshot() -> Result<serde_json::Value> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let session = client_request_multi(&path, "SESSION_JSON")
        .map_err(|err| anyhow!("connect {}: {err}", path.display()))?;
    Ok(serde_json::from_str(&session)?)
}

fn session_scene(session: &serde_json::Value) -> Scene {
    session_scene_for_cols(session, info_scene_cols())
}

fn session_scene_for_cols(session: &serde_json::Value, cols: u16) -> Scene {
    let panes = session
        .get("panes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rows = session_scene_rows(panes.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let kind = session
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("kittwm-session");
    let layout = session
        .get("layout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let focus = session
        .get("focus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let kind_label = truncate(kind, 32);
    let layout_label = truncate(layout, 32);
    let focus_label = truncate(focus, 32);
    let schema = session
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .map(|version| version.to_string())
        .unwrap_or_else(|| "-".to_string());
    let mut layers = vec![
        Layer {
            label: Some(session_backdrop_label(
                &kind_label,
                &schema,
                &layout_label,
                &focus_label,
                panes.len(),
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(163, 190, 140, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-session-heading:manifest".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, pane) in panes.iter().take(18).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let window = pane
            .get("window")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-");
        let title = pane
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-");
        let command = pane
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-");
        let weight = pane
            .get("weight")
            .and_then(serde_json::Value::as_u64)
            .map(|weight| weight.to_string())
            .unwrap_or_else(|| "-".to_string());
        let focused = pane
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let window_label = truncate(window, 32);
        let title_label = truncate(title, 48);
        let command_label = truncate(command, 48);
        layers.push(Layer {
            label: Some(session_row_label(
                idx,
                &window_label,
                &title_label,
                &command_label,
                &weight,
                focused,
            )),
            root: Node::Rect {
                rect: session_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: if focused {
                        Rgba::rgba(235, 203, 139, 255)
                    } else {
                        Rgba::rgba(163, 190, 140, 255)
                    },
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn session_row_label(
    idx: usize,
    window_label: &str,
    title_label: &str,
    command_label: &str,
    weight: &str,
    focused: bool,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-session-row::window=:title=:command=:weight=:focused="
            .len()
            .saturating_add(window_label.len())
            .saturating_add(title_label.len())
            .saturating_add(command_label.len())
            .saturating_add(weight.len())
            .saturating_add(25),
    );
    label.push_str("kittwm-session-row:");
    let _ = write!(label, "{idx}");
    label.push_str(":window=");
    label.push_str(window_label);
    label.push_str(":title=");
    label.push_str(title_label);
    label.push_str(":command=");
    label.push_str(command_label);
    label.push_str(":weight=");
    label.push_str(weight);
    label.push_str(":focused=");
    let _ = write!(label, "{focused}");
    label
}

fn session_backdrop_label(
    kind_label: &str,
    schema: &str,
    layout_label: &str,
    focus_label: &str,
    pane_count: usize,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-session-backdrop:kind=:schema=:layout=:focus=:panes="
            .len()
            .saturating_add(kind_label.len())
            .saturating_add(schema.len())
            .saturating_add(layout_label.len())
            .saturating_add(focus_label.len())
            .saturating_add(20),
    );
    label.push_str("kittwm-session-backdrop:kind=");
    label.push_str(kind_label);
    label.push_str(":schema=");
    label.push_str(schema);
    label.push_str(":layout=");
    label.push_str(layout_label);
    label.push_str(":focus=");
    label.push_str(focus_label);
    label.push_str(":panes=");
    let _ = write!(label, "{pane_count}");
    label
}

fn session_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn session_scene_rows(pane_count: usize) -> u16 {
    let rows = pane_count.saturating_add(5).min(u16::MAX as usize) as u16;
    rows.clamp(8, 24)
}

fn chrome_graphical_cmd(kitty: bool) -> Result<()> {
    let chrome = load_chrome_snapshot()?;
    let scene = chrome_scene(&chrome);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn load_chrome_snapshot() -> Result<serde_json::Value> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let chrome = client_request_multi(&path, "CHROME_JSON")
        .map_err(|err| anyhow!("connect {}: {err}", path.display()))?;
    Ok(serde_json::from_str(&chrome)?)
}

fn chrome_scene(chrome: &serde_json::Value) -> Scene {
    let cols = info_scene_cols();
    let rows = 10;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let workspace = kittwm_scene_workspace_from(chrome.get("workspace"));
    let owner = chrome
        .get("owner")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .unwrap_or("-");
    let top = chrome
        .get("top_bar_rows")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let bottom = chrome
        .get("bottom_bar_rows")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let left = chrome
        .get("left_cols")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let right = chrome
        .get("right_cols")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let gap_cols = chrome
        .get("gap_cols")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let gap_rows = chrome
        .get("gap_rows")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let tilable_rows = chrome
        .get("tilable_rows")
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let workspace_label = truncate(&workspace, 32);
    let owner_label = truncate(owner, 32);
    let tilable_rows_label = truncate(&tilable_rows, 32);
    let mut layers = vec![
        Layer {
            label: Some(chrome_backdrop_label(
                &workspace_label,
                &owner_label,
                top,
                bottom,
                left,
                right,
                gap_cols,
                gap_rows,
                &tilable_rows_label,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-chrome-heading:drawable-reservation".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for idx in 0..8 {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(chrome_scene_row_label(
                idx,
                &workspace_label,
                &owner_label,
                top,
                bottom,
                left,
                right,
                gap_cols,
                gap_rows,
            )),
            root: Node::Rect {
                rect: chrome_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(136, 192, 208, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn chrome_backdrop_label(
    workspace_label: &str,
    owner_label: &str,
    top: u64,
    bottom: u64,
    left: u64,
    right: u64,
    gap_cols: u64,
    gap_rows: u64,
    tilable_rows_label: &str,
) -> String {
    let mut label = String::with_capacity(
        "kittwm-chrome-backdrop:workspace=:owner=:top=:bottom=:left=:right=:gap_cols=:gap_rows=:tilable_rows="
            .len()
            .saturating_add(workspace_label.len())
            .saturating_add(owner_label.len())
            .saturating_add(tilable_rows_label.len())
            .saturating_add(120),
    );
    label.push_str("kittwm-chrome-backdrop:workspace=");
    label.push_str(workspace_label);
    label.push_str(":owner=");
    label.push_str(owner_label);
    label.push_str(":top=");
    let _ = write!(label, "{top}");
    label.push_str(":bottom=");
    let _ = write!(label, "{bottom}");
    label.push_str(":left=");
    let _ = write!(label, "{left}");
    label.push_str(":right=");
    let _ = write!(label, "{right}");
    label.push_str(":gap_cols=");
    let _ = write!(label, "{gap_cols}");
    label.push_str(":gap_rows=");
    let _ = write!(label, "{gap_rows}");
    label.push_str(":tilable_rows=");
    label.push_str(tilable_rows_label);
    label
}

fn chrome_scene_row_label(
    idx: usize,
    workspace: &str,
    owner: &str,
    top: u64,
    bottom: u64,
    left: u64,
    right: u64,
    gap_cols: u64,
    gap_rows: u64,
) -> String {
    let mut out = String::with_capacity(44);
    let _ = write!(out, "kittwm-chrome-row:{idx}:");
    match idx {
        0 => {
            out.push_str("workspace=");
            out.push_str(workspace);
        }
        1 => {
            out.push_str("owner=");
            out.push_str(owner);
        }
        2 => {
            let _ = write!(out, "top_bar_rows={top}");
        }
        3 => {
            let _ = write!(out, "bottom_bar_rows={bottom}");
        }
        4 => {
            let _ = write!(out, "left_cols={left}");
        }
        5 => {
            let _ = write!(out, "right_cols={right}");
        }
        6 => {
            let _ = write!(out, "gap_cols={gap_cols}");
        }
        7 => {
            let _ = write!(out, "gap_rows={gap_rows}");
        }
        _ => out.push_str("unknown=-"),
    }
    out
}

fn chrome_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn status_graphical_cmd(kitty: bool) -> Result<()> {
    let status = load_status_snapshot()?;
    let scene = status_scene(&status);
    print_scene_or_kitty(&scene, kitty, kittwm_sdk::SurfacePlacementRole::Decoration)
}

fn load_status_snapshot() -> Result<serde_json::Value> {
    use kittui_cli::daemon::{client_request_multi, default_socket_path};
    let path = default_socket_path();
    let status = client_request_multi(&path, "STATUS_JSON")
        .map_err(|err| anyhow!("connect {}: {err}", path.display()))?;
    Ok(serde_json::from_str(&status)?)
}

fn status_scene(status: &serde_json::Value) -> Scene {
    status_scene_for_cols(status, status_scene_cols())
}

fn status_scene_for_cols(status: &serde_json::Value, cols: u16) -> Scene {
    let rows = 9;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let pid = status
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    let uptime = status
        .get("uptime_s")
        .and_then(serde_json::Value::as_u64)
        .map(|seconds| seconds.to_string())
        .unwrap_or_else(|| "-".to_string());
    let panes = status
        .get("panes")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let pending = status
        .get("pending")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let focus = status
        .get("focus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let layout = status
        .get("layout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let workspace = kittwm_scene_workspace_from(status.get("workspace"));
    let sock = status
        .get("sock")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let workspace_label = truncate(&workspace, 32);
    let layout_label = truncate(layout, 32);
    let focus_label = truncate(focus, 32);
    let sock_label = truncate(sock, 48);
    let mut layers = vec![
        Layer {
            label: Some(status_scene_backdrop_label(
                &pid,
                panes,
                pending,
                &focus_label,
                &layout_label,
                &workspace_label,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(163, 190, 140, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(status_scene_heading_label(&sock_label)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for idx in 0..7 {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(status_scene_row_label(
                idx,
                &pid,
                &uptime,
                &workspace_label,
                &layout_label,
                &focus_label,
                panes,
                pending,
            )),
            root: Node::Rect {
                rect: status_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(163, 190, 140, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn status_scene_backdrop_label(
    pid: &str,
    panes: u64,
    pending: u64,
    focus_label: &str,
    layout_label: &str,
    workspace_label: &str,
) -> String {
    let mut out = String::with_capacity(
        "kittwm-status-backdrop:pid=:panes=:pending=:focus=:layout=:workspace="
            .len()
            .saturating_add(pid.len())
            .saturating_add(focus_label.len())
            .saturating_add(layout_label.len())
            .saturating_add(workspace_label.len())
            .saturating_add(40),
    );
    out.push_str("kittwm-status-backdrop:pid=");
    out.push_str(pid);
    out.push_str(":panes=");
    let _ = write!(out, "{panes}");
    out.push_str(":pending=");
    let _ = write!(out, "{pending}");
    out.push_str(":focus=");
    out.push_str(focus_label);
    out.push_str(":layout=");
    out.push_str(layout_label);
    out.push_str(":workspace=");
    out.push_str(workspace_label);
    out
}

fn status_scene_heading_label(sock_label: &str) -> String {
    let mut out = String::with_capacity(
        "kittwm-status-heading:sock="
            .len()
            .saturating_add(sock_label.len()),
    );
    out.push_str("kittwm-status-heading:sock=");
    out.push_str(sock_label);
    out
}

fn status_scene_row_label(
    idx: usize,
    pid: &str,
    uptime: &str,
    workspace: &str,
    layout: &str,
    focus: &str,
    panes: u64,
    pending: u64,
) -> String {
    let mut out = String::with_capacity(40);
    let _ = write!(out, "kittwm-status-row:{idx}:");
    match idx {
        0 => {
            out.push_str("pid=");
            out.push_str(pid);
        }
        1 => {
            out.push_str("uptime_s=");
            out.push_str(uptime);
        }
        2 => {
            out.push_str("workspace=");
            out.push_str(workspace);
        }
        3 => {
            out.push_str("layout=");
            out.push_str(layout);
        }
        4 => {
            out.push_str("focus=");
            out.push_str(focus);
        }
        5 => {
            let _ = write!(out, "panes={panes}");
        }
        6 => {
            let _ = write!(out, "pending={pending}");
        }
        _ => out.push_str("unknown=-"),
    }
    out
}

fn status_scene_cols() -> u16 {
    let detected = TerminalInfo::detect().columns;
    status_scene_cols_from_sources(
        std::env::var("KITTWM_STATUS_COLS")
            .or_else(|_| std::env::var("COLUMNS"))
            .ok()
            .as_deref(),
        detected,
    )
}

fn status_scene_cols_from_sources(value: Option<&str>, detected_cols: Option<u16>) -> u16 {
    graphical_scene_cols_from_sources(value, detected_cols, 72, 140)
}

fn status_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    let margin = 10.0_f32.min((width / 4.0).max(0.0));
    KittuiPxRect::new(margin, y, (width - margin * 2.0).max(1.0), 1.5)
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
        "Commands: PING STATUS PANES EVENTS [ms] SPAWN <argv> WINDOWS DISPLAYS HELP QUIT (Ctrl-D to detach)"
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

fn shortcuts_cmd() -> Result<()> {
    print!("{}", kittui_cli::shortcuts::render_native_shortcuts());
    Ok(())
}

fn shortcuts_scene_json_cmd() -> Result<()> {
    println!("{}", serde_json::to_string(&shortcuts_scene())?);
    Ok(())
}

fn shortcuts_kitty_cmd() -> Result<()> {
    let scene = shortcuts_scene();
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;
    let options = kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Overlay);
    let placement = runtime.place_at_with_options(&scene, scene.footprint, &options)?;
    print!("{}", placement.to_bytes());
    Ok(())
}

fn shortcuts_scene() -> Scene {
    shortcuts_scene_for_cols(shortcuts_scene_cols())
}

fn shortcuts_scene_for_cols(cols: u16) -> Scene {
    let entries = kittui_cli::shortcuts::NATIVE_SHORTCUT_ENTRIES;
    let rows = shortcuts_scene_rows(entries.len());
    let visible_entries =
        shortcuts_scene_visible_entries(entries, shortcuts_scene_entry_limit(rows));
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let mut layers = vec![
        Layer {
            label: Some(shortcuts_scene_backdrop_label(entries.len())),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-shortcuts-heading:kittwm shortcuts".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, entry) in visible_entries.iter().enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(shortcuts_scene_row_label(
                entry.id,
                entry.keys,
                entry.description,
            )),
            root: Node::Rect {
                rect: shortcuts_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(163, 190, 140, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn shortcuts_scene_entry_limit(rows: u16) -> usize {
    rows.saturating_sub(2) as usize
}

fn shortcuts_scene_visible_entries<'a>(
    entries: &'a [kittui_cli::shortcuts::NativeShortcut],
    limit: usize,
) -> Vec<&'a kittui_cli::shortcuts::NativeShortcut> {
    let mut visible = entries.iter().take(limit).collect::<Vec<_>>();
    if visible.len() == limit && !visible.iter().any(|entry| entry.id == "title_markers") {
        if let (Some(marker_entry), Some(last)) = (
            entries.iter().find(|entry| entry.id == "title_markers"),
            visible.last_mut(),
        ) {
            *last = marker_entry;
        }
    }
    visible
}

fn shortcuts_scene_row_label(id: &str, keys: &str, description: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-shortcut-row:::".len() + id.len() + keys.len() + description.len(),
    );
    label.push_str("kittwm-shortcut-row:");
    label.push_str(id);
    label.push(':');
    label.push_str(keys);
    label.push(':');
    label.push_str(description);
    label
}

fn shortcuts_scene_backdrop_label(entry_count: usize) -> String {
    let mut label = String::with_capacity("kittwm-shortcuts-backdrop:count=".len() + 20);
    label.push_str("kittwm-shortcuts-backdrop:count=");
    let _ = write!(label, "{entry_count}");
    label
}

fn shortcuts_scene_rows(entry_count: usize) -> u16 {
    let rows = entry_count.saturating_add(3).min(u16::MAX as usize) as u16;
    rows.clamp(4, 18)
}

fn shortcuts_scene_cols() -> u16 {
    let detected = TerminalInfo::detect().columns;
    shortcuts_scene_cols_from_sources(
        std::env::var("KITTWM_SHORTCUTS_COLS")
            .or_else(|_| std::env::var("COLUMNS"))
            .ok()
            .as_deref(),
        detected,
    )
}

fn shortcuts_scene_cols_from_sources(value: Option<&str>, detected_cols: Option<u16>) -> u16 {
    graphical_scene_cols_from_sources(value, detected_cols, 72, 140)
}

fn shortcuts_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    let effective_width = width.max(1.0);
    let margin = 10.0_f32.min((effective_width / 4.0).max(0.0));
    let rect_width = (effective_width - margin * 2.0).max(0.0);
    KittuiPxRect::new(margin, y, rect_width, 1.5)
}

fn showcase_scene_json_cmd() -> Result<()> {
    println!(
        "{}",
        kittui_cli::session::native_showcase_scene_json(96, 24, true)?
    );
    Ok(())
}

fn showcase_metrics_json_cmd() -> Result<()> {
    println!(
        "{}",
        kittui_cli::session::native_showcase_metrics_json(96, 24, true)?
    );
    Ok(())
}

fn showcase_composition_json_cmd() -> Result<()> {
    println!(
        "{}",
        kittui_cli::session::native_showcase_composition_json(96, 24, true)?
    );
    Ok(())
}

fn tui_smoke_json_cmd() -> Result<()> {
    println!("{}", kittui_cli::session::native_tui_smoke_matrix_json()?);
    Ok(())
}

fn shortcuts_json_cmd() -> Result<()> {
    print!("{}", kittui_cli::shortcuts::render_native_shortcuts_json());
    Ok(())
}

fn keymap_cmd(cli: &Cli) -> Result<()> {
    let km = load_keymap(cli)?;
    if cli.keymap_check {
        return keymap_check_cmd(&km);
    }
    if cli.keymap_scene_json || cli.keymap_kitty {
        let scene = keymap_scene(&km);
        return print_scene_or_kitty(
            &scene,
            cli.keymap_kitty,
            kittwm_sdk::SurfacePlacementRole::Decoration,
        );
    }
    print!("{}", km.render_table());
    Ok(())
}

fn load_keymap(cli: &Cli) -> Result<kittui_cli::keymap::Keymap> {
    if let Some(path) = &cli.keymap_path {
        kittui_cli::keymap::Keymap::load(std::path::Path::new(path))
    } else {
        Ok(kittui_cli::keymap::default_keymap())
    }
}

fn keymap_scene(km: &kittui_cli::keymap::Keymap) -> Scene {
    let cols = info_scene_cols();
    let rows = keymap_scene_rows(km.bindings.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let prefix_label = km
        .prefix
        .as_ref()
        .map(|prefix| keymap_keyspec_label(prefix, 32))
        .unwrap_or_else(|| "<none>".to_string());
    let duplicates = keymap_duplicate_count(km);
    let mut layers = vec![
        Layer {
            label: Some(keymap_scene_backdrop_label(
                km.bindings.len(),
                &prefix_label,
                duplicates,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(235, 203, 139, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-keymap-heading:resolved-keymap".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, binding) in km.bindings.iter().take(20).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let chord_label = keymap_chord_label(&binding.chord, 48);
        let action_label = keymap_action_label(&binding.action, 48);
        layers.push(Layer {
            label: Some(keymap_scene_row_label(idx, &chord_label, &action_label)),
            root: Node::Rect {
                rect: keymap_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(235, 203, 139, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn keymap_scene_backdrop_label(bindings: usize, prefix_label: &str, duplicates: usize) -> String {
    let mut label = String::with_capacity(
        "kittwm-keymap-backdrop:bindings=:prefix=:duplicates="
            .len()
            .saturating_add(prefix_label.len())
            .saturating_add(40),
    );
    label.push_str("kittwm-keymap-backdrop:bindings=");
    let _ = write!(label, "{bindings}");
    label.push_str(":prefix=");
    label.push_str(prefix_label);
    label.push_str(":duplicates=");
    let _ = write!(label, "{duplicates}");
    label
}

fn keymap_scene_row_label(idx: usize, chord_label: &str, action_label: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-keymap-row::"
            .len()
            .saturating_add(chord_label.len())
            .saturating_add(action_label.len())
            .saturating_add(20),
    );
    label.push_str("kittwm-keymap-row:");
    let _ = write!(label, "{idx}");
    label.push(':');
    label.push_str(chord_label);
    label.push(':');
    label.push_str(action_label);
    label
}

fn keymap_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn keymap_keyspec_label(spec: &kittui_cli::keymap::KeySpec, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(max);
    let mut used = 0usize;
    if spec.mods.ctrl {
        out.push_str("C-");
        used += 2;
    }
    if spec.mods.alt {
        out.push_str("M-");
        used += 2;
    }
    if spec.mods.shift {
        out.push_str("S-");
        used += 2;
    }
    if used >= max {
        return truncate(&out, max);
    }
    out.push_str(&truncate(&spec.key, max - used));
    truncate(&out, max)
}

fn keymap_chord_label(chord: &[kittui_cli::keymap::KeySpec], max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(max);
    let mut used = 0usize;
    for spec in chord {
        if !out.is_empty() {
            if used + 1 >= max {
                out.push('…');
                return truncate(&out, max);
            }
            out.push(' ');
            used += 1;
        }
        if used >= max {
            return truncate(&out, max);
        }
        let label = keymap_keyspec_label(spec, max - used);
        used += label.chars().count();
        out.push_str(&label);
        if out.ends_with('…') {
            return out;
        }
    }
    out
}

fn keymap_action_label(action: &kittui_cli::keymap::Action, max: usize) -> String {
    match action {
        kittui_cli::keymap::Action::Custom(action) => truncate(action, max),
        action => truncate(&action.to_string(), max),
    }
}

fn keymap_scene_rows(binding_count: usize) -> u16 {
    binding_count.saturating_add(5).clamp(8, 28) as u16
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
                    | kittui_cli::keymap::Action::WorkspaceSwitch(_)
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
    {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for (chord, actions) in duplicates {
            write_duplicate_action_labels(&mut out, chord, actions)?;
        }
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

fn write_duplicate_action_labels(
    mut out: impl std::io::Write,
    chord: &str,
    actions: &[String],
) -> std::io::Result<()> {
    write!(out, "  {chord}: ")?;
    for (idx, action) in actions.iter().enumerate() {
        if idx > 0 {
            out.write_all(b", ")?;
        }
        out.write_all(action.as_bytes())?;
    }
    out.write_all(b"\n")
}

fn remote_apps_cmd(cli: &Cli, host: &str) -> Result<()> {
    if cli.apps_scene_json || cli.apps_kitty {
        return Err(anyhow!(
            "remote apps currently supports text/json/first/launch-first/json launch-first; run `ssh {host} kittwm apps-kitty` when remote kittwm is installed"
        ));
    }
    let limit = cli.apps_limit.unwrap_or(50);
    let mode = remote_apps_mode(cli);
    run_pooled_ssh_script(
        host,
        &remote_apps_env(
            host,
            cli.apps_filter.as_deref(),
            limit,
            mode,
            cli.apps_force_fallback,
        ),
        remote_apps_script(),
        "remote apps",
        mode.requests_graphical_forwarding(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteListingKind {
    Windows,
    Displays,
}

impl RemoteListingKind {
    fn env_value(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Displays => "displays",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Windows => "remote windows",
            Self::Displays => "remote displays",
        }
    }
}

fn remote_listing_cmd(
    kind: RemoteListingKind,
    host: &str,
    query: Option<&str>,
    json: bool,
    force_fallback: bool,
) -> Result<()> {
    run_pooled_ssh_script(
        host,
        &[
            (
                "KITTWM_REMOTE_KIND".to_string(),
                kind.env_value().to_string(),
            ),
            (
                "KITTWM_REMOTE_QUERY".to_string(),
                query.unwrap_or_default().to_string(),
            ),
            (
                "KITTWM_REMOTE_JSON".to_string(),
                if json { "1" } else { "0" }.to_string(),
            ),
            (
                "KITTWM_REMOTE_FORCE_FALLBACK".to_string(),
                if force_fallback { "1" } else { "0" }.to_string(),
            ),
            ("KITTWM_REMOTE_TARGET".to_string(), host.to_string()),
        ],
        remote_listing_script(),
        kind.label(),
        false,
    )
}

fn run_pooled_ssh_script(
    host: &str,
    env: &[(String, String)],
    script: &str,
    label: &str,
    graphical_forwarding: bool,
) -> Result<()> {
    let args = pooled_ssh_args_with_forwarding(host, env, script, graphical_forwarding)?;
    let status = std::process::Command::new("ssh")
        .args(&args)
        .status()
        .map_err(|e| anyhow!("ssh {label} {host}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("ssh {label} {host} exited with {status}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteAppsMode {
    List,
    Json,
    First,
    FirstJson,
    LaunchFirst,
    LaunchFirstJson,
}

impl RemoteAppsMode {
    fn requests_graphical_forwarding(self) -> bool {
        matches!(self, Self::LaunchFirst | Self::LaunchFirstJson)
    }
}

fn remote_apps_mode(cli: &Cli) -> RemoteAppsMode {
    if cli.apps_launch_first && cli.json {
        RemoteAppsMode::LaunchFirstJson
    } else if cli.apps_launch_first {
        RemoteAppsMode::LaunchFirst
    } else if cli.apps_first && cli.json {
        RemoteAppsMode::FirstJson
    } else if cli.apps_first {
        RemoteAppsMode::First
    } else if cli.json {
        RemoteAppsMode::Json
    } else {
        RemoteAppsMode::List
    }
}

fn remote_apps_env(
    host: &str,
    query: Option<&str>,
    limit: usize,
    mode: RemoteAppsMode,
    force_fallback: bool,
) -> Vec<(String, String)> {
    vec![
        ("KITTWM_REMOTE_TARGET".to_string(), host.to_string()),
        (
            "KITTWM_REMOTE_QUERY".to_string(),
            query.unwrap_or_default().to_string(),
        ),
        ("KITTWM_REMOTE_LIMIT".to_string(), limit.to_string()),
        (
            "KITTWM_REMOTE_MODE".to_string(),
            match mode {
                RemoteAppsMode::List => "list",
                RemoteAppsMode::Json => "json",
                RemoteAppsMode::First => "first",
                RemoteAppsMode::FirstJson => "first-json",
                RemoteAppsMode::LaunchFirst => "launch-first",
                RemoteAppsMode::LaunchFirstJson => "launch-first-json",
            }
            .to_string(),
        ),
        (
            "KITTWM_REMOTE_FORCE_FALLBACK".to_string(),
            if force_fallback { "1" } else { "0" }.to_string(),
        ),
    ]
}

fn pooled_ssh_args(host: &str, env: &[(String, String)], script: &str) -> Result<Vec<String>> {
    pooled_ssh_args_with_forwarding(host, env, script, false)
}

fn pooled_ssh_args_with_forwarding(
    host: &str,
    env: &[(String, String)],
    script: &str,
    graphical_forwarding: bool,
) -> Result<Vec<String>> {
    if host.trim().is_empty() {
        return Err(anyhow!("--remote HOST must not be empty"));
    }
    let control_path = pooled_ssh_control_path(graphical_forwarding)?;
    let mut args = vec![
        "-o".to_string(),
        "ControlMaster=auto".to_string(),
        "-o".to_string(),
        "ControlPersist=10m".to_string(),
        "-o".to_string(),
        ssh_control_path_arg(&control_path),
    ];
    if graphical_forwarding {
        args.push("-Y".to_string());
    }
    args.extend([host.to_string(), "env".to_string()]);
    args.extend(env.iter().map(|(key, value)| ssh_env_arg(key, value)));
    args.extend(["sh".to_string(), "-lc".to_string(), script.to_string()]);
    Ok(args)
}

fn ssh_env_arg(key: &str, value: &str) -> String {
    let quoted = shell_quote(value);
    let mut out = String::with_capacity(key.len() + 1 + quoted.len());
    out.push_str(key);
    out.push('=');
    out.push_str(&quoted);
    out
}

fn ssh_control_path_arg(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    let mut out = String::with_capacity("ControlPath=".len() + path.len());
    out.push_str("ControlPath=");
    out.push_str(&path);
    out
}

fn pooled_ssh_control_path(graphical_forwarding: bool) -> Result<std::path::PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("kittwm-ssh");
    std::fs::create_dir_all(&dir).with_context(|| create_dir_context(&dir))?;
    Ok(dir.join(if graphical_forwarding { "%C-x11" } else { "%C" }))
}

fn create_dir_context(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    let mut out = String::with_capacity("create ".len() + path.len());
    out.push_str("create ");
    out.push_str(&path);
    out
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b':' | b','))
    {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn remote_apps_script() -> &'static str {
    r#"host=$(hostname 2>/dev/null || printf unknown)
command_host=${KITTWM_REMOTE_TARGET:-$host}
json_escape() {
    awk '{ gsub(/\\/, "\\\\"); gsub(/\"/, "\\\""); printf "\"%s\"", $0 }'
}
json_option() {
    value=$1
    if [ -n "$value" ]; then
        printf '%s' "$value" | json_escape
    else
        printf 'null'
    fi
}
kittwm_remote_forced_fallback_json() {
    if [ "${KITTWM_REMOTE_FORCE_FALLBACK:-0}" = "1" ]; then
        printf true
    else
        printf false
    fi
}
kittwm_remote_json_mode() {
    case "${KITTWM_REMOTE_MODE:-list}" in
        json|first-json|launch-first-json) return 0 ;;
        *) return 1 ;;
    esac
}
kittwm_remote_emit_kittwm_output() {
    if kittwm_remote_json_mode; then
        while IFS= read -r line; do
            case "$line" in
                \{*)
                    rest=${line#\{}
                    printf '{"host":%s,"target_host":%s,"source":"kittwm","forced_fallback":false,%s\n' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$rest"
                    ;;
                *) printf '%s\n' "$line" ;;
            esac
        done
    else
        cat
    fi
}
if [ "${KITTWM_REMOTE_FORCE_FALLBACK:-0}" != "1" ] && command -v kittwm >/dev/null 2>&1; then
    set -- apps --limit "${KITTWM_REMOTE_LIMIT:-50}"
    if [ -n "${KITTWM_REMOTE_QUERY:-}" ]; then set -- "$@" --filter "$KITTWM_REMOTE_QUERY"; fi
    case "${KITTWM_REMOTE_MODE:-list}" in
        json) set -- "$@" --json ;;
        first) set -- "$@" --first ;;
        first-json) set -- "$@" --first --json ;;
        launch-first) set -- "$@" --launch-first ;;
        launch-first-json) set -- "$@" --launch-first --json ;;
    esac
    kittwm_err=$(mktemp "${TMPDIR:-/tmp}/kittwm-remote-apps.XXXXXX" 2>/dev/null || printf '')
    if [ -n "$kittwm_err" ]; then
        kittwm_out=$(kittwm "$@" 2>"$kittwm_err")
        kittwm_status=$?
        if [ $kittwm_status -eq 0 ]; then
            rm -f "$kittwm_err"
            printf '%s\n' "$kittwm_out" | kittwm_remote_emit_kittwm_output
            exit 0
        fi
        cat "$kittwm_err" >&2
        rm -f "$kittwm_err"
    else
        kittwm_out=$(kittwm "$@" 2>&1)
        kittwm_status=$?
        if [ $kittwm_status -eq 0 ]; then
            printf '%s\n' "$kittwm_out" | kittwm_remote_emit_kittwm_output
            exit 0
        fi
        printf '%s\n' "$kittwm_out" >&2
    fi
    printf 'WARN remote kittwm apps failed; falling back to shell app discovery on target=%s host=%s\n' "$command_host" "$host" >&2
fi
json_escape() {
    awk '{ gsub(/\\/, "\\\\"); gsub(/\"/, "\\\""); printf "\"%s\"", $0 }'
}
json_option() {
    value=$1
    if [ -n "$value" ]; then
        printf '%s' "$value" | json_escape
    else
        printf 'null'
    fi
}
kittwm_remote_list_path_commands() {
    old_ifs=$IFS
    IFS=:
    for dir in $PATH; do
        [ -d "$dir" ] || continue
        for path in "$dir"/*; do
            [ -f "$path" ] && [ -x "$path" ] && printf 'path\t%s\n' "$(basename "$path")"
        done
    done
    IFS=$old_ifs
}
kittwm_remote_list_macos_apps() {
    command -v open >/dev/null 2>&1 || return 0
    for root in /Applications /System/Applications "$HOME/Applications"; do
        [ -d "$root" ] || continue
        find "$root" -name '*.app' -prune 2>/dev/null | while IFS= read -r app; do
            name=$(basename "$app" .app)
            [ -n "$name" ] && printf 'macos\t%s\n' "$name"
        done
    done
}
kittwm_remote_desktop_field_matches() {
    field_values=$1
    current_values="${XDG_CURRENT_DESKTOP:-};${DESKTOP_SESSION:-}"
    [ -n "$field_values" ] || return 1
    awk -v field="$field_values" -v current="$current_values" 'BEGIN {
        n=split(tolower(field), f, ";");
        m=split(tolower(current), c, /[;:]/);
        for (i=1; i<=n; i++) for (j=1; j<=m; j++) if (f[i] != "" && c[j] != "" && f[i] == c[j]) exit 0;
        exit 1;
    }'
}
kittwm_remote_desktop_localized_values() {
    key=$1
    desktop=$2
    awk -F= -v key="$key" '$1 ~ "^" key "\\[[^]]+\\]$" { print substr($0, index($0, "=") + 1) }' "$desktop" 2>/dev/null | tr '\n' ';'
}
kittwm_remote_try_exec_token() {
    value=$1
    case "$value" in
        \"*) printf '%s\n' "$value" | sed -E 's/^"(([^"\\]|\\.)*)".*/\1/; s/\\"/"/g; s/\\\\/\\/g' ;;
        \'*) printf '%s\n' "$value" | sed -E "s/^'([^']*)'.*/\\1/" ;;
        *) printf '%s\n' "$value" | awk '{ print $1 }' ;;
    esac
}
kittwm_remote_try_exec_available() {
    token=$(kittwm_remote_try_exec_token "$1")
    [ -n "$token" ] || return 1
    case "$token" in
        */*) [ -x "$token" ] ;;
        *) command -v "$token" >/dev/null 2>&1 ;;
    esac
}
kittwm_remote_linux_desktop_roots() {
    printf '%s\n' "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    old_ifs=$IFS
    IFS=:
    for dir in ${XDG_DATA_DIRS:-/usr/local/share:/usr/share}; do
        [ -n "$dir" ] && printf '%s\n' "$dir/applications"
    done
    IFS=$old_ifs
}
kittwm_remote_list_linux_desktop_apps() {
    kittwm_remote_linux_desktop_roots | awk '!seen[$0]++' | while IFS= read -r root; do
        [ -d "$root" ] || continue
        find "$root" -name '*.desktop' -type f 2>/dev/null | while IFS= read -r desktop; do
            entry_type=$(awk -F= '$1 == "Type" { print tolower($2); exit }' "$desktop" 2>/dev/null)
            [ -n "$entry_type" ] && [ "$entry_type" != "application" ] && continue
            hidden=$(awk -F= '$1 == "Hidden" || $1 == "NoDisplay" { v=tolower($2); if (v == "true" || v == "1") { print "1"; exit } }' "$desktop" 2>/dev/null)
            [ "$hidden" = "1" ] && continue
            only_show_in=$(awk -F= '$1 == "OnlyShowIn" { print $2; exit }' "$desktop" 2>/dev/null)
            not_show_in=$(awk -F= '$1 == "NotShowIn" { print $2; exit }' "$desktop" 2>/dev/null)
            try_exec=$(awk -F= '$1 == "TryExec" { print $2; exit }' "$desktop" 2>/dev/null)
            [ -n "$only_show_in" ] && ! kittwm_remote_desktop_field_matches "$only_show_in" && continue
            [ -n "$not_show_in" ] && kittwm_remote_desktop_field_matches "$not_show_in" && continue
            [ -n "$try_exec" ] && ! kittwm_remote_try_exec_available "$try_exec" && continue
            id=$(basename "$desktop" .desktop)
            name=$(awk -F= '$1 == "Name" { print substr($0, index($0, "=") + 1); exit }' "$desktop" 2>/dev/null)
            localized_names=$(kittwm_remote_desktop_localized_values Name "$desktop")
            generic_name=$(awk -F= '$1 == "GenericName" { print substr($0, index($0, "=") + 1); exit }' "$desktop" 2>/dev/null)
            localized_generic_names=$(kittwm_remote_desktop_localized_values GenericName "$desktop")
            comment=$(awk -F= '$1 == "Comment" { print substr($0, index($0, "=") + 1); exit }' "$desktop" 2>/dev/null)
            localized_comments=$(kittwm_remote_desktop_localized_values Comment "$desktop")
            keywords=$(awk -F= '$1 == "Keywords" { print substr($0, index($0, "=") + 1); exit }' "$desktop" 2>/dev/null)
            localized_keywords=$(kittwm_remote_desktop_localized_values Keywords "$desktop")
            categories=$(awk -F= '$1 == "Categories" { print substr($0, index($0, "=") + 1); exit }' "$desktop" 2>/dev/null)
            exec_line=$(awk -F= '$1 == "Exec" { print substr($0, index($0, "=") + 1); exit }' "$desktop" 2>/dev/null)
            [ -n "$name" ] || name="$id"
            [ -n "$id" ] && printf 'desktop\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$name" "$exec_line" "$desktop" "$generic_name" "$keywords" "$categories" "$localized_names" "$localized_generic_names" "$localized_keywords" "$comment" "$localized_comments"
        done
    done
}
kittwm_remote_candidates() {
    { kittwm_remote_list_path_commands; kittwm_remote_list_macos_apps; kittwm_remote_list_linux_desktop_apps; } | awk -F '\t' -v q="${KITTWM_REMOTE_QUERY:-}" 'BEGIN { q=tolower(q) } !seen[$1 FS $2]++ && (q == "" || index(tolower($0), q))'
}
kittwm_remote_candidate_count() {
    wanted=$1
    kittwm_remote_candidates | awk -F '\t' -v wanted="$wanted" '$1 == wanted { print }' | head -n "$limit" | awk 'END { print NR + 0 }'
}
limit=${KITTWM_REMOTE_LIMIT:-50}
mode=${KITTWM_REMOTE_MODE:-list}
host=$(hostname 2>/dev/null || printf unknown)
command_host=${KITTWM_REMOTE_TARGET:-$host}
kittwm_remote_launch_error() {
    code=$1
    message=$2
    hint=${3:-}
    if [ "$mode" = "launch-first-json" ]; then
        printf '{"host":%s,"target_host":%s,"source":"fallback","forced_fallback":%s,"mode":"launch-first","filter":%s,"error":%s,"message":%s,"hint":%s}
' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$(kittwm_remote_forced_fallback_json)" "$(json_option "${KITTWM_REMOTE_QUERY:-}")" "$(printf '%s' "$code" | json_escape)" "$(printf '%s' "$message" | json_escape)" "$(json_option "$hint")"
    elif [ -n "$hint" ]; then
        printf 'ERR %s; %s
' "$message" "$hint"
    else
        printf 'ERR %s
' "$message"
    fi
}
case "$mode" in
    json)
        path_count=$(kittwm_remote_candidate_count path)
        macos_count=$(kittwm_remote_candidate_count macos)
        linux_desktop_count=$(kittwm_remote_candidate_count desktop)
        total_count=$((path_count + macos_count + linux_desktop_count))
        printf '{"host":%s,"target_host":%s,"source":"fallback","forced_fallback":%s,"mode":"shell-path-macos-linux-desktop","filter":%s,"limit":%s,"path_commands":[' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$(kittwm_remote_forced_fallback_json)" "$(json_option "${KITTWM_REMOTE_QUERY:-}")" "$limit"
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "path" { print $2 }' | head -n "$limit" | while IFS= read -r cmd; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$cmd" | json_escape
        done
        printf '],"macos_apps":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "macos" { print $2 }' | head -n "$limit" | while IFS= read -r app; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$app" | json_escape
        done
        printf '],"linux_desktop_ids":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $2 }' | head -n "$limit" | while IFS= read -r id; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$id" | json_escape
        done
        printf '],"linux_desktop_files":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $5 }' | head -n "$limit" | while IFS= read -r file; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$file" | json_escape
        done
        printf '],"linux_desktop_apps":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print ($3 != "" ? $3 : $2) }' | head -n "$limit" | while IFS= read -r app; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$app" | json_escape
        done
        printf '],"linux_desktop_localized_names":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $9 }' | head -n "$limit" | while IFS= read -r localized; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$localized" | json_escape
        done
        printf '],"linux_desktop_generic_names":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $6 }' | head -n "$limit" | while IFS= read -r generic; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$generic" | json_escape
        done
        printf '],"linux_desktop_keywords":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $7 }' | head -n "$limit" | while IFS= read -r keywords; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$keywords" | json_escape
        done
        printf '],"linux_desktop_categories":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $8 }' | head -n "$limit" | while IFS= read -r categories; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$categories" | json_escape
        done
        printf '],"linux_desktop_comments":['
        first=1
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { print $12 }' | head -n "$limit" | while IFS= read -r comment; do
            [ $first -eq 1 ] || printf ','
            first=0
            printf '%s' "$comment" | json_escape
        done
        printf '],"path_count":%s,"macos_count":%s,"linux_desktop_count":%s,"total_count":%s}' "$path_count" "$macos_count" "$linux_desktop_count" "$total_count"
        ;;
    first|first-json)
        candidate=$(kittwm_remote_candidates | head -n 1)
        [ -n "$candidate" ] || {
            if [ "$mode" = "first-json" ]; then
                printf '{"host":%s,"target_host":%s,"source":"fallback","forced_fallback":%s,"mode":"first","filter":%s,"error":"no_candidates","message":"no remote app candidates matched"}\n' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$(kittwm_remote_forced_fallback_json)" "$(json_option "${KITTWM_REMOTE_QUERY:-}")"
            else
                echo "ERR no remote app candidates matched"
            fi
            exit 1
        }
        kind=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $1}')
        name=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $2}')
        label=$(printf '%s\n' "$candidate" | awk -F '\t' '{print ($3 != "" ? $3 : $2)}')
        desktop_file=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $5}')
        if [ "$mode" = "first-json" ]; then
            printf '{"host":%s,"target_host":%s,"source":"fallback","forced_fallback":%s,"mode":"first","filter":%s,"kind":%s,"candidate":%s,"name":%s,"desktop_file":%s}\n' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$(kittwm_remote_forced_fallback_json)" "$(json_option "${KITTWM_REMOTE_QUERY:-}")" "$(printf '%s' "$kind" | json_escape)" "$(printf '%s' "$name" | json_escape)" "$(printf '%s' "$label" | json_escape)" "$(json_option "$desktop_file")"
        else
            printf '%s:%s\n' "$kind" "$label"
        fi
        ;;
    launch-first|launch-first-json)
        candidate=$(kittwm_remote_candidates | head -n 1)
        [ -n "$candidate" ] || { kittwm_remote_launch_error no_candidates "no remote app candidates matched"; exit 1; }
        kind=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $1}')
        name=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $2}')
        label=$(printf '%s\n' "$candidate" | awk -F '\t' '{print ($3 != "" ? $3 : $2)}')
        exec_line=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $4}')
        desktop_file=$(printf '%s\n' "$candidate" | awk -F '\t' '{print $5}')
        launch_pid=""
        launch_method=""
        if [ "$kind" = "macos" ]; then
            open -a "$name" >/dev/null 2>&1 &
            launch_pid=$!
            launch_method="open"
        elif [ "$kind" = "desktop" ]; then
            if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
                kittwm_remote_launch_error no_graphical_display "no remote graphical display is available for Linux desktop launch" "try: kittwm remote $command_host graphical (checks X11 forwarding and waypipe)"; exit 1
            fi
            if command -v gtk-launch >/dev/null 2>&1; then
                gtk-launch "$name" >/dev/null 2>&1 &
                launch_pid=$!
                if wait "$launch_pid"; then
                    launch_method="gtk-launch"
                else
                    launch_pid=""
                fi
            fi
            if [ -z "$launch_pid" ] && [ -n "$desktop_file" ] && command -v gio >/dev/null 2>&1; then
                gio launch "$desktop_file" >/dev/null 2>&1 &
                launch_pid=$!
                if wait "$launch_pid"; then
                    launch_method="gio"
                else
                    launch_pid=""
                fi
            fi
            if [ -z "$launch_pid" ] && [ -n "$exec_line" ]; then
                desktop_exec=$(printf '%s\n' "$exec_line" | sed -E 's/[[:space:]]+%[fFuUdDnNickvm]//g; s/%[fFuUdDnNickvm]//g')
                sh -lc "$desktop_exec" >/dev/null 2>&1 &
                launch_pid=$!
                launch_method="desktop-exec"
            elif [ -z "$launch_pid" ]; then
                kittwm_remote_launch_error no_desktop_exec_fallback "gtk-launch/gio failed and no Linux desktop Exec fallback is available"; exit 1
            fi
        else
            "$name" >/dev/null 2>&1 &
            launch_pid=$!
            launch_method="path"
        fi
        if [ "$mode" = "launch-first-json" ]; then
            printf '{"host":%s,"target_host":%s,"source":"fallback","forced_fallback":%s,"mode":"launch-first","filter":%s,"kind":%s,"method":%s,"candidate":%s,"name":%s,"desktop_file":%s,"pid":%s}\n' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$(kittwm_remote_forced_fallback_json)" "$(json_option "${KITTWM_REMOTE_QUERY:-}")" "$(printf '%s' "$kind" | json_escape)" "$(printf '%s' "$launch_method" | json_escape)" "$(printf '%s' "$name" | json_escape)" "$(printf '%s' "$label" | json_escape)" "$(json_option "$desktop_file")" "$(printf '%s' "$launch_pid" | json_escape)"
        else
            printf 'kittwm remote apps: launched pid=%s kind=%s method=%s forced_fallback=%s name=%s host=%s target_host=%s\n' "$launch_pid" "$kind" "$launch_method" "$(kittwm_remote_forced_fallback_json)" "$label" "$host" "$command_host"
        fi
        ;;
    *)
        printf 'kittwm remote apps\n==================\nhost: %s\ntarget host: %s\nmode: shell-path-macos-linux-desktop\nforced fallback: %s\n' "$host" "$command_host" "$(kittwm_remote_forced_fallback_json)"
        if [ -n "${KITTWM_REMOTE_QUERY:-}" ]; then printf 'filter: %s\n' "$KITTWM_REMOTE_QUERY"; fi
        printf 'PATH commands (first %s):\n' "$limit"
        kittwm_remote_candidates | awk -F '\t' '$1 == "path" { print "  "$2 }' | head -n "$limit"
        printf 'macOS applications (first %s):\n' "$limit"
        kittwm_remote_candidates | awk -F '\t' '$1 == "macos" { print "  "$2 }' | head -n "$limit"
        printf 'Linux desktop entries (first %s):\n' "$limit"
        kittwm_remote_candidates | awk -F '\t' '$1 == "desktop" { label=($3 != "" ? $3 : $2); detail=""; if ($9 != "") detail=$9; if ($6 != "") detail=(detail != "" ? detail"; "$6 : $6); if ($8 != "") detail=(detail != "" ? detail"; "$8 : $8); print "  "label" ("$2") — "$5(detail != "" ? " — "detail : "") }' | head -n "$limit"
        ;;
esac
"#
}

fn remote_listing_script() -> &'static str {
    r#"kind=${KITTWM_REMOTE_KIND:-windows}
query=${KITTWM_REMOTE_QUERY:-}
json=${KITTWM_REMOTE_JSON:-0}
host=$(hostname 2>/dev/null || printf unknown)
command_host=${KITTWM_REMOTE_TARGET:-$host}
json_escape() {
    awk 'BEGIN { ORS="" } { gsub(/\\/, "\\\\"); gsub(/"/, "\\\""); gsub(/\r/, "\\r"); gsub(/\t/, "\\t"); printf "\"%s\"", $0 }'
}
json_option() {
    value=$1
    if [ -n "$value" ]; then
        printf '%s' "$value" | json_escape
    else
        printf 'null'
    fi
}
kittwm_remote_emit_json_lines() {
    mode=${1:-fallback}
    source=${2:-unknown}
    forced_fallback=$([ "${KITTWM_REMOTE_FORCE_FALLBACK:-0}" = "1" ] && printf true || printf false)
    printf '{"host":%s,"target_host":%s,"kind":%s,"filter":%s,"forced_fallback":%s,"mode":%s,"source":%s,"lines":[' "$(printf '%s' "$host" | json_escape)" "$(printf '%s' "$command_host" | json_escape)" "$(printf '%s' "$kind" | json_escape)" "$(json_option "$query")" "$forced_fallback" "$(printf '%s' "$mode" | json_escape)" "$(printf '%s' "$source" | json_escape)"
    first=1
    count=0
    while IFS= read -r line; do
        [ $first -eq 1 ] || printf ','
        first=0
        count=$((count + 1))
        printf '%s' "$line" | json_escape
    done
    printf '],"count":%s}\n' "$count"
}
kittwm_remote_emit() {
    mode=${1:-fallback}
    source=${2:-unknown}
    if [ "$json" = "1" ]; then
        kittwm_remote_emit_json_lines "$mode" "$source"
    else
        cat
    fi
}
kittwm_remote_filter() {
    if [ -n "$query" ]; then
        awk -v q="$query" 'BEGIN { q=tolower(q) } index(tolower($0), q)'
    else
        cat
    fi
}
kittwm_remote_sway_outputs_python() {
    python3 -c 'import json,sys
for output in json.load(sys.stdin):
    mode = output.get("current_mode") or {}
    print("  {name} {make} {model} {width}x{height} active={active}".format(
        name=output.get("name") or "?",
        make=output.get("make") or "",
        model=output.get("model") or "",
        width=mode.get("width") or 0,
        height=mode.get("height") or 0,
        active=str(bool(output.get("active", False))).lower(),
    ))'
}
kittwm_remote_sway_tree_python() {
    python3 -c 'import json,sys
root = json.load(sys.stdin)
def walk(node):
    if not isinstance(node, dict):
        return
    if node.get("type") == "con" and (node.get("app_id") is not None or node.get("window") is not None):
        props = node.get("window_properties") or {}
        print("  {id} {app}  {name}".format(
            id=node.get("id") or 0,
            app=node.get("app_id") or props.get("class") or "?",
            name=node.get("name") or "",
        ))
    for key in ("nodes", "floating_nodes"):
        for child in node.get(key) or []:
            walk(child)
walk(root)'
}
if [ "${KITTWM_REMOTE_FORCE_FALLBACK:-0}" != "1" ] && command -v kittwm >/dev/null 2>&1; then
    kittwm_err=$(mktemp "${TMPDIR:-/tmp}/kittwm-remote-list.XXXXXX" 2>/dev/null || printf '')
    if [ -n "$kittwm_err" ]; then
        case "$kind" in
            displays) kittwm_out=$(kittwm --list-displays 2>"$kittwm_err") ;;
            *) kittwm_out=$(kittwm --list-windows 2>"$kittwm_err") ;;
        esac
        kittwm_status=$?
        if [ $kittwm_status -eq 0 ]; then
            rm -f "$kittwm_err"
            printf '%s\n' "$kittwm_out" | kittwm_remote_filter | kittwm_remote_emit kittwm kittwm
            exit 0
        fi
        cat "$kittwm_err" >&2
        rm -f "$kittwm_err"
    else
        case "$kind" in
            displays) kittwm_out=$(kittwm --list-displays 2>&1) ;;
            *) kittwm_out=$(kittwm --list-windows 2>&1) ;;
        esac
        kittwm_status=$?
        if [ $kittwm_status -eq 0 ]; then
            printf '%s\n' "$kittwm_out" | kittwm_remote_filter | kittwm_remote_emit kittwm kittwm
            exit 0
        fi
        printf '%s\n' "$kittwm_out" >&2
    fi
    printf 'WARN remote kittwm %s listing failed; falling back to platform discovery on target=%s host=%s\n' "$kind" "$command_host" "$host" >&2
fi
case "$kind" in
    displays)
        if [ "$json" != "1" ]; then
            printf 'kittwm remote displays\n======================\nhost: %s\ntarget host: %s\nmode: fallback\nforced fallback: %s\n' "$host" "$command_host" "$([ "${KITTWM_REMOTE_FORCE_FALLBACK:-0}" = "1" ] && printf true || printf false)"
            [ -z "$query" ] || printf 'filter: %s\n' "$query"
        fi
        if command -v swaymsg >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
            swaymsg -t get_outputs 2>/dev/null | jq -r '.[] | "  " + (.name // "?") + " " + (.make // "") + " " + (.model // "") + " " + ((.current_mode.width // 0)|tostring) + "x" + ((.current_mode.height // 0)|tostring) + " active=" + ((.active // false)|tostring)' | kittwm_remote_filter | kittwm_remote_emit fallback swaymsg-jq
        elif command -v swaymsg >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
            swaymsg -t get_outputs 2>/dev/null | kittwm_remote_sway_outputs_python | kittwm_remote_filter | kittwm_remote_emit fallback swaymsg-python3
        elif command -v xrandr >/dev/null 2>&1; then
            (xrandr --listmonitors 2>/dev/null || xrandr --query 2>/dev/null | awk '/ connected/{print "  "$0}') | kittwm_remote_filter | kittwm_remote_emit fallback xrandr
        elif command -v system_profiler >/dev/null 2>&1; then
            system_profiler SPDisplaysDataType 2>/dev/null | awk '/^[[:space:]]*(Resolution|Main Display|Online|Display Type):/{print "  "$0}' | kittwm_remote_filter | kittwm_remote_emit fallback system-profiler
        else
            printf '  capability unavailable: install remote kittwm, swaymsg+jq, swaymsg+python3, xrandr, or system_profiler\n' | kittwm_remote_emit fallback unavailable
        fi
        ;;
    *)
        if [ "$json" != "1" ]; then
            printf 'kittwm remote windows\n=====================\nhost: %s\ntarget host: %s\nmode: fallback\nforced fallback: %s\n' "$host" "$command_host" "$([ "${KITTWM_REMOTE_FORCE_FALLBACK:-0}" = "1" ] && printf true || printf false)"
            [ -z "$query" ] || printf 'filter: %s\n' "$query"
        fi
        if command -v swaymsg >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
            swaymsg -t get_tree 2>/dev/null | jq -r '.. | objects | select((.type? == "con") and ((.app_id? != null) or (.window? != null))) | "  " + ((.id // 0)|tostring) + " " + (.app_id // .window_properties.class // "?") + "  " + (.name // "")' | kittwm_remote_filter | kittwm_remote_emit fallback swaymsg-jq
        elif command -v swaymsg >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
            swaymsg -t get_tree 2>/dev/null | kittwm_remote_sway_tree_python | kittwm_remote_filter | kittwm_remote_emit fallback swaymsg-python3
        elif command -v wmctrl >/dev/null 2>&1; then
            (wmctrl -lx 2>/dev/null || wmctrl -l) | kittwm_remote_filter | kittwm_remote_emit fallback wmctrl
        elif command -v xdotool >/dev/null 2>&1; then
            xdotool search --onlyvisible --name '.*' 2>/dev/null | while IFS= read -r id; do
                class=$(xdotool getwindowclassname "$id" 2>/dev/null || printf '?')
                title=$(xdotool getwindowname "$id" 2>/dev/null || printf '')
                printf '  %s %s  %s\n' "$id" "$class" "$title"
            done | kittwm_remote_filter | kittwm_remote_emit fallback xdotool
        elif command -v osascript >/dev/null 2>&1; then
            osascript -e 'tell application "System Events" to repeat with p in (processes whose background only is false)' -e 'set pname to name of p' -e 'repeat with w in windows of p' -e 'try' -e 'set wname to name of w' -e 'if wname is not "" then log pname & "  " & wname' -e 'end try' -e 'end repeat' -e 'end repeat' 2>&1 | sed 's/^/  /' | kittwm_remote_filter | kittwm_remote_emit fallback osascript
        else
            printf '  capability unavailable: install remote kittwm, swaymsg+jq, swaymsg+python3, wmctrl, xdotool, or enable macOS osascript accessibility\n' | kittwm_remote_emit fallback unavailable
        fi
        ;;
esac
"#
}

fn apps_cmd(cli: &Cli) -> Result<()> {
    if let Some(host) = cli.remote_host.as_deref() {
        return remote_apps_cmd(cli, host);
    }
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
    let linux_apps = linux_desktop_apps(limit, query);
    if cli.apps_scene_json || cli.apps_kitty {
        let summary = AppsSummary {
            default_cmd: default_cmd.clone(),
            default_resolved: default_path.as_ref().map(|p| p.display().to_string()),
            filter: query.map(str::to_string),
            limit,
            path_commands: path_cmds.clone(),
            macos_apps: mac_apps.clone(),
        };
        let scene = apps_scene(&summary);
        return print_scene_or_kitty(
            &scene,
            cli.apps_kitty,
            kittwm_sdk::SurfacePlacementRole::Decoration,
        );
    }
    if cli.apps_first || cli.apps_launch_first {
        let Some(selected) = first_app_candidate(&path_cmds, &mac_apps, &linux_apps) else {
            if cli.json {
                println!(
                    "{}",
                    if cli.apps_launch_first {
                        app_launch_json_error(query, "no_candidates", "no app candidates matched")
                    } else {
                        app_first_json_error(query, "no_candidates", "no app candidates matched")
                    }
                );
                return Ok(());
            }
            return Err(anyhow!("no app candidates matched"));
        };
        if cli.apps_launch_first {
            let (pid, method) = launch_app_candidate(&selected)?;
            if cli.json {
                println!("{}", app_launch_json(query, &selected, pid, method));
            } else {
                println!(
                    "kittwm apps: launched pid={} kind={} method={} name={}",
                    pid,
                    selected.kind,
                    method,
                    selected.display_name()
                );
            }
        } else if cli.json {
            println!("{}", app_first_json(query, &selected));
        } else {
            println!("{}:{}", selected.kind, selected.display_name());
        }
        return Ok(());
    }
    if cli.json {
        let mut out = String::with_capacity(
            default_cmd
                .len()
                .saturating_add(path_cmds.iter().map(String::len).sum::<usize>())
                .saturating_add(mac_apps.iter().map(String::len).sum::<usize>())
                .saturating_add(128),
        );
        let _ = write!(
            out,
            "{{\"mode\": \"shell-path-macos-linux-desktop\", \"default_command\": {default_cmd:?}, \"default_resolved\": "
        );
        if let Some(path) = default_path.as_ref() {
            let _ = write!(out, "{:?}", path.display().to_string());
        } else {
            out.push_str("null");
        }
        let path_count = path_cmds.len();
        let macos_count = mac_apps.len();
        let linux_desktop_count = linux_apps.len();
        let total_count = path_count
            .saturating_add(macos_count)
            .saturating_add(linux_desktop_count);
        let linux_desktop_ids: Vec<String> = linux_apps.iter().map(|app| app.id.clone()).collect();
        let linux_desktop_files: Vec<String> =
            linux_apps.iter().map(|app| app.file.clone()).collect();
        let linux_desktop_labels: Vec<String> =
            linux_apps.iter().map(|app| app.label.clone()).collect();
        let linux_desktop_localized_names: Vec<String> = linux_apps
            .iter()
            .map(|app| app.localized_names.clone())
            .collect();
        let linux_desktop_generic_names: Vec<String> = linux_apps
            .iter()
            .map(|app| app.generic_name.clone())
            .collect();
        let linux_desktop_keywords: Vec<String> =
            linux_apps.iter().map(|app| app.keywords.clone()).collect();
        let linux_desktop_categories: Vec<String> = linux_apps
            .iter()
            .map(|app| app.categories.clone())
            .collect();
        let linux_desktop_comments: Vec<String> =
            linux_apps.iter().map(|app| app.comment.clone()).collect();
        let _ = write!(
            out,
            ", \"filter\": {}, \"limit\": {limit}, \"path_commands\": [{}], \"macos_apps\": [{}], \"linux_desktop_ids\": [{}], \"linux_desktop_files\": [{}], \"linux_desktop_apps\": [{}], \"linux_desktop_localized_names\": [{}], \"linux_desktop_generic_names\": [{}], \"linux_desktop_keywords\": [{}], \"linux_desktop_categories\": [{}], \"linux_desktop_comments\": [{}], \"path_count\": {path_count}, \"macos_count\": {macos_count}, \"linux_desktop_count\": {linux_desktop_count}, \"total_count\": {total_count}}}",
            json_option_string(query),
            json_string_array(&path_cmds),
            json_string_array(&mac_apps),
            json_string_array(&linux_desktop_ids),
            json_string_array(&linux_desktop_files),
            json_string_array(&linux_desktop_labels),
            json_string_array(&linux_desktop_localized_names),
            json_string_array(&linux_desktop_generic_names),
            json_string_array(&linux_desktop_keywords),
            json_string_array(&linux_desktop_categories),
            json_string_array(&linux_desktop_comments),
        );
        println!("{out}");
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
    if !linux_apps.is_empty() {
        println!();
        println!("Linux desktop entries (first {limit}):");
        for app in &linux_apps {
            println!("  {}", linux_desktop_app_row(app));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct AppsSummary {
    default_cmd: String,
    default_resolved: Option<String>,
    filter: Option<String>,
    limit: usize,
    path_commands: Vec<String>,
    macos_apps: Vec<String>,
}

fn apps_scene(summary: &AppsSummary) -> Scene {
    let cols = info_scene_cols();
    let rows = apps_scene_rows(summary.path_commands.len(), summary.macos_apps.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let resolved = summary.default_resolved.as_deref().unwrap_or("<not found>");
    let filter = summary.filter.as_deref().unwrap_or("<none>");
    let filter_label = truncate(filter, 32);
    let default_label = truncate(&summary.default_cmd, 48);
    let resolved_label = truncate(resolved, 48);
    let mut layers = vec![
        Layer {
            label: Some(apps_scene_backdrop_label(
                summary.path_commands.len(),
                summary.macos_apps.len(),
                summary.limit,
                &filter_label,
                &default_label,
                &resolved_label,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(163, 190, 140, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-apps-heading:launcher-candidates".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    let mut row = 2usize;
    for cmd in summary.path_commands.iter().take(16) {
        let y = row as f32 * cell.height_px as f32;
        let cmd_label = truncate(cmd, 48);
        layers.push(Layer {
            label: Some(apps_scene_row_label("path", &cmd_label)),
            root: Node::Rect {
                rect: apps_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(163, 190, 140, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
        row += 1;
    }
    for app in summary.macos_apps.iter().take(8) {
        let y = row as f32 * cell.height_px as f32;
        let app_label = truncate(app, 48);
        layers.push(Layer {
            label: Some(apps_scene_row_label("macos", &app_label)),
            root: Node::Rect {
                rect: apps_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(235, 203, 139, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
        row += 1;
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn apps_scene_backdrop_label(
    path_count: usize,
    macos_count: usize,
    limit: usize,
    filter_label: &str,
    default_label: &str,
    resolved_label: &str,
) -> String {
    let mut out = String::with_capacity(
        "kittwm-apps-backdrop:path_count=:macos_count=:limit=:filter=:default=:resolved="
            .len()
            .saturating_add(filter_label.len())
            .saturating_add(default_label.len())
            .saturating_add(resolved_label.len())
            .saturating_add(60),
    );
    out.push_str("kittwm-apps-backdrop:path_count=");
    let _ = write!(out, "{path_count}");
    out.push_str(":macos_count=");
    let _ = write!(out, "{macos_count}");
    out.push_str(":limit=");
    let _ = write!(out, "{limit}");
    out.push_str(":filter=");
    out.push_str(filter_label);
    out.push_str(":default=");
    out.push_str(default_label);
    out.push_str(":resolved=");
    out.push_str(resolved_label);
    out
}

fn apps_scene_row_label(kind: &str, label: &str) -> String {
    let mut out = String::with_capacity(
        "kittwm-app-row::"
            .len()
            .saturating_add(kind.len())
            .saturating_add(label.len()),
    );
    out.push_str("kittwm-app-row:");
    out.push_str(kind);
    out.push(':');
    out.push_str(label);
    out
}

fn apps_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn apps_scene_rows(path_count: usize, macos_count: usize) -> u16 {
    let total = path_count
        .saturating_add(macos_count)
        .min(u16::MAX as usize) as u16;
    total.saturating_add(7).clamp(8, 30)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxDesktopApp {
    id: String,
    label: String,
    file: String,
    exec: String,
    localized_names: String,
    generic_name: String,
    keywords: String,
    categories: String,
    comment: String,
}

#[cfg(target_os = "linux")]
fn linux_desktop_apps(limit: usize, query: Option<&str>) -> Vec<LinuxDesktopApp> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for root in linux_desktop_roots() {
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                    continue;
                }
                let Ok(contents) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let id = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("app.desktop")
                    .to_string();
                let file = path.display().to_string();
                let Some(app) = parse_linux_desktop_app(&id, &file, &contents) else {
                    continue;
                };
                if !linux_desktop_app_matches(&app, query) || !seen.insert(app.id.clone()) {
                    continue;
                }
                out.push(app);
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn linux_desktop_apps(_limit: usize, _query: Option<&str>) -> Vec<LinuxDesktopApp> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn linux_desktop_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(std::path::PathBuf::from(home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        roots.push(std::path::PathBuf::from(home).join(".local/share/applications"));
    }
    let dirs = std::env::var_os("XDG_DATA_DIRS")
        .map(|dirs| dirs.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    for dir in dirs.split(':').filter(|dir| !dir.is_empty()) {
        roots.push(std::path::PathBuf::from(dir).join("applications"));
    }
    roots
}

fn join_desktop_metadata(primary: Option<String>, localized: Vec<String>) -> String {
    let mut values = Vec::new();
    if let Some(primary) = primary.filter(|value| !value.is_empty()) {
        values.push(primary);
    }
    for value in localized {
        if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
            values.push(value);
        }
    }
    values.join(";")
}

fn desktop_environment_matches(field_values: &str) -> bool {
    let current = [
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        std::env::var("DESKTOP_SESSION").unwrap_or_default(),
    ];
    field_values
        .split(';')
        .filter(|value| !value.is_empty())
        .any(|field| {
            current.iter().any(|value| {
                value
                    .split([';', ':'])
                    .filter(|value| !value.is_empty())
                    .any(|value| value.eq_ignore_ascii_case(field))
            })
        })
}

fn desktop_try_exec_token(value: &str) -> Option<String> {
    let mut chars = value.trim_start().chars().peekable();
    let quote = matches!(chars.peek(), Some('\'' | '"')).then(|| chars.next().unwrap());
    let mut token = String::new();
    while let Some(ch) = chars.next() {
        if Some(ch) == quote || quote.is_none() && ch.is_whitespace() {
            break;
        }
        if ch == '\\' {
            if let Some(next) = chars.next() {
                token.push(next);
            }
        } else {
            token.push(ch);
        }
    }
    (!token.is_empty()).then_some(token)
}

fn desktop_try_exec_available(value: &str) -> bool {
    let Some(token) = desktop_try_exec_token(value) else {
        return false;
    };
    let path = std::path::Path::new(&token);
    if path.components().count() > 1 || path.is_absolute() {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(&token);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_linux_desktop_app(id: &str, file: &str, contents: &str) -> Option<LinuxDesktopApp> {
    let mut in_desktop_entry = false;
    let mut entry_seen = false;
    let mut ty = None::<String>;
    let mut name = None::<String>;
    let mut localized_names = Vec::<String>::new();
    let mut generic_name = None::<String>;
    let mut localized_generic_names = Vec::<String>::new();
    let mut comment = None::<String>;
    let mut localized_comments = Vec::<String>::new();
    let mut keywords = None::<String>;
    let mut localized_keywords = Vec::<String>::new();
    let mut categories = None::<String>;
    let mut exec = None::<String>;
    let mut only_show_in = None::<String>;
    let mut not_show_in = None::<String>;
    let mut try_exec = None::<String>;
    let mut hidden = false;
    let mut no_display = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            entry_seen |= in_desktop_entry;
            continue;
        }
        if entry_seen && !in_desktop_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "Type" => ty = Some(value.to_string()),
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "GenericName" => generic_name = Some(value.to_string()),
            "Comment" => comment = Some(value.to_string()),
            "Keywords" => keywords = Some(value.to_string()),
            "Categories" => categories = Some(value.to_string()),
            "OnlyShowIn" => only_show_in = Some(value.to_string()),
            "NotShowIn" => not_show_in = Some(value.to_string()),
            "TryExec" => try_exec = Some(value.to_string()),
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
            localized if localized.starts_with("Name[") => localized_names.push(value.to_string()),
            localized if localized.starts_with("GenericName[") => {
                localized_generic_names.push(value.to_string())
            }
            localized if localized.starts_with("Comment[") => {
                localized_comments.push(value.to_string())
            }
            localized if localized.starts_with("Keywords[") => {
                localized_keywords.push(value.to_string())
            }
            _ => {}
        }
    }
    if hidden
        || no_display
        || ty.as_deref().is_some_and(|ty| ty != "Application")
        || exec.as_deref().unwrap_or("").is_empty()
        || only_show_in
            .as_deref()
            .is_some_and(|value| !desktop_environment_matches(value))
        || not_show_in
            .as_deref()
            .is_some_and(desktop_environment_matches)
        || try_exec
            .as_deref()
            .is_some_and(|value| !desktop_try_exec_available(value))
    {
        return None;
    }
    Some(LinuxDesktopApp {
        id: id.to_string(),
        label: name.unwrap_or_else(|| id.trim_end_matches(".desktop").to_string()),
        file: file.to_string(),
        exec: exec.unwrap_or_default(),
        localized_names: join_desktop_metadata(None, localized_names),
        generic_name: join_desktop_metadata(generic_name, localized_generic_names),
        keywords: join_desktop_metadata(keywords, localized_keywords),
        categories: categories.unwrap_or_default(),
        comment: join_desktop_metadata(comment, localized_comments),
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_desktop_app_matches(app: &LinuxDesktopApp, query: Option<&str>) -> bool {
    let Some(query) = query else {
        return true;
    };
    let query = query.to_ascii_lowercase();
    app.id.to_ascii_lowercase().contains(&query)
        || app.label.to_ascii_lowercase().contains(&query)
        || app.file.to_ascii_lowercase().contains(&query)
        || app.localized_names.to_ascii_lowercase().contains(&query)
        || app.generic_name.to_ascii_lowercase().contains(&query)
        || app.keywords.to_ascii_lowercase().contains(&query)
        || app.categories.to_ascii_lowercase().contains(&query)
        || app.comment.to_ascii_lowercase().contains(&query)
}

fn linux_desktop_app_row(app: &LinuxDesktopApp) -> String {
    let detail = linux_desktop_app_detail(app);
    let mut row = String::with_capacity(
        app.label
            .len()
            .saturating_add(app.id.len())
            .saturating_add(app.file.len())
            .saturating_add(detail.len())
            .saturating_add(" () —  — ".len()),
    );
    row.push_str(&app.label);
    row.push_str(" (");
    row.push_str(&app.id);
    row.push_str(") — ");
    row.push_str(&app.file);
    if !detail.is_empty() {
        row.push_str(" — ");
        row.push_str(&detail);
    }
    row
}

fn linux_desktop_app_detail(app: &LinuxDesktopApp) -> String {
    let mut detail = String::new();
    for value in [&app.localized_names, &app.generic_name, &app.categories] {
        if value.is_empty() {
            continue;
        }
        if !detail.is_empty() {
            detail.push_str("; ");
        }
        detail.push_str(value);
    }
    detail
}

fn json_option_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_string(), |value| format!("{value:?}"))
}

fn app_first_json(query: Option<&str>, candidate: &AppCandidate) -> String {
    format!(
        "{{\"mode\":\"first\",\"filter\":{},\"kind\":{:?},\"candidate\":{:?},\"name\":{:?},\"desktop_file\":{}}}",
        json_option_string(query),
        candidate.kind,
        candidate.name,
        candidate.display_name(),
        json_option_string(candidate.desktop_file.as_deref())
    )
}

fn app_first_json_error(query: Option<&str>, code: &str, message: &str) -> String {
    format!(
        "{{\"mode\":\"first\",\"filter\":{},\"error\":{:?},\"message\":{:?}}}",
        json_option_string(query),
        code,
        message
    )
}

fn app_default_launch_method(candidate: &AppCandidate) -> &'static str {
    match candidate.kind {
        "macos" => "open",
        "desktop" => "desktop",
        _ => "path",
    }
}

fn app_launch_json(
    query: Option<&str>,
    candidate: &AppCandidate,
    pid: u32,
    method: &str,
) -> String {
    format!(
        "{{\"mode\":\"launch-first\",\"filter\":{},\"kind\":{:?},\"method\":{:?},\"candidate\":{:?},\"name\":{:?},\"desktop_file\":{},\"pid\":{:?}}}",
        json_option_string(query),
        candidate.kind,
        method,
        candidate.name,
        candidate.display_name(),
        json_option_string(candidate.desktop_file.as_deref()),
        pid.to_string()
    )
}

fn app_launch_json_error(query: Option<&str>, code: &str, message: &str) -> String {
    format!(
        "{{\"mode\":\"launch-first\",\"filter\":{},\"error\":{:?},\"message\":{:?}}}",
        json_option_string(query),
        code,
        message
    )
}

fn json_string_array(items: &[String]) -> String {
    let capacity = items
        .iter()
        .map(|item| item.len().saturating_add(4))
        .sum::<usize>()
        .saturating_sub((items.is_empty() as usize).saturating_mul(2));
    let mut out = String::with_capacity(capacity);
    for item in items {
        if !out.is_empty() {
            out.push_str(", ");
        }
        let _ = write!(out, "{item:?}");
    }
    out
}

fn filter_candidates(items: Vec<String>, query: Option<&str>, limit: usize) -> Vec<String> {
    let Some(query) = query else {
        return items.into_iter().take(limit).collect();
    };
    let mut scored: Vec<(u8, String)> = items
        .into_iter()
        .filter_map(|item| candidate_match_score(&item, query).map(|score| (score, item)))
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| a_score.cmp(b_score).then_with(|| a.cmp(b)));
    scored
        .into_iter()
        .map(|(_, item)| item)
        .take(limit)
        .collect()
}

fn candidate_match_score(item: &str, query: &str) -> Option<u8> {
    if ascii_casefold_eq(item, query) {
        Some(0)
    } else if ascii_casefold_starts_with(item, query) {
        Some(1)
    } else if ascii_casefold_contains(item, query) {
        Some(2)
    } else {
        None
    }
}

fn ascii_casefold_eq(item: &str, query: &str) -> bool {
    item.len() == query.len() && ascii_casefold_starts_with(item, query)
}

fn ascii_casefold_starts_with(item: &str, query: &str) -> bool {
    let item = item.as_bytes();
    let query = query.as_bytes();
    item.len() >= query.len()
        && item
            .iter()
            .zip(query.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}

fn ascii_casefold_contains(item: &str, query: &str) -> bool {
    let item = item.as_bytes();
    let query = query.as_bytes();
    if query.is_empty() {
        return true;
    }
    item.len() >= query.len()
        && item.windows(query.len()).any(|window| {
            window
                .iter()
                .zip(query.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
        })
}

#[derive(Debug, Clone)]
struct AppCandidate {
    kind: &'static str,
    name: String,
    label: Option<String>,
    desktop_file: Option<String>,
    exec_line: Option<String>,
}

impl AppCandidate {
    fn path(name: impl Into<String>) -> Self {
        Self {
            kind: "path",
            name: name.into(),
            label: None,
            desktop_file: None,
            exec_line: None,
        }
    }

    fn macos(name: impl Into<String>) -> Self {
        Self {
            kind: "macos",
            name: name.into(),
            label: None,
            desktop_file: None,
            exec_line: None,
        }
    }

    fn desktop(app: &LinuxDesktopApp) -> Self {
        Self {
            kind: "desktop",
            name: app.id.clone(),
            label: Some(app.label.clone()),
            desktop_file: Some(app.file.clone()),
            exec_line: Some(app.exec.clone()),
        }
    }

    fn none() -> Self {
        Self {
            kind: "none",
            name: "<no matches>".to_string(),
            label: None,
            desktop_file: None,
            exec_line: None,
        }
    }

    fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

fn first_app_candidate(
    path_cmds: &[String],
    mac_apps: &[String],
    linux_apps: &[LinuxDesktopApp],
) -> Option<AppCandidate> {
    path_cmds
        .first()
        .map(|name| AppCandidate::path(name.clone()))
        .or_else(|| {
            mac_apps
                .first()
                .map(|name| AppCandidate::macos(name.clone()))
        })
        .or_else(|| linux_apps.first().map(AppCandidate::desktop))
}

fn launch_app_candidate(candidate: &AppCandidate) -> Result<(u32, &'static str)> {
    if candidate.kind == "desktop" {
        return launch_desktop_app_candidate(candidate);
    }
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
    Ok((child.id(), app_default_launch_method(candidate)))
}

fn launch_desktop_app_candidate(candidate: &AppCandidate) -> Result<(u32, &'static str)> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(anyhow!(
            "launch candidate desktop:{}: no graphical display is available; try: kittwm remote HOST graphical",
            candidate.name
        ));
    }
    if find_on_path("gtk-launch").is_some() {
        if let Ok(pid) = spawn_detached_command("gtk-launch", &[candidate.name.as_str()]) {
            return Ok((pid, "gtk-launch"));
        }
    }
    if let Some(file) = candidate.desktop_file.as_deref() {
        if find_on_path("gio").is_some() {
            if let Ok(pid) = spawn_detached_command("gio", &["launch", file]) {
                return Ok((pid, "gio"));
            }
        }
    }
    if let Some(exec_line) = candidate.exec_line.as_deref() {
        let desktop_exec = strip_desktop_exec_field_codes(exec_line);
        if !desktop_exec.trim().is_empty() {
            return spawn_shell_command(&desktop_exec).map(|pid| (pid, "desktop-exec"));
        }
    }
    Err(anyhow!(
        "launch candidate desktop:{}: gtk-launch/gio failed and no Linux desktop Exec fallback is available",
        candidate.name
    ))
}

fn spawn_detached_command(program: &str, args: &[&str]) -> Result<u32> {
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("launch {program}"))?;
    Ok(child.id())
}

fn spawn_shell_command(command: &str) -> Result<u32> {
    spawn_detached_command("sh", &["-lc", command])
}

fn strip_desktop_exec_field_codes(exec_line: &str) -> String {
    exec_line
        .split_whitespace()
        .filter(|part| !part.starts_with('%'))
        .map(|part| part.replace("%f", "").replace("%u", ""))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
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
            request: replace_spawn_request(&argv),
        })
    }
}

fn replace_spawn_request(argv: &[String]) -> String {
    let shell_words = argv_to_shell_words(argv);
    let mut request = String::with_capacity("SPAWN ".len().saturating_add(shell_words.len()));
    request.push_str("SPAWN ");
    request.push_str(&shell_words);
    request
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
    let capacity = args
        .iter()
        .map(|arg| arg.len().saturating_add(2))
        .sum::<usize>()
        .saturating_add(args.len().saturating_sub(1));
    let mut out = String::with_capacity(capacity);
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        if arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_/.:".contains(c))
        {
            out.push_str(arg);
        } else {
            out.push('\'');
            for ch in arg.chars() {
                if ch == '\'' {
                    out.push_str("'\\''");
                } else {
                    out.push(ch);
                }
            }
            out.push('\'');
        }
    }
    out
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
        .map(AppCandidate::path)
        .chain(mac_app_candidates.into_iter().map(AppCandidate::macos))
        .take(limit)
        .collect();
    if candidates.is_empty() {
        candidates.push(AppCandidate::none());
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
        let (pid, method) = launch_app_candidate(candidate)?;
        println!(
            "kittwm launcher: launched selection={} pid={} kind={} method={} name={}",
            selected,
            pid,
            candidate.kind,
            method,
            candidate.display_name()
        );
        return Ok(());
    }
    if cli.launcher_scene_json || cli.launcher_kitty {
        let scene = launcher_scene(query, selected_idx, &candidates);
        return print_scene_or_kitty(
            &scene,
            cli.launcher_kitty,
            kittwm_sdk::SurfacePlacementRole::Overlay,
        );
    }

    let width = 62usize;
    println!("┌{}┐", "─".repeat(width));
    println!("│{:^width$}│", "kittwm launcher", width = width);
    println!("├{}┤", "─".repeat(width));
    println!("│ query: {:<qwidth$}│", query, qwidth = width - 8);
    println!("├{}┤", "─".repeat(width));
    for (idx, cand) in candidates.iter().enumerate() {
        let selected = idx == selected_idx;
        let text = launcher_preview_row_text(idx + 1, cand, selected);
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

fn launcher_scene(query: &str, selected_idx: usize, candidates: &[AppCandidate]) -> Scene {
    let cols = info_scene_cols();
    let rows = launcher_scene_rows(candidates.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let selected = launcher_selected_label(candidates.get(selected_idx));
    let query_label = truncate(query, 48);
    let mut layers = vec![
        Layer {
            label: Some(launcher_backdrop_label(
                &query_label,
                selected_idx + 1,
                candidates.len(),
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some(launcher_heading_label(&selected)),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for (idx, candidate) in candidates.iter().take(18).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let selected = idx == selected_idx;
        let name_label = truncate(candidate.display_name(), 48);
        layers.push(Layer {
            label: Some(launcher_row_label(
                idx + 1,
                candidate.kind,
                &name_label,
                selected,
            )),
            root: Node::Rect {
                rect: launcher_scene_row_rect(width, y),
                fill: Paint::Solid {
                    color: if selected {
                        Rgba::rgba(235, 203, 139, 255)
                    } else {
                        Rgba::rgba(136, 192, 208, 255)
                    },
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn launcher_row_label(index: usize, kind: &str, name_label: &str, selected: bool) -> String {
    let mut label = String::with_capacity(
        "kittwm-launcher-row:::selected="
            .len()
            .saturating_add(kind.len())
            .saturating_add(name_label.len())
            .saturating_add(24),
    );
    label.push_str("kittwm-launcher-row:");
    let _ = write!(label, "{index}");
    label.push(':');
    label.push_str(kind);
    label.push(':');
    label.push_str(name_label);
    label.push_str(":selected=");
    label.push_str(if selected { "true" } else { "false" });
    label
}

fn launcher_heading_label(selected: &str) -> String {
    let mut label = String::with_capacity(
        "kittwm-launcher-heading:selected="
            .len()
            .saturating_add(selected.len()),
    );
    label.push_str("kittwm-launcher-heading:selected=");
    label.push_str(selected);
    label
}

fn launcher_backdrop_label(query_label: &str, selected: usize, count: usize) -> String {
    let mut label = String::with_capacity(
        "kittwm-launcher-backdrop:query=:selected=:count="
            .len()
            .saturating_add(query_label.len())
            .saturating_add(40),
    );
    label.push_str("kittwm-launcher-backdrop:query=");
    label.push_str(query_label);
    label.push_str(":selected=");
    let _ = write!(label, "{selected}");
    label.push_str(":count=");
    let _ = write!(label, "{count}");
    label
}

fn launcher_preview_row_text(index: usize, candidate: &AppCandidate, selected: bool) -> String {
    let mut text = String::with_capacity(
        " 00. [] "
            .len()
            .saturating_add(candidate.kind.len().max(5))
            .saturating_add(candidate.display_name().len()),
    );
    text.push_str(if selected { "▶" } else { " " });
    text.push(' ');
    let _ = write!(text, "{index:>2}");
    text.push_str(". [");
    let _ = write!(text, "{:<5}", candidate.kind);
    text.push_str("] ");
    text.push_str(candidate.display_name());
    text
}

fn launcher_selected_label(candidate: Option<&AppCandidate>) -> String {
    let Some(candidate) = candidate else {
        return "none:<none>".to_string();
    };
    let name = truncate(candidate.display_name(), 48);
    let mut out = String::with_capacity(
        candidate
            .kind
            .len()
            .saturating_add(name.len())
            .saturating_add(1),
    );
    out.push_str(candidate.kind);
    out.push(':');
    out.push_str(&name);
    out
}

fn launcher_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
}

fn launcher_scene_rows(candidate_count: usize) -> u16 {
    let count = candidate_count.min(u16::MAX as usize) as u16;
    count.saturating_add(5).clamp(8, 24)
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

fn native_browser_default_out_path(pid: u32) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity("/tmp/kittwm-native-browser-.png".len() + 10);
    out.push_str("/tmp/kittwm-native-browser-");
    let _ = write!(out, "{pid}");
    out.push_str(".png");
    out
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
        .unwrap_or_else(|| native_browser_default_out_path(std::process::id()));
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

#[derive(Clone, Debug)]
struct ConfigSummary {
    config_path: String,
    background_color: String,
    background_opacity: f32,
    background_effects: usize,
    colorscheme_name: String,
    colorscheme_fg: String,
    colorscheme_bg: String,
    colorscheme_colors: usize,
    terminal_backend: String,
    terminal_command: String,
    libghostty_theme: String,
    libghostty_background: String,
    libghostty_opacity: f32,
    libghostty_kitty_graphics: bool,
    hidpi_enabled: bool,
    cell_width_px: u32,
    cell_height_px: u32,
    tile_gap_px: u32,
    tile_gap_cols: u16,
    tile_gap_rows: u16,
    header_gap_px: u32,
    header_gap_rows: u16,
    footer_gap_px: u32,
    footer_gap_rows: u16,
    keymap_path: String,
    launch_cmd: String,
    launch_query: String,
    launcher_overlay: String,
    prefix: String,
    bindings: usize,
    duplicate_chords: usize,
    status: &'static str,
}

#[cfg(test)]
const KITTWM_BASE_CELL_WIDTH_PX: u32 = 8;
#[cfg(test)]
const KITTWM_BASE_CELL_HEIGHT_PX: u32 = 16;
#[cfg(test)]
const KITTWM_HIDPI_SCALE: u32 = 2;
#[cfg(test)]
const KITTWM_MAX_CELL_WIDTH_PX: u32 = 64;
#[cfg(test)]
const KITTWM_MAX_CELL_HEIGHT_PX: u32 = 128;

#[cfg(test)]
fn kittwm_hidpi_enabled_from_env() -> bool {
    !matches!(
        std::env::var("KITTWM_HIDPI")
            .unwrap_or_else(|_| "1".to_string())
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

#[cfg(test)]
fn kittwm_env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
fn kittwm_cell_px_from_env(key: &str, base: u32, max: u32, hidpi: bool) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            if hidpi {
                base.saturating_mul(KITTWM_HIDPI_SCALE)
            } else {
                base
            }
        })
        .clamp(1, max)
}

fn config_cmd(cli: &Cli) -> Result<()> {
    let summary = config_summary(cli)?;
    if cli.config_scene_json || cli.config_kitty {
        let scene = config_scene(&summary);
        return print_scene_or_kitty(
            &scene,
            cli.config_kitty,
            kittwm_sdk::SurfacePlacementRole::Decoration,
        );
    }
    println!("kittwm config");
    println!("============");
    println!("config_path            : {}", summary.config_path);
    println!("background.color       : {}", summary.background_color);
    println!("background.opacity     : {:.2}", summary.background_opacity);
    println!("background.effects     : {}", summary.background_effects);
    println!("colorscheme.name       : {}", summary.colorscheme_name);
    println!("colorscheme.fg         : {}", summary.colorscheme_fg);
    println!("colorscheme.bg         : {}", summary.colorscheme_bg);
    println!("colorscheme.colors     : {}", summary.colorscheme_colors);
    println!("terminal.backend       : {}", summary.terminal_backend);
    println!("terminal.command       : {}", summary.terminal_command);
    println!("libghostty.theme       : {}", summary.libghostty_theme);
    println!("libghostty.background  : {}", summary.libghostty_background);
    println!("libghostty.opacity     : {:.2}", summary.libghostty_opacity);
    println!(
        "libghostty.kitty_graphics: {}",
        summary.libghostty_kitty_graphics
    );
    println!("display.hidpi          : {}", summary.hidpi_enabled);
    println!("display.cell_width_px  : {}", summary.cell_width_px);
    println!("display.cell_height_px : {}", summary.cell_height_px);
    println!("display.tile_gap_px    : {}", summary.tile_gap_px);
    println!(
        "display.tile_gap_cells : {}x{}",
        summary.tile_gap_cols, summary.tile_gap_rows
    );
    println!("display.header_gap_px  : {}", summary.header_gap_px);
    println!("display.header_gap_rows: {}", summary.header_gap_rows);
    println!("display.footer_gap_px  : {}", summary.footer_gap_px);
    println!("display.footer_gap_rows: {}", summary.footer_gap_rows);
    println!("KITTUI_WM_KEYMAP       : {}", summary.keymap_path);
    println!("KITTUI_WM_LAUNCH_CMD   : {}", summary.launch_cmd);
    println!("KITTUI_WM_LAUNCH_QUERY : {}", summary.launch_query);
    println!("KITTUI_WM_LAUNCHER_OVERLAY: {}", summary.launcher_overlay);
    println!("prefix                 : {}", summary.prefix);
    println!("bindings               : {}", summary.bindings);
    println!("duplicate_chords       : {}", summary.duplicate_chords);
    println!("status                 : {}", summary.status);
    Ok(())
}

fn config_summary(cli: &Cli) -> Result<ConfigSummary> {
    let env_keymap_path = std::env::var("KITTUI_WM_KEYMAP").ok();
    let keymap_path = cli.keymap_path.clone().or(env_keymap_path);
    let keymap = if let Some(path) = &keymap_path {
        kittui_cli::keymap::Keymap::load(std::path::Path::new(path))?
    } else {
        kittui_cli::keymap::default_keymap()
    };
    let duplicate_chords = keymap_duplicate_count(&keymap);
    let kittwm_config = KittwmConfig::load_default()?;
    let display_tuning = kittui_cli::session::native_display_tuning();
    Ok(ConfigSummary {
        config_path: default_kittwm_config_path().display().to_string(),
        background_color: kittwm_config.background.color,
        background_opacity: kittwm_config.background.opacity,
        background_effects: kittwm_config.background.effects.len(),
        colorscheme_name: kittwm_config.colorscheme.name,
        colorscheme_fg: kittwm_config.colorscheme.fg,
        colorscheme_bg: kittwm_config.colorscheme.bg,
        colorscheme_colors: kittwm_config.colorscheme.colors.len(),
        terminal_backend: kittwm_config.terminal.backend,
        terminal_command: kittwm_config
            .terminal
            .command
            .unwrap_or_else(|| "<shell>".to_string()),
        libghostty_theme: kittwm_config.libghostty.theme,
        libghostty_background: kittwm_config.libghostty.background,
        libghostty_opacity: kittwm_config.libghostty.background_opacity,
        libghostty_kitty_graphics: kittwm_config.libghostty.kitty_graphics,
        hidpi_enabled: display_tuning.hidpi_enabled,
        cell_width_px: display_tuning.cell_width_px,
        cell_height_px: display_tuning.cell_height_px,
        tile_gap_px: display_tuning.tile_gap_px,
        tile_gap_cols: display_tuning.tile_gap_cols,
        tile_gap_rows: display_tuning.tile_gap_rows,
        header_gap_px: display_tuning.header_gap_px,
        header_gap_rows: display_tuning.header_gap_rows,
        footer_gap_px: display_tuning.footer_gap_px,
        footer_gap_rows: display_tuning.footer_gap_rows,
        keymap_path: keymap_path.unwrap_or_else(|| "<default>".to_string()),
        launch_cmd: std::env::var("KITTUI_WM_LAUNCH_CMD")
            .unwrap_or_else(|_| "<default: xterm>".to_string()),
        launch_query: std::env::var("KITTUI_WM_LAUNCH_QUERY")
            .unwrap_or_else(|_| "<unset>".to_string()),
        launcher_overlay: std::env::var("KITTUI_WM_LAUNCHER_OVERLAY")
            .unwrap_or_else(|_| "<unset>".to_string()),
        prefix: keymap
            .prefix
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<none>".to_string()),
        bindings: keymap.bindings.len(),
        duplicate_chords,
        status: if duplicate_chords == 0 {
            "ok"
        } else {
            "duplicate chords found"
        },
    })
}

fn config_scene(summary: &ConfigSummary) -> Scene {
    config_scene_for_cols(summary, info_scene_cols())
}

fn config_scene_row_label(
    idx: usize,
    summary: &ConfigSummary,
    config_path: &str,
    background_color: &str,
    colorscheme_name: &str,
    colorscheme_fg: &str,
    colorscheme_bg: &str,
    terminal_backend: &str,
    libghostty_theme: &str,
    keymap_path: &str,
    launch_cmd: &str,
    launch_query: &str,
    launcher_overlay: &str,
    prefix: &str,
    status: &str,
) -> String {
    let mut out = String::with_capacity(72);
    let _ = write!(out, "kittwm-config-row:{idx}:");
    match idx {
        0 => {
            out.push_str("config_path=");
            out.push_str(config_path);
        }
        1 => {
            out.push_str("background.color=");
            out.push_str(background_color);
        }
        2 => {
            let _ = write!(out, "background.opacity={:.2}", summary.background_opacity);
        }
        3 => {
            let _ = write!(out, "background.effects={}", summary.background_effects);
        }
        4 => {
            out.push_str("colorscheme.name=");
            out.push_str(colorscheme_name);
        }
        5 => {
            out.push_str("colorscheme.fg=");
            out.push_str(colorscheme_fg);
        }
        6 => {
            out.push_str("colorscheme.bg=");
            out.push_str(colorscheme_bg);
        }
        7 => {
            let _ = write!(out, "colorscheme.colors={}", summary.colorscheme_colors);
        }
        8 => {
            out.push_str("terminal.backend=");
            out.push_str(terminal_backend);
        }
        9 => {
            out.push_str("libghostty.theme=");
            out.push_str(libghostty_theme);
        }
        10 => {
            let _ = write!(out, "libghostty.opacity={:.2}", summary.libghostty_opacity);
        }
        11 => {
            let _ = write!(out, "display.hidpi={}", summary.hidpi_enabled);
        }
        12 => {
            let _ = write!(
                out,
                "display.cell_px={}x{}",
                summary.cell_width_px, summary.cell_height_px
            );
        }
        13 => {
            let _ = write!(
                out,
                "display.tile_gap={}px={}x{}cells",
                summary.tile_gap_px, summary.tile_gap_cols, summary.tile_gap_rows
            );
        }
        14 => {
            let _ = write!(
                out,
                "display.header_gap={}px={}rows",
                summary.header_gap_px, summary.header_gap_rows
            );
        }
        15 => {
            let _ = write!(
                out,
                "display.footer_gap={}px={}rows",
                summary.footer_gap_px, summary.footer_gap_rows
            );
        }
        16 => {
            out.push_str("keymap=");
            out.push_str(keymap_path);
        }
        17 => {
            out.push_str("launch_cmd=");
            out.push_str(launch_cmd);
        }
        18 => {
            out.push_str("launch_query=");
            out.push_str(launch_query);
        }
        19 => {
            out.push_str("launcher_overlay=");
            out.push_str(launcher_overlay);
        }
        20 => {
            out.push_str("prefix=");
            out.push_str(prefix);
        }
        21 => {
            let _ = write!(out, "bindings={}", summary.bindings);
        }
        22 => {
            let _ = write!(out, "duplicates={}", summary.duplicate_chords);
        }
        23 => {
            out.push_str("status=");
            out.push_str(status);
        }
        _ => out.push_str("unknown=-"),
    }
    out
}

fn config_scene_backdrop_label(
    keymap_path: &str,
    bindings: usize,
    duplicate_chords: usize,
    status: &str,
) -> String {
    let mut out = String::with_capacity(
        "kittwm-config-backdrop:keymap=:bindings=:duplicates=:status=".len()
            + keymap_path.len()
            + status.len()
            + 40,
    );
    out.push_str("kittwm-config-backdrop:keymap=");
    out.push_str(keymap_path);
    out.push_str(":bindings=");
    let _ = write!(out, "{bindings}");
    out.push_str(":duplicates=");
    let _ = write!(out, "{duplicate_chords}");
    out.push_str(":status=");
    out.push_str(status);
    out
}

fn config_scene_for_cols(summary: &ConfigSummary, cols: u16) -> Scene {
    let rows = 30;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let config_path = truncate(&summary.config_path, 48);
    let background_color = truncate(&summary.background_color, 32);
    let colorscheme_name = truncate(&summary.colorscheme_name, 32);
    let colorscheme_fg = truncate(&summary.colorscheme_fg, 32);
    let colorscheme_bg = truncate(&summary.colorscheme_bg, 32);
    let terminal_backend = truncate(&summary.terminal_backend, 32);
    let libghostty_theme = truncate(&summary.libghostty_theme, 32);
    let keymap_path = truncate(&summary.keymap_path, 48);
    let launch_cmd = truncate(&summary.launch_cmd, 48);
    let launch_query = truncate(&summary.launch_query, 48);
    let launcher_overlay = truncate(&summary.launcher_overlay, 48);
    let prefix = truncate(&summary.prefix, 32);
    let status = truncate(summary.status, 32);
    let mut layers = vec![
        Layer {
            label: Some(config_scene_backdrop_label(
                &keymap_path,
                summary.bindings,
                summary.duplicate_chords,
                &status,
            )),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, height),
                fill: Paint::Solid {
                    color: Rgba::rgba(7, 17, 31, 238),
                },
                stroke: Some(Stroke::inside(
                    1.5,
                    Paint::Solid {
                        color: Rgba::rgba(136, 192, 208, 255),
                    },
                )),
                corners: Corners::uniform(8.0),
            },
        },
        Layer {
            label: Some("kittwm-config-heading:readiness".to_string()),
            root: Node::Rect {
                rect: KittuiPxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
                fill: Paint::Solid {
                    color: Rgba::rgba(94, 129, 172, 210),
                },
                stroke: None,
                corners: Corners {
                    tl: 8.0,
                    tr: 8.0,
                    bl: 0.0,
                    br: 0.0,
                },
            },
        },
    ];
    for idx in 0..24 {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(config_scene_row_label(
                idx,
                summary,
                &config_path,
                &background_color,
                &colorscheme_name,
                &colorscheme_fg,
                &colorscheme_bg,
                &terminal_backend,
                &libghostty_theme,
                &keymap_path,
                &launch_cmd,
                &launch_query,
                &launcher_overlay,
                &prefix,
                &status,
            )),
            root: Node::Rect {
                rect: info_indicator_rect(width, y),
                fill: Paint::Solid {
                    color: Rgba::rgba(136, 192, 208, 255),
                },
                stroke: None,
                corners: Corners::uniform(1.0),
            },
        });
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn keymap_duplicate_count(km: &kittui_cli::keymap::Keymap) -> usize {
    let mut seen = std::collections::HashMap::<&[kittui_cli::keymap::KeySpec], usize>::new();
    for binding in &km.bindings {
        *seen.entry(binding.chord.as_slice()).or_default() += 1;
    }
    seen.values().filter(|&&n| n > 1).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn config_display_tuning_helpers_default_hidpi_and_respect_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        for key in [
            "KITTWM_HIDPI",
            "KITTWM_NATIVE_CELL_WIDTH_PX",
            "KITTWM_NATIVE_CELL_HEIGHT_PX",
            "KITTWM_TILE_GAP_PX",
        ] {
            std::env::remove_var(key);
        }
        assert!(kittwm_hidpi_enabled_from_env());
        assert_eq!(
            kittwm_cell_px_from_env(
                "KITTWM_NATIVE_CELL_WIDTH_PX",
                KITTWM_BASE_CELL_WIDTH_PX,
                KITTWM_MAX_CELL_WIDTH_PX,
                true,
            ),
            16
        );
        std::env::set_var("KITTWM_HIDPI", "0");
        assert!(!kittwm_hidpi_enabled_from_env());
        std::env::set_var("KITTWM_NATIVE_CELL_WIDTH_PX", "24");
        std::env::set_var("KITTWM_NATIVE_CELL_HEIGHT_PX", "48");
        std::env::set_var("KITTWM_TILE_GAP_PX", "18");
        assert_eq!(
            kittwm_cell_px_from_env(
                "KITTWM_NATIVE_CELL_WIDTH_PX",
                KITTWM_BASE_CELL_WIDTH_PX,
                KITTWM_MAX_CELL_WIDTH_PX,
                false,
            ),
            24
        );
        assert_eq!(
            kittwm_cell_px_from_env(
                "KITTWM_NATIVE_CELL_HEIGHT_PX",
                KITTWM_BASE_CELL_HEIGHT_PX,
                KITTWM_MAX_CELL_HEIGHT_PX,
                false,
            ),
            48
        );
        assert_eq!(kittwm_env_u32("KITTWM_TILE_GAP_PX", 0), 18);
        for key in [
            "KITTWM_HIDPI",
            "KITTWM_NATIVE_CELL_WIDTH_PX",
            "KITTWM_NATIVE_CELL_HEIGHT_PX",
            "KITTWM_TILE_GAP_PX",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn broken_pipe_panic_payload_is_detected() {
        assert!(panic_payload_is_broken_pipe(
            &"failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(panic_payload_is_broken_pipe(
            &"failed printing to stdout: os error 32".to_string()
        ));
        assert!(!panic_payload_is_broken_pipe(&"unrelated panic"));
    }

    #[test]
    fn kittwm_log_clock_line_builds_directly() {
        assert_eq!(kittwm_log_clock_line(7, 3), "7.003");
        assert_eq!(kittwm_log_clock_line(123, 456), "123.456");
        assert!(kittwm_log_clock_line(123, 456).capacity() >= "123.456".len());
    }

    #[test]
    fn fatal_error_log_line_builds_directly() {
        let err = anyhow!("synthetic failure");
        let line = fatal_error_log_line(&err);
        assert_eq!(line, "fatal error: synthetic failure");
        assert!(line.capacity() >= line.len());
    }

    #[test]
    fn panic_log_line_builds_directly() {
        let line = panic_log_line("synthetic crash");
        assert_eq!(line, "panic: synthetic crash");
        assert_eq!(line.capacity(), line.len());
    }

    #[test]
    fn panic_payload_message_reports_string_and_non_string_payloads() {
        assert_eq!(panic_payload_message(&"boom"), "boom");
        assert_eq!(
            panic_payload_message(&"owned boom".to_string()),
            "owned boom"
        );
        assert_eq!(panic_payload_message(&42usize), "non-string panic payload");
    }

    fn process_event_log_test_filename(pid: u32, clock: &str) -> String {
        let clock = clock.replace('.', "-");
        let mut name = String::with_capacity("kittwm-process-event--.log".len() + 10 + clock.len());
        name.push_str("kittwm-process-event-");
        let _ = write!(name, "{pid}");
        name.push('-');
        name.push_str(&clock);
        name.push_str(".log");
        name
    }

    #[test]
    fn process_event_log_test_filename_builds_directly() {
        let name = process_event_log_test_filename(123, "456.789");
        assert_eq!(name, "kittwm-process-event-123-456-789.log");
        assert!(name.capacity() >= name.len());
    }

    #[test]
    fn kittwm_process_event_log_appends_timestamped_line() {
        let _guard = ENV_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(process_event_log_test_filename(
            std::process::id(),
            &kittwm_log_clock(),
        ));
        std::env::set_var("KITTUI_WM_LOG", &path);
        log_kittwm_process_event("panic: synthetic crash");
        let body = std::fs::read_to_string(&path).expect("read synthetic kittwm log");
        assert!(body.contains("] panic: synthetic crash"));
        let _ = std::fs::remove_file(&path);
        std::env::remove_var("KITTUI_WM_LOG");
    }

    #[test]
    fn truncate_uses_bounded_prefix_for_huge_fields() {
        let huge = "window-title-".repeat(10_000);
        let clipped = truncate(&huge, 12);
        assert_eq!(clipped, "window-titl…");
        assert_eq!(clipped.chars().count(), 12);
        assert!(clipped.capacity() >= 12);
        let short = truncate("short", 12);
        assert_eq!(short, "short");
        assert!(short.capacity() >= 12);
        assert_eq!(truncate("anything", 1), "…");
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn kittwm_scene_placement_options_are_absolute_no_placeholder() {
        let decoration =
            kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Decoration);
        assert!(!decoration.unicode_placeholder);
        assert_eq!(
            decoration.z_index,
            kittwm_z_index(kittwm_sdk::SurfacePlacementRole::Decoration)
        );
        let overlay = kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Overlay);
        assert!(!overlay.unicode_placeholder);
        assert_eq!(
            overlay.z_index,
            kittwm_z_index(kittwm_sdk::SurfacePlacementRole::Overlay)
        );
    }

    #[test]
    fn info_scene_rows_saturate_before_clamping() {
        assert_eq!(info_scene_rows(0), 5);
        assert_eq!(info_scene_rows(8), 13);
        assert_eq!(info_scene_rows(u64::MAX), 18);
    }

    #[test]
    fn native_surfaces_scene_rows_saturate_before_clamping() {
        assert_eq!(native_surfaces_scene_rows(0), 8);
        assert_eq!(native_surfaces_scene_rows(12), 17);
        assert_eq!(native_surfaces_scene_rows(usize::MAX), 22);
    }

    #[test]
    fn daily_help_row_label_builds_directly() {
        let label = daily_help_row_label("examples", 2, "kittwm spawn htop");
        assert_eq!(label, "kittwm-daily-help-row:examples:2:kittwm spawn htop");
        assert_eq!(
            label.capacity(),
            "kittwm-daily-help-row::".len() + "examples".len() + 20 + "kittwm spawn htop".len()
        );
    }

    #[test]
    fn daily_help_heading_label_builds_directly() {
        let label = daily_help_heading_label("quickstart", "kittwm quickstart");
        assert_eq!(
            label,
            "kittwm-daily-help-heading:quickstart:kittwm quickstart"
        );
        assert_eq!(
            label.capacity(),
            "kittwm-daily-help-heading::".len() + "quickstart".len() + "kittwm quickstart".len()
        );
    }

    #[test]
    fn daily_help_backdrop_label_builds_directly() {
        let label = daily_help_backdrop_label("quickstart", 12, 5);
        assert_eq!(
            label,
            "kittwm-daily-help-backdrop:quickstart:lines=12:commands=5"
        );
        assert_eq!(
            label.capacity(),
            "kittwm-daily-help-backdrop::lines=:commands=".len() + "quickstart".len() + 40
        );
    }

    #[test]
    fn daily_help_scene_rows_saturate_before_clamping() {
        assert_eq!(daily_help_scene_rows(0), 8);
        assert_eq!(daily_help_scene_rows(20), 24);
        assert_eq!(daily_help_scene_rows(usize::MAX), 30);
    }

    #[test]
    fn commands_scene_rows_saturate_before_clamping() {
        assert_eq!(commands_scene_rows(0), 8);
        assert_eq!(commands_scene_rows(20), 25);
        assert_eq!(commands_scene_rows(usize::MAX), 28);
    }

    #[test]
    fn keymap_scene_rows_saturate_before_clamping() {
        assert_eq!(keymap_scene_rows(0), 8);
        assert_eq!(keymap_scene_rows(20), 25);
        assert_eq!(keymap_scene_rows(usize::MAX), 28);
    }

    #[test]
    fn launcher_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = launcher_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn apps_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = apps_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn daily_help_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = daily_help_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn session_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = session_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn architecture_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = architecture_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn commands_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = commands_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn keymap_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = keymap_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn shortcuts_scene_row_rect_fits_tiny_widths() {
        for width in [0.0_f32, 1.0, 8.0, 40.0] {
            let rect = shortcuts_scene_row_rect(width, 2.0);
            assert!(rect.origin.0 >= 0.0, "{rect:?}");
            assert!(
                rect.origin.0 + rect.width <= width.max(1.0),
                "width={width} rect={rect:?}"
            );
        }
    }

    #[test]
    fn doctor_scene_cols_respects_narrow_positive_widths() {
        assert_eq!(doctor_scene_cols_from_sources(Some("1"), None), 1);
        assert_eq!(doctor_scene_cols_from_sources(Some("8"), None), 8);
        assert_eq!(doctor_scene_cols_from_sources(Some("31"), None), 31);
        assert_eq!(doctor_scene_cols_from_sources(Some("0"), None), 64);
        assert_eq!(doctor_scene_cols_from_sources(Some("240"), None), 120);
        assert_eq!(doctor_scene_cols_from_sources(None, None), 64);
        assert_eq!(doctor_scene_cols_from_sources(None, Some(100)), 100);
        assert_eq!(doctor_scene_cols_from_sources(Some("0"), Some(100)), 100);
        assert_eq!(doctor_scene_cols_from_sources(None, Some(u16::MAX)), 120);
    }

    #[test]
    fn update_options_parse_status_check_json_and_paths() {
        let mut values = args(&[
            "--check",
            "--json",
            "--repository",
            "owner/repo",
            "--install-dir",
            "/tmp/kittwm-bin",
        ])
        .into_iter();
        let options = parse_update_options(&mut values).unwrap();
        assert_eq!(options.action, UpdateAction::Check);
        assert!(options.json);
        assert_eq!(options.repository.as_deref(), Some("owner/repo"));
        assert_eq!(
            options.install_dir.as_deref(),
            Some(std::path::Path::new("/tmp/kittwm-bin"))
        );

        let mut values = args(&["status"]).into_iter();
        assert_eq!(
            parse_update_options(&mut values).unwrap().action,
            UpdateAction::Status
        );
    }

    #[test]
    fn lifecycle_aliases_map_to_modes() {
        assert_eq!(lifecycle_alias_mode("start").unwrap(), Mode::Session);
        assert_eq!(lifecycle_alias_mode("stop").unwrap(), Mode::Kill);
        assert!(lifecycle_alias_mode("restart").is_err());
    }

    #[test]
    fn app_limit_errors_are_actionable() {
        assert_eq!(parse_limit_value("3").unwrap(), 3);
        let err = parse_limit_value("nope").unwrap_err().to_string();
        assert!(err.contains("--limit expects integer"), "{err}");
        assert!(err.contains("got \"nope\""), "{err}");
        assert!(err.contains("try: kittwm apps --limit 10"), "{err}");
        assert!(err.contains("help: kittwm help apps"), "{err}");
        let missing = missing_limit_error().to_string();
        assert!(missing.contains("--limit requires an integer"), "{missing}");
        assert!(missing.contains("try: kittwm apps --limit 10"), "{missing}");
    }

    #[test]
    fn app_filter_missing_query_error_is_actionable() {
        let err = missing_filter_error().to_string();
        assert!(err.contains("--filter requires a query"), "{err}");
        assert!(err.contains("try: kittwm apps --filter terminal"), "{err}");
        assert!(err.contains("help: kittwm help apps"), "{err}");
    }

    #[test]
    fn log_commands_parse_path_and_tail_follow() {
        assert_eq!(parse_log_command(&[]).unwrap(), LogCommand::Path);
        assert_eq!(
            parse_log_command(&args(&["path"])).unwrap(),
            LogCommand::Path
        );
        assert_eq!(
            parse_log_command(&args(&["tail"])).unwrap(),
            LogCommand::Tail { follow: false }
        );
        assert_eq!(
            parse_log_command(&args(&["tail", "-f"])).unwrap(),
            LogCommand::Tail { follow: true }
        );
        assert_eq!(
            parse_log_command(&args(&["tail", "--follow"])).unwrap(),
            LogCommand::Tail { follow: true }
        );
        let err = parse_log_command(&args(&["tail", "--bad"]))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("usage: kittwm log path | kittwm log tail [-f]"),
            "{err}"
        );
        assert!(err.contains("try: kittwm log tail -f"), "{err}");
        assert!(err.contains("help: kittwm help log"), "{err}");
    }

    #[test]
    fn log_help_mentions_default_path_and_follow() {
        let help = help_topic_text("log").unwrap();
        assert!(help.contains("/tmp/kittui-wm.log"), "{help}");
        assert!(help.contains("kittwm log tail -f"), "{help}");
    }

    #[test]
    fn log_help_mentions_custom_log_path_example() {
        let help = help_topic_text("log").unwrap();
        assert!(
            help.contains("KITTUI_WM_LOG=/tmp/demo.log kittwm"),
            "{help}"
        );
        assert!(help.contains("per-session log file"), "{help}");
    }

    #[test]
    fn unknown_command_errors_point_to_useful_help() {
        let err = friendly_unknown_command_error("pane").to_string();
        assert!(err.contains("unknown kittwm command"), "{err}");
        assert!(err.contains("Did you mean?"), "{err}");
        assert!(err.contains("kittwm panes"), "{err}");
        assert!(err.contains("kittwm quickstart"), "{err}");
        assert!(err.contains("kittwm examples"), "{err}");
        assert!(err.contains("kittwm cheat"), "{err}");
        assert!(err.contains("kittwm help topics"), "{err}");
    }

    #[test]
    fn extra_help_topic_errors_are_actionable() {
        let err = extra_help_topic_error("help", "extra").to_string();
        assert!(err.contains("accepts at most one topic"), "{err}");
        assert!(err.contains("got \"extra\""), "{err}");
        assert!(err.contains("try: kittwm help panes"), "{err}");
        assert!(err.contains("help: kittwm help topics"), "{err}");
    }

    #[test]
    fn unknown_help_topic_errors_point_to_topics() {
        let err = help_topic_text("panez").unwrap_err().to_string();
        assert!(err.capacity() >= err.len());
        assert!(err.contains("unknown kittwm help topic"), "{err}");
        assert!(err.contains("kittwm help panes"), "{err}");
        assert!(err.contains("kittwm help topics"), "{err}");
        assert!(err.contains("kittwm help input"), "{err}");
        assert!(err.contains("kittwm help inspect"), "{err}");
        assert!(err.contains("kittwm help log"), "{err}");
        assert!(err.contains("kittwm help completions"), "{err}");
    }

    #[test]
    fn help_topics_lists_log_topic() {
        let text = help_topic_text("topics").unwrap();
        assert!(text.contains("log      debug log path"), "{text}");
        assert!(known_help_topics().contains(&"log"));
    }

    #[test]
    fn help_topics_lists_ssh_topic() {
        let text = help_topic_text("topics").unwrap();
        assert!(text.contains("ssh      pooled SSH workflows"), "{text}");
        assert!(known_help_topics().contains(&"ssh"));
        assert!(known_kittwm_commands().contains(&"ssh"));
        assert!(known_kittwm_commands().contains(&"remote"));
    }

    #[test]
    fn help_topic_ssh_lists_remote_workflow() {
        let text = help_topic_text("ssh").unwrap();
        assert!(
            text.contains("If the remote has kittwm installed"),
            "{text}"
        );
        assert!(text.contains("kittwm remote HOST"), "{text}");
        assert!(text.contains("kittwm apps --remote HOST"), "{text}");
        assert!(text.contains("kittwm doctor --remote HOST"), "{text}");
        assert!(
            text.contains("kittwm remote HOST fallback launch"),
            "{text}"
        );
        assert!(text.contains("kittwm remote HOST fallback open"), "{text}");
        assert!(text.contains("kittwm remote HOST fallback run"), "{text}");
        assert!(text.contains("kittwm remote HOST fallback start"), "{text}");
        assert!(text.contains("kittwm remote HOST terminal"), "{text}");
        assert!(text.contains("kittwm remote HOST wm"), "{text}");
        assert!(text.contains("kittwm-terminal --remote HOST"), "{text}");
        assert!(text.contains("ControlMaster=auto"), "{text}");
    }

    #[test]
    fn help_topics_mentions_daily_guides() {
        let text = help_topic_text("topics").unwrap();
        assert!(text.contains("Daily guides:"), "{text}");
        assert!(text.contains("kittwm quickstart"), "{text}");
        assert!(text.contains("kittwm examples"), "{text}");
        assert!(text.contains("kittwm cheat"), "{text}");
    }

    #[test]
    fn help_topic_completions_lists_shell_examples() {
        let text = help_topic_text("completions").unwrap();
        assert!(text.contains("kittwm completions bash"), "{text}");
        assert!(text.contains("kittwm completions zsh"), "{text}");
        assert!(text.contains("kittwm completions fish"), "{text}");
        assert!(
            text.contains("kittwm completions bash >> ~/.bashrc"),
            "{text}"
        );
        assert!(
            text.contains("kittwm completions zsh >> ~/.zshrc"),
            "{text}"
        );
        assert!(
            text.contains("mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish"),
            "{text}"
        );
        assert!(known_help_topics().contains(&"completions"));
    }

    #[test]
    fn keymap_duplicate_action_writer_streams_comma_separated_labels() {
        let mut out = Vec::new();
        write_duplicate_action_labels(
            &mut out,
            "C-a t",
            &["float.toggle".to_string(), "terminal.launch".to_string()],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "  C-a t: float.toggle, terminal.launch\n"
        );
    }

    #[test]
    fn completions_missing_shell_error_is_actionable() {
        let err = missing_completion_shell_error().to_string();
        assert!(err.contains("requires a shell"), "{err}");
        assert!(err.contains("bash, zsh, or fish"), "{err}");
        assert!(err.contains("try: kittwm completions bash"), "{err}");
        assert!(err.contains("help: kittwm help completions"), "{err}");
    }

    #[test]
    fn completions_extra_shell_error_is_actionable() {
        let err = extra_completion_shell_error("extra").to_string();
        assert!(err.contains("accepts one shell"), "{err}");
        assert!(err.contains("got \"extra\""), "{err}");
        assert!(err.contains("expected bash, zsh, or fish"), "{err}");
        assert!(err.contains("try: kittwm completions bash"), "{err}");
        assert!(err.contains("help: kittwm help completions"), "{err}");
    }

    #[test]
    fn completions_include_daily_driver_aliases() {
        let bash = completions_text("bash").unwrap();
        assert!(bash.contains("complete -F _kittwm kittwm"), "{bash}");
        assert!(bash.contains("quickstart"), "{bash}");
        assert!(bash.contains("spawn"), "{bash}");
        assert!(bash.contains("nudge"), "{bash}");
        assert!(bash.contains("--nudge-pane"), "{bash}");
        assert!(bash.contains("--reset-pane-offset"), "{bash}");
        assert!(bash.contains("--reset-all-pane-offsets"), "{bash}");
        assert!(bash.contains("--reset-pane-weights"), "{bash}");
        assert!(bash.contains("--panes-json"), "{bash}");
        assert!(bash.contains("--remote"), "{bash}");
        assert!(bash.contains("--launch-first"), "{bash}");
        assert!(bash.contains("--gui"), "{bash}");
        assert!(bash.contains("--wayland"), "{bash}");
        assert!(bash.contains("kittwm"), "{bash}");
        assert!(bash.contains("desktop"), "{bash}");
        assert!(bash.contains("wm"), "{bash}");
        assert!(bash.contains("fallback"), "{bash}");
        assert!(bash.contains("select"), "{bash}");
        assert!(bash.contains("pick"), "{bash}");
        assert!(bash.contains("start"), "{bash}");
        assert!(bash.contains("login"), "{bash}");
        assert!(bash.contains("console"), "{bash}");
        assert!(bash.contains("tty"), "{bash}");
        assert!(bash.contains("x11"), "{bash}");
        assert!(bash.contains("gui"), "{bash}");
        assert!(bash.contains("graphical"), "{bash}");
        assert!(bash.contains("wayland"), "{bash}");
        assert!(bash.contains("forwarding"), "{bash}");
        assert!(bash.contains("forward"), "{bash}");
        assert!(bash.contains("--forward"), "{bash}");
        assert!(bash.contains("open"), "{bash}");
        assert!(bash.contains("run"), "{bash}");
        assert!(bash.contains("app"), "{bash}");
        assert!(bash.contains("term"), "{bash}");
        assert!(bash.contains("monitors"), "{bash}");
        assert!(bash.contains("screens"), "{bash}");
        assert!(bash.contains("win"), "{bash}");

        let zsh = completions_text("zsh").unwrap();
        assert!(zsh.contains("#compdef kittwm"), "{zsh}");
        assert!(zsh.contains("commands-json"), "{zsh}");
        assert!(zsh.contains("nudge"), "{zsh}");
        assert!(zsh.contains("reset-position"), "{zsh}");
        assert!(zsh.contains("--remote"), "{zsh}");
        assert!(zsh.contains("--gui"), "{zsh}");
        assert!(zsh.contains("--wayland"), "{zsh}");
        assert!(zsh.contains("desktop"), "{zsh}");
        assert!(zsh.contains("x11"), "{zsh}");
        assert!(zsh.contains("gui"), "{zsh}");
        assert!(zsh.contains("wayland"), "{zsh}");
        assert!(zsh.contains("forwarding"), "{zsh}");
        assert!(zsh.contains("forward"), "{zsh}");
        assert!(zsh.contains("--forward"), "{zsh}");
        assert!(zsh.contains("open"), "{zsh}");
        assert!(zsh.contains("run"), "{zsh}");
        assert!(zsh.contains("app"), "{zsh}");
        assert!(zsh.contains("term"), "{zsh}");
        assert!(zsh.contains("monitors"), "{zsh}");
        assert!(zsh.contains("screens"), "{zsh}");
        assert!(zsh.contains("win"), "{zsh}");

        let fish = completions_text("fish").unwrap();
        assert!(fish.contains("complete -c kittwm"), "{fish}");
        assert!(fish.contains("cheat"), "{fish}");
        assert!(fish.contains("nudge"), "{fish}");
        assert!(fish.contains("reset-position"), "{fish}");
        assert!(fish.contains("--remote"), "{fish}");
        assert!(fish.contains("--gui"), "{fish}");
        assert!(fish.contains("--wayland"), "{fish}");
        assert!(fish.contains("kittwm"), "{fish}");
        assert!(fish.contains("desktop"), "{fish}");
        assert!(fish.contains("x11"), "{fish}");
        assert!(fish.contains("gui"), "{fish}");
        assert!(fish.contains("wayland"), "{fish}");
        assert!(fish.contains("forwarding"), "{fish}");
        assert!(fish.contains("forward"), "{fish}");
        assert!(fish.contains("--forward"), "{fish}");
        assert!(fish.contains("open"), "{fish}");
        assert!(fish.contains("run"), "{fish}");
        assert!(fish.contains("app"), "{fish}");
        assert!(fish.contains("term"), "{fish}");
        assert!(fish.contains("monitors"), "{fish}");
        assert!(fish.contains("screens"), "{fish}");
        assert!(fish.contains("win"), "{fish}");
        assert_eq!(fish, fish_completions_text());
        assert_eq!(fish.capacity(), fish.len());
        assert!(std::ptr::eq(completion_words(), completion_words()));
        let err = completions_text("powershell").unwrap_err().to_string();
        assert!(err.contains("expected bash, zsh, or fish"), "{err}");
        assert!(err.contains("try: kittwm completions bash"), "{err}");
        assert!(err.contains("help: kittwm help completions"), "{err}");
    }

    #[test]
    fn commands_catalog_lists_daily_driver_aliases() {
        let text = commands_text();
        assert!(text.capacity() >= text.len());
        assert!(text.contains("kittwm commands"), "{text}");
        assert!(text.contains("LIFECYCLE"), "{text}");
        assert_eq!(text.matches("\nHELP\n").count(), 1, "{text}");
        assert_eq!(text.matches("\nDIAGNOSTICS\n").count(), 1, "{text}");
        assert!(
            text.contains("Daily workflows: kittwm examples | kittwm cheat | kittwm help topics"),
            "{text}"
        );
        assert!(text.contains("help topics"), "{text}");
        assert!(text.contains("help completions"), "{text}");
        assert!(text.contains("spawn CMD [ARGS...]"), "{text}");
        assert!(
            text.contains("split [WINDOW] columns|rows|grid CMD [ARGS...]"),
            "{text}"
        );
        assert!(text.contains("focus WINDOW"), "{text}");
        assert!(text.contains("doctor"), "{text}");

        let json_text = commands_json_text();
        assert!(json_text.ends_with('\n'));
        assert_eq!(json_text.matches('\n').count(), 1);
        assert!(json_text.capacity() >= json_text.len());
        let json: serde_json::Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(json["kind"], "kittwm-local-commands");
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "quickstart" && entry["category"] == "help" }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "quickstart-kitty" && entry["category"] == "help"
        }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "examples-kitty" && entry["category"] == "help" }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "cheat-kitty" && entry["category"] == "help" }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "split [WINDOW] columns|rows|grid CMD [ARGS...]"
                && entry["category"] == "action"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "wait [WINDOW] TEXT" && entry["category"] == "action"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "log tail [-f]" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "session-kitty" && entry["category"] == "session"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "help-kitty [topic]" && entry["category"] == "help"
        }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "status-kitty" && entry["category"] == "inspect" }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "chrome-kitty" && entry["category"] == "inspect" }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "apps-kitty" && entry["category"] == "apps" }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "remote HOST" && entry["category"] == "remote" }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST help" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST status" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST gui" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST forward" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST kittwm" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST desktop" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST wm" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list apps QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list windows" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list windows --json" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list windows --fallback"
                && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list win" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST win QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list displays --fallback"
                && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list monitors" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST monitors QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST list screens" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST screens QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST apps QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST apps QUERY --json" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST apps QUERY --fallback" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback apps QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback launch QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback open QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback run QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback start QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback windows QUERY"
                && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST fallback displays QUERY"
                && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST applications QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST programs QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST software QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST app QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST app QUERY --json" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST application QUERY --json"
                && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST program QUERY --json" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST select QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST pick QUERY --json" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST launch QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST launch QUERY --fallback"
                && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST launch QUERY --json" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST open QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST run QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST start QUERY" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "apps --remote HOST" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "doctor --remote HOST" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST terminal CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST term CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST cmd CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST command CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST exec CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST sh CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST login CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST console CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "remote HOST tty CMD" && entry["category"] == "remote"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "kittwm-terminal --remote HOST" && entry["category"] == "remote"
        }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "launcher-kitty" && entry["category"] == "apps" }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "architecture-json" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "architecture-kitty" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "native-surfaces" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "native-surfaces-json" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "native-surfaces-kitty" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "keymap-kitty" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "config-kitty" && entry["category"] == "diagnostics"
        }));
        assert!(json["commands"].as_array().unwrap().iter().any(|entry| {
            entry["command"] == "commands-scene-json" && entry["category"] == "help"
        }));
        assert!(json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["command"] == "commands-kitty" && entry["category"] == "help" }));
    }

    #[test]
    fn session_row_label_builds_directly() {
        let label = session_row_label(1, "native-2", "editor", "vim", "2", true);
        assert_eq!(
            label,
            "kittwm-session-row:1:window=native-2:title=editor:command=vim:weight=2:focused=true"
        );
        assert!(label.capacity() >= label.len());
    }

    fn synthetic_native_window_id(idx: usize) -> String {
        use std::fmt::Write as _;

        let mut id = String::with_capacity("native-".len() + 20);
        id.push_str("native-");
        let _ = write!(id, "{idx}");
        id
    }

    #[test]
    fn synthetic_native_window_id_builds_directly() {
        let id = synthetic_native_window_id(42);
        assert_eq!(id, "native-42");
        assert!(id.capacity() >= id.len());
    }

    #[test]
    fn session_scene_rows_saturate_large_manifest_counts() {
        assert_eq!(session_scene_rows(0), 8);
        assert_eq!(session_scene_rows(4), 9);
        assert_eq!(session_scene_rows(usize::MAX), 24);

        let panes = (0..128)
            .map(|idx| {
                serde_json::json!({
                    "index": idx,
                    "window": synthetic_native_window_id(idx),
                    "title": "shell",
                    "command": "bash",
                    "weight": 1,
                    "focused": false
                })
            })
            .collect::<Vec<_>>();
        let session = serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-native-session",
            "layout": "rows",
            "focus": "native-1",
            "panes": panes
        });
        let scene = session_scene_for_cols(&session, 80);
        assert_eq!(scene.footprint.rows, 24);
    }

    #[test]
    fn session_scene_rows_fit_narrow_width() {
        let session = serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-native-session",
            "layout": "rows",
            "focus": "native-1",
            "panes": [
                {"index":0,"window":"native-1","title":"shell","command":"bash","weight":1,"focused":true}
            ]
        });
        let scene = session_scene_for_cols(&session, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
    }

    #[test]
    fn session_backdrop_label_builds_directly() {
        let label = session_backdrop_label("kittwm-native-session", "1", "rows", "native-2", 2);
        assert_eq!(
            label,
            "kittwm-session-backdrop:kind=kittwm-native-session:schema=1:layout=rows:focus=native-2:panes=2"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn session_scene_labels_manifest_panes() {
        let session = serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-native-session",
            "layout": "rows",
            "focus": "native-2",
            "panes": [
                {"index":0,"window":"native-1","title":"shell","command":"bash","weight":1,"focused":false},
                {"index":1,"window":"native-2","title":"editor","command":"vim","weight":2,"focused":true}
            ]
        });
        let scene = session_scene(&session);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-session-backdrop:kind=kittwm-native-session:schema=1:layout=rows:focus=native-2:panes=2"
            )),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-session-row:1:window=native-2:title=editor:command=vim:weight=2:focused=true"
            )),
            "{labels:?}"
        );
    }

    #[test]
    fn session_scene_clips_pathological_label_fields() {
        let session = serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-native-session-with-a-pathologically-long-kind",
            "layout": "layout-name-that-is-pathologically-long",
            "focus": "native-window-with-a-pathologically-long-focus-id",
            "panes": [
                {
                    "index":0,
                    "window":"native-window-with-a-pathologically-long-window-id",
                    "title":"pane-title-that-is-pathologically-long-and-would-bloat-scene-labels",
                    "command":"command --with --a-pathologically-long-argument-list --that-bloats-labels",
                    "weight":1,
                    "focused":true
                }
            ]
        });
        let scene = session_scene_for_cols(&session, 8);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-session-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("kind=kittwm-native-session-with-a-pa…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("layout=layout-name-that-is-pathologica…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("focus=native-window-with-a-pathologic…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 170, "{backdrop}");
        let row = labels
            .iter()
            .find(|label| label.starts_with("kittwm-session-row:0:"))
            .unwrap();
        assert!(
            row.contains("window=native-window-with-a-pathologic…"),
            "{row}"
        );
        assert!(
            row.contains("title=pane-title-that-is-pathologically-long-and-woul…"),
            "{row}"
        );
        assert!(
            row.contains("command=command --with --a-pathologically-long-argument…"),
            "{row}"
        );
        assert!(row.len() < 220, "{row}");
    }

    #[test]
    fn chrome_scene_row_rects_fit_narrow_widths() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_INFO_COLS", "8");
        let chrome = serde_json::json!({
            "workspace": "dev",
            "top_bar_rows": 2,
            "bottom_bar_rows": 1,
            "left_cols": 4,
            "right_cols": 3,
            "gap_cols": 1,
            "gap_rows": 2,
            "owner": "bar",
            "tilable_rows": 19
        });
        let scene = chrome_scene(&chrome);
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if layer
                .label
                .as_deref()
                .unwrap_or_default()
                .contains("kittwm-chrome-row:")
            {
                let Node::Rect { rect, .. } = &layer.root else {
                    panic!("expected row rect");
                };
                assert!(rect.origin.0 >= 0.0, "{rect:?}");
                assert!(rect.width >= 1.0, "{rect:?}");
                assert!(
                    rect.origin.0 + rect.width <= width + 0.01,
                    "{rect:?} > {width}"
                );
            }
        }
        std::env::remove_var("KITTWM_INFO_COLS");
    }

    #[test]
    fn chrome_scene_clips_pathological_label_fields() {
        let chrome = serde_json::json!({
            "workspace": " workspace-name-that-is-pathologically-long ",
            "top_bar_rows": 2,
            "bottom_bar_rows": 1,
            "left_cols": 4,
            "right_cols": 3,
            "gap_cols": 1,
            "gap_rows": 2,
            "owner": " owner-name-that-is-pathologically-long ",
            "tilable_rows": "tilable-row-value-that-is-pathologically-long"
        });
        let scene = chrome_scene(&chrome);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-chrome-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("workspace=workspace-name-that-is-patholog…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("owner=owner-name-that-is-pathological…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("tilable_rows=\"tilable-row-value-that-is-path…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 230, "{backdrop}");
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-chrome-row:0:workspace=workspace-name-that-is-patholog…")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-chrome-row:1:owner=owner-name-that-is-pathological…")),
            "{labels:?}"
        );
    }

    #[test]
    fn chrome_backdrop_label_builds_directly() {
        let label = chrome_backdrop_label("dev", "bar", 2, 1, 4, 3, 1, 2, "19");
        assert_eq!(
            label,
            "kittwm-chrome-backdrop:workspace=dev:owner=bar:top=2:bottom=1:left=4:right=3:gap_cols=1:gap_rows=2:tilable_rows=19"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn chrome_scene_labels_reservation_contract() {
        let chrome = serde_json::json!({
            "workspace": " dev ",
            "top_bar_rows": 2,
            "bottom_bar_rows": 1,
            "left_cols": 4,
            "right_cols": 3,
            "gap_cols": 1,
            "gap_rows": 2,
            "owner": " bar ",
            "tilable_rows": 19
        });
        let scene = chrome_scene(&chrome);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-chrome-backdrop:workspace=dev:owner=bar:top=2:bottom=1:left=4:right=3:gap_cols=1:gap_rows=2:tilable_rows=19"
            )),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-chrome-row:2:top_bar_rows=2")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-chrome-row:7:gap_rows=2")),
            "{labels:?}"
        );
    }

    #[test]
    fn help_topic_row_label_builds_directly() {
        let label = help_topic_row_label("panes", 3, "--spawn-pty CMD");
        assert_eq!(label, "kittwm-help-topic-row:panes:3:--spawn-pty CMD");
        assert_eq!(
            label.capacity(),
            "kittwm-help-topic-row::".len() + "panes".len() + 20 + "--spawn-pty CMD".len()
        );
    }

    #[test]
    fn help_topic_heading_label_builds_directly() {
        let label = help_topic_heading_label("panes", "kittwm help panes");
        assert_eq!(label, "kittwm-help-topic-heading:panes:kittwm help panes");
        assert_eq!(
            label.capacity(),
            "kittwm-help-topic-heading::".len() + "panes".len() + "kittwm help panes".len()
        );
    }

    #[test]
    fn help_topic_backdrop_label_builds_directly() {
        let label = help_topic_backdrop_label("panes", 9, 4);
        assert_eq!(label, "kittwm-help-topic-backdrop:panes:lines=9:commands=4");
        assert_eq!(
            label.capacity(),
            "kittwm-help-topic-backdrop::lines=:commands=".len() + "panes".len() + 40
        );
    }

    #[test]
    fn help_topic_scene_rows_saturate_large_help_text() {
        assert_eq!(help_topic_scene_rows(0), 8);
        assert_eq!(help_topic_scene_rows(8), 12);
        assert_eq!(help_topic_scene_rows(usize::MAX), 30);

        let mut text = String::with_capacity(128 * "kittwm help line 000\n".len());
        for idx in 0..128 {
            if idx > 0 {
                text.push('\n');
            }
            text.push_str("kittwm help line ");
            let _ = write!(text, "{idx}");
        }
        let scene = help_topic_scene_for_cols("stress", &text, 80);
        assert_eq!(scene.footprint.rows, 30);
    }

    #[test]
    fn help_topic_scene_rows_fit_narrow_width() {
        let text = help_topic_text("panes").unwrap();
        let scene = help_topic_scene_for_cols("panes", text, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
    }

    #[test]
    fn help_topic_scene_labels_existing_topic_text() {
        let text = help_topic_text("panes").unwrap();
        let scene = help_topic_scene("panes", text);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("kittwm-help-topic-backdrop:panes:")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-help-topic-heading:panes:kittwm help panes")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("--spawn-pty CMD")),
            "{labels:?}"
        );
    }

    fn help_topic_stress_text() -> String {
        let heading = "heading-".repeat(1024);
        let row = "row-".repeat(2048);
        let mut text = String::with_capacity(
            heading.len() + "kittwm --".len() + row.len() + "plain text".len() + 2,
        );
        text.push_str(&heading);
        text.push('\n');
        text.push_str("kittwm --");
        text.push_str(&row);
        text.push('\n');
        text.push_str("plain text");
        text
    }

    #[test]
    fn help_topic_stress_text_builds_directly() {
        let text = help_topic_stress_text();
        assert!(text.starts_with("heading-heading-"));
        assert!(text.contains("\nkittwm --row-row-"));
        assert!(text.ends_with("\nplain text"));
        assert_eq!(text.capacity(), text.len());
    }

    #[test]
    fn help_topic_scene_labels_clip_pathological_payloads() {
        let topic = "topic-".repeat(1024);
        let text = help_topic_stress_text();
        let scene = help_topic_scene_for_cols(&topic, &text, 80);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.iter().any(|label| label.contains('…')), "{labels:?}");
        assert!(
            labels
                .iter()
                .all(|label| !label.contains(&"topic-".repeat(32))),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .all(|label| !label.contains(&"heading-".repeat(16))),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .all(|label| !label.contains(&"row-".repeat(32))),
            "{labels:?}"
        );
    }

    #[test]
    fn filter_candidates_prefers_exact_then_prefix_matches() {
        let items = vec![
            "multixterm".to_string(),
            "xterm".to_string(),
            "xtermcontrol".to_string(),
        ];
        assert_eq!(
            filter_candidates(items, Some("xterm"), 10),
            vec![
                "xterm".to_string(),
                "xtermcontrol".to_string(),
                "multixterm".to_string()
            ]
        );
    }

    #[test]
    fn candidate_match_score_avoids_candidate_and_query_lowercase_allocation() {
        let huge = candidate_match_test_wrapped_text("Needle", 10_000);
        assert_eq!(candidate_match_score("Needle", "NeEdLe"), Some(0));
        assert_eq!(candidate_match_score("NeedleSuffix", "NEEDLE"), Some(1));
        assert_eq!(candidate_match_score(&huge, "nEeDlE"), Some(2));
        assert_eq!(candidate_match_score(&huge, "missing"), None);
        assert!(ascii_casefold_contains("RésuméNeedle", "NeEdLe"));
        let huge_title = candidate_match_test_wrapped_text("Terminal", 10_000);
        assert!(ascii_casefold_contains(&huge_title, "TERMINAL"));
        assert!(!ascii_casefold_contains(&huge_title, "browser"));
    }

    #[test]
    fn candidate_match_test_wrapped_text_builds_directly() {
        let text = candidate_match_test_wrapped_text("Needle", 3);
        assert_eq!(text, "xxxNeedleyyy");
        assert!(text.capacity() >= text.len());
    }

    fn candidate_match_test_wrapped_text(needle: &str, count: usize) -> String {
        let mut text = String::with_capacity(count + needle.len() + count);
        text.extend(std::iter::repeat_n('x', count));
        text.push_str(needle);
        text.extend(std::iter::repeat_n('y', count));
        text
    }

    #[test]
    fn launcher_row_label_builds_directly() {
        assert_eq!(
            launcher_row_label(2, "macos", "Terminal", true),
            "kittwm-launcher-row:2:macos:Terminal:selected=true"
        );
        assert_eq!(
            launcher_row_label(1, "path", "xterm", false),
            "kittwm-launcher-row:1:path:xterm:selected=false"
        );
    }

    #[test]
    fn launcher_heading_label_builds_directly() {
        assert_eq!(
            launcher_heading_label("path:Terminal"),
            "kittwm-launcher-heading:selected=path:Terminal"
        );
    }

    #[test]
    fn launcher_backdrop_label_builds_directly() {
        assert_eq!(
            launcher_backdrop_label("term", 2, 3),
            "kittwm-launcher-backdrop:query=term:selected=2:count=3"
        );
    }

    #[test]
    fn launcher_preview_row_text_builds_directly() {
        let candidate = AppCandidate::path("Terminal");
        assert_eq!(
            launcher_preview_row_text(2, &candidate, true),
            "▶  2. [path ] Terminal"
        );
        assert_eq!(
            launcher_preview_row_text(12, &candidate, false),
            "  12. [path ] Terminal"
        );
    }

    #[test]
    fn launcher_selected_label_builds_directly_and_bounds_name() {
        let candidate =
            AppCandidate::path("path-command-name-that-is-pathologically-long-and-noisy");
        let label = launcher_selected_label(Some(&candidate));
        assert!(
            label.starts_with("path:path-command-name-that-is-pathologically-long"),
            "{label}"
        );
        assert!(label.ends_with('…'), "{label}");
        assert_eq!(launcher_selected_label(None), "none:<none>");
    }

    #[test]
    fn launcher_scene_rows_saturate_before_clamping() {
        assert_eq!(launcher_scene_rows(0), 8);
        assert_eq!(launcher_scene_rows(3), 8);
        assert_eq!(launcher_scene_rows(19), 24);
        assert_eq!(launcher_scene_rows(usize::MAX), 24);
    }

    #[test]
    fn launcher_scene_row_rects_fit_narrow_widths() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_INFO_COLS", "8");
        let candidates = vec![AppCandidate::path("xterm"), AppCandidate::macos("Terminal")];
        let scene = launcher_scene("term", 1, &candidates);
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if layer
                .label
                .as_deref()
                .unwrap_or_default()
                .contains("kittwm-launcher-row:")
            {
                let Node::Rect { rect, .. } = &layer.root else {
                    panic!("expected row rect");
                };
                assert!(rect.origin.0 >= 0.0, "{rect:?}");
                assert!(rect.width >= 1.0, "{rect:?}");
                assert!(
                    rect.origin.0 + rect.width <= width + 0.01,
                    "{rect:?} > {width}"
                );
            }
        }
        std::env::remove_var("KITTWM_INFO_COLS");
    }

    #[test]
    fn launcher_scene_labels_selected_candidate() {
        let candidates = vec![AppCandidate::path("xterm"), AppCandidate::macos("Terminal")];
        let scene = launcher_scene("term", 1, &candidates);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label
                    .contains("kittwm-launcher-backdrop:query=term:selected=2:count=2")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-launcher-heading:selected=macos:Terminal")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-launcher-row:2:macos:Terminal:selected=true")),
            "{labels:?}"
        );
    }

    #[test]
    fn launcher_scene_clips_pathological_label_fields() {
        let candidates = vec![
            AppCandidate::path("path-command-name-that-is-pathologically-long-and-bloats-labels"),
            AppCandidate::macos(
                "macOS Application Name That Is Pathologically Long And Bloats Labels",
            ),
        ];
        let scene = launcher_scene(
            "query-value-that-is-pathologically-long-and-would-bloat-labels",
            1,
            &candidates,
        );
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-launcher-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("query=query-value-that-is-pathologically-long-and-wou…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 120, "{backdrop}");
        let heading = labels
            .iter()
            .find(|label| label.starts_with("kittwm-launcher-heading:"))
            .unwrap();
        assert!(
            heading.contains("selected=macos:macOS Application Name That Is Pathologically L…"),
            "{heading}"
        );
        assert!(heading.len() < 100, "{heading}");
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-launcher-row:1:path:path-command-name-that-is-pathologically-long-a…"
            )),
            "{labels:?}"
        );
        assert!(labels
            .iter()
            .any(|label| label.contains("kittwm-launcher-row:2:macos:macOS Application Name That Is Pathologically L…:selected=true")), "{labels:?}");
    }

    #[test]
    fn json_string_array_builds_without_intermediate_rows() {
        assert_eq!(json_string_array(&[]), "");
        assert_eq!(
            json_string_array(&["alpha".to_string(), "quote \" item".to_string()]),
            "\"alpha\", \"quote \\\" item\""
        );
    }

    #[test]
    fn apps_json_response_writes_default_path_or_null() {
        let cli = Cli {
            apps: true,
            json: true,
            apps_limit: Some(1),
            ..Cli::default()
        };
        assert!(apps_cmd(&cli).is_ok());
    }

    #[test]
    fn apps_json_helpers_include_filter_limit_and_counts() {
        let query = Some("term");
        assert_eq!(json_option_string(query), "\"term\"");
        assert_eq!(json_option_string(None), "null");
        let path_count = 2usize;
        let macos_count = 1usize;
        let linux_desktop_count = 3usize;
        let total_count = path_count
            .saturating_add(macos_count)
            .saturating_add(linux_desktop_count);
        let json = format!(
            "{{\"mode\": \"shell-path-macos-linux-desktop\", \"filter\": {}, \"limit\": {}, \"linux_desktop_ids\": [{}], \"linux_desktop_files\": [{}], \"linux_desktop_apps\": [{}], \"linux_desktop_localized_names\": [{}], \"linux_desktop_generic_names\": [{}], \"linux_desktop_keywords\": [{}], \"linux_desktop_categories\": [{}], \"linux_desktop_comments\": [{}], \"path_count\": {}, \"macos_count\": {}, \"linux_desktop_count\": {}, \"total_count\": {}}}",
            json_option_string(query),
            10,
            json_string_array(&["org.example.Term.desktop".to_string()]),
            json_string_array(&["/usr/share/applications/org.example.Term.desktop".to_string()]),
            json_string_array(&["Terminal".to_string()]),
            json_string_array(&["Terminale".to_string()]),
            json_string_array(&["Terminal emulator".to_string()]),
            json_string_array(&["shell;console;".to_string()]),
            json_string_array(&["System;TerminalEmulator;".to_string()]),
            json_string_array(&["Open a shell".to_string()]),
            path_count,
            macos_count,
            linux_desktop_count,
            total_count
        );
        assert!(
            json.contains("\"mode\": \"shell-path-macos-linux-desktop\""),
            "{json}"
        );
        assert!(json.contains("\"filter\": \"term\""), "{json}");
        assert!(json.contains("\"limit\": 10"), "{json}");
        assert!(
            json.contains("\"linux_desktop_ids\": [\"org.example.Term.desktop\"]"),
            "{json}"
        );
        assert!(json.contains("\"linux_desktop_files\":"), "{json}");
        assert!(
            json.contains("\"linux_desktop_apps\": [\"Terminal\"]"),
            "{json}"
        );
        assert!(
            json.contains("\"linux_desktop_localized_names\": [\"Terminale\"]"),
            "{json}"
        );
        assert!(
            json.contains("\"linux_desktop_generic_names\": [\"Terminal emulator\"]"),
            "{json}"
        );
        assert!(
            json.contains("\"linux_desktop_keywords\": [\"shell;console;\"]"),
            "{json}"
        );
        assert!(
            json.contains("\"linux_desktop_categories\": [\"System;TerminalEmulator;\"]"),
            "{json}"
        );
        assert!(
            json.contains("\"linux_desktop_comments\": [\"Open a shell\"]"),
            "{json}"
        );
        assert!(json.contains("\"path_count\": 2"), "{json}");
        assert!(json.contains("\"macos_count\": 1"), "{json}");
        assert!(json.contains("\"linux_desktop_count\": 3"), "{json}");
        assert!(json.contains("\"total_count\": 6"), "{json}");
    }

    #[test]
    fn linux_desktop_app_parser_filters_launcher_entries() {
        let app = parse_linux_desktop_app(
            "org.example.Term.desktop",
            "/usr/share/applications/org.example.Term.desktop",
            "[Desktop Entry]\nType=Application\nName=Example Terminal\nName[fr]=Terminale Exemple\nGenericName=Terminal emulator\nGenericName[en_GB]=Command line\nComment=Open a shell\nComment[en_GB]=Open a terminal window\nKeywords=shell;console;\nKeywords[en_GB]=cli;tty;\nCategories=System;TerminalEmulator;\nExec=example-term\n",
        )
        .unwrap();
        assert_eq!(app.label, "Example Terminal");
        assert_eq!(app.localized_names, "Terminale Exemple");
        assert_eq!(app.generic_name, "Terminal emulator;Command line");
        assert_eq!(app.comment, "Open a shell;Open a terminal window");
        assert_eq!(app.keywords, "shell;console;;cli;tty;");
        assert_eq!(app.categories, "System;TerminalEmulator;");
        assert!(linux_desktop_app_matches(&app, Some("terminal")));
        assert!(linux_desktop_app_matches(&app, Some("org.example")));
        assert!(linux_desktop_app_matches(&app, Some("console")));
        assert!(linux_desktop_app_matches(&app, Some("command line")));
        assert!(linux_desktop_app_matches(&app, Some("Terminale Exemple")));
        assert!(linux_desktop_app_matches(&app, Some("open a shell")));
        assert!(linux_desktop_app_matches(&app, Some("TerminalEmulator")));
        assert!(!linux_desktop_app_matches(&app, Some("browser")));
        assert!(parse_linux_desktop_app(
            "hidden.desktop",
            "/tmp/hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=Hidden\nExec=hidden\nNoDisplay=true\n",
        )
        .is_none());
        assert!(parse_linux_desktop_app(
            "wrong-desktop.desktop",
            "/tmp/wrong-desktop.desktop",
            "[Desktop Entry]\nType=Application\nName=Wrong Desktop\nExec=wrong\nOnlyShowIn=DefinitelyNoSuchDesktop;\n",
        )
        .is_none());
        assert!(parse_linux_desktop_app(
            "missing-tryexec.desktop",
            "/tmp/missing-tryexec.desktop",
            "[Desktop Entry]\nType=Application\nName=Missing TryExec\nExec=missing\nTryExec=definitely-no-such-kittwm-binary --flag\n",
        )
        .is_none());
        let current_exe = std::env::current_exe().unwrap();
        let quoted_try_exec = format!("'{}' --ignored", current_exe.display());
        let app_with_try_exec = parse_linux_desktop_app(
            "tryexec.desktop",
            "/tmp/tryexec.desktop",
            &format!(
                "[Desktop Entry]\nType=Application\nName=TryExec App\nExec=tryexec\nTryExec={quoted_try_exec}\n"
            ),
        )
        .unwrap();
        assert_eq!(app_with_try_exec.label, "TryExec App");
        assert_eq!(
            desktop_try_exec_token("'quoted command' --flag").as_deref(),
            Some("quoted command")
        );
        let detail = linux_desktop_app_detail(&app);
        assert_eq!(
            detail,
            "Terminale Exemple; Terminal emulator;Command line; System;TerminalEmulator;"
        );
        let row = linux_desktop_app_row(&app);
        assert_eq!(
            row,
            "Example Terminal (org.example.Term.desktop) — /usr/share/applications/org.example.Term.desktop — Terminale Exemple; Terminal emulator;Command line; System;TerminalEmulator;"
        );
        assert_eq!(row.capacity(), row.len());
        assert!(parse_linux_desktop_app(
            "link.desktop",
            "/tmp/link.desktop",
            "[Desktop Entry]\nType=Link\nName=Link\nExec=link\n",
        )
        .is_none());
    }

    #[test]
    fn app_launch_first_json_reports_structured_success_and_errors() {
        let candidate = AppCandidate::path("xterm");
        let success = app_launch_json(Some("term"), &candidate, 42, "path");
        assert!(success.contains("\"mode\":\"launch-first\""), "{success}");
        assert!(success.contains("\"filter\":\"term\""), "{success}");
        assert!(success.contains("\"kind\":\"path\""), "{success}");
        assert!(success.contains("\"method\":\"path\""), "{success}");
        assert!(success.contains("\"candidate\":\"xterm\""), "{success}");
        assert!(success.contains("\"pid\":\"42\""), "{success}");

        let first = app_first_json(Some("term"), &candidate);
        assert!(first.contains("\"mode\":\"first\""), "{first}");
        assert!(first.contains("\"candidate\":\"xterm\""), "{first}");
        let first_error = app_first_json_error(
            Some("missing"),
            "no_candidates",
            "no app candidates matched",
        );
        assert!(first_error.contains("\"mode\":\"first\""), "{first_error}");
        assert!(
            first_error.contains("\"error\":\"no_candidates\""),
            "{first_error}"
        );

        let error = app_launch_json_error(
            Some("missing"),
            "no_candidates",
            "no app candidates matched",
        );
        assert!(error.contains("\"error\":\"no_candidates\""), "{error}");
        assert!(
            error.contains("\"message\":\"no app candidates matched\""),
            "{error}"
        );
    }

    #[test]
    fn app_candidates_include_linux_desktop_launchers() {
        let linux_app = LinuxDesktopApp {
            id: "org.example.Term.desktop".to_string(),
            label: "Example Terminal".to_string(),
            file: "/usr/share/applications/org.example.Term.desktop".to_string(),
            exec: "example-term %U".to_string(),
            localized_names: "Terminale Exemple".to_string(),
            generic_name: "Terminal emulator".to_string(),
            keywords: "shell;console;".to_string(),
            categories: "System;TerminalEmulator;".to_string(),
            comment: "Open a shell".to_string(),
        };
        let selected = first_app_candidate(&[], &[], &[linux_app]).unwrap();
        assert_eq!(selected.kind, "desktop");
        assert_eq!(selected.name, "org.example.Term.desktop");
        assert_eq!(selected.display_name(), "Example Terminal");
        assert_eq!(
            selected.desktop_file.as_deref(),
            Some("/usr/share/applications/org.example.Term.desktop")
        );
        assert_eq!(
            strip_desktop_exec_field_codes("example-term %U --new-window %f"),
            "example-term --new-window"
        );
        let json = app_launch_json(Some("term"), &selected, 99, "gtk-launch");
        assert!(json.contains("\"kind\":\"desktop\""), "{json}");
        assert!(
            json.contains("\"candidate\":\"org.example.Term.desktop\""),
            "{json}"
        );
        assert!(json.contains("\"name\":\"Example Terminal\""), "{json}");
        assert!(json.contains("\"method\":\"gtk-launch\""), "{json}");
        assert!(
            json.contains("\"desktop_file\":\"/usr/share/applications/org.example.Term.desktop\""),
            "{json}"
        );
    }

    #[test]
    fn remote_aliases_map_to_pooled_ssh_commands() {
        let mut cli = Cli::default();
        cli.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut cli,
            "apps",
            &args(&["--filter", "firefox", "--launch-first"]),
        )
        .unwrap();
        assert!(cli.apps);
        assert_eq!(cli.remote_host.as_deref(), Some("buildbox"));
        assert_eq!(cli.apps_filter.as_deref(), Some("firefox"));
        assert!(cli.apps_launch_first);

        let mut app_query = Cli::default();
        app_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut app_query, "apps", &args(&["fire", "fox", "--first"]))
            .unwrap();
        assert!(app_query.apps);
        assert_eq!(app_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(app_query.apps_first);

        let mut applications_query = Cli::default();
        applications_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut applications_query,
            "applications",
            &args(&["fire", "fox", "--json"]),
        )
        .unwrap();
        assert!(applications_query.apps);
        assert_eq!(applications_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(applications_query.json);

        let mut programs_query = Cli::default();
        programs_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut programs_query, "programs", &args(&["fire", "fox"]))
            .unwrap();
        assert!(programs_query.apps);
        assert_eq!(programs_query.apps_filter.as_deref(), Some("fire fox"));

        let mut software_query = Cli::default();
        software_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut software_query,
            "software",
            &args(&["fire", "fox", "--first"]),
        )
        .unwrap();
        assert!(software_query.apps);
        assert_eq!(software_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(software_query.apps_first);

        let mut fallback_query = Cli::default();
        fallback_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut fallback_query,
            "apps",
            &args(&["fire", "fox", "--fallback"]),
        )
        .unwrap();
        assert!(fallback_query.apps);
        assert_eq!(fallback_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(fallback_query.apps_force_fallback);

        let mut fallback_wrapper_query = Cli::default();
        fallback_wrapper_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut fallback_wrapper_query,
            "fallback",
            &args(&["apps", "fire", "fox"]),
        )
        .unwrap();
        assert!(fallback_wrapper_query.apps);
        assert_eq!(
            fallback_wrapper_query.apps_filter.as_deref(),
            Some("fire fox")
        );
        assert!(fallback_wrapper_query.apps_force_fallback);

        let mut fallback_launch = Cli::default();
        fallback_launch.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut fallback_launch,
            "fallback",
            &args(&["launch", "fire", "fox"]),
        )
        .unwrap();
        assert!(fallback_launch.apps);
        assert_eq!(fallback_launch.apps_filter.as_deref(), Some("fire fox"));
        assert!(fallback_launch.apps_launch_first);
        assert!(fallback_launch.apps_force_fallback);

        let mut singular_app_query = Cli::default();
        singular_app_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut singular_app_query, "app", &args(&["fire", "fox"])).unwrap();
        assert!(singular_app_query.apps);
        assert_eq!(singular_app_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(singular_app_query.apps_first);

        let mut singular_app_json_query = Cli::default();
        singular_app_json_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut singular_app_json_query,
            "app",
            &args(&["fire", "fox", "--json"]),
        )
        .unwrap();
        assert!(singular_app_json_query.apps);
        assert_eq!(
            singular_app_json_query.apps_filter.as_deref(),
            Some("fire fox")
        );
        assert!(singular_app_json_query.apps_first);
        assert!(singular_app_json_query.json);

        let mut application_json_query = Cli::default();
        application_json_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut application_json_query,
            "application",
            &args(&["fire", "fox", "--json"]),
        )
        .unwrap();
        assert!(application_json_query.apps);
        assert_eq!(
            application_json_query.apps_filter.as_deref(),
            Some("fire fox")
        );
        assert!(application_json_query.apps_first);
        assert!(application_json_query.json);

        let mut program_json_query = Cli::default();
        program_json_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut program_json_query,
            "program",
            &args(&["fire", "fox", "--json"]),
        )
        .unwrap();
        assert!(program_json_query.apps);
        assert_eq!(program_json_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(program_json_query.apps_first);
        assert!(program_json_query.json);

        let mut select_query = Cli::default();
        select_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut select_query, "select", &args(&["fire", "fox"])).unwrap();
        assert!(select_query.apps);
        assert_eq!(select_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(select_query.apps_first);

        let mut pick_json_query = Cli::default();
        pick_json_query.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut pick_json_query,
            "pick",
            &args(&["fire", "fox", "--json"]),
        )
        .unwrap();
        assert!(pick_json_query.apps);
        assert_eq!(pick_json_query.apps_filter.as_deref(), Some("fire fox"));
        assert!(pick_json_query.apps_first);
        assert!(pick_json_query.json);

        let mut doctor = Cli::default();
        doctor.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut doctor, "doctor", &args(&["--json"])).unwrap();
        assert!(doctor.doctor);
        assert!(doctor.json);

        let mut status = Cli::default();
        status.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut status, "status", &[]).unwrap();
        assert!(status.doctor);

        let mut x11_status = Cli::default();
        x11_status.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut x11_status, "status", &args(&["--x11"])).unwrap();
        assert!(x11_status.doctor);
        assert!(x11_status.remote_doctor_graphical);

        let mut wayland_status = Cli::default();
        wayland_status.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut wayland_status, "status", &args(&["--wayland"])).unwrap();
        assert!(wayland_status.doctor);
        assert!(wayland_status.remote_doctor_graphical);

        let mut x11_alias = Cli::default();
        x11_alias.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut x11_alias, "x11", &args(&["--json"])).unwrap();
        assert!(x11_alias.doctor);
        assert!(x11_alias.json);
        assert!(x11_alias.remote_doctor_graphical);

        let mut forwarding_alias = Cli::default();
        forwarding_alias.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut forwarding_alias, "forwarding", &[]).unwrap();
        assert!(forwarding_alias.doctor);
        assert!(forwarding_alias.remote_doctor_graphical);

        let mut forward_alias = Cli::default();
        forward_alias.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut forward_alias, "forward", &[]).unwrap();
        assert!(forward_alias.doctor);
        assert!(forward_alias.remote_doctor_graphical);

        let mut forward_status = Cli::default();
        forward_status.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut forward_status, "status", &args(&["--forward"])).unwrap();
        assert!(forward_status.doctor);
        assert!(forward_status.remote_doctor_graphical);

        let mut gui_alias = Cli::default();
        gui_alias.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut gui_alias, "gui", &[]).unwrap();
        assert!(gui_alias.doctor);
        assert!(gui_alias.remote_doctor_graphical);

        let mut wayland_alias = Cli::default();
        wayland_alias.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut wayland_alias, "wayland", &[]).unwrap();
        assert!(wayland_alias.doctor);
        assert!(wayland_alias.remote_doctor_graphical);

        let mut help = Cli::default();
        help.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut help, "help", &[]).unwrap();
        assert!(help.remote_help);

        let mut windows = Cli::default();
        windows.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut windows, "windows", &[]).unwrap();
        assert!(windows.list_windows);

        let mut list_apps = Cli::default();
        list_apps.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_apps, "list", &args(&["apps", "terminal"])).unwrap();
        assert!(list_apps.apps);
        assert_eq!(list_apps.apps_filter.as_deref(), Some("terminal"));

        let mut list_app = Cli::default();
        list_app.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_app, "list", &args(&["app", "terminal"])).unwrap();
        assert!(list_app.apps);
        assert_eq!(list_app.apps_filter.as_deref(), Some("terminal"));

        let mut list_applications = Cli::default();
        list_applications.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut list_applications,
            "list",
            &args(&["applications", "terminal"]),
        )
        .unwrap();
        assert!(list_applications.apps);
        assert_eq!(list_applications.apps_filter.as_deref(), Some("terminal"));

        let mut list_programs = Cli::default();
        list_programs.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_programs, "list", &args(&["programs", "terminal"]))
            .unwrap();
        assert!(list_programs.apps);
        assert_eq!(list_programs.apps_filter.as_deref(), Some("terminal"));

        let mut list_software = Cli::default();
        list_software.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_software, "list", &args(&["software", "terminal"]))
            .unwrap();
        assert!(list_software.apps);
        assert_eq!(list_software.apps_filter.as_deref(), Some("terminal"));

        let mut list_windows = Cli::default();
        list_windows.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_windows, "list", &args(&["windows", "firefox"]))
            .unwrap();
        assert!(list_windows.list_windows);
        assert_eq!(
            list_windows.remote_listing_filter.as_deref(),
            Some("firefox")
        );

        let mut list_windows_json = Cli::default();
        list_windows_json.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut list_windows_json,
            "list",
            &args(&["windows", "firefox", "--json"]),
        )
        .unwrap();
        assert!(list_windows_json.list_windows);
        assert_eq!(
            list_windows_json.remote_listing_filter.as_deref(),
            Some("firefox")
        );
        assert!(list_windows_json.json);

        let mut list_windows_fallback = Cli::default();
        list_windows_fallback.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut list_windows_fallback,
            "list",
            &args(&["windows", "firefox", "--fallback"]),
        )
        .unwrap();
        assert!(list_windows_fallback.list_windows);
        assert_eq!(
            list_windows_fallback.remote_listing_filter.as_deref(),
            Some("firefox")
        );
        assert!(list_windows_fallback.remote_listing_force_fallback);

        let mut fallback_windows = Cli::default();
        fallback_windows.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut fallback_windows,
            "fallback",
            &args(&["windows", "firefox"]),
        )
        .unwrap();
        assert!(fallback_windows.list_windows);
        assert_eq!(
            fallback_windows.remote_listing_filter.as_deref(),
            Some("firefox")
        );
        assert!(fallback_windows.remote_listing_force_fallback);

        let mut win = Cli::default();
        win.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut win, "win", &args(&["firefox"])).unwrap();
        assert!(win.list_windows);
        assert_eq!(win.remote_listing_filter.as_deref(), Some("firefox"));

        let mut list_win = Cli::default();
        list_win.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_win, "list", &args(&["win", "firefox"])).unwrap();
        assert!(list_win.list_windows);
        assert_eq!(list_win.remote_listing_filter.as_deref(), Some("firefox"));

        let mut displays = Cli::default();
        displays.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut displays, "displays", &args(&["retina"])).unwrap();
        assert!(displays.list_displays);
        assert_eq!(displays.remote_listing_filter.as_deref(), Some("retina"));

        let mut monitors = Cli::default();
        monitors.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut monitors, "monitors", &args(&["retina"])).unwrap();
        assert!(monitors.list_displays);
        assert_eq!(monitors.remote_listing_filter.as_deref(), Some("retina"));

        let mut screens = Cli::default();
        screens.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut screens, "screens", &args(&["retina"])).unwrap();
        assert!(screens.list_displays);
        assert_eq!(screens.remote_listing_filter.as_deref(), Some("retina"));

        let mut monitors_fallback = Cli::default();
        monitors_fallback.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut monitors_fallback,
            "monitors",
            &args(&["retina", "--fallback"]),
        )
        .unwrap();
        assert!(monitors_fallback.list_displays);
        assert_eq!(
            monitors_fallback.remote_listing_filter.as_deref(),
            Some("retina")
        );
        assert!(monitors_fallback.remote_listing_force_fallback);

        let mut fallback_displays = Cli::default();
        fallback_displays.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut fallback_displays,
            "fallback",
            &args(&["displays", "retina"]),
        )
        .unwrap();
        assert!(fallback_displays.list_displays);
        assert_eq!(
            fallback_displays.remote_listing_filter.as_deref(),
            Some("retina")
        );
        assert!(fallback_displays.remote_listing_force_fallback);

        let mut list_screens = Cli::default();
        list_screens.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut list_screens, "list", &args(&["screens", "retina"]))
            .unwrap();
        assert!(list_screens.list_displays);
        assert_eq!(
            list_screens.remote_listing_filter.as_deref(),
            Some("retina")
        );

        let mut terminal = Cli::default();
        terminal.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut terminal, "terminal", &args(&["htop"])).unwrap();
        assert_eq!(
            terminal.remote_terminal_args.as_deref(),
            Some(
                &[
                    "--remote".to_string(),
                    "buildbox".to_string(),
                    "--title".to_string(),
                    "buildbox: htop".to_string(),
                    "htop".to_string()
                ][..]
            )
        );

        let mut term = Cli::default();
        term.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut term, "term", &args(&["htop"])).unwrap();
        assert_eq!(term.remote_terminal_args, terminal.remote_terminal_args);

        let mut terminal_after_separator = Cli::default();
        terminal_after_separator.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut terminal_after_separator,
            "terminal",
            &args(&["--", "top"]),
        )
        .unwrap();
        assert_eq!(
            terminal_after_separator.remote_terminal_args.as_deref(),
            Some(
                &[
                    "--remote".to_string(),
                    "buildbox".to_string(),
                    "--title".to_string(),
                    "buildbox: top".to_string(),
                    "--".to_string(),
                    "top".to_string()
                ][..]
            )
        );

        let mut titled_terminal = Cli::default();
        titled_terminal.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut titled_terminal,
            "terminal",
            &args(&["--title", "logs", "tail", "-f", "/tmp/app.log"]),
        )
        .unwrap();
        assert_eq!(
            titled_terminal.remote_terminal_args.as_deref(),
            Some(
                &[
                    "--remote".to_string(),
                    "buildbox".to_string(),
                    "--title".to_string(),
                    "logs".to_string(),
                    "tail".to_string(),
                    "-f".to_string(),
                    "/tmp/app.log".to_string()
                ][..]
            )
        );

        let mut cmd = Cli::default();
        cmd.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut cmd, "cmd", &args(&["htop"])).unwrap();
        assert_eq!(cmd.remote_terminal_args, terminal.remote_terminal_args);

        let mut command = Cli::default();
        command.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut command, "command", &args(&["htop"])).unwrap();
        assert_eq!(command.remote_terminal_args, terminal.remote_terminal_args);

        let mut exec = Cli::default();
        exec.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut exec, "exec", &args(&["htop"])).unwrap();
        assert_eq!(exec.remote_terminal_args, terminal.remote_terminal_args);

        let mut sh = Cli::default();
        sh.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut sh, "sh", &args(&["htop"])).unwrap();
        assert_eq!(sh.remote_terminal_args, terminal.remote_terminal_args);

        let mut login = Cli::default();
        login.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut login, "login", &args(&["htop"])).unwrap();
        assert_eq!(login.remote_terminal_args, terminal.remote_terminal_args);

        let mut console = Cli::default();
        console.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut console, "console", &args(&["htop"])).unwrap();
        assert_eq!(console.remote_terminal_args, terminal.remote_terminal_args);

        let mut tty = Cli::default();
        tty.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut tty, "tty", &args(&["htop"])).unwrap();
        assert_eq!(tty.remote_terminal_args, terminal.remote_terminal_args);

        let mut shell = Cli::default();
        shell.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut shell, "shell", &[]).unwrap();
        assert_eq!(
            shell.remote_terminal_args.as_deref(),
            Some(
                &[
                    "--remote".to_string(),
                    "buildbox".to_string(),
                    "--title".to_string(),
                    "buildbox".to_string()
                ][..]
            )
        );

        let mut remote_kittwm = Cli::default();
        remote_kittwm.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut remote_kittwm, "kittwm", &args(&["--workspace", "ops"]))
            .unwrap();
        assert_eq!(
            remote_kittwm.remote_terminal_args.as_deref(),
            Some(
                &[
                    "--remote".to_string(),
                    "buildbox".to_string(),
                    "--title".to_string(),
                    "buildbox".to_string(),
                    "--".to_string(),
                    "kittwm".to_string(),
                    "--workspace".to_string(),
                    "ops".to_string()
                ][..]
            )
        );

        let mut remote_wm = Cli::default();
        remote_wm.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut remote_wm, "wm", &args(&["--workspace", "ops"])).unwrap();
        assert_eq!(
            remote_wm.remote_terminal_args,
            remote_kittwm.remote_terminal_args
        );

        let mut launch = Cli::default();
        launch.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut launch, "launch", &args(&["fire", "fox"])).unwrap();
        assert!(launch.apps);
        assert_eq!(launch.apps_filter.as_deref(), Some("fire fox"));
        assert!(launch.apps_launch_first);

        let mut fallback_direct_launch = Cli::default();
        fallback_direct_launch.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(
            &mut fallback_direct_launch,
            "launch",
            &args(&["fire", "fox", "--fallback"]),
        )
        .unwrap();
        assert!(fallback_direct_launch.apps);
        assert_eq!(
            fallback_direct_launch.apps_filter.as_deref(),
            Some("fire fox")
        );
        assert!(fallback_direct_launch.apps_launch_first);
        assert!(fallback_direct_launch.apps_force_fallback);

        let mut open = Cli::default();
        open.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut open, "open", &args(&["firefox"])).unwrap();
        assert!(open.apps);
        assert_eq!(open.apps_filter.as_deref(), Some("firefox"));
        assert!(open.apps_launch_first);

        let mut run = Cli::default();
        run.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut run, "run", &args(&["code"])).unwrap();
        assert!(run.apps);
        assert_eq!(run.apps_filter.as_deref(), Some("code"));
        assert!(run.apps_launch_first);

        let mut start = Cli::default();
        start.remote_host = Some("buildbox".to_string());
        parse_remote_alias_action(&mut start, "start", &args(&["firefox", "--json"])).unwrap();
        assert!(start.apps);
        assert_eq!(start.apps_filter.as_deref(), Some("firefox"));
        assert!(start.apps_launch_first);
        assert!(start.json);
    }

    #[test]
    fn doctor_executable_provenance_reports_path_and_realpath() {
        let provenance = doctor_executable_provenance(Some(std::path::PathBuf::from(".")));
        assert_eq!(provenance.path.as_deref(), Some("."));
        assert!(provenance.realpath.is_some(), "{provenance:?}");

        let mut text = String::new();
        append_doctor_executable_rows(&mut text, &provenance);
        assert!(text.contains("  executable     : ."), "{text}");
        assert!(text.contains("  executable real:"), "{text}");

        let missing = doctor_executable_provenance(Some(std::path::PathBuf::from(
            "definitely-missing-kittwm-binary",
        )));
        assert_eq!(
            missing.path.as_deref(),
            Some("definitely-missing-kittwm-binary")
        );
        assert_eq!(missing.realpath, None);
    }

    #[test]
    fn remote_doctor_script_reports_installed_and_forwarding_paths() {
        let script = remote_doctor_script();
        assert!(script.contains("command -v kittwm"), "{script}");
        assert!(script.contains("kittwm doctor --json"), "{script}");
        assert!(script.contains("kittwm_healthy"), "{script}");
        assert!(script.contains("startup_check"), "{script}");
        assert!(script.contains("target_host"), "{script}");
        assert!(script.contains("KITTWM_REMOTE_TARGET"), "{script}");
        assert!(script.contains("local_commands"), "{script}");
        assert!(script.contains("fallback_commands"), "{script}");
        assert!(script.contains("terminal_commands"), "{script}");
        assert!(
            script.contains("kittwm remote $command_host list apps firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote $command_host launch firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote $command_host fallback apps firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote $command_host fallback launch firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote $command_host list windows --fallback"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote $command_host terminal htop"),
            "{script}"
        );
        assert!(script.contains("DISPLAY"), "{script}");
        assert!(script.contains("x11_forwarding_available"), "{script}");
        assert!(script.contains("command -v waypipe"), "{script}");
        assert!(script.contains("waypipe_available"), "{script}");
        assert!(script.contains("waypipe_path"), "{script}");
        assert!(script.contains("kittwm remote %s kittwm"), "{script}");
        assert!(script.contains("kittwm remote %s graphical"), "{script}");
        assert!(script.contains("kittwm remote %s list"), "{script}");
        assert!(
            script.contains("kittwm remote %s launch firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote %s fallback apps firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote %s fallback launch firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote %s fallback windows firefox"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote %s fallback displays retina"),
            "{script}"
        );
        assert!(
            script.contains("kittwm remote %s launch firefox --fallback"),
            "{script}"
        );
        assert!(script.contains("kittwm remote %s shell"), "{script}");
        assert!(
            script.contains("kittwm remote %s terminal htop"),
            "{script}"
        );
        let args = pooled_ssh_args(
            "host.example",
            &[
                ("KITTWM_REMOTE_DOCTOR_JSON".to_string(), "1".to_string()),
                (
                    "KITTWM_REMOTE_TARGET".to_string(),
                    "host.example".to_string(),
                ),
            ],
            script,
        )
        .unwrap();
        assert!(args.contains(&"KITTWM_REMOTE_DOCTOR_JSON=1".to_string()));
        assert!(args.contains(&"KITTWM_REMOTE_TARGET=host.example".to_string()));
        assert!(args.contains(&"host.example".to_string()));
        let x11_args = pooled_ssh_args_with_forwarding(
            "host.example",
            &[(
                "KITTWM_REMOTE_DOCTOR_GRAPHICAL".to_string(),
                "1".to_string(),
            )],
            script,
            true,
        )
        .unwrap();
        assert!(x11_args.contains(&"-Y".to_string()), "{x11_args:?}");
        assert!(
            x11_args
                .iter()
                .any(|arg| arg.starts_with("ControlPath=") && arg.ends_with("%C-x11")),
            "{x11_args:?}"
        );
    }

    #[test]
    fn remote_apps_env_selects_kittwm_or_shell_modes() {
        let cli = Cli {
            apps: true,
            apps_launch_first: true,
            apps_filter: Some("Visual Studio Code".to_string()),
            apps_limit: Some(7),
            ..Cli::default()
        };
        let env = remote_apps_env(
            "host.example",
            cli.apps_filter.as_deref(),
            7,
            remote_apps_mode(&cli),
            false,
        );
        assert!(env.contains(&(
            "KITTWM_REMOTE_QUERY".to_string(),
            "Visual Studio Code".to_string()
        )));
        assert!(env.contains(&("KITTWM_REMOTE_LIMIT".to_string(), "7".to_string())));
        assert!(env.contains(&(
            "KITTWM_REMOTE_TARGET".to_string(),
            "host.example".to_string()
        )));
        assert!(env.contains(&("KITTWM_REMOTE_MODE".to_string(), "launch-first".to_string())));
        assert!(env.contains(&("KITTWM_REMOTE_FORCE_FALLBACK".to_string(), "0".to_string())));
        assert!(remote_apps_mode(&cli).requests_graphical_forwarding());
        let json_launch_cli = Cli {
            apps: true,
            apps_launch_first: true,
            json: true,
            apps_filter: Some("Visual Studio Code".to_string()),
            apps_limit: Some(7),
            ..Cli::default()
        };
        assert_eq!(
            remote_apps_mode(&json_launch_cli),
            RemoteAppsMode::LaunchFirstJson
        );
        assert!(remote_apps_env(
            "host.example",
            json_launch_cli.apps_filter.as_deref(),
            7,
            remote_apps_mode(&json_launch_cli),
            false
        )
        .contains(&(
            "KITTWM_REMOTE_MODE".to_string(),
            "launch-first-json".to_string()
        )));
        assert!(remote_apps_mode(&json_launch_cli).requests_graphical_forwarding());
        let json_first_cli = Cli {
            apps: true,
            apps_first: true,
            json: true,
            apps_filter: Some("Visual Studio Code".to_string()),
            apps_limit: Some(7),
            ..Cli::default()
        };
        assert_eq!(remote_apps_mode(&json_first_cli), RemoteAppsMode::FirstJson);
        assert!(!remote_apps_mode(&json_first_cli).requests_graphical_forwarding());
        assert!(remote_apps_env(
            "host.example",
            json_first_cli.apps_filter.as_deref(),
            7,
            remote_apps_mode(&json_first_cli),
            false
        )
        .contains(&("KITTWM_REMOTE_MODE".to_string(), "first-json".to_string())));
        let args = pooled_ssh_args_with_forwarding(
            "host.example",
            &remote_apps_env(
                "host.example",
                cli.apps_filter.as_deref(),
                7,
                remote_apps_mode(&cli),
                false,
            ),
            "echo ok",
            remote_apps_mode(&cli).requests_graphical_forwarding(),
        )
        .unwrap();
        assert!(args.contains(&"-Y".to_string()), "{args:?}");
        assert!(
            args.iter()
                .any(|arg| arg.starts_with("ControlPath=") && arg.ends_with("%C-x11")),
            "{args:?}"
        );
        let fallback_env = remote_apps_env(
            "host.example",
            cli.apps_filter.as_deref(),
            7,
            remote_apps_mode(&cli),
            true,
        );
        assert!(
            fallback_env.contains(&("KITTWM_REMOTE_FORCE_FALLBACK".to_string(), "1".to_string()))
        );

        let script = remote_apps_script();
        assert!(script.contains("command -v kittwm"), "{script}");
        assert!(script.contains("kittwm_status"), "{script}");
        assert!(
            script.contains("kittwm_remote_emit_kittwm_output"),
            "{script}"
        );
        assert!(script.contains("\"source\":\"kittwm\""), "{script}");
        assert!(
            script.contains("WARN remote kittwm apps failed; falling back"),
            "{script}"
        );
        assert!(script.contains("target=%s host=%s"), "{script}");
        assert!(
            script.contains("kittwm_remote_list_path_commands"),
            "{script}"
        );
        assert!(script.contains("kittwm_remote_list_macos_apps"), "{script}");
        assert!(
            script.contains("kittwm_remote_list_linux_desktop_apps"),
            "{script}"
        );
        assert!(
            script.contains("kittwm_remote_linux_desktop_roots"),
            "{script}"
        );
        assert!(script.contains("XDG_DATA_HOME"), "{script}");
        assert!(script.contains("XDG_DATA_DIRS"), "{script}");
        assert!(script.contains("open -a"), "{script}");
        assert!(script.contains("gtk-launch"), "{script}");
        assert!(script.contains("gio launch"), "{script}");
        assert!(script.contains("KITTWM_REMOTE_TARGET"), "{script}");
        assert!(script.contains("target_host"), "{script}");
        assert!(script.contains("target host:"), "{script}");
        assert!(script.contains("target_host=%s"), "{script}");
        assert!(script.contains("no remote graphical display"), "{script}");
        assert!(
            script.contains("kittwm remote $command_host graphical"),
            "{script}"
        );
        assert!(
            script.contains("checks X11 forwarding and waypipe"),
            "{script}"
        );
        assert!(script.contains("$1 == \"Type\""), "{script}");
        assert!(script.contains("application"), "{script}");
        assert!(script.contains("$1 == \"Name\""), "{script}");
        assert!(
            script.contains("kittwm_remote_desktop_localized_values"),
            "{script}"
        );
        assert!(script.contains("localized_names="), "{script}");
        assert!(script.contains("localized_generic_names="), "{script}");
        assert!(script.contains("localized_comments="), "{script}");
        assert!(script.contains("localized_keywords="), "{script}");
        assert!(script.contains("$1 == \"GenericName\""), "{script}");
        assert!(script.contains("$1 == \"Comment\""), "{script}");
        assert!(script.contains("$1 == \"Keywords\""), "{script}");
        assert!(script.contains("$1 == \"Categories\""), "{script}");
        assert!(script.contains("$1 == \"Exec\""), "{script}");
        assert!(script.contains("$1 == \"Hidden\""), "{script}");
        assert!(script.contains("$1 == \"NoDisplay\""), "{script}");
        assert!(script.contains("$1 == \"OnlyShowIn\""), "{script}");
        assert!(script.contains("$1 == \"NotShowIn\""), "{script}");
        assert!(script.contains("$1 == \"TryExec\""), "{script}");
        assert!(script.contains("kittwm_remote_try_exec_token"), "{script}");
        assert!(
            script.contains("kittwm_remote_try_exec_available"),
            "{script}"
        );
        assert!(script.contains("[ -x \"$token\" ]"), "{script}");
        assert!(script.contains("XDG_CURRENT_DESKTOP"), "{script}");
        assert!(script.contains("desktop_file="), "{script}");
        assert!(script.contains("desktop_exec="), "{script}");
        assert!(script.contains("launch_method=\"gtk-launch\""), "{script}");
        assert!(script.contains("launch_method=\"gio\""), "{script}");
        assert!(
            script.contains("launch_method=\"desktop-exec\""),
            "{script}"
        );
        assert!(script.contains("method=%s"), "{script}");
        assert!(script.contains("first-json"), "{script}");
        assert!(
            script.contains("first-json) set -- \"$@\" --first --json"),
            "{script}"
        );
        assert!(script.contains("\"mode\":\"first\""), "{script}");
        assert!(script.contains("launch-first-json"), "{script}");
        assert!(
            script.contains("launch-first-json) set -- \"$@\" --launch-first --json"),
            "{script}"
        );
        assert!(script.contains("\"mode\":\"launch-first\""), "{script}");
        assert!(script.contains("kittwm_remote_launch_error"), "{script}");
        assert!(script.contains("\"error\":"), "{script}");
        assert!(script.contains("\"message\":"), "{script}");
        assert!(script.contains("no_candidates"), "{script}");
        assert!(script.contains("no_graphical_display"), "{script}");
        assert!(script.contains("no_desktop_exec_fallback"), "{script}");
        assert!(script.contains("\"method\":"), "{script}");
        assert!(script.contains("\"candidate\":"), "{script}");
        assert!(script.contains("json_option()"), "{script}");
        assert!(
            script.contains("$(json_option \"${KITTWM_REMOTE_QUERY:-}\")"),
            "{script}"
        );
        assert!(script.contains("$(json_option \"$hint\")"), "{script}");
        assert!(script.contains("\"desktop_file\":"), "{script}");
        assert!(
            script.contains("$(json_option \"$desktop_file\")"),
            "{script}"
        );
        assert!(script.contains("\"pid\":"), "{script}");
        assert!(script.contains("gtk-launch/gio failed"), "{script}");
        assert!(script.contains("index(tolower($0), q)"), "{script}");
        assert!(
            script.contains("shell-path-macos-linux-desktop"),
            "{script}"
        );
        assert!(script.contains("\"filter\":"), "{script}");
        assert!(script.contains("\"source\":\"fallback\""), "{script}");
        assert!(script.contains("\"forced_fallback\":"), "{script}");
        assert!(
            script.contains("kittwm_remote_forced_fallback_json"),
            "{script}"
        );
        assert!(script.contains("forced fallback:"), "{script}");
        assert!(script.contains("forced_fallback=%s"), "{script}");
        assert!(script.contains("\"limit\":"), "{script}");
        assert!(script.contains("\"macos_apps\":"), "{script}");
        assert!(script.contains("\"linux_desktop_ids\":"), "{script}");
        assert!(script.contains("\"linux_desktop_files\":"), "{script}");
        assert!(script.contains("\"linux_desktop_apps\":"), "{script}");
        assert!(
            script.contains("\"linux_desktop_localized_names\":"),
            "{script}"
        );
        assert!(script.contains("{ print $9 }"), "{script}");
        assert!(
            script.contains("\"linux_desktop_generic_names\":"),
            "{script}"
        );
        assert!(script.contains("\"linux_desktop_keywords\":"), "{script}");
        assert!(script.contains("\"linux_desktop_categories\":"), "{script}");
        assert!(script.contains("\"linux_desktop_comments\":"), "{script}");
        assert!(script.contains("\"path_count\":"), "{script}");
        assert!(script.contains("\"macos_count\":"), "{script}");
        assert!(script.contains("\"linux_desktop_count\":"), "{script}");
        assert!(script.contains("\"total_count\":"), "{script}");
        assert!(script.contains("kittwm_remote_candidate_count"), "{script}");
        assert!(script.contains("detail=\"\""), "{script}");
        assert!(script.contains("if ($9 != \"\") detail=$9"), "{script}");
        assert!(script.contains(") — \"$5"), "{script}");
        assert!(script.contains("detail\"; \""), "{script}");
    }

    #[test]
    fn remote_listing_modes_delegate_to_kittwm_or_platform_fallbacks() {
        assert_eq!(RemoteListingKind::Windows.env_value(), "windows");
        assert_eq!(RemoteListingKind::Displays.env_value(), "displays");
        let script = remote_listing_script();
        assert!(script.contains("KITTWM_REMOTE_JSON"), "{script}");
        assert!(script.contains("KITTWM_REMOTE_TARGET"), "{script}");
        assert!(script.contains("target_host"), "{script}");
        assert!(script.contains("target host:"), "{script}");
        assert!(script.contains("KITTWM_REMOTE_FORCE_FALLBACK"), "{script}");
        assert!(
            script.contains("KITTWM_REMOTE_FORCE_FALLBACK:-0"),
            "{script}"
        );
        assert!(script.contains("\"forced_fallback\":"), "{script}");
        assert!(script.contains("forced fallback:"), "{script}");
        assert!(script.contains("kittwm_remote_emit_json_lines"), "{script}");
        assert!(script.contains("\"mode\":"), "{script}");
        assert!(script.contains("\"source\":"), "{script}");
        assert!(script.contains("\"lines\":"), "{script}");
        assert!(script.contains("\"count\":"), "{script}");
        assert!(
            script.contains("kittwm_remote_emit kittwm kittwm"),
            "{script}"
        );
        assert!(script.contains("fallback swaymsg-jq"), "{script}");
        assert!(script.contains("fallback xrandr"), "{script}");
        assert!(script.contains("fallback wmctrl"), "{script}");
        assert!(script.contains("kittwm --list-windows"), "{script}");
        assert!(script.contains("kittwm --list-displays"), "{script}");
        assert!(script.contains("kittwm_status"), "{script}");
        assert!(
            script.contains("WARN remote kittwm %s listing failed; falling back"),
            "{script}"
        );
        assert!(script.contains("target=%s host=%s"), "{script}");
        assert!(script.contains("KITTWM_REMOTE_QUERY"), "{script}");
        assert!(script.contains("json_option()"), "{script}");
        assert!(script.contains("$(json_option \"$query\")"), "{script}");
        assert!(script.contains("kittwm_remote_filter"), "{script}");
        assert!(script.contains("swaymsg"), "{script}");
        assert!(script.contains("jq"), "{script}");
        assert!(script.contains("python3"), "{script}");
        assert!(
            script.contains("kittwm_remote_sway_outputs_python"),
            "{script}"
        );
        assert!(
            script.contains("kittwm_remote_sway_tree_python"),
            "{script}"
        );
        assert!(script.contains("swaymsg+python3"), "{script}");
        assert!(script.contains("wmctrl -lx"), "{script}");
        assert!(script.contains("getwindowclassname"), "{script}");
        assert!(script.contains("xrandr"), "{script}");
        assert!(script.contains("system_profiler"), "{script}");
    }

    #[test]
    fn create_dir_context_builds_directly() {
        let path = std::path::PathBuf::from("/tmp/kittwm-ssh");
        let context = create_dir_context(&path);
        assert_eq!(context, "create /tmp/kittwm-ssh");
        assert_eq!(context.capacity(), context.len());
    }

    #[test]
    fn ssh_env_arg_builds_directly() {
        let arg = ssh_env_arg("KITTWM_REMOTE_QUERY", "Visual Studio Code");
        assert_eq!(arg, "KITTWM_REMOTE_QUERY='Visual Studio Code'");
        assert_eq!(arg.capacity(), arg.len());
        assert_eq!(ssh_env_arg("Q", "it's ok"), "Q='it'\\''s ok'");
    }

    #[test]
    fn ssh_control_path_arg_builds_directly() {
        let path = std::path::PathBuf::from("/tmp/kittwm-ssh/%C");
        let arg = ssh_control_path_arg(&path);
        assert_eq!(arg, "ControlPath=/tmp/kittwm-ssh/%C");
        assert_eq!(arg.capacity(), arg.len());
    }

    #[test]
    fn pooled_ssh_args_enable_controlmaster_and_quote_env() {
        let args = pooled_ssh_args(
            "host.example",
            &[(
                "KITTWM_REMOTE_QUERY".to_string(),
                "Visual Studio Code".to_string(),
            )],
            "echo ok",
        )
        .unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "ControlMaster=auto"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "ControlPersist=10m"]));
        assert!(args
            .iter()
            .any(|arg| arg.starts_with("ControlPath=") && arg.ends_with("%C")));
        assert!(args.contains(&"host.example".to_string()));
        assert!(args.contains(&"KITTWM_REMOTE_QUERY='Visual Studio Code'".to_string()));
        assert_eq!(shell_quote("it's ok"), "'it'\\''s ok'");
    }

    #[test]
    fn apps_scene_backdrop_label_builds_directly() {
        assert_eq!(
            apps_scene_backdrop_label(2, 1, 10, "term", "zsh", "/bin/zsh"),
            "kittwm-apps-backdrop:path_count=2:macos_count=1:limit=10:filter=term:default=zsh:resolved=/bin/zsh"
        );
    }

    #[test]
    fn apps_scene_row_label_builds_directly() {
        assert_eq!(
            apps_scene_row_label("path", "xterm"),
            "kittwm-app-row:path:xterm"
        );
        assert_eq!(
            apps_scene_row_label("macos", "Terminal"),
            "kittwm-app-row:macos:Terminal"
        );
    }

    #[test]
    fn apps_scene_rows_saturate_before_clamping() {
        assert_eq!(apps_scene_rows(0, 0), 8);
        assert_eq!(apps_scene_rows(1, 0), 8);
        assert_eq!(apps_scene_rows(20, 2), 29);
        assert_eq!(apps_scene_rows(usize::MAX, usize::MAX), 30);
    }

    #[test]
    fn apps_scene_row_rects_fit_narrow_widths() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_INFO_COLS", "8");
        let summary = AppsSummary {
            default_cmd: "xterm".to_string(),
            default_resolved: Some("/usr/bin/xterm".to_string()),
            filter: Some("term".to_string()),
            limit: 5,
            path_commands: vec!["xterm".to_string(), "alacritty".to_string()],
            macos_apps: vec!["Terminal".to_string()],
        };
        let scene = apps_scene(&summary);
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if layer
                .label
                .as_deref()
                .unwrap_or_default()
                .contains("kittwm-app-row:")
            {
                let Node::Rect { rect, .. } = &layer.root else {
                    panic!("expected row rect");
                };
                assert!(rect.origin.0 >= 0.0, "{rect:?}");
                assert!(rect.width >= 1.0, "{rect:?}");
                assert!(
                    rect.origin.0 + rect.width <= width + 0.01,
                    "{rect:?} > {width}"
                );
            }
        }
        std::env::remove_var("KITTWM_INFO_COLS");
    }

    #[test]
    fn apps_scene_labels_launcher_candidates() {
        let summary = AppsSummary {
            default_cmd: "xterm".to_string(),
            default_resolved: Some("/usr/bin/xterm".to_string()),
            filter: Some("term".to_string()),
            limit: 5,
            path_commands: vec!["xterm".to_string(), "alacritty".to_string()],
            macos_apps: vec!["Terminal".to_string()],
        };
        let scene = apps_scene(&summary);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-apps-backdrop:path_count=2:macos_count=1:limit=5:filter=term:default=xterm:resolved=/usr/bin/xterm"
            )),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-app-row:path:xterm")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-app-row:macos:Terminal")),
            "{labels:?}"
        );
    }

    #[test]
    fn apps_scene_clips_pathological_label_fields() {
        let summary = AppsSummary {
            default_cmd: "default-command-with-a-pathologically-long-name".to_string(),
            default_resolved: Some(
                "/very/long/path/to/default-command-that-would-bloat-labels".to_string(),
            ),
            filter: Some("filter-value-that-is-pathologically-long".to_string()),
            limit: 5,
            path_commands: vec![
                "path-command-name-that-is-pathologically-long-and-bloats-labels".to_string(),
            ],
            macos_apps: vec![
                "macOS Application Name That Is Pathologically Long And Bloats Labels".to_string(),
            ],
        };
        let scene = apps_scene(&summary);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-apps-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("filter=filter-value-that-is-pathologic…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("default=default-command-with-a-pathologically-long-name"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("resolved=/very/long/path/to/default-command-that-would-b…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 220, "{backdrop}");
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-app-row:path:path-command-name-that-is-pathologically-long-a…")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-app-row:macos:macOS Application Name That Is Pathologically L…")),
            "{labels:?}"
        );
    }

    #[test]
    fn status_scene_backdrop_label_builds_directly() {
        assert_eq!(
            status_scene_backdrop_label("1234", 2, 1, "native-2", "rows", "dev"),
            "kittwm-status-backdrop:pid=1234:panes=2:pending=1:focus=native-2:layout=rows:workspace=dev"
        );
    }

    #[test]
    fn status_scene_heading_label_builds_directly() {
        assert_eq!(
            status_scene_heading_label("/tmp/kittwm.sock"),
            "kittwm-status-heading:sock=/tmp/kittwm.sock"
        );
    }

    #[test]
    fn status_scene_width_respects_narrow_columns() {
        assert_eq!(status_scene_cols_from_sources(Some("8"), None), 8);
        assert_eq!(status_scene_cols_from_sources(Some("0"), None), 72);
        assert_eq!(status_scene_cols_from_sources(Some("240"), None), 140);
        assert_eq!(status_scene_cols_from_sources(None, Some(100)), 100);
        assert_eq!(status_scene_cols_from_sources(Some("0"), Some(100)), 100);
        assert_eq!(status_scene_cols_from_sources(None, Some(u16::MAX)), 140);

        let status = serde_json::json!({
            "pid": 1234,
            "panes": 2,
            "focus": "native-2",
            "layout": "rows"
        });
        let scene = status_scene_for_cols(&status, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
        assert_eq!(status_scene_row_rect(8.0, 0.0).origin.0, 2.0);
    }

    #[test]
    fn status_scene_labels_daemon_snapshot() {
        let status = serde_json::json!({
            "pid": 1234,
            "uptime_s": 55,
            "sock": "/tmp/kittwm.sock",
            "panes": 2,
            "pending": 1,
            "focus": "native-2",
            "layout": "rows",
            "workspace": " dev "
        });
        let scene = status_scene(&status);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-status-backdrop:pid=1234:panes=2:pending=1:focus=native-2:layout=rows:workspace=dev"
            )),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-status-heading:sock=/tmp/kittwm.sock")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-status-row:4:focus=native-2")),
            "{labels:?}"
        );
    }

    #[test]
    fn status_scene_bounds_pathological_label_fields() {
        let status = serde_json::json!({
            "pid": 1234,
            "uptime_s": 55,
            "sock": "/tmp/kittwm/".to_string() + &"sock".repeat(40),
            "panes": 2,
            "pending": 1,
            "focus": "native-window-with-a-pathologically-long-focus-id",
            "layout": "layout-name-that-is-pathologically-long",
            "workspace": "workspace-name-that-is-pathologically-long"
        });
        let scene = status_scene_for_cols(&status, 8);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-status-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("focus=native-window-with-a-pathologic…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("layout=layout-name-that-is-pathologica…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("workspace=workspace-name-that-is-patholog…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 180, "{backdrop}");
        let heading = labels
            .iter()
            .find(|label| label.starts_with("kittwm-status-heading:"))
            .unwrap();
        assert!(heading.ends_with('…'), "{heading}");
        assert!(heading.len() < 80, "{heading}");
        let focus_row = labels
            .iter()
            .find(|label| label.starts_with("kittwm-status-row:4:"))
            .unwrap();
        assert!(
            focus_row.contains("native-window-with-a-pathologic…"),
            "{focus_row}"
        );
    }

    #[test]
    fn daily_help_scenes_label_existing_quickstart_examples_and_cheat() {
        let cases = [
            ("quickstart", quickstart_text(), "open a terminal pane"),
            ("examples", examples_text(), "kittwm spawn htop"),
            ("cheat", cheat_text(), "kittwm line focused 'cargo test'"),
        ];
        for (kind, text, needle) in cases {
            let scene = daily_help_scene(kind, text);
            let labels = scene
                .layers
                .iter()
                .filter_map(|layer| layer.label.as_deref())
                .collect::<Vec<_>>();
            let backdrop_prefix =
                daily_help_test_label_prefix("kittwm-daily-help-backdrop:", kind, ":");
            assert!(
                labels
                    .iter()
                    .any(|label| label.starts_with(&backdrop_prefix)),
                "{kind}: {labels:?}"
            );
            let heading_prefix =
                daily_help_test_label_prefix("kittwm-daily-help-heading:", kind, ":");
            assert!(
                labels.iter().any(|label| label.contains(&heading_prefix)),
                "{kind}: {labels:?}"
            );
            assert!(
                labels.iter().any(|label| label.contains(needle)),
                "{kind}: {labels:?}"
            );
        }
    }

    #[test]
    fn daily_help_test_label_prefix_builds_directly() {
        let prefix = daily_help_test_label_prefix("kittwm-daily-help-backdrop:", "quickstart", ":");
        assert_eq!(prefix, "kittwm-daily-help-backdrop:quickstart:");
        assert!(prefix.capacity() >= prefix.len());
    }

    fn daily_help_test_label_prefix(prefix: &str, kind: &str, suffix: &str) -> String {
        let mut label = String::with_capacity(prefix.len() + kind.len() + suffix.len());
        label.push_str(prefix);
        label.push_str(kind);
        label.push_str(suffix);
        label
    }

    #[test]
    fn daily_guides_include_ssh_remote_workflows() {
        for text in [quickstart_text(), examples_text(), cheat_text()] {
            assert!(text.contains("kittwm doctor --remote"), "{text}");
            assert!(text.contains("kittwm apps --remote"), "{text}");
            assert!(text.contains("kittwm-terminal --remote"), "{text}");
        }
        assert!(quickstart_text().contains("kittwm --list-displays --remote"));
        assert!(examples_text().contains("kittwm --list-windows --remote"));
    }

    fn daily_help_stress_text() -> String {
        let heading = "heading-".repeat(1024);
        let row = "row-".repeat(2048);
        let mut text = String::with_capacity(
            heading.len() + "kittwm ".len() + row.len() + "plain text".len() + 2,
        );
        text.push_str(&heading);
        text.push('\n');
        text.push_str("kittwm ");
        text.push_str(&row);
        text.push('\n');
        text.push_str("plain text");
        text
    }

    #[test]
    fn daily_help_stress_text_builds_directly() {
        let text = daily_help_stress_text();
        assert!(text.starts_with("heading-heading-"));
        assert!(text.contains("\nkittwm row-row-"));
        assert!(text.ends_with("\nplain text"));
        assert_eq!(text.capacity(), text.len());
    }

    #[test]
    fn daily_help_scene_labels_clip_pathological_payloads() {
        let kind = "kind-".repeat(1024);
        let text = daily_help_stress_text();
        let scene = daily_help_scene(&kind, &text);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.iter().any(|label| label.contains('…')), "{labels:?}");
        assert!(
            labels
                .iter()
                .all(|label| !label.contains(&"kind-".repeat(32))),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .all(|label| !label.contains(&"heading-".repeat(16))),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .all(|label| !label.contains(&"row-".repeat(32))),
            "{labels:?}"
        );
    }

    fn sample_config_summary() -> ConfigSummary {
        ConfigSummary {
            config_path: "/tmp/kittwm/config.yaml".to_string(),
            background_color: "nord0".to_string(),
            background_opacity: 0.6,
            background_effects: 1,
            colorscheme_name: "nord".to_string(),
            colorscheme_fg: "#d8dee9".to_string(),
            colorscheme_bg: "#2e3440".to_string(),
            colorscheme_colors: 16,
            terminal_backend: "ghostty".to_string(),
            terminal_command: "<shell>".to_string(),
            libghostty_theme: "nord".to_string(),
            libghostty_background: "nord0".to_string(),
            libghostty_opacity: 0.72,
            libghostty_kitty_graphics: true,
            hidpi_enabled: true,
            cell_width_px: 16,
            cell_height_px: 32,
            tile_gap_px: 10,
            tile_gap_cols: 1,
            tile_gap_rows: 1,
            header_gap_px: 8,
            header_gap_rows: 1,
            footer_gap_px: 6,
            footer_gap_rows: 1,
            keymap_path: "<default>".to_string(),
            launch_cmd: "<default: xterm>".to_string(),
            launch_query: "<unset>".to_string(),
            launcher_overlay: "1".to_string(),
            prefix: "C-a".to_string(),
            bindings: 12,
            duplicate_chords: 0,
            status: "ok",
        }
    }

    #[test]
    fn config_scene_rows_fit_narrow_width() {
        let summary = sample_config_summary();
        let scene = config_scene_for_cols(&summary, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
    }

    #[test]
    fn config_scene_backdrop_label_builds_directly() {
        let label = config_scene_backdrop_label("<default>", 12, 0, "ok");
        assert_eq!(
            label,
            "kittwm-config-backdrop:keymap=<default>:bindings=12:duplicates=0:status=ok"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn config_scene_labels_readiness_summary() {
        let summary = sample_config_summary();
        let scene = config_scene(&summary);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-config-backdrop:keymap=<default>:bindings=12:duplicates=0:status=ok"
            )),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:1:background.color=nord0")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:4:colorscheme.name=nord")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:8:terminal.backend=ghostty")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:10:libghostty.opacity=0.72")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:11:display.hidpi=true")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:12:display.cell_px=16x32")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:13:display.tile_gap=10px=1x1cells")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:20:prefix=C-a")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-config-row:23:status=ok")),
            "{labels:?}"
        );
    }

    #[test]
    fn config_scene_clips_pathological_label_fields() {
        let mut summary = sample_config_summary();
        summary.config_path =
            "/very/long/path/to/kittwm/config/that/would/bloat/scene/labels.yaml".to_string();
        summary.background_color = "background-color-name-that-is-pathologically-long".to_string();
        summary.colorscheme_name = "colorscheme-name-that-is-pathologically-long".to_string();
        summary.colorscheme_colors = usize::MAX;
        summary.keymap_path =
            "/very/long/path/to/keymap/that/would/bloat/scene/labels.yaml".to_string();
        summary.launch_cmd =
            "launcher-command --with --pathologically --long --arguments".to_string();
        summary.launch_query =
            "query-value-that-is-pathologically-long-and-would-bloat-labels".to_string();
        summary.launcher_overlay =
            "overlay-value-that-is-pathologically-long-and-would-bloat-labels".to_string();
        summary.prefix = "prefix-value-that-is-pathologically-long".to_string();
        let scene = config_scene_for_cols(&summary, 8);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-config-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("keymap=/very/long/path/to/keymap/that/would/bloat/scen…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 140, "{backdrop}");
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-config-row:0:config_path=/very/long/path/to/kittwm/config/that/would/blo…"
            )),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-config-row:17:launch_cmd=launcher-command --with --pathologically --long…"
            )),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-config-row:18:launch_query=query-value-that-is-pathologically-long-and-wou…")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-config-row:20:prefix=prefix-value-that-is-pathologic…")),
            "{labels:?}"
        );
    }

    #[test]
    fn keymap_scene_backdrop_label_builds_directly() {
        assert_eq!(
            keymap_scene_backdrop_label(12, "C-a", 1),
            "kittwm-keymap-backdrop:bindings=12:prefix=C-a:duplicates=1"
        );
    }

    #[test]
    fn keymap_scene_row_label_builds_directly() {
        assert_eq!(
            keymap_scene_row_label(2, "C-a c", "workspace.new"),
            "kittwm-keymap-row:2:C-a c:workspace.new"
        );
    }

    #[test]
    fn keymap_scene_labels_prefix_bindings_and_actions() {
        let km = kittui_cli::keymap::default_keymap();
        let scene = keymap_scene(&km);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-keymap-backdrop:bindings=")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("prefix=C-a")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("launch") || label.contains("split.vertical.launcher")),
            "{labels:?}"
        );
    }

    #[test]
    fn keymap_duplicate_test_alternate_key_builds_directly() {
        let key = keymap_duplicate_test_alternate_key("C-a");
        assert_eq!(key, "C-a-other");
        assert_eq!(key.capacity(), key.len());
    }

    fn keymap_duplicate_test_alternate_key(base: &str) -> String {
        let mut key = String::with_capacity(base.len() + "-other".len());
        key.push_str(base);
        key.push_str("-other");
        key
    }

    #[test]
    fn keymap_duplicate_count_uses_chord_identity() {
        let large_key = "pathologically-long-key-name-".repeat(128);
        let chord = vec![kittui_cli::keymap::KeySpec {
            mods: kittui_cli::keymap::KeyMods {
                ctrl: true,
                alt: false,
                shift: false,
            },
            key: large_key.clone(),
        }];
        let km = kittui_cli::keymap::Keymap {
            prefix: None,
            bindings: vec![
                kittui_cli::keymap::Binding {
                    chord: chord.clone(),
                    action: kittui_cli::keymap::Action::Launch,
                },
                kittui_cli::keymap::Binding {
                    chord,
                    action: kittui_cli::keymap::Action::Quit,
                },
                kittui_cli::keymap::Binding {
                    chord: vec![kittui_cli::keymap::KeySpec {
                        mods: kittui_cli::keymap::KeyMods {
                            ctrl: true,
                            alt: false,
                            shift: false,
                        },
                        key: keymap_duplicate_test_alternate_key(&large_key),
                    }],
                    action: kittui_cli::keymap::Action::WorkspaceNext,
                },
            ],
        };
        assert_eq!(keymap_duplicate_count(&km), 1);
    }

    #[test]
    fn keymap_scene_clips_pathological_label_fields() {
        let km = kittui_cli::keymap::Keymap {
            prefix: Some(kittui_cli::keymap::KeySpec {
                mods: kittui_cli::keymap::KeyMods {
                    ctrl: true,
                    alt: true,
                    shift: true,
                },
                key: "prefix-key-that-is-pathologically-long".to_string(),
            }),
            bindings: vec![kittui_cli::keymap::Binding {
                chord: vec![kittui_cli::keymap::KeySpec {
                    mods: kittui_cli::keymap::KeyMods {
                        ctrl: true,
                        alt: true,
                        shift: true,
                    },
                    key: "binding-key-that-is-pathologically-long-and-would-bloat-labels"
                        .to_string(),
                }],
                action: kittui_cli::keymap::Action::Custom(
                    "custom.action.with.pathologically.long.name.and.extra.suffix".to_string(),
                ),
            }],
        };
        let scene = keymap_scene(&km);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-keymap-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("prefix=C-M-S-prefix-key-that-is-pathol…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 90, "{backdrop}");
        let row = labels
            .iter()
            .find(|label| label.starts_with("kittwm-keymap-row:0:"))
            .unwrap();
        assert!(
            row.contains("C-M-S-binding-key-that-is-pathologically-long-a…"),
            "{row}"
        );
        assert!(
            row.contains("custom.action.with.pathologically.long.name.and…"),
            "{row}"
        );
        assert!(row.len() < 130, "{row}");

        let key_label = keymap_keyspec_label(km.prefix.as_ref().unwrap(), 32);
        assert_eq!(key_label.chars().count(), 32);
        assert!(key_label.capacity() >= 32);
        let chord_label = keymap_chord_label(&km.bindings[0].chord, 48);
        assert_eq!(chord_label.chars().count(), 48);
        assert!(chord_label.capacity() >= 48);
    }

    #[test]
    fn commands_scene_row_label_builds_directly() {
        assert_eq!(
            commands_scene_row_label("help", "commands-kitty", "Render command catalog"),
            "kittwm-command-row:help:commands-kitty:Render command catalog"
        );
    }

    #[test]
    fn commands_backdrop_label_builds_directly() {
        let label = commands_backdrop_label(12, "help=3,lifecycle=2");
        assert_eq!(
            label,
            "kittwm-commands-backdrop:count=12:categories=help=3,lifecycle=2"
        );
        assert_eq!(
            label.capacity(),
            "kittwm-commands-backdrop:count=:categories=".len() + 20 + "help=3,lifecycle=2".len()
        );
    }

    #[test]
    fn commands_scene_labels_catalog_categories_and_rows() {
        let scene = commands_scene();
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("kittwm-commands-backdrop:count=")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("help=")),
            "{labels:?}"
        );
        let mut by_category = std::collections::BTreeMap::new();
        by_category.insert("help", 3usize);
        by_category.insert("lifecycle", 2usize);
        let summary = command_category_summary_label(&by_category);
        assert_eq!(summary, "help=3,lifecycle=2");
        assert!(summary.capacity() >= 32);
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-command-row:help:commands-kitty")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-command-row:lifecycle:start")),
            "{labels:?}"
        );
    }

    #[test]
    fn shortcuts_scene_row_label_builds_directly() {
        let label = shortcuts_scene_row_label("launch_terminal", "C-a Enter", "launch terminal");
        assert_eq!(
            label,
            "kittwm-shortcut-row:launch_terminal:C-a Enter:launch terminal"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn shortcuts_scene_backdrop_label_builds_directly() {
        let label = shortcuts_scene_backdrop_label(12);
        assert_eq!(label, "kittwm-shortcuts-backdrop:count=12");
        assert_eq!(
            label.capacity(),
            "kittwm-shortcuts-backdrop:count=".len() + 20
        );
    }

    #[test]
    fn shortcuts_scene_rows_saturate_large_catalog_counts() {
        assert_eq!(shortcuts_scene_rows(0), 4);
        assert_eq!(shortcuts_scene_rows(3), 6);
        assert_eq!(shortcuts_scene_rows(usize::MAX), 18);
        assert_eq!(
            shortcuts_scene_entry_limit(shortcuts_scene_rows(usize::MAX)),
            16
        );
    }

    #[test]
    fn shortcuts_scene_width_respects_narrow_columns() {
        assert_eq!(shortcuts_scene_cols_from_sources(Some("8"), None), 8);
        assert_eq!(shortcuts_scene_cols_from_sources(Some("0"), None), 72);
        assert_eq!(shortcuts_scene_cols_from_sources(Some("240"), None), 140);
        assert_eq!(shortcuts_scene_cols_from_sources(None, Some(100)), 100);
        assert_eq!(shortcuts_scene_cols_from_sources(Some("0"), Some(100)), 100);
        assert_eq!(shortcuts_scene_cols_from_sources(None, Some(u16::MAX)), 140);

        let scene = shortcuts_scene_for_cols(1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
        assert_eq!(shortcuts_scene_row_rect(8.0, 0.0).origin.0, 2.0);
    }

    #[test]
    fn shortcuts_scene_labels_shared_shortcut_catalog() {
        let scene = shortcuts_scene();
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("kittwm-shortcuts-backdrop:count=")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("launch_terminal:C-a Enter")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("open_launcher:C-a g")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("title_markers:title markers")),
            "{labels:?}"
        );
    }

    #[test]
    fn shortcuts_scene_visible_entries_keep_title_marker_legend() {
        let entries = kittui_cli::shortcuts::NATIVE_SHORTCUT_ENTRIES;
        let visible = shortcuts_scene_visible_entries(entries, 12);
        assert_eq!(visible.len(), 12);
        assert!(visible.iter().any(|entry| entry.id == "title_markers"));
        assert_eq!(visible.last().map(|entry| entry.id), Some("title_markers"));

        let visible = shortcuts_scene_visible_entries(entries, shortcuts_scene_entry_limit(18));
        assert_eq!(visible.len(), 16);
        assert!(visible.iter().any(|entry| entry.id == "title_markers"));
        assert_eq!(visible.last().map(|entry| entry.id), Some("title_markers"));
    }

    #[test]
    fn architecture_scene_rows_saturate_large_contract_counts() {
        assert_eq!(architecture_scene_rows(0, 0, 0), 10);
        assert_eq!(architecture_scene_rows(2, 3, 4), 15);
        assert_eq!(
            architecture_scene_rows(usize::MAX, usize::MAX, usize::MAX),
            30
        );
    }

    #[test]
    fn architecture_scene_row_rects_fit_narrow_widths() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_INFO_COLS", "8");
        let contract = kittwm_sdk::ArchitectureContract::current();
        let scene = architecture_scene(&contract);
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if layer
                .label
                .as_deref()
                .unwrap_or_default()
                .contains("kittwm-architecture-")
            {
                if let Node::Rect { rect, .. } = &layer.root {
                    assert!(rect.origin.0 >= 0.0, "{rect:?}");
                    assert!(rect.width >= 1.0, "{rect:?}");
                    assert!(
                        rect.origin.0 + rect.width <= width + 0.01,
                        "{rect:?} > {width}"
                    );
                }
            }
        }
        std::env::remove_var("KITTWM_INFO_COLS");
    }

    #[test]
    fn architecture_surface_label_builds_directly() {
        let label = architecture_surface_label(
            "kittwm-browser",
            "browser",
            true,
            true,
            "HeadlessBrowserApp",
        );
        assert_eq!(
            label,
            "kittwm-architecture-surface:kittwm-browser:kind=browser:sdk=true:kitty=true:kittui=HeadlessBrowserApp"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn architecture_plane_label_builds_directly() {
        let label = architecture_plane_label("decorations", 20);
        assert_eq!(label, "kittwm-architecture-plane:decorations:z=20");
        assert_eq!(
            label.capacity(),
            "kittwm-architecture-plane::z=".len() + "decorations".len() + 12
        );
    }

    #[test]
    fn architecture_layer_label_builds_directly() {
        assert_eq!(
            architecture_layer_label("tiling-engine", "kittui-wm", 2, 1, 3),
            "kittwm-architecture-layer:tiling-engine:owner=kittui-wm:responsibilities=2:must_not=1:native_contracts=3"
        );
    }

    #[test]
    fn architecture_backdrop_label_builds_directly() {
        let label = architecture_backdrop_label(3, 4, 5, 1);
        assert_eq!(
            label,
            "kittwm-architecture-backdrop:layers=3:planes=4:surfaces=5:schema=1"
        );
        assert_eq!(
            label.capacity(),
            "kittwm-architecture-backdrop:layers=:planes=:surfaces=:schema=".len() + 80
        );
    }

    #[test]
    fn architecture_scene_heading_label_builds_directly() {
        let label = architecture_scene_heading_label("kittwm-v2");
        assert_eq!(label, "kittwm-architecture-heading:kittwm-v2");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn architecture_scene_labels_layers_planes_and_surfaces() {
        let contract = kittwm_sdk::ArchitectureContract::current();
        let scene = architecture_scene(&contract);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-architecture-backdrop:layers=")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-architecture-layer:tiling-engine")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-architecture-plane:decorations:z=20")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-architecture-surface:kittwm-bar:kind=chrome")),
            "{labels:?}"
        );
    }

    #[test]
    fn helper_scene_placement_options_avoid_unicode_placeholders() {
        let decoration =
            kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Decoration);
        assert!(!decoration.unicode_placeholder);
        assert_eq!(
            decoration.z_index,
            kittwm_z_index(kittwm_sdk::SurfacePlacementRole::Decoration)
        );
        let overlay = kittwm_scene_placement_options(kittwm_sdk::SurfacePlacementRole::Overlay);
        assert!(!overlay.unicode_placeholder);
        assert_eq!(
            overlay.z_index,
            kittwm_z_index(kittwm_sdk::SurfacePlacementRole::Overlay)
        );
    }

    #[test]
    fn native_surfaces_text_reports_sdk_and_kitty_native_coverage() {
        let text = native_surfaces_text();
        assert!(text.contains("kittwm native surfaces"), "{text}");
        assert!(text.contains("all ready: yes"), "{text}");
        assert!(text.contains("kittwm-terminal"), "{text}");
        assert!(text.contains("kind:terminal"), "{text}");
        assert!(text.contains("kittwm-browser"), "{text}");
        assert!(text.contains("SurfaceSpec::browser"), "{text}");
        assert!(
            text.contains("Runtime::place_png_frame_with_options"),
            "{text}"
        );
        assert!(text.contains("kittwm-bar"), "{text}");
        assert!(text.contains("BarModel::scene"), "{text}");
    }

    #[test]
    fn native_surfaces_scene_row_rects_fit_narrow_widths() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_INFO_COLS", "8");
        let contract = kittwm_sdk::ArchitectureContract::current();
        let scene = native_surfaces_scene(&contract);
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if layer
                .label
                .as_deref()
                .unwrap_or_default()
                .contains("kittwm-native-surface-row:")
            {
                let Node::Rect { rect, .. } = &layer.root else {
                    panic!("expected row rect");
                };
                assert!(rect.origin.0 >= 0.0, "{rect:?}");
                assert!(rect.width >= 1.0, "{rect:?}");
                assert!(
                    rect.origin.0 + rect.width <= width + 0.01,
                    "{rect:?} > {width}"
                );
            }
        }
        std::env::remove_var("KITTWM_INFO_COLS");
    }

    #[test]
    fn native_browser_default_out_path_builds_directly() {
        let path = native_browser_default_out_path(42);
        assert_eq!(path, "/tmp/kittwm-native-browser-42.png");
        assert!(path.capacity() >= "/tmp/kittwm-native-browser-".len() + 10 + ".png".len());
    }

    #[test]
    fn record_paths_build_directly() {
        let dir = kittwm_record_default_out_dir(12345);
        assert_eq!(dir, "/tmp/kittwm-record-12345");
        assert!(dir.capacity() >= dir.len());

        let frame = kittwm_record_frame_path(&dir, 7, 3);
        assert_eq!(frame, "/tmp/kittwm-record-12345/frame-00007-win3.png");
        assert!(frame.capacity() >= frame.len());

        let apng = kittwm_record_apng_path(&dir);
        assert_eq!(apng, "/tmp/kittwm-record-12345/kittwm.apng");
        assert_eq!(apng.capacity(), apng.len());
    }

    #[test]
    fn native_surface_row_label_builds_directly() {
        let label = native_surface_row_label(
            2,
            "kittwm-browser",
            "browser",
            true,
            true,
            true,
            "app-surfaces",
            "0",
            "HeadlessBrowserApp",
        );
        assert_eq!(
            label,
            "kittwm-native-surface-row:2:kittwm-browser:kind=browser:ready=true:sdk=true:kitty=true:plane=app-surfaces:z=0:kittui=HeadlessBrowserApp"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn native_surfaces_backdrop_label_builds_directly() {
        let label = native_surfaces_backdrop_label(4, true);
        assert_eq!(
            label,
            "kittwm-native-surfaces-backdrop:count=4:all_ready=true"
        );
        assert_eq!(
            label.capacity(),
            "kittwm-native-surfaces-backdrop:count=:all_ready=".len() + 25
        );
    }

    #[test]
    fn native_surfaces_scene_labels_sdk_kittui_kitty_coverage() {
        let contract = kittwm_sdk::ArchitectureContract::current();
        let scene = native_surfaces_scene(&contract);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label
                    .contains("kittwm-native-surfaces-backdrop:count=4:all_ready=true")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-native-surface-row:0:kittwm-terminal:kind=terminal:ready=true")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-bar:kind=chrome") && label.contains("z=20")),
            "{labels:?}"
        );
    }

    #[test]
    fn native_surfaces_json_reports_sdk_and_kitty_native_coverage() {
        let text = native_surfaces_json_text();
        assert!(text.ends_with('\n'));
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["kind"], "kittwm-native-surface-coverage");
        assert_eq!(json["all_ready"], true);
        let surfaces = json["surfaces"].as_array().unwrap();
        assert!(surfaces.iter().any(|surface| {
            surface["name"] == "kittwm-terminal"
                && surface["sdk_backed"] == true
                && surface["kitty_graphics_native"] == true
        }));
        assert!(surfaces.iter().any(|surface| {
            surface["name"] == "kittwm-browser"
                && surface["sdk_entry"] == "SurfaceSpec::browser"
                && surface["kittui_entry"]
                    == "HeadlessBrowserApp -> Runtime::place_png_frame_with_options"
        }));
        assert!(surfaces.iter().any(|surface| {
            surface["name"] == "kittwm-bar"
                && surface["surface_kind"] == "chrome"
                && surface["kittui_entry"] == "BarModel::scene -> Runtime::place_at_with_options"
        }));
    }

    #[test]
    fn architecture_contract_names_clean_wm_boundaries() {
        let json_text = architecture_contract_json_text();
        assert!(json_text.ends_with('\n'));
        assert_eq!(json_text.matches('\n').count(), 1);
        assert!(json_text.capacity() >= json_text.len());
        let json: serde_json::Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(json["kind"], "kittwm-architecture-contract");
        let layer_ids = json["layers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|layer| layer["id"].as_str())
            .collect::<Vec<_>>();
        for expected in [
            "sdk-control-plane",
            "tiling-engine",
            "surface-renderer",
            "decoration-renderer",
            "kitty-compositor",
        ] {
            assert!(layer_ids.contains(&expected), "{layer_ids:?}");
        }
        assert!(json["composition_order"]
            .as_array()
            .unwrap()
            .iter()
            .any(|plane| plane["plane"] == "decorations" && plane["z_index"] == 20));
        assert!(json["first_party_native_surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|surface| surface["name"] == "kittwm-browser"
                && surface["sdk_entry"] == "SurfaceSpec::browser"));
    }

    #[test]
    fn quickstart_teaches_daily_driver_path() {
        let text = quickstart_text();
        assert!(text.contains("kittwm quickstart"), "{text}");
        assert!(text.contains("C-a Enter"), "{text}");
        assert!(!text.contains("C-a Enter / C-a t"), "{text}");
        assert!(text.contains("C-a t"), "{text}");
        assert!(text.contains("toggle floating mode"), "{text}");
        assert!(text.contains("C-a f"), "{text}");
        assert!(text.contains("toggle fullscreen"), "{text}");
        assert!(text.contains("C-a e"), "{text}");
        assert!(text.contains("toggle current split"), "{text}");
        assert!(text.contains("C-a g"), "{text}");
        assert!(text.contains("kittwm info"), "{text}");
        assert!(text.contains("kittwm spawn htop"), "{text}");
        assert!(
            text.contains("kittwm paste focused 'multi-line text'"),
            "{text}"
        );
        assert!(
            text.contains("kittwm --paste-bytes-b64 focused cGFzdGUgbWU="),
            "{text}"
        );
        assert!(
            text.contains("kittwm-launch --browser https://example.com"),
            "{text}"
        );
        assert!(text.contains("kittwm-terminal --events-ms 1000"), "{text}");
        assert!(text.contains("kittwm-top --json"), "{text}");
        assert!(text.contains("kittwm-bar --reserve --kitty"), "{text}");
        assert!(
            text.contains("kittwm-browser --semantic-snapshot https://example.com"),
            "{text}"
        );
        assert!(text.contains("kittwm examples"), "{text}");
        assert!(text.contains("kittwm cheat"), "{text}");
        assert!(text.contains("kittwm help topics"), "{text}");
        assert!(text.contains("kittwm help completions"), "{text}");
        assert!(
            text.contains("kittwm completions bash >> ~/.bashrc"),
            "{text}"
        );
        assert!(
            text.contains("kittwm completions zsh >> ~/.zshrc"),
            "{text}"
        );
        assert!(
            text.contains("mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish"),
            "{text}"
        );
    }

    #[test]
    fn examples_are_copy_paste_daily_driver_commands() {
        let text = examples_text();
        for line in [
            "kittwm completions bash >> ~/.bashrc",
            "kittwm completions zsh >> ~/.zshrc",
            "mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish",
            "kittwm info",
            "kittwm spawn htop",
            "kittwm line focused 'cargo test -p kittui-cli'",
            "kittwm paste focused 'multi-line text'",
            "kittwm --send-bytes-b64 focused aGkKAA==",
            "kittwm --paste-bytes-b64 focused cGFzdGUgbWU=",
            "kittwm --paste-file focused -",
            "kittwm --wait-output-json-ms 10000 focused 'build finished'",
            "kittwm balance",
            "kittwm --save-session session.json",
            "kittwm-launch --browser https://example.com",
            "kittwm-terminal --events-ms 1000",
            "kittwm-top --json",
            "kittwm-bar --reserve --kitty",
            "kittwm-browser --semantic-snapshot https://example.com",
            "kittwm help panes",
        ] {
            assert!(text.contains(line), "missing {line}: {text}");
        }
    }

    #[test]
    fn cheat_sheet_is_compact_daily_reference() {
        let text = cheat_text();
        assert!(text.contains("C-a Enter"), "{text}");
        assert!(!text.contains("C-a Enter/t"), "{text}");
        assert!(text.contains("C-a t float"), "{text}");
        assert!(text.contains("C-a f full"), "{text}");
        assert!(text.contains("C-a e split-toggle"), "{text}");
        assert!(text.contains("C-a g launcher"), "{text}");
        assert!(text.contains("kittwm info"), "{text}");
        assert!(text.contains("kittwm spawn htop"), "{text}");
        assert!(text.contains("kittwm balance"), "{text}");
        assert!(text.contains("kittwm wait focused 'Finished'"), "{text}");
        assert!(text.contains("kittwm-launch --browser URL"), "{text}");
        assert!(text.contains("kittwm-terminal --events-ms 1000"), "{text}");
        assert!(text.contains("kittwm-top --json"), "{text}");
        assert!(text.contains("kittwm-bar --reserve --kitty"), "{text}");
        assert!(
            text.contains("kittwm-browser --semantic-snapshot URL"),
            "{text}"
        );
        assert!(text.contains("kittwm help completions"), "{text}");
        assert!(
            text.contains("kittwm completions bash >> ~/.bashrc"),
            "{text}"
        );
        assert!(
            text.contains("kittwm completions zsh >> ~/.zshrc"),
            "{text}"
        );
        assert!(
            text.contains("mkdir -p ~/.config/fish/completions && kittwm completions fish > ~/.config/fish/completions/kittwm.fish"),
            "{text}"
        );
        assert!(
            text.lines().count() < quickstart_text().lines().count(),
            "{text}"
        );
    }

    fn sample_doctor_display_tuning() -> kittui_cli::session::NativeDisplayTuning {
        kittui_cli::session::NativeDisplayTuning {
            hidpi_enabled: true,
            cell_width_px: 16,
            cell_height_px: 32,
            tile_gap_px: 25,
            header_gap_px: 49,
            footer_gap_px: 1,
            tile_gap_cols: 2,
            tile_gap_rows: 1,
            header_gap_rows: 2,
            footer_gap_rows: 1,
        }
    }

    #[test]
    fn doctor_socket_hint_builds_reachable_and_missing_variants_directly() {
        let path = std::path::Path::new("/tmp/kittwm-test.sock");
        assert_eq!(
            doctor_socket_hint(path, true),
            "running WM detected at /tmp/kittwm-test.sock; inspect it with `kittwm info`, `kittwm panes`, or `kittwm events 1000`."
        );
        assert_eq!(
            doctor_socket_hint(path, false),
            "no running WM socket at /tmp/kittwm-test.sock; start one with `kittwm`, then inspect with `kittwm info`."
        );
    }

    #[test]
    fn doctor_log_row_builds_present_and_missing_variants_directly() {
        let mut out = String::new();
        append_doctor_log_row(&mut out, "/tmp/kittui-wm.log", true, 86);
        append_doctor_log_row(&mut out, "/tmp/missing.log", false, 0);
        assert_eq!(
            out,
            "  log            : /tmp/kittui-wm.log (present, 86 bytes)\n  log            : /tmp/missing.log (missing)\n"
        );
    }

    #[test]
    fn doctor_wrapped_rows_indent_continuations_under_value_column() {
        let mut out = String::new();
        append_doctor_wrapped_row(
            &mut out,
            "  renderer        : ",
            "tmux detected: kittwm defaults to the pure terminal renderer",
            44,
        );
        let lines = out.lines().collect::<Vec<_>>();
        assert!(lines.len() > 1, "{out}");
        assert!(
            lines[0].starts_with("  renderer        : tmux detected:"),
            "{out}"
        );
        assert!(
            lines[1].starts_with("                    defaults"),
            "{out}"
        );
        assert!(lines.iter().all(|line| line.chars().count() <= 44), "{out}");
    }

    #[test]
    fn doctor_daily_driver_readiness_mentions_next_steps() {
        let diagnostics = TransportDiagnostics::detect(&TerminalInfo::override_with(
            Some(80),
            Some(24),
            CellSize::new(8, 16),
            false,
            false,
            kittui::Transport::Direct,
        ));
        let text = doctor_daily_driver_text(&diagnostics, false);
        assert!(text.contains("Daily driver readiness"), "{text}");
        assert!(text.contains("kittwm quickstart"), "{text}");
        assert!(text.contains("kittwm info"), "{text}");
        assert!(text.contains("KITTWM_NATIVE_RENDERER="), "{text}");
    }

    #[test]
    fn doctor_scene_respects_narrow_positive_columns() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_DOCTOR_COLS", "8");
        let diagnostics = TransportDiagnostics::detect(&TerminalInfo::override_with(
            Some(8),
            Some(6),
            CellSize::new(8, 16),
            true,
            true,
            kittui::Transport::Direct,
        ));
        let display_tuning = sample_doctor_display_tuning();
        let scene = doctor_scene(&diagnostics, true, 1, &display_tuning);
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = &layer.root {
                assert!(rect.origin.0 >= 0.0, "{rect:?}");
                assert!(rect.width >= 1.0, "{rect:?}");
                assert!(
                    rect.origin.0 + rect.width <= width + 0.01,
                    "{rect:?} > {width}"
                );
            }
        }
        std::env::set_var("KITTWM_DOCTOR_COLS", "200");
        assert_eq!(doctor_scene_cols(), 120);
        std::env::remove_var("KITTWM_DOCTOR_COLS");
    }

    #[test]
    fn kitty_probe_matched_status_builds_directly() {
        let status = kitty_probe_matched_status(&KittyResponseStatus::Ok);
        assert_eq!(status, "matched:Ok");
        assert!(status.capacity() >= status.len());
    }

    #[test]
    fn doctor_display_label_builds_directly() {
        let label = doctor_display_label("hidpi=true:cell=16x32");
        assert_eq!(label, "kittwm-doctor-display:hidpi=true:cell=16x32");
        assert_eq!(
            label.capacity(),
            "kittwm-doctor-display:".len() + "hidpi=true:cell=16x32".len()
        );
    }

    #[test]
    fn doctor_readiness_label_builds_directly() {
        let label = doctor_readiness_label("kitty-ready", false, true, 2, "log-present");
        assert_eq!(
            label,
            "kittwm-doctor-readiness:kitty-ready:tmux=false:remote=true:displays=2:log-present"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn doctor_heading_label_builds_directly() {
        let label = doctor_heading_label(
            kittui::Transport::Direct,
            kittui_core::terminal::GraphicsCompressionMode::Zlib,
        );
        assert_eq!(
            label,
            "kittwm-doctor-heading:transport=Direct:compression=Zlib"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn doctor_backdrop_label_builds_directly() {
        let label = doctor_backdrop_label("kitty-ready");
        assert_eq!(label, "kittwm-doctor-backdrop:kitty-ready");
        assert_eq!(
            label.capacity(),
            "kittwm-doctor-backdrop:".len() + "kitty-ready".len()
        );
    }

    #[test]
    fn doctor_scene_labels_transport_readiness_for_graphical_inspection() {
        let diagnostics = TransportDiagnostics::detect(&TerminalInfo::override_with(
            Some(80),
            Some(24),
            CellSize::new(8, 16),
            true,
            true,
            kittui::Transport::Direct,
        ));
        let display_tuning = sample_doctor_display_tuning();
        let scene = doctor_scene(&diagnostics, true, 2, &display_tuning);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("kittwm-doctor-backdrop:")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("transport=Direct")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("displays=2:log-present")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains(
                "kittwm-doctor-display:hidpi=true:cell=16x32:tile_gap=25px=2x1:header_gap=49px=2:footer_gap=1px=1"
            )),
            "{labels:?}"
        );
        assert_eq!(
            doctor_display_tuning_label(&display_tuning),
            "hidpi=true:cell=16x32:tile_gap=25px=2x1:header_gap=49px=2:footer_gap=1px=1"
        );
    }

    #[test]
    fn info_scene_cols_respect_narrow_positive_widths() {
        assert_eq!(info_scene_cols_from_sources(Some("1"), None), 1);
        assert_eq!(info_scene_cols_from_sources(Some("8"), None), 8);
        assert_eq!(info_scene_cols_from_sources(Some("39"), None), 39);
        assert_eq!(info_scene_cols_from_sources(Some("0"), None), 72);
        assert_eq!(info_scene_cols_from_sources(Some("240"), None), 140);
        assert_eq!(info_scene_cols_from_sources(None, None), 72);
        assert_eq!(info_scene_cols_from_sources(None, Some(100)), 100);
        assert_eq!(info_scene_cols_from_sources(Some("0"), Some(100)), 100);
        assert_eq!(info_scene_cols_from_sources(None, Some(u16::MAX)), 140);
    }

    #[test]
    fn info_scene_respects_narrow_positive_columns() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_INFO_COLS", "8");
        let status = serde_json::json!({
            "panes": 2,
            "focus": "native-2",
            "layout": "columns",
            "workspace": "dev"
        });
        let chrome = serde_json::json!({
            "workspace": "dev",
            "top_bar_rows": 1,
            "tilable_rows": 5
        });
        let panes = serde_json::json!({
            "panes_detail": [
                {"window":"native-1","title":"shell","focused":false},
                {"window":"native-2","title":"editor","focused":true}
            ]
        });
        let scene = info_scene(
            std::path::Path::new("/tmp/kittwm-test.sock"),
            &status,
            &chrome,
            &panes,
        );
        assert_eq!(scene.footprint.cols, 8);
        let width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = &layer.root {
                assert!(rect.origin.0 >= 0.0, "{rect:?}");
                assert!(rect.width >= 1.0, "{rect:?}");
                assert!(
                    rect.origin.0 + rect.width <= width + 0.01,
                    "{rect:?} > {width}"
                );
            }
        }
        std::env::set_var("KITTWM_INFO_COLS", "200");
        assert_eq!(info_scene_cols(), 140);
        std::env::remove_var("KITTWM_INFO_COLS");
    }

    #[test]
    fn info_pane_label_builds_directly() {
        let label = info_pane_label("native-2", true, "editor");
        assert_eq!(label, "kittwm-info-pane:native-2:focused=true:title=editor");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn info_chrome_label_builds_directly() {
        let label = info_chrome_label(1, 23);
        assert_eq!(label, "kittwm-info-chrome:top_bar_rows=1:tilable_rows=23");
        assert_eq!(
            label.capacity(),
            "kittwm-info-chrome:top_bar_rows=:tilable_rows=".len() + 40
        );
    }

    #[test]
    fn info_heading_label_builds_directly() {
        assert_eq!(
            info_heading_label("/tmp/kittwm.sock", "native-2", "columns"),
            "kittwm-info-heading:socket=/tmp/kittwm.sock:focus=native-2:layout=columns"
        );
    }

    #[test]
    fn info_backdrop_label_builds_directly() {
        let label = info_backdrop_label("dev", 2);
        assert_eq!(label, "kittwm-info-backdrop:workspace=dev:panes=2");
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn info_output_formats_daily_driver_snapshot() {
        let status = serde_json::json!({
            "panes": 2,
            "focus": "native-2",
            "layout": "columns",
            "workspace": "dev"
        });
        let chrome = serde_json::json!({
            "workspace": " dev ",
            "top_bar_rows": 1,
            "tilable_rows": 23
        });
        let panes = serde_json::json!({
            "panes_detail": [
                {"window":"native-1","title":"shell","focused":false,"x":0,"y":1,"cols":40,"rows":23},
                {"window":"native-2","title":"editor","focused":true,"x":40,"y":1,"cols":40,"rows":23}
            ]
        });
        let text = format_info_output(
            std::path::Path::new("/tmp/kittwm-test.sock"),
            &status,
            &chrome,
            &panes,
        );
        assert!(text.contains("kittwm info"), "{text}");
        assert!(text.contains("workspace: dev"), "{text}");
        assert!(
            text.contains("panes: 2 focus=native-2 layout=columns"),
            "{text}"
        );
        assert!(text.contains("* native-2  editor"), "{text}");
        assert!(text.contains("kittwm --spawn-pty 'htop'"), "{text}");
        assert!(text.capacity() >= text.len());

        let scene = info_scene(
            std::path::Path::new("/tmp/kittwm-test.sock"),
            &status,
            &chrome,
            &panes,
        );
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("workspace=dev:panes=2")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("focus=native-2")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("top_bar_rows=1")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-info-pane:native-2:focused=true:title=editor")),
            "{labels:?}"
        );
    }

    #[test]
    fn info_scene_clips_pathological_label_fields() {
        let status = serde_json::json!({
            "panes": 1,
            "focus": "native-window-with-a-pathologically-long-focus-id",
            "layout": "layout-name-that-is-pathologically-long",
            "workspace": "status-workspace-that-is-pathologically-long"
        });
        let chrome = serde_json::json!({
            "workspace": "workspace-name-that-is-pathologically-long",
            "top_bar_rows": 1,
            "tilable_rows": 5
        });
        let panes = serde_json::json!({
            "panes_detail": [
                {
                    "window":"native-window-with-a-pathologically-long-window-id",
                    "title":"pane-title-that-is-pathologically-long-and-would-bloat-scene-labels",
                    "focused":true
                }
            ]
        });
        let scene = info_scene(
            std::path::Path::new("/very/long/path/to/kittwm/socket/that/would/bloat/labels.sock"),
            &status,
            &chrome,
            &panes,
        );
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-info-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("workspace=workspace-name-that-is-patholog…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 80, "{backdrop}");
        let heading = labels
            .iter()
            .find(|label| label.starts_with("kittwm-info-heading:"))
            .unwrap();
        assert!(
            heading.contains("socket=/very/long/path/to/kittwm/socket/that/would/blo…"),
            "{heading}"
        );
        assert!(
            heading.contains("focus=native-window-with-a-pathologic…"),
            "{heading}"
        );
        assert!(
            heading.contains("layout=layout-name-that-is-pathologica…"),
            "{heading}"
        );
        assert!(heading.len() < 170, "{heading}");
        let row = labels
            .iter()
            .find(|label| label.starts_with("kittwm-info-pane:"))
            .unwrap();
        assert!(
            row.contains("kittwm-info-pane:native-window-with-a-pathologic…"),
            "{row}"
        );
        assert!(
            row.contains("title=pane-title-that-is-pathologically-long-and-woul…"),
            "{row}"
        );
        assert!(row.len() < 130, "{row}");
    }

    #[test]
    fn events_backdrop_label_builds_directly() {
        assert_eq!(
            events_backdrop_label(3, 250),
            "kittwm-events-backdrop:count=3:ms=250"
        );
    }

    #[test]
    fn events_row_label_builds_directly() {
        assert_eq!(
            events_row_label(2, "pane_frame_presented"),
            "kittwm-event-row:2:pane_frame_presented"
        );
    }

    #[test]
    fn events_heading_label_builds_directly() {
        assert_eq!(
            events_heading_label("status,pane_opened"),
            "kittwm-events-heading:status,pane_opened"
        );
    }

    #[test]
    fn events_scene_rows_saturate_large_event_counts() {
        assert_eq!(events_scene_rows(0), 5);
        assert_eq!(events_scene_rows(3), 7);
        assert_eq!(events_scene_rows(usize::MAX), 18);

        let mut kinds = Vec::with_capacity(128);
        for idx in 0..128 {
            let mut kind = String::with_capacity("pane_frame_presented_".len() + 3);
            kind.push_str("pane_frame_presented_");
            let _ = write!(kind, "{idx}");
            kinds.push(kind);
        }
        let scene = events_scene_for_cols(250, &kinds, 80);
        assert_eq!(scene.footprint.rows, 18);
    }

    #[test]
    fn events_scene_rows_fit_narrow_width() {
        let kinds = vec!["status".to_string(), "pane_frame_presented".to_string()];
        let scene = events_scene_for_cols(250, &kinds, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
    }

    #[test]
    fn events_scene_labels_bounded_event_kinds() {
        let lines = r#"{"kind":"status"}
{"kind":"pane_opened"}
{"kind":"pane_frame_presented"}
END
"#;
        let kinds = event_kinds_from_lines(lines);
        assert_eq!(kinds, vec!["status", "pane_opened", "pane_frame_presented"]);
        let scene = events_scene(250, &kinds);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains("count=3:ms=250")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("status,pane_opened,pane_frame_presented")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kittwm-event-row:2:pane_frame_presented")),
            "{labels:?}"
        );
    }

    #[test]
    fn events_scene_clips_pathological_event_kind_labels() {
        let kinds = vec![
            "status".to_string(),
            "pane_frame_presented_with_a_pathologically_long_event_kind".to_string(),
            "another_pathologically_long_event_kind_for_heading".to_string(),
        ];
        let summary = events_summary_label(&kinds);
        assert!(summary.capacity() >= 3 * 33);
        let scene = events_scene_for_cols(250, &kinds, 8);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let heading = labels
            .iter()
            .find(|label| label.starts_with("kittwm-events-heading:"))
            .unwrap();
        assert!(
            heading.contains("pane_frame_presented_with_a_pat…"),
            "{heading}"
        );
        assert!(
            heading.contains("another_pathologically_long_eve…"),
            "{heading}"
        );
        assert!(heading.len() < 110, "{heading}");
        let row = labels
            .iter()
            .find(|label| label.starts_with("kittwm-event-row:1:"))
            .unwrap();
        assert!(
            row.contains("pane_frame_presented_with_a_pathologically_long…"),
            "{row}"
        );
        assert!(row.len() < 80, "{row}");
    }

    #[test]
    fn panes_backdrop_label_builds_directly() {
        assert_eq!(
            panes_backdrop_label(2, "native-2", "rows"),
            "kittwm-panes-backdrop:panes=2:focus=native-2:layout=rows"
        );
    }

    #[test]
    fn panes_scene_rows_saturate_large_detail_counts() {
        assert_eq!(panes_scene_rows(0), 5);
        assert_eq!(panes_scene_rows(3), 7);
        assert_eq!(panes_scene_rows(usize::MAX), 18);

        let details = (0..128)
            .map(|idx| {
                serde_json::json!({
                    "window": synthetic_native_window_id(idx),
                    "title": "shell",
                    "focused": false,
                    "app_cols": 80,
                    "app_rows": 24
                })
            })
            .collect::<Vec<_>>();
        let panes = serde_json::json!({
            "panes": details.len(),
            "focus": "native-1",
            "layout": "columns",
            "panes_detail": details
        });
        let scene = panes_scene_for_cols(&panes, 80);
        assert_eq!(scene.footprint.rows, 18);
    }

    #[test]
    fn panes_scene_rows_fit_narrow_width() {
        let panes = serde_json::json!({
            "panes": 1,
            "focus": "native-1",
            "layout": "columns",
            "panes_detail": [
                {"window":"native-1","title":"shell","focused":true,"app_cols":1,"app_rows":1}
            ]
        });
        let scene = panes_scene_for_cols(&panes, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
    }

    #[test]
    fn panes_scene_row_label_builds_directly() {
        let label = panes_scene_row_label("native-2", true, "editor", 80, 12);
        assert_eq!(
            label,
            "kittwm-pane-row:native-2:focused=true:title=editor:app=80x12"
        );
        assert!(label.capacity() >= label.len());
    }

    #[test]
    fn panes_scene_labels_focus_layout_and_app_bounds() {
        let panes = serde_json::json!({
            "panes": 2,
            "focus": "native-2",
            "layout": "rows",
            "panes_detail": [
                {"window":"native-1","title":"shell","focused":false,"app_cols":40,"app_rows":10},
                {"window":"native-2","title":"editor","focused":true,"app_cols":80,"app_rows":12}
            ]
        });
        let scene = panes_scene(&panes);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("panes=2:focus=native-2:layout=rows")),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label
                .contains("kittwm-pane-row:native-2:focused=true:title=editor:app=80x12")),
            "{labels:?}"
        );
    }

    #[test]
    fn panes_scene_clips_pathological_label_fields() {
        let panes = serde_json::json!({
            "panes": 1,
            "focus": "native-window-with-a-pathologically-long-focus-id",
            "layout": "layout-name-that-is-pathologically-long",
            "panes_detail": [
                {
                    "window":"native-window-with-a-pathologically-long-window-id",
                    "title":"pane-title-that-is-pathologically-long-and-would-bloat-scene-labels",
                    "focused":true,
                    "app_cols":80,
                    "app_rows":24
                }
            ]
        });
        let scene = panes_scene_for_cols(&panes, 8);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        let backdrop = labels
            .iter()
            .find(|label| label.starts_with("kittwm-panes-backdrop:"))
            .unwrap();
        assert!(
            backdrop.contains("focus=native-window-with-a-pathologic…"),
            "{backdrop}"
        );
        assert!(
            backdrop.contains("layout=layout-name-that-is-pathologica…"),
            "{backdrop}"
        );
        assert!(backdrop.len() < 120, "{backdrop}");
        let row = labels
            .iter()
            .find(|label| label.starts_with("kittwm-pane-row:"))
            .unwrap();
        assert!(row.contains("native-window-with-a-pathologic…"), "{row}");
        assert!(
            row.contains("title=pane-title-that-is-pathologically-long-and-woul…"),
            "{row}"
        );
        assert!(row.len() < 140, "{row}");
    }

    #[test]
    fn kittwm_help_is_grouped_for_daily_driver_use() {
        let text = kittwm_help_text();
        for heading in [
            "USAGE",
            "DAILY DRIVER BASICS",
            "COMMON INSPECTION",
            "PANE CONTROL",
            "INPUT AND AUTOMATION",
            "EXAMPLES",
        ] {
            assert!(text.contains(heading), "missing {heading}: {text}");
        }
        assert!(text.contains("kittwm --panes"), "{text}");
        assert!(text.contains("--spawn-pty CMD"), "{text}");
        assert!(text.contains("nudge [WINDOW] DX DY"), "{text}");
        assert!(text.contains("reset-position [WINDOW]"), "{text}");
        assert!(text.contains("reset-positions"), "{text}");
        assert!(text.contains("reset-weights"), "{text}");
        assert!(text.contains("--wait-output-json-ms"), "{text}");
        assert!(
            text.contains(
                "shift/alt/ctrl arrows, insert/delete, home/end/page, shift-tab, f5..f12"
            ),
            "{text}"
        );
        assert!(text.contains("kittwm shortcuts"), "{text}");
        assert!(text.contains("kittwm showcase-scene-json"), "{text}");
        assert!(text.contains("kittwm showcase-metrics-json"), "{text}");
        assert!(text.contains("kittwm showcase-composition-json"), "{text}");
        assert!(text.contains("kittwm tui-smoke-json"), "{text}");
    }

    fn help_command_tree_needle(command: &str) -> String {
        let mut needle = String::with_capacity("kittwm ".len() + command.len());
        needle.push_str("kittwm ");
        needle.push_str(command);
        needle
    }

    #[test]
    fn help_command_tree_needle_builds_directly() {
        let needle = help_command_tree_needle("spawn htop");
        assert_eq!(needle, "kittwm spawn htop");
        assert_eq!(needle.capacity(), needle.len());
    }

    #[test]
    fn kittwm_help_command_tree_is_derived_from_catalog() {
        let tree = kittwm_help_command_tree_text();
        assert!(
            tree.contains("COMMAND TREE (derived from kittwm parser catalog)"),
            "{tree}"
        );
        for entry in local_command_entries() {
            let needle = help_command_tree_needle(entry.command);
            assert!(tree.contains(&needle), "missing {needle:?}: {tree}");
            assert!(
                tree.contains(entry.description),
                "missing description {:?}: {tree}",
                entry.description
            );
        }
    }

    #[test]
    fn kittwm_help_mentions_first_party_helper_examples() {
        let text = kittwm_help_text();
        assert!(text.contains("FIRST-PARTY HELPERS"), "{text}");
        assert!(
            text.contains("kittwm-launch --browser https://example.com"),
            "{text}"
        );
        assert!(text.contains("kittwm-terminal --events-ms 1000"), "{text}");
        assert!(text.contains("kittwm-top --json"), "{text}");
        assert!(text.contains("kittwm-bar --reserve --kitty"), "{text}");
        assert!(
            text.contains("kittwm-browser --semantic-snapshot https://example.com"),
            "{text}"
        );
    }

    #[test]
    fn shortcuts_command_uses_native_shortcut_list() {
        let text = kittui_cli::shortcuts::render_native_shortcuts();
        assert!(
            text.contains("C-a Enter          launch terminal"),
            "{text}"
        );
        assert!(
            text.contains("C-a t              toggle floating mode"),
            "{text}"
        );
        assert!(
            text.contains("C-a f              toggle fullscreen"),
            "{text}"
        );
        assert!(
            text.contains("C-a e              toggle current split"),
            "{text}"
        );
        assert!(
            !text.contains("C-a Enter / C-a t  launch terminal"),
            "{text}"
        );
        assert!(text.contains("toggle this help"), "{text}");
        assert!(text.contains("Ctrl-]"), "{text}");
    }

    #[test]
    fn shortcuts_json_command_uses_native_shortcut_catalog() {
        let value: serde_json::Value =
            serde_json::from_str(&kittui_cli::shortcuts::render_native_shortcuts_json()).unwrap();
        assert_eq!(value["kind"], "kittwm-native-shortcuts");
        let shortcuts = value["shortcuts"].as_array().unwrap();
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "launch_terminal" && entry["keys"] == "C-a Enter"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "toggle_floating" && entry["keys"] == "C-a t"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "toggle_fullscreen" && entry["keys"] == "C-a f"));
        assert!(shortcuts
            .iter()
            .any(|entry| entry["id"] == "toggle_split" && entry["keys"] == "C-a e"));
    }

    #[test]
    fn help_topic_start_mentions_copyable_examples() {
        let text = help_topic_text("start").unwrap();
        assert!(text.contains("KITTWM_WORKSPACE=dev kittwm"), "{text}");
        assert!(
            text.contains("KITTWM_NATIVE_RENDERER=kitty kittwm"),
            "{text}"
        );
        assert!(
            text.contains("KITTWM_NATIVE_CHROME_RENDERER=affordance-scene kittwm"),
            "{text}"
        );
    }

    #[test]
    fn help_topic_panes_is_focused() {
        let text = help_topic_text("panes").unwrap();
        assert!(text.contains("--spawn-pty CMD"), "{text}");
        assert!(
            text.contains("split [WINDOW] columns|rows|grid CMD [ARGS...]"),
            "{text}"
        );
        assert!(text.contains("SPLIT_PANE"), "{text}");
        assert!(text.contains("--balance-panes"), "{text}");
        assert!(text.contains("--reset-pane-weights"), "{text}");
        assert!(!text.contains("--probe-kitty"), "{text}");
    }

    #[test]
    fn help_topic_panes_mentions_copyable_examples() {
        let text = help_topic_text("panes").unwrap();
        assert!(text.contains("kittwm spawn htop"), "{text}");
        assert!(text.contains("kittwm split focused columns htop"), "{text}");
        assert!(text.contains("kittwm focus next"), "{text}");
        assert!(text.contains("kittwm balance"), "{text}");
        assert!(text.contains("reset-weights"), "{text}");
    }

    #[test]
    fn help_topic_input_is_focused() {
        let text = help_topic_text("input").unwrap();
        assert!(text.contains("--send-text WINDOW TEXT"), "{text}");
        assert!(
            text.contains(
                "shift/alt/ctrl arrows, insert/delete, home/end/page, shift-tab, f5..f12"
            ),
            "{text}"
        );
        assert!(text.contains("--semantic-action"), "{text}");
        assert!(!text.contains("--save-session"), "{text}");
    }

    #[test]
    fn help_topic_input_mentions_copyable_examples() {
        let text = help_topic_text("input").unwrap();
        assert!(text.contains("kittwm line focused 'echo ready'"), "{text}");
        assert!(
            text.contains("kittwm paste focused 'multi-line text'"),
            "{text}"
        );
        assert!(text.contains("kittwm key focused ctrl-c"), "{text}");
        assert!(
            text.contains("kittwm --semantic-action focused button-1 press '{}'"),
            "{text}"
        );
    }

    #[test]
    fn help_topic_inspect_mentions_first_party_helper_examples() {
        let text = help_topic_text("inspect").unwrap();
        assert!(text.contains("kittwm-top --json"), "{text}");
        assert!(
            text.contains("kittwm-browser --semantic-snapshot URL"),
            "{text}"
        );
        assert!(text.contains("--semantic-snapshot WINDOW"), "{text}");
    }

    #[test]
    fn help_topic_events_mentions_terminal_helper_examples() {
        let text = help_topic_text("events").unwrap();
        assert!(text.contains("kittwm-terminal --events-ms 1000"), "{text}");
        assert!(
            text.contains("kittwm-terminal --events-scene-json 1000"),
            "{text}"
        );
        assert!(text.contains("--events-ms MS"), "{text}");
    }

    #[test]
    fn help_topic_session_mentions_save_restore_examples() {
        let text = help_topic_text("session").unwrap();
        assert!(
            text.contains("kittwm --save-session session.json"),
            "{text}"
        );
        assert!(
            text.contains("kittwm --restore-session session.json"),
            "{text}"
        );
        assert!(text.contains("kittwm --save-session -"), "{text}");
        assert!(text.contains("--session-json"), "{text}");
    }

    #[test]
    fn help_topic_apps_mentions_bar_chrome_contract() {
        let text = help_topic_text("apps").unwrap();
        assert!(text.contains("kittwm-launch"), "{text}");
        assert!(text.contains("kittwm-top"), "{text}");
        assert!(text.contains("kittwm-bar --kitty --reserve"), "{text}");
        assert!(text.contains("kittwm-bar --release"), "{text}");
    }

    #[test]
    fn help_topic_apps_mentions_first_party_helper_examples() {
        let text = help_topic_text("apps").unwrap();
        assert!(text.contains("kittwm-launch --browser URL"), "{text}");
        assert!(
            text.contains("kittwm-terminal --title logs -- tail -f /tmp/app.log"),
            "{text}"
        );
        assert!(text.contains("kittwm-top"), "{text}");
    }

    #[test]
    fn help_topic_rejects_unknown_topic() {
        let err = help_topic_text("bogus").unwrap_err();
        assert!(
            err.to_string().contains("unknown kittwm help topic"),
            "{err}"
        );
    }

    #[test]
    fn action_aliases_map_to_socket_commands() {
        assert_eq!(
            spawn_alias_request(&args(&["htop", "--tree"])).unwrap(),
            "SPAWN_PTY htop --tree"
        );
        assert_eq!(
            split_alias_request(&args(&["columns", "htop", "--tree"])).unwrap(),
            "SPLIT_PANE focused columns htop --tree"
        );
        assert_eq!(
            split_alias_request(&args(&["native-1", "rows", "echo", "hi there"])).unwrap(),
            "SPLIT_PANE native-1 rows echo 'hi there'"
        );
        assert_eq!(
            split_alias_request(&args(&["grid", "kittwm-top"])).unwrap(),
            "SPLIT_PANE focused grid kittwm-top"
        );
        assert_eq!(read_alias_request(false, &[]).unwrap(), "READ_TEXT focused");
        assert_eq!(
            read_alias_request(true, &args(&["native-2"])).unwrap(),
            "READ_TEXT_JSON native-2"
        );
        assert_eq!(
            default_window_payload_alias("SEND_TEXT", "type", &args(&["hello"])).unwrap(),
            "SEND_TEXT focused hello"
        );
        assert_eq!(
            default_window_payload_alias("SEND_LINE", "line", &args(&["native-2", "make test"]))
                .unwrap(),
            "SEND_LINE native-2 make test"
        );
        assert_eq!(
            default_window_payload_alias(
                "PASTE_BYTES_B64",
                "paste",
                &args(&["native-2", "paste me"])
            )
            .unwrap(),
            "PASTE_BYTES_B64 native-2 cGFzdGUgbWU="
        );
        assert_eq!(
            default_window_payload_alias("SEND_KEY", "key", &args(&[" ctrl-c "])).unwrap(),
            "SEND_KEY focused ctrl-c"
        );
        assert!(default_window_payload_alias("SEND_KEY", "key", &args(&["page down"])).is_err());
        assert_eq!(
            default_window_payload_alias("WAIT_OUTPUT", "wait", &args(&["native-2", " Ready "]))
                .unwrap(),
            "WAIT_OUTPUT native-2 Ready"
        );
        assert!(default_window_payload_alias("WAIT_OUTPUT", "wait", &args(&["   "])).is_err());
        assert!(spawn_alias_request(&[]).is_err());
        assert!(read_alias_request(false, &args(&["a", "b"])).is_err());
    }

    #[test]
    fn inspection_aliases_map_to_socket_commands() {
        assert_eq!(
            parse_inspection_alias("panes", None, None)
                .unwrap()
                .as_deref(),
            Some("PANES")
        );
        assert_eq!(
            parse_inspection_alias("panes-json", None, None)
                .unwrap()
                .as_deref(),
            Some("PANES_JSON")
        );
        assert_eq!(
            parse_inspection_alias("events", Some("2500".to_string()), None)
                .unwrap()
                .as_deref(),
            Some("EVENTS 2500")
        );
        assert!(parse_inspection_alias("status", None, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn inspection_aliases_reject_extra_args() {
        let err =
            parse_inspection_alias("events", Some("10".to_string()), Some("extra".to_string()))
                .unwrap_err();
        assert!(err.to_string().contains("at most one"), "{err}");
        let err = parse_inspection_alias("panes", Some("extra".to_string()), None).unwrap_err();
        assert!(err.to_string().contains("does not accept"), "{err}");
    }

    #[test]
    fn pane_control_aliases_map_to_socket_commands() {
        assert_eq!(
            parse_pane_control_alias("focus", args(&["native-2"]).into_iter()).unwrap(),
            "FOCUS_PANE native-2"
        );
        assert_eq!(
            parse_pane_control_alias("close", Vec::<String>::new().into_iter()).unwrap(),
            "CLOSE_PANE focused"
        );
        assert_eq!(
            parse_pane_control_alias("layout", args(&["rows"]).into_iter()).unwrap(),
            "LAYOUT rows"
        );
        assert_eq!(
            parse_pane_control_alias("move", args(&["last"]).into_iter()).unwrap(),
            "MOVE_PANE focused last"
        );
        assert_eq!(
            parse_pane_control_alias("raise", Vec::<String>::new().into_iter()).unwrap(),
            "MOVE_PANE focused last"
        );
        assert_eq!(
            parse_pane_control_alias("lower", args(&["native-2"]).into_iter()).unwrap(),
            "MOVE_PANE native-2 first"
        );
        assert_eq!(
            parse_pane_control_alias("nudge", args(&["3", "-2"]).into_iter()).unwrap(),
            "NUDGE_PANE focused 3 -2"
        );
        assert_eq!(
            parse_pane_control_alias("nudge", args(&["native-2", "3", "-2"]).into_iter()).unwrap(),
            "NUDGE_PANE native-2 3 -2"
        );
        assert_eq!(
            parse_pane_control_alias("reset-position", Vec::<String>::new().into_iter()).unwrap(),
            "RESET_PANE_OFFSET focused"
        );
        assert_eq!(
            parse_pane_control_alias("reset-offset", args(&["native-2"]).into_iter()).unwrap(),
            "RESET_PANE_OFFSET native-2"
        );
        assert_eq!(
            parse_pane_control_alias("reset-positions", Vec::<String>::new().into_iter()).unwrap(),
            "RESET_ALL_PANE_OFFSETS"
        );
        assert_eq!(
            parse_pane_control_alias("resize", args(&["native-2", "+2"]).into_iter()).unwrap(),
            "RESIZE_PANE native-2 +2"
        );
        assert_eq!(
            parse_pane_control_alias("balance", Vec::<String>::new().into_iter()).unwrap(),
            "BALANCE_PANES"
        );
        assert_eq!(
            parse_pane_control_alias("reset-weights", Vec::<String>::new().into_iter()).unwrap(),
            "BALANCE_PANES"
        );
        assert_eq!(
            parse_pane_control_alias("reset-weight", Vec::<String>::new().into_iter()).unwrap(),
            "BALANCE_PANES"
        );
        assert_eq!(
            parse_pane_control_alias("rename", args(&["native-2", "Editor"]).into_iter()).unwrap(),
            "RENAME_PANE native-2 Editor"
        );
    }

    #[test]
    fn pane_control_aliases_reject_bad_inputs() {
        assert!(parse_pane_control_alias("focus", Vec::<String>::new().into_iter()).is_err());
        assert!(parse_pane_control_alias("balance", args(&["extra"]).into_iter()).is_err());
        assert!(parse_pane_control_alias("reset-weights", args(&["extra"]).into_iter()).is_err());
        assert!(parse_pane_control_alias("layout", args(&["diagonal"]).into_iter()).is_err());
        assert!(parse_pane_control_alias("nudge", args(&["0", "0"]).into_iter()).is_err());
        assert!(
            parse_pane_control_alias("nudge", args(&["native-1", "x", "1"]).into_iter()).is_err()
        );
        assert_eq!(
            parse_pane_control_alias("layout", args(&["grid"]).into_iter()).unwrap(),
            "LAYOUT grid"
        );
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
    fn replace_spawn_request_builds_directly() {
        assert_eq!(
            replace_spawn_request(&args(&["printf", "Bob's pane"])),
            "SPAWN printf 'Bob'\\''s pane'"
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
    fn socket_target_flags_are_mutually_exclusive() {
        let mut cli = Cli::default();
        cli.socket = Some("/tmp/kittwm-test.sock".to_string());
        assert!(validate_socket_target_flags(&cli).is_ok());
        cli.display = Some(":7".to_string());
        let err = validate_socket_target_flags(&cli).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn normalize_daemon_command_uppercases_only_verb() {
        let status = normalize_daemon_command("status");
        assert_eq!(status, "STATUS");
        assert_eq!(status.capacity(), status.len());
        let spawn = normalize_daemon_command("spawn printf MixedCase");
        assert_eq!(spawn, "SPAWN printf MixedCase");
        assert_eq!(spawn.capacity(), spawn.len());
        assert_eq!(
            normalize_daemon_command("apps_first Safari"),
            "APPS_FIRST Safari"
        );
    }

    #[test]
    fn pane_control_requests_validate_and_preserve_payloads() {
        let spawn_request = protocol_payload_request("spawn_pty", "  htop  ").unwrap();
        assert_eq!(spawn_request, "SPAWN_PTY htop");
        assert_eq!(spawn_request.capacity(), spawn_request.len());
        let focus_request = protocol_token_request("focus_pane", "native-2").unwrap();
        assert_eq!(focus_request, "FOCUS_PANE native-2");
        assert_eq!(focus_request.capacity(), focus_request.len());
        let layout = layout_request("ROWS").unwrap();
        assert_eq!(layout, "LAYOUT rows");
        assert_eq!(layout.capacity(), layout.len());
        let split_request = split_pane_request("focused", "ROWS", " htop --tree ").unwrap();
        assert_eq!(split_request, "SPLIT_PANE focused rows htop --tree");
        assert_eq!(split_request.capacity(), split_request.len());
        let move_request = move_pane_request("focused", "LAST").unwrap();
        assert_eq!(move_request, "MOVE_PANE focused last");
        assert_eq!(move_request.capacity(), move_request.len());
        let nudge_request = nudge_pane_request("focused", "3", "-2").unwrap();
        assert_eq!(nudge_request, "NUDGE_PANE focused 3 -2");
        assert!(nudge_request.capacity() >= nudge_request.len());
        assert_eq!(
            reset_pane_offset_request("focused").unwrap(),
            "RESET_PANE_OFFSET focused"
        );
        assert_eq!(
            nudge_parse_context("dx", "bad"),
            "nudge dx must be an i16: \"bad\""
        );
        assert_eq!(
            nudge_parse_context("dy", " 100000 "),
            "nudge dy must be an i16: \" 100000 \""
        );
        let resize_request = resize_pane_request("focused", "+2").unwrap();
        assert_eq!(resize_request, "RESIZE_PANE focused +2");
        assert_eq!(resize_request.capacity(), resize_request.len());
        let rename_request = rename_pane_request("native-2", " Editor Pane ").unwrap();
        assert_eq!(rename_request, "RENAME_PANE native-2 Editor Pane");
        assert_eq!(rename_request.capacity(), rename_request.len());
        assert!(rename_pane_request("native-2", "   ").is_err());
        assert_eq!(
            protocol_payload_request("apps_first", "Safari Browser").unwrap(),
            "APPS_FIRST Safari Browser"
        );
        assert_eq!(
            protocol_payload_request("apps_launch_first", "Visual Studio Code").unwrap(),
            "APPS_LAUNCH_FIRST Visual Studio Code"
        );
        assert!(layout_request("diagonal").is_err());
        assert!(move_pane_request("bad window", "last").is_err());
    }

    #[test]
    fn normalize_daemon_command_preserves_json_inspection_verbs() {
        assert_eq!(normalize_daemon_command("status_json"), "STATUS_JSON");
        assert_eq!(normalize_daemon_command("help_json"), "HELP_JSON");
        assert_eq!(normalize_daemon_command("chrome_json"), "CHROME_JSON");
        assert_eq!(normalize_daemon_command("panes_json"), "PANES_JSON");
        assert_eq!(normalize_daemon_command("session_json"), "SESSION_JSON");
        assert_eq!(normalize_daemon_command("clipboard_json"), "CLIPBOARD_JSON");
    }

    #[test]
    fn automation_request_preserves_payload_case_and_spaces() {
        assert_eq!(
            text_payload_request("send_line", "focused", "echo Mixed Case", "line").unwrap(),
            "SEND_LINE focused echo Mixed Case"
        );
        assert_eq!(
            paste_text_request("focused", "paste me", "paste").unwrap(),
            "PASTE_BYTES_B64 focused cGFzdGUgbWU="
        );
        assert_eq!(
            paste_text_request("focused", "   ", "paste").unwrap(),
            "PASTE_BYTES_B64 focused ICAg"
        );
        assert!(paste_text_request("focused", "", "paste").is_err());
        assert_eq!(
            text_payload_request("send_text", "focused", "   ", "type").unwrap(),
            "SEND_TEXT focused    "
        );
        assert!(text_payload_request("send_text", "focused", "", "type").is_err());
        let read_request = automation_request("read_text", "native-2", "").unwrap();
        assert_eq!(read_request, "READ_TEXT native-2");
        assert_eq!(read_request.capacity(), read_request.len());
        assert_eq!(
            automation_request("READ_TEXT_JSON", "focused", "").unwrap(),
            "READ_TEXT_JSON focused"
        );
        assert_eq!(
            automation_request("READ_SCROLLBACK_JSON", "native-2", "").unwrap(),
            "READ_SCROLLBACK_JSON native-2"
        );
        let wait_text_ms = wait_ms_request("WAIT_TEXT_MS", "2500", "focused", "Ready Now").unwrap();
        assert_eq!(wait_text_ms, "WAIT_TEXT_MS focused 2500 Ready Now");
        assert!(wait_text_ms.capacity() >= wait_text_ms.len());
        assert_eq!(
            automation_request("WAIT_TEXT_JSON", "focused", "Ready Now").unwrap(),
            "WAIT_TEXT_JSON focused Ready Now"
        );
        assert_eq!(
            wait_ms_request("WAIT_TEXT_JSON_MS", "2500", "focused", "Ready Now").unwrap(),
            "WAIT_TEXT_JSON_MS focused 2500 Ready Now"
        );
        assert_eq!(
            send_bytes_b64_request("focused", " aGkKAA== ").unwrap(),
            "SEND_BYTES_B64 focused aGkKAA=="
        );
        assert_eq!(
            paste_bytes_b64_request("focused", " aGkKAA== ").unwrap(),
            "PASTE_BYTES_B64 focused aGkKAA=="
        );
        assert!(send_bytes_b64_request("focused", "").is_err());
        assert!(send_bytes_b64_request("focused", "!!!").is_err());
        assert!(paste_bytes_b64_request("focused", "").is_err());
        assert!(paste_bytes_b64_request("focused", "!!!").is_err());
        let mouse_request = send_mouse_request("focused", "press-left", "7", "9").unwrap();
        assert_eq!(mouse_request, "SEND_MOUSE focused press-left 7 9");
        assert!(mouse_request.capacity() >= mouse_request.len());
        assert_eq!(
            send_mouse_request("focused", "move-left", "7", "9").unwrap(),
            "SEND_MOUSE focused move-left 7 9"
        );
        assert_eq!(
            send_mouse_request("focused", "release-right", "7", "9").unwrap(),
            "SEND_MOUSE focused release-right 7 9"
        );
        assert_eq!(
            send_bytes_request("focused", b"hi\n\0").unwrap(),
            "SEND_BYTES_B64 focused aGkKAA=="
        );
        assert_eq!(
            paste_bytes_request("focused", b"hi\n\0").unwrap(),
            "PASTE_BYTES_B64 focused aGkKAA=="
        );
        assert_eq!(
            send_bytes_request("focused", b"\0\xff\x1b[31m").unwrap(),
            "SEND_BYTES_B64 focused AP8bWzMxbQ=="
        );
        assert_eq!(
            paste_bytes_request("focused", b"\0\xff\x1b[31m").unwrap(),
            "PASTE_BYTES_B64 focused AP8bWzMxbQ=="
        );
        assert_eq!(
            wait_ms_request("WAIT_OUTPUT_MS", "2500", "focused", " Ready Now ").unwrap(),
            "WAIT_OUTPUT_MS focused 2500 Ready Now"
        );
        assert_eq!(
            wait_request("WAIT_OUTPUT_JSON", "focused", " Ready Now ").unwrap(),
            "WAIT_OUTPUT_JSON focused Ready Now"
        );
        assert_eq!(
            wait_ms_request("WAIT_OUTPUT_JSON_MS", "2500", "focused", " Ready Now ").unwrap(),
            "WAIT_OUTPUT_JSON_MS focused 2500 Ready Now"
        );
        assert!(wait_request("WAIT_TEXT", "focused", "   ").is_err());
        assert_eq!(
            semantic_snapshot_request("focused").unwrap(),
            "SEMANTIC_SNAPSHOT focused"
        );
        assert_eq!(
            semantic_focus_request("focused", "native-1.screen").unwrap(),
            "SEMANTIC_FOCUS focused native-1.screen"
        );
        assert_eq!(
            semantic_publish_request(
                "focused",
                r#"{"schema_version":1,"surface":"native-1","revision":1,"root":{"id":"native-1.root","role":"group"}}"#
            )
            .unwrap(),
            r#"SEMANTIC_PUBLISH focused {"revision":1,"root":{"id":"native-1.root","role":"group"},"schema_version":1,"surface":"native-1"}"#
        );
        let semantic_action = semantic_action_request(
            "focused",
            "native-1.screen",
            "insert_text",
            r#"{"text":"hi"}"#,
        )
        .unwrap();
        assert_eq!(
            semantic_action,
            r#"SEMANTIC_ACTION focused native-1.screen insert_text {"text":"hi"}"#
        );
        assert_eq!(semantic_action.capacity(), semantic_action.len());
        assert!(semantic_action_request("focused", "bad component", "set", "{}").is_err());
        assert!(semantic_action_request("focused", "field", "set", "not-json").is_err());
        assert!(semantic_publish_request("focused", "not-json").is_err());
        let events = events_request("2500").unwrap();
        assert_eq!(events, "EVENTS 2500");
        assert_eq!(events.capacity(), events.len());
        let snapshot_events = events_request_millis(750);
        assert_eq!(snapshot_events, "EVENTS 750");
        assert_eq!(snapshot_events.capacity(), snapshot_events.len());
        assert!(events_request("0").is_err());
        assert!(events_request("60001").is_err());
        assert!(wait_ms_request("WAIT_TEXT_MS", "0", "focused", "ready").is_err());
        assert!(send_mouse_request("focused", "drag", "7", "9").is_err());
        assert!(send_key_request("focused", "page down").is_err());
        assert!(automation_request("SEND_KEY", "bad window", "ctrl-c").is_err());
    }

    #[test]
    fn save_session_json_file_text_appends_newline_directly() {
        let text = save_session_json_file_text("{\n  \"layout\": \"rows\"\n}");
        assert_eq!(text, "{\n  \"layout\": \"rows\"\n}\n");
        assert_eq!(text.capacity(), text.len());
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
        assert_eq!(request.capacity(), request.len());
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
