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
    //
    // `rgba` deliberately mirrors `rgb` as an ergonomic constructor and is used
    // at 150+ call sites; renaming to satisfy `self_named_constructors` would be
    // a wide breaking change for no behavioural gain, so the lint is scoped-allowed
    // here to keep `kittui-core` strict-clippy clean (see bd-dc44f1).
    #[allow(clippy::self_named_constructors)]
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
        let bytes = hex.as_bytes();
        let rgba = match bytes.len() {
            3 => [
                doubled_hex_byte(bytes[0]),
                doubled_hex_byte(bytes[1]),
                doubled_hex_byte(bytes[2]),
                0xff,
            ],
            4 => [
                doubled_hex_byte(bytes[0]),
                doubled_hex_byte(bytes[1]),
                doubled_hex_byte(bytes[2]),
                doubled_hex_byte(bytes[3]),
            ],
            6 => [
                hex_byte(bytes[0], bytes[1]),
                hex_byte(bytes[2], bytes[3]),
                hex_byte(bytes[4], bytes[5]),
                0xff,
            ],
            8 => [
                hex_byte(bytes[0], bytes[1]),
                hex_byte(bytes[2], bytes[3]),
                hex_byte(bytes[4], bytes[5]),
                hex_byte(bytes[6], bytes[7]),
            ],
            _ => return Err(ColorParseError::InvalidLiteral(input.to_owned())),
        };
        Ok(Self(rgba[0], rgba[1], rgba[2], rgba[3]))
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

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!("Rgba::parse validates ASCII hex digits before decoding"),
    }
}

fn doubled_hex_byte(byte: u8) -> u8 {
    let nibble = hex_nibble(byte);
    (nibble << 4) | nibble
}

fn hex_byte(high: u8, low: u8) -> u8 {
    (hex_nibble(high) << 4) | hex_nibble(low)
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
    fn hex_helpers_decode_without_expanded_string() {
        assert_eq!(doubled_hex_byte(b'a'), 0xaa);
        assert_eq!(doubled_hex_byte(b'F'), 0xff);
        assert_eq!(hex_byte(b'0', b'D'), 0x0d);
    }

    #[test]
    fn parse_shortforms_expand_per_css() {
        assert_eq!(Rgba::parse("#abc").unwrap(), Rgba(0xaa, 0xbb, 0xcc, 0xff));
        assert_eq!(Rgba::parse("#abcd").unwrap(), Rgba(0xaa, 0xbb, 0xcc, 0xdd));
        assert_eq!(Rgba::parse("00d8ff").unwrap(), Rgba(0x00, 0xd8, 0xff, 0xff));
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
