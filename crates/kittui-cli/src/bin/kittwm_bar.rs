//! `kittwm-bar` — a tiny first-party SDK status/top-bar renderer.
//!
//! This is intentionally small: it can be used as a standalone proof of the
//! default kittwm top bar model without requiring the live session to spawn it
//! yet. When a kittwm socket is available it reads typed SDK status; otherwise
//! it falls back to environment/default values so it remains useful in tests and
//! shell prompts.

use std::process::ExitCode;
use std::time::SystemTime;

use kittui::{CellRect, Rgba, Runtime, TerminalInfo};
use kittui_cli::top_bar::{workspace_chip_total_cols, workspace_label, BarModel};
use kittwm_sdk::{ChromeReservationRequest, ChromeReservationStatus, Kittwm, KittwmConfig, Status};
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
            workspace: normalized_optional_string(status.workspace.as_deref()),
            top_bar_rows: status.top_bar_rows,
            bottom_bar_rows: status.bottom_bar_rows,
            left_cols: status.left_cols,
            right_cols: status.right_cols,
            gap_cols: status.gap_cols,
            gap_rows: status.gap_rows,
            owner: normalized_optional_string(status.owner.as_deref()),
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
        let owner = reservation_owner_from_env();
        let request = ChromeReservationRequest::top_bar(1).owner(owner);
        client
            .reserve_chrome(&request)
            .map_err(|err| format!("reserve chrome: {err}"))?;
    }
    Ok(())
}

fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn reservation_owner_from_env() -> String {
    normalized_optional_string(std::env::var("KITTWM_WINDOW").ok().as_deref())
        .unwrap_or_else(|| "kittwm-bar".to_string())
}

fn status_workspace_label(status: &Status) -> String {
    normalized_optional_string(status.workspace.as_deref())
        .or_else(|| {
            status
                .chrome
                .as_ref()
                .and_then(|chrome| normalized_optional_string(chrome.workspace.as_deref()))
        })
        .unwrap_or_else(workspace_label)
}

fn render_kitty_bar(model: &BarOutputModel) -> Result<String, String> {
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()
        .map_err(|err| err.to_string())?;
    let scene = model.scene(scene_cols());
    let options = kittwm_bar_kitty_options(&scene);
    runtime
        .place_at_with_options(
            &scene,
            CellRect::new(0, 0, scene.footprint.cols, scene.footprint.rows),
            &options,
        )
        .map(|placement| {
            let mut bytes = placement.to_bytes();
            bytes.push_str(&kittwm_bar_kitty_text_overlay(model, scene.footprint.cols));
            bytes
        })
        .map_err(|err| err.to_string())
}

fn kittwm_bar_kitty_text_overlay(model: &BarOutputModel, cols: u16) -> String {
    let config = KittwmConfig::load_default().unwrap_or_default();
    kittwm_bar_kitty_text_overlay_with_config(model, cols, &config)
}

fn kittwm_bar_kitty_text_overlay_with_config(
    model: &BarOutputModel,
    cols: u16,
    config: &KittwmConfig,
) -> String {
    let palette = kittwm_bar_overlay_palette(config);
    let mut out = String::from("\x1b[0m\x1b[1;1H\x1b[K");
    let mut workspace_cols = 0u16;
    for label in kittwm_bar_overlay_labels(&model.bar, cols) {
        let active = model.bar.workspace.trim() == label;
        let (fg, bg) = if active {
            (palette.active_fg, palette.active_bg)
        } else {
            (palette.inactive_fg, palette.inactive_bg)
        };
        let Some(chip_text) = kittwm_bar_overlay_fit_chip_text(&label, cols, workspace_cols) else {
            break;
        };
        out.push_str(&format!(
            "\x1b[1m{}{}{}\x1b[0m ",
            ansi_fg(fg),
            ansi_bg(bg),
            chip_text
        ));
        workspace_cols = workspace_cols.saturating_add(kittwm_bar_overlay_text_cols(&chip_text, 1));
    }
    let clock = model
        .bar
        .time
        .strip_suffix(" UTC")
        .unwrap_or(&model.bar.time);
    let clock_text = format!(" {clock} ");
    if let Some(clock_col) = kittwm_bar_overlay_clock_col(
        cols,
        workspace_cols,
        kittwm_bar_overlay_text_cols(&clock_text, 0),
    ) {
        out.push_str(&format!(
            "\x1b[1;{}H\x1b[1m{}{}{}\x1b[0m",
            clock_col,
            ansi_fg(palette.clock_fg),
            ansi_bg(palette.clock_bg),
            clock_text
        ));
    }
    out
}

