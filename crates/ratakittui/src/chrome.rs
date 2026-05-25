//! `Chrome` builder and its compilation to a kittui `Scene`.
//!
//! Chrome is the declarative envelope around any ratatui widget. The fields
//! are independent of the widget: a `Block`-shaped chrome works the same on
//! a `Paragraph` or a `Table`. Compilation to a `Scene` happens lazily via
//! [`Chrome::to_scene`].
//!
//! Chrome strictly reserves edge cells via `padding` so the inner widget
//! never paints over its own border. This is the JS Pi implementation's
//! single most common bug; ratakittui's design rule is to make it
//! representationally impossible.
//!
//! No new node types are introduced here; `Chrome` only composes the
//! primitives defined in `kittui-core` (Rect, Gradient, Glow, Scanlines,
//! Group, Composite, Mask, Clip).

use ratatui::layout::Rect;

use kittui::{
    CellRect, CellSize, Corners, Direction, Layer, Node, Paint, PhaseCurve, Px, PxRect, Rgba,
    Scene, Stop, Stroke,
};
use kittui_core::node::StrokeAlign;
use kittui_core::Animation;

/// Background fill mode.
#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    /// Flat solid color.
    Solid(Rgba),
    /// Two-stop linear gradient with direction.
    Linear {
        /// Direction the gradient sweeps along.
        direction: Direction,
        /// Start color (offset 0.0).
        start: Rgba,
        /// End color (offset 1.0).
        end: Rgba,
    },
    /// Radial gradient anchored at fractional `(cx, cy)` with fractional `radius`.
    Radial {
        /// Center x in [0,1].
        cx: f32,
        /// Center y in [0,1].
        cy: f32,
        /// Radius as fraction of min(width,height).
        radius: f32,
        /// Inner stop color.
        inner: Rgba,
        /// Outer stop color.
        outer: Rgba,
    },
}

/// Border stroke description. Joined corners across widgets are handled by
/// `JoinGroup`; everything single-widget-local lives here.
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    /// Stroke color (alpha respected).
    pub color: Rgba,
    /// Stroke width in pixels.
    pub width_px: f32,
    /// Per-corner radius in pixels. Defaults to a uniform soft rounding.
    pub corners: Corners,
    /// How the stroke is anchored relative to the rectangle edge.
    pub align: StrokeAlign,
}

impl Border {
    /// Construct a uniform-radius rounded border.
    pub fn rounded(color: Rgba, width_px: f32, radius_px: f32) -> Self {
        Self {
            color,
            width_px,
            corners: Corners::uniform(radius_px),
            align: StrokeAlign::Inside,
        }
    }

    /// Construct a square (non-rounded) border.
    pub fn square(color: Rgba, width_px: f32) -> Self {
        Self {
            color,
            width_px,
            corners: Corners::default(),
            align: StrokeAlign::Inside,
        }
    }
}

/// A title or footer strip. The strip is a one-cell-tall band glued to the
/// top (title) or bottom (footer) edge of the chrome. The strip is drawn at
/// the scene level; the ratatui inner widget never paints into the strip's
/// cells because of the padding reservation policy.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    /// Strip background.
    pub background: Background,
    /// Optional accent color used to draw a single-pixel underline (title)
    /// or overline (footer) within the strip cell.
    pub accent: Option<Rgba>,
}

/// Radial glow with optional pulse animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Glow {
    /// Glow color (alpha controls maximum intensity).
    pub color: Rgba,
    /// Center fractional position.
    pub cx: f32,
    /// Center fractional position.
    pub cy: f32,
    /// Radius fraction of min(width,height).
    pub radius: f32,
    /// Intensity multiplier in [0,1].
    pub intensity: f32,
    /// Optional pulse animation.
    pub pulse: Option<Pulse>,
}

