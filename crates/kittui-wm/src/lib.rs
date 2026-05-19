//! kittui-wm
//!
//! Terminal window manager substrate. v0.1 is a placeholder: it defines
//! the `WindowGeometry` shape the eventual WM will route across the
//! renderer + cache + protocol layers, plus a `WindowTree::layout`
//! that's deliberately stubbed.
//!
//! The point of even shipping a stub crate today is that DESIGN.md
//! commits to "kittui-wm (long term)" as a phasing item, and we want
//! the workspace slot reserved so downstream consumers can take a
//! version dependency on `kittui-wm` and rely on the API growing only
//! additively from here.
//!
//! Real implementation lands once the tmux-border showcase exercises
//! the diff-driven composition path (DESIGN.md `## Future ideas`).

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use kittui::CellRect;

/// Geometry of a managed window in cell coordinates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// Cell-space rectangle the window occupies.
    pub rect: CellRect,
    /// Stable window id assigned by the WM.
    pub id: WindowId,
    /// Z-order (higher = on top).
    pub z: u16,
}

/// Stable window id allocated by the WM.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// Placeholder window tree. The real type grows split/stack/tab semantics
/// once the substrate is exercised by a real host.
#[derive(Default)]
pub struct WindowTree {
    windows: Vec<WindowGeometry>,
}

impl WindowTree {
    /// Construct an empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a window. The WM is in charge of conflict-free layout
    /// resolution; for now we just append.
    pub fn push(&mut self, window: WindowGeometry) {
        self.windows.push(window);
    }

    /// Borrow the windows in z-order.
    pub fn windows(&self) -> &[WindowGeometry] {
        &self.windows
    }

    /// Stub layout pass. Future revisions return per-window scenes.
    pub fn layout(&self) -> Vec<WindowGeometry> {
        let mut sorted = self.windows.clone();
        sorted.sort_by_key(|w| w.z);
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_sorts_by_z() {
        let mut tree = WindowTree::new();
        tree.push(WindowGeometry {
            rect: CellRect::new(0, 0, 40, 10),
            id: WindowId(1),
            z: 2,
        });
        tree.push(WindowGeometry {
            rect: CellRect::new(40, 0, 40, 10),
            id: WindowId(2),
            z: 1,
        });
        let laid = tree.layout();
        assert_eq!(laid[0].id, WindowId(2));
        assert_eq!(laid[1].id, WindowId(1));
    }
}
