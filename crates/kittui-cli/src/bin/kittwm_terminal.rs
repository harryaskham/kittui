use std::env;
use std::fmt::Write as FmtWrite;
use std::process::ExitCode;

use kittui::{
    CellRect, CellSize, Corners, Layer, Node, Paint, PxRect, Rgba, Runtime, Scene, Stroke,
    TerminalInfo,
};
use kittwm_sdk::{Kittwm, KittwmConfig, PanesStatus, Status, SurfaceSpec, WindowSpec};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusMode {
    None,
    Text,
    SceneJson,
    Kitty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventsMode {
    Text,
    SceneJson,
    Kitty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventsRequest {
    ms: u64,
    mode: EventsMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalArgs {
    replace: bool,
    title: Option<String>,
    command: String,
    command_explicit: bool,
    remote_host: Option<String>,
    status: StatusMode,
    events: Option<EventsRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct TerminalStatusModel {
    panes: u64,
    focus: String,
    layout: String,
    details: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct TerminalEventsModel {
    ms: u64,
    count: usize,
    kinds: Vec<String>,
}

impl TerminalArgs {
    fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut replace = false;
        let mut title = None;
        let mut command = None;
        let mut remote_host = None;
        let mut status = StatusMode::None;
        let mut events = None;
        let mut iter = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => return Err(help_text()),
                "--replace" => replace = true,
                "--new-window" => replace = false,
                "--status" if status == StatusMode::None => status = StatusMode::Text,
                "--status-scene-json" if status == StatusMode::None => {
                    status = StatusMode::SceneJson
                }
                "--status-kitty" | "--status-graphics" if status == StatusMode::None => {
                    status = StatusMode::Kitty
                }
                "--status" | "--status-scene-json" | "--status-kitty" | "--status-graphics" => {
                    return Err("choose only one status output mode".to_string())
                }
                "--events-ms" => {
                    if events.is_some() {
                        return Err("choose only one events output mode".to_string());
                    }
                    events = Some(EventsRequest {
                        ms: parse_events_ms(&mut iter, "--events-ms")?,
                        mode: EventsMode::Text,
                    });
                }
                "--events-scene-json" => {
                    if events.is_some() {
                        return Err("choose only one events output mode".to_string());
                    }
                    events = Some(EventsRequest {
                        ms: parse_events_ms(&mut iter, "--events-scene-json")?,
                        mode: EventsMode::SceneJson,
                    });
                }
                "--events-kitty" | "--events-graphics" => {
                    if events.is_some() {
                        return Err("choose only one events output mode".to_string());
                    }
                    events = Some(EventsRequest {
                        ms: parse_events_ms(&mut iter, arg.as_str())?,
                        mode: EventsMode::Kitty,
                    });
                }
                "--title" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--title requires a value".to_string())?;
                    title = Some(value);
                }
                "--remote" | "--host" => {
                    remote_host = Some(
                        iter.next()
                            .ok_or_else(|| "--remote requires a host".to_string())?,
                    );
                }
                "--command" | "-c" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--command requires a value".to_string())?;
                    command = Some(value);
                }
                "--" => {
                    if iter.peek().is_some() {
                        command = Some(shell_words_from_iter(iter.by_ref()));
                    }
                    break;
                }
                other if other.starts_with('-') => return Err(unknown_option_error(other)),
                other => {
                    command = Some(shell_words_from_iter(
                        std::iter::once(other.to_string()).chain(iter),
                    ));
                    break;
                }
            }
        }
        let command_explicit = command.is_some();
        Ok(Self {
            replace,
            title,
            command: command.unwrap_or_else(default_terminal_command),
            command_explicit,
            remote_host,
            status,
            events,
        })
    }
}

fn unknown_option_error(option: &str) -> String {
    let help = help_text();
    let mut out = String::with_capacity("unknown option \n\n".len() + option.len() + help.len());
    out.push_str("unknown option ");
    out.push_str(option);
    out.push_str("\n\n");
    out.push_str(&help);
    out
}

fn parse_events_ms<I>(iter: &mut std::iter::Peekable<I>, flag: &str) -> Result<u64, String>
where
    I: Iterator<Item = String>,
{
    let value = iter
        .next()
        .ok_or_else(|| terminal_flag_error(flag, " requires milliseconds"))?;
    value
        .parse()
        .map_err(|_| terminal_flag_error(flag, " expects an integer"))
}

