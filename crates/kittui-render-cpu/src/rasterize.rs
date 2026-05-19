//! Scene → pixmap rasterizer. The renderer is straightforward: scan each
//! pixel inside a node's bounding rect, evaluate the node's shading function
//! at that pixel, and blend into the pixmap. The renderer is intentionally
//! conservative — its goal is correctness and reference parity, not raw
//! speed. The GPU backend is where high-fps rendering lives.

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
        Node::Image { src, .. } => Err(RenderError::UnsupportedImage(format!("{:?}", src_kind(src)))),
        Node::Group { opacity: o, children } => {
            let combined = (opacity * o.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            for child in children {
                render_node(child, combined, BlendMode::Normal, phase, pixmap)?;
            }
            Ok(())
        }
        Node::Composite { mode, children } => {
            for child in children {
                render_node(child, opacity, *mode, phase, pixmap)?;
            }
            Ok(())
        }
        Node::Mask { child, .. } => render_node(child, opacity, BlendMode::Normal, phase, pixmap),
        Node::Clip { child, .. } => render_node(child, opacity, BlendMode::Normal, phase, pixmap),
    }
}

fn src_kind(src: &ImageRef) -> &'static str {
    match src {
        ImageRef::Path { .. } => "path",
        ImageRef::Bytes { .. } => "bytes",
        ImageRef::Cached { .. } => "cached",
    }
}

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
fn _fit_unused(_: Fit) {} // Fit will be used when image rasterization lands.
