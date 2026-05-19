//! ratakittui — ratatui ↔ kittui adapter.
//!
//! Implements the design laid out in `DESIGN.md`'s `## ratakittui` section:
//!
//! - `Chrome`: declarative description of background / border / title /
//!   footer / glow / scanlines / shadow / padding / clip, independent of
//!   which ratatui widget it wraps. Compiles down to a kittui `Scene`.
//! - Per-widget wrappers (`KittuiBlock`, `KittuiParagraph`, `KittuiList`,
//!   `KittuiTable`, `KittuiTabs`, `KittuiGauge`, `KittuiSparkline`,
//!   `KittuiLineGauge`, `KittuiBarChart`, `KittuiChart`, `KittuiCanvas`,
//!   `KittuiScrollbar`, `KittuiClear`) plus inline (`KittuiChip`,
//!   `KittuiTitle`, `KittuiDivider`, `KittuiLine`).
//! - `JoinGroup`: declare adjacency and produce one composite scene.
//! - `LifecycleTracker`: per-frame diff that emits delete escapes for
//!   widgets that left the tree.
//! - `draw_with_kittui`: ratatui draw adapter that injects upload /
//!   placement / embed bytes around the buffer flush.
//!
//! No `unsafe`. Stdout is never touched directly; effects are returned to
//! the host as byte buffers.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

mod chrome;
mod join;
mod lifecycle;
mod widgets;

pub use chrome::*;
pub use join::*;
pub use lifecycle::*;
pub use widgets::*;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use kittui::{Placement, Runtime, Scene};

/// Side-effect bundle returned by every `KittuiX::render_with` call. Hosts
/// pass these to [`LifecycleTracker::keep`] and write their bytes to the
/// terminal around the buffer flush.
#[derive(Clone, Debug, Default)]
pub struct RenderEffects {
    /// Stable scene id of the produced chrome. Empty if no chrome was drawn.
    pub scene_id: Option<kittui::SceneId>,
    /// kitty graphics image id assigned to the scene, if placement succeeded.
    pub image_id: Option<u32>,
    /// Upload escape sequence(s). Empty on cache + placement hit.
    pub upload: String,
    /// Placement escape sequence positioning the image under the cursor.
    pub placement: String,
    /// Unicode-placeholder grid the host writes at the chrome origin.
    pub embed: String,
    /// Cell footprint the chrome occupies in the terminal grid.
    pub footprint: Option<kittui::CellRect>,
}

impl RenderEffects {
    /// Build effects from a freshly produced placement.
    pub fn from_placement(placement: &Placement, scene_id: kittui::SceneId) -> Self {
        Self {
            scene_id: Some(scene_id),
            image_id: Some(placement.image_id),
            upload: placement.upload.clone(),
            placement: placement.placement.clone(),
            embed: placement.embed.clone(),
            footprint: Some(placement.footprint),
        }
    }

    /// Whether there's any chrome to flush.
    pub fn is_empty(&self) -> bool {
        self.scene_id.is_none()
    }
}

/// Render a kittui scene as chrome for a widget area and produce its
/// [`RenderEffects`]. This is the shared helper every widget wrapper uses;
/// hosts that want to build chrome without a wrapper can call it directly.
///
/// Returns `RenderEffects::default()` when there is no chrome (i.e. an
/// empty `Chrome`).
pub fn render_chrome(area: Rect, chrome: &Chrome, runtime: &Runtime) -> RenderEffects {
    let Some(scene) = chrome.to_scene(area) else {
        return RenderEffects::default();
    };
    let id = scene.id();
    match runtime.place(&scene) {
        Ok(placement) => RenderEffects::from_placement(&placement, id),
        Err(_) => RenderEffects::default(),
    }
}

/// Render `widget` into `area` of `buf` after first rendering `chrome`
/// through `runtime`. The chrome's effects are returned for the host to
/// flush; the widget's text lives in the ratatui `Buffer` as usual.
pub fn render_with_chrome<W: Widget>(
    widget: W,
    chrome: &Chrome,
    area: Rect,
    buf: &mut Buffer,
    runtime: &Runtime,
) -> RenderEffects {
    let effects = render_chrome(area, chrome, runtime);
    let inner = chrome.inner_rect(area);
    widget.render(inner, buf);
    effects
}

/// Build the cell rect a chrome's scene occupies in the terminal grid.
#[allow(dead_code)]
fn area_to_cell_rect(area: Rect) -> kittui::CellRect {
    kittui::CellRect {
        x: area.x,
        y: area.y,
        cols: area.width,
        rows: area.height,
    }
}

/// Convert a kittui `CellRect` back to a ratatui `Rect`.
#[allow(dead_code)]
fn cell_rect_to_area(rect: kittui::CellRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        width: rect.cols,
        height: rect.rows,
    }
}

#[allow(dead_code)]
fn _scene_must_be_send_sync(_s: Scene) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_round_trip_is_identity() {
        let area = Rect::new(3, 5, 60, 8);
        let rect = area_to_cell_rect(area);
        assert_eq!(cell_rect_to_area(rect), area);
    }
}
