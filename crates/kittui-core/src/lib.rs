//! kittui-core
//!
//! General terminal-graphics primitives shared by every kittui crate. Holds
//! only data types, geometry, color, hashing, and animation phase math.
//! Contains no I/O, no rasterization, no protocol code — those are layered
//! crates that consume these types. The intent is that the same `Scene`
//! travels unchanged from a Rust builder, JSON over FFI, or a CLI invocation.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

pub mod animation;
pub mod color;
pub mod geom;
pub mod hash;
pub mod node;
pub mod paint;
pub mod scene;
pub mod terminal;

pub use animation::{
    Animation, PhaseCurve, STANDARD_ANIMATION_CYCLE_MS, STANDARD_ANIMATION_FPS,
    STANDARD_ANIMATION_FRAMES,
};
pub use color::Rgba;
pub use geom::{CellRect, CellSize, Px, PxRect};
pub use node::{BlendMode, Corners, Direction, Fit, ImageRef, Layer, Node, Stop, Stroke};
pub use paint::{LinearGradient, Paint, RadialGradient};
pub use scene::{Scene, SceneId};
pub use terminal::TerminalInfo;
