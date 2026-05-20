//! Scene container and stable identity.

use serde::{Deserialize, Serialize};

use crate::animation::Animation;
use crate::color::Rgba;
use crate::geom::{CellRect, CellSize, PxRect};
use crate::hash;
use crate::node::{BlendMode, Layer, Node, Stop, Stroke};
use crate::paint::{LinearGradient, Paint, RadialGradient};

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
    /// for the kitty image id. Hashes the deterministic normalized scene so
    /// renderer-equivalent inputs share cache entries and kitty image ids.
    pub fn id(&self) -> SceneId {
        SceneId(hash::blake3_of_serializable(&self.normalized()))
    }

    /// Return the deterministic render-equivalent form used for identity.
    /// This keeps the serde wire shape stable while avoiding cache misses from
    /// no-op layers, floating-point jitter, and gradient stop ordering noise.
    pub fn normalized(&self) -> Self {
        Self {
            footprint: self.footprint,
            cell_size: self.cell_size,
            layers: self.layers.iter().filter_map(normalize_layer).collect(),
            animation: self.animation.clone(),
        }
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

fn normalize_layer(layer: &Layer) -> Option<Layer> {
    normalize_node(&layer.root).map(|root| Layer { label: None, root })
}

fn normalize_node(node: &Node) -> Option<Node> {
    match node {
        Node::Rect {
            rect,
            fill,
            stroke,
            corners,
        } => {
            let rect = normalize_rect(*rect)?;
            let fill = normalize_paint(fill.clone());
            let stroke = stroke.as_ref().and_then(normalize_stroke);
            if is_transparent_solid(&fill) && stroke.is_none() {
                return None;
            }
            Some(Node::Rect {
                rect,
                fill,
                stroke,
                corners: normalize_corners(*corners),
            })
        }
        Node::Gradient {
            rect,
            stops,
            direction,
        } => Some(Node::Gradient {
            rect: normalize_rect(*rect)?,
            stops: normalize_stops(stops),
            direction: *direction,
        }),
        Node::Glow {
            rect,
            center_x_frac,
            center_y_frac,
            radius_frac,
            color,
            intensity,
        } => {
            let intensity = intensity.clamp(0.0, 1.0);
            let color = *color;
            if intensity == 0.0 || color.3 == 0 {
                return None;
            }
            Some(Node::Glow {
                rect: normalize_rect(*rect)?,
                center_x_frac: center_x_frac.clamp(0.0, 1.0),
                center_y_frac: center_y_frac.clamp(0.0, 1.0),
                radius_frac: radius_frac.max(0.0),
                color,
                intensity,
            })
        }
        Node::Scanlines {
            rect,
            alpha,
            period_px,
        } => {
            if *alpha == 0 || *period_px == 0 {
                return None;
            }
            Some(Node::Scanlines {
                rect: normalize_rect(*rect)?,
                alpha: *alpha,
                period_px: *period_px,
            })
        }
        Node::Image {
            rect,
            src,
            fit,
            tint,
        } => Some(Node::Image {
            rect: normalize_rect(*rect)?,
            src: src.clone(),
            fit: *fit,
            tint: tint.filter(|c| c.3 != 0),
        }),
        Node::Group { opacity, children } => {
            let opacity = opacity.clamp(0.0, 1.0);
            if opacity == 0.0 {
                return None;
            }
            normalize_children(children).map(|children| Node::Group { opacity, children })
        }
        Node::Composite { mode, children } => {
            let children = normalize_children(children)?;
            if *mode == BlendMode::Normal && children.len() == 1 {
                return children.into_iter().next();
            }
            Some(Node::Composite {
                mode: *mode,
                children,
            })
        }
        Node::Mask { mask, child } => Some(Node::Mask {
            mask: Box::new(normalize_node(mask)?),
            child: Box::new(normalize_node(child)?),
        }),
        Node::Clip { rect, child } => Some(Node::Clip {
            rect: normalize_rect(*rect)?,
            child: Box::new(normalize_node(child)?),
        }),
        Node::Shader {
            rect,
            source,
            uniforms,
        } => Some(Node::Shader {
            rect: normalize_rect(*rect)?,
            source: source.clone(),
            uniforms: uniforms.clone(),
        }),
    }
}

fn normalize_children(children: &[Node]) -> Option<Vec<Node>> {
    let normalized: Vec<Node> = children.iter().filter_map(normalize_node).collect();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_rect(rect: PxRect) -> Option<PxRect> {
    let width = snap(rect.width.max(0.0));
    let height = snap(rect.height.max(0.0));
    if width == 0.0 || height == 0.0 {
        return None;
    }
    Some(PxRect::new(
        snap(rect.origin.0),
        snap(rect.origin.1),
        width,
        height,
    ))
}

fn normalize_corners(corners: crate::node::Corners) -> crate::node::Corners {
    crate::node::Corners {
        tl: snap(corners.tl.max(0.0)),
        tr: snap(corners.tr.max(0.0)),
        bl: snap(corners.bl.max(0.0)),
        br: snap(corners.br.max(0.0)),
    }
}

fn normalize_stroke(stroke: &Stroke) -> Option<Stroke> {
    let width_px = snap(stroke.width_px.max(0.0));
    if width_px == 0.0 {
        return None;
    }
    let paint = normalize_paint(stroke.paint.clone());
    (!is_transparent_solid(&paint)).then_some(Stroke {
        align: stroke.align,
        width_px,
        paint,
    })
}

fn normalize_paint(paint: Paint) -> Paint {
    match paint {
        Paint::Solid { color } => Paint::Solid { color },
        Paint::Linear(LinearGradient { direction, stops }) => Paint::Linear(LinearGradient {
            direction,
            stops: normalize_stops(&stops),
        }),
        Paint::Radial(RadialGradient {
            center_x_frac,
            center_y_frac,
            radius_frac,
            stops,
        }) => Paint::Radial(RadialGradient {
            center_x_frac: center_x_frac.clamp(0.0, 1.0),
            center_y_frac: center_y_frac.clamp(0.0, 1.0),
            radius_frac: radius_frac.max(0.0),
            stops: normalize_stops(&stops),
        }),
    }
}

fn normalize_stops(stops: &[Stop]) -> Vec<Stop> {
    let mut normalized: Vec<Stop> = stops
        .iter()
        .map(|stop| Stop {
            offset: snap(stop.offset.clamp(0.0, 1.0)),
            color: stop.color,
        })
        .collect();
    normalized.sort_by(|a, b| {
        a.offset
            .total_cmp(&b.offset)
            .then_with(|| rgba_key(a.color).cmp(&rgba_key(b.color)))
    });
    normalized
}

fn is_transparent_solid(paint: &Paint) -> bool {
    matches!(paint, Paint::Solid { color } if color.3 == 0)
}

fn rgba_key(color: Rgba) -> [u8; 4] {
    [color.0, color.1, color.2, color.3]
}

fn snap(value: f32) -> f32 {
    let snapped = (value * 64.0).round() / 64.0;
    if snapped == 0.0 {
        0.0
    } else {
        snapped
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
    use crate::node::{BlendMode, Corners, Direction, Layer, Node, Stop, Stroke, StrokeAlign};
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

    #[test]
    fn scene_id_ignores_debug_labels_and_empty_layers() {
        let mut a = sample_scene();
        a.layers[0].label = Some("debug label".into());
        a.layers.push(Layer::anon(Node::Group {
            opacity: 0.0,
            children: vec![Node::Rect {
                rect: PxRect::new(1.0, 1.0, 2.0, 2.0),
                fill: Paint::Solid {
                    color: Rgba::rgba(255, 0, 0, 255),
                },
                stroke: None,
                corners: Corners::default(),
            }],
        }));
        assert_eq!(a.id(), sample_scene().id());
    }

    #[test]
    fn scene_id_snaps_subpixel_noise_and_removes_zero_stroke() {
        let a = sample_scene();
        let mut b = sample_scene();
        let Node::Rect { rect, stroke, .. } = &mut b.layers[0].root else {
            panic!("sample rect");
        };
        rect.origin.0 += 0.001;
        rect.origin.1 -= 0.001;
        *stroke = Some(Stroke {
            align: StrokeAlign::Inside,
            width_px: 0.0,
            paint: Paint::Solid {
                color: Rgba::rgb(255, 255, 255),
            },
        });
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn scene_id_sorts_and_clamps_gradient_stops() {
        let cell = CellSize::default();
        let rect = CellRect::new(0, 0, 4, 2).to_pixels(cell);
        let a = Scene {
            footprint: CellRect::new(0, 0, 4, 2),
            cell_size: cell,
            layers: vec![Layer::anon(Node::Gradient {
                rect,
                stops: vec![
                    Stop {
                        offset: 0.0,
                        color: Rgba::rgb(0, 0, 0),
                    },
                    Stop {
                        offset: 1.0,
                        color: Rgba::rgb(255, 255, 255),
                    },
                ],
                direction: Direction::Horizontal,
            })],
            animation: None,
        };
        let mut b = a.clone();
        if let Node::Gradient { stops, .. } = &mut b.layers[0].root {
            stops.reverse();
            stops[0].offset = 2.0;
            stops[1].offset = -1.0;
        }
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn normal_composite_with_single_child_hashes_as_child() {
        let mut a = sample_scene();
        let child = a.layers[0].root.clone();
        a.layers[0].root = Node::Composite {
            mode: BlendMode::Normal,
            children: vec![child],
        };
        assert_eq!(a.id(), sample_scene().id());
    }
}
