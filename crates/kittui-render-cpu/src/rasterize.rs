//! Scene → pixmap rasterizer. The renderer is straightforward: scan each
//! pixel inside a node's bounding rect, evaluate the node's shading function
//! at that pixel, and blend into the pixmap. The renderer is intentionally
//! conservative — its goal is correctness and reference parity, not raw
//! speed. The GPU backend is where high-fps rendering lives.

use std::fmt::Write as FmtWrite;

use kittui_core::color::Rgba;
use kittui_core::geom::{Px, PxRect};
use kittui_core::node::{BlendMode, Direction, Fit, ImageRef, Layer, Node, StrokeAlign};
use kittui_core::paint::{LinearGradient, Paint, RadialGradient};
use kittui_core::Scene;

use crate::pixmap::Pixmap;
use crate::RenderError;

/// Walk all layers and rasterize their roots into `pixmap`. `phase` is in
/// `[0,1]` and is forwarded to nodes that read animation phase (currently
/// only the glow intensity multiplier).
pub fn render_scene(scene: &Scene, phase: f32, pixmap: &mut Pixmap) -> Result<(), RenderError> {
    for layer in &scene.layers {
        render_layer(layer, phase, pixmap)?;
    }
    Ok(())
}

fn render_layer(layer: &Layer, phase: f32, pixmap: &mut Pixmap) -> Result<(), RenderError> {
    render_node(&layer.root, 1.0, BlendMode::Normal, phase, pixmap)
}

