//! `kittwm-bar` — a tiny first-party SDK status/top-bar renderer.
//!
//! This is intentionally small: it can be used as a standalone proof of the
//! default kittwm top bar model without requiring the live session to spawn it
//! yet. When a kittwm socket is available it reads typed SDK status; otherwise
//! it falls back to environment/default values so it remains useful in tests and
//! shell prompts.

use std::process::ExitCode;
use std::time::SystemTime;

use kittui::{CellRect, Runtime, TerminalInfo};
use kittui_cli::top_bar::{workspace_label, BarModel};
use kittwm_sdk::{ChromeReservationRequest, ChromeReservationStatus, Kittwm};
use serde::Serialize;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum OutputMode {
    Text,
    Json,
    SceneJson,
    Kitty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BarOptions {
    mode: OutputMode,
    reserve: bool,
    release: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BarChromeModel {
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_bar_rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bottom_bar_rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    left_cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    right_cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tilable_rows: Option<u16>,
}

impl From<&ChromeReservationStatus> for BarChromeModel {
    fn from(status: &ChromeReservationStatus) -> Self {
        Self {
            workspace: status.workspace.clone(),
            top_bar_rows: status.top_bar_rows,
            bottom_bar_rows: status.bottom_bar_rows,
            left_cols: status.left_cols,
            right_cols: status.right_cols,
            gap_cols: status.gap_cols,
            gap_rows: status.gap_rows,
            owner: status.owner.clone(),
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
    let opts = match parse_options(std::env::args().skip(1)) {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("kittwm-bar: {err}");
            eprintln!("usage: kittwm-bar [--json|--scene-json|--kitty] [--reserve|--release]");
            return ExitCode::from(2);
        }
    };
    if let Err(err) = apply_reservation_options(&opts) {
        eprintln!("kittwm-bar: {err}");
        return ExitCode::from(1);
    }
    let model = load_bar_model(SystemTime::now());
    match opts.mode {
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
        OutputMode::Kitty => match render_kitty_bar(&model) {
            Ok(bytes) => print!("{bytes}"),
            Err(err) => {
                eprintln!("kittwm-bar: kitty render failed: {err}");
                return ExitCode::from(1);
            }
        },
    }
    ExitCode::SUCCESS
}

fn parse_options(args: impl IntoIterator<Item = String>) -> Result<BarOptions, String> {
    let mut mode = OutputMode::Text;
    let mut reserve = false;
    let mut release = false;
    for arg in args {
        match arg.as_str() {
            "--json" if mode == OutputMode::Text => mode = OutputMode::Json,
            "--scene-json" if mode == OutputMode::Text => mode = OutputMode::SceneJson,
            "--kitty" | "--graphics" if mode == OutputMode::Text => mode = OutputMode::Kitty,
            "--json" | "--scene-json" | "--kitty" | "--graphics" => {
                return Err("choose only one of --json, --scene-json, or --kitty".to_string())
            }
            "--reserve" => reserve = true,
            "--release" | "--clear-reservation" => release = true,
            "--help" | "-h" => return Err("usage requested".to_string()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if reserve && release {
        return Err("choose only one of --reserve or --release".to_string());
    }
    Ok(BarOptions {
        mode,
        reserve,
        release,
    })
}

fn apply_reservation_options(opts: &BarOptions) -> Result<(), String> {
    if !opts.reserve && !opts.release {
        return Ok(());
    }
    let client = Kittwm::connect_from_env().map_err(|err| format!("connect to kittwm: {err}"))?;
    if opts.release {
        client
            .clear_chrome_reservation()
            .map_err(|err| format!("clear chrome reservation: {err}"))?;
    } else {
        let owner = std::env::var("KITTWM_WINDOW")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "kittwm-bar".to_string());
        let request = ChromeReservationRequest::top_bar(1).owner(owner);
        client
            .reserve_chrome(&request)
            .map_err(|err| format!("reserve chrome: {err}"))?;
    }
    Ok(())
}

fn render_kitty_bar(model: &BarOutputModel) -> Result<String, String> {
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()
        .map_err(|err| err.to_string())?;
    let scene = model.scene(scene_cols());
    let mut options = kittui_kitty::PlacementOptions::unicode();
    options.z_index = 20;
    runtime
        .place_at_with_options(
            &scene,
            CellRect::new(0, 0, scene.footprint.cols, scene.footprint.rows),
            &options,
        )
        .map(|placement| placement.to_bytes())
        .map_err(|err| err.to_string())
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
        let err = parse_options(["--json".to_string(), "--scene-json".to_string()]).unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
    }

    #[test]
    fn reservation_flags_are_mutually_exclusive() {
        let err = parse_options(["--reserve".to_string(), "--release".to_string()]).unwrap_err();
        assert!(err.contains("choose only one"), "{err}");
        let opts = parse_options(["--kitty".to_string(), "--reserve".to_string()]).unwrap();
        assert_eq!(opts.mode, OutputMode::Kitty);
        assert!(opts.reserve);
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
                bottom_bar_rows: Some(1),
                left_cols: Some(2),
                right_cols: Some(3),
                gap_cols: Some(1),
                gap_rows: Some(1),
                owner: Some("bar".to_string()),
                tilable_rows: Some(22),
            }),
        };
        let json = serde_json::to_value(&model).unwrap();
        assert_eq!(json["workspace"], "dev");
        assert_eq!(json["chrome"]["workspace"], "dev");
        assert_eq!(json["chrome"]["top_bar_rows"], 1);
        assert_eq!(json["chrome"]["bottom_bar_rows"], 1);
        assert_eq!(json["chrome"]["left_cols"], 2);
        assert_eq!(json["chrome"]["right_cols"], 3);
        assert_eq!(json["chrome"]["gap_cols"], 1);
        assert_eq!(json["chrome"]["gap_rows"], 1);
        assert_eq!(json["chrome"]["owner"], "bar");
        assert_eq!(json["chrome"]["tilable_rows"], 22);
        assert!(model.render().contains("panes:2"));
    }
}