/// Pulse parameters when a glow should animate. Frame data uploads once;
/// the kitty terminal animates natively.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Pulse {
    /// Number of frames in the loop. >= 2.
    pub frames: u16,
    /// Full loop duration in milliseconds.
    pub cycle_ms: u32,
}

/// Scanline overlay.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Scanlines {
    /// Per-line alpha in 0..=255.
    pub alpha: u8,
    /// Pixel period between scanlines.
    pub period_px: u8,
}

/// Drop shadow. Composes existing nodes: a darker `Rect` translated and
/// blurred by composition with a radial `Glow`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Shadow {
    /// Offset in pixels (positive = down/right).
    pub dx_px: f32,
    /// Offset in pixels.
    pub dy_px: f32,
    /// Shadow color (alpha respected).
    pub color: Rgba,
}

/// Padding reserved between chrome edges and the inner widget rect, in cells.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    /// Cells reserved at the top of the chrome (used by titles).
    pub top: u16,
    /// Cells reserved at the bottom of the chrome (used by footers).
    pub bottom: u16,
    /// Cells reserved on the left.
    pub left: u16,
    /// Cells reserved on the right.
    pub right: u16,
}

impl Padding {
    /// Uniform padding on all sides.
    pub fn uniform(cells: u16) -> Self {
        Self {
            top: cells,
            bottom: cells,
            left: cells,
            right: cells,
        }
    }

    /// Padding with explicit `(top, right, bottom, left)`.
    pub fn trbl(top: u16, right: u16, bottom: u16, left: u16) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// Clipping policy for the chrome and the inner widget.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ClipPolicy {
    /// Inner widget cannot draw outside its post-padding rect (default).
    #[default]
    Strict,
    /// Inner widget can overflow; chrome still clips at its rect.
    Overflow,
}

/// The declarative chrome description. See module docs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chrome {
    /// Background fill.
    pub background: Option<Background>,
    /// Optional border.
    pub border: Option<Border>,
    /// Optional title strip (top, one cell tall).
    pub title: Option<Strip>,
    /// Optional footer strip (bottom, one cell tall).
    pub footer: Option<Strip>,
    /// Optional radial glow.
    pub glow: Option<Glow>,
    /// Optional scanlines overlay.
    pub scanlines: Option<Scanlines>,
    /// Optional drop shadow.
    pub shadow: Option<Shadow>,
    /// Cells reserved between chrome and inner content.
    pub padding: Padding,
    /// Clipping policy.
    pub clip: ClipPolicy,
}

impl Chrome {
    /// Empty chrome: no decoration at all. Equivalent to `Chrome::default()`.
    pub fn none() -> Self {
        Self::default()
    }

    /// Builder helper to set the background.
    pub fn background(mut self, bg: Background) -> Self {
        self.background = Some(bg);
        self
    }

    /// Builder helper to set the border.
    pub fn border(mut self, b: Border) -> Self {
        self.border = Some(b);
        self
    }

    /// Builder helper to set a title strip and reserve one top cell.
    pub fn title(mut self, strip: Strip) -> Self {
        self.title = Some(strip);
        if self.padding.top == 0 {
            self.padding.top = 1;
        }
        self
    }

    /// Builder helper to set a footer strip and reserve one bottom cell.
    pub fn footer(mut self, strip: Strip) -> Self {
        self.footer = Some(strip);
        if self.padding.bottom == 0 {
            self.padding.bottom = 1;
        }
        self
    }

    /// Builder helper to set the glow.
    pub fn glow(mut self, g: Glow) -> Self {
        self.glow = Some(g);
        self
    }

    /// Builder helper to set scanlines.
    pub fn scanlines(mut self, s: Scanlines) -> Self {
        self.scanlines = Some(s);
        self
    }

    /// Builder helper to set drop shadow.
    pub fn shadow(mut self, s: Shadow) -> Self {
        self.shadow = Some(s);
        self
    }

    /// Builder helper to set padding.
    pub fn padding(mut self, p: Padding) -> Self {
        self.padding = p;
        self
    }

