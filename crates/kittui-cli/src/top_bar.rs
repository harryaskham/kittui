//! Shared kittwm top-bar model and scene helpers.
//!
//! This module intentionally lives in `kittui-cli`: it is higher-level WM/app
//! chrome, not a kittui-core primitive.

use std::borrow::Cow;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use kittui::{Corners, Layer, Node, Paint, PxRect, Rgba, Scene, Stroke};
use kittui_affordances::{title_chrome, InlineChipColors, InlineStyle, InlineTheme};
use kittwm_sdk::KittwmConfig;
use ratatui::layout::Rect;
use serde::Serialize;

const TOP_BAR_LABEL_MAX_CHARS: usize = 64;

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
        let mut left = self.workspace_chips_text();
        let clock = self.time.strip_suffix(" UTC").unwrap_or(&self.time);
        let right = top_bar_clock_text(clock);
        if cols == 0 {
            let mut out = String::with_capacity(left.len().saturating_add(right.len()));
            out.push_str(&left);
            out.push_str(&right);
            return out;
        }
        let left_width = left.chars().count();
        let right_width = right.chars().count();
        if left_width + right_width > cols {
            left = self.workspace_chips_text_constrained();
            let mut out = String::with_capacity(cols);
            push_chars_until(&mut out, &left, cols);
            push_chars_until(&mut out, &right, cols);
            return out;
        }
        let mut out = String::with_capacity(cols);
        out.push_str(&left);
        out.extend(std::iter::repeat(' ').take(cols - left_width - right_width));
        out.push_str(&right);
        out
    }

    fn workspace_chips_text(&self) -> String {
        self.workspace_chips_text_from_labels(self.workspace_chip_labels())
    }

    fn workspace_chips_text_constrained(&self) -> String {
        self.workspace_chips_text_from_labels(self.workspace_chip_labels_active_first())
    }

    pub fn workspace_chip_labels_active_first(&self) -> Vec<String> {
        let workspace = self.workspace.trim();
        let mut labels = self.workspace_chip_labels();
        if !workspace.is_empty() {
            labels.sort_by_key(|label| usize::from(label != workspace));
        }
        labels
    }

    fn workspace_chips_text_from_labels(&self, labels: Vec<String>) -> String {
        let workspace = self.workspace.trim();
        let mut out = String::with_capacity(labels.len().saturating_mul(8).saturating_add(1));
        for label in labels {
            let display = top_bar_display_label(&label);
            if label == workspace {
                out.push_str("|[");
                out.push_str(&display);
                out.push(']');
            } else {
                out.push_str("| ");
                out.push_str(&display);
                out.push(' ');
            }
        }
        out.push('|');
        out
    }

    /// Workspace chips shown in the bar.
    ///
    /// The first-launch bar should represent only existing workspace state.
    /// Additional workspaces are created on demand by navigation/keymap actions,
    /// so the bar starts with one active chip instead of advertising inactive
    /// non-existent workspaces.
    pub fn workspace_chip_labels(&self) -> Vec<String> {
        let workspace = self.workspace.trim();
        vec![if workspace.is_empty() {
            "1".to_string()
        } else {
            workspace.to_string()
        }]
    }

    fn workspace_chip_labels_for_scene(&self, cols: u16) -> Vec<String> {
        let labels = self.workspace_chip_labels();
        let total_cols = workspace_chip_total_cols(&labels);
        if total_cols > cols {
            self.workspace_chip_labels_active_first()
        } else {
            labels
        }
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
        let workspace = self.workspace.trim();
        let display_workspace = top_bar_display_label(workspace);
        for layer in &mut scene.layers {
            if layer.label.as_deref() == Some("background") {
                layer.label = Some(top_bar_background_label(
                    label_prefix,
                    self.state.as_str(),
                    &display_workspace,
                ));
            }
        }
        let cell_w = scene.cell_size.width_px.max(1) as f32;
        let cell_h = scene.cell_size.height_px.max(1) as f32;
        let chip_h = (cell_h - 4.0).max(6.0);
        let scene_w = cols.max(1) as f32 * cell_w;
        let mut chip_x = 1.0;
        let mut last_chip_end_x = 0.0;
        for label in self.workspace_chip_labels_for_scene(cols) {
            let active = self.workspace.trim() == label;
            let display_label = top_bar_display_label(&label);
            let natural_chip_w = (display_label.chars().count() as f32 + 2.0).max(3.0) * cell_w;
            let Some(chip_w) = top_bar_bounded_chip_width(scene_w, chip_x, natural_chip_w, cell_w)
            else {
                break;
            };
            let x = chip_x;
            let y = ((cell_h - chip_h) / 2.0).max(0.0);
            scene.layers.push(Layer::new(
                format!("{label_prefix}-workspace-chip-shadow:{display_label}"),
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
                    "{label_prefix}-workspace-chip:{display_label}:{}",
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
            last_chip_end_x = x + chip_w;
            chip_x = last_chip_end_x + 3.0;
        }
        let clock = self.time.strip_suffix(" UTC").unwrap_or(&self.time);
        let clock_cols = clock.chars().count().max(5) as f32 + 2.0;
        let clock_w = (clock_cols * cell_w).min(cols.max(1) as f32 * cell_w);
        if let Some(clock_x) =
            top_bar_clock_chip_x(cols.max(1) as f32 * cell_w, last_chip_end_x, clock_w)
        {
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

fn top_bar_background_label(label_prefix: &str, state: &str, display_workspace: &str) -> String {
    let mut out = String::with_capacity(
        label_prefix
            .len()
            .saturating_add(state.len())
            .saturating_add(display_workspace.len())
            .saturating_add(2),
    );
    out.push_str(label_prefix);
    out.push(':');
    out.push_str(state);
    out.push(':');
    out.push_str(display_workspace);
    out
}

fn top_bar_clock_text(clock: &str) -> String {
    let mut out = String::with_capacity(clock.len().saturating_add(2));
    out.push(' ');
    out.push_str(clock);
    out.push(' ');
    out
}

fn push_chars_until(out: &mut String, text: &str, max_chars: usize) {
    let remaining = max_chars.saturating_sub(out.chars().count());
    out.extend(text.chars().take(remaining));
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

pub fn workspace_chip_total_cols(labels: &[String]) -> u16 {
    labels.iter().fold(0u16, |total, label| {
        let chip_cols = workspace_chip_label_cols(label).saturating_add(3);
        total.saturating_add(chip_cols)
    })
}

fn workspace_chip_label_cols(label: &str) -> u16 {
    label
        .chars()
        .take(TOP_BAR_LABEL_MAX_CHARS)
        .count()
        .min(u16::MAX as usize) as u16
}

fn top_bar_display_label(label: &str) -> Cow<'_, str> {
    let mut boundary = label.len();
    let mut chars = label.char_indices();
    for _ in 0..TOP_BAR_LABEL_MAX_CHARS {
        let Some((idx, _)) = chars.next() else {
            return Cow::Borrowed(label);
        };
        boundary = idx;
    }
    if chars.next().is_none() {
        return Cow::Borrowed(label);
    }
    let mut out = String::with_capacity(TOP_BAR_LABEL_MAX_CHARS);
    out.push_str(&label[..boundary]);
    out.push('…');
    Cow::Owned(out)
}

fn top_bar_clock_chip_x(total_width: f32, chip_end_x: f32, clock_width: f32) -> Option<f32> {
    let gap = 4.0;
    let right_aligned = (total_width - clock_width - 1.0).max(0.0);
    (chip_end_x + gap <= right_aligned).then_some(right_aligned)
}

fn top_bar_bounded_chip_width(
    total_width: f32,
    chip_x: f32,
    natural_width: f32,
    cell_width: f32,
) -> Option<f32> {
    let min_width = 3.0 * cell_width.max(1.0);
    let available = (total_width - chip_x - 1.0).max(0.0);
    (available >= min_width).then_some(natural_width.min(available))
}

/// Workspace label from environment, defaulting to `1`.
pub fn workspace_label() -> String {
    match std::env::var("KITTWM_WORKSPACE") {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                "1".to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => "1".to_string(),
    }
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
    let mut out = String::with_capacity("00:00 UTC".len());
    push_two_digit(&mut out, hour);
    out.push(':');
    push_two_digit(&mut out, minute);
    out.push_str(" UTC");
    out
}

fn push_two_digit(out: &mut String, value: u64) {
    out.push(char::from(b'0' + ((value / 10) % 10) as u8));
    out.push(char::from(b'0' + (value % 10) as u8));
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        assert!(rendered.contains("|[1]|"), "{rendered}");
        assert!(!rendered.contains("| 2 | 3 |"), "{rendered}");
        assert!(rendered.contains("12:34"), "{rendered}");
        assert!(!rendered.contains("kittui-bar"), "{rendered}");
    }

    #[test]
    fn workspace_label_trims_environment_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WORKSPACE", " dev ");
        assert_eq!(workspace_label(), "dev");
        std::env::set_var("KITTWM_WORKSPACE", "   ");
        assert_eq!(workspace_label(), "1");
        std::env::remove_var("KITTWM_WORKSPACE");
    }

    #[test]
    fn text_bar_marks_active_workspace() {
        let model = BarModel::new("2", 1, "native-1", true, UNIX_EPOCH);
        let rendered = model.render();
        assert!(rendered.contains("|[2]|"), "{rendered}");
        assert!(!rendered.contains("| 1 |"), "{rendered}");
        assert!(!rendered.contains("| 3 |"), "{rendered}");
        let chips = model.workspace_chips_text_from_labels(vec!["2".to_string()]);
        assert_eq!(chips, "|[2]|");
        assert!(chips.capacity() >= 8);
    }

    #[test]
    fn narrow_text_bar_prioritizes_active_workspace() {
        let custom = BarModel::new("dev", 1, "native-1", true, UNIX_EPOCH);
        let rendered = custom.render_i3bar(8);
        assert!(rendered.starts_with("|[dev]"), "{rendered}");
        assert!(!rendered.contains("| 1 | 2"), "{rendered}");

        let numeric = BarModel::new("3", 1, "native-1", true, UNIX_EPOCH);
        let rendered = numeric.render_i3bar(8);
        assert!(rendered.starts_with("|[3]"), "{rendered}");
        assert!(!rendered.starts_with("| 1 | 2"), "{rendered}");
        assert!(rendered.capacity() >= 8);
        assert_eq!(numeric.workspace_chip_labels_active_first(), vec!["3"]);
    }

    #[test]
    fn exact_fit_text_bar_keeps_normal_chip_order() {
        let model = BarModel::new("dev", 1, "native-1", true, UNIX_EPOCH);
        let full = model.render();
        let exact = model.render_i3bar(full.chars().count());
        assert_eq!(exact, full);
        assert!(exact.contains("|[dev]|"), "{exact}");
        assert!(!exact.contains("| 1 | 2 | 3 |"), "{exact}");
    }

    #[test]
    fn text_bar_includes_custom_active_workspace() {
        let model = BarModel::new("dev", 1, "native-1", true, UNIX_EPOCH);
        let rendered = model.render();
        assert!(rendered.contains("|[dev]|"), "{rendered}");
        assert!(!rendered.contains("| 1 | 2 | 3 |"), "{rendered}");
        assert_eq!(model.workspace_chip_labels(), vec!["dev"]);
    }

    #[test]
    fn workspace_label_trims_env_and_defaults_blank_values() {
        std::env::set_var("KITTWM_WORKSPACE", " dev ");
        assert_eq!(workspace_label(), "dev");
        std::env::set_var("KITTWM_WORKSPACE", "\t\n  ");
        let blank = workspace_label();
        assert_eq!(blank, "1");
        assert_eq!(blank.capacity(), 1);
        std::env::remove_var("KITTWM_WORKSPACE");
        assert_eq!(workspace_label(), "1");
    }

    #[test]
    fn time_label_uses_utc_clock_minutes() {
        assert_eq!(time_label(UNIX_EPOCH), "00:00 UTC");
        assert_eq!(
            time_label(UNIX_EPOCH + std::time::Duration::from_secs(23 * 3_600 + 59 * 60)),
            "23:59 UTC"
        );
        let label = time_label(UNIX_EPOCH + std::time::Duration::from_secs(5 * 60));
        assert_eq!(label, "00:05 UTC");
        assert_eq!(label.capacity(), "00:00 UTC".len());
    }

    #[test]
    fn workspace_chip_width_accounting_bounds_long_labels() {
        let long = "x".repeat(u16::MAX as usize);
        let labels = vec!["1".to_string(), long.clone()];
        assert_eq!(
            workspace_chip_total_cols(&labels),
            4 + TOP_BAR_LABEL_MAX_CHARS as u16 + 3
        );
        assert_eq!(
            workspace_chip_label_cols(&long),
            TOP_BAR_LABEL_MAX_CHARS as u16
        );

        let model = BarModel::new(long.clone(), 0, "-", false, UNIX_EPOCH);
        let constrained = model.workspace_chip_labels_for_scene(8);
        assert_eq!(constrained.first(), Some(&long));
    }

    #[test]
    fn text_and_scene_workspace_labels_are_bounded_for_pathological_input() {
        let long = "workspace-".repeat(10_000);
        let short = top_bar_display_label("dev");
        assert_eq!(short, "dev");
        assert!(matches!(short, Cow::Borrowed("dev")));
        let display = top_bar_display_label(&long);
        assert_eq!(display.chars().count(), TOP_BAR_LABEL_MAX_CHARS);
        assert!(matches!(display, Cow::Owned(_)));
        assert!(display.ends_with('…'));
        let model = BarModel::new(long, 1, "native-1", true, UNIX_EPOCH);
        let rendered = model.render();
        assert!(
            rendered.chars().count() < 100,
            "{}",
            rendered.chars().count()
        );
        assert!(rendered.contains('…'), "{rendered}");
        let narrow = model.render_i3bar(20);
        assert_eq!(narrow.chars().count(), 20);
        let scene = model.scene(20);
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains('…')));
        assert!(!scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains(&"workspace-".repeat(128))));
    }

    #[test]
    fn top_bar_clock_chip_skips_when_workspace_chips_overlap() {
        assert_eq!(top_bar_clock_chip_x(320.0, 80.0, 72.0), Some(247.0));
        assert_eq!(top_bar_clock_chip_x(160.0, 100.0, 72.0), None);
    }

    #[test]
    fn top_bar_clock_text_wraps_trimmed_clock() {
        assert_eq!(top_bar_clock_text("09:05"), " 09:05 ");
        assert_eq!(top_bar_clock_text(""), "  ");
    }

    #[test]
    fn top_bar_background_label_builds_directly() {
        assert_eq!(
            top_bar_background_label("kittwm-bar", "active", "dev"),
            "kittwm-bar:active:dev"
        );
    }

    #[test]
    fn top_bar_scene_clock_uses_actual_workspace_chip_end() {
        let model = BarModel::new("dev", 1, "native-1", true, UNIX_EPOCH);
        let scene = model.scene_with_prefix(23, "kittwm-bar");
        assert!(
            scene
                .layers
                .iter()
                .any(|layer| layer.label.as_deref()
                    == Some("kittwm-bar-clock-chip:00:00:high-contrast")),
            "{:#?}",
            scene
                .layers
                .iter()
                .map(|layer| &layer.label)
                .collect::<Vec<_>>()
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
    fn graphical_workspace_chip_geometry_is_bounded_to_scene_width() {
        let model = BarModel::new("super-long-workspace-name", 0, "-", false, UNIX_EPOCH);
        let scene = model.scene(18);
        let max_width = scene.footprint.cols as f32 * scene.cell_size.width_px as f32;
        for layer in scene.layers.iter().filter(|layer| {
            layer
                .label
                .as_deref()
                .unwrap_or_default()
                .contains("workspace-chip")
        }) {
            if let Node::Rect { rect, .. } = layer.root {
                assert!(rect.origin.0 + rect.width <= max_width, "{layer:?}");
            }
        }
        assert_eq!(top_bar_bounded_chip_width(24.0, 1.0, 80.0, 8.0), None);
        assert_eq!(
            top_bar_bounded_chip_width(80.0, 1.0, 200.0, 8.0),
            Some(78.0)
        );
    }

    #[test]
    fn custom_workspace_label_is_rendered_as_active_chip() {
        let model = BarModel::new("dev", 0, "-", false, UNIX_EPOCH);
        assert_eq!(model.workspace_chip_labels(), vec!["dev"]);
        let rendered = model.render();
        assert!(rendered.contains("|[dev]|"), "{rendered}");
        let scene = model.scene(60);
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip:dev:active")));
        assert!(!scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("action=workspace.switch")));
    }

    #[test]
    fn scene_root_label_trims_workspace_label() {
        let model = BarModel::new(" dev ", 0, "-", false, UNIX_EPOCH);
        let scene = model.scene(60);
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("kittwm-bar:empty:dev")));
        assert!(!scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains(" dev ")));
    }

    #[test]
    fn scene_shape_omits_clock_when_custom_workspace_would_overlap() {
        let model = BarModel::new("very-long-workspace-name", 0, "-", false, UNIX_EPOCH);
        let scene = model.scene(24);
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip:very-long-workspace-name:active")));
        assert!(!scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("clock-chip-foreground")));
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
            .contains("|[1]|")));
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

    #[test]
    fn scene_shape_includes_custom_workspace_chip() {
        let model = BarModel::new("dev", 0, "-", false, UNIX_EPOCH);
        let scene = model.scene(42);
        assert!(scene.layers.iter().any(|layer| layer
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("workspace-chip:dev:active")));
    }

    #[test]
    fn constrained_scene_prioritizes_active_workspace_chip() {
        let model = BarModel::new("3", 0, "-", false, UNIX_EPOCH);
        let scene = model.scene(4);
        let labels = scene
            .layers
            .iter()
            .filter_map(|layer| layer.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("workspace-chip:3:active")),
            "{labels:?}"
        );
        assert!(
            !labels
                .iter()
                .any(|label| label.contains("workspace-chip:1:inactive")),
            "{labels:?}"
        );
    }
}