fn terminal_flag_error(flag: &str, message: &str) -> String {
    let mut out = String::with_capacity(flag.len() + message.len());
    out.push_str(flag);
    out.push_str(message);
    out
}

fn default_terminal_command() -> String {
    let config = KittwmConfig::load_default().unwrap_or_default();
    env::var("KITTWM_TERMINAL_CMD")
        .or_else(|_| env::var("KITTWM_TERMINAL_BINARY"))
        .or_else(|_| config.terminal.command.ok_or(env::VarError::NotPresent))
        .or_else(|_| env::var("SHELL").map(login_shell_command))
        .unwrap_or_else(|_| "/bin/sh -l".to_string())
}

fn login_shell_command(shell: String) -> String {
    let mut out = String::with_capacity(shell.len() + " -l".len());
    out.push_str(&shell);
    out.push_str(" -l");
    out
}

#[cfg(test)]
fn shell_words(args: &[String]) -> String {
    let mut out = String::with_capacity(
        args.iter()
            .map(|arg| arg.len().saturating_add(2))
            .sum::<usize>(),
    );
    push_shell_words(&mut out, args.iter());
    out
}

fn shell_words_from_iter<I>(args: I) -> String
where
    I: IntoIterator<Item = String>,
{
    let iter = args.into_iter();
    let (lower, upper) = iter.size_hint();
    let estimated_args = upper.unwrap_or(lower).max(lower);
    let mut out = String::with_capacity(estimated_args.saturating_mul(8));
    push_shell_words(&mut out, iter);
    out
}

fn push_shell_words<I, S>(out: &mut String, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for (idx, arg) in args.into_iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        push_shell_word(out, arg.as_ref());
    }
}

