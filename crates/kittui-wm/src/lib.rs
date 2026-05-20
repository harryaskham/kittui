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
