//! kittui-wm
//!
//! Terminal window manager substrate. v0.2 ships real split / stack / tab
//! layout semantics over a tree of nodes, producing a deterministic flat
//! list of `WindowGeometry` entries the renderer can drive directly.
//!
//! Design choice: layout returns geometry only, in cell coordinates. Scene
//! generation and diff-driven composition stay in `kittui` so this crate
//! avoids pulling the renderer/cache tree as a dependency.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use kittui::CellRect;

/// Platform accessibility-tree semantic adapter proof.
pub mod accessibility;
/// Dirty-grid helpers for future frame transport policy.
pub mod dirty;
/// Backend-independent native app surfaces (PTY terminal, headless browser).
pub mod native;
/// Semantic component surface model and kittui-affordances renderer bridge.
pub mod semantic;

/// Stable window id allocated by the WM.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

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

/// Split direction for a `LayoutNode::Split`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SplitDirection {
    /// Children laid out left-to-right.
    Horizontal,
    /// Children laid out top-to-bottom.
    Vertical,
}

/// A node in the window tree. Layout is recursive: `Window` is a leaf,
/// `Split` divides its rect proportionally among children, `Stack` z-orders
/// children in the same rect, `Tab` shows only the active child.
pub enum LayoutNode {
    /// A leaf window with a stable id and z-order offset.
    Window {
        /// Stable id.
        id: WindowId,
        /// Relative z bump within its parent stack/tab.
        z: u16,
    },
    /// Split children along a direction with `weights` summing to anything;
    /// each child gets `weight / sum(weights)` of the parent rect along the
    /// split axis.
    Split {
        /// Direction.
        direction: SplitDirection,
        /// `(weight, child)` pairs.
        children: Vec<(f32, LayoutNode)>,
    },
    /// Stack children at the same rect; later children are on top.
    Stack(Vec<LayoutNode>),
    /// Tab children at the same rect; only `active_index` is visible.
    Tab {
        /// Active tab index.
        active_index: usize,
        /// Children, in tab order.
        children: Vec<LayoutNode>,
    },
}

/// Layout root: an outer rect plus a single node.
pub struct WindowTree {
    /// The viewport rectangle to lay out into.
    pub rect: CellRect,
    /// The root layout node.
    pub root: LayoutNode,
}

impl WindowTree {
    /// Lay out the tree and return one `WindowGeometry` per visible window
    /// in input (depth-first) order.
    pub fn layout(&self) -> Vec<WindowGeometry> {
        let mut out = Vec::new();
        layout_node(&self.root, self.rect, 0, &mut out);
        // Deterministic z then id ordering for stable downstream composition.
        out.sort_by_key(|w| (w.z, w.id.0));
        out
    }
}

fn layout_node(node: &LayoutNode, rect: CellRect, z_base: u16, out: &mut Vec<WindowGeometry>) {
    match node {
        LayoutNode::Window { id, z } => {
            out.push(WindowGeometry {
                rect,
                id: *id,
                z: z_base.saturating_add(*z),
            });
        }
        LayoutNode::Split {
            direction,
            children,
        } => {
            let total: f32 = children.iter().map(|(w, _)| *w).sum::<f32>().max(0.000_1);
            let along: u32 = match direction {
                SplitDirection::Horizontal => rect.cols as u32,
                SplitDirection::Vertical => rect.rows as u32,
            };
            let mut consumed: u32 = 0;
            let mut cursor: u16 = match direction {
                SplitDirection::Horizontal => rect.x,
                SplitDirection::Vertical => rect.y,
            };
            for (i, (weight, child)) in children.iter().enumerate() {
                let is_last = i + 1 == children.len();
                let remaining = along.saturating_sub(consumed);
                let len: u16 = if is_last {
                    remaining.min(u32::from(u16::MAX)) as u16
                } else {
                    let slice = ((weight / total) * along as f32).round() as u32;
                    let clamped = slice.min(remaining);
                    consumed = consumed.saturating_add(clamped).min(along);
                    clamped.min(u32::from(u16::MAX)) as u16
                };
                let child_rect = match direction {
                    SplitDirection::Horizontal => CellRect::new(cursor, rect.y, len, rect.rows),
                    SplitDirection::Vertical => CellRect::new(rect.x, cursor, rect.cols, len),
                };
                layout_node(child, child_rect, z_base, out);
                cursor = cursor.saturating_add(len);
            }
        }
        LayoutNode::Stack(children) => {
            for (i, child) in children.iter().enumerate() {
                layout_node(child, rect, z_base.saturating_add(i as u16), out);
            }
        }
        LayoutNode::Tab {
            active_index,
            children,
        } => {
            if let Some(child) = children.get(*active_index) {
                layout_node(child, rect, z_base, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u32) -> LayoutNode {
        LayoutNode::Window {
            id: WindowId(id),
            z: 0,
        }
    }

    #[test]
    fn single_window_fills_viewport() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 80, 24),
            root: win(1),
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 1);
        assert_eq!(geo[0].rect, CellRect::new(0, 0, 80, 24));
    }

    #[test]
    fn horizontal_split_divides_columns() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 80, 24),
            root: LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                children: vec![(1.0, win(1)), (1.0, win(2))],
            },
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 2);
        let total: u32 = geo.iter().map(|g| g.rect.cols as u32).sum();
        assert_eq!(total, 80);
        // No overlap.
        assert_eq!(
            geo.iter()
                .map(|g| (g.rect.x, g.rect.cols))
                .collect::<Vec<_>>(),
            vec![(0, 40), (40, 40)]
        );
    }

    #[test]
    fn vertical_split_with_uneven_weights_uses_floor_with_last_remainder() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 10, 7),
            root: LayoutNode::Split {
                direction: SplitDirection::Vertical,
                children: vec![(2.0, win(1)), (1.0, win(2))],
            },
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 2);
        let total: u32 = geo.iter().map(|g| g.rect.rows as u32).sum();
        assert_eq!(total, 7);
    }

    #[test]
    fn split_never_over_consumes_tiny_horizontal_rect() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 1, 3),
            root: LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                children: vec![(100.0, win(1)), (100.0, win(2)), (100.0, win(3))],
            },
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 3);
        assert!(geo.iter().all(|g| g.rect.x <= 1));
        assert_eq!(geo.iter().map(|g| g.rect.cols as u32).sum::<u32>(), 1);
        assert_eq!(
            geo.iter()
                .map(|g| (g.rect.x, g.rect.cols))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 0), (0, 1)]
        );
    }

    #[test]
    fn split_never_over_consumes_tiny_vertical_rect() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 3, 1),
            root: LayoutNode::Split {
                direction: SplitDirection::Vertical,
                children: vec![(100.0, win(1)), (100.0, win(2)), (100.0, win(3))],
            },
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 3);
        assert!(geo.iter().all(|g| g.rect.y <= 1));
        assert_eq!(geo.iter().map(|g| g.rect.rows as u32).sum::<u32>(), 1);
        assert_eq!(
            geo.iter()
                .map(|g| (g.rect.y, g.rect.rows))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 0), (0, 1)]
        );
    }

    #[test]
    fn stack_assigns_increasing_z() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 10, 5),
            root: LayoutNode::Stack(vec![win(1), win(2), win(3)]),
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 3);
        let zs: Vec<_> = geo.iter().map(|g| g.z).collect();
        assert_eq!(zs, vec![0, 1, 2]);
    }

    #[test]
    fn tab_shows_only_active_child() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 10, 5),
            root: LayoutNode::Tab {
                active_index: 1,
                children: vec![win(1), win(2), win(3)],
            },
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 1);
        assert_eq!(geo[0].id, WindowId(2));
    }

    #[test]
    fn nested_split_inside_stack_keeps_no_overlap_within_each_layer() {
        let tree = WindowTree {
            rect: CellRect::new(0, 0, 20, 10),
            root: LayoutNode::Stack(vec![
                LayoutNode::Split {
                    direction: SplitDirection::Horizontal,
                    children: vec![(1.0, win(1)), (1.0, win(2))],
                },
                win(3),
            ]),
        };
        let geo = tree.layout();
        assert_eq!(geo.len(), 3);
        // Children of the inner split should tile the viewport horizontally.
        let split_xs: Vec<_> = geo
            .iter()
            .filter(|g| g.z == 0)
            .map(|g| (g.rect.x, g.rect.cols))
            .collect();
        assert_eq!(split_xs.len(), 2);
        let total: u32 = split_xs.iter().map(|(_, c)| *c as u32).sum();
        assert_eq!(total, 20);
    }
}

/// Reusable kittwm window chrome theme helpers.
pub mod chrome {
    use kittui::{PxRect, Rgba};
    use kittui_affordances::{InlineChipColors, InlineStyle, InlineTheme};
    use kittui_core::node::{Corners, Layer, Node, Stroke, StrokeAlign};
    use kittui_core::paint::Paint;