fn push_shell_word(out: &mut String, arg: &str) {
    if arg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    {
        out.push_str(arg);
        return;
    }
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

fn help_text() -> String {
    "kittwm-terminal — first-party terminal client for kittwm\n\n\
Usage:\n  kittwm-terminal [--replace|--new-window] [--title TITLE] [--command CMD]\n  kittwm-terminal [--replace|--new-window] [--title TITLE] -- PROGRAM [ARGS...]\n  kittwm-terminal --remote HOST [--title TITLE] [--command CMD]\n  kittwm-terminal --status\n  kittwm-terminal --status-scene-json\n  kittwm-terminal --status-kitty\n  kittwm-terminal --events-ms MS\n  kittwm-terminal --events-scene-json MS\n  kittwm-terminal --events-kitty MS\n\n\
Options:\n  --replace              Replace the currently focused pane (default)\n  --new-window           Spawn a new kittwm native pane\n  --remote HOST          Open a local kittwm terminal pane running pooled SSH\n  --title TITLE          Set the terminal surface title\n  --command CMD, -c CMD  Run CMD through the configured shell\n  --status               Print typed SDK status/pane detail\n  --status-scene-json    Emit the status card as kittui Scene JSON\n  --status-kitty         Render the status card with kitty graphics\n  --events-ms MS         Print a bounded event batch\n  --events-scene-json MS Emit the event batch as kittui Scene JSON\n  --events-kitty MS      Render the event batch with kitty graphics\n  --help, -h             Show this help text\n\n\
Examples:\n  kittwm-terminal\n  kittwm-terminal --title logs -- tail -f /tmp/app.log\n  kittwm-terminal --remote buildbox          # pane title defaults to buildbox\n  kittwm-terminal --remote buildbox -- htop  # pane title defaults to buildbox: htop\n  kittwm-terminal --remote buildbox --title logs -- tail -f /tmp/app.log\n  kittwm-terminal --replace --command 'zsh -l'\n  kittwm-terminal --status\n  kittwm-terminal --events-ms 1000\n\n\
Connects through KITTWM_SOCKET/KITTWM_DISPLAY using kittwm-sdk and asks the\n\
running kittwm instance to spawn or replace a native terminal surface.\n\
--status prints typed SDK status/pane detail; --status-scene-json and\n\
--status-kitty render the same model as a kittui/kitty-native status card;\n\
--events-ms prints a bounded event batch for lifecycle/debugging;\n\
--events-scene-json and --events-kitty render that batch as a kittui card.\n"
        .to_string()
}

fn terminal_status_model(status: Status, panes: PanesStatus) -> TerminalStatusModel {
    TerminalStatusModel {
        panes: status.panes.unwrap_or(panes.panes),
        focus: status.focus.unwrap_or(panes.focus),
        layout: status.layout.unwrap_or(panes.layout),
        details: panes.panes_detail.len(),
    }
}

fn render_status_text(model: &TerminalStatusModel) -> String {
    let mut out = String::with_capacity(
        "status panes= focus= layout= details=\n".len()
            + 20
            + model.focus.len()
            + model.layout.len()
            + 20,
    );
    let _ = write!(
        out,
        "status panes={} focus={} layout={} details={}\n",
        model.panes, model.focus, model.layout, model.details
    );
    out
}

fn terminal_status_scene(model: &TerminalStatusModel) -> Scene {
    terminal_status_scene_for_cols(model, terminal_status_scene_cols())
}

fn terminal_status_scene_for_cols(model: &TerminalStatusModel, cols: u16) -> Scene {
    let rows = 5;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let content_rect = terminal_card_content_rect(width, cell);
    let status_label = terminal_status_scene_text_label(model);
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers: vec![
            Layer {
                label: Some("kittwm-terminal-status-backdrop".to_string()),
                root: Node::Rect {
                    rect: PxRect::new(0.0, 0.0, width, height),
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
                label: Some("kittwm-terminal-status-heading".to_string()),
                root: Node::Rect {
                    rect: PxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
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
                label: Some(status_label),
                root: Node::Rect {
                    rect: content_rect,
                    fill: Paint::Solid {
                        color: Rgba::rgba(163, 190, 140, 255),
                    },
                    stroke: None,
                    corners: Corners::uniform(1.0),
                },
            },
        ],
        animation: None,
    }
}

fn terminal_status_scene_text_label(model: &TerminalStatusModel) -> String {
    let focus = terminal_scene_label_text(&model.focus, 24);
    let layout = terminal_scene_label_text(&model.layout, 24);
    let details = terminal_scene_label_text(&model.details.to_string(), 12);
    let mut out = String::with_capacity(
        "kittwm-terminal-status-text:panes= focus= layout= details=".len()
            + 20
            + focus.len()
            + layout.len()
            + details.len(),
    );
    let _ = write!(
        out,
        "kittwm-terminal-status-text:panes={} focus={} layout={} details={}",
        model.panes, focus, layout, details
    );
    out
}

fn terminal_status_scene_cols() -> u16 {
    let detected = TerminalInfo::detect().columns;
    terminal_status_scene_cols_from_sources(
        env::var("KITTWM_TERMINAL_STATUS_COLS")
            .or_else(|_| env::var("COLUMNS"))
            .ok()
            .as_deref(),
        detected,
    )
}

fn terminal_status_scene_cols_from_sources(value: Option<&str>, detected_cols: Option<u16>) -> u16 {
    value
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|cols| *cols > 0)
        .or_else(|| detected_cols.filter(|cols| *cols > 0))
        .map(|cols| cols.min(120))
        .unwrap_or(56)
}

fn terminal_card_content_rect(width: f32, cell: CellSize) -> PxRect {
    let margin = 10.0_f32.min((width / 4.0).max(0.0));
    PxRect::new(
        margin,
        cell.height_px as f32 * 2.2,
        (width - margin * 2.0).max(1.0),
        2.0,
    )
}

fn render_status_kitty(model: &TerminalStatusModel) -> Result<String, String> {
    render_scene_kitty(&terminal_status_scene(model))
}

fn terminal_events_model(ms: u64, kinds: Vec<String>) -> TerminalEventsModel {
    TerminalEventsModel {
        ms: ms.clamp(1, 60_000),
        count: kinds.len(),
        kinds,
    }
}

fn render_events_text(model: &TerminalEventsModel) -> String {
    let body_len: usize = model
        .kinds
        .iter()
        .map(|kind| kind.len().saturating_add(1))
        .sum();
    let mut out = String::with_capacity(32usize.saturating_add(body_len));
    out.push_str("events count=");
    out.push_str(&model.count.to_string());
    out.push_str(" ms=");
    out.push_str(&model.ms.to_string());
    out.push('\n');
    for kind in &model.kinds {
        out.push_str(kind);
        out.push('\n');
    }
    out
}

fn terminal_scene_label_text(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let mut out = String::with_capacity(max);
    for _ in 0..max {
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

fn terminal_events_scene(model: &TerminalEventsModel) -> Scene {
    terminal_events_scene_for_cols(model, terminal_status_scene_cols())
}

fn terminal_events_heading_label(model: &TerminalEventsModel) -> String {
    let mut label = String::with_capacity("kittwm-terminal-events-heading:count= ms=".len() + 40);
    let _ = write!(
        label,
        "kittwm-terminal-events-heading:count={} ms={}",
        model.count, model.ms
    );
    label
}

fn terminal_events_kinds_label(summary: &str) -> String {
    let mut label = String::with_capacity("kittwm-terminal-events-kinds:".len() + summary.len());
    label.push_str("kittwm-terminal-events-kinds:");
    label.push_str(summary);
    label
}

fn terminal_events_scene_for_cols(model: &TerminalEventsModel, cols: u16) -> Scene {
    let rows = 5;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let content_rect = terminal_card_content_rect(width, cell);
    let summary = terminal_events_summary_label(&model.kinds);
    let kinds_label = terminal_events_kinds_label(&summary);
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: cell,
        layers: vec![
            Layer {
                label: Some("kittwm-terminal-events-backdrop".to_string()),
                root: Node::Rect {
                    rect: PxRect::new(0.0, 0.0, width, height),
                    fill: Paint::Solid {
                        color: Rgba::rgba(17, 25, 44, 238),
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
                label: Some(terminal_events_heading_label(model)),
                root: Node::Rect {
                    rect: PxRect::new(0.0, 0.0, width, cell.height_px as f32 * 1.4),
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
                label: Some(kinds_label),
                root: Node::Rect {
                    rect: content_rect,
                    fill: Paint::Solid {
                        color: Rgba::rgba(235, 203, 139, 255),
                    },
                    stroke: None,
                    corners: Corners::uniform(1.0),
                },
            },
        ],
        animation: None,
    }
}

fn terminal_events_summary_label(kinds: &[String]) -> String {
    let mut summary = String::with_capacity(kinds.len().min(5).saturating_mul(25));
    for kind in kinds.iter().take(5) {
        if !summary.is_empty() {
            summary.push(',');
        }
        summary.push_str(&terminal_scene_label_text(kind, 24));
    }
    summary
}

fn render_events_kitty(model: &TerminalEventsModel) -> Result<String, String> {
    render_scene_kitty(&terminal_events_scene(model))
}

fn render_scene_kitty(scene: &Scene) -> Result<String, String> {
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()
        .map_err(|err| err.to_string())?;
    let options = terminal_scene_placement_options();
    runtime
        .place_at_with_options(scene, scene.footprint, &options)
        .map(|placement| placement.to_bytes())
        .map_err(|err| err.to_string())
}

fn terminal_scene_placement_options() -> kittui_kitty::PlacementOptions {
    let mut options = kittui_kitty::PlacementOptions::absolute();
    options.z_index = 20;
    options
}

fn run(args: TerminalArgs) -> Result<String, String> {
    let wm = Kittwm::connect_from_env().map_err(|err| sdk_error("connect to kittwm", &err))?;
    if args.status != StatusMode::None {
        let status = wm.status().map_err(|err| sdk_error("read status", &err))?;
        let panes = wm.panes().map_err(|err| sdk_error("read panes", &err))?;
        let model = terminal_status_model(status, panes);
        return match args.status {
            StatusMode::Text => Ok(render_status_text(&model)),
            StatusMode::SceneJson => scene_json_line(&terminal_status_scene(&model))
                .map_err(|err| sdk_error("encode status scene", &err)),
            StatusMode::Kitty => render_status_kitty(&model),
            StatusMode::None => unreachable!(),
        };
    }
    if let Some(request) = args.events {
        let events = wm
            .events_ms(request.ms)
            .map_err(|err| sdk_error("read events", &err))?;
        let model = terminal_events_model(
            request.ms,
            events
                .into_iter()
                .map(|event| event.kind().to_string())
                .collect(),
        );
        return match request.mode {
            EventsMode::Text => Ok(render_events_text(&model)),
            EventsMode::SceneJson => scene_json_line(&terminal_events_scene(&model))
                .map_err(|err| sdk_error("encode events scene", &err)),
            EventsMode::Kitty => render_events_kitty(&model),
        };
    }
    let command = terminal_spawn_command(&args)?;
    let title = terminal_surface_title(&args);
    if args.replace {
        wm.replace_current(&WindowSpec { title, command })
            .map_err(|err| sdk_error("replace current terminal", &err))
    } else {
        let mut spec = SurfaceSpec::terminal(command);
        if let Some(title) = title {
            spec = spec.titled(title);
        }
        wm.spawn_surface(&spec)
            .map(|spawn| spawn.reply)
            .map_err(|err| sdk_error("spawn terminal surface", &err))
    }
}

fn terminal_spawn_command(args: &TerminalArgs) -> Result<String, String> {
    if let Some(host) = args.remote_host.as_deref() {
        return remote_terminal_command(host, &args.command, args.command_explicit);
    }
    Ok(args.command.clone())
}

fn terminal_surface_title(args: &TerminalArgs) -> Option<String> {
    args.title.clone().or_else(|| {
        args.remote_host
            .as_ref()
            .map(|host| remote_terminal_default_title(host, &args.command, args.command_explicit))
    })
}

fn remote_terminal_default_title(host: &str, command: &str, command_explicit: bool) -> String {
    if command_explicit {
        let command_label = command.split_whitespace().next().unwrap_or(command);
        if !command_label.is_empty() {
            return format!("{host}: {command_label}");
        }
    }
    host.to_string()
}

fn remote_terminal_command(
    host: &str,
    command: &str,
    command_explicit: bool,
) -> Result<String, String> {
    if host.trim().is_empty() {
        return Err("--remote requires a non-empty host".to_string());
    }
    let control_path =
        remote_terminal_control_path().map_err(|err| sdk_error("prepare ssh pool", &err))?;
    let mut argv = vec![
        "ssh".to_string(),
        "-tt".to_string(),
        "-o".to_string(),
        "ControlMaster=auto".to_string(),
        "-o".to_string(),
        "ControlPersist=10m".to_string(),
        "-o".to_string(),
        ssh_control_path_arg(&control_path),
        host.to_string(),
    ];
    if command_explicit {
        argv.extend(["sh".to_string(), "-lc".to_string(), command.to_string()]);
    }
    Ok(shell_words(&argv))
}

fn ssh_control_path_arg(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    let mut out = String::with_capacity("ControlPath=".len() + path.len());
    out.push_str("ControlPath=");
    out.push_str(&path);
    out
}

fn remote_terminal_control_path() -> std::io::Result<std::path::PathBuf> {
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".cache"))
        })
        .unwrap_or_else(env::temp_dir);
    let dir = base.join("kittwm-ssh");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("%C"))
}

