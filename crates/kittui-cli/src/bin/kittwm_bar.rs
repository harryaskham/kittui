//! `kittwm-bar` — a tiny first-party SDK status/top-bar renderer.
//!
//! This is intentionally small: it can be used as a standalone proof of the
//! default kittwm top bar model without requiring the live session to spawn it
//! yet. When a kittwm socket is available it reads typed SDK status; otherwise
//! it falls back to environment/default values so it remains useful in tests and
//! shell prompts.

use std::process::ExitCode;
use std::time::SystemTime;

use kittui_cli::top_bar::{workspace_label, BarModel};
use kittwm_sdk::{ChromeReservationStatus, Kittwm};
use serde::Serialize;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OutputMode {
    Text,
    Json,
    SceneJson,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BarChromeModel {
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_bar_rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tilable_rows: Option<u16>,
}

impl From<&ChromeReservationStatus> for BarChromeModel {
    fn from(status: &ChromeReservationStatus) -> Self {
        Self {
            workspace: status.workspace.clone(),
            top_bar_rows: status.top_bar_rows,
            tilable_rows: status.tilable_rows,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BarOutputModel {
    #[serde(flatten)]
    bar: BarModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    chrome: Option<BarChromeModel>,
}

impl BarOutputModel {
    fn offline(now: SystemTime) -> Self {
        Self {
            bar: BarModel::offline(now),
            chrome: None,
        }
    }

    fn render(&self) -> String {
        self.bar.render()
    }

    fn scene(&self, cols: u16) -> kittui::Scene {
        self.bar.scene(cols)
    }
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

fn load_bar_model(now: SystemTime) -> BarOutputModel {
    let Ok(client) = Kittwm::connect_from_env() else {
        return BarOutputModel::offline(now);
    };
    match client.status() {
        Ok(status) => {
            let panes = status.panes.unwrap_or(status.panes_detail.len() as u64);
            let workspace = status
                .workspace_id()
                .map(str::to_string)
                .unwrap_or_else(workspace_label);
            let chrome = status.chrome_reservation().map(BarChromeModel::from);
            BarOutputModel {
                bar: BarModel::new(
                    workspace,
                    panes,
                    status.focus.unwrap_or_else(|| "-".to_string()),
                    true,
                    now,
                ),
                chrome,
            }
        }
        Err(_) => BarOutputModel::offline(now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn output_mode_rejects_multiple_formats() {
        let err =
            parse_output_mode(["--json".to_string(), "--scene-json".to_string()]).unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
    }

    #[test]
    fn bar_model_json_shape_is_stable() {
        let model = BarOutputModel::offline(UNIX_EPOCH);
        let json = serde_json::to_value(&model).unwrap();
        assert_eq!(json["workspace"], "1");
        assert_eq!(json["panes"], 0);
        assert_eq!(json["state"], "empty");
        assert_eq!(json["connected"], false);
        assert!(json.get("chrome").is_none());
    }

    #[test]
    fn scene_json_shape_is_stable() {
        let model = BarOutputModel::offline(UNIX_EPOCH);
        let json = serde_json::to_value(model.scene(12)).unwrap();
        assert_eq!(json["footprint"]["cols"], 12);
        assert_eq!(json["footprint"]["rows"], 1);
        assert_eq!(json["layers"][0]["label"], "kittwm-bar:empty:1");
    }

    #[test]
    fn bar_output_json_includes_chrome_when_available() {
        let model = BarOutputModel {
            bar: BarModel::new("dev", 2, "native-2", true, UNIX_EPOCH),
            chrome: Some(BarChromeModel {
                workspace: Some("dev".to_string()),
                top_bar_rows: Some(1),
                tilable_rows: Some(23),
            }),
        };
        let json = serde_json::to_value(&model).unwrap();
        assert_eq!(json["workspace"], "dev");
        assert_eq!(json["chrome"]["workspace"], "dev");
        assert_eq!(json["chrome"]["top_bar_rows"], 1);
        assert_eq!(json["chrome"]["tilable_rows"], 23);
        assert!(model.render().contains("panes:2"));
    }
}