    /// Render-time state for a single window's chrome.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct WindowChromeState {
        /// Whether this window is currently focused.
        pub focused: bool,
        /// Whether this window is tiled by the WM layout tree.
        pub tiled: bool,
        /// Human-readable title or source label.
        pub title: String,
    }

    impl WindowChromeState {
        /// Construct a window chrome state record.
        pub fn new(focused: bool, tiled: bool, title: impl Into<String>) -> Self {
            Self {
                focused,
                tiled,
                title: title.into(),
            }
        }
    }

    /// Default color/shape tokens for kittwm chrome.
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct WindowChromeTheme {
        /// Border for focused windows.
        pub focused_border: Rgba,
        /// Border for unfocused windows.
        pub unfocused_border: Rgba,
        /// Transparent overlay fill.
        pub overlay_fill: Rgba,
        /// Focused border width.
        pub focused_border_width_px: f32,
        /// Unfocused border width.
        pub unfocused_border_width_px: f32,
        /// Rounded corner radius.
        pub corner_radius_px: f32,
    }

    impl Default for WindowChromeTheme {
        fn default() -> Self {
            let focused = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Neon);
            let unfocused = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Metal);
            Self {
                focused_border: focused.border,
                unfocused_border: unfocused.border,
                overlay_fill: unfocused.fill,
                focused_border_width_px: 2.0,
                unfocused_border_width_px: 1.0,
                corner_radius_px: 4.0,
            }
        }
    }

    impl WindowChromeTheme {
        /// Build the chrome layers for a window rectangle and state.
        pub fn layers(&self, rect: PxRect, state: &WindowChromeState) -> Vec<Layer> {
            let border = if state.focused {
                self.focused_border
            } else {
                self.unfocused_border
            };
            let width_px = if state.focused {
                self.focused_border_width_px
            } else {
                self.unfocused_border_width_px
            };
            let mode_label = if state.tiled { "tiled" } else { "floating" };
            vec![Layer::new(
                format!("wm-chrome:{mode_label}:{}", state.title),
                Node::Rect {
                    rect,
                    fill: Paint::Solid {
                        color: self.overlay_fill,
                    },
                    stroke: Some(Stroke {
                        align: StrokeAlign::Inside,
                        width_px,
                        paint: Paint::Solid { color: border },
                    }),
                    corners: Corners::uniform(self.corner_radius_px),
                },
            )]
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn default_theme_distinguishes_focused_and_unfocused_chrome() {
            let theme = WindowChromeTheme::default();
            let focused_tokens = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Neon);
            let unfocused_tokens = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Metal);
            assert_eq!(theme.focused_border, focused_tokens.border);
            assert_eq!(theme.unfocused_border, unfocused_tokens.border);
            assert_eq!(theme.overlay_fill, unfocused_tokens.fill);
            let rect = PxRect::new(0.0, 0.0, 80.0, 48.0);
            let focused = theme.layers(rect, &WindowChromeState::new(true, true, "term"));
            let unfocused = theme.layers(rect, &WindowChromeState::new(false, true, "term"));
            assert_eq!(focused.len(), 1);
            assert_eq!(unfocused.len(), 1);
            assert_eq!(focused[0].label.as_deref(), Some("wm-chrome:tiled:term"));
            match (&focused[0].root, &unfocused[0].root) {
                (
                    Node::Rect {
                        stroke: Some(a), ..
                    },
                    Node::Rect {
                        stroke: Some(b), ..
                    },
                ) => {
                    assert_ne!(a.width_px, b.width_px);
                    assert_ne!(a.paint, b.paint);
                }
                other => panic!("expected stroked rect chrome, got {other:?}"),
            }
        }
    }
}

/// Compositor that turns Xvfb-backed `XServer` windows into placed kittui
/// scenes, routes pointer events back to the X server, and tracks per-window
/// chrome through a `LifecycleTracker`-compatible delete pass.
pub mod compositor {
    use std::collections::HashMap;

    use kittui::{CellRect, CellSize, Scene};
    use kittui_core::geom::PxRect;
    use kittui_core::node::{Layer, Node};
    use kittui_input::{InputEvent, MouseButton};

    use crate::chrome::{WindowChromeState, WindowChromeTheme};
    use kittui_xvfb::{XButton, XPointerEvent, XServer, XWindowId};
    use parking_lot::Mutex;