fn render_node(
    node: &Node,
    opacity: f32,
    _blend: BlendMode,
    phase: f32,
    pixmap: &mut Pixmap,
) -> Result<(), RenderError> {
    match node {
        Node::Rect {
            rect,
            fill,
            stroke,
            corners,
        } => {
            rasterize_rect(rect, fill, stroke.as_ref(), corners, opacity, pixmap);
            Ok(())
        }
        Node::Gradient {
            rect,
            stops,
            direction,
        } => {
            rasterize_gradient(rect, stops, *direction, opacity, pixmap);
            Ok(())
        }
        Node::Glow {
            rect,
            center_x_frac,
            center_y_frac,
            radius_frac,
            color,
            intensity,
        } => {
            // Intensity is modulated by phase so animated scenes can pulse
            // without re-uploading: each frame just samples this curve.
            let pulse = 0.5 + 0.5 * (phase * std::f32::consts::TAU).sin();
            let eff = (*intensity * pulse * opacity).clamp(0.0, 1.0);
            rasterize_glow(
                rect,
                *center_x_frac,
                *center_y_frac,
                *radius_frac,
                *color,
                eff,
                pixmap,
            );
            Ok(())
        }
        Node::Scanlines {
            rect,
            alpha,
            period_px,
        } => {
            rasterize_scanlines(rect, *alpha, *period_px, opacity, pixmap);
            Ok(())
        }
        Node::Image { rect, src, fit, tint } => {
            rasterize_image(rect, src, *fit, *tint, opacity, pixmap)
        }
        Node::Shader { .. } => Err(RenderError::UnsupportedImage(
            "Node::Shader is GPU-only; CPU WGSL execution lands later".to_owned(),
        )),
        Node::Group { opacity: o, children } => {
            let combined = (opacity * o.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            for child in children {
                render_node(child, combined, BlendMode::Normal, phase, pixmap)?;
            }
            Ok(())
        }
        Node::Composite { mode, children } => {
            if *mode == BlendMode::Normal {
                for child in children {
                    render_node(child, opacity, BlendMode::Normal, phase, pixmap)?;
                }
                return Ok(());
            }
            // Render each child into its own scratch, then combine onto the
            // main pixmap using the requested blend mode. The first child uses
            // Normal (over transparent), subsequent children use `mode`.
            for (i, child) in children.iter().enumerate() {
                let mut scratch = Pixmap::new(pixmap.width(), pixmap.height());
                render_node(child, opacity, BlendMode::Normal, phase, &mut scratch)?;
                let m = if i == 0 { BlendMode::Normal } else { *mode };
                for y in 0..pixmap.height() {
                    for x in 0..pixmap.width() {
                        let src = scratch.get(x, y);
                        if src.3 == 0 {
                            continue;
                        }
                        pixmap.blend_with(x, y, src, m);
                    }
                }
            }
            Ok(())
        }
        Node::Mask { mask, child } => {
            // Render mask + child into separate scratch pixmaps, then
            // multiply child alpha by mask alpha, blend into main.
            let mut mask_buf = Pixmap::new(pixmap.width(), pixmap.height());
            render_node(mask, 1.0, BlendMode::Normal, phase, &mut mask_buf)?;
            let mut child_buf = Pixmap::new(pixmap.width(), pixmap.height());
            render_node(child, opacity, BlendMode::Normal, phase, &mut child_buf)?;
            for y in 0..pixmap.height() {
                for x in 0..pixmap.width() {
                    let c = child_buf.get(x, y);
                    if c.3 == 0 {
                        continue;
                    }
                    let m = mask_buf.get(x, y);
                    let a = (c.3 as u16 * m.3 as u16 / 255) as u8;
                    if a == 0 {
                        continue;
                    }
                    pixmap.blend(x, y, Rgba(c.0, c.1, c.2, a));
                }
            }
            Ok(())
        }
        Node::Clip { rect, child } => {
            // Render child into a scratch and copy only the clip rectangle.
            let mut scratch = Pixmap::new(pixmap.width(), pixmap.height());
            render_node(child, opacity, BlendMode::Normal, phase, &mut scratch)?;
            let (x0, y0, x1, y1) = bounds(rect, pixmap);
            for y in y0..y1 {
                for x in x0..x1 {
                    let c = scratch.get(x, y);
                    if c.3 == 0 {
                        continue;
                    }
                    pixmap.blend(x, y, c);
                }
            }
            Ok(())
        }
    }
}

fn _src_kind_unused() {}

fn rasterize_rect(
    rect: &PxRect,
    fill: &Paint,
    stroke: Option<&kittui_core::Stroke>,
    corners: &kittui_core::Corners,
    opacity: f32,
    pixmap: &mut Pixmap,
) {
    let (x0, y0, x1, y1) = bounds(rect, pixmap);
    let radius_for = |x: f32, y: f32| {
        let in_tl = x < rect.origin.0 + corners.tl && y < rect.origin.1 + corners.tl;
        let in_tr = x > rect.right() - corners.tr && y < rect.origin.1 + corners.tr;
        let in_bl = x < rect.origin.0 + corners.bl && y > rect.bottom() - corners.bl;
        let in_br = x > rect.right() - corners.br && y > rect.bottom() - corners.br;
        if in_tl {
            Some((Px(rect.origin.0 + corners.tl, rect.origin.1 + corners.tl), corners.tl))
        } else if in_tr {
            Some((Px(rect.right() - corners.tr, rect.origin.1 + corners.tr), corners.tr))
        } else if in_bl {
            Some((Px(rect.origin.0 + corners.bl, rect.bottom() - corners.bl), corners.bl))
        } else if in_br {
            Some((Px(rect.right() - corners.br, rect.bottom() - corners.br), corners.br))
        } else {
            None
        }
    };

    for y in y0..y1 {
        for x in x0..x1 {
            let px = Px(x as f32 + 0.5, y as f32 + 0.5);
            let coverage = if corners.is_square() {
                1.0
            } else if let Some((center, r)) = radius_for(px.0, px.1) {
                let dx = px.0 - center.0;
                let dy = px.1 - center.1;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > r {
                    0.0
                } else if dist > r - 1.0 {
                    (r - dist).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            } else {
                1.0
            };
            if coverage <= 0.0 {
                continue;
            }
            let color = sample_paint(fill, rect, px);
            let blended = scale_alpha(color, opacity * coverage);
            pixmap.blend(x, y, blended);
        }
    }

    if let Some(stroke) = stroke {
        let half = stroke.width_px * 0.5;
        let (inset, outset) = match stroke.align {
            StrokeAlign::Inside => (stroke.width_px, 0.0),
            StrokeAlign::Outside => (0.0, stroke.width_px),
            StrokeAlign::Center => (half, half),
        };
        let outer = PxRect::new(
            rect.origin.0 - outset,
            rect.origin.1 - outset,
            rect.width + outset * 2.0,
            rect.height + outset * 2.0,
        );
        let inner = PxRect::new(
            rect.origin.0 + inset,
            rect.origin.1 + inset,
            (rect.width - inset * 2.0).max(0.0),
            (rect.height - inset * 2.0).max(0.0),
        );
        let (ox0, oy0, ox1, oy1) = bounds(&outer, pixmap);
        for y in oy0..oy1 {
            for x in ox0..ox1 {
                let px = Px(x as f32 + 0.5, y as f32 + 0.5);
                let in_outer = outer.contains(px);
                let in_inner = inner.contains(px);
                if in_outer && !in_inner {
                    let color = sample_paint(&stroke.paint, &outer, px);
                    let blended = scale_alpha(color, opacity);
                    pixmap.blend(x, y, blended);
                }
            }
        }
    }
}

fn rasterize_gradient(
    rect: &PxRect,
    stops: &[kittui_core::node::Stop],
    direction: Direction,
    opacity: f32,
    pixmap: &mut Pixmap,
) {
    if stops.is_empty() {
        return;
    }
    let (x0, y0, x1, y1) = bounds(rect, pixmap);
    for y in y0..y1 {
        for x in x0..x1 {
            let px = Px(x as f32 + 0.5, y as f32 + 0.5);
            let t = gradient_t(direction, rect, px);
            let color = sample_stops(stops, t);
            pixmap.blend(x, y, scale_alpha(color, opacity));
        }
    }
}

fn rasterize_glow(
    rect: &PxRect,
    cx_frac: f32,
    cy_frac: f32,
    radius_frac: f32,
    color: Rgba,
    intensity: f32,
    pixmap: &mut Pixmap,
) {
    let (x0, y0, x1, y1) = bounds(rect, pixmap);
    let cx = rect.origin.0 + rect.width * cx_frac;
    let cy = rect.origin.1 + rect.height * cy_frac;
    let r = rect.width.min(rect.height) * radius_frac;
    if r <= 0.0 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d > r {
                continue;
            }
            // Smoothstep falloff for a soft glow.
            let t = 1.0 - (d / r);
            let weight = t * t * (3.0 - 2.0 * t) * intensity;
            let mut shaded = color;
            shaded.3 = ((color.3 as f32) * weight).clamp(0.0, 255.0) as u8;
            pixmap.blend(x, y, shaded);
        }
    }
}