#[cfg(not(test))]
fn shell_words(args: &[String]) -> String {
    let mut out = String::with_capacity(
        args.iter()
            .map(|arg| arg.len().saturating_add(2))
            .sum::<usize>(),
    );
    push_shell_words(&mut out, args.iter());
    out
}

fn sdk_error(prefix: &str, err: &impl std::fmt::Display) -> String {
    let mut out = String::with_capacity(prefix.len().saturating_add(2).saturating_add(64));
    out.push_str(prefix);
    out.push_str(": ");
    let _ = write!(out, "{err}");
    out
}

fn scene_json_line(scene: &Scene) -> Result<String, serde_json::Error> {
    let mut out = serde_json::to_string(scene)?;
    out.push('\n');
    Ok(out)
}

fn main() -> ExitCode {
    let parsed = match TerminalArgs::parse_from(env::args().skip(1)) {
        Ok(args) => args,
        Err(message) if message.starts_with("kittwm-terminal") => {
            print!("{message}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(parsed) {
        Ok(reply) => {
            print!("{reply}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("kittwm-terminal: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittwm_sdk::{PanesStatus, Status};

    #[test]
    fn parses_replace_title_and_command() {
        let args =
            TerminalArgs::parse_from(["--replace", "--title", "dev shell", "--command", "zsh -l"])
                .unwrap();
        assert_eq!(
            args,
            TerminalArgs {
                replace: true,
                title: Some("dev shell".to_string()),
                command: "zsh -l".to_string(),
                command_explicit: true,
                remote_host: None,
                status: StatusMode::None,
                events: None,
            }
        );
    }

    #[test]
    fn parses_program_after_separator() {
        let args = TerminalArgs::parse_from(["--", "echo", "hello world"]).unwrap();
        assert_eq!(args.command, "echo 'hello world'");

        let bare = TerminalArgs::parse_from(["printf", "it's", "ok"]).unwrap();
        assert_eq!(bare.command, "printf 'it'\\''s' ok");
    }

    #[test]
    fn login_shell_command_builds_directly() {
        let command = login_shell_command("/bin/zsh".to_string());
        assert_eq!(command, "/bin/zsh -l");
        assert_eq!(command.capacity(), command.len());
    }

    #[test]
    fn ssh_control_path_arg_builds_directly() {
        let path = std::path::PathBuf::from("/tmp/kittwm-ssh/%C");
        let arg = ssh_control_path_arg(&path);
        assert_eq!(arg, "ControlPath=/tmp/kittwm-ssh/%C");
        assert_eq!(arg.capacity(), arg.len());
    }

    #[test]
    fn parses_remote_terminal_host_and_builds_pooled_ssh_command() {
        let args = TerminalArgs::parse_from(["--remote", "buildbox", "--", "htop"]).unwrap();
        assert_eq!(args.remote_host.as_deref(), Some("buildbox"));
        assert_eq!(
            terminal_surface_title(&args).as_deref(),
            Some("buildbox: htop")
        );
        assert_eq!(
            remote_terminal_default_title("buildbox", "htop -d 1", true),
            "buildbox: htop"
        );
        assert!(args.command_explicit);
        let command =
            remote_terminal_command("buildbox", &args.command, args.command_explicit).unwrap();
        assert!(command.contains("ssh -tt"), "{command}");
        assert!(command.contains("ControlMaster=auto"), "{command}");
        assert!(command.contains("ControlPersist=10m"), "{command}");
        assert!(command.contains("buildbox sh -lc htop"), "{command}");

        let titled = TerminalArgs::parse_from([
            "--remote",
            "buildbox",
            "--title",
            "logs",
            "--",
            "tail",
            "-f",
            "/tmp/app.log",
        ])
        .unwrap();
        assert_eq!(terminal_surface_title(&titled).as_deref(), Some("logs"));

        let login_args = TerminalArgs::parse_from(["--remote", "buildbox"]).unwrap();
        assert_eq!(
            terminal_surface_title(&login_args).as_deref(),
            Some("buildbox")
        );

        let login = remote_terminal_command("buildbox", "/bin/sh -l", false).unwrap();
        assert!(login.contains("ssh -tt"), "{login}");
        assert!(login.ends_with(" buildbox"), "{login}");
    }

    #[test]
    fn unknown_option_error_includes_help() {
        let err = TerminalArgs::parse_from(["--wat"]).unwrap_err();
        assert!(err.starts_with("unknown option --wat"), "{err}");
        assert!(err.contains("kittwm-terminal"), "{err}");
        assert_eq!(err.capacity(), err.len());
    }

    #[test]
    fn sdk_error_builds_diagnostics_directly() {
        let err = sdk_error("connect to kittwm", &"socket missing");
        assert_eq!(err, "connect to kittwm: socket missing");
        assert!(err.capacity() >= err.len());
        let encode = sdk_error("encode status scene", &"bad scene");
        assert_eq!(encode, "encode status scene: bad scene");
    }

    #[test]
    fn events_ms_errors_include_flag_without_formatting() {
        let missing = TerminalArgs::parse_from(["--events-ms"]).unwrap_err();
        assert_eq!(missing, "--events-ms requires milliseconds");
        assert_eq!(missing.capacity(), missing.len());
        let invalid = TerminalArgs::parse_from(["--events-kitty", "soon"]).unwrap_err();
        assert_eq!(invalid, "--events-kitty expects an integer");
        assert_eq!(invalid.capacity(), invalid.len());
    }

    #[test]
    fn parses_status_and_events_modes() {
        let status = TerminalArgs::parse_from(["--status"]).unwrap();
        assert_eq!(status.status, StatusMode::Text);
        assert_eq!(status.events, None);
        let scene = TerminalArgs::parse_from(["--status-scene-json"]).unwrap();
        assert_eq!(scene.status, StatusMode::SceneJson);
        let kitty = TerminalArgs::parse_from(["--status-kitty"]).unwrap();
        assert_eq!(kitty.status, StatusMode::Kitty);
        let events = TerminalArgs::parse_from(["--events-ms", "250"]).unwrap();
        assert_eq!(events.status, StatusMode::None);
        assert_eq!(
            events.events,
            Some(EventsRequest {
                ms: 250,
                mode: EventsMode::Text,
            })
        );
        let event_scene = TerminalArgs::parse_from(["--events-scene-json", "500"]).unwrap();
        assert_eq!(
            event_scene.events,
            Some(EventsRequest {
                ms: 500,
                mode: EventsMode::SceneJson,
            })
        );
        let event_kitty = TerminalArgs::parse_from(["--events-kitty", "750"]).unwrap();
        assert_eq!(
            event_kitty.events,
            Some(EventsRequest {
                ms: 750,
                mode: EventsMode::Kitty,
            })
        );
        let err = TerminalArgs::parse_from(["--status", "--status-kitty"]).unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
        let err =
            TerminalArgs::parse_from(["--events-ms", "10", "--events-kitty", "10"]).unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
    }

    #[test]
    fn terminal_status_scene_width_respects_narrow_columns() {
        assert_eq!(terminal_status_scene_cols_from_sources(Some("8"), None), 8);
        assert_eq!(terminal_status_scene_cols_from_sources(Some("0"), None), 56);
        assert_eq!(
            terminal_status_scene_cols_from_sources(None, Some(100)),
            100
        );
        assert_eq!(
            terminal_status_scene_cols_from_sources(Some("0"), Some(100)),
            100
        );
        assert_eq!(
            terminal_status_scene_cols_from_sources(Some("240"), Some(100)),
            120
        );
        assert_eq!(
            terminal_status_scene_cols_from_sources(None, Some(u16::MAX)),
            120
        );

        let model = TerminalStatusModel {
            panes: 1,
            focus: "native-1".to_string(),
            layout: "columns".to_string(),
            details: 1,
        };
        let scene = terminal_status_scene_for_cols(&model, 1);
        assert_eq!(scene.footprint.cols, 1);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in &scene.layers {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
        assert_eq!(
            terminal_card_content_rect(8.0, CellSize::default())
                .origin
                .0,
            2.0
        );
    }

    #[test]
    fn shell_words_builds_directly_and_preserves_quoting() {
        assert_eq!(shell_words(&[]), "");
        assert_eq!(
            shell_words(&[
                "printf".to_string(),
                "hello world".to_string(),
                "it's".to_string(),
                "/tmp/file:name".to_string(),
            ]),
            "printf 'hello world' 'it'\\''s' /tmp/file:name"
        );
    }

    #[test]
    fn scene_json_line_appends_newline_directly() {
        let model = TerminalStatusModel {
            panes: 1,
            focus: "native-1".to_string(),
            layout: "columns".to_string(),
            details: 0,
        };
        let json = scene_json_line(&terminal_status_scene(&model)).unwrap();
        assert!(json.ends_with('\n'));
        assert!(json.contains("kittwm-terminal-status-backdrop"), "{json}");
    }

    #[test]
    fn status_model_scene_contains_typed_sdk_status() {
        let model = terminal_status_model(
            Status {
                pending: Some(0),
                panes: Some(2),
                focus: Some("native-2".to_string()),
                layout: Some("rows".to_string()),
                workspace: None,
                chrome: None,
                focused_pane: None,
                panes_detail: Vec::new(),
            },
            PanesStatus {
                panes: 3,
                focus: "native-3".to_string(),
                layout: "columns".to_string(),
                workspace: None,
                chrome: None,
                panes_detail: vec![kittwm_sdk::NativePaneDetail {
                    window: "native-1".to_string(),
                    title: "shell".to_string(),
                    weight: 1,
                    ..kittwm_sdk::NativePaneDetail::default()
                }],
            },
        );
        let text = render_status_text(&model);
        assert_eq!(
            text,
            "status panes=2 focus=native-2 layout=rows details=1\n"
        );
        assert!(text.capacity() >= text.len());
        let scene = terminal_status_scene(&model);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.contains(&"kittwm-terminal-status-backdrop"),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("panes=2 focus=native-2 layout=rows details=1")),
            "{labels:?}"
        );
    }

    #[test]
    fn terminal_scene_label_text_uses_bounded_prefix_for_huge_fields() {
        let huge = "terminal-event-".repeat(10_000);
        let clipped = terminal_scene_label_text(&huge, 16);
        assert_eq!(clipped, "terminal-event-…");
        assert_eq!(clipped.chars().count(), 16);
        assert!(clipped.capacity() >= 16);
        let short = terminal_scene_label_text("short", 16);
        assert_eq!(short, "short");
        assert!(short.capacity() >= 16);
        assert_eq!(terminal_scene_label_text("anything", 1), "…");
        assert_eq!(terminal_scene_label_text("anything", 0), "");
    }

    #[test]
    fn terminal_status_scene_bounds_text_label_payloads() {
        let model = TerminalStatusModel {
            panes: 123,
            focus: "focused-window-with-a-pathologically-long-id".to_string(),
            layout: "layout-name-that-is-far-too-long-for-a-scene-label".to_string(),
            details: usize::MAX,
        };
        let direct = terminal_status_scene_text_label(&model);
        assert!(direct.contains("focused-window-with-a-p…"), "{direct}");
        assert!(direct.contains("layout-name-that-is-far…"), "{direct}");
        assert!(direct.len() < 150, "{direct}");
        assert!(direct.capacity() >= direct.len());

        let scene = terminal_status_scene_for_cols(&model, 8);
        let label = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .find(|label| label.starts_with("kittwm-terminal-status-text:"))
            .unwrap();
        assert!(label.contains("focused-window-with-a-p…"), "{label}");
        assert!(label.contains("layout-name-that-is-far…"), "{label}");
        assert!(label.len() < 150, "{label}");
    }

    #[test]
    fn terminal_events_heading_label_builds_directly() {
        let model =
            terminal_events_model(250, vec!["status".to_string(), "pane_opened".to_string()]);
        assert_eq!(
            terminal_events_heading_label(&model),
            "kittwm-terminal-events-heading:count=2 ms=250"
        );
    }

    #[test]
    fn terminal_events_kinds_label_builds_directly() {
        assert_eq!(
            terminal_events_kinds_label("status,pane_opened"),
            "kittwm-terminal-events-kinds:status,pane_opened"
        );
    }

    #[test]
    fn terminal_status_kitty_uses_absolute_no_placeholder_options() {
        let options = terminal_scene_placement_options();
        assert!(!options.unicode_placeholder);
        assert_eq!(options.z_index, 20);
    }

    #[test]
    fn events_model_scene_contains_bounded_event_summary() {
        let model = terminal_events_model(
            250,
            vec![
                "status".to_string(),
                "pane_opened".to_string(),
                "pane_frame_presented".to_string(),
            ],
        );
        assert_eq!(
            render_events_text(&model),
            "events count=3 ms=250\nstatus\npane_opened\npane_frame_presented\n"
        );
        let scene = terminal_events_scene(&model);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels.contains(&"kittwm-terminal-events-backdrop"),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("count=3 ms=250")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("status,pane_opened,pane_frame_presented")),
            "{labels:?}"
        );
    }

    #[test]
    fn terminal_events_scene_bounds_event_label_payloads() {
        let model = terminal_events_model(
            250,
            vec![
                "status".to_string(),
                "pane_frame_presented_with_a_pathologically_long_kind_name".to_string(),
                "layout".to_string(),
                "input".to_string(),
                "another_pathologically_long_event_kind_name".to_string(),
                "not-included".to_string(),
            ],
        );
        let scene = terminal_events_scene_for_cols(&model, 8);
        let label = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .find(|label| label.starts_with("kittwm-terminal-events-kinds:"))
            .unwrap();
        let summary = terminal_events_summary_label(&model.kinds);
        assert!(summary.capacity() >= 5 * 25);
        let direct = terminal_events_kinds_label(&summary);
        assert_eq!(direct, label);
        assert_eq!(direct.capacity(), direct.len());
        assert!(label.contains("pane_frame_presented_wi…"), "{label}");
        assert!(label.contains("another_pathologically_…"), "{label}");
        assert!(!label.contains("not-included"), "{label}");
        assert!(label.len() < 150, "{label}");
    }

    #[test]
    fn help_is_success_path() {
        let err = TerminalArgs::parse_from(["--help"]).unwrap_err();
        assert!(err.starts_with("kittwm-terminal"));
        assert!(err.contains("Options:"), "{err}");
        assert!(err.contains("--replace"), "{err}");
        assert!(err.contains("--new-window"), "{err}");
        assert!(err.contains("--title TITLE"), "{err}");
        assert!(err.contains("--command CMD, -c CMD"), "{err}");
        assert!(err.contains("--events-scene-json MS"), "{err}");
        assert!(err.contains("--help, -h"), "{err}");
    }

    #[test]
    fn help_text_lists_copyable_examples() {
        let help = help_text();
        assert!(help.contains("Examples:"), "{help}");
        assert!(help.contains("kittwm-terminal\n"), "{help}");
        assert!(
            help.contains("kittwm-terminal --title logs -- tail -f /tmp/app.log"),
            "{help}"
        );
        assert!(
            help.contains(
                "kittwm-terminal --remote buildbox          # pane title defaults to buildbox"
            ),
            "{help}"
        );
        assert!(
            help.contains(
                "kittwm-terminal --remote buildbox -- htop  # pane title defaults to buildbox: htop"
            ),
            "{help}"
        );
        assert!(
            help.contains("kittwm-terminal --replace --command 'zsh -l'"),
            "{help}"
        );
        assert!(help.contains("kittwm-terminal --status"), "{help}");
        assert!(help.contains("kittwm-terminal --events-ms 1000"), "{help}");
    }
}
