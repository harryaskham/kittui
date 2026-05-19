//! 8-bit RGBA color with stable parsing across CSS hex literals.

use serde::{Deserialize, Serialize};

/// 8-bit-per-channel RGBA color. Order is `r, g, b, a` with 0..=255 channels;
/// `a == 0` is fully transparent and `a == 255` is fully opaque.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    /// Construct an opaque color from `r,g,b` channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 0xff)
    }

    /// Construct a color from `r,g,b,a` channels.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(r, g, b, a)
    }

    /// Parse a CSS-style hex literal (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`,
    /// or the same without the leading `#`). Case-insensitive.
    pub fn parse(input: &str) -> Result<Self, ColorParseError> {
        let s = input.trim();
        let hex = s.strip_prefix('#').unwrap_or(s);
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ColorParseError::InvalidLiteral(input.to_owned()));
        }
        let expanded = match hex.len() {
            3 => format!(
                "{}ff",
                hex.chars().flat_map(|c| [c, c]).collect::<String>()
            ),
            4 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 => format!("{hex}ff"),
            8 => hex.to_owned(),
            _ => return Err(ColorParseError::InvalidLiteral(input.to_owned())),
        };
        let nibble = |range| u8::from_str_radix(&expanded[range], 16).unwrap();
        Ok(Self(nibble(0..2), nibble(2..4), nibble(4..6), nibble(6..8)))
    }

    /// Linearly interpolate between `self` and `other`. `t` is clamped to
    /// `[0,1]`; channels are interpolated in straight (non-premultiplied)
    /// space to match how the legacy JS renderer composes.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| ((a as f32) * (1.0 - t) + (b as f32) * t).round() as u8;
        Self(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
            mix(self.3, other.3),
        )
    }

    /// Premultiplied alpha view of the channels as `f32` in `[0,1]`.
    pub fn premultiplied(self) -> [f32; 4] {
        let a = self.3 as f32 / 255.0;
        [
            (self.0 as f32 / 255.0) * a,
            (self.1 as f32 / 255.0) * a,
            (self.2 as f32 / 255.0) * a,
            a,
        ]
    }
}

/// Error returned by [`Rgba::parse`] when the input is not a recognised hex
/// literal.
#[derive(Debug, thiserror::Error)]
pub enum ColorParseError {
    /// The literal could not be parsed as `#rgb` / `#rrggbb` / `#rrggbbaa` etc.
    #[error("invalid color literal: {0}")]
    InvalidLiteral(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortforms_expand_per_css() {
        assert_eq!(Rgba::parse("#abc").unwrap(), Rgba(0xaa, 0xbb, 0xcc, 0xff));
        assert_eq!(Rgba::parse("#abcd").unwrap(), Rgba(0xaa, 0xbb, 0xcc, 0xdd));
        assert_eq!(
            Rgba::parse("00d8ff").unwrap(),
            Rgba(0x00, 0xd8, 0xff, 0xff)
        );
        assert_eq!(
            Rgba::parse("#00d8ff80").unwrap(),
            Rgba(0x00, 0xd8, 0xff, 0x80)
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Rgba::parse("not a color").is_err());
        assert!(Rgba::parse("#zz").is_err());
        assert!(Rgba::parse("#12345").is_err());
    }

    #[test]
    fn lerp_endpoints_round_trip() {
        let a = Rgba::rgb(0, 0, 0);
        let b = Rgba::rgb(255, 128, 64);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        let mid = a.lerp(b, 0.5);
        assert_eq!(mid, Rgba(128, 64, 32, 255));
    }
}
