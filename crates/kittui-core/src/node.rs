//! Scene graph nodes.

use serde::{Deserialize, Serialize};

use crate::color::Rgba;
use crate::geom::PxRect;
use crate::paint::Paint;

/// The direction a gradient sweeps along, in pixel space.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
    /// Top-left to bottom-right diagonal.
    Diagonal,
}

/// Blend mode for [`Node::Composite`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Standard source-over compositing.
    Normal,
    /// Additive blend; channels are summed and clamped.
    Add,
    /// Multiplicative blend; channels are multiplied in `[0,1]` space.
    Multiply,
    /// Screen blend, the inverse of multiply.
    Screen,
}

/// How an image fills its placement rectangle.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Stretch to fill, preserving aspect ratio (letterbox).
    Contain,
    /// Stretch to cover, preserving aspect ratio (crops overflow).
    Cover,
    /// Stretch to exact rect; aspect ratio is not preserved.
    Stretch,
    /// Center without resizing.
    None,
}

/// Per-corner radius in pixels. All zeros means a perfect rectangle.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Corners {
    /// Top-left corner radius.
    pub tl: f32,
    /// Top-right corner radius.
    pub tr: f32,
    /// Bottom-left corner radius.
    pub bl: f32,
    /// Bottom-right corner radius.
    pub br: f32,
}

impl Corners {
    /// Uniform radius for all four corners.
    pub const fn uniform(r: f32) -> Self {
        Self {
            tl: r,
            tr: r,
            bl: r,
            br: r,
        }
    }

    /// Whether this corner set is effectively a rectangle.
    pub fn is_square(self) -> bool {
        self.tl == 0.0 && self.tr == 0.0 && self.bl == 0.0 && self.br == 0.0
    }
}

/// Where to align a stroke relative to the geometric edge.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    /// Stroke is drawn entirely inside the rectangle.
    Inside,
    /// Stroke is drawn entirely outside the rectangle.
    Outside,
    /// Stroke straddles the edge.
    Center,
}

/// A stroke describes the outline of a [`Node::Rect`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Where the stroke is anchored relative to the rect edge.
    pub align: StrokeAlign,
    /// Stroke thickness in pixels.
    pub width_px: f32,
    /// Stroke fill (can itself be a gradient).
    pub paint: Paint,
}

impl Stroke {
    /// Convenience constructor for an inside-aligned stroke.
    pub fn inside(width_px: f32, paint: Paint) -> Self {
        Self {
            align: StrokeAlign::Inside,
            width_px,
            paint,
        }
    }
}

/// A gradient stop along `[0,1]`.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// Position along the gradient axis.
    pub offset: f32,
    /// Color at this stop.
    pub color: Rgba,
}

/// Reference to an external image. PNG/JPEG bytes only in v1; SVG support is
/// gated behind a feature flag added later.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ImageRef {
    /// File on disk.
    Path {
        /// Absolute path.
        path: String,
    },
    /// Raw image bytes (PNG / JPEG). Stored inline in the scene.
    Bytes {
        /// The encoded image data.
        bytes: Vec<u8>,
    },
    /// Reference to an already-cached image id.
    Cached {
        /// Content hash of the image.
        hash: String,
    },
}

/// A single node in the scene graph.
///
/// The renderer walks layers back-to-front and composites the result. Nodes
/// are pure data; no rasterization decisions live here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    /// Filled and optionally stroked rectangle with optional rounded corners.
    Rect {
        /// Pixel-space bounds.
        rect: PxRect,
        /// Fill paint.
        fill: Paint,
        /// Optional outline.
        stroke: Option<Stroke>,
        /// Corner radii.
        corners: Corners,
    },
    /// A gradient fill without a stroke.
    Gradient {
        /// Pixel-space bounds.
        rect: PxRect,
        /// Ordered stops.
        stops: Vec<Stop>,
        /// Direction of the gradient axis.
        direction: Direction,
    },
    /// Radial glow centered at a point.
    Glow {
        /// Pixel-space bounds the glow lives within.
        rect: PxRect,
        /// Center `x` as a fraction `[0,1]` of `rect.width`.
        center_x_frac: f32,
        /// Center `y` as a fraction `[0,1]` of `rect.height`.
        center_y_frac: f32,
        /// Radius as a fraction of `min(rect.width, rect.height)`.
        radius_frac: f32,
        /// Glow color (alpha controls intensity).
        color: Rgba,
        /// Multiplier applied to the falloff curve, `[0,1]`.
        intensity: f32,
    },
    /// Horizontal scanline overlay.
    Scanlines {
        /// Pixel-space bounds.
        rect: PxRect,
        /// Alpha of each scanline.
        alpha: u8,
        /// Pixel period between scanlines.
        period_px: u8,
    },
    /// Embedded image.
    Image {
        /// Placement rectangle.
        rect: PxRect,
        /// Image source.
        src: ImageRef,
        /// How the image fills the rectangle.
        fit: Fit,
        /// Optional multiplicative tint.
        tint: Option<Rgba>,
    },
    /// Group with opacity, useful for fading whole sub-trees.
    Group {
        /// Opacity in `[0,1]` applied to the entire subtree.
        opacity: f32,
        /// Children rendered in order.
        children: Vec<Node>,
    },
    /// Composite blend; controls how `children` mix.
    Composite {
        /// Blend mode applied across children.
        mode: BlendMode,
        /// Children rendered in order.
        children: Vec<Node>,
    },
    /// Mask the child node by another node's alpha.
    Mask {
        /// Mask node (alpha defines visibility).
        mask: Box<Node>,
        /// Content to be masked.
        child: Box<Node>,
    },
    /// Clip the child to an axis-aligned rectangle.
    Clip {
        /// Clip bounds.
        rect: PxRect,
        /// Content.
        child: Box<Node>,
    },
    /// User-supplied fragment shader. The shader is WGSL source code
    /// run by the GPU backend against the node's rectangle. The CPU
    /// backend rejects scenes containing this variant (until a
    /// naga-driven WGSL→CPU compiler lands).
    Shader {
        /// Pixel-space bounds the shader paints into.
        rect: PxRect,
        /// WGSL fragment shader source. Must declare
        /// `@fragment fn fs_main(@location(0) frag: vec2<f32>) -> @location(0) vec4<f32>`.
        source: String,
        /// Up to 4 user-supplied vec4 uniforms forwarded as the
        /// `user0..user3` fields of the shader's uniform block.
        #[serde(default)]
        uniforms: Vec<[f32; 4]>,
    },
}

/// A layer is a back-to-front slice of the scene. Layers exist to make
/// z-ordering explicit and to allow diff-based partial re-renders later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Optional debug label. Not used by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Layer content.
    pub root: Node,
}

impl Layer {
    /// Construct a labelled layer.
    pub fn new(label: impl Into<String>, root: Node) -> Self {
        Self {
            label: Some(label.into()),
            root,
        }
    }

    /// Construct an unlabelled layer.
    pub fn anon(root: Node) -> Self {
        Self { label: None, root }
    }
}
