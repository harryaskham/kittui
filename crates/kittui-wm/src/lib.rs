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

fn layout_node(
    node: &LayoutNode,
    rect: CellRect,
    z_base: u16,
    out: &mut Vec<WindowGeometry>,
) {
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
                let len: u16 = if is_last {
                    (along - consumed) as u16
                } else {
                    let slice = ((weight / total) * along as f32).round() as u32;
                    consumed = consumed.saturating_add(slice);
                    slice as u16
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

/// Compositor that turns Xvfb-backed `XServer` windows into placed kittui
/// scenes, routes pointer events back to the X server, and tracks per-window
/// chrome through a `LifecycleTracker`-compatible delete pass.
pub mod compositor {
    use std::collections::HashMap;

    use kittui::{CellRect, CellSize, Rgba, Scene};
    use kittui_core::geom::PxRect;
    use kittui_core::node::{Corners, Layer, Node};
    use kittui_core::paint::Paint;
    use kittui_input::{InputEvent, MouseButton};
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
    }

    impl<S: XServer> Compositor<S> {
        /// Construct a compositor over `server` with the given terminal cell metric.
        pub fn new(server: S, cell: CellSize) -> Self {
            Self {
                server,
                cell,
                focused: Mutex::new(None),
                modes: Mutex::new(HashMap::new()),
            }
        }

        /// Mark a window as floating or tiled.
        pub fn set_mode(&self, id: XWindowId, mode: WindowMode) {
            self.modes.lock().insert(id, mode);
        }

        /// Borrow the underlying X server for direct access (advanced use).
        pub fn server(&self) -> &S {
            &self.server
        }

        /// Build one kittui Scene per X window, with simple border chrome.
        pub fn compose(&self) -> Result<Vec<Scene>, kittui_xvfb::XError> {
            self.compose_with_layout(&Layout::all_floating())
        }

        /// Build scenes using an explicit [`Layout`]. Tiled windows use the
        /// `tiled_rect` slot in the layout; floating windows keep their
        /// X-server-provided pixel rect.
        pub fn compose_with_layout(
            &self,
            layout: &Layout,
        ) -> Result<Vec<Scene>, kittui_xvfb::XError> {
            let windows = self.server.windows()?;
            let modes = self.modes.lock().clone();
            let mut out = Vec::with_capacity(windows.len());
            for w in &windows {
                let mode = modes.get(&w.id).copied().unwrap_or(WindowMode::Floating);
                let target_rect = match mode {
                    WindowMode::Floating => w.rect,
                    WindowMode::Tiled => layout.tiled_rect(w.id).unwrap_or(w.rect),
                };
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
                let rect = PxRect::new(0.0, 0.0, target_rect.width, target_rect.height);
                let border = Rgba::parse("#00d8ff").unwrap();
                let bg = Rgba::parse("#00000080").unwrap();
                let png = encode_rgba(&cap.rgba, cap.width, cap.height);
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
                out.push(Scene {
                    footprint,
                    cell_size: self.cell,
                    layers,
                    animation: None,
                });
            }
            Ok(out)
        }

        /// Walk the windows top-down and return the window id at `(col, row)`.
        pub fn hit_test(&self, col: u16, row: u16) -> Option<XWindowId> {
            let windows = self.server.windows().ok()?;
            // Iterate in reverse so later windows (drawn on top) win.
            for w in windows.iter().rev() {
                if hit(&w.rect, &self.cell, col, row) {
                    return Some(w.id);
                }
            }
            None
        }

        /// Translate a kittui-input event into one or more `XPointerEvent`s
        /// and inject them into the server. Returns the events injected.
        pub fn route_pointer(&self, ev: &InputEvent) -> Vec<XPointerEvent> {
            let mut routed = Vec::new();
            match ev {
                InputEvent::MousePress { col, row, button, .. }
                | InputEvent::MouseRelease { col, row, button, .. }
                | InputEvent::MouseMove { col, row, button, .. } => {
                    let Some(id) = self.hit_test(*col, *row) else {
                        return routed;
                    };
                    *self.focused.lock() = Some(id);
                    let Ok(windows) = self.server.windows() else {
                        return routed;
                    };
                    let Some(window) = windows.iter().find(|w| w.id == id) else {
                        return routed;
                    };
                    let (lx, ly) = local_px(*col, *row, &self.cell, &window.rect);
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

        /// Look up the tiled slot for a window, if any.
        pub fn tiled_rect(&self, id: XWindowId) -> Option<PxRect> {
            self.tiled.get(&id).copied()
        }
    }

    fn hit(rect: &PxRect, cell: &CellSize, col: u16, row: u16) -> bool {
        let px = (col as f32) * cell.width_px as f32;
        let py = (row as f32) * cell.height_px as f32;
        px >= rect.origin.0
            && px < rect.origin.0 + rect.width
            && py >= rect.origin.1
            && py < rect.origin.1 + rect.height
    }

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
            // beta sits at px (80,16) → cell (10, 1).
            assert_eq!(comp.hit_test(11, 1), Some(XWindowId(2)));
            assert_eq!(comp.hit_test(1, 1), Some(XWindowId(1)));
            assert_eq!(comp.hit_test(50, 50), None);
        }

        #[test]
        fn route_pointer_injects_move_then_press() {
            let comp = Compositor::new(server(), CellSize::new(8, 16));
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
    use std::collections::HashMap;

    use kittui::{CellRect, CellSize, Rgba, Scene};
    use kittui_core::geom::PxRect;
    use kittui_core::node::{Corners, Layer, Node};
    use kittui_core::paint::Paint;
    use kittui_input::{InputEvent, MouseButton};
    use kittui_xvfb::{XButton, XError, XPointerEvent, XServer, XWindowId};
    use parking_lot::Mutex;

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

    /// Multi-backend compositor. Cheap to clone (interior mutability).
    pub struct MultiCompositor {
        backends: Vec<Box<dyn XServer + Send + Sync>>,
        cell: CellSize,
        modes: Mutex<HashMap<WindowKey, WindowMode>>,
        focused: Mutex<Option<WindowKey>>,
    }

    impl MultiCompositor {
        /// Construct a compositor with no backends attached. Use [`attach`].
        pub fn new(cell: CellSize) -> Self {
            Self {
                backends: Vec::new(),
                cell,
                modes: Mutex::new(HashMap::new()),
                focused: Mutex::new(None),
            }
        }

        /// Attach a backend; returns its index.
        pub fn attach(&mut self, server: Box<dyn XServer + Send + Sync>) -> usize {
            self.backends.push(server);
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
                    let cols = ((target_rect.width / self.cell.width_px as f32).ceil() as u16)
                        .max(1);
                    let rows = ((target_rect.height / self.cell.height_px as f32).ceil() as u16)
                        .max(1);
                    let footprint = CellRect::new(
                        (target_rect.origin.0 / self.cell.width_px as f32) as u16,
                        (target_rect.origin.1 / self.cell.height_px as f32) as u16,
                        cols,
                        rows,
                    );
                    let rect = PxRect::new(0.0, 0.0, target_rect.width, target_rect.height);
                    let border = backend_color(i);
                    let bg = Rgba::parse("#00000080").unwrap();
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
                InputEvent::MousePress { col, row, button, .. }
                | InputEvent::MouseRelease { col, row, button, .. }
                | InputEvent::MouseMove { col, row, button, .. } => (*col, *row, *button),
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

    /// Per-backend accent colour so the user can visually tell which window
    /// came from which source. Cyan / violet / lime / amber / rose, cycling.
    fn backend_color(idx: usize) -> Rgba {
        const PALETTE: &[&str] = &[
            "#00d8ff", "#b48cff", "#c0ff5a", "#ffa44d", "#ff5e8e", "#72fbd6",
        ];
        Rgba::parse(PALETTE[idx % PALETTE.len()]).unwrap()
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
    }
}