    /// Layout mode for one window.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum WindowMode {
        /// Free-floating at an explicit pixel rect; draggable.
        Floating,
        /// Tiled within the WM's split tree.
        Tiled,
    }

    /// Compositor state. Cheap to clone (interior mutability).
    pub struct Compositor<S: XServer> {
        server: S,
        cell: CellSize,
        focused: Mutex<Option<XWindowId>>,
        modes: Mutex<HashMap<XWindowId, WindowMode>>,
        fullscreen: Mutex<HashMap<XWindowId, bool>>,
        z_order: Mutex<Vec<XWindowId>>,
        /// Last-known `(source_rect, terminal_footprint)` pair per window,
        /// populated by every `compose_with_layout` call. Pointer routing
        /// reads this to map terminal cells back into source pixels.
        placements: Mutex<HashMap<XWindowId, WindowPlacement>>,
        placement_order: Mutex<Vec<XWindowId>>,
    }

    /// Per-window mapping between the captured source surface and the
    /// terminal cell footprint it renders into.
    #[derive(Copy, Clone, Debug)]
    struct WindowPlacement {
        source_rect: PxRect,
        footprint: CellRect,
    }

    /// WM hot-path frame returned by `Compositor::raw_frames`. The session
    /// loop forwards this directly to `kittui::Runtime::place_raw_frame`
    /// without going through Scene / Pixmap / PNG encode.
    pub struct RawFrame {
        /// Backend-local window id.
        pub window_id: XWindowId,
        /// Stable kitty image id derived from the window id.
        pub image_id: u32,
        /// Frame width in pixels.
        pub width: u32,
        /// Frame height in pixels.
        pub height: u32,
        /// Tight RGBA8 bytes (row-major, no padding).
        pub rgba: Vec<u8>,
        /// Terminal cell footprint this frame should be placed at.
        pub footprint: CellRect,
        /// Human-readable chrome title for the frame.
        pub title: String,
        /// Whether this frame is focused.
        pub focused: bool,
        /// Layout mode used for chrome labels.
        pub mode: WindowMode,
        /// Whether this frame is fullscreened by the compositor.
        pub fullscreen: bool,
    }

    /// Derive a stable 32-bit kitty image id for an XWindowId. The kitty
    /// protocol image id space is per-terminal, so any stable mapping is
    /// fine; we mix in a high bit so XvfbServer's small ids don't collide
    /// with kittui's own scene-derived image ids.
    fn kitty_image_id_for(id: XWindowId) -> u32 {
        0x4000_0000 | id.0
    }

    impl<S: XServer> Compositor<S> {
        /// Construct a compositor over `server` with the given terminal cell metric.
        pub fn new(server: S, cell: CellSize) -> Self {
            Self {
                server,
                cell,
                focused: Mutex::new(None),
                modes: Mutex::new(HashMap::new()),
                fullscreen: Mutex::new(HashMap::new()),
                z_order: Mutex::new(Vec::new()),
                placements: Mutex::new(HashMap::new()),
                placement_order: Mutex::new(Vec::new()),
            }
        }

        /// Mark a window as floating or tiled.
        pub fn set_mode(&self, id: XWindowId, mode: WindowMode) {
            self.modes.lock().insert(id, mode);
        }

        /// Return the tracked mode for `id`, defaulting to floating.
        pub fn mode_of(&self, id: XWindowId) -> WindowMode {
            self.modes
                .lock()
                .get(&id)
                .copied()
                .unwrap_or(WindowMode::Floating)
        }

        /// Toggle the focused-or-first backend window between floating and tiled.
        pub fn toggle_focused_mode(
            &self,
        ) -> Result<Option<(XWindowId, WindowMode)>, kittui_xvfb::XError> {
            let Some(id) = self.focused_or_first_window()? else {
                return Ok(None);
            };
            self.set_focused(id);
            let next = match self.mode_of(id) {
                WindowMode::Floating => WindowMode::Tiled,
                WindowMode::Tiled => WindowMode::Floating,
            };
            self.set_mode(id, next);
            Ok(Some((id, next)))
        }

        /// Return whether a window is currently fullscreened.
        pub fn fullscreen_of(&self, id: XWindowId) -> bool {
            self.fullscreen.lock().get(&id).copied().unwrap_or(false)
        }

        /// Toggle fullscreen for the focused-or-first backend window.
        pub fn toggle_focused_fullscreen(
            &self,
        ) -> Result<Option<(XWindowId, bool)>, kittui_xvfb::XError> {
            let Some(id) = self.focused_or_first_window()? else {
                return Ok(None);
            };
            self.set_focused(id);
            let next = !self.fullscreen_of(id);
            self.fullscreen.lock().insert(id, next);
            Ok(Some((id, next)))
        }

        /// Set the focused backend window for chrome and key routing.
        pub fn set_focused(&self, id: XWindowId) {
            *self.focused.lock() = Some(id);
        }

        /// Return the focused backend window, if any.
        pub fn focused_window(&self) -> Option<XWindowId> {
            *self.focused.lock()
        }

        /// Return the focused window if it still exists, otherwise the first known window.
        pub fn focused_or_first_window(&self) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            let windows = self.server.windows()?;
            Ok(match self.focused_window() {
                Some(id) if windows.iter().any(|w| w.id == id) => Some(id),
                _ => windows.first().map(|w| w.id),
            })
        }

        /// Raise the focused-or-first backend window one z-order step.
        pub fn raise_focused(&self) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            self.move_focused_z(1)
        }

        /// Lower the focused-or-first backend window one z-order step.
        pub fn lower_focused(&self) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            self.move_focused_z(-1)
        }

        fn ordered_window_ids(&self, windows: &[kittui_xvfb::XWindow]) -> Vec<XWindowId> {
            let base = windows.iter().map(|w| w.id).collect::<Vec<_>>();
            let z_order = self.z_order.lock();
            let mut ordered = z_order
                .iter()
                .copied()
                .filter(|id| base.contains(id))
                .collect::<Vec<_>>();
            for id in base {
                if !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
            ordered
        }

        fn move_focused_z(&self, delta: isize) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            let windows = self.server.windows()?;
            let Some(id) = self.focused_or_first_window()? else {
                return Ok(None);
            };
            self.set_focused(id);
            let mut order = self.ordered_window_ids(&windows);
            let Some(pos) = order.iter().position(|candidate| *candidate == id) else {
                return Ok(Some(id));
            };
            let len = order.len() as isize;
            let next = (pos as isize + delta).clamp(0, len.saturating_sub(1)) as usize;
            order.remove(pos);
            order.insert(next, id);
            *self.z_order.lock() = order;
            Ok(Some(id))
        }

        /// Focus the next known backend window, wrapping at the end.
        pub fn focus_next(&self) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            self.focus_relative(1)
        }

        /// Focus the previous known backend window, wrapping at the start.
        pub fn focus_prev(&self) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            self.focus_relative(-1)
        }

        fn focus_relative(&self, delta: isize) -> Result<Option<XWindowId>, kittui_xvfb::XError> {
            let windows = self.server.windows()?;
            if windows.is_empty() {
                *self.focused.lock() = None;
                return Ok(None);
            }
            let ids = windows.iter().map(|w| w.id).collect::<Vec<_>>();
            let current = self.focused_window().unwrap_or(ids[0]);
            let pos = ids.iter().position(|id| *id == current).unwrap_or(0);
            let len = ids.len() as isize;
            let next = ((pos as isize + delta).rem_euclid(len)) as usize;
            let id = ids[next];
            self.set_focused(id);
            Ok(Some(id))
        }

        /// Borrow the underlying X server for direct access (advanced use).
        pub fn server(&self) -> &S {
            &self.server
        }

        /// Build one kittui Scene per X window, with simple border chrome.
        pub fn compose(&self) -> Result<Vec<Scene>, kittui_xvfb::XError> {
            self.compose_with_layout(&Layout::all_floating())
        }

        /// WM hot-path equivalent of [`compose_with_layout`] that returns
        /// raw RGBA frames + their cell footprints instead of `Scene`s. The
        /// session loop forwards each `RawFrame` straight to
        /// `kittui::Runtime::place_raw_frame` so the per-frame cost drops to
        /// one base64 + one write, no PNG encode.
        pub fn raw_frames(&self, layout: &Layout) -> Result<Vec<RawFrame>, kittui_xvfb::XError> {
            let windows = self.server.windows()?;
            let ordered_ids = self.ordered_window_ids(&windows);
            let windows_by_id = windows.iter().map(|w| (w.id, w)).collect::<HashMap<_, _>>();
            let modes = self.modes.lock().clone();
            let fullscreen = self.fullscreen.lock().clone();
            let focused_window = self.focused_or_first_window()?;
            let layout_bounds = layout.bounds();
            let mut placements_snapshot = HashMap::new();
            let mut placement_order = Vec::with_capacity(windows.len());
            let mut out = Vec::with_capacity(windows.len());
            for id in ordered_ids {
                let Some(w) = windows_by_id.get(&id).copied() else {
                    continue;
                };
                let mode = modes.get(&w.id).copied().unwrap_or(WindowMode::Floating);
                let is_fullscreen = fullscreen.get(&w.id).copied().unwrap_or(false);
                let target_rect =
                    target_rect_for(w.rect, mode, is_fullscreen, layout, layout_bounds, w.id);
                let cap = self.server.capture(w.id)?;
                let footprint_cols =
                    ((target_rect.width / self.cell.width_px as f32).ceil() as u16).max(1);
                let footprint_rows =
                    ((target_rect.height / self.cell.height_px as f32).ceil() as u16).max(1);
                let footprint = CellRect::new(
                    (target_rect.origin.0 / self.cell.width_px as f32) as u16,
                    (target_rect.origin.1 / self.cell.height_px as f32) as u16,
                    footprint_cols,
                    footprint_rows,
                );
                placements_snapshot.insert(
                    w.id,
                    WindowPlacement {
                        source_rect: PxRect::new(0.0, 0.0, cap.width as f32, cap.height as f32),
                        footprint,
                    },
                );
                placement_order.push(w.id);
                out.push(RawFrame {
                    window_id: w.id,
                    image_id: kitty_image_id_for(w.id),
                    width: cap.width,
                    height: cap.height,
                    rgba: cap.rgba,
                    footprint,
                    title: format!("x11:{}", w.id.0),
                    focused: focused_window == Some(w.id),
                    mode,
                    fullscreen: is_fullscreen,
                });
            }
            *self.placements.lock() = placements_snapshot;
            *self.placement_order.lock() = placement_order;
            Ok(out)
        }

        /// Build scenes using an explicit [`Layout`]. Tiled windows use the
        /// `tiled_rect` slot in the layout; floating windows keep their
        /// X-server-provided pixel rect.
        pub fn compose_with_layout(
            &self,
            layout: &Layout,
        ) -> Result<Vec<Scene>, kittui_xvfb::XError> {
            let windows = self.server.windows()?;
            let ordered_ids = self.ordered_window_ids(&windows);
            let windows_by_id = windows.iter().map(|w| (w.id, w)).collect::<HashMap<_, _>>();
            let modes = self.modes.lock().clone();
            let fullscreen = self.fullscreen.lock().clone();
            let focused_window = self.focused_or_first_window()?;
            let layout_bounds = layout.bounds();
            let mut placements_snapshot = HashMap::new();
            let mut placement_order = Vec::with_capacity(windows.len());
            let mut out = Vec::with_capacity(windows.len());
            for id in ordered_ids {
                let Some(w) = windows_by_id.get(&id).copied() else {
                    continue;
                };
                let mode = modes.get(&w.id).copied().unwrap_or(WindowMode::Floating);
                let is_fullscreen = fullscreen.get(&w.id).copied().unwrap_or(false);
                let target_rect =
                    target_rect_for(w.rect, mode, is_fullscreen, layout, layout_bounds, w.id);
                let cap = self.server.capture(w.id)?;
                let footprint_cols =
                    ((target_rect.width / self.cell.width_px as f32).ceil() as u16).max(1);
                let footprint_rows =
                    ((target_rect.height / self.cell.height_px as f32).ceil() as u16).max(1);
                let footprint = CellRect::new(
                    (target_rect.origin.0 / self.cell.width_px as f32) as u16,
                    (target_rect.origin.1 / self.cell.height_px as f32) as u16,
                    footprint_cols,
                    footprint_rows,
                );
                placements_snapshot.insert(
                    w.id,
                    WindowPlacement {
                        // The source rect is the captured surface itself,
                        // not the on-screen rect: pointer events need to
                        // land in source-image space because that is what
                        // the backend captured.
                        source_rect: PxRect::new(0.0, 0.0, cap.width as f32, cap.height as f32),
                        footprint,
                    },
                );
                placement_order.push(w.id);
                let rect = PxRect::new(0.0, 0.0, target_rect.width, target_rect.height);
                let png = encode_rgba(&cap.rgba, cap.width, cap.height);
                let mut layers = vec![Layer::anon(Node::Image {
                    rect,
                    src: kittui_core::node::ImageRef::Bytes { bytes: png },
                    fit: kittui_core::node::Fit::Stretch,
                    tint: None,
                })];
                let focused = focused_window == Some(w.id);
                let title = format!("x11:{}", w.id.0);
                layers.extend(WindowChromeTheme::default().layers(
                    rect,
                    &WindowChromeState::new(focused, mode == WindowMode::Tiled, title),
                ));
                out.push(Scene {
                    footprint,
                    cell_size: self.cell,
                    layers,
                    animation: None,
                });
            }
            *self.placements.lock() = placements_snapshot;
            *self.placement_order.lock() = placement_order;
            Ok(out)
        }

        /// Walk the windows top-down and return the window id at `(col, row)`.
        ///
        /// Uses the per-window placement recorded by the most recent
        /// `compose_with_layout` call so terminal cells map back to the
        /// right window regardless of how the source pixels were scaled.
        pub fn hit_test(&self, col: u16, row: u16) -> Option<XWindowId> {
            let placements = self.placements.lock();
            let order = self.placement_order.lock();
            // Later rendered windows are topmost for overlapping terminal cells.
            for id in order.iter().rev() {
                let Some(p) = placements.get(id) else {
                    continue;
                };
                if footprint_contains(&p.footprint, col, row) {
                    return Some(*id);
                }
            }
            None
        }

        /// Translate a kittui-input event into one or more `XPointerEvent`s
        /// and inject them into the server. Returns the events injected.
        pub fn route_pointer(&self, ev: &InputEvent) -> Vec<XPointerEvent> {
            let mut routed = Vec::new();
            match ev {
                InputEvent::MousePress {
                    col, row, button, ..
                }
                | InputEvent::MouseRelease {
                    col, row, button, ..
                }
                | InputEvent::MouseMove {
                    col, row, button, ..
                } => {
                    let Some(id) = self.hit_test(*col, *row) else {
                        return routed;
                    };
                    *self.focused.lock() = Some(id);
                    let placement = match self.placements.lock().get(&id).copied() {
                        Some(p) => p,
                        None => return routed,
                    };
                    let (lx, ly) = footprint_to_source_px(
                        *col,
                        *row,
                        &placement.footprint,
                        &placement.source_rect,
                    );
                    let x_event = XPointerEvent::Move {
                        window: id,
                        x_px: lx,
                        y_px: ly,
                    };
                    let _ = self.server.inject_pointer(x_event);
                    routed.push(x_event);
                    if let Some(xbtn) = button_to_x(*button) {
                        let mid = match ev {
                            InputEvent::MousePress { .. } => XPointerEvent::Press {
                                window: id,
                                button: xbtn,
                            },
                            InputEvent::MouseRelease { .. } => XPointerEvent::Release {
                                window: id,
                                button: xbtn,
                            },
                            _ => return routed,
                        };
                        let _ = self.server.inject_pointer(mid);
                        routed.push(mid);
                    }
                }
                _ => {}
            }
            routed
        }

        /// Translate a kittui-input key event into an X11 keysym + press flag
        /// and forward to the focused window. v1 uses a minimal mapping; the
        /// full keymap lands once kittui-wm exposes a layout.
        pub fn route_key(&self, ev: &InputEvent) -> Option<(u32, bool)> {
            let sym = match ev {
                InputEvent::Char { ch, .. } => *ch as u32,
                InputEvent::Key { key, .. } => keysym_for(*key)?,
                _ => return None,
            };
            let pressed = matches!(ev, InputEvent::Char { .. } | InputEvent::Key { .. });
            let _ = self.server.inject_key(sym, pressed);
            Some((sym, pressed))
        }
    }

    /// Mapping from window id to its tiled rectangle in X-server pixels.
    #[derive(Default, Clone, Debug)]
    pub struct Layout {
        tiled: HashMap<XWindowId, PxRect>,
    }

    impl Layout {
        /// Default: no tiled windows. Everything is floating.
        pub fn all_floating() -> Self {
            Self::default()
        }

        /// Assign `rect` as the tiled slot for `id`. Compositor will use
        /// this rect when the window's mode is [`WindowMode::Tiled`].
        pub fn tile(&mut self, id: XWindowId, rect: PxRect) {
            self.tiled.insert(id, rect);
        }

        /// Remove all tiled slots.
        pub fn clear(&mut self) {
            self.tiled.clear();
        }

        /// Return all tiled slots in arbitrary order.
        pub fn tiled_slots(&self) -> &HashMap<XWindowId, PxRect> {
            &self.tiled
        }

        /// Look up the tiled slot for a window, if any.
        pub fn tiled_rect(&self, id: XWindowId) -> Option<PxRect> {
            self.tiled.get(&id).copied()
        }

        /// Bounding rectangle enclosing all tiled slots, if any exist.
        pub fn bounds(&self) -> Option<PxRect> {
            let mut rects = self.tiled.values().copied();
            let first = rects.next()?;
            Some(rects.fold(first, px_rect_union))
        }
    }

    fn target_rect_for(
        window_rect: PxRect,
        mode: WindowMode,
        fullscreen: bool,
        layout: &Layout,
        layout_bounds: Option<PxRect>,
        id: XWindowId,
    ) -> PxRect {
        if fullscreen {
            return layout_bounds.unwrap_or_else(|| layout.tiled_rect(id).unwrap_or(window_rect));
        }
        match mode {
            WindowMode::Floating => window_rect,
            WindowMode::Tiled => layout.tiled_rect(id).unwrap_or(window_rect),
        }
    }

    fn px_rect_union(a: PxRect, b: PxRect) -> PxRect {
        let min_x = a.origin.0.min(b.origin.0);
        let min_y = a.origin.1.min(b.origin.1);
        let max_x = (a.origin.0 + a.width).max(b.origin.0 + b.width);
        let max_y = (a.origin.1 + a.height).max(b.origin.1 + b.height);
        PxRect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    fn footprint_contains(fp: &CellRect, col: u16, row: u16) -> bool {
        col >= fp.x && col < fp.x + fp.cols && row >= fp.y && row < fp.y + fp.rows
    }

    /// Map a terminal cell `(col, row)` inside `footprint` to the source
    /// pixel coordinate it represents inside `source_rect`. Used to route
    /// pointer events into the captured surface regardless of how the
    /// source was scaled into the terminal footprint.
    fn footprint_to_source_px(
        col: u16,
        row: u16,
        footprint: &CellRect,
        source_rect: &PxRect,
    ) -> (i32, i32) {
        let fx = if footprint.cols == 0 {
            0.0
        } else {
            (col.saturating_sub(footprint.x) as f32 + 0.5) / footprint.cols as f32
        };
        let fy = if footprint.rows == 0 {
            0.0
        } else {
            (row.saturating_sub(footprint.y) as f32 + 0.5) / footprint.rows as f32
        };
        let x = source_rect.origin.0 + fx * source_rect.width;
        let y = source_rect.origin.1 + fy * source_rect.height;
        (x as i32, y as i32)
    }

    #[allow(dead_code)]
    fn hit(rect: &PxRect, cell: &CellSize, col: u16, row: u16) -> bool {
        let px = (col as f32) * cell.width_px as f32;
        let py = (row as f32) * cell.height_px as f32;
        px >= rect.origin.0
            && px < rect.origin.0 + rect.width
            && py >= rect.origin.1
            && py < rect.origin.1 + rect.height
    }

    #[allow(dead_code)]
    fn local_px(col: u16, row: u16, cell: &CellSize, rect: &PxRect) -> (i32, i32) {
        let px = (col as f32) * cell.width_px as f32 - rect.origin.0;
        let py = (row as f32) * cell.height_px as f32 - rect.origin.1;
        (px as i32, py as i32)
    }

    fn button_to_x(b: MouseButton) -> Option<XButton> {
        Some(match b {
            MouseButton::Left => XButton::Left,
            MouseButton::Middle => XButton::Middle,
            MouseButton::Right => XButton::Right,
            MouseButton::ScrollUp => XButton::ScrollUp,
            MouseButton::ScrollDown => XButton::ScrollDown,
            MouseButton::None | MouseButton::Other(_) => return None,
        })
    }

    /// Map kittui-input named keys to X11 keysyms. v1 covers the common
    /// navigation + control keys; printable characters go through
    /// `InputEvent::Char`.
    fn keysym_for(k: kittui_input::Key) -> Option<u32> {
        use kittui_input::Key;
        Some(match k {
            Key::Up => 0xff52,
            Key::Down => 0xff54,
            Key::Left => 0xff51,
            Key::Right => 0xff53,
            Key::Home => 0xff50,
            Key::End => 0xff57,
            Key::PageUp => 0xff55,
            Key::PageDown => 0xff56,
            Key::Insert => 0xff63,
            Key::Delete => 0xffff,
            Key::Tab => 0xff09,
            Key::Backspace => 0xff08,
            Key::Enter => 0xff0d,
            Key::Escape => 0xff1b,
            Key::F(n) if (1..=12).contains(&n) => 0xffbd + n as u32,
            Key::F(_) => return None,
        })
    }

    fn encode_rgba(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
        // The compositor produces tight RGBA8; reuse the CPU renderer's PNG
        // encoder to ship to Node::Image::Bytes. This stays dep-free relative
        // to kittui-render-cpu which is already in this crate's tree.
        let mut pixmap = kittui_render_cpu::Pixmap::new(w, h);
        pixmap.data_mut().copy_from_slice(rgba);
        kittui_render_cpu::encode_png(&pixmap)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use kittui_xvfb::FakeServer;

        fn server() -> FakeServer {
            FakeServer::with_windows(vec![
                (
                    XWindowId(1),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "alpha",
                    [0xff, 0x00, 0x00, 0xff],
                ),
                (
                    XWindowId(2),
                    PxRect::new(80.0, 16.0, 64.0, 32.0),
                    "beta",
                    [0x00, 0xff, 0x00, 0xff],
                ),
            ])
        }

        #[test]
        fn compose_returns_one_scene_per_window() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let scenes = comp.compose().unwrap();
            assert_eq!(scenes.len(), 2);
        }

        #[test]
        fn hit_test_picks_topmost_window() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            // Compose to populate the placements snapshot used by hit_test.
            let _ = comp.compose().unwrap();
            // beta sits at px (80,16) → cell (10, 1).
            assert_eq!(comp.hit_test(11, 1), Some(XWindowId(2)));
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(1)));
            assert_eq!(comp.hit_test(50, 50), None);
        }

        #[test]
        fn route_pointer_injects_move_then_press() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let _ = comp.compose().unwrap();
            let routed = comp.route_pointer(&InputEvent::MousePress {
                button: MouseButton::Left,
                col: 11,
                row: 1,
                mods: Default::default(),
            });
            assert_eq!(routed.len(), 2);
            assert!(matches!(routed[0], XPointerEvent::Move { .. }));
            assert!(matches!(routed[1], XPointerEvent::Press { .. }));
        }

        #[test]
        fn route_key_forwards_ascii_to_focused_window() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let key = comp.route_key(&InputEvent::Char {
                ch: 'q',
                mods: Default::default(),
            });
            assert_eq!(key, Some(('q' as u32, true)));
        }

        #[test]
        fn route_key_maps_named_keys_to_x11_keysyms() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let up = comp.route_key(&InputEvent::Key {
                key: kittui_input::Key::Up,
                mods: Default::default(),
            });
            assert_eq!(up, Some((0xff52, true)));
            let f7 = comp.route_key(&InputEvent::Key {
                key: kittui_input::Key::F(7),
                mods: Default::default(),
            });
            assert_eq!(f7, Some((0xffbd + 7, true)));
        }

        #[test]
        fn hit_test_uses_last_rendered_window_as_topmost() {
            let server = FakeServer::with_windows(vec![
                (
                    XWindowId(1),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "bottom",
                    [0xff, 0x00, 0x00, 0xff],
                ),
                (
                    XWindowId(2),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "top",
                    [0x00, 0xff, 0x00, 0xff],
                ),
            ]);
            let comp = Compositor::new(server, CellSize::new(8, 16));
            let _ = comp.compose_with_layout(&Layout::all_floating()).unwrap();
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(2)));
            let routed = comp.route_pointer(&InputEvent::MousePress {
                button: MouseButton::Left,
                col: 1,
                row: 1,
                mods: Default::default(),
            });
            assert!(matches!(
                routed.first(),
                Some(XPointerEvent::Move {
                    window: XWindowId(2),
                    ..
                })
            ));
        }

        #[test]
        fn raise_and_lower_focused_window_changes_hit_test_order() {
            let server = FakeServer::with_windows(vec![
                (
                    XWindowId(1),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "bottom",
                    [0xff, 0x00, 0x00, 0xff],
                ),
                (
                    XWindowId(2),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "top",
                    [0x00, 0xff, 0x00, 0xff],
                ),
            ]);
            let comp = Compositor::new(server, CellSize::new(8, 16));
            let _ = comp.compose_with_layout(&Layout::all_floating()).unwrap();
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(2)));
            comp.set_focused(XWindowId(1));
            assert_eq!(comp.raise_focused().unwrap(), Some(XWindowId(1)));
            let frames = comp.raw_frames(&Layout::all_floating()).unwrap();
            assert_eq!(frames.last().unwrap().window_id, XWindowId(1));
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(1)));
            assert_eq!(comp.lower_focused().unwrap(), Some(XWindowId(1)));
            let frames = comp.raw_frames(&Layout::all_floating()).unwrap();
            assert_eq!(frames.first().unwrap().window_id, XWindowId(1));
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(2)));
        }

        #[test]
        fn raw_frames_update_hit_test_order() {
            let server = FakeServer::with_windows(vec![
                (
                    XWindowId(1),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "bottom",
                    [0xff, 0x00, 0x00, 0xff],
                ),
                (
                    XWindowId(2),
                    PxRect::new(0.0, 0.0, 64.0, 32.0),
                    "top",
                    [0x00, 0xff, 0x00, 0xff],
                ),
            ]);
            let comp = Compositor::new(server, CellSize::new(8, 16));
            let _ = comp.raw_frames(&Layout::all_floating()).unwrap();
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(2)));
        }

        #[test]
        fn pointer_in_downscaled_window_maps_back_to_source_pixels() {
            // Source surface is 1000x500 pixels; cell metric is 8x16; the
            // window is tiled into an 80x24 cell footprint at the origin.
            // A click at the centre of that footprint must land at the
            // centre of the source rect.
            let server = FakeServer::with_windows(vec![(
                XWindowId(42),
                PxRect::new(0.0, 0.0, 1000.0, 500.0),
                "hd",
                [0xaa, 0xbb, 0xcc, 0xff],
            )]);
            let comp = Compositor::new(server, CellSize::new(8, 16));
            let mut layout = Layout::all_floating();
            layout.tile(
                XWindowId(42),
                PxRect::new(0.0, 0.0, 80.0 * 8.0, 24.0 * 16.0),
            );
            comp.set_mode(XWindowId(42), WindowMode::Tiled);
            let _ = comp.compose_with_layout(&layout).unwrap();
            // Centre cell of an 80x24 footprint is (40, 12).
            let routed = comp.route_pointer(&InputEvent::MousePress {
                button: MouseButton::Left,
                col: 40,
                row: 12,
                mods: Default::default(),
            });
            assert_eq!(routed.len(), 2);
            if let XPointerEvent::Move { x_px, y_px, .. } = routed[0] {
                // Allow a few pixels of half-cell rounding either side of
                // (1000/2, 500/2) = (500, 250).
                assert!((x_px - 500).abs() < 20, "x_px should be ~500, got {x_px}");
                assert!((y_px - 250).abs() < 20, "y_px should be ~250, got {y_px}");
            } else {
                panic!("expected Move, got {:?}", routed[0]);
            }
        }

        #[test]
        fn compositor_chrome_labels_focus_and_layout_mode() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            comp.set_mode(XWindowId(2), WindowMode::Tiled);
            comp.set_focused(XWindowId(2));
            let mut layout = Layout::all_floating();
            layout.tile(XWindowId(2), PxRect::new(0.0, 0.0, 32.0, 16.0));
            let scenes = comp.compose_with_layout(&layout).unwrap();
            let labels = scenes
                .iter()
                .flat_map(|scene| {
                    scene
                        .layers
                        .iter()
                        .filter_map(|layer| layer.label.as_deref())
                })
                .collect::<Vec<_>>();
            assert!(labels.contains(&"wm-chrome:floating:x11:1"), "{labels:?}");
            assert!(labels.contains(&"wm-chrome:tiled:x11:2"), "{labels:?}");
            assert_eq!(comp.focused_window(), Some(XWindowId(2)));
        }

        #[test]
        fn raw_frames_include_chrome_metadata() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            comp.set_mode(XWindowId(2), WindowMode::Tiled);
            comp.set_focused(XWindowId(2));
            let mut layout = Layout::all_floating();
            layout.tile(XWindowId(2), PxRect::new(0.0, 0.0, 32.0, 16.0));
            let frames = comp.raw_frames(&layout).unwrap();
            let focused = frames
                .iter()
                .find(|frame| frame.window_id == XWindowId(2))
                .unwrap();
            assert!(focused.focused);
            assert_eq!(focused.mode, WindowMode::Tiled);
            assert_eq!(focused.title, "x11:2");
            let unfocused = frames
                .iter()
                .find(|frame| frame.window_id == XWindowId(1))
                .unwrap();
            assert!(!unfocused.focused);
            assert_eq!(unfocused.mode, WindowMode::Floating);
            assert!(!unfocused.fullscreen);
        }

        #[test]
        fn raw_frames_fullscreen_uses_layout_bounds() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let mut layout = Layout::all_floating();
            layout.tile(XWindowId(1), PxRect::new(0.0, 0.0, 32.0, 16.0));
            layout.tile(XWindowId(2), PxRect::new(32.0, 0.0, 32.0, 16.0));
            assert_eq!(
                comp.toggle_focused_fullscreen().unwrap(),
                Some((XWindowId(1), true))
            );
            let frames = comp.raw_frames(&layout).unwrap();
            let fullscreen = frames
                .iter()
                .find(|frame| frame.window_id == XWindowId(1))
                .unwrap();
            assert!(fullscreen.fullscreen);
            assert_eq!(fullscreen.footprint, CellRect::new(0, 0, 8, 1));
            assert_eq!(
                comp.toggle_focused_fullscreen().unwrap(),
                Some((XWindowId(1), false))
            );
            assert!(!comp.fullscreen_of(XWindowId(1)));
        }

        #[test]
        fn raw_frames_default_to_single_focused_window_and_cycle() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let frames = comp.raw_frames(&Layout::all_floating()).unwrap();
            assert_eq!(frames.iter().filter(|frame| frame.focused).count(), 1);
            assert_eq!(
                frames.iter().find(|frame| frame.focused).unwrap().window_id,
                XWindowId(1)
            );
            assert_eq!(comp.focus_next().unwrap(), Some(XWindowId(2)));
            let frames = comp.raw_frames(&Layout::all_floating()).unwrap();
            assert_eq!(
                frames.iter().find(|frame| frame.focused).unwrap().window_id,
                XWindowId(2)
            );
            assert_eq!(comp.focus_next().unwrap(), Some(XWindowId(1)));
            assert_eq!(comp.focus_prev().unwrap(), Some(XWindowId(2)));
        }

        #[test]
        fn toggle_focused_mode_changes_compositor_mode() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            assert_eq!(comp.mode_of(XWindowId(1)), WindowMode::Floating);
            assert_eq!(
                comp.toggle_focused_mode().unwrap(),
                Some((XWindowId(1), WindowMode::Tiled))
            );
            assert_eq!(comp.focused_window(), Some(XWindowId(1)));
            assert_eq!(comp.mode_of(XWindowId(1)), WindowMode::Tiled);
            assert_eq!(
                comp.toggle_focused_mode().unwrap(),
                Some((XWindowId(1), WindowMode::Floating))
            );
            assert_eq!(comp.mode_of(XWindowId(1)), WindowMode::Floating);
        }

        #[test]
        fn compose_defaults_to_one_focused_window() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            let scenes = comp.compose_with_layout(&Layout::all_floating()).unwrap();
            let chrome_widths = scenes
                .iter()
                .flat_map(|scene| scene.layers.iter())
                .filter(|layer| {
                    layer
                        .label
                        .as_deref()
                        .is_some_and(|label| label.starts_with("wm-chrome:"))
                })
                .filter_map(|layer| match &layer.root {
                    Node::Rect {
                        stroke: Some(stroke),
                        ..
                    } => Some(stroke.width_px),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                chrome_widths.iter().filter(|width| **width == 2.0).count(),
                1
            );
            assert_eq!(
                chrome_widths.iter().filter(|width| **width == 1.0).count(),
                1
            );
        }

        #[test]
        fn tiled_windows_use_layout_rect_instead_of_x_rect() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
            comp.set_mode(XWindowId(1), WindowMode::Tiled);
            let mut layout = Layout::all_floating();
            layout.tile(XWindowId(1), PxRect::new(40.0, 0.0, 32.0, 16.0));
            let scenes = comp.compose_with_layout(&layout).unwrap();
            assert_eq!(scenes.len(), 2);
            // The tiled window's footprint should originate at cell (5, 0)
            // (40 / 8 = 5) and span 4 cols (32 / 8) by 1 row (16 / 16).
            let tiled = &scenes[0];
            assert_eq!(tiled.footprint.x, 5);
            assert_eq!(tiled.footprint.cols, 4);
            assert_eq!(tiled.footprint.rows, 1);
        }
    }
}