fn kittwm_bar_overlay_text_cols(text: &str, padding_cols: u16) -> u16 {
    let count = text.chars().count().min(u16::MAX as usize) as u16;
    count.saturating_add(padding_cols)
}

fn kittwm_bar_overlay_labels(model: &BarModel, cols: u16) -> Vec<String> {
    let labels = model.workspace_chip_labels();
    let total_cols = workspace_chip_total_cols(&labels);
    if total_cols > cols {
        model.workspace_chip_labels_active_first()
    } else {
        labels
    }
}

fn kittwm_bar_overlay_fit_chip_text(label: &str, cols: u16, used_cols: u16) -> Option<String> {
    let remaining = cols.saturating_sub(used_cols);
    if remaining == 0 {
        return None;
    }
    let max_chip_cols = remaining.saturating_sub(1) as usize;
    if max_chip_cols < 3 {
        return None;
    }
    let chip = format!(" {label} ");
    if chip.chars().count() <= max_chip_cols {
        return Some(chip);
    }
    let label_width = max_chip_cols.saturating_sub(2);
    if label_width == 0 {
        return None;
    }
    let clipped = label.chars().take(label_width).collect::<String>();
    Some(format!(" {clipped} "))
}

fn kittwm_bar_overlay_clock_col(cols: u16, workspace_cols: u16, clock_cols: u16) -> Option<u16> {
    if clock_cols > cols {
        return None;
    }
    let min_gap = 1;
    (workspace_cols
        .saturating_add(min_gap)
        .saturating_add(clock_cols)
        <= cols)
        .then(|| cols - clock_cols + 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KittwmBarOverlayPalette {
    active_fg: (u8, u8, u8),
    active_bg: (u8, u8, u8),
    inactive_fg: (u8, u8, u8),
    inactive_bg: (u8, u8, u8),
    clock_fg: (u8, u8, u8),
    clock_bg: (u8, u8, u8),
}

fn kittwm_bar_overlay_palette(config: &KittwmConfig) -> KittwmBarOverlayPalette {
    let inactive_bg =
        parse_bar_rgb(&config.background.color).unwrap_or(Rgba(0x3b, 0x42, 0x52, 255));
    let active_bg = config
        .colorscheme
        .ansi_color(4)
        .and_then(|value| parse_bar_rgb(value))
        .unwrap_or(Rgba(0x88, 0xc0, 0xd0, 255));
    let clock_bg = inactive_bg;
    KittwmBarOverlayPalette {
        active_fg: ansi_rgb(high_contrast_text_for(active_bg)),
        active_bg: ansi_rgb(active_bg),
        inactive_fg: ansi_rgb(high_contrast_text_for(inactive_bg)),
        inactive_bg: ansi_rgb(inactive_bg),
        clock_fg: ansi_rgb(high_contrast_text_for(clock_bg)),
        clock_bg: ansi_rgb(clock_bg),
    }
}

fn parse_bar_rgb(value: &str) -> Option<Rgba> {
    match value.trim().to_ascii_lowercase().as_str() {
        "nord0" => Some(Rgba(0x2e, 0x34, 0x40, 255)),
        "nord1" => Some(Rgba(0x3b, 0x42, 0x52, 255)),
        "nord2" => Some(Rgba(0x43, 0x4c, 0x5e, 255)),
        "nord3" => Some(Rgba(0x4c, 0x56, 0x6a, 255)),
        "nord4" => Some(Rgba(0xd8, 0xde, 0xe9, 255)),
        "nord5" => Some(Rgba(0xe5, 0xe9, 0xf0, 255)),
        "nord6" => Some(Rgba(0xec, 0xef, 0xf4, 255)),
        "nord7" => Some(Rgba(0x8f, 0xbc, 0xbb, 255)),
        "nord8" => Some(Rgba(0x88, 0xc0, 0xd0, 255)),
        "nord9" => Some(Rgba(0x81, 0xa1, 0xc1, 255)),
        "nord10" => Some(Rgba(0x5e, 0x81, 0xac, 255)),
        "nord11" => Some(Rgba(0xbf, 0x61, 0x6a, 255)),
        "nord12" => Some(Rgba(0xd0, 0x87, 0x70, 255)),
        "nord13" => Some(Rgba(0xeb, 0xcb, 0x8b, 255)),
        "nord14" => Some(Rgba(0xa3, 0xbe, 0x8c, 255)),
        "nord15" => Some(Rgba(0xb4, 0x8e, 0xad, 255)),
        other => Rgba::parse(other)
            .ok()
            .map(|color| Rgba(color.0, color.1, color.2, 255)),
    }
}

fn high_contrast_text_for(bg: Rgba) -> Rgba {
    let luminance = (u32::from(bg.0) * 299 + u32::from(bg.1) * 587 + u32::from(bg.2) * 114) / 1000;
    if luminance > 150 {
        Rgba(0x2e, 0x34, 0x40, 255)
    } else {
        Rgba(0xec, 0xef, 0xf4, 255)
    }
}

fn ansi_rgb(color: Rgba) -> (u8, u8, u8) {
    (color.0, color.1, color.2)
}

fn ansi_fg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn ansi_bg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[48;2;{r};{g};{b}m")
}

fn kittwm_bar_kitty_options(scene: &kittui::Scene) -> kittui_kitty::PlacementOptions {
    kittui_kitty::PlacementOptions::stable_absolute(scene.id().kitty_image_id()).with_z_index(20)
}

const MAX_KITTWM_BAR_COLS: u16 = 1000;

fn scene_cols() -> u16 {
    scene_cols_from_value(
        std::env::var("KITTWM_BAR_COLS")
            .or_else(|_| std::env::var("COLUMNS"))
            .ok()
            .as_deref(),
    )
}

fn scene_cols_from_value(value: Option<&str>) -> u16 {
    value
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|cols| *cols > 0)
        .map(|cols| cols.min(MAX_KITTWM_BAR_COLS))
        .unwrap_or(80)
}

fn load_bar_model(now: SystemTime) -> BarOutputModel {
    let Ok(client) = Kittwm::connect_from_env() else {
        return BarOutputModel::offline(now);
    };
    match client.status() {
        Ok(status) => {
            let panes = status.panes.unwrap_or(status.panes_detail.len() as u64);
            let workspace = status_workspace_label(&status);
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    fn reservation_owner_trims_window_env_and_defaults_blank_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WINDOW", " bar ");
        assert_eq!(reservation_owner_from_env(), "bar");
        std::env::set_var("KITTWM_WINDOW", "   ");
        assert_eq!(reservation_owner_from_env(), "kittwm-bar");
        std::env::remove_var("KITTWM_WINDOW");
        assert_eq!(reservation_owner_from_env(), "kittwm-bar");
    }

    #[test]
    fn sdk_status_fields_are_normalized_for_bar_model() {
        let status = Status {
            pending: None,
            panes: None,
            focus: None,
            layout: None,
            workspace: Some(" dev ".to_string()),
            chrome: Some(ChromeReservationStatus {
                workspace: Some(" dev ".to_string()),
                owner: Some(" bar ".to_string()),
                top_bar_rows: Some(1),
                ..ChromeReservationStatus::default()
            }),
            focused_pane: None,
            panes_detail: Vec::new(),
        };
        assert_eq!(status_workspace_label(&status), "dev");
        let chrome = BarChromeModel::from(status.chrome_reservation().unwrap());
        assert_eq!(chrome.workspace.as_deref(), Some("dev"));
        assert_eq!(chrome.owner.as_deref(), Some("bar"));

        let chrome_fallback = Status {
            pending: None,
            panes: None,
            focus: None,
            layout: None,
            workspace: Some("   ".to_string()),
            chrome: Some(ChromeReservationStatus {
                workspace: Some(" chrome-dev ".to_string()),
                ..ChromeReservationStatus::default()
            }),
            focused_pane: None,
            panes_detail: Vec::new(),
        };
        assert_eq!(status_workspace_label(&chrome_fallback), "chrome-dev");

        let blank = Status {
            pending: None,
            panes: None,
            focus: None,
            layout: None,
            workspace: Some("   ".to_string()),
            chrome: None,
            focused_pane: None,
            panes_detail: Vec::new(),
        };
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTWM_WORKSPACE", " fallback ");
        assert_eq!(status_workspace_label(&blank), "fallback");
        std::env::remove_var("KITTWM_WORKSPACE");

        let blank_chrome = BarChromeModel::from(&ChromeReservationStatus {
            workspace: Some("   ".to_string()),
            owner: Some("   ".to_string()),
            ..ChromeReservationStatus::default()
        });
        assert_eq!(blank_chrome.workspace, None);
        assert_eq!(blank_chrome.owner, None);
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
    fn kitty_bar_uses_stable_absolute_no_placeholder_options() {
        let model = BarOutputModel::offline(UNIX_EPOCH);
        let scene = model.scene(80);
        let options = kittwm_bar_kitty_options(&scene);
        assert_eq!(options.placement_id, Some(scene.id().kitty_image_id()));
        assert_eq!(options.z_index, 20);
        assert!(!options.unicode_placeholder);
    }

    #[test]
    fn scene_cols_caps_pathological_overrides() {
        assert_eq!(scene_cols_from_value(None), 80);
        assert_eq!(scene_cols_from_value(Some("0")), 80);
        assert_eq!(scene_cols_from_value(Some("120")), 120);
        assert_eq!(scene_cols_from_value(Some("65535")), MAX_KITTWM_BAR_COLS);
    }

    #[test]
    fn kitty_bar_overlay_text_cols_saturate_pathological_labels() {
        let long = "x".repeat(u16::MAX as usize + 32);
        assert_eq!(kittwm_bar_overlay_text_cols(&long, 0), u16::MAX);
        assert_eq!(kittwm_bar_overlay_text_cols(&long, 1), u16::MAX);
        assert_eq!(kittwm_bar_overlay_text_cols(" dev ", 1), 6);
    }

    #[test]
    fn kitty_bar_text_overlay_draws_visible_chips_and_clock() {
        let model = BarOutputModel {
            bar: BarModel::new(
                "2",
                0,
                "-",
                false,
                UNIX_EPOCH + std::time::Duration::from_secs(9 * 3600 + 5 * 60),
            ),
            chrome: None,
        };
        let overlay = kittwm_bar_kitty_text_overlay(&model, 40);
        assert!(overlay.starts_with("\x1b[0m\x1b[1;1H"), "{overlay:?}");
        assert!(!overlay.contains(" 1 "), "{overlay:?}");
        assert!(overlay.contains(" 2 "), "{overlay:?}");
        assert!(!overlay.contains(" 3 "), "{overlay:?}");
        assert!(overlay.contains(" 09:05 "), "{overlay:?}");
        assert!(overlay.contains("\x1b[38;2;"), "{overlay:?}");
        assert!(overlay.contains("\x1b[48;2;"), "{overlay:?}");
    }

    #[test]
    fn kitty_bar_text_overlay_uses_configured_theme_colors() {
        let mut config = KittwmConfig::default();
        config.background.color = "#112233".to_string();
        config.colorscheme.colors[4] = "#ddeeff".to_string();
        let model = BarOutputModel {
            bar: BarModel::new("2", 0, "-", false, UNIX_EPOCH),
            chrome: None,
        };
        let overlay = kittwm_bar_kitty_text_overlay_with_config(&model, 40, &config);
        assert!(overlay.contains("\x1b[48;2;221;238;255m 2 "), "{overlay:?}");
        assert!(!overlay.contains("\x1b[48;2;17;34;51m 1 "), "{overlay:?}");
        assert!(
            overlay.contains("\x1b[48;2;17;34;51m 00:00 "),
            "{overlay:?}"
        );
    }

    #[test]
    fn kitty_bar_text_overlay_includes_custom_workspace_label() {
        let model = BarOutputModel {
            bar: BarModel::new("dev", 0, "-", false, UNIX_EPOCH),
            chrome: None,
        };
        let overlay = kittwm_bar_kitty_text_overlay(&model, 60);
        assert!(overlay.starts_with("\x1b[0m\x1b[1;1H\x1b[K"), "{overlay:?}");
        assert!(overlay.contains(" dev "), "{overlay:?}");
    }

    #[test]
    fn kitty_bar_overlay_labels_saturate_long_workspace_width() {
        let long = "x".repeat(u16::MAX as usize);
        let model = BarModel::new(long.clone(), 0, "-", false, UNIX_EPOCH);
        let labels = kittwm_bar_overlay_labels(&model, 8);
        assert_eq!(labels.first(), Some(&long));
    }

    #[test]
    fn kitty_bar_text_overlay_prioritizes_active_workspace_when_constrained() {
        let custom = BarModel::new("dev", 0, "-", false, UNIX_EPOCH);
        assert_eq!(kittwm_bar_overlay_labels(&custom, 60), vec!["dev"]);
        assert_eq!(kittwm_bar_overlay_labels(&custom, 8), vec!["dev"]);
        let numeric = BarModel::new("3", 0, "-", false, UNIX_EPOCH);
        assert_eq!(kittwm_bar_overlay_labels(&numeric, 8), vec!["3"]);
    }

    #[test]
    fn kitty_bar_text_overlay_clips_long_workspace_labels_to_row() {
        let model = BarOutputModel {
            bar: BarModel::new("super-long-workspace-name", 0, "-", false, UNIX_EPOCH),
            chrome: None,
        };
        let overlay = kittwm_bar_kitty_text_overlay(&model, 18);
        assert!(overlay.starts_with("\x1b[0m\x1b[1;1H\x1b[K"), "{overlay:?}");
        assert!(overlay.contains(" super-long-work "), "{overlay:?}");
        assert!(
            !overlay.contains("super-long-workspace-name"),
            "{overlay:?}"
        );
        assert_eq!(
            kittwm_bar_overlay_fit_chip_text("abcdef", 6, 0),
            Some(" abc ".to_string())
        );
        assert_eq!(kittwm_bar_overlay_fit_chip_text("abcdef", 2, 0), None);
    }

    #[test]
    fn kitty_bar_text_overlay_omits_clock_when_workspace_chips_would_overlap() {
        let model = BarOutputModel {
            bar: BarModel::new(
                "super-long-workspace-name",
                0,
                "-",
                false,
                UNIX_EPOCH + std::time::Duration::from_secs(9 * 3600 + 5 * 60),
            ),
            chrome: None,
        };
        let overlay = kittwm_bar_kitty_text_overlay(&model, 18);
        assert!(overlay.contains(" super-long-work "), "{overlay:?}");
        assert!(
            !overlay.contains(" super-long-workspace-name "),
            "{overlay:?}"
        );
        assert!(!overlay.contains(" 09:05 "), "{overlay:?}");
        assert_eq!(kittwm_bar_overlay_clock_col(40, 12, 7), Some(34));
        assert_eq!(kittwm_bar_overlay_clock_col(18, 20, 7), None);
        assert_eq!(kittwm_bar_overlay_clock_col(4, 0, 7), None);
    }

    #[test]
    fn kitty_bar_overlay_prioritizes_numeric_active_workspace_when_constrained() {
        let model = BarOutputModel {
            bar: BarModel::new("3", 0, "-", false, UNIX_EPOCH),
            chrome: None,
        };
        assert_eq!(kittwm_bar_overlay_labels(&model.bar, 80), vec!["3"]);
        assert_eq!(kittwm_bar_overlay_labels(&model.bar, 8), vec!["3"]);
        let overlay = kittwm_bar_kitty_text_overlay(&model, 8);
        assert!(overlay.contains(" 3 "), "{overlay:?}");
        assert!(!overlay.contains(" 1  "), "{overlay:?}");
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
        assert!(model.render().contains("|[dev]|"));
        assert!(!model.render().contains("| 1 | 2 | 3 |"));
    }
}