fn rasterize_scanlines(rect: &PxRect, alpha: u8, period_px: u8, opacity: f32, pixmap: &mut Pixmap) {
    let (x0, y0, x1, y1) = bounds(rect, pixmap);
    let period = period_px.max(1) as i32;
    for y in y0..y1 {
        if (y as i32) % period != 0 {
            continue;
        }
        for x in x0..x1 {
            let mut color = Rgba::default();
            color.3 = ((alpha as f32) * opacity).clamp(0.0, 255.0) as u8;
            pixmap.blend(x, y, color);
        }
    }
}

fn sample_paint(paint: &Paint, rect: &PxRect, px: Px) -> Rgba {
    match paint {
        Paint::Solid { color } => *color,
        Paint::Linear(LinearGradient { direction, stops }) => {
            sample_stops(stops, gradient_t(*direction, rect, px))
        }
        Paint::Radial(RadialGradient {
            center_x_frac,
            center_y_frac,
            radius_frac,
            stops,
        }) => {
            let cx = rect.origin.0 + rect.width * *center_x_frac;
            let cy = rect.origin.1 + rect.height * *center_y_frac;
            let r = rect.width.min(rect.height) * *radius_frac;
            let dx = px.0 - cx;
            let dy = px.1 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let t = if r > 0.0 { (d / r).clamp(0.0, 1.0) } else { 0.0 };
            sample_stops(stops, t)
        }
    }
}

