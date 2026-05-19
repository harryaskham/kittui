//! ratakittui — ratatui ↔ kittui adapter.
//!
//! This crate is a scaffold today: it declares the `KittuiDecorated` widget
//! wrapper and the diff-driven lifecycle tracker. Full coverage (Block,
//! Paragraph, List, Table, Tabs, Gauge, Sparkline, Chart, Canvas,
//! Scrollbar) and joined-border composition land in subsequent commits.
//!
//! The shape of the API is intentionally fixed now so downstream crates
//! can take a dependency without churn.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use std::collections::HashMap;

use parking_lot::Mutex;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use kittui::Scene;
use kittui::{Runtime, SceneId};

/// Tracks placements across ratatui draw cycles and drives the diff-based
/// upload + delete protocol. Hosts hold one of these for the lifetime of the
/// application.
pub struct LifecycleTracker {
    placed_last_frame: Mutex<HashMap<SceneId, u32>>,
    placed_this_frame: Mutex<HashMap<SceneId, u32>>,
}

impl Default for LifecycleTracker {
    fn default() -> Self {
        Self {
            placed_last_frame: Mutex::new(HashMap::new()),
            placed_this_frame: Mutex::new(HashMap::new()),
        }
    }
}

impl LifecycleTracker {
    /// Construct a fresh tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new frame. Existing placements are moved into "last frame".
    pub fn begin_frame(&self) {
        let mut last = self.placed_last_frame.lock();
        let mut this = self.placed_this_frame.lock();
        last.clear();
        for (k, v) in this.drain() {
            last.insert(k, v);
        }
    }

    /// Record that a scene was placed this frame so it survives the cleanup
    /// pass at end-of-frame.
    pub fn keep(&self, id: SceneId, image_id: u32) {
        self.placed_this_frame.lock().insert(id, image_id);
    }

    /// Finish the frame: return image ids that were placed last frame but
    /// not this frame, so the host can issue delete escapes.
    pub fn end_frame(&self) -> Vec<u32> {
        let last = self.placed_last_frame.lock();
        let this = self.placed_this_frame.lock();
        last.iter()
            .filter(|(id, _)| !this.contains_key(*id))
            .map(|(_, image_id)| *image_id)
            .collect()
    }
}

/// Decorate a ratatui widget with a kittui scene. The widget's text is
/// rendered as usual; the kittui scene is composited under it using the
/// kitty graphics protocol. The scene's footprint is derived from the
/// ratatui `Rect` at render time.
///
/// Full decoration coverage (joined borders, chips, titles, footers, etc.)
/// will arrive as additional builder methods on this type.
pub struct KittuiDecorated<'a, W> {
    /// Underlying ratatui widget.
    pub widget: W,
    /// Builder that turns a `Rect` into a kittui `Scene`.
    pub scene_for_rect: Box<dyn Fn(Rect) -> Scene + Send + Sync + 'a>,
}

impl<'a, W: Widget> KittuiDecorated<'a, W> {
    /// Construct a decorated widget.
    pub fn new(widget: W, scene_for_rect: impl Fn(Rect) -> Scene + Send + Sync + 'a) -> Self {
        Self {
            widget,
            scene_for_rect: Box::new(scene_for_rect),
        }
    }

    /// Render the kittui chrome through `runtime` and the ratatui widget on
    /// top. Returns the placement strings the host must write to the
    /// terminal before the widget's text is flushed.
    pub fn render_with(self, area: Rect, buf: &mut Buffer, runtime: &Runtime) -> RenderEffects {
        let scene = (self.scene_for_rect)(area);
        let id = scene.id();
        let placement = runtime.place(&scene).ok();
        self.widget.render(area, buf);
        RenderEffects {
            scene_id: id,
            image_id: placement.as_ref().map(|p| p.image_id),
            upload: placement.as_ref().map(|p| p.upload.clone()).unwrap_or_default(),
            placement: placement.as_ref().map(|p| p.placement.clone()).unwrap_or_default(),
            embed: placement.map(|p| p.embed).unwrap_or_default(),
        }
    }
}

/// Side-effect bundle returned by `KittuiDecorated::render_with`. Hosts pass
/// these to `LifecycleTracker::keep` and write the bytes to the terminal at
/// the correct point in their flush.
pub struct RenderEffects {
    /// Stable scene id (for lifecycle tracking).
    pub scene_id: SceneId,
    /// kitty image id assigned to the scene, if placement succeeded.
    pub image_id: Option<u32>,
    /// Upload escape sequence (empty on cache hit).
    pub upload: String,
    /// Placement escape sequence.
    pub placement: String,
    /// Unicode-placeholder grid.
    pub embed: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_tracker_diffs_against_last_frame() {
        let tracker = LifecycleTracker::new();

        tracker.begin_frame();
        tracker.keep(SceneId("a".repeat(64)), 1);
        tracker.keep(SceneId("b".repeat(64)), 2);
        assert!(tracker.end_frame().is_empty());

        tracker.begin_frame();
        tracker.keep(SceneId("a".repeat(64)), 1);
        let deleted = tracker.end_frame();
        assert_eq!(deleted, vec![2]);
    }
}