    /// Builder helper to set clip policy.
    pub fn clip(mut self, c: ClipPolicy) -> Self {
        self.clip = c;
        self
    }

    /// Whether the chrome will produce any nodes.
    pub fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.border.is_none()
            && self.title.is_none()
            && self.footer.is_none()
            && self.glow.is_none()
            && self.scanlines.is_none()
            && self.shadow.is_none()
    }

    /// Compute the inner ratatui `Rect` that the wrapped widget should
    /// render into after padding is reserved.
    pub fn inner_rect(&self, area: Rect) -> Rect {
        let Padding {
            top,
            right,
            bottom,
            left,
        } = self.padding;
        let x = area.x.saturating_add(left);
        let y = area.y.saturating_add(top);
        let w = area.width.saturating_sub(left).saturating_sub(right);
        let h = area.height.saturating_sub(top).saturating_sub(bottom);
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Compile this chrome into a `Scene` covering the chrome rectangle.
    /// Returns `None` if the chrome is empty.
    pub fn to_scene(&self, area: Rect) -> Option<Scene> {
        if self.is_empty() || area.width == 0 || area.height == 0 {
            return None;
        }
        let cell = CellSize::default();
        let footprint = CellRect::new(area.x, area.y, area.width, area.height);
        let rect = footprint.to_pixels(cell);
        let mut layers: Vec<Layer> = Vec::with_capacity(8);

        if let Some(shadow) = &self.shadow {
            layers.push(Layer::new(
                "shadow",
                Node::Glow {
                    rect: shifted(rect, shadow.dx_px, shadow.dy_px),
                    center_x_frac: 0.5,
                    center_y_frac: 0.5,
                    radius_frac: 0.55,
                    color: shadow.color,
                    intensity: 0.8,
                },
            ));
        }

        if let Some(bg) = &self.background {
            layers.push(Layer::new("background", background_node(bg, rect)));
        }

        if let Some(border) = &self.border {
            layers.push(Layer::new("border", border_node(border, rect)));
        }

        if let Some(title) = &self.title {
            layers.push(Layer::new(
                "title",
                strip_node(title, top_strip(rect, cell)),
            ));
        }
        if let Some(footer) = &self.footer {
            layers.push(Layer::new(
                "footer",
                strip_node(footer, bottom_strip(rect, cell)),
            ));
        }

        if let Some(scanlines) = &self.scanlines {
            layers.push(Layer::new(
                "scanlines",
                Node::Scanlines {
                    rect,
                    alpha: scanlines.alpha,
                    period_px: scanlines.period_px,
                },
            ));
        }

        let mut animation: Option<Animation> = None;
        if let Some(glow) = &self.glow {
            layers.push(Layer::new(
                "glow",
                Node::Glow {
                    rect,
                    center_x_frac: glow.cx,
                    center_y_frac: glow.cy,
                    radius_frac: glow.radius,
                    color: glow.color,
                    intensity: glow.intensity,
                },
            ));
            if let Some(pulse) = glow.pulse {
                animation = Some(Animation {
                    frames: pulse.frames,
                    cycle_ms: pulse.cycle_ms,
                    curve: PhaseCurve::Pulse { harmonics: 0 },
                    loops: 0,
                });
            }
        }

        Some(Scene {
            footprint,
            cell_size: cell,
            layers,
            animation,
        })
    }
}

