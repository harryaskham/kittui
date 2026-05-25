//! Unified theme tokens for higher-level kittui affordances.
//!
//! This module is intentionally outside `kittui-core`: it bridges existing
//! affordance palettes, inline tokens, panels, controls, markdown components,
//! and kittwm chrome without turning the primitive scene crate into a design
//! system.

use kittui::Rgba;

use crate::inline_theme::{InlineAccentPalette, InlineChipColors, InlineStyle, InlineTheme};
use crate::palette::{Palette, Tone};

/// Semantic color role shared by affordance consumers.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ThemeRole {
    /// Main surface/background fill.
    Surface,
    /// Alternate elevated surface.
    SurfaceAlt,
    /// Border/accent rail.
    Border,
    /// Focus ring / selected affordance.
    Focus,
    /// Subtle highlight or glass reflection.
    Highlight,
    /// Primary text.
    Text,
    /// Muted/subtle text.
    MutedText,
    /// Success/positive accent.
    Success,
    /// Warning/caution accent.
    Warning,
    /// Danger/error accent.
    Danger,
}

/// Unified resolved tokens for optional affordance components.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffordanceTheme {
    /// Theme family backing inline/chrome defaults.
    pub inline_theme: InlineTheme,
    /// Tone backing legacy panel/control palettes.
    pub tone: Tone,
    /// Legacy tone palette, retained for compatibility.
    pub palette: Palette,
    /// Glass inline chip colors.
    pub glass: InlineChipColors,
    /// Chrome inline chip colors.
    pub chrome: InlineChipColors,
    /// Metal inline chip colors.
    pub metal: InlineChipColors,
    /// Neon/focus inline chip colors.
    pub neon: InlineChipColors,
    /// Multi-surface accent cycle.
    pub accents: InlineAccentPalette,
    /// Warning color.
    pub warning: Rgba,
    /// Danger color.
    pub danger: Rgba,
    /// Success color.
    pub success: Rgba,
}

impl AffordanceTheme {
    /// Resolve the default theme for a tone.
    pub fn for_tone(tone: Tone) -> Self {
        let inline_theme = InlineTheme::Nord;
        Self {
            inline_theme,
            tone,
            palette: Palette::for_tone(tone),
            glass: InlineChipColors::resolve(inline_theme, InlineStyle::Glass),
            chrome: InlineChipColors::resolve(inline_theme, InlineStyle::Chrome),
            metal: InlineChipColors::resolve(inline_theme, InlineStyle::Metal),
            neon: InlineChipColors::resolve(inline_theme, InlineStyle::Neon),
            accents: InlineAccentPalette::resolve(inline_theme),
            warning: Rgba::parse("#ebcb8b").expect("default warning color"),
            danger: Rgba::parse("#bf616a").expect("default danger color"),
            success: Rgba::parse("#a3be8c").expect("default success color"),
        }
    }

    /// Resolve a semantic color role.
    pub fn color(&self, role: ThemeRole) -> Rgba {
        match role {
            ThemeRole::Surface => self.palette.bg_top,
            ThemeRole::SurfaceAlt => self.palette.bg_bottom,
            ThemeRole::Border => self.palette.rail,
            ThemeRole::Focus => self.neon.border,
            ThemeRole::Highlight => self.glass.highlight,
            ThemeRole::Text => self.glass.fg,
            ThemeRole::MutedText => self.metal.border,
            ThemeRole::Success => self.success,
            ThemeRole::Warning => self.warning,
            ThemeRole::Danger => self.danger,
        }
    }

    /// Inline chip colors for a style.
    pub fn inline_colors(&self, style: InlineStyle) -> InlineChipColors {
        match style {
            InlineStyle::Glass => self.glass,
            InlineStyle::Chrome => self.chrome,
            InlineStyle::Metal => self.metal,
            InlineStyle::Neon => self.neon,
        }
    }

    /// Back-compat legacy tonal palette.
    pub fn palette(&self) -> Palette {
        self.palette
    }
}

impl Default for AffordanceTheme {
    fn default() -> Self {
        Self::for_tone(Tone::Assistant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_theme_bridges_palette_inline_and_accents() {
        let theme = AffordanceTheme::for_tone(Tone::Assistant);
        assert_eq!(theme.palette(), Palette::for_tone(Tone::Assistant));
        assert_eq!(
            theme.inline_colors(InlineStyle::Glass),
            InlineChipColors::resolve(InlineTheme::Nord, InlineStyle::Glass)
        );
        assert_eq!(theme.color(ThemeRole::Border), theme.palette.rail);
        assert_eq!(theme.color(ThemeRole::Focus), theme.neon.border);
        assert_eq!(
            theme.accents.color(0),
            InlineAccentPalette::resolve(InlineTheme::Nord).color(0)
        );
    }
}
