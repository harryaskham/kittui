//! Paint descriptions for fills and strokes.

use serde::{Deserialize, Serialize};

use crate::color::Rgba;
use crate::node::{Direction, Stop};

/// A paint is either a solid color or a parameterized gradient. Gradients
/// carry their own stop list so they round-trip through serde without
/// losing precision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Paint {
    /// A flat fill.
    Solid {
        /// The color to fill with.
        color: Rgba,
    },
    /// A linear two-or-more stop gradient.
    Linear(LinearGradient),
    /// A circular radial gradient anchored to a point.
    Radial(RadialGradient),
}

impl Paint {
    /// Convenience for an opaque solid color from a CSS literal.
    pub fn solid(literal: &str) -> Result<Self, crate::color::ColorParseError> {
        Ok(Self::Solid {
            color: Rgba::parse(literal)?,
        })
    }

    /// Construct a two-stop linear gradient between `start` and `end`.
    pub fn linear(
        start: &str,
        end: &str,
        direction: Direction,
    ) -> Result<Self, crate::color::ColorParseError> {
        Ok(Self::Linear(LinearGradient {
            direction,
            stops: vec![
                Stop {
                    offset: 0.0,
                    color: Rgba::parse(start)?,
                },
                Stop {
                    offset: 1.0,
                    color: Rgba::parse(end)?,
                },
            ],
        }))
    }
}

/// A linear gradient with `Direction` and ordered stops. `offset` is `[0,1]`
/// along the gradient axis. The renderer expects at least two stops.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    /// Direction the gradient sweeps along.
    pub direction: Direction,
    /// Ordered stop list.
    pub stops: Vec<Stop>,
}

/// A radial gradient centered at a fractional position with a fractional
/// radius (both relative to the containing rectangle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadialGradient {
    /// Center `x` as a fraction `[0,1]` of the rect width.
    pub center_x_frac: f32,
    /// Center `y` as a fraction `[0,1]` of the rect height.
    pub center_y_frac: f32,
    /// Radius as a fraction of `min(width, height)`.
    pub radius_frac: f32,
    /// Ordered stop list.
    pub stops: Vec<Stop>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_two_stop_builder() {
        let paint = Paint::linear("#000", "#fff", Direction::Horizontal).unwrap();
        match paint {
            Paint::Linear(LinearGradient { stops, direction }) => {
                assert_eq!(direction, Direction::Horizontal);
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[0].color, Rgba::rgb(0, 0, 0));
                assert_eq!(stops[1].color, Rgba::rgb(255, 255, 255));
            }
            _ => panic!("expected linear"),
        }
    }
}
