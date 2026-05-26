use std::env;
use std::process::ExitCode;

use kittui::{
    CellRect, CellSize, Corners, Layer, Node, Paint, PxRect, Rgba, Runtime, Scene, Stroke,
    TerminalInfo,
};
use kittwm_sdk::{Kittwm, PanesStatus, Status, SurfaceSpec, WindowSpec};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusMode {
    None,
    Text,
    SceneJson,
    Kitty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalArgs {
    replace: bool,
    title: Option<String>,
    command: String,
    status: StatusMode,
    events_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct TerminalStatusModel {
    panes: u64,
    focus: String,
    layout: String,
    details: usize,
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
        let mut status = StatusMode::None;
        let mut events_ms = None;
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
                    let value = iter
                        .next()
                        .ok_or_else(|| "--events-ms requires milliseconds".to_string())?;
                    events_ms = Some(
                        value
                            .parse()
                            .map_err(|_| "--events-ms expects an integer".to_string())?,
                    );
                }
                "--title" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--title requires a value".to_string())?;
                    title = Some(value);
                }
                "--command" | "-c" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--command requires a value".to_string())?;
                    command = Some(value);
                }
                "--" => {
                    let rest = iter.collect::<Vec<_>>();
                    if !rest.is_empty() {
                        command = Some(shell_words(&rest));
                    }
                    break;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other}\n\n{}", help_text()));
                }
                other => {
                    let mut rest = vec![other.to_string()];
                    rest.extend(iter);
                    command = Some(shell_words(&rest));
                    break;
                }
            }
        }
        Ok(Self {
            replace,
            title,
            command: command.unwrap_or_else(default_terminal_command),
            status,
            events_ms,
        })
    }
}

fn default_terminal_command() -> String {
    env::var("KITTWM_TERMINAL_CMD")
        .or_else(|_| env::var("SHELL").map(|shell| format!("{shell} -l")))
        .unwrap_or_else(|_| "/bin/sh -l".to_string())
}

fn shell_words(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':'))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn help_text() -> String {
    "kittwm-terminal — first-party terminal client for kittwm\n\n\
Usage:\n  kittwm-terminal [--replace|--new-window] [--title TITLE] [--command CMD]\n  kittwm-terminal [--replace|--new-window] [--title TITLE] -- PROGRAM [ARGS...]\n  kittwm-terminal --status\n  kittwm-terminal --status-scene-json\n  kittwm-terminal --status-kitty\n  kittwm-terminal --events-ms MS\n\n\
Connects through KITTWM_SOCKET/KITTWM_DISPLAY using kittwm-sdk and asks the\n\
running kittwm instance to spawn or replace a native terminal surface.\n\
--status prints typed SDK status/pane detail; --status-scene-json and\n\
--status-kitty render the same model as a kittui/kitty-native status card;\n\
--events-ms prints a bounded event batch for lifecycle/debugging.\n"
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
    format!(
        "status panes={} focus={} layout={} details={}\n",
        model.panes, model.focus, model.layout, model.details
    )
}

