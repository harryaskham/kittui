//! Shared kittwm top-bar model and scene helpers.
//!
//! This module intentionally lives in `kittui-cli`: it is higher-level WM/app
//! chrome, not a kittui-core primitive.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use kittui::{Corners, Layer, Node, Paint, PxRect, Rgba, Scene, Stroke};
use kittui_affordances::{title_chrome, InlineChipColors, InlineStyle, InlineTheme};
use kittwm_sdk::KittwmConfig;
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
        let theme = top_bar_theme(self.connected, self.state.as_str());
        let mut scene = title_chrome(theme.fill, theme.border)
            .to_scene(Rect::new(0, 0, cols.max(1), 1))
            .expect("title chrome produces a one-line scene");
        for layer in &mut scene.layers {
            if layer.label.as_deref() == Some("background") {
                layer.label = Some(format!("{label_prefix}:{}:{}", self.state, self.workspace));
            }
        }
        let cell_w = scene.cell_size.width_px.max(1) as f32;
        let cell_h = scene.cell_size.height_px.max(1) as f32;
        let chip_w = 3.0 * cell_w;
        let chip_h = (cell_h - 4.0).max(6.0);
        for idx in 1..=3 {
            let active = self.workspace == idx.to_string();
            let x = 1.0 + (idx - 1) as f32 * (chip_w + 3.0);
            let y = ((cell_h - chip_h) / 2.0).max(0.0);
            scene.layers.push(Layer::new(
                format!("{label_prefix}-workspace-chip-shadow:{idx}"),
                Node::Rect {
                    rect: PxRect::new(x + 1.0, y + 1.0, chip_w, chip_h),
                    fill: Paint::Solid {
                        color: theme.shadow,
                    },
                    stroke: None,
                    corners: Corners::uniform(7.0),
                },
            ));
            scene.layers.push(Layer::new(
                format!(
                    "{label_prefix}-workspace-chip:{idx}:{}:action=workspace.switch.{idx}",
                    if active { "active" } else { "inactive" }
                ),
                Node::Rect {
                    rect: PxRect::new(x, y, chip_w, chip_h),
                    fill: Paint::Solid {
                        color: if active {
                            theme.chip_active
                        } else {
                            theme.chip_inactive
                        },
                    },
                    stroke: Some(Stroke::inside(
                        if active { 2.0 } else { 1.0 },
                        Paint::Solid {
                            color: if active { theme.clock_fg } else { theme.border },
                        },
                    )),
                    corners: Corners::uniform(7.0),
                },
            ));
        }
        let clock = self.time.strip_suffix(" UTC").unwrap_or(&self.time);
        let clock_cols = clock.chars().count().max(5) as f32 + 2.0;
        let clock_w = (clock_cols * cell_w).min(cols.max(1) as f32 * cell_w);
        let clock_x = (cols.max(1) as f32 * cell_w - clock_w - 1.0).max(0.0);
        scene.layers.push(Layer::new(
            format!("{label_prefix}-clock-chip:{clock}:high-contrast"),
            Node::Rect {
                rect: PxRect::new(clock_x + 1.0, 1.0, clock_w, chip_h),
                fill: Paint::Solid {
                    color: theme.shadow,
                },
                stroke: None,
                corners: Corners::uniform(7.0),
            },
        ));
        scene.layers.push(Layer::new(
            format!("{label_prefix}-clock-chip-foreground:{clock}"),
            Node::Rect {
                rect: PxRect::new(clock_x, ((cell_h - chip_h) / 2.0).max(0.0), clock_w, chip_h),
                fill: Paint::Solid {
                    color: theme.clock_bg,
                },
                stroke: Some(Stroke::inside(
                    1.0,
                    Paint::Solid {
                        color: theme.clock_fg,
                    },
                )),
                corners: Corners::uniform(7.0),
            },
        ));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TopBarTheme {
    fill: Rgba,
    border: Rgba,
    chip_active: Rgba,
    chip_inactive: Rgba,
    clock_bg: Rgba,
    clock_fg: Rgba,
    shadow: Rgba,
}

static TOP_BAR_CONFIG_THEME: OnceLock<Option<KittwmConfig>> = OnceLock::new();

fn top_bar_theme(connected: bool, state: &str) -> TopBarTheme {
    let style = match (connected, state) {
        (true, "active") => InlineStyle::Neon,
        (true, _) => InlineStyle::Glass,
        (false, _) => InlineStyle::Metal,
    };
    let colors = InlineChipColors::resolve(InlineTheme::Nord, style);
    let mut theme = TopBarTheme {
        fill: with_alpha(colors.fill, colors.fill.3.max(190)),
        border: colors.border,
        chip_active: with_alpha(colors.highlight, colors.highlight.3.max(230)),
        chip_inactive: with_alpha(colors.fill, colors.fill.3.max(185)),
        clock_bg: Rgba(0x2e, 0x34, 0x40, 235),
        clock_fg: Rgba(0xec, 0xef, 0xf4, 255),
        shadow: Rgba(0x00, 0x00, 0x00, top_bar_shadow_alpha()),
    };
    if let Some(config) = TOP_BAR_CONFIG_THEME
        .get_or_init(|| KittwmConfig::load_default().ok())
        .as_ref()
    {
        apply_kittwm_config_to_top_bar_theme(&mut theme, config);
    }
    theme
}

