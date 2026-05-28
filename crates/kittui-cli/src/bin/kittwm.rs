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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
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
                    return Err(anyhow!(
                        "kittwm help accepts at most one topic, got {extra:?}"
                    ));
                }
                break;
            }
            "help-scene-json" => {
                out.help_scene_topic = Some(args.next().unwrap_or_else(|| "topics".to_string()));
                if let Some(extra) = args.next() {
                    return Err(anyhow!(
                        "kittwm help-scene-json accepts at most one topic, got {extra:?}"
                    ));
                }
                break;
            }
            "help-kitty" | "help-graphics" => {
                out.help_kitty_topic = Some(args.next().unwrap_or_else(|| "topics".to_string()));
                if let Some(extra) = args.next() {
                    return Err(anyhow!(
                        "kittwm help-kitty accepts at most one topic, got {extra:?}"
                    ));
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
                out.completions = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("kittwm completions SHELL"))?,
                );
                if let Some(extra) = args.next() {
                    return Err(anyhow!(
                        "kittwm completions accepts one shell, got {extra:?}"
                    ));
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
            "focus" | "close" | "layout" | "move" | "resize" | "balance" | "rename" => {
                out.automation_request = Some(parse_pane_control_alias(a.as_str(), args.by_ref())?);
                break;
            }
            "apps" => out.apps = true,
            "apps-scene-json" => out.apps_scene_json = true,
            "apps-kitty" | "apps-graphics" => out.apps_kitty = true,
            "native-terminal" => out.native_terminal = true,
            "native-browser" => out.native_browser = true,
            "--socket" => {
                out.socket = Some(args.next().ok_or_else(|| anyhow!("--socket PATH"))?);
            }
            "--display" => {
                out.display = Some(args.next().ok_or_else(|| anyhow!("--display DISPLAY"))?);
            }
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

fn kittwm_help_text() -> &'static str {
    r#"kittwm — terminal-native window manager

USAGE
  kittwm                         Start the WM in this terminal (empty workspace + top bar)
  kittwm start                   Explicit start alias for the same default session
  kittwm stop                    Stop a socket daemon (alias for --kill)
  kittwm --socket PATH COMMAND   Target a running WM socket for one command
  kittwm --display :N COMMAND    Target a DISPLAY-like kittwm socket token
  kittwm --help                  Show this overview
  kittwm help <topic>            Show focused help (when available)
  kittwm help-kitty [topic]      Render focused help with kitty graphics
  kittwm info                    Show friendly running-WM overview
  kittwm status-kitty            Render daemon status with kitty graphics
  kittwm quickstart              Show first-run daily-driver checklist
  kittwm quickstart-kitty        Render quickstart with kitty graphics
  kittwm examples                Show copy-paste daily-driver workflows
  kittwm examples-kitty          Render examples with kitty graphics
  kittwm log path                Print debug log path
  kittwm log tail -f             Follow debug log from another terminal
  kittwm commands                Show grouped local CLI command catalog
  kittwm commands-json           Show local CLI command catalog JSON
  kittwm commands-scene-json     Emit local command catalog as a kittui Scene
  kittwm commands-kitty          Render local command catalog with kitty graphics
  kittwm architecture-json       Emit WM architecture/separation contract JSON
  kittwm architecture-scene-json Emit architecture contract as a kittui Scene
  kittwm architecture-kitty      Render architecture contract with kitty graphics
  kittwm native-surfaces         Show first-party native surface coverage
  kittwm native-surfaces-json    Emit first-party native surface coverage JSON
  kittwm native-surfaces-scene-json Emit coverage as a kittui Scene
  kittwm native-surfaces-kitty   Render coverage with kitty graphics
  kittwm showcase-scene-json     Emit a representative graphical WM scene artifact
  kittwm showcase-metrics-json   Emit scene/layer/pixel metrics for that artifact
  kittwm showcase-composition-json Emit ordered app/chrome/overlay composition graph
  kittwm tui-smoke-json          Emit terminal/TUI conformance smoke matrix
  kittwm update [--status|--check] Self-update from GitHub release assets
  kittwm mcp                     Expose shared update tools over MCP stdio
  kittwm completions SHELL       Print shell completions (bash|zsh|fish)
  kittwm cheat                   Show compact daily-driver cheat sheet
  kittwm cheat-kitty             Render cheat sheet with kitty graphics

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
  focus WINDOW                Alias for --focus-pane WINDOW
  close [WINDOW]              Alias for --close-pane (default focused)
  layout columns|rows         Alias for --layout
  move [WINDOW] DIR           Alias for --move-pane (default focused)
  resize [WINDOW] AMOUNT      Alias for --resize-pane (default focused)
  balance                     Alias for --balance-panes
  rename WINDOW TITLE         Alias for --rename-pane
  --spawn-pty CMD             Spawn a terminal pane
  --focus-pane WINDOW         Focus pane by id, or use focused
  --focus-next | --focus-prev Cycle focus
  --close-pane WINDOW         Close pane; last pane returns to empty workspace
  --layout columns|rows       Change tiling axis
  --move-pane WINDOW DIR      DIR: left/right/up/down/first/last
  --resize-pane WINDOW N      N: grow/shrink/+N/-N
  --balance-panes             Equalize pane weights
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
  --send-key WINDOW KEY            KEY: ctrl-c, escape, enter, arrows, ...
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

For complete socket verbs: kittwm --help-json
For interactive key chords: kittwm shortcuts
"#
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
            label: Some(format!(
                "kittwm-help-topic-backdrop:{topic_label}:lines={}:commands={command_count}",
                content_lines.len()
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
            label: Some(format!(
                "kittwm-help-topic-heading:{topic_label}:{heading_label}"
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
    ];
    for (idx, line) in content_lines.iter().skip(1).take(20).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let trimmed = line.trim();
        let row_label = truncate(trimmed, 80);
        layers.push(Layer {
            label: Some(format!(
                "kittwm-help-topic-row:{topic_label}:{idx}:{row_label}"
            )),
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
             apps     app discovery and launch helpers\n\n\
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
             Ctrl-]                         exit kittwm\n"),
        "panes" | "pane" => Ok("kittwm help panes
\
             =================

\
             --spawn-pty CMD                spawn a native PTY pane
\
             focus WINDOW                   focus window or focused token
\
             close [WINDOW]                 close pane (default focused)
\
             layout columns|rows            switch layout axis
\
             move [WINDOW] DIR              move pane (default focused)
\
             resize [WINDOW] AMOUNT         resize pane weight (default focused)
\
             balance                        equalize weights
\
             rename WINDOW TITLE            set display title
\
             --focus-pane WINDOW            focus window or focused token
\
             --focus-next / --focus-prev    cycle focus
\
             --close-pane WINDOW            close pane; last pane returns empty
\
             --layout columns|rows          switch layout axis
\
             --move-pane WINDOW DIR         left/right/up/down/first/last
\
             --resize-pane WINDOW AMOUNT    grow/shrink/+N/-N pane weight
\
             --balance-panes                equalize weights
\
             --rename-pane WINDOW TITLE     set display title

\
             Socket equivalents include SPAWN_PTY, FOCUS_PANE, CLOSE_PANE,
\
             LAYOUT, MOVE_PANE, RESIZE_PANE, BALANCE_PANES, and RENAME_PANE.
"),
        "logs" | "log" => Ok("kittwm help log\n\
             ================\n\n\
             kittwm writes debug logs to KITTUI_WM_LOG when set, otherwise /tmp/kittui-wm.log.\n\
             Open kittwm in one terminal, then watch it from another:\n\n\
             kittwm log path              print the active log path\n\
             kittwm log tail              print recent log lines\n\
             kittwm log tail -f           follow the log, like tail -f\n"),
        "input" => Ok("kittwm help input\n\
             =================\n\n\
             type [WINDOW] TEXT             short alias for --send-text\n\
             line [WINDOW] TEXT             short alias for --send-line\n\
             paste [WINDOW] TEXT            paste text with bracketed-paste support\n\
             key [WINDOW] KEY               short alias for --send-key\n\
             --send-text WINDOW TEXT        send text bytes\n\
             --send-line WINDOW TEXT        send text plus newline\n\
             --send-key WINDOW KEY          send named key (ctrl-c, escape, arrows)\n\
             --send-mouse WINDOW EVENT C R  send terminal mouse event if app enabled it\n\
             --send-bytes-b64 WINDOW B64    send exact bytes\n\
             --paste-bytes-b64 WINDOW B64   paste exact bytes\n\
             --send-file WINDOW PATH|-      send bytes from file/stdin\n\
             --paste-file WINDOW PATH|-     paste bytes with bracketed-paste support\n\
             --semantic-action WINDOW COMPONENT ACTION JSON\n\
                                            invoke semantic action\n\
             --semantic-focus WINDOW COMPONENT request semantic focus\n"),
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
             --apps-json                    app discovery catalog\n"),
        "session" | "sessions" => Ok("kittwm help session\n\
             ===================\n\n\
             --save-session PATH|-          write SESSION_JSON manifest\n\
             --restore-session PATH|-       queue RESTORE_SESSION_JSON\n\
             --session-json                 print current SESSION_JSON\n\n\
             Session manifests store layout axis, focus, pane order, titles,\n\
             commands, and weights. Restore replaces the native pane set.\n"),
        "events" | "event" => Ok("kittwm help events\n\
             ==================\n\n\
             --events                       stream bounded EVENTS output\n\
             --events-ms MS                 stream EVENTS for explicit timeout\n\n\
             EVENTS starts with status, then pane/focus/layout/input/frame,\n\
             semantic, and surface side-effect event envelopes, ending with END.\n"),
        "apps" | "app" => Ok("kittwm help apps\n\
             ================\n\n\
             apps                           list launch candidates\n\
             --apps-json                    APPS_JSON catalog\n\
             --apps-first QUERY             first matching app candidate\n\
             --apps-launch-first QUERY      launch first matching candidate\n\
             launcher [--filter Q] [--limit N]\n\
                                            boxed launcher preview\n\
             kittwm-launch                  first-party SDK launcher helper\n\
             kittwm-bar --kitty --reserve   kitty-native top bar chrome app; reserves drawable row\n\
             kittwm-bar --release           clear the bar chrome reservation\n"),
        other => Err(friendly_unknown_help_topic_error(other)),
    }
}

fn known_help_topics() -> &'static [&'static str] {
    &[
        "topics", "start", "panes", "input", "inspect", "session", "events", "apps",
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
    let mut msg = format!("unknown kittwm command or flag {command:?}.");
    if let Some(suggestion) = suggestion {
        msg.push_str(&format!("\n\nDid you mean?\n  kittwm {suggestion}"));
    }
    msg.push_str("\n\nStart here:\n  kittwm quickstart\n  kittwm --help\n  kittwm help topics\n");
    anyhow!(msg)
}

fn friendly_unknown_help_topic_error(topic: &str) -> anyhow::Error {
    let suggestion = closest_command(topic, known_help_topics());
    let mut msg = format!("unknown kittwm help topic {topic:?}.");
    if let Some(suggestion) = suggestion {
        msg.push_str(&format!("\n\nDid you mean?\n  kittwm help {suggestion}"));
    }
    msg.push_str("\n\nAvailable topics:\n  kittwm help topics\n  kittwm quickstart\n");
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
        _ => Err(anyhow!("usage: kittwm log path | kittwm log tail [-f]")),
    }
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
            let axis = next().ok_or_else(|| anyhow!("kittwm layout columns|rows"))?;
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
        "resize" => {
            let first = next().ok_or_else(|| anyhow!("kittwm resize [WINDOW] AMOUNT"))?;
            let second = next();
            let (window, amount) = match second {
                Some(amount) => (first, amount),
                None => ("focused".to_string(), first),
            };
            resize_pane_request(&window, &amount)?
        }
        "balance" => "BALANCE_PANES".to_string(),
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
            eprintln!("kittwm: {e}");
            ExitCode::from(1)
        }
        Err(payload) if panic_payload_is_broken_pipe(payload.as_ref()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(101),
    }
}

fn panic_payload_is_broken_pipe(payload: &(dyn std::any::Any + Send)) -> bool {
    let Some(message) = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
    else {
        return false;
    };
    message.contains("Broken pipe") || message.contains("os error 32")
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
    if cli.doctor || cli.doctor_scene_json || cli.doctor_kitty {
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

fn doctor_cmd(json: bool, scene_json: bool, kitty: bool, probe_kitty: bool) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();

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
        buf.push_str(&format!(
            "  \"transport_diagnostics\": {},\n",
            serde_json::to_string(&transport_diagnostics)?
        ));
        buf.push_str(&format!(
            "  \"display_tuning\": {},\n",
            serde_json::to_string(&display_tuning)?
        ));
        buf.push_str(&format!("  \"log_path\": {:?},\n", log_path));
        buf.push_str(&format!("  \"log_present\": {},\n", log_present));
        buf.push_str(&format!("  \"log_size_bytes\": {}\n", log_size));
        buf.push_str("}\n");
        write_stdout_or_ignore_broken_pipe(buf.as_bytes())?;
    } else {
        let mut out = String::new();
        out.push_str("kittwm doctor\n");
        out.push_str("============\n");
        let _ = writeln!(out, "  version        : {version}");
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
        let _ = writeln!(
            out,
            "  log            : {} ({}{})",
            log_path,
            if log_present { "present" } else { "missing" },
            if log_present {
                format!(", {log_size} bytes")
            } else {
                String::new()
            }
        );
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
                label: Some(format!("kittwm-doctor-backdrop:{readiness}")),
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
                label: Some(format!(
                    "kittwm-doctor-heading:transport={:?}:compression={:?}",
                    transport.selected_transport, transport.compression_mode
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
                label: Some(format!(
                    "kittwm-doctor-readiness:{readiness}:tmux={}:remote={}:displays={display_count}:{log_state}",
                    transport.tmux, transport.remote
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
                label: Some(format!("kittwm-doctor-display:{display_label}")),
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

fn doctor_display_tuning_label(
    display_tuning: &kittui_cli::session::NativeDisplayTuning,
) -> String {
    format!(
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
    )
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
    let socket_hint = if socket_reachable {
        format!(
            "running WM detected at {}; inspect it with `kittwm info`, `kittwm panes`, or `kittwm events 1000`.",
            socket_path.display()
        )
    } else {
        format!(
            "no running WM socket at {}; start one with `kittwm`, then inspect with `kittwm info`.",
            socket_path.display()
        )
    };
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
                    status: format!("matched:{:?}", response.status),
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
    let component = protocol_token(component, "semantic component")?;
    let action = protocol_token(action, "semantic action")?;
    serde_json::from_str::<serde_json::Value>(payload)
        .map_err(|_| anyhow!("--semantic-action JSON payload must be valid JSON"))?;
    automation_request(
        "SEMANTIC_ACTION",
        window,
        &format!("{component} {action} {payload}"),
    )
}

fn send_key_request(window: &str, key: &str) -> Result<String> {
    let key = protocol_token(key, "--send-key KEY")?;
    automation_request("SEND_KEY", window, &key)
}

fn send_mouse_request(window: &str, event: &str, col: &str, row: &str) -> Result<String> {
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
    automation_request("SEND_MOUSE", window, &format!("{event} {col} {row}"))
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
    let title = title.trim();
    if title.is_empty() {
        return Err(anyhow!("--rename-pane TITLE must be nonempty"));
    }
    protocol_payload_request("RENAME_PANE", &format!("{window} {title}"))
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
    Ok(format!("EVENTS {parsed}"))
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
    let parsed = ms
        .trim()
        .parse::<u64>()
        .map_err(|_| anyhow!("{verb} expects integer milliseconds"))?;
    if parsed == 0 || parsed > 60_000 {
        return Err(anyhow!("{verb} must be in 1..=60000"));
    }
    let needle = wait_needle(needle, verb)?;
    automation_request(verb, window, &format!("{parsed} {needle}"))
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
    Ok(format!(
        "RESTORE_SESSION_JSON {}",
        serde_json::to_string(&value)?
    ))
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
            description: "start the foreground terminal WM",
        },
        LocalCommandEntry {
            command: "stop",
            category: "lifecycle",
            description: "stop a socket daemon",
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
            description: "kittui scene local command catalog",
        },
        LocalCommandEntry {
            command: "commands-kitty",
            category: "help",
            description: "kitty-graphics local command catalog",
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
            description: "WM architecture contract kitty graphics",
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
            description: "serve shared update tools over MCP stdio",
        },
        LocalCommandEntry {
            command: "help <topic>",
            category: "help",
            description: "focused topic help",
        },
        LocalCommandEntry {
            command: "help-scene-json [topic]",
            category: "help",
            description: "focused topic help kittui scene",
        },
        LocalCommandEntry {
            command: "help-kitty [topic]",
            category: "help",
            description: "focused topic help kitty graphics",
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
            description: "daemon status kitty graphics",
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
            command: "layout columns|rows",
            category: "panes",
            description: "change layout axis",
        },
        LocalCommandEntry {
            command: "move [WINDOW] DIR",
            category: "panes",
            description: "move pane",
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
    let mut out = String::from("kittwm commands — local CLI catalog\n");
    let mut current = "";
    for entry in local_command_entries() {
        if entry.category != current {
            current = entry.category;
            out.push_str(&format!("\n{}\n", current.to_ascii_uppercase()));
        }
        out.push_str(&format!("  {:28} {}\n", entry.command, entry.description));
    }
    out.push_str("\nFor socket verbs from a running WM: kittwm --help-json\n");
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
    format!(
        "{}\n",
        serde_json::json!({
            "schema_version": 1,
            "kind": "kittwm-local-commands",
            "commands": commands,
        })
    )
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
            label: Some(format!(
                "kittwm-commands-backdrop:count={}:categories={summary}",
                entries.len()
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
            label: Some(format!(
                "kittwm-command-row:{}:{}:{}",
                entry.category, entry.command, entry.description
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
    format!(
        "{}\n",
        serde_json::to_string(&kittwm_sdk::ArchitectureContract::current())
            .expect("architecture contract serializes")
    )
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
            label: Some(format!(
                "kittwm-architecture-backdrop:layers={}:planes={}:surfaces={}:schema={}",
                contract.layers.len(),
                contract.composition_order.len(),
                contract.first_party_native_surfaces.len(),
                contract.schema_version
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
            label: Some(format!("kittwm-architecture-heading:{}", contract.kind)),
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
            label: Some(format!(
                "kittwm-architecture-layer:{}:owner={}:responsibilities={}:must_not={}:native_contracts={}",
                layer.id,
                layer.owner,
                layer.responsibilities.len(),
                layer.must_not.len(),
                layer.native_contracts.len()
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
            label: Some(format!(
                "kittwm-architecture-plane:{}:z={}",
                plane.plane, plane.z_index
            )),
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
            label: Some(format!(
                "kittwm-architecture-surface:{}:kind={}:sdk={}:kitty={}:kittui={}",
                surface.name,
                surface.surface_kind,
                surface.sdk_backed,
                surface.kitty_graphics_native,
                surface.kittui_entry
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
    format!(
        "{}\n",
        serde_json::json!({
            "schema_version": contract.schema_version,
            "kind": "kittwm-native-surface-coverage",
            "all_ready": all_ready,
            "surfaces": surfaces,
        })
    )
}

fn native_surfaces_json_cmd() -> Result<()> {
    print!("{}", native_surfaces_json_text());
    Ok(())
}

fn native_surfaces_text() -> String {
    let contract = kittwm_sdk::ArchitectureContract::current();
    let mut out = String::from("kittwm native surfaces — SDK + kitty graphics coverage\n");
    out.push_str(&format!(
        "all ready: {}\n\n",
        if contract.all_native_surfaces_ready() {
            "yes"
        } else {
            "no"
        }
    ));
    for surface in &contract.first_party_native_surfaces {
        out.push_str(&format!(
            "  {:16} kind:{:<9} sdk:{:<38} kitty:{}\n",
            surface.name,
            surface.surface_kind,
            surface.sdk_entry,
            if surface.kitty_graphics_native {
                "yes"
            } else {
                "no"
            }
        ));
        out.push_str(&format!("    kittui: {}\n", surface.kittui_entry));
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
            label: Some(format!(
                "kittwm-native-surfaces-backdrop:count={}:all_ready={all_ready}",
                surfaces.len()
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
            label: Some(format!(
                "kittwm-native-surface-row:{}:{}:kind={}:ready={}:sdk={}:kitty={}:plane={plane}:z={z_index}:kittui={}",
                idx,
                surface.name,
                surface.surface_kind,
                surface.is_native_ready(),
                surface.sdk_backed,
                surface.kitty_graphics_native,
                surface.kittui_entry
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

fn completion_words() -> Vec<&'static str> {
    let mut words = local_command_entries()
        .iter()
        .filter_map(|entry| entry.command.split_whitespace().next())
        .collect::<Vec<_>>();
    words.extend([
        "--help",
        "--socket",
        "--display",
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
        "--wait-output-json-ms",
    ]);
    words.sort_unstable();
    words.dedup();
    words
}

fn completions_text(shell: &str) -> Result<String> {
    match shell {
        "bash" => Ok(bash_completions_text()),
        "zsh" => Ok(zsh_completions_text()),
        "fish" => Ok(fish_completions_text()),
        other => Err(anyhow!(
            "unsupported completion shell {other:?}; expected bash, zsh, or fish"
        )),
    }
}

fn push_completion_words(out: &mut String) {
    for (idx, word) in completion_words().into_iter().enumerate() {
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

More help
   kittwm --help
   kittwm help topics
   kittwm help panes
   kittwm help input
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

INSPECT
  kittwm info
  kittwm status
  kittwm panes
  kittwm panes-json
  kittwm events 1000
  kittwm --chrome-json

SPAWN AND TYPE
  kittwm spawn htop
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
  kittwm move last
  kittwm resize focused +2
  kittwm balance
  kittwm rename focused editor

SESSION
  kittwm --save-session session.json
  kittwm --restore-session session.json

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
  kittwm move last          kittwm resize focused +2
  kittwm balance            kittwm rename focused editor

AUTOMATION
  kittwm type focused 'echo hi'
  kittwm line focused 'cargo test'
  kittwm paste focused 'multi-line text'
  kittwm read-json focused
  kittwm wait focused 'Finished'

MORE
  kittwm quickstart         kittwm examples    kittwm help panes
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
            label: Some(format!(
                "kittwm-daily-help-backdrop:{kind_label}:lines={}:commands={command_count}",
                content_lines.len()
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
            label: Some(format!(
                "kittwm-daily-help-heading:{kind_label}:{heading_label}"
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
    ];
    for (idx, line) in content_lines.iter().skip(1).take(20).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        let trimmed = line.trim();
        let row_label = truncate(trimmed, 80);
        layers.push(Layer {
            label: Some(format!(
                "kittwm-daily-help-row:{kind_label}:{idx}:{row_label}"
            )),
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

    let mut out = format!(
        "kittwm info\n  socket: {}\n  workspace: {workspace}\n  chrome: top_bar_rows={top_bar_rows} tilable_rows={tilable_rows}\n  panes: {pane_count} focus={focus} layout={layout}\n",
        socket.display()
    );
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
                let bounds = match (
                    pane.get("x").and_then(serde_json::Value::as_u64),
                    pane.get("y").and_then(serde_json::Value::as_u64),
                    pane.get("cols").and_then(serde_json::Value::as_u64),
                    pane.get("rows").and_then(serde_json::Value::as_u64),
                ) {
                    (Some(x), Some(y), Some(cols), Some(rows)) => {
                        format!(" {x},{y} {cols}x{rows}")
                    }
                    _ => String::new(),
                };
                out.push_str(&format!("  {focused} {window}  {title}{bounds}\n"));
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
            label: Some(format!(
                "kittwm-info-backdrop:workspace={workspace_label}:panes={pane_count}"
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
            label: Some(format!(
                "kittwm-info-heading:socket={socket_label}:focus={focus_label}:layout={layout_label}"
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
            label: Some(format!(
                "kittwm-info-chrome:top_bar_rows={top_bar_rows}:tilable_rows={tilable_rows}"
            )),
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
                label: Some(format!(
                    "kittwm-info-pane:{window_label}:focused={focused}:title={title_label}"
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
    }
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers,
        animation: None,
    }
}

fn info_scene_rows(pane_count: u64) -> u16 {
    pane_count.saturating_add(5).clamp(5, 18) as u16
}

fn daily_help_scene_row_rect(width: f32, y: f32) -> KittuiPxRect {
    info_indicator_rect(width, y)
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
    let reply = client_request_multi(&path, &format!("EVENTS {ms}"))
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
            label: Some(format!(
                "kittwm-events-backdrop:count={}:ms={ms}",
                kinds.len()
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
            label: Some(format!("kittwm-events-heading:{summary}")),
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
            label: Some(format!("kittwm-event-row:{idx}:{kind_label}")),
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
            label: Some(format!(
                "kittwm-panes-backdrop:panes={pane_count}:focus={focus_label}:layout={layout_label}"
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
            label: Some(format!(
                "kittwm-pane-row:{window_label}:focused={focused}:title={title_label}:app={app_cols}x{app_rows}"
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
            label: Some(format!(
                "kittwm-session-backdrop:kind={kind_label}:schema={schema}:layout={layout_label}:focus={focus_label}:panes={}",
                panes.len()
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
            label: Some(format!(
                "kittwm-session-row:{idx}:window={window_label}:title={title_label}:command={command_label}:weight={weight}:focused={focused}"
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
    let rows_data = [
        format!("workspace={workspace_label}"),
        format!("owner={owner_label}"),
        format!("top_bar_rows={top}"),
        format!("bottom_bar_rows={bottom}"),
        format!("left_cols={left}"),
        format!("right_cols={right}"),
        format!("gap_cols={gap_cols}"),
        format!("gap_rows={gap_rows}"),
    ];
    let mut layers = vec![
        Layer {
            label: Some(format!(
                "kittwm-chrome-backdrop:workspace={workspace_label}:owner={owner_label}:top={top}:bottom={bottom}:left={left}:right={right}:gap_cols={gap_cols}:gap_rows={gap_rows}:tilable_rows={tilable_rows_label}"
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
    for (idx, row) in rows_data.iter().enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(format!("kittwm-chrome-row:{idx}:{row}")),
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
    let rows_data = [
        format!("pid={pid}"),
        format!("uptime_s={uptime}"),
        format!("workspace={workspace_label}"),
        format!("layout={layout_label}"),
        format!("focus={focus_label}"),
        format!("panes={panes}"),
        format!("pending={pending}"),
    ];
    let mut layers = vec![
        Layer {
            label: Some(format!(
                "kittwm-status-backdrop:pid={pid}:panes={panes}:pending={pending}:focus={focus_label}:layout={layout_label}:workspace={workspace_label}"
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
            label: Some(format!("kittwm-status-heading:sock={sock_label}")),
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
    for (idx, row) in rows_data.iter().enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(format!("kittwm-status-row:{idx}:{row}")),
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
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let mut layers = vec![
        Layer {
            label: Some(format!("kittwm-shortcuts-backdrop:count={}", entries.len())),
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
    for (idx, entry) in entries.iter().take(12).enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(format!(
                "kittwm-shortcut-row:{}:{}:{}",
                entry.id, entry.keys, entry.description
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
            label: Some(format!(
                "kittwm-keymap-backdrop:bindings={}:prefix={prefix_label}:duplicates={duplicates}",
                km.bindings.len()
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
            label: Some(format!(
                "kittwm-keymap-row:{idx}:{chord_label}:{action_label}"
            )),
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
            label: Some(format!(
                "kittwm-apps-backdrop:path_count={}:macos_count={}:limit={}:filter={filter_label}:default={default_label}:resolved={resolved_label}",
                summary.path_commands.len(),
                summary.macos_apps.len(),
                summary.limit
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
            label: Some(format!("kittwm-app-row:path:{cmd_label}")),
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
            label: Some(format!("kittwm-app-row:macos:{app_label}")),
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

fn launcher_scene(query: &str, selected_idx: usize, candidates: &[AppCandidate]) -> Scene {
    let cols = info_scene_cols();
    let rows = launcher_scene_rows(candidates.len());
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let selected = candidates
        .get(selected_idx)
        .map(|candidate| format!("{}:{}", candidate.kind, truncate(&candidate.name, 48)))
        .unwrap_or_else(|| "none:<none>".to_string());
    let query_label = truncate(query, 48);
    let mut layers = vec![
        Layer {
            label: Some(format!(
                "kittwm-launcher-backdrop:query={query_label}:selected={}:count={}",
                selected_idx + 1,
                candidates.len()
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
            label: Some(format!("kittwm-launcher-heading:selected={selected}")),
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
        let name_label = truncate(&candidate.name, 48);
        layers.push(Layer {
            label: Some(format!(
                "kittwm-launcher-row:{}:{}:{name_label}:selected={selected}",
                idx + 1,
                candidate.kind
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

fn config_scene_for_cols(summary: &ConfigSummary, cols: u16) -> Scene {
    let rows = 30;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let config_path = truncate(&summary.config_path, 48);
    let background_color = truncate(&summary.background_color, 32);
    let background_effects = summary.background_effects.to_string();
    let colorscheme_name = truncate(&summary.colorscheme_name, 32);
    let colorscheme_fg = truncate(&summary.colorscheme_fg, 32);
    let colorscheme_bg = truncate(&summary.colorscheme_bg, 32);
    let colorscheme_colors = summary.colorscheme_colors.to_string();
    let terminal_backend = truncate(&summary.terminal_backend, 32);
    let libghostty_theme = truncate(&summary.libghostty_theme, 32);
    let keymap_path = truncate(&summary.keymap_path, 48);
    let launch_cmd = truncate(&summary.launch_cmd, 48);
    let launch_query = truncate(&summary.launch_query, 48);
    let launcher_overlay = truncate(&summary.launcher_overlay, 48);
    let display_hidpi = summary.hidpi_enabled.to_string();
    let display_cell = format!("{}x{}", summary.cell_width_px, summary.cell_height_px);
    let tile_gap = format!(
        "{}px={}x{}cells",
        summary.tile_gap_px, summary.tile_gap_cols, summary.tile_gap_rows
    );
    let header_gap = format!(
        "{}px={}rows",
        summary.header_gap_px, summary.header_gap_rows
    );
    let footer_gap = format!(
        "{}px={}rows",
        summary.footer_gap_px, summary.footer_gap_rows
    );
    let prefix = truncate(&summary.prefix, 32);
    let status = truncate(summary.status, 32);
    let rows_data = [
        format!("config_path={config_path}"),
        format!("background.color={background_color}"),
        format!("background.opacity={:.2}", summary.background_opacity),
        format!("background.effects={background_effects}"),
        format!("colorscheme.name={colorscheme_name}"),
        format!("colorscheme.fg={colorscheme_fg}"),
        format!("colorscheme.bg={colorscheme_bg}"),
        format!("colorscheme.colors={colorscheme_colors}"),
        format!("terminal.backend={terminal_backend}"),
        format!("libghostty.theme={libghostty_theme}"),
        format!("libghostty.opacity={:.2}", summary.libghostty_opacity),
        format!("display.hidpi={display_hidpi}"),
        format!("display.cell_px={display_cell}"),
        format!("display.tile_gap={tile_gap}"),
        format!("display.header_gap={header_gap}"),
        format!("display.footer_gap={footer_gap}"),
        format!("keymap={keymap_path}"),
        format!("launch_cmd={launch_cmd}"),
        format!("launch_query={launch_query}"),
        format!("launcher_overlay={launcher_overlay}"),
        format!("prefix={prefix}"),
        format!("bindings={}", summary.bindings),
        format!("duplicates={}", summary.duplicate_chords),
        format!("status={status}"),
    ];
    let mut layers = vec![
        Layer {
            label: Some(format!(
                "kittwm-config-backdrop:keymap={keymap_path}:bindings={}:duplicates={}:status={status}",
                summary.bindings, summary.duplicate_chords
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
    for (idx, row) in rows_data.iter().enumerate() {
        let y = (idx as f32 + 2.0) * cell.height_px as f32;
        layers.push(Layer {
            label: Some(format!("kittwm-config-row:{idx}:{row}")),
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
        assert!(parse_log_command(&args(&["tail", "--bad"])).is_err());
    }

    #[test]
    fn log_help_mentions_default_path_and_follow() {
        let help = help_topic_text("log").unwrap();
        assert!(help.contains("/tmp/kittui-wm.log"), "{help}");
        assert!(help.contains("kittwm log tail -f"), "{help}");
    }

    #[test]
    fn unknown_command_errors_point_to_useful_help() {
        let err = friendly_unknown_command_error("pane").to_string();
        assert!(err.contains("unknown kittwm command"), "{err}");
        assert!(err.contains("Did you mean?"), "{err}");
        assert!(err.contains("kittwm panes"), "{err}");
        assert!(err.contains("kittwm quickstart"), "{err}");
        assert!(err.contains("kittwm help topics"), "{err}");
    }

    #[test]
    fn unknown_help_topic_errors_point_to_topics() {
        let err = help_topic_text("panez").unwrap_err().to_string();
        assert!(err.contains("unknown kittwm help topic"), "{err}");
        assert!(err.contains("kittwm help panes"), "{err}");
        assert!(err.contains("kittwm help topics"), "{err}");
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
    fn completions_include_daily_driver_aliases() {
        let bash = completions_text("bash").unwrap();
        assert!(bash.contains("complete -F _kittwm kittwm"), "{bash}");
        assert!(bash.contains("quickstart"), "{bash}");
        assert!(bash.contains("spawn"), "{bash}");
        assert!(bash.contains("--panes-json"), "{bash}");

        let zsh = completions_text("zsh").unwrap();
        assert!(zsh.contains("#compdef kittwm"), "{zsh}");
        assert!(zsh.contains("commands-json"), "{zsh}");

        let fish = completions_text("fish").unwrap();
        assert!(fish.contains("complete -c kittwm"), "{fish}");
        assert!(fish.contains("cheat"), "{fish}");
        assert_eq!(fish, fish_completions_text());
        assert_eq!(fish.capacity(), fish.len());
        assert!(completions_text("powershell").is_err());
    }

    #[test]
    fn commands_catalog_lists_daily_driver_aliases() {
        let text = commands_text();
        assert!(text.contains("kittwm commands"), "{text}");
        assert!(text.contains("LIFECYCLE"), "{text}");
        assert!(text.contains("spawn CMD [ARGS...]"), "{text}");
        assert!(text.contains("focus WINDOW"), "{text}");
        assert!(text.contains("doctor"), "{text}");

        let json: serde_json::Value = serde_json::from_str(&commands_json_text()).unwrap();
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
    fn session_scene_rows_saturate_large_manifest_counts() {
        assert_eq!(session_scene_rows(0), 8);
        assert_eq!(session_scene_rows(4), 9);
        assert_eq!(session_scene_rows(usize::MAX), 24);

        let panes = (0..128)
            .map(|idx| {
                serde_json::json!({
                    "index": idx,
                    "window": format!("native-{idx}"),
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
    fn help_topic_scene_rows_saturate_large_help_text() {
        assert_eq!(help_topic_scene_rows(0), 8);
        assert_eq!(help_topic_scene_rows(8), 12);
        assert_eq!(help_topic_scene_rows(usize::MAX), 30);

        let text = (0..128)
            .map(|idx| format!("kittwm help line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");
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

    #[test]
    fn help_topic_scene_labels_clip_pathological_payloads() {
        let topic = "topic-".repeat(1024);
        let text = format!(
            "{}\n{}\n{}",
            "heading-".repeat(1024),
            "kittwm --".to_string() + &"row-".repeat(2048),
            "plain text"
        );
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
        let huge = format!("{}Needle{}", "x".repeat(10_000), "y".repeat(10_000));
        assert_eq!(candidate_match_score("Needle", "NeEdLe"), Some(0));
        assert_eq!(candidate_match_score("NeedleSuffix", "NEEDLE"), Some(1));
        assert_eq!(candidate_match_score(&huge, "nEeDlE"), Some(2));
        assert_eq!(candidate_match_score(&huge, "missing"), None);
        assert!(ascii_casefold_contains("RésuméNeedle", "NeEdLe"));
        let huge_title = format!("{}Terminal{}", "x".repeat(10_000), "y".repeat(10_000));
        assert!(ascii_casefold_contains(&huge_title, "TERMINAL"));
        assert!(!ascii_casefold_contains(&huge_title, "browser"));
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
        let candidates = vec![
            AppCandidate {
                kind: "path",
                name: "xterm".to_string(),
            },
            AppCandidate {
                kind: "macos",
                name: "Terminal".to_string(),
            },
        ];
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
        let candidates = vec![
            AppCandidate {
                kind: "path",
                name: "xterm".to_string(),
            },
            AppCandidate {
                kind: "macos",
                name: "Terminal".to_string(),
            },
        ];
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
            AppCandidate {
                kind: "path",
                name: "path-command-name-that-is-pathologically-long-and-bloats-labels".to_string(),
            },
            AppCandidate {
                kind: "macos",
                name: "macOS Application Name That Is Pathologically Long And Bloats Labels"
                    .to_string(),
            },
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
            assert!(
                labels
                    .iter()
                    .any(|label| label.starts_with(&format!("kittwm-daily-help-backdrop:{kind}:"))),
                "{kind}: {labels:?}"
            );
            assert!(
                labels
                    .iter()
                    .any(|label| label.contains(&format!("kittwm-daily-help-heading:{kind}:"))),
                "{kind}: {labels:?}"
            );
            assert!(
                labels.iter().any(|label| label.contains(needle)),
                "{kind}: {labels:?}"
            );
        }
    }

    #[test]
    fn daily_help_scene_labels_clip_pathological_payloads() {
        let kind = "kind-".repeat(1024);
        let text = format!(
            "{}\n{}\n{}",
            "heading-".repeat(1024),
            "kittwm ".to_string() + &"row-".repeat(2048),
            "plain text"
        );
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
                        key: format!("{large_key}-other"),
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
    fn shortcuts_scene_rows_saturate_large_catalog_counts() {
        assert_eq!(shortcuts_scene_rows(0), 4);
        assert_eq!(shortcuts_scene_rows(3), 6);
        assert_eq!(shortcuts_scene_rows(usize::MAX), 18);
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
                    .contains("kittwm-native-surfaces-backdrop:count=3:all_ready=true")),
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
        let json: serde_json::Value = serde_json::from_str(&native_surfaces_json_text()).unwrap();
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
        let json: serde_json::Value =
            serde_json::from_str(&architecture_contract_json_text()).unwrap();
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
        assert!(text.contains("kittwm help topics"), "{text}");
    }

    #[test]
    fn examples_are_copy_paste_daily_driver_commands() {
        let text = examples_text();
        for line in [
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
    fn events_scene_rows_saturate_large_event_counts() {
        assert_eq!(events_scene_rows(0), 5);
        assert_eq!(events_scene_rows(3), 7);
        assert_eq!(events_scene_rows(usize::MAX), 18);

        let kinds = (0..128)
            .map(|idx| format!("pane_frame_presented_{idx}"))
            .collect::<Vec<_>>();
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
    fn panes_scene_rows_saturate_large_detail_counts() {
        assert_eq!(panes_scene_rows(0), 5);
        assert_eq!(panes_scene_rows(3), 7);
        assert_eq!(panes_scene_rows(usize::MAX), 18);

        let details = (0..128)
            .map(|idx| {
                serde_json::json!({
                    "window": format!("native-{idx}"),
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
        assert!(text.contains("--wait-output-json-ms"), "{text}");
        assert!(text.contains("kittwm shortcuts"), "{text}");
        assert!(text.contains("kittwm showcase-scene-json"), "{text}");
        assert!(text.contains("kittwm showcase-metrics-json"), "{text}");
        assert!(text.contains("kittwm showcase-composition-json"), "{text}");
        assert!(text.contains("kittwm tui-smoke-json"), "{text}");
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
    fn help_topic_panes_is_focused() {
        let text = help_topic_text("panes").unwrap();
        assert!(text.contains("--spawn-pty CMD"), "{text}");
        assert!(text.contains("--balance-panes"), "{text}");
        assert!(!text.contains("--probe-kitty"), "{text}");
    }

    #[test]
    fn help_topic_input_is_focused() {
        let text = help_topic_text("input").unwrap();
        assert!(text.contains("--send-text WINDOW TEXT"), "{text}");
        assert!(text.contains("--semantic-action"), "{text}");
        assert!(!text.contains("--save-session"), "{text}");
    }

    #[test]
    fn help_topic_apps_mentions_bar_chrome_contract() {
        let text = help_topic_text("apps").unwrap();
        assert!(text.contains("kittwm-launch"), "{text}");
        assert!(text.contains("kittwm-bar --kitty --reserve"), "{text}");
        assert!(text.contains("kittwm-bar --release"), "{text}");
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
            parse_pane_control_alias("resize", args(&["native-2", "+2"]).into_iter()).unwrap(),
            "RESIZE_PANE native-2 +2"
        );
        assert_eq!(
            parse_pane_control_alias("balance", Vec::<String>::new().into_iter()).unwrap(),
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
        assert!(parse_pane_control_alias("layout", args(&["diagonal"]).into_iter()).is_err());
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
            rename_pane_request("native-2", " Editor Pane ").unwrap(),
            "RENAME_PANE native-2 Editor Pane"
        );
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
        assert_eq!(
            automation_request("read_text", "native-2", "").unwrap(),
            "READ_TEXT native-2"
        );
        assert_eq!(
            automation_request("READ_TEXT_JSON", "focused", "").unwrap(),
            "READ_TEXT_JSON focused"
        );
        assert_eq!(
            automation_request("READ_SCROLLBACK_JSON", "native-2", "").unwrap(),
            "READ_SCROLLBACK_JSON native-2"
        );
        assert_eq!(
            wait_ms_request("WAIT_TEXT_MS", "2500", "focused", "Ready Now").unwrap(),
            "WAIT_TEXT_MS focused 2500 Ready Now"
        );
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
        assert_eq!(
            send_mouse_request("focused", "press-left", "7", "9").unwrap(),
            "SEND_MOUSE focused press-left 7 9"
        );
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
        assert_eq!(
            semantic_action_request(
                "focused",
                "native-1.screen",
                "insert_text",
                r#"{"text":"hi"}"#
            )
            .unwrap(),
            r#"SEMANTIC_ACTION focused native-1.screen insert_text {"text":"hi"}"#
        );
        assert!(semantic_action_request("focused", "bad component", "set", "{}").is_err());
        assert!(semantic_action_request("focused", "field", "set", "not-json").is_err());
        assert!(semantic_publish_request("focused", "not-json").is_err());
        assert_eq!(events_request("2500").unwrap(), "EVENTS 2500");
        assert!(events_request("0").is_err());
        assert!(events_request("60001").is_err());
        assert!(wait_ms_request("WAIT_TEXT_MS", "0", "focused", "ready").is_err());
        assert!(send_mouse_request("focused", "drag", "7", "9").is_err());
        assert!(send_key_request("focused", "page down").is_err());
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