/// Backend-multiplexed compositor: many `XServer` backends in one session.
///
/// Each window is identified by a globally-unique [`WindowKey`] = `(backend_index, x_window_id)`,
/// so two backends can't collide. The compositor renders each window as a
/// kittui scene with chrome regardless of whether it originated locally, on
/// a remote Xvfb over SSH, on a Quartz display capture, in XQuartz, or
/// elsewhere. Per-window mode (Floating / Tiled) and chrome theme live on
/// the compositor; backends only have to honour the small `XServer` contract.
pub mod multi {
    use kittui::{CellRect, CellSize, Rgba, Scene};
    use kittui_affordances::{InlineAccentPalette, InlineChipColors, InlineStyle, InlineTheme};
    use kittui_core::geom::PxRect;
    use kittui_core::node::{Corners, Layer, Node};
    use kittui_core::paint::Paint;
    use kittui_input::{InputEvent, MouseButton};
    use kittui_xvfb::{XButton, XCapture, XError, XPointerEvent, XServer, XWindow, XWindowId};
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Globally-unique window identifier across all attached backends.
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
    pub struct WindowKey {
        /// Index into the compositor's backend list.
        pub backend: usize,
        /// Backend-local window id.
        pub window: XWindowId,
    }

    /// Layout mode for one window in the multiplex compositor.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum WindowMode {
        /// Free-floating at the backend's reported rect.
        Floating,
        /// Tiled into the [`Layout`]'s slot for this window.
        Tiled,
    }

    /// Tiled-slot map keyed by `WindowKey`.
    #[derive(Default, Clone, Debug)]
    pub struct Layout {
        tiled: HashMap<WindowKey, PxRect>,
    }

    impl Layout {
        /// Empty layout (every window stays floating).
        pub fn empty() -> Self {
            Self::default()
        }
        /// Assign a tiled slot for `key`.
        pub fn tile(&mut self, key: WindowKey, rect: PxRect) {
            self.tiled.insert(key, rect);
        }
        /// Look up the tiled slot for `key`.
        pub fn tiled_rect(&self, key: WindowKey) -> Option<PxRect> {
            self.tiled.get(&key).copied()
        }
    }

    /// Background worker that continuously captures one backend's windows
    /// and stores the latest frame per `XWindowId` in a slot the UI thread
    /// reads non-blockingly.
    ///
    /// A `Pump` keeps a single OS thread per backend. The thread loops:
    /// 1. call `backend.windows()`,
    /// 2. for each window, call `backend.capture(id)`,
    /// 3. store `(window_metadata, capture)` in the slot keyed by `XWindowId`,
    /// 4. sleep for a small interval (~16ms) and repeat.
    ///
    /// The UI thread reads the slot via `Pump::snapshot()` which never
    /// blocks: missing windows return empty, slow captures stale until
    /// they catch up. Dropping the `Pump` signals the worker to exit on
    /// its next iteration.
    pub struct Pump {
        backend: Arc<dyn XServer + Send + Sync>,
        slot: Arc<PumpSlot>,
        stop: Arc<AtomicBool>,
        _worker: Option<std::thread::JoinHandle<()>>,
    }

    /// Latest-frame-wins snapshot the Pump worker writes to and the UI
    /// thread reads from. `XCapture` is the heavy part (RGBA bytes).
    #[derive(Default)]
    struct PumpSlot {
        windows: Mutex<Vec<XWindow>>,
        captures: Mutex<HashMap<XWindowId, XCapture>>,
        last_err: Mutex<Option<String>>,
    }

    /// What the Pump's UI-thread reader gets back for one window: the
    /// last-known window metadata + the last-known capture (if any).
    #[derive(Clone)]
    pub struct PumpFrame {
        /// Window metadata as of the most recent successful enumeration.
        pub window: XWindow,
        /// Last successful capture; `None` if the worker hasn't captured
        /// this window yet.
        pub capture: Option<XCapture>,
    }

    impl Pump {
        /// Spawn a worker thread that captures `backend`'s windows in a
        /// loop. Returns immediately; first frames arrive asynchronously.
        pub fn spawn(backend: Arc<dyn XServer + Send + Sync>) -> Self {
            let slot = Arc::new(PumpSlot::default());
            let stop = Arc::new(AtomicBool::new(false));
            let worker = {
                let backend = backend.clone();
                let slot = slot.clone();
                let stop = stop.clone();
                std::thread::Builder::new()
                    .name("kittui-wm-pump".into())
                    .spawn(move || pump_worker(backend, slot, stop))
                    .ok()
            };
            Self {
                backend,
                slot,
                stop,
                _worker: worker,
            }
        }

        /// Non-blocking read of the current frames for every known window.
        pub fn snapshot(&self) -> Vec<PumpFrame> {
            let windows = self.slot.windows.lock().clone();
            let captures = self.slot.captures.lock();
            windows
                .into_iter()
                .map(|w| {
                    let capture = captures.get(&w.id).cloned();
                    PumpFrame { window: w, capture }
                })
                .collect()
        }

        /// Last recorded backend error, if any. Cleared on next success.
        pub fn last_error(&self) -> Option<String> {
            self.slot.last_err.lock().clone()
        }

        /// Forward an input event to the underlying backend. Pointer/key
        /// injection runs on the calling thread because injection is fast
        /// (every backend's `inject_*` is non-blocking by contract).
        pub fn inject_pointer(&self, ev: XPointerEvent) -> Result<(), XError> {
            self.backend.inject_pointer(ev)
        }

        /// See [`Pump::inject_pointer`].
        pub fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
            self.backend.inject_key(sym, pressed)
        }
    }

    impl Drop for Pump {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            // Don't join: the worker may be inside a blocking capture and
            // we don't want the UI thread to wait. The thread is a daemon
            // in effect and exits on its own once stop is observed.
        }
    }

    /// `Arc<dyn XServer>` adapter so a backend can live behind a `Pump`
    /// (which holds an Arc) while still satisfying the compositor's
    /// `Box<dyn XServer + Send + Sync>` synchronous slot.
    struct SharedServer(Arc<dyn XServer + Send + Sync>);

    impl XServer for SharedServer {
        fn windows(&self) -> Result<Vec<XWindow>, XError> {
            self.0.windows()
        }
        fn capture(&self, id: XWindowId) -> Result<XCapture, XError> {
            self.0.capture(id)
        }
        fn resize_window(&self, id: XWindowId, width: u32, height: u32) -> Result<(), XError> {
            self.0.resize_window(id, width, height)
        }
        fn inject_pointer(&self, ev: XPointerEvent) -> Result<(), XError> {
            self.0.inject_pointer(ev)
        }
        fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
            self.0.inject_key(sym, pressed)
        }
    }

    fn pump_worker(
        backend: Arc<dyn XServer + Send + Sync>,
        slot: Arc<PumpSlot>,
        stop: Arc<AtomicBool>,
    ) {
        let tick = std::time::Duration::from_millis(16);
        while !stop.load(Ordering::Relaxed) {
            match backend.windows() {
                Ok(windows) => {
                    *slot.windows.lock() = windows.clone();
                    *slot.last_err.lock() = None;
                    for w in &windows {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        match backend.capture(w.id) {
                            Ok(cap) => {
                                slot.captures.lock().insert(w.id, cap);
                            }
                            Err(e) => {
                                *slot.last_err.lock() = Some(format!("capture {:?}: {e}", w.id));
                            }
                        }
                    }
                }
                Err(e) => {
                    *slot.last_err.lock() = Some(format!("windows: {e}"));
                }
            }
            std::thread::sleep(tick);
        }
    }

    /// Multi-backend compositor. Cheap to clone (interior mutability).
    pub struct MultiCompositor {
        backends: Vec<Box<dyn XServer + Send + Sync>>,
        /// Optional per-backend `Pump`. When attached via `attach_pump`,
        /// the UI-thread compose path can read frames non-blockingly via
        /// `compose_via_pumps`. Sync compose (`compose`) ignores pumps.
        pumps: Vec<Option<Pump>>,
        cell: CellSize,
        modes: Mutex<HashMap<WindowKey, WindowMode>>,
        focused: Mutex<Option<WindowKey>>,
    }

    impl MultiCompositor {
        /// Construct a compositor with no backends attached. Use [`attach`].
        pub fn new(cell: CellSize) -> Self {
            Self {
                backends: Vec::new(),
                pumps: Vec::new(),
                cell,
                modes: Mutex::new(HashMap::new()),
                focused: Mutex::new(None),
            }
        }

        /// Attach a backend (synchronous capture path); returns its index.
        pub fn attach(&mut self, server: Box<dyn XServer + Send + Sync>) -> usize {
            self.backends.push(server);
            self.pumps.push(None);
            self.backends.len() - 1
        }

        /// Attach a backend and spawn a `Pump` worker thread for it. The
        /// compositor stores a synchronous handle so existing sync API
        /// keeps working; new async API reads from the pump's slot.
        ///
        /// Returns the backend's index.
        pub fn attach_pump(&mut self, backend: Arc<dyn XServer + Send + Sync>) -> usize {
            let pump = Pump::spawn(backend.clone());
            self.backends.push(Box::new(SharedServer(backend)));
            self.pumps.push(Some(pump));
            self.backends.len() - 1
        }

        /// Number of attached backends.
        pub fn backend_count(&self) -> usize {
            self.backends.len()
        }

        /// Set a window's layout mode.
        pub fn set_mode(&self, key: WindowKey, mode: WindowMode) {
            self.modes.lock().insert(key, mode);
        }

        /// Build one kittui scene per visible window across all backends.
        pub fn compose(&self, layout: &Layout) -> Result<Vec<(WindowKey, Scene)>, XError> {
            let modes = self.modes.lock().clone();
            let mut out = Vec::new();
            for (i, server) in self.backends.iter().enumerate() {
                let windows = server.windows()?;
                for w in windows {
                    let key = WindowKey {
                        backend: i,
                        window: w.id,
                    };
                    let mode = modes.get(&key).copied().unwrap_or(WindowMode::Floating);
                    let target_rect = match mode {
                        WindowMode::Floating => w.rect,
                        WindowMode::Tiled => layout.tiled_rect(key).unwrap_or(w.rect),
                    };
                    let cap = server.capture(w.id)?;
                    let cols =
                        ((target_rect.width / self.cell.width_px as f32).ceil() as u16).max(1);
                    let rows =
                        ((target_rect.height / self.cell.height_px as f32).ceil() as u16).max(1);
                    let footprint = CellRect::new(
                        (target_rect.origin.0 / self.cell.width_px as f32) as u16,
                        (target_rect.origin.1 / self.cell.height_px as f32) as u16,
                        cols,
                        rows,
                    );
                    let rect = PxRect::new(0.0, 0.0, target_rect.width, target_rect.height);
                    let border = backend_color(i);
                    let bg = compositor_overlay_fill();
                    let png = kittui_render_cpu::encode_png(&{
                        let mut p = kittui_render_cpu::Pixmap::new(cap.width, cap.height);
                        p.data_mut().copy_from_slice(&cap.rgba);
                        p
                    });
                    let layers = vec![
                        Layer::anon(Node::Image {
                            rect,
                            src: kittui_core::node::ImageRef::Bytes { bytes: png },
                            fit: kittui_core::node::Fit::Stretch,
                            tint: None,
                        }),
                        Layer::anon(Node::Rect {
                            rect,
                            fill: Paint::Solid { color: bg },
                            stroke: Some(kittui_core::node::Stroke {
                                align: kittui_core::node::StrokeAlign::Inside,
                                width_px: 1.5,
                                paint: Paint::Solid { color: border },
                            }),
                            corners: Corners::uniform(4.0),
                        }),
                    ];
                    out.push((
                        key,
                        Scene {
                            footprint,
                            cell_size: self.cell,
                            layers,
                            animation: None,
                        },
                    ));
                }
            }
            Ok(out)
        }

        /// Non-blocking compose that reads cached frames from attached
        /// `Pump`s instead of calling `XServer::capture` synchronously.
        ///
        /// Backends attached via `attach` (no pump) are skipped. Backends
        /// attached via `attach_pump` contribute every window the pump
        /// has seen so far; if a capture hasn't arrived yet for a window
        /// the scene's image layer is omitted but the border chrome still
        /// renders, so the user sees a placeholder until the first frame
        /// lands.
        ///
        /// This method never blocks on backend work and is the path the
        /// daemon's UI thread should call once `bd-fb5d9d` lands.
        pub fn compose_via_pumps(&self, layout: &Layout) -> Vec<(WindowKey, Scene)> {
            let modes = self.modes.lock().clone();
            let mut out = Vec::new();
            for (i, pump_opt) in self.pumps.iter().enumerate() {
                let Some(pump) = pump_opt else { continue };
                for frame in pump.snapshot() {
                    let key = WindowKey {
                        backend: i,
                        window: frame.window.id,
                    };
                    let mode = modes.get(&key).copied().unwrap_or(WindowMode::Floating);
                    let target_rect = match mode {
                        WindowMode::Floating => frame.window.rect,
                        WindowMode::Tiled => layout.tiled_rect(key).unwrap_or(frame.window.rect),
                    };
                    let cols =
                        ((target_rect.width / self.cell.width_px as f32).ceil() as u16).max(1);
                    let rows =
                        ((target_rect.height / self.cell.height_px as f32).ceil() as u16).max(1);
                    let footprint = CellRect::new(
                        (target_rect.origin.0 / self.cell.width_px as f32) as u16,
                        (target_rect.origin.1 / self.cell.height_px as f32) as u16,
                        cols,
                        rows,
                    );
                    let rect = PxRect::new(0.0, 0.0, target_rect.width, target_rect.height);
                    let border = backend_color(i);
                    let bg = compositor_overlay_fill();
                    let mut layers = Vec::with_capacity(2);
                    if let Some(cap) = frame.capture {
                        let png = kittui_render_cpu::encode_png(&{
                            let mut p = kittui_render_cpu::Pixmap::new(cap.width, cap.height);
                            p.data_mut().copy_from_slice(&cap.rgba);
                            p
                        });
                        layers.push(Layer::anon(Node::Image {
                            rect,
                            src: kittui_core::node::ImageRef::Bytes { bytes: png },
                            fit: kittui_core::node::Fit::Stretch,
                            tint: None,
                        }));
                    }
                    layers.push(Layer::anon(Node::Rect {
                        rect,
                        fill: Paint::Solid { color: bg },
                        stroke: Some(kittui_core::node::Stroke {
                            align: kittui_core::node::StrokeAlign::Inside,
                            width_px: 1.5,
                            paint: Paint::Solid { color: border },
                        }),
                        corners: Corners::uniform(4.0),
                    }));
                    out.push((
                        key,
                        Scene {
                            footprint,
                            cell_size: self.cell,
                            layers,
                            animation: None,
                        },
                    ));
                }
            }
            out
        }

        /// Hit-test in cell-space, returning the topmost window across all backends.
        pub fn hit_test(&self, col: u16, row: u16) -> Option<WindowKey> {
            // Later backends z-stack above earlier ones; later windows within
            // a backend z-stack above earlier ones.
            for (i, server) in self.backends.iter().enumerate().rev() {
                if let Ok(windows) = server.windows() {
                    for w in windows.iter().rev() {
                        let px = (col as f32) * self.cell.width_px as f32;
                        let py = (row as f32) * self.cell.height_px as f32;
                        if px >= w.rect.origin.0
                            && px < w.rect.origin.0 + w.rect.width
                            && py >= w.rect.origin.1
                            && py < w.rect.origin.1 + w.rect.height
                        {
                            return Some(WindowKey {
                                backend: i,
                                window: w.id,
                            });
                        }
                    }
                }
            }
            None
        }

        /// Route a parsed pointer event to the topmost window's owning backend.
        pub fn route_pointer(&self, ev: &InputEvent) -> Vec<(WindowKey, XPointerEvent)> {
            let (col, row, button) = match ev {
                InputEvent::MousePress {
                    col, row, button, ..
                }
                | InputEvent::MouseRelease {
                    col, row, button, ..
                }
                | InputEvent::MouseMove {
                    col, row, button, ..
                } => (*col, *row, *button),
                _ => return Vec::new(),
            };
            let Some(key) = self.hit_test(col, row) else {
                return Vec::new();
            };
            *self.focused.lock() = Some(key);
            let server = &self.backends[key.backend];
            let Ok(windows) = server.windows() else {
                return Vec::new();
            };
            let Some(win) = windows.iter().find(|w| w.id == key.window) else {
                return Vec::new();
            };
            let local_x = ((col as f32) * self.cell.width_px as f32 - win.rect.origin.0) as i32;
            let local_y = ((row as f32) * self.cell.height_px as f32 - win.rect.origin.1) as i32;
            let mut routed = Vec::new();
            let move_ev = XPointerEvent::Move {
                window: key.window,
                x_px: local_x,
                y_px: local_y,
            };
            let _ = server.inject_pointer(move_ev);
            routed.push((key, move_ev));
            if let Some(xbtn) = button_to_x(button) {
                let click_ev = match ev {
                    InputEvent::MousePress { .. } => XPointerEvent::Press {
                        window: key.window,
                        button: xbtn,
                    },
                    InputEvent::MouseRelease { .. } => XPointerEvent::Release {
                        window: key.window,
                        button: xbtn,
                    },
                    _ => return routed,
                };
                let _ = server.inject_pointer(click_ev);
                routed.push((key, click_ev));
            }
            routed
        }

        /// Route a key event to the focused window's owning backend.
        pub fn route_key(&self, ev: &InputEvent) -> Option<(WindowKey, u32, bool)> {
            let key = (*self.focused.lock())?;
            let sym = match ev {
                InputEvent::Char { ch, .. } => *ch as u32,
                _ => return None,
            };
            let pressed = true;
            let _ = self.backends[key.backend].inject_key(sym, pressed);
            Some((key, sym, pressed))
        }
    }

    fn button_to_x(b: MouseButton) -> Option<XButton> {
        Some(match b {
            MouseButton::Left => XButton::Left,
            MouseButton::Middle => XButton::Middle,
            MouseButton::Right => XButton::Right,
            MouseButton::ScrollUp => XButton::ScrollUp,
            MouseButton::ScrollDown => XButton::ScrollDown,
            MouseButton::None | MouseButton::Other(_) => return None,
        })
    }

    fn compositor_overlay_fill() -> Rgba {
        InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Metal).fill
    }

    /// Per-backend accent colour so the user can visually tell which window
    /// came from which source. Uses the shared kittui-affordances accent cycle.
    fn backend_color(idx: usize) -> Rgba {
        InlineAccentPalette::resolve(InlineTheme::Nord).color(idx)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use kittui_xvfb::FakeServer;

        fn server_a() -> FakeServer {
            FakeServer::with_windows(vec![(
                XWindowId(1),
                PxRect::new(0.0, 0.0, 64.0, 32.0),
                "alpha",
                [0xff, 0x00, 0x00, 0xff],
            )])
        }
        fn server_b() -> FakeServer {
            FakeServer::with_windows(vec![(
                XWindowId(2),
                PxRect::new(80.0, 16.0, 64.0, 32.0),
                "beta",
                [0x00, 0xff, 0x00, 0xff],
            )])
        }

        #[test]
        fn backend_color_uses_shared_accent_palette_and_cycles() {
            let palette = InlineAccentPalette::resolve(InlineTheme::Nord);
            assert_eq!(backend_color(0), palette.color(0));
            assert_eq!(backend_color(1), palette.color(1));
            assert_eq!(backend_color(palette.colors().len()), palette.color(0));
        }

        #[test]
        fn multi_compositor_overlay_fill_uses_shared_inline_tokens() {
            let colors = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Metal);
            assert_eq!(compositor_overlay_fill(), colors.fill);
        }

        #[test]
        fn attach_multiple_backends_and_compose() {
            let mut comp = MultiCompositor::new(CellSize::new(8, 16));
            let a = comp.attach(Box::new(server_a()));
            let b = comp.attach(Box::new(server_b()));
            assert_eq!(a, 0);
            assert_eq!(b, 1);
            let scenes = comp.compose(&Layout::empty()).unwrap();
            assert_eq!(scenes.len(), 2);
            assert_eq!(scenes[0].0.backend, 0);
            assert_eq!(scenes[1].0.backend, 1);
        }

        #[test]
        fn hit_test_resolves_topmost_across_backends() {
            let mut comp = MultiCompositor::new(CellSize::new(8, 16));
            comp.attach(Box::new(server_a()));
            comp.attach(Box::new(server_b()));
            // Pixel (90, 20) -> cell (11, 1) lives in server_b's window.
            let key = comp.hit_test(11, 1).expect("hit");
            assert_eq!(key.backend, 1);
            assert_eq!(key.window, XWindowId(2));
            // Pixel (10, 10) -> cell (1, 0) lives in server_a's window.
            let key = comp.hit_test(1, 0).expect("hit");
            assert_eq!(key.backend, 0);
            assert_eq!(key.window, XWindowId(1));
        }

        #[test]
        fn route_pointer_injects_into_correct_backend() {
            let mut comp = MultiCompositor::new(CellSize::new(8, 16));
            comp.attach(Box::new(server_a()));
            comp.attach(Box::new(server_b()));
            let routed = comp.route_pointer(&InputEvent::MousePress {
                button: MouseButton::Left,
                col: 11,
                row: 1,
                mods: Default::default(),
            });
            assert_eq!(routed.len(), 2);
            assert_eq!(routed[0].0.backend, 1);
            assert!(matches!(routed[0].1, XPointerEvent::Move { .. }));
            assert!(matches!(routed[1].1, XPointerEvent::Press { .. }));
        }

        // ---- Pump tests ----------------------------------------------

        struct SlowFakeServer {
            inner: FakeServer,
            delay: std::time::Duration,
        }

        impl XServer for SlowFakeServer {
            fn windows(&self) -> Result<Vec<kittui_xvfb::XWindow>, XError> {
                self.inner.windows()
            }
            fn capture(&self, id: XWindowId) -> Result<kittui_xvfb::XCapture, XError> {
                std::thread::sleep(self.delay);
                self.inner.capture(id)
            }
            fn resize_window(&self, id: XWindowId, width: u32, height: u32) -> Result<(), XError> {
                self.inner.resize_window(id, width, height)
            }
            fn inject_pointer(&self, ev: XPointerEvent) -> Result<(), XError> {
                self.inner.inject_pointer(ev)
            }
            fn inject_key(&self, sym: u32, pressed: bool) -> Result<(), XError> {
                self.inner.inject_key(sym, pressed)
            }
        }

        #[test]
        fn pump_returns_frames_asynchronously() {
            let server = std::sync::Arc::new(server_a());
            let pump = Pump::spawn(server);
            // Poll up to 1s for the first frame to arrive.
            let mut frame_seen = false;
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                let snap = pump.snapshot();
                if !snap.is_empty() && snap[0].capture.is_some() {
                    frame_seen = true;
                    break;
                }
            }
            assert!(frame_seen, "pump never produced a frame");
        }

        #[test]
        fn pump_does_not_block_compose_via_pumps() {
            // Slow backend: each capture takes 500ms. With a 33ms UI frame
            // budget, sync compose() would blow it; compose_via_pumps()
            // must return effectively instantly because it only reads the
            // pump's cached slot.
            let slow = std::sync::Arc::new(SlowFakeServer {
                inner: server_a(),
                delay: std::time::Duration::from_millis(500),
            });
            let fast = std::sync::Arc::new(server_b());
            let mut comp = MultiCompositor::new(CellSize::new(8, 16));
            comp.attach_pump(slow);
            comp.attach_pump(fast);

            // Let the pumps warm up so each has at least one frame.
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                let scenes = comp.compose_via_pumps(&Layout::empty());
                if scenes.len() == 2
                    && scenes.iter().all(|(_, s)| {
                        s.layers
                            .iter()
                            .any(|l| matches!(l.root, Node::Image { .. }))
                    })
                {
                    break;
                }
            }

            // Now measure: every compose_via_pumps call must finish well
            // under the slow capture's 500ms latency.
            let start = std::time::Instant::now();
            for _ in 0..10 {
                let _ = comp.compose_via_pumps(&Layout::empty());
            }
            let elapsed = start.elapsed();
            assert!(
                elapsed < std::time::Duration::from_millis(100),
                "compose_via_pumps blocked on slow backend: {elapsed:?}"
            );
        }
    }
}
