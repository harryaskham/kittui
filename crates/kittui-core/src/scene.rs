//! Scene container and stable identity.

use serde::{Deserialize, Serialize};

use crate::animation::Animation;
use crate::geom::{CellRect, CellSize};
use crate::hash;
use crate::node::Layer;

/// A scene is the unit of rasterization, caching, and placement. It is the
/// only shape that crosses the renderer / cache / protocol boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Cell footprint reserved for the rendered output.
    pub footprint: CellRect,
    /// Pixel-per-cell metric. The renderer uses this to size the raster.
    pub cell_size: CellSize,
    /// Back-to-front layer list.
    pub layers: Vec<Layer>,
    /// Optional animation descriptor. When present, the renderer produces N
    /// frames and the kitty layer uses the protocol's native animation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<Animation>,
}

impl Scene {
    /// Stable identity of the scene used as the cache key and as the basis
    /// for the kitty image id. Includes everything the renderer reads.
    pub fn id(&self) -> SceneId {
        SceneId(hash::blake3_of_serializable(self))
    }

    /// Pixel-space width of the rendered raster.
    pub fn pixel_width(&self) -> u32 {
        self.footprint.cols as u32 * self.cell_size.width_px as u32
    }

    /// Pixel-space height of the rendered raster.
    pub fn pixel_height(&self) -> u32 {
        self.footprint.rows as u32 * self.cell_size.height_px as u32
    }
}

/// Stable identifier derived from a [`Scene`] via blake3. Two scenes with the
/// same id render to identical bytes and share cache + kitty ids.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SceneId(pub String);

impl SceneId {
    /// First 8 hex characters of the id, for log lines and ratio image ids.
    pub fn short(&self) -> &str {
        &self.0[..self.0.len().min(8)]
    }

    /// Derive a 32-bit image id suitable for the kitty graphics protocol.
    /// We use the first 4 bytes of the underlying blake3 digest; collisions
    /// are bounded by the cache layer's content-addressed eviction.
    pub fn kitty_image_id(&self) -> u32 {
        let bytes = &self.0.as_bytes();
        let nibble = |i: usize| {
            let c = bytes[i] as char;
            c.to_digit(16).unwrap_or(0)
        };
        (0..8).fold(0u32, |acc, i| (acc << 4) | nibble(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;
    use crate::geom::PxRect;
    use crate::node::{Corners, Layer, Node};
    use crate::paint::Paint;

    fn sample_scene() -> Scene {
        Scene {
            footprint: CellRect::new(0, 0, 4, 2),
            cell_size: CellSize::default(),
            layers: vec![Layer::anon(Node::Rect {
                rect: PxRect::new(0.0, 0.0, 32.0, 32.0),
                fill: Paint::Solid {
                    color: Rgba::rgb(0, 0, 0),
                },
                stroke: None,
                corners: Corners::default(),
            })],
            animation: None,
        }
    }

    #[test]
    fn scene_id_is_stable_across_clones() {
        let a = sample_scene();
        let b = a.clone();
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn scene_id_changes_when_content_changes() {
        let mut a = sample_scene();
        let original = a.id();
        a.footprint.cols += 1;
        assert_ne!(a.id(), original);
    }
}