fn background_node(bg: &Background, rect: PxRect) -> Node {
    match *bg {
        Background::Solid(color) => Node::Rect {
            rect,
            fill: Paint::Solid { color },
            stroke: None,
            corners: Corners::default(),
        },
        Background::Linear {
            direction,
            start,
            end,
        } => Node::Gradient {
            rect,
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
        Background::Radial {
            cx,
            cy,
            radius,
            inner,
            outer,
        } => Node::Glow {
            rect,
            center_x_frac: cx,
            center_y_frac: cy,
            radius_frac: radius,
            color: blend_inner_outer(inner, outer),
            intensity: 1.0,
        },
    }
}

fn blend_inner_outer(inner: Rgba, outer: Rgba) -> Rgba {
    // The CPU `Glow` node draws a single-color falloff; until the renderer
    // grows true radial gradients with stops, approximate with the midpoint
    // so the gradient direction reads correctly.
    inner.lerp(outer, 0.5)
}

fn border_node(b: &Border, rect: PxRect) -> Node {
    Node::Rect {
        rect,
        fill: Paint::Solid {
            color: Rgba::rgba(0, 0, 0, 0),
        },
        stroke: Some(Stroke {
            align: b.align,
            width_px: b.width_px,
            paint: Paint::Solid { color: b.color },
        }),
        corners: b.corners,
    }
}

fn strip_node(strip: &Strip, rect: PxRect) -> Node {
    // Compose strip background + optional accent line via a Composite group
    // so it survives masking by joined-border resolution later.
    let mut children = vec![background_node(&strip.background, rect)];
    if let Some(accent) = strip.accent {
        let band_h = (rect.height * 0.18).max(1.0);
        let band = PxRect::new(rect.origin.0, rect.bottom() - band_h, rect.width, band_h);
        children.push(Node::Rect {
            rect: band,
            fill: Paint::Solid { color: accent },
            stroke: None,
            corners: Corners::default(),
        });
    }
    Node::Composite {
        mode: kittui_core::node::BlendMode::Normal,
        children,
    }
}

fn top_strip(rect: PxRect, cell: CellSize) -> PxRect {
    PxRect::new(
        rect.origin.0,
        rect.origin.1,
        rect.width,
        cell.height_px as f32,
    )
}

fn bottom_strip(rect: PxRect, cell: CellSize) -> PxRect {
    PxRect::new(
        rect.origin.0,
        rect.bottom() - cell.height_px as f32,
        rect.width,
        cell.height_px as f32,
    )
}

fn shifted(rect: PxRect, dx: f32, dy: f32) -> PxRect {
    PxRect {
        origin: Px(rect.origin.0 + dx, rect.origin.1 + dy),
        width: rect.width,
        height: rect.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn empty_chrome_produces_no_scene() {
        let chrome = Chrome::none();
        assert!(chrome.to_scene(Rect::new(0, 0, 10, 4)).is_none());
    }

    #[test]
    fn title_reserves_top_cell_and_strip_layer() {
        let chrome = Chrome::default().title(Strip {
            background: Background::Solid(Rgba::rgb(0, 0xd8, 0xff)),
            accent: None,
        });
        let inner = chrome.inner_rect(Rect::new(0, 0, 20, 4));
        assert_eq!(inner.y, 1);
        let scene = chrome.to_scene(Rect::new(0, 0, 20, 4)).unwrap();
        assert!(scene
            .layers
            .iter()
            .any(|l| l.label.as_deref() == Some("title")));
    }

    #[test]
    fn pulse_glow_yields_animation() {
        let chrome = Chrome::default().glow(Glow {
            color: Rgba::rgb(0, 0xd8, 0xff),
            cx: 0.5,
            cy: 0.5,
            radius: 0.5,
            intensity: 0.6,
            pulse: Some(Pulse {
                frames: 180,
                cycle_ms: 3000,
            }),
        });
        let scene = chrome.to_scene(Rect::new(0, 0, 20, 4)).unwrap();
        let animation = scene.animation.as_ref().unwrap();
        assert_eq!(animation.frames, 180);
        assert_eq!(animation.cycle_ms, 3000);
    }

    #[test]
    fn padding_clamps_inner_rect() {
        let chrome = Chrome::default().padding(Padding::uniform(2));
        let inner = chrome.inner_rect(Rect::new(0, 0, 3, 3));
        assert_eq!(inner.width, 0);
        assert_eq!(inner.height, 0);
    }
}
