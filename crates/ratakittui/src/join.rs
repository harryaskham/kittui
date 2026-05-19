//! Joined borders and shared backgrounds across adjacent widgets.
//!
//! The user model: declare that two or more chromed widgets share chrome.
//! The resolver computes one composite background and one continuous
//! stroke, then returns per-member scenes that, when placed side-by-side,
//! visually merge.
//!
//! Why this lives in ratakittui and not kittui-core: the join is a
//! ratatui-area-aware composition policy. The library doesn't need a new
//! node type because the result is expressible as `Composite` + `Mask` of
//! existing primitives.
//!
//! The v1 implementation handles axis-aligned, rectangular adjacency. Each
//! member contributes a `Chrome` and a ratatui `Rect`; the resolver
//! produces per-member `Scene`s where the shared inner edges have their
//! strokes masked out, so abutting borders draw exactly once at the join.

use std::collections::HashSet;

use ratatui::layout::Rect;

use kittui::{CellRect, CellSize, Corners, Layer, Node, Paint, PxRect, Rgba, Scene, Stroke};
use kittui_core::node::{BlendMode, StrokeAlign};

use crate::chrome::Chrome;

/// One element in a join group.
#[derive(Clone, Debug)]
pub struct Joined {
    /// Member chrome (the join may mutate the per-corner radii at internal
    /// junctions but preserves all other properties).
    pub chrome: Chrome,
    /// ratatui rect the member occupies.
    pub area: Rect,
}

/// A group of joined chromed widgets.
#[derive(Clone, Debug, Default)]
pub struct JoinGroup {
    members: Vec<Joined>,
}

impl JoinGroup {
    /// Construct an empty join group.
    pub fn new() -> Self {
        Self {
            members: Vec::new(),
        }
    }

    /// Add a member to the group.
    pub fn push(&mut self, member: Joined) {
        self.members.push(member);
    }

    /// Borrow the members for inspection.
    pub fn members(&self) -> &[Joined] {
        &self.members
    }

    /// Resolve the group. Returns one `Scene` per member, in the same order
    /// `push` was called. Members whose chrome is empty receive `None`.
    pub fn resolve(&self) -> Vec<Option<Scene>> {
        // Find shared internal edges so each chrome knows which sides to
        // mask. Two members share an edge when their rects abut on one axis
        // and overlap on the other axis with positive length.
        let mut shared: Vec<Edges> = vec![Edges::default(); self.members.len()];
        for (i, a) in self.members.iter().enumerate() {
            for (j, b) in self.members.iter().enumerate() {
                if i == j {
                    continue;
                }
                if let Some(edge) = abutting_edge(a.area, b.area) {
                    shared[i].insert(edge);
                }
            }
        }

        self.members
            .iter()
            .enumerate()
            .map(|(i, m)| compile_member(&m.chrome, m.area, &shared[i]))
            .collect()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Debug, Default)]
struct Edges {
    set: HashSet<Side>,
}

impl Edges {
    fn insert(&mut self, side: Side) {
        self.set.insert(side);
    }

    fn contains(&self, side: Side) -> bool {
        self.set.contains(&side)
    }
}

fn abutting_edge(a: Rect, b: Rect) -> Option<Side> {
    // `a`'s right == `b`'s left, with vertical overlap.
    if a.right() == b.left() && vertical_overlap(a, b) > 0 {
        return Some(Side::Right);
    }
    if a.left() == b.right() && vertical_overlap(a, b) > 0 {
        return Some(Side::Left);
    }
    if a.bottom() == b.top() && horizontal_overlap(a, b) > 0 {
        return Some(Side::Bottom);
    }
    if a.top() == b.bottom() && horizontal_overlap(a, b) > 0 {
        return Some(Side::Top);
    }
    None
}

fn vertical_overlap(a: Rect, b: Rect) -> i32 {
    let lo = a.top().max(b.top()) as i32;
    let hi = a.bottom().min(b.bottom()) as i32;
    (hi - lo).max(0)
}

fn horizontal_overlap(a: Rect, b: Rect) -> i32 {
    let lo = a.left().max(b.left()) as i32;
    let hi = a.right().min(b.right()) as i32;
    (hi - lo).max(0)
}