fn apply_kittwm_config_to_top_bar_theme(theme: &mut TopBarTheme, config: &KittwmConfig) {
    if let Some(bg) = parse_bar_color(&config.background.color, 210) {
        theme.fill = with_alpha(bg, config_alpha(config.background.opacity, 210));
        theme.chip_inactive = with_alpha(bg, config_alpha(config.background.opacity, 195));
        theme.clock_bg = with_alpha(bg, 240);
    }
    if let Some(fg) = parse_bar_color(&config.colorscheme.fg, 255) {
        theme.clock_fg = fg;
        theme.border = fg;
    }
    if let Some(accent) = config
        .colorscheme
        .ansi_color(4)
        .and_then(|value| parse_bar_color(value, 235))
    {
        theme.chip_active = accent;
    }
    theme.shadow = with_alpha(theme.shadow, top_bar_shadow_alpha());
}

fn config_alpha(opacity: f32, fallback: u8) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0)
        .round()
        .max(fallback as f32) as u8
}

fn top_bar_shadow_alpha() -> u8 {
    std::env::var("KITTWM_BAR_SHADOW_ALPHA")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(105)
}

fn parse_bar_color(value: &str, alpha: u8) -> Option<Rgba> {
    let rgba = match value.trim().to_ascii_lowercase().as_str() {
        "nord0" => Rgba(0x2e, 0x34, 0x40, alpha),
        "nord1" => Rgba(0x3b, 0x42, 0x52, alpha),
        "nord2" => Rgba(0x43, 0x4c, 0x5e, alpha),
        "nord3" => Rgba(0x4c, 0x56, 0x6a, alpha),
        "nord4" => Rgba(0xd8, 0xde, 0xe9, alpha),
        "nord5" => Rgba(0xe5, 0xe9, 0xf0, alpha),
        "nord6" => Rgba(0xec, 0xef, 0xf4, alpha),
        "nord7" => Rgba(0x8f, 0xbc, 0xbb, alpha),
        "nord8" => Rgba(0x88, 0xc0, 0xd0, alpha),
        "nord9" => Rgba(0x81, 0xa1, 0xc1, alpha),
        "nord10" => Rgba(0x5e, 0x81, 0xac, alpha),
        "nord11" => Rgba(0xbf, 0x61, 0x6a, alpha),
        "nord12" => Rgba(0xd0, 0x87, 0x70, alpha),
        "nord13" => Rgba(0xeb, 0xcb, 0x8b, alpha),
        "nord14" => Rgba(0xa3, 0xbe, 0x8c, alpha),
        "nord15" => Rgba(0xb4, 0x8e, 0xad, alpha),
        other => Rgba::parse(other)
            .ok()
            .map(|color| with_alpha(color, alpha))?,
    };
    Some(rgba)
}

#[cfg(test)]
fn top_bar_theme_colors(connected: bool, state: &str) -> (Rgba, Rgba) {
    let theme = top_bar_theme(connected, state);
    (theme.fill, theme.border)
}

fn with_alpha(color: Rgba, alpha: u8) -> Rgba {
    Rgba(color.0, color.1, color.2, alpha)
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
    fn top_bar_theme_colors_remain_visible_after_config_resolution() {
        let (active_fill, active_border) = top_bar_theme_colors(true, "active");
        assert!(active_fill.3 >= 190, "{active_fill:?}");
        assert_eq!(active_border.3, 255, "{active_border:?}");

        let (offline_fill, offline_border) = top_bar_theme_colors(false, "empty");
        assert!(offline_fill.3 >= 190, "{offline_fill:?}");
        assert_eq!(offline_border.3, 255, "{offline_border:?}");
    }

    #[test]
    fn top_bar_theme_applies_configured_colors_and_shadow_alpha() {
        let mut theme = top_bar_theme(false, "empty");
        let mut config = KittwmConfig::default();
        config.background.color = "#112233".to_string();
        config.background.opacity = 0.5;
        config.colorscheme.fg = "#ddeeff".to_string();
        config.colorscheme.colors[4] = "#445566".to_string();
        apply_kittwm_config_to_top_bar_theme(&mut theme, &config);
        assert_eq!(theme.fill, Rgba(0x11, 0x22, 0x33, 210));
        assert_eq!(theme.chip_inactive, Rgba(0x11, 0x22, 0x33, 195));
        assert_eq!(theme.border, Rgba(0xdd, 0xee, 0xff, 255));
        assert_eq!(theme.clock_fg, Rgba(0xdd, 0xee, 0xff, 255));
        assert_eq!(theme.chip_active, Rgba(0x44, 0x55, 0x66, 235));
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
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip-shadow:1")));
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("clock-chip-foreground:00:00")));
    }
}
