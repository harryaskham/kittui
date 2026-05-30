//! General `Scene` builder helpers exposed by the facade. These are still
//! primitive-level (rect, gradient, glow, scanlines). Affordance helpers
//! such as "panel" or "chip" live in consumers (the CLI, the showcase
//! example, ratakittui), where users are free to compose them.

pub use kittui_cache::default_cache_dir;

use kittui_core::{
    color::Rgba,
    geom::{CellRect, CellSize, PxRect},
    node::{Corners, Direction, Layer, Node, Stop, Stroke, StrokeAlign},
    paint::Paint,
    Scene,
};

/// Construct a scene by passing a list of layers.
pub fn scene(footprint: CellRect, cell_size: CellSize, layers: Vec<Layer>) -> Scene {
    Scene {
        footprint,
        cell_size,
        layers,
        animation: None,
    }
}

/// A solid-color background layer that fills the entire scene footprint.
pub fn background_solid(footprint: CellRect, cell_size: CellSize, color: Rgba) -> Layer {
    Layer::new(
        "background",
        Node::Rect {
            rect: footprint.to_pixels(cell_size),
            fill: Paint::Solid { color },
            stroke: None,
            corners: Corners::default(),
        },
    )
}

/// A two-stop linear gradient layer covering the scene footprint.
pub fn background_linear(
    footprint: CellRect,
    cell_size: CellSize,
    direction: Direction,
    start: Rgba,
    end: Rgba,
) -> Layer {
    Layer::new(
        "background",
        Node::Gradient {
            rect: footprint.to_pixels(cell_size),
            stops: vec![
                Stop {
                    offset: 0.0,
                    color: start,
                },
                Stop {
                    offset: 1.0,
                    color: end,
                },
            ],
            direction,
        },
    )
}

/// A bordered rounded-rect layer. Used by every higher-level "panel" or
/// "box" affordance the CLI and showcase compose.
pub fn rounded_rect(
    rect: PxRect,
    fill: Rgba,
    stroke_color: Rgba,
    stroke_width_px: f32,
    corner_radius: f32,
) -> Layer {
    Layer::new(
        "rounded_rect",
        Node::Rect {
            rect,
            fill: Paint::Solid { color: fill },
            stroke: Some(Stroke {
                align: StrokeAlign::Inside,
                width_px: stroke_width_px,
                paint: Paint::Solid {
                    color: stroke_color,
                },
            }),
            corners: Corners::uniform(corner_radius),
        },
    )
}

/// A radial glow layer centred inside its rectangle.
pub fn glow_layer(rect: PxRect, color: Rgba, intensity: f32) -> Layer {
    Layer::new(
        "glow",
        Node::Glow {
            rect,
            center_x_frac: 0.5,
            center_y_frac: 0.5,
            radius_frac: 0.5,
            color,
            intensity,
        },
    )
}

/// Convenience builders used by tests and the CLI's smoke commands. These
/// are deliberately tiny and don't accumulate styling options — full
/// affordance composition lives in the CLI and showcase consumer code.
pub mod builders {
    use super::*;

    /// Construct a `cols × rows` solid-color box scene.
    pub fn simple_solid_box(cols: u16, rows: u16, color: &str) -> Scene {
        let cell = CellSize::default();
        let footprint = CellRect::new(0, 0, cols, rows);
        let bg = background_solid(footprint, cell, Rgba::parse(color).unwrap_or_default());
        scene(footprint, cell, vec![bg])
    }
}
