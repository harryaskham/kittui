//! `kittwm-bar` — a tiny first-party SDK status/top-bar renderer.
//!
//! This is intentionally small: it can be used as a standalone proof of the
//! default kittwm top bar model without requiring the live session to spawn it
//! yet. When a kittwm socket is available it reads typed SDK status; otherwise
//! it falls back to environment/default values so it remains useful in tests and
//! shell prompts.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use kittwm_sdk::Kittwm;
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
}

fn main() -> ExitCode {
    let json = std::env::args().skip(1).any(|arg| arg == "--json");
    let model = load_bar_model(SystemTime::now());
    if json {
        match serde_json::to_string(&model) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                eprintln!("kittwm-bar: json encode failed: {err}");
                return ExitCode::from(1);
            }
        }
    } else {
        println!("{}", model.render());
    }
    ExitCode::SUCCESS
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
}