fn sample_stops(stops: &[kittui_core::node::Stop], t: f32) -> Rgba {
    if stops.is_empty() {
        return Rgba::default();
    }
    if stops.len() == 1 || t <= stops[0].offset {
        return stops[0].color;
    }
    if t >= stops[stops.len() - 1].offset {
        return stops[stops.len() - 1].color;
    }
    for pair in stops.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if t >= a.offset && t <= b.offset {
            let span = (b.offset - a.offset).max(f32::EPSILON);
            let local = (t - a.offset) / span;
            return a.color.lerp(b.color, local);
        }
    }
    stops[stops.len() - 1].color
}

fn gradient_t(direction: Direction, rect: &PxRect, px: Px) -> f32 {
    match direction {
        Direction::Horizontal => {
            ((px.0 - rect.origin.0) / rect.width.max(f32::EPSILON)).clamp(0.0, 1.0)
        }
        Direction::Vertical => {
            ((px.1 - rect.origin.1) / rect.height.max(f32::EPSILON)).clamp(0.0, 1.0)
        }
        Direction::Diagonal => {
            let nx = (px.0 - rect.origin.0) / rect.width.max(f32::EPSILON);
            let ny = (px.1 - rect.origin.1) / rect.height.max(f32::EPSILON);
            ((nx + ny) * 0.5).clamp(0.0, 1.0)
        }
    }
}

fn scale_alpha(mut color: Rgba, scale: f32) -> Rgba {
    color.3 = ((color.3 as f32) * scale.clamp(0.0, 1.0)).round() as u8;
    color
}

fn bounds(rect: &PxRect, pixmap: &Pixmap) -> (u32, u32, u32, u32) {
    let x0 = rect.origin.0.floor().max(0.0) as u32;
    let y0 = rect.origin.1.floor().max(0.0) as u32;
    let x1 = rect
        .right()
        .ceil()
        .clamp(0.0, pixmap.width() as f32) as u32;
    let y1 = rect
        .bottom()
        .ceil()
        .clamp(0.0, pixmap.height() as f32) as u32;
    (x0, y0, x1, y1)
}

#[allow(dead_code)]
fn _fit_unused(_: Fit) {}

fn fit_into_rect(dst: PxRect, src_w: u32, src_h: u32, fit: Fit) -> PxRect {
    if src_w == 0 || src_h == 0 || dst.width == 0.0 || dst.height == 0.0 {
        return dst;
    }
    let sw = src_w as f32;
    let sh = src_h as f32;
    match fit {
        Fit::Stretch => dst,
        Fit::Contain => {
            let scale = (dst.width / sw).min(dst.height / sh);
            let w = sw * scale;
            let h = sh * scale;
            PxRect::new(
                dst.origin.0 + (dst.width - w) * 0.5,
                dst.origin.1 + (dst.height - h) * 0.5,
                w,
                h,
            )
        }
        Fit::Cover => {
            let scale = (dst.width / sw).max(dst.height / sh);
            let w = sw * scale;
            let h = sh * scale;
            PxRect::new(
                dst.origin.0 + (dst.width - w) * 0.5,
                dst.origin.1 + (dst.height - h) * 0.5,
                w,
                h,
            )
        }
        Fit::None => PxRect::new(
            dst.origin.0 + (dst.width - sw) * 0.5,
            dst.origin.1 + (dst.height - sh) * 0.5,
            sw,
            sh,
        ),
    }
}

