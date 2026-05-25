//! Tonal palette used by the v0.1 affordance set. Mirrors the
//! assistant / tool / user palette family the JS Pi implementation
//! exposes today.

use kittui::Rgba;

/// Conversation tone the palette is built around.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Tone {
    /// Assistant / agent message.
    Assistant,
    /// Tool output / system surface.
    Tool,
    /// User message.
    User,
}

/// Resolved palette colors for a tone.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Palette {
    /// Top color of the vertical background gradient.
    pub bg_top: Rgba,
    /// Bottom color of the vertical background gradient.
    pub bg_bottom: Rgba,
    /// Rail / border accent color.
    pub rail: Rgba,
    /// Glow color (alpha controls intensity ceiling).
    pub glow: Rgba,
}

impl Palette {
    /// Return the canonical palette for a tone. These match the values
    /// the showcase example uses today; hosts can clone and tweak.
    pub fn for_tone(tone: Tone) -> Self {
        let p = |s| Rgba::parse(s).unwrap();
        match tone {
            Tone::Assistant => Self {
                bg_top: p("#07111fff"),
                bg_bottom: p("#11192cff"),
                rail: p("#00d8ff"),
                glow: p("#00d8ffaa"),
            },
            Tone::Tool => Self {
                bg_top: p("#080d1bff"),
                bg_bottom: p("#171326ff"),
                rail: p("#b48cff"),
                glow: p("#b48cffaa"),
            },
            Tone::User => Self {
                bg_top: p("#061817ff"),
                bg_bottom: p("#0e202cff"),
                rail: p("#72fbd6"),
                glow: p("#72fbd6aa"),
            },
        }
    }
}