fn terminal_status_scene(model: &TerminalStatusModel) -> Scene {
    let cols = terminal_status_scene_cols();
    let rows = 5;
    let cell = CellSize::default();
    let width = cols as f32 * cell.width_px as f32;
    let height = rows as f32 * cell.height_px as f32;
    let focus = model.focus.chars().take(24).collect::<String>();
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
                label: Some(format!(
                    "kittwm-terminal-status-text:panes={} focus={} layout={} details={}",
                    model.panes, focus, model.layout, model.details
                )),
                root: Node::Rect {
                    rect: PxRect::new(
                        10.0,
                        cell.height_px as f32 * 2.2,
                        (width - 20.0).max(1.0),
                        2.0,
                    ),
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

fn terminal_status_scene_cols() -> u16 {
    env::var("KITTWM_TERMINAL_STATUS_COLS")
        .or_else(|_| env::var("COLUMNS"))
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|cols| *cols > 0)
        .unwrap_or(56)
        .clamp(24, 120)
}

fn render_status_kitty(model: &TerminalStatusModel) -> Result<String, String> {
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()
        .map_err(|err| err.to_string())?;
    let scene = terminal_status_scene(model);
    let mut options = kittui_kitty::PlacementOptions::unicode();
    options.z_index = 20;
    runtime
        .place_at_with_options(&scene, scene.footprint, &options)
        .map(|placement| placement.to_bytes())
        .map_err(|err| err.to_string())
}

fn run(args: TerminalArgs) -> Result<String, String> {
    let wm = Kittwm::connect_from_env().map_err(|err| format!("connect to kittwm: {err}"))?;
    if args.status != StatusMode::None {
        let status = wm.status().map_err(|err| format!("read status: {err}"))?;
        let panes = wm.panes().map_err(|err| format!("read panes: {err}"))?;
        let model = terminal_status_model(status, panes);
        return match args.status {
            StatusMode::Text => Ok(render_status_text(&model)),
            StatusMode::SceneJson => serde_json::to_string(&terminal_status_scene(&model))
                .map(|json| format!("{json}\n"))
                .map_err(|err| format!("encode status scene: {err}")),
            StatusMode::Kitty => render_status_kitty(&model),
            StatusMode::None => unreachable!(),
        };
    }
    if let Some(ms) = args.events_ms {
        let events = wm
            .events_ms(ms)
            .map_err(|err| format!("read events: {err}"))?;
        let mut out = format!("events count={} ms={}\n", events.len(), ms.clamp(1, 60_000));
        for event in events {
            out.push_str(event.kind());
            out.push('\n');
        }
        return Ok(out);
    }
    if args.replace {
        wm.replace_current(&WindowSpec {
            title: args.title,
            command: args.command,
        })
        .map_err(|err| format!("replace current terminal: {err}"))
    } else {
        let mut spec = SurfaceSpec::terminal(args.command);
        if let Some(title) = args.title {
            spec = spec.titled(title);
        }
        wm.spawn_surface(&spec)
            .map(|spawn| spawn.reply)
            .map_err(|err| format!("spawn terminal surface: {err}"))
    }
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
                status: StatusMode::None,
                events_ms: None,
            }
        );
    }

    #[test]
    fn parses_program_after_separator() {
        let args = TerminalArgs::parse_from(["--", "echo", "hello world"]).unwrap();
        assert_eq!(args.command, "echo 'hello world'");
    }

    #[test]
    fn parses_status_and_events_modes() {
        let status = TerminalArgs::parse_from(["--status"]).unwrap();
        assert_eq!(status.status, StatusMode::Text);
        assert_eq!(status.events_ms, None);
        let scene = TerminalArgs::parse_from(["--status-scene-json"]).unwrap();
        assert_eq!(scene.status, StatusMode::SceneJson);
        let kitty = TerminalArgs::parse_from(["--status-kitty"]).unwrap();
        assert_eq!(kitty.status, StatusMode::Kitty);
        let events = TerminalArgs::parse_from(["--events-ms", "250"]).unwrap();
        assert_eq!(events.status, StatusMode::None);
        assert_eq!(events.events_ms, Some(250));
        let err = TerminalArgs::parse_from(["--status", "--status-kitty"]).unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
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
                    focused: false,
                    weight: 1,
                    pid: None,
                    command: None,
                    x: None,
                    y: None,
                    cols: None,
                    rows: None,
                    app_x: None,
                    app_y: None,
                    app_cols: None,
                    app_rows: None,
                    cursor_col: None,
                    cursor_row: None,
                    cursor_visible: None,
                    bracketed_paste: None,
                    application_cursor_keys: None,
                    mouse_reporting: None,
                    mouse_button_motion: None,
                    mouse_all_motion: None,
                    mouse_sgr: None,
                    dirty_frame: None,
                    transport: None,
                }],
            },
        );
        assert_eq!(
            render_status_text(&model),
            "status panes=2 focus=native-2 layout=rows details=1\n"
        );
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
    fn help_is_success_path() {
        let err = TerminalArgs::parse_from(["--help"]).unwrap_err();
        assert!(err.starts_with("kittwm-terminal"));
    }
}
