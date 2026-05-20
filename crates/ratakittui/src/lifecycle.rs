//! Per-frame lifecycle tracker and `draw_with_kittui` integration.
//!
//! The tracker is the single source of truth for which kittui scenes are
//! currently placed in the terminal. The host calls `begin_frame` before
//! drawing, accumulates effects through `keep`, then calls `end_frame` to
//! get the set of `image_id`s whose placements expired.
//!
//! No part of this module writes to stdout. Hosts collect the per-widget
//! `RenderEffects` and the lifecycle deletes into byte buffers themselves.

use std::collections::HashMap;
use std::io;

use parking_lot::Mutex;
use ratatui::Terminal;
use ratatui::Frame;
use ratatui::backend::Backend;

use kittui::{CellRect, Runtime, SceneId};

use crate::RenderEffects;

/// State per placed scene the tracker remembers between frames.
#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
struct PlacedState {
    image_id: u32,
    footprint: CellRect,
}

/// Per-frame lifecycle tracker. Cheap to clone (each `Mutex` is `Send`).
#[derive(Default)]
pub struct LifecycleTracker {
    prev: Mutex<HashMap<SceneId, PlacedState>>,
    current: Mutex<HashMap<SceneId, PlacedState>>,
}

impl LifecycleTracker {
    /// Construct an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new frame. Move current placements into "prev" and clear
    /// `current` so the upcoming `keep` calls populate it.
    pub fn begin_frame(&self) {
        let mut prev = self.prev.lock();
        let mut current = self.current.lock();
        prev.clear();
        for (id, st) in current.drain() {
            prev.insert(id, st);
        }
    }

    /// Mark a scene as placed this frame.
    pub fn keep(&self, effects: &RenderEffects) {
        if let (Some(id), Some(image_id), Some(footprint)) = (
            effects.scene_id.clone(),
            effects.image_id,
            effects.footprint,
        ) {
            self.current.lock().insert(
                id,
                PlacedState {
                    image_id,
                    footprint,
                },
            );
        }
    }

    /// End the frame and return the set of `image_id`s placed in the
    /// previous frame but not in the current one.
    pub fn end_frame(&self) -> Vec<u32> {
        let prev = self.prev.lock();
        let current = self.current.lock();
        prev.iter()
            .filter(|(id, _)| !current.contains_key(*id))
            .map(|(_, st)| st.image_id)
            .collect()
    }
}

/// Bytes the host should flush after a draw cycle. Splits the chrome
/// upload payload from the placement+embed payload so hosts can choose to
/// emit uploads at frame boundaries and placements next to widget rows.
#[derive(Default, Clone, Debug)]
pub struct DrawFlush {
    /// Concatenated upload escape sequences for the frame.
    pub upload: String,
    /// Concatenated placement + embed strings for the frame.
    pub placement: String,
    /// Delete escape sequences for placements that fell out of the frame.
    pub deletes: String,
}

impl DrawFlush {
    /// Whether there is any output at all.
    pub fn is_empty(&self) -> bool {
        self.upload.is_empty() && self.placement.is_empty() && self.deletes.is_empty()
    }
}

/// Frame-scoped sink that widget wrappers push their `RenderEffects` into.
/// `draw_with_kittui` constructs one of these per frame and drains it after
/// the buffer flush.
#[derive(Default)]
pub struct EffectsSink {
    effects: Mutex<Vec<RenderEffects>>,
}

impl EffectsSink {
    /// Construct an empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push effects produced by a widget wrapper.
    pub fn push(&self, effects: RenderEffects) {
        self.effects.lock().push(effects);
    }

    /// Consume the sink and return the accumulated effects in push order.
    pub fn drain(&self) -> Vec<RenderEffects> {
        self.effects.lock().drain(..).collect()
    }
}

/// Combine accumulated effects + lifecycle deletes into a `DrawFlush`.
///
/// Per-effect placements are preceded by a cursor-position escape
/// (`\x1b[<row>;<col>H`) computed from the effect's footprint so each
/// image anchors at the chrome origin instead of the current cursor. This
/// matches what a live host would do around its buffer flush.
pub fn finalize_frame(
    sink: &EffectsSink,
    tracker: &LifecycleTracker,
    runtime: &Runtime,
) -> DrawFlush {
    let mut out = DrawFlush::default();
    for effects in sink.drain() {
        tracker.keep(&effects);
        out.upload.push_str(&effects.upload);
        if let Some(fp) = effects.footprint {
            use std::fmt::Write as _;
            let _ = write!(out.placement, "\x1b[{};{}H", fp.y + 1, fp.x + 1);
        }
        out.placement.push_str(&effects.placement);
        out.placement.push_str(&effects.embed);
    }
    for image_id in tracker.end_frame() {
        out.deletes.push_str(&runtime.unplace(image_id));
    }
    out
}

/// ratatui draw adapter. Hosts call this in place of `terminal.draw(|f| ..)`.
///
/// The closure receives a `&mut Frame` plus the `EffectsSink` widget
/// wrappers push into. After the draw, the function returns a `DrawFlush`
/// the host can write to its terminal stream.
///
/// The host owns the terminal write side; this function does not perform
/// any IO beyond what `terminal.draw` already does.
pub fn draw_with_kittui<B, F>(
    terminal: &mut Terminal<B>,
    runtime: &Runtime,
    tracker: &LifecycleTracker,
    f: F,
) -> io::Result<DrawFlush>
where
    B: Backend,
    F: FnOnce(&mut Frame<'_>, &EffectsSink),
{
    let sink = EffectsSink::new();
    tracker.begin_frame();
    terminal.draw(|frame| f(frame, &sink))?;
    Ok(finalize_frame(&sink, tracker, runtime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui::CellRect;

    fn effects(id: &str, image_id: u32) -> RenderEffects {
        RenderEffects {
            scene_id: Some(SceneId(id.to_owned())),
            image_id: Some(image_id),
            upload: String::new(),
            placement: String::new(),
            embed: String::new(),
            footprint: Some(CellRect::new(0, 0, 4, 2)),
        }
    }

    #[test]
    fn tracker_diffs_against_previous_frame() {
        let tracker = LifecycleTracker::new();
        tracker.begin_frame();
        tracker.keep(&effects("a", 1));
        tracker.keep(&effects("b", 2));
        assert!(tracker.end_frame().is_empty());

        tracker.begin_frame();
        tracker.keep(&effects("a", 1));
        assert_eq!(tracker.end_frame(), vec![2]);
    }

    #[test]
    fn effects_sink_preserves_push_order() {
        let sink = EffectsSink::new();
        sink.push(effects("a", 1));
        sink.push(effects("b", 2));
        let drained = sink.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].image_id, Some(1));
        assert_eq!(drained[1].image_id, Some(2));
    }
}
