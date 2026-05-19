//! kittui-render-cpu
//!
//! The reference CPU rasterizer for kittui scenes. This crate is the
//! correctness oracle: golden snapshots live here and the GPU renderer is
//! diff-tested against it. It is intentionally allocation-light in the hot
//! path: a single `Pixmap` is reused per render call and tile work is
//! parallelized via `rayon`.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

mod pixmap;
mod png;
mod rasterize;

pub use pixmap::Pixmap;
pub use png::{encode_apng, encode_png};

use kittui_core::{Animation, Scene};

/// Errors surfaced by the CPU renderer.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// Animation present but its phase curve does not close the loop.
    #[error("animation phase curve does not close the loop (start phase != end phase)")]
    AnimationDoesNotLoop,
    /// Image source cannot currently be resolved by the CPU renderer.
    #[error("image source {0} is not supported by the CPU renderer in this version")]
    UnsupportedImage(String),
}

/// A still raster produced from a non-animated scene.
pub struct RasterFrame {
    /// PNG-encoded bytes ready to upload via kitty graphics.
    pub png: Vec<u8>,
    /// Width in pixels.
    pub width_px: u32,
    /// Height in pixels.
    pub height_px: u32,
}

/// An animated raster produced from a scene with an `Animation`. Each frame
/// is encoded as a standalone PNG; the kitty layer uploads them once and
/// uses kitty's native animation control to loop.
pub struct RasterAnimation {
    /// Per-frame PNGs.
    pub frames: Vec<Vec<u8>>,
    /// Per-frame delay in milliseconds.
    pub frame_delays_ms: Vec<u32>,
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// Number of times to play. `0` means loop forever.
    pub loops: u32,
}

/// Render a scene into a [`RasterFrame`]. Ignores any animation descriptor.
pub fn render_still(scene: &Scene) -> Result<RasterFrame, RenderError> {
    let mut pixmap = Pixmap::new(scene.pixel_width(), scene.pixel_height());
    rasterize::render_scene(scene, 0.0, &mut pixmap)?;
    let png = encode_png(&pixmap);
    Ok(RasterFrame {
        png,
        width_px: pixmap.width(),
        height_px: pixmap.height(),
    })
}

/// Render every frame of an animated scene. Each frame uses the same
/// `Pixmap` storage between renders.
pub fn render_animation(scene: &Scene) -> Result<RasterAnimation, RenderError> {
    let animation = scene.animation.as_ref().cloned().unwrap_or_else(|| {
        Animation::pulse(2, 0) // safe placeholder; never returned
    });
    if !animation.curve.closes_loop() {
        return Err(RenderError::AnimationDoesNotLoop);
    }
    let phases = animation.phases();
    let mut frames = Vec::with_capacity(phases.len());
    let mut pixmap = Pixmap::new(scene.pixel_width(), scene.pixel_height());
    for phase in phases {
        pixmap.clear();
        rasterize::render_scene(scene, phase, &mut pixmap)?;
        frames.push(encode_png(&pixmap));
    }
    let delays: Vec<u32> = (0..animation.frames).map(|i| animation.delay_ms(i)).collect();
    Ok(RasterAnimation {
        frames,
        frame_delays_ms: delays,
        width_px: pixmap.width(),
        height_px: pixmap.height(),
        loops: animation.loops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui_core::color::Rgba;
    use kittui_core::geom::{CellRect, CellSize, PxRect};
    use kittui_core::node::{Corners, Layer, Node};
    use kittui_core::paint::Paint;

    fn solid_scene() -> Scene {
        Scene {
            footprint: CellRect::new(0, 0, 4, 2),
            cell_size: CellSize::new(8, 16),
            layers: vec![Layer::anon(Node::Rect {
                rect: PxRect::new(0.0, 0.0, 32.0, 32.0),
                fill: Paint::Solid {
                    color: Rgba::rgb(0x00, 0xd8, 0xff),
                },
                stroke: None,
                corners: Corners::default(),
            })],
            animation: None,
        }
    }

    #[test]
    fn render_still_produces_png_signature() {
        let frame = render_still(&solid_scene()).unwrap();
        assert_eq!(&frame.png[..8], &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
        assert_eq!(frame.width_px, 32);
        assert_eq!(frame.height_px, 32);
    }

    #[test]
    fn render_still_is_byte_stable() {
        let a = render_still(&solid_scene()).unwrap();
        let b = render_still(&solid_scene()).unwrap();
        assert_eq!(a.png, b.png);
    }
}
