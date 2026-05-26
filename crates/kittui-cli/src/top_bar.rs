//! Shared kittwm top-bar model and scene helpers.
//!
//! This module intentionally lives in `kittui-cli`: it is higher-level WM/app
//! chrome, not a kittui-core primitive.

use std::time::{SystemTime, UNIX_EPOCH};

use kittui::{Layer, Node, Rgba, Scene};
use kittui_affordances::{title_chrome, InlineChipColors, InlineStyle, InlineTheme};
use ratatui::layout::Rect;
use serde::Serialize;

/// Small, serializable status model for kittwm's top bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BarModel {
    /// Workspace label/id, currently `1` by default.
    pub workspace: String,
    /// Pane count in the workspace.
    pub panes: u64,
    /// Human-readable state, currently `empty` or `active`.
    pub state: String,
    /// Focused pane/window id, or `-`.
    pub focus: String,
    /// Display time label.
    pub time: String,
    /// Whether this model came from a live SDK connection.
    pub connected: bool,
}

impl BarModel {
    /// Construct a model from explicit parts.
    pub fn new(
        workspace: impl Into<String>,
        panes: u64,
        focus: impl Into<String>,
        connected: bool,
        now: SystemTime,
    ) -> Self {
        let state = if panes == 0 { "empty" } else { "active" };
        Self {
            workspace: workspace.into(),
            panes,
            state: state.to_string(),
            focus: focus.into(),
            time: time_label(now),
            connected,
        }
    }

    /// Offline/default model for an empty workspace.
    pub fn offline(now: SystemTime) -> Self {
        Self::new(workspace_label(), 0, "-", false, now)
    }

    /// Model used by the in-process live native session.
    pub fn live(
        workspace: impl Into<String>,
        panes: usize,
        focus: impl Into<String>,
        now: SystemTime,
    ) -> Self {
        Self::new(workspace, panes as u64, focus, true, now)
    }

    /// Render a minimal i3bar-style one-line text bar.
    pub fn render(&self) -> String {
        self.render_i3bar(0)
    }

    /// Render a minimal i3bar-style bar padded to a target width when provided.
    pub fn render_i3bar(&self, cols: usize) -> String {
        let left = self.workspace_chips_text();
        let clock = self.time.strip_suffix(" UTC").unwrap_or(&self.time);
        let right = format!(" {clock} ");
        if cols == 0 {
            return format!("{left}{right}");
        }
        let left_width = left.chars().count();
        let right_width = right.chars().count();
        if left_width + right_width >= cols {
            return format!("{left}{right}").chars().take(cols).collect();
        }
        format!(
            "{left}{}{right}",
            " ".repeat(cols - left_width - right_width)
        )
    }

    fn workspace_chips_text(&self) -> String {
        let workspace = self.workspace.trim();
        (1..=3)
            .map(|idx| {
                let label = idx.to_string();
                if label == workspace {
                    format!("| {label} ")
                } else {
                    format!("| {label} ")
                }
            })
            .collect::<String>()
            + "|"
    }

    /// Render the bar as a one-line kittui scene.
    pub fn scene(&self, cols: u16) -> Scene {
        self.scene_with_prefix(cols, "kittwm-bar")
    }

    /// Render the bar as a one-line scene with caller-chosen diagnostic labels.
    pub fn scene_with_prefix(&self, cols: u16, label_prefix: &str) -> Scene {
        let (left, right) = top_bar_theme_colors(self.connected, self.state.as_str());
        let mut scene = title_chrome(left, right)
            .to_scene(Rect::new(0, 0, cols.max(1), 1))
            .expect("title chrome produces a one-line scene");
        for layer in &mut scene.layers {
            if layer.label.as_deref() == Some("background") {
                layer.label = Some(format!("{label_prefix}:{}:{}", self.state, self.workspace));
            }
        }
        for idx in 1..=3 {
            scene.layers.push(Layer::new(
                format!(
                    "{label_prefix}-workspace-chip:{idx}:{}:action=workspace.switch.{idx}",
                    if self.workspace == idx.to_string() {
                        "active"
                    } else {
                        "inactive"
                    }
                ),
                Node::Group {
                    opacity: 1.0,
                    children: Vec::new(),
                },
            ));
        }
        scene.layers.push(Layer::new(
            format!(
                "{label_prefix}-text:{}",
                self.render_i3bar(cols as usize).trim()
            ),
            Node::Group {
                opacity: 1.0,
                children: Vec::new(),
            },
        ));
        scene
    }
}

fn top_bar_theme_colors(connected: bool, state: &str) -> (Rgba, Rgba) {
    let style = match (connected, state) {
        (true, "active") => InlineStyle::Neon,
        (true, _) => InlineStyle::Glass,
        (false, _) => InlineStyle::Metal,
    };
    let colors = InlineChipColors::resolve(InlineTheme::Nord, style);
    (colors.fill, colors.border)
}

/// Workspace label from environment, defaulting to `1`.
pub fn workspace_label() -> String {
    std::env::var("KITTWM_WORKSPACE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "1".to_string())
}

/// UTC HH:MM label from a system time.
pub fn time_label(now: SystemTime) -> String {
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
        let model = BarModel::new(
            "1",
            0,
            "-",
            false,
            UNIX_EPOCH + std::time::Duration::from_secs(12 * 3_600 + 34 * 60),
        );
        assert_eq!(model.workspace, "1");
        assert_eq!(model.panes, 0);
        assert_eq!(model.state, "empty");
        assert!(!model.connected);
        let rendered = model.render();
        assert!(rendered.contains("| 1 | 2 | 3 |"), "{rendered}");
        assert!(rendered.contains("12:34"), "{rendered}");
        assert!(!rendered.contains("kittui-bar"), "{rendered}");
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
    fn top_bar_theme_colors_use_shared_inline_tokens() {
        let (active_fill, active_border) = top_bar_theme_colors(true, "active");
        let active = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Neon);
        assert_eq!((active_fill, active_border), (active.fill, active.border));

        let (offline_fill, offline_border) = top_bar_theme_colors(false, "empty");
        let offline = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Metal);
        assert_eq!(
            (offline_fill, offline_border),
            (offline.fill, offline.border)
        );
    }

    #[test]
    fn scene_shape_carries_state_and_text_labels() {
        let model = BarModel::new("1", 0, "-", false, UNIX_EPOCH);
        let scene = model.scene(42);
        assert_eq!(scene.footprint.cols, 42);
        assert_eq!(scene.footprint.rows, 1);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("kittwm-bar:empty:1")));
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("| 1 | 2 | 3 |")));
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip:1:active")));
    }
}
