//! Theme/style tokens for inline one-line components.
//!
//! These tokens are intentionally reusable outside the `kittui` CLI: shell
//! prompt helpers, tmux statusline generators, kittwm chrome, and future
//! inline renderers should all resolve the same defaults here.

use kittui::Rgba;
use kittui_core::color::ColorParseError;

/// Inline theme family.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum InlineTheme {
    /// Nord-inspired palette.
    Nord,
}

/// Inline visual style within a theme.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum InlineStyle {
    /// Translucent glass fill, bright stroke, subtle highlight.
    Glass,
    /// More opaque glossy chrome.
    Chrome,
    /// Neutral high-contrast metal.
    Metal,
    /// Dark fill with saturated neon accent.
    Neon,
}

/// Resolved colors for inline chip-like components.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct InlineChipColors {
    /// Chip fill color.
    pub fill: Rgba,
    /// Chip border/stroke color.
    pub border: Rgba,
    /// Top highlight color.
    pub highlight: Rgba,
    /// Foreground text color.
    pub fg: Rgba,
}

impl InlineChipColors {
    /// Resolve default colors for a theme/style pair.
    pub fn resolve(theme: InlineTheme, style: InlineStyle) -> Self {
        match (theme, style) {
            (InlineTheme::Nord, InlineStyle::Glass) => Self {
                fill: Rgba::rgba(46, 52, 64, 175),
                border: Rgba::rgba(136, 192, 208, 230),
                highlight: Rgba::rgba(236, 239, 244, 70),
                fg: Rgba::rgb(236, 239, 244),
            },
            (InlineTheme::Nord, InlineStyle::Chrome) => Self {
                fill: Rgba::rgba(59, 66, 82, 230),
                border: Rgba::rgba(129, 161, 193, 255),
                highlight: Rgba::rgba(216, 222, 233, 95),
                fg: Rgba::rgb(236, 239, 244),
            },
            (InlineTheme::Nord, InlineStyle::Metal) => Self {
                fill: Rgba::rgba(67, 76, 94, 235),
                border: Rgba::rgba(216, 222, 233, 240),
                highlight: Rgba::rgba(236, 239, 244, 80),
                fg: Rgba::rgb(236, 239, 244),
            },
            (InlineTheme::Nord, InlineStyle::Neon) => Self {
                fill: Rgba::rgba(46, 52, 64, 205),
                border: Rgba::rgba(136, 192, 208, 255),
                highlight: Rgba::rgba(180, 142, 173, 90),
                fg: Rgba::rgb(236, 239, 244),
            },
        }
    }

    /// Apply optional fill/border/foreground overrides.
    pub fn with_overrides(
        mut self,
        fill: Option<Rgba>,
        border: Option<Rgba>,
        fg: Option<Rgba>,
    ) -> Self {
        if let Some(fill) = fill {
            self.fill = fill;
        }
        if let Some(border) = border {
            self.border = border;
        }
        if let Some(fg) = fg {
            self.fg = fg;
        }
        self
    }
}

/// Ordered source/backend accent colors for multi-surface chrome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineAccentPalette {
    colors: Vec<Rgba>,
}

impl InlineAccentPalette {
    /// Resolve the default accent cycle for a theme.
    pub fn resolve(theme: InlineTheme) -> Self {
        match theme {
            InlineTheme::Nord => Self {
                colors: vec![
                    parse_nord_inline_color("cyan").expect("default nord cyan accent"),
                    parse_nord_inline_color("purple").expect("default nord purple accent"),
                    parse_nord_inline_color("green").expect("default nord green accent"),
                    parse_nord_inline_color("yellow").expect("default nord yellow accent"),
                    parse_nord_inline_color("red").expect("default nord red accent"),
                    parse_nord_inline_color("blue").expect("default nord blue accent"),
                ],
            },
        }
    }

    /// Return the cycling accent for `index`.
    pub fn color(&self, index: usize) -> Rgba {
        self.colors[index % self.colors.len()]
    }

    /// Expose the deterministic ordered accent tokens.
    pub fn colors(&self) -> &[Rgba] {
        &self.colors
    }
}

/// Parse a color override for the Nord inline theme.
///
/// Accepts CSS hex literals, zero-based Nord palette indices, and a handful
/// of stable names (`bg`, `fg`, `frost`, `blue`, `red`, `orange`, `yellow`,
/// `green`, `purple`).
pub fn parse_nord_inline_color(value: &str) -> Result<Rgba, ColorParseError> {
    let nord = [
        "#2e3440cc",
        "#3b4252",
        "#434c5e",
        "#4c566a",
        "#d8dee9",
        "#e5e9f0",
        "#eceff4",
        "#8fbcbb",
        "#88c0d0",
        "#81a1c1",
        "#5e81ac",
        "#bf616a",
        "#d08770",
        "#ebcb8b",
        "#a3be8c",
        "#b48ead",
    ];
    if let Ok(index) = value.parse::<usize>() {
        if let Some(color) = nord.get(index) {
            return Rgba::parse(color);
        }
    }
    let named = match value.to_ascii_lowercase().as_str() {
        "bg" | "polar-night" => Some(nord[0]),
        "fg" | "snow" => Some(nord[6]),
        "frost" | "cyan" => Some(nord[8]),
        "blue" => Some(nord[10]),
        "red" => Some(nord[11]),
        "orange" => Some(nord[12]),
        "yellow" => Some(nord[13]),
        "green" => Some(nord[14]),
        "purple" => Some(nord[15]),
        _ => None,
    };
    Rgba::parse(named.unwrap_or(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nord_glass_defaults_are_translucent_and_high_contrast() {
        let colors = InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Glass);
        assert_eq!(colors.fill, Rgba::rgba(46, 52, 64, 175));
        assert_eq!(colors.border, Rgba::rgba(136, 192, 208, 230));
        assert_eq!(colors.fg, Rgba::rgb(236, 239, 244));
        assert!(colors.highlight.3 > 0);
    }

    #[test]
    fn nord_accent_palette_cycles_named_theme_tokens() {
        let palette = InlineAccentPalette::resolve(InlineTheme::Nord);
        assert_eq!(palette.colors().len(), 6);
        assert_eq!(palette.color(0), parse_nord_inline_color("cyan").unwrap());
        assert_eq!(palette.color(1), parse_nord_inline_color("purple").unwrap());
        assert_eq!(palette.color(6), palette.color(0));
    }

    #[test]
    fn nord_inline_color_overrides_accept_indices_names_and_hex() {
        assert_eq!(
            parse_nord_inline_color("8").unwrap(),
            Rgba::parse("#88c0d0").unwrap()
        );
        assert_eq!(
            parse_nord_inline_color("purple").unwrap(),
            Rgba::parse("#b48ead").unwrap()
        );
        assert_eq!(
            parse_nord_inline_color("#abcdef").unwrap(),
            Rgba::parse("#abcdef").unwrap()
        );
    }
}
