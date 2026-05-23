//! `kittwm-bar` — a tiny first-party SDK status/top-bar renderer.
//!
//! This is intentionally small: it can be used as a standalone proof of the
//! default kittwm top bar model without requiring the live session to spawn it
//! yet. When a kittwm socket is available it reads typed SDK status; otherwise
//! it falls back to environment/default values so it remains useful in tests and
//! shell prompts.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use kittui::{Rgba, Scene};
use kittui_affordances::title_chrome;
use kittwm_sdk::Kittwm;
use ratatui::layout::Rect;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BarModel {
    workspace: String,
    panes: u64,
    state: String,
    focus: String,
    time: String,
    connected: bool,
}

impl BarModel {
    fn offline(now: SystemTime) -> Self {
        Self {
            workspace: workspace_label(),
            panes: 0,
            state: "empty".to_string(),
            focus: "-".to_string(),
            time: time_label(now),
            connected: false,
        }
    }

    fn render(&self) -> String {
        format!(
            " kittui-bar  ws:{}  {}  panes:{}  focus:{}  {} ",
            self.workspace, self.state, self.panes, self.focus, self.time
        )
    }

    fn scene(&self, cols: u16) -> Scene {
        let (left, right) = match (self.connected, self.state.as_str()) {
            (true, "active") => (Rgba::rgb(0x18, 0x4e, 0x77), Rgba::rgb(0x52, 0xb6, 0x9a)),
            (true, _) => (Rgba::rgb(0x24, 0x24, 0x36), Rgba::rgb(0x5a, 0x4f, 0x7c)),
            (false, _) => (Rgba::rgb(0x20, 0x20, 0x24), Rgba::rgb(0x3a, 0x3a, 0x44)),
        };
        let mut scene = title_chrome(left, right)
            .to_scene(Rect::new(0, 0, cols.max(1), 1))
            .expect("title chrome produces a one-line scene");
        for layer in &mut scene.layers {
            if layer.label.as_deref() == Some("background") {
                layer.label = Some(format!("kittwm-bar:{}:{}", self.state, self.workspace));
            }
        }
        scene
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OutputMode {
    Text,
    Json,
    SceneJson,
}

fn main() -> ExitCode {
    let mode = match parse_output_mode(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(err) => {
            eprintln!("kittwm-bar: {err}");
            eprintln!("usage: kittwm-bar [--json|--scene-json]");
            return ExitCode::from(2);
        }
    };
    let model = load_bar_model(SystemTime::now());
    match mode {
        OutputMode::Text => println!("{}", model.render()),
        OutputMode::Json => match serde_json::to_string(&model) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                eprintln!("kittwm-bar: json encode failed: {err}");
                return ExitCode::from(1);
            }
        },
        OutputMode::SceneJson => match serde_json::to_string(&model.scene(scene_cols())) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                eprintln!("kittwm-bar: scene json encode failed: {err}");
                return ExitCode::from(1);
            }
        },
    }
    ExitCode::SUCCESS
}

fn parse_output_mode(args: impl IntoIterator<Item = String>) -> Result<OutputMode, String> {
    let mut mode = OutputMode::Text;
    for arg in args {
        match arg.as_str() {
            "--json" if mode == OutputMode::Text => mode = OutputMode::Json,
            "--scene-json" if mode == OutputMode::Text => mode = OutputMode::SceneJson,
            "--json" | "--scene-json" => {
                return Err("choose only one of --json or --scene-json".to_string())
            }
            "--help" | "-h" => return Err("usage requested".to_string()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(mode)
}

fn scene_cols() -> u16 {
    std::env::var("KITTWM_BAR_COLS")
        .or_else(|_| std::env::var("COLUMNS"))
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|cols| *cols > 0)
        .unwrap_or(80)
}

fn load_bar_model(now: SystemTime) -> BarModel {
    let Ok(client) = Kittwm::connect_from_env() else {
        return BarModel::offline(now);
    };
    match client.status() {
        Ok(status) => {
            let panes = status.panes.unwrap_or(status.panes_detail.len() as u64);
            BarModel {
                workspace: workspace_label(),
                panes,
                state: if panes == 0 { "empty" } else { "active" }.to_string(),
                focus: status.focus.unwrap_or_else(|| "-".to_string()),
                time: time_label(now),
                connected: true,
            }
        }
        Err(_) => BarModel::offline(now),
    }
}

fn workspace_label() -> String {
    std::env::var("KITTWM_WORKSPACE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "1".to_string())
}

fn time_label(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let day = secs % 86_400;
    let hour = day / 3_600;
    let minute = (day % 3_600) / 60;
    format!("{hour:02}:{minute:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_bar_model_renders_empty_workspace() {
        let model =
            BarModel::offline(UNIX_EPOCH + std::time::Duration::from_secs(12 * 3_600 + 34 * 60));
        assert_eq!(model.workspace, "1");
        assert_eq!(model.panes, 0);
        assert_eq!(model.state, "empty");
        assert!(!model.connected);
        let rendered = model.render();
        assert!(rendered.contains("kittui-bar"), "{rendered}");
        assert!(rendered.contains("ws:1"), "{rendered}");
        assert!(rendered.contains("empty"), "{rendered}");
        assert!(rendered.contains("12:34 UTC"), "{rendered}");
    }

    #[test]
    fn time_label_uses_utc_clock_minutes() {
        assert_eq!(time_label(UNIX_EPOCH), "00:00 UTC");
        assert_eq!(
            time_label(UNIX_EPOCH + std::time::Duration::from_secs(23 * 3_600 + 59 * 60)),
            "23:59 UTC"
        );
    }

    #[test]
    fn bar_model_json_shape_is_stable() {
        let model = BarModel::offline(UNIX_EPOCH);
        let json = serde_json::to_value(&model).unwrap();
        assert_eq!(json["workspace"], "1");
        assert_eq!(json["panes"], 0);
        assert_eq!(json["state"], "empty");
        assert_eq!(json["connected"], false);
    }

    #[test]
    fn bar_model_scene_is_one_line_artifact() {
        let model = BarModel::offline(UNIX_EPOCH);
        let scene = model.scene(42);
        assert_eq!(scene.footprint.cols, 42);
        assert_eq!(scene.footprint.rows, 1);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("kittwm-bar:empty:1")));
    }

    #[test]
    fn scene_json_shape_is_stable() {
        let model = BarModel::offline(UNIX_EPOCH);
        let json = serde_json::to_value(model.scene(12)).unwrap();
        assert_eq!(json["footprint"]["cols"], 12);
        assert_eq!(json["footprint"]["rows"], 1);
        assert_eq!(json["layers"][0]["label"], "kittwm-bar:empty:1");
    }

    #[test]
    fn output_mode_rejects_multiple_formats() {
        let err = parse_output_mode(["--json".to_string(), "--scene-json".to_string()])
            .unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
    }
}