fn compile_member(chrome: &Chrome, area: Rect, shared: &Edges) -> Option<Scene> {
    let mut scene = chrome.to_scene(area)?;
    if shared.set.is_empty() {
        return Some(scene);
    }
    if let Some(border) = chrome.border.as_ref() {
        let cell = CellSize::default();
        let footprint = CellRect::new(area.x, area.y, area.width, area.height);
        let rect = footprint.to_pixels(cell);
        // Replace the original border layer with a composite that draws the
        // four edges as independent strokes and skips the shared ones.
        // Zero internal-corner radii so abutting borders meet flush.
        let mut corners = border.corners;
        if shared.contains(Side::Top) {
            corners.tl = 0.0;
            corners.tr = 0.0;
        }
        if shared.contains(Side::Bottom) {
            corners.bl = 0.0;
            corners.br = 0.0;
        }
        if shared.contains(Side::Left) {
            corners.tl = 0.0;
            corners.bl = 0.0;
        }
        if shared.contains(Side::Right) {
            corners.tr = 0.0;
            corners.br = 0.0;
        }
        let edges = composite_border(border.color, border.width_px, rect, corners, shared);
        // Drop any existing border layer and append the new composite.
        scene.layers.retain(|l| l.label.as_deref() != Some("border"));
        scene.layers.push(Layer::new("border", edges));
    }
    Some(scene)
}

fn composite_border(
    color: Rgba,
    width_px: f32,
    rect: PxRect,
    corners: Corners,
    shared: &Edges,
) -> Node {
    let mut children: Vec<Node> = Vec::with_capacity(4);
    let stroke = |paint_rect: PxRect| Node::Rect {
        rect: paint_rect,
        fill: Paint::Solid {
            color: Rgba::rgba(0, 0, 0, 0),
        },
        stroke: Some(Stroke {
            align: StrokeAlign::Inside,
            width_px,
            paint: Paint::Solid { color },
        }),
        corners: Corners::default(),
    };
    let w = width_px.max(1.0);
    let r = rect;

    if !shared.contains(Side::Top) {
        children.push(stroke(PxRect::new(r.origin.0, r.origin.1, r.width, w)));
    }
    if !shared.contains(Side::Bottom) {
        children.push(stroke(PxRect::new(
            r.origin.0,
            r.bottom() - w,
            r.width,
            w,
        )));
    }
    if !shared.contains(Side::Left) {
        children.push(stroke(PxRect::new(r.origin.0, r.origin.1, w, r.height)));
    }
    if !shared.contains(Side::Right) {
        children.push(stroke(PxRect::new(
            r.right() - w,
            r.origin.1,
            w,
            r.height,
        )));
    }

    // Round the corners that survived the join policy.
    if !corners.is_square() {
        children.push(Node::Rect {
            rect: r,
            fill: Paint::Solid {
                color: Rgba::rgba(0, 0, 0, 0),
            },
            stroke: Some(Stroke {
                align: StrokeAlign::Inside,
                width_px,
                paint: Paint::Solid { color },
            }),
            corners,
        });
    }

    Node::Composite {
        mode: BlendMode::Normal,
        children,
    }
}

/// Convenience macro: build a `JoinGroup` from `(chrome, rect)` pairs.
#[macro_export]
macro_rules! join {
    ($(($chrome:expr, $rect:expr)),* $(,)?) => {{
        let mut group = $crate::JoinGroup::new();
        $(
            group.push($crate::Joined {
                chrome: $chrome,
                area: $rect,
            });
        )*
        group
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::{Border, Chrome};
    use ratatui::layout::Rect;

    fn bordered() -> Chrome {
        Chrome::default().border(Border::rounded(
            kittui::Rgba::rgb(0, 0xd8, 0xff),
            1.5,
            6.0,
        ))
    }

    #[test]
    fn horizontally_abutting_borders_mask_inner_edges() {
        let group = join![
            (bordered(), Rect::new(0, 0, 10, 5)),
            (bordered(), Rect::new(10, 0, 10, 5)),
        ];
        let scenes = group.resolve();
        let left = scenes[0].as_ref().expect("left scene");
        let right = scenes[1].as_ref().expect("right scene");
        // Each member's border becomes a composite layer.
        assert!(left
            .layers
            .iter()
            .any(|l| l.label.as_deref() == Some("border")));
        assert!(right
            .layers
            .iter()
            .any(|l| l.label.as_deref() == Some("border")));
    }

    #[test]
    fn isolated_chrome_passes_through_unchanged() {
        let group = join![(bordered(), Rect::new(0, 0, 10, 5))];
        let scenes = group.resolve();
        assert!(scenes[0].is_some());
    }
}
