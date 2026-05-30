//! Compose a kittui scene from a parsed pane layout.
//!
//! Tmux's pane separators live between panes (one cell wide, between
//! the right edge of one pane and the left edge of its neighbour;
//! similarly for top/bottom). We build a `ratakittui::JoinGroup` whose
//! members are one chrome per pane; the join resolver masks shared
//! inner edges so the merged stroke draws once at every separator.
//!
//! The output is the kittui escape stream the host writes to stdout
//! after positioning the cursor at the top-left of the terminal.

use kittui::{Rgba, Runtime};
use ratakittui::{Border, Chrome, JoinGroup, Joined};

use crate::parse::Pane;

/// Compose options. v0.1 keeps the surface small; future revisions can
/// add per-pane tint, status-line chrome, glow, etc.
#[derive(Clone, Debug)]
pub struct ComposeOptions {
    /// Border color used for all pane separators.
    pub border_color: Rgba,
    /// Border thickness in pixels.
    pub border_width_px: f32,
    /// Corner radius in pixels for outer-most corners.
    pub corner_radius_px: f32,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        Self {
            border_color: Rgba::parse("#00d8ff").unwrap(),
            border_width_px: 1.5,
            corner_radius_px: 6.0,
        }
    }
}

/// Output of [`compose_pane_chrome`].
#[derive(Clone, Debug)]
pub struct ComposeOutput {
    /// Bytes to write to the terminal at the cursor's current position.
    pub bytes: String,
    /// Number of placement scenes produced (one per pane).
    pub placements: usize,
}

/// Build a chrome scene per pane and place them through `runtime`. The
/// join resolver merges shared inner edges so abutting borders draw
/// once.
pub fn compose_pane_chrome(
    runtime: &Runtime,
    panes: &[Pane],
    options: &ComposeOptions,
) -> ComposeOutput {
    let chrome = Chrome::default().border(Border::rounded(
        options.border_color,
        options.border_width_px,
        options.corner_radius_px,
    ));

    let mut group = JoinGroup::new();
    for pane in panes {
        group.push(Joined {
            chrome: chrome.clone(),
            area: ratatui::layout::Rect {
                x: pane.left,
                y: pane.top,
                width: pane.width,
                height: pane.height,
            },
        });
    }

    let mut bytes = String::new();
    let mut placements = 0;
    for scene in group.resolve().into_iter().flatten() {
        if let Ok(placement) = runtime.place(&scene) {
            bytes.push_str(&placement.upload);
            bytes.push_str(&placement.placement);
            bytes.push_str(&placement.embed);
            placements += 1;
        }
    }
    ComposeOutput { bytes, placements }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui::RendererKind;
    use std::fmt::Write as FmtWrite;

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(tmux_test_temp_dir_name(pid, nanos));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn tmux_test_temp_dir_name(pid: u32, nanos: u128) -> String {
        let mut name = String::with_capacity(
            "kittui-tmux-".len() + decimal_len(pid as u128) + 1 + decimal_len(nanos),
        );
        name.push_str("kittui-tmux-");
        write!(name, "{pid}-{nanos}").expect("write to string");
        name
    }

    fn decimal_len(mut value: u128) -> usize {
        let mut digits = 1;
        while value >= 10 {
            value /= 10;
            digits += 1;
        }
        digits
    }

    #[test]
    fn tmux_test_temp_dir_name_builds_directly() {
        let name = tmux_test_temp_dir_name(1234, 5678);
        assert_eq!(name, "kittui-tmux-1234-5678");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn composes_two_panes_into_a_join_group_with_two_placements() {
        let runtime = Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap();
        let panes = vec![
            Pane {
                id: crate::parse::PaneId(0),
                left: 0,
                top: 0,
                width: 40,
                height: 10,
            },
            Pane {
                id: crate::parse::PaneId(1),
                left: 40,
                top: 0,
                width: 40,
                height: 10,
            },
        ];
        let out = compose_pane_chrome(&runtime, &panes, &ComposeOptions::default());
        assert_eq!(out.placements, 2);
        assert!(out.bytes.contains("\x1b_G"));
    }
}
