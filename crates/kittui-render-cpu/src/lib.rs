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

    fn pixmap_from(scene: &Scene) -> crate::Pixmap {
        let mut p = crate::Pixmap::new(scene.pixel_width(), scene.pixel_height());
        crate::rasterize::render_scene(scene, 0.0, &mut p).unwrap();
        p
    }

    fn red_rect(rect: PxRect) -> Node {
        Node::Rect {
            rect,
            fill: Paint::Solid {
                color: Rgba::rgba(255, 0, 0, 255),
            },
            stroke: None,
            corners: Corners::default(),
        }
    }

    #[test]
    fn clip_clamps_drawing_to_rect() {
        let footprint = CellRect::new(0, 0, 4, 2);
        let scene = Scene {
            footprint,
            cell_size: CellSize::new(8, 16),
            layers: vec![Layer::anon(Node::Clip {
                rect: PxRect::new(0.0, 0.0, 8.0, 8.0),
                child: Box::new(red_rect(PxRect::new(0.0, 0.0, 32.0, 32.0))),
            })],
            animation: None,
        };
        let p = pixmap_from(&scene);
        // Inside the clip rect: red, opaque.
        let c = p.get(2, 2);
        assert_eq!((c.0, c.3), (255, 255));
        // Outside the clip rect: transparent.
        let c = p.get(20, 20);
        assert_eq!(c.3, 0);
    }

    #[test]
    fn mask_alpha_attenuates_child() {
        let footprint = CellRect::new(0, 0, 4, 2);
        let mask = Node::Rect {
            rect: PxRect::new(0.0, 0.0, 32.0, 32.0),
            fill: Paint::Solid {
                // Half-alpha mask.
                color: Rgba::rgba(255, 255, 255, 128),
            },
            stroke: None,
            corners: Corners::default(),
        };
        let scene = Scene {
            footprint,
            cell_size: CellSize::new(8, 16),
            layers: vec![Layer::anon(Node::Mask {
                mask: Box::new(mask),
                child: Box::new(red_rect(PxRect::new(0.0, 0.0, 32.0, 32.0))),
            })],
            animation: None,
        };
        let p = pixmap_from(&scene);
        let c = p.get(4, 4);
        // ~ 128/255 of opaque red.
        assert!(c.3 > 100 && c.3 < 160, "alpha {} should be ~128", c.3);
        assert_eq!(c.0, 255);
    }

    #[test]
    fn composite_add_brightens_overlap() {
        let footprint = CellRect::new(0, 0, 4, 2);
        let scene = Scene {
            footprint,
            cell_size: CellSize::new(8, 16),
            layers: vec![Layer::anon(Node::Composite {
                mode: kittui_core::node::BlendMode::Add,
                children: vec![
                    Node::Rect {
                        rect: PxRect::new(0.0, 0.0, 32.0, 32.0),
                        fill: Paint::Solid {
                            color: Rgba::rgba(100, 0, 0, 255),
                        },
                        stroke: None,
                        corners: Corners::default(),
                    },
                    Node::Rect {
                        rect: PxRect::new(0.0, 0.0, 32.0, 32.0),
                        fill: Paint::Solid {
                            color: Rgba::rgba(0, 100, 0, 255),
                        },
                        stroke: None,
                        corners: Corners::default(),
                    },
                ],
            })],
            animation: None,
        };
        let p = pixmap_from(&scene);
        let c = p.get(8, 8);
        // Both channels present after additive blend onto transparent backdrop.
        assert!(c.0 > 50);
        assert!(c.1 > 50);
    }
}

#[cfg(test)]
#[cfg(feature = "image-decoders")]
mod image_tests {
    use super::*;
    use kittui_core::geom::{CellRect, CellSize, PxRect};
    use kittui_core::node::{Fit, ImageRef, Layer, Node};

    fn tiny_png() -> Vec<u8> {
        // Build a 2x2 RGBA PNG via the image crate (test-only).
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        let mut out = Vec::new();
        let enc = image::codecs::png::PngEncoder::new(&mut out);
        use image::ImageEncoder;
        enc.write_image(img.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn image_node_rasterizes_inline_bytes() {
        let png = tiny_png();
        let footprint = CellRect::new(0, 0, 4, 2);
        let scene = Scene {
            footprint,
            cell_size: CellSize::new(8, 16),
            layers: vec![Layer::anon(Node::Image {
                rect: PxRect::new(0.0, 0.0, 32.0, 32.0),
                src: ImageRef::Bytes { bytes: png },
                fit: Fit::Stretch,
                tint: None,
            })],
            animation: None,
        };
        let mut p = crate::Pixmap::new(scene.pixel_width(), scene.pixel_height());
        crate::rasterize::render_scene(&scene, 0.0, &mut p).unwrap();
        // Sample the four quadrants and verify each picked up the right pixel.
        let c_tl = p.get(2, 2);
        let c_tr = p.get(20, 2);
        let c_bl = p.get(2, 20);
        let c_br = p.get(20, 20);
        assert!(c_tl.0 > 200 && c_tl.1 < 50, "top-left ~red: {c_tl:?}");
        assert!(c_tr.1 > 200 && c_tr.0 < 50, "top-right ~green: {c_tr:?}");
        assert!(c_bl.2 > 200, "bottom-left ~blue: {c_bl:?}");
        assert!(c_br.0 > 200 && c_br.1 > 200 && c_br.2 > 200, "bottom-right ~white: {c_br:?}");
    }
}