fn rasterize_image(
    rect: &PxRect,
    src: &ImageRef,
    fit: Fit,
    tint: Option<Rgba>,
    opacity: f32,
    pixmap: &mut Pixmap,
) -> Result<(), RenderError> {
    let bytes = load_image_bytes(src)?;
    let (w, h, rgba) = decode_image(&bytes)?;
    let placement = fit_into_rect(*rect, w, h, fit);
    let (x0, y0, x1, y1) = bounds(&placement, pixmap);
    if x1 <= x0 || y1 <= y0 {
        return Ok(());
    }
    let target_w = placement.width.max(1.0);
    let target_h = placement.height.max(1.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let fx = (x as f32 + 0.5 - placement.origin.0) / target_w;
            let fy = (y as f32 + 0.5 - placement.origin.1) / target_h;
            if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
                continue;
            }
            let sx = (fx * w as f32) as u32;
            let sy = (fy * h as f32) as u32;
            let idx = ((sy * w + sx) * 4) as usize;
            let mut color = Rgba(rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]);
            if let Some(t) = tint {
                color = Rgba(
                    ((color.0 as u16 * t.0 as u16) / 255) as u8,
                    ((color.1 as u16 * t.1 as u16) / 255) as u8,
                    ((color.2 as u16 * t.2 as u16) / 255) as u8,
                    ((color.3 as u16 * t.3 as u16) / 255) as u8,
                );
            }
            color.3 = ((color.3 as f32) * opacity).clamp(0.0, 255.0) as u8;
            if color.3 == 0 {
                continue;
            }
            pixmap.blend(x, y, color);
        }
    }
    Ok(())
}

fn load_image_bytes(src: &ImageRef) -> Result<Vec<u8>, RenderError> {
    match src {
        ImageRef::Bytes { bytes } => Ok(bytes.clone()),
        ImageRef::Path { path } => std::fs::read(path)
            .map_err(|e| RenderError::UnsupportedImage(image_read_error(path, &e))),
        ImageRef::Cached { hash } => Err(RenderError::UnsupportedImage(cached_image_error(hash))),
    }
}

fn image_read_error(path: &str, err: &std::io::Error) -> String {
    let mut message = String::with_capacity("failed to read : ".len() + path.len() + err.to_string().len());
    message.push_str("failed to read ");
    message.push_str(path);
    message.push_str(": ");
    write!(message, "{err}").expect("write to string");
    message
}

fn cached_image_error(hash: &str) -> String {
    let mut message = String::with_capacity(
        "cached image refs not supported in CPU rasterizer: ".len() + hash.len(),
    );
    message.push_str("cached image refs not supported in CPU rasterizer: ");
    write!(message, "{hash}").expect("write to string");
    message
}

#[cfg(feature = "image-decoders")]
fn decode_image(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    use image::GenericImageView;
    let img = image::load_from_memory(bytes)
        .map_err(|e| RenderError::UnsupportedImage(format!("decode failed: {e}")))?;
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8().into_raw();
    Ok((w, h, rgba))
}

#[cfg(not(feature = "image-decoders"))]
fn decode_image(_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    Err(RenderError::UnsupportedImage(
        "image-decoders feature disabled; rebuild with --features image-decoders".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_read_error_builds_directly() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let message = image_read_error("/tmp/missing.png", &err);
        assert_eq!(message, "failed to read /tmp/missing.png: missing");
        assert!(message.capacity() >= message.len());
    }

    #[test]
    fn load_image_bytes_reports_path_read_errors_with_stable_message() {
        let path = "/tmp/kittui-render-cpu-definitely-missing.png";
        let err = load_image_bytes(&ImageRef::Path {
            path: path.to_string(),
        })
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.starts_with(
                "image source failed to read /tmp/kittui-render-cpu-definitely-missing.png: "
            ),
            "{message}"
        );
        assert!(
            message.ends_with(" is not supported by the CPU renderer in this version"),
            "{message}"
        );
    }

    #[test]
    fn cached_image_error_builds_directly() {
        let message = cached_image_error("abc123");
        assert_eq!(
            message,
            "cached image refs not supported in CPU rasterizer: abc123"
        );
        assert!(message.capacity() >= message.len());
    }

    #[test]
    fn load_image_bytes_rejects_cached_refs_with_stable_message() {
        let err = load_image_bytes(&ImageRef::Cached {
            hash: "abc123".to_string(),
        })
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "image source cached image refs not supported in CPU rasterizer: abc123 is not supported by the CPU renderer in this version"
        );
    }
}
