//! Higher-level UI components for rich kittui documents and overlays.
//!
//! These are intentionally lightweight: they provide deterministic sizing,
//! labels, tones, and chrome metadata that renderers (ratakittui, markdown
//! viewer, kittwm overlays) can turn into scenes or terminal widgets.

use kittui::Rgba;
use ratakittui::{Background, Border, Chrome, Padding, Shadow};

use crate::palette::{Palette, Tone};

/// Semantic component style.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ComponentKind {
    /// Paragraph/text container.
    TextBox,
    /// Level-1 heading.
    H1,
    /// Level-2 heading.
    H2,
    /// Level-3 heading.
    H3,
    /// Document title.
    Title,
    /// Prominent banner/callout.
    Banner,
    /// Header bar.
    Header,
    /// Footer bar.
    Footer,
    /// Inline chip.
    TextChip,
}

/// Concrete component payload + chrome.
#[derive(Clone, Debug)]
pub struct UiComponent {
    /// Semantic kind.
    pub kind: ComponentKind,
    /// Text payload.
    pub text: String,
    /// Suggested width in terminal cells.
    pub width_cells: u16,
    /// Suggested height in terminal cells.
    pub height_cells: u16,
    /// Visual chrome for renderers that consume ratakittui styles.
    pub chrome: Chrome,
}

impl UiComponent {
    /// Build a textbox with wrapping width and tone.
    pub fn textbox(text: impl Into<String>, width_cells: u16, tone: Tone) -> Self {
        let text = text.into();
        let height_cells = wrapped_height(&text, width_cells.saturating_sub(4).max(1)) + 2;
        Self {
            kind: ComponentKind::TextBox,
            text,
            width_cells,
            height_cells,
            chrome: card_chrome(tone).padding(Padding::trbl(1, 2, 1, 2)),
        }
    }

    /// Build a heading at level 1, 2, or 3.
    pub fn heading(level: u8, text: impl Into<String>, width_cells: u16) -> Self {
        let kind = match level {
            1 => ComponentKind::H1,
            2 => ComponentKind::H2,
            _ => ComponentKind::H3,
        };
        let tone = match kind {
            ComponentKind::H1 => Tone::Assistant,
            ComponentKind::H2 => Tone::User,
            _ => Tone::Tool,
        };
        let height_cells = if level == 1 { 3 } else { 2 };
        Self {
            kind,
            text: text.into(),
            width_cells,
            height_cells,
            chrome: bar_chrome(tone),
        }
    }

    /// Build a document title.
    pub fn title(text: impl Into<String>, width_cells: u16) -> Self {
        Self::bar(ComponentKind::Title, text, width_cells, Tone::Assistant, 3)
    }

    /// Build a banner/callout.
    pub fn banner(text: impl Into<String>, width_cells: u16, tone: Tone) -> Self {
        Self::bar(ComponentKind::Banner, text, width_cells, tone, 3)
    }

    /// Build a header bar.
    pub fn header(text: impl Into<String>, width_cells: u16) -> Self {
        Self::bar(ComponentKind::Header, text, width_cells, Tone::Assistant, 2)
    }

    /// Build a footer bar.
    pub fn footer(text: impl Into<String>, width_cells: u16) -> Self {
        Self::bar(ComponentKind::Footer, text, width_cells, Tone::Tool, 2)
    }

    /// Build an inline text chip.
    pub fn textchip(text: impl Into<String>, tone: Tone) -> Self {
        let text = text.into();
        let width_cells = (text.chars().count() as u16).saturating_add(4).max(4);
        Self {
            kind: ComponentKind::TextChip,
            text,
            width_cells,
            height_cells: 1,
            chrome: chip_chrome(tone),
        }
    }

    fn bar(
        kind: ComponentKind,
        text: impl Into<String>,
        width_cells: u16,
        tone: Tone,
        height_cells: u16,
    ) -> Self {
        Self {
            kind,
            text: text.into(),
            width_cells,
            height_cells,
            chrome: bar_chrome(tone),
        }
    }
}

/// Convenience textbox constructor.
pub fn textbox(text: impl Into<String>, width_cells: u16, tone: Tone) -> UiComponent {
    UiComponent::textbox(text, width_cells, tone)
}

/// Convenience h1 constructor.
pub fn h1(text: impl Into<String>, width_cells: u16) -> UiComponent {
    UiComponent::heading(1, text, width_cells)
}

/// Convenience h2 constructor.
pub fn h2(text: impl Into<String>, width_cells: u16) -> UiComponent {
    UiComponent::heading(2, text, width_cells)
}

/// Convenience h3 constructor.
pub fn h3(text: impl Into<String>, width_cells: u16) -> UiComponent {
    UiComponent::heading(3, text, width_cells)
}

/// Convenience title constructor.
pub fn title(text: impl Into<String>, width_cells: u16) -> UiComponent {
    UiComponent::title(text, width_cells)
}

/// Convenience banner constructor.
pub fn banner(text: impl Into<String>, width_cells: u16, tone: Tone) -> UiComponent {
    UiComponent::banner(text, width_cells, tone)
}

/// Convenience header constructor.
pub fn header(text: impl Into<String>, width_cells: u16) -> UiComponent {
    UiComponent::header(text, width_cells)
}

/// Convenience footer constructor.
pub fn footer(text: impl Into<String>, width_cells: u16) -> UiComponent {
    UiComponent::footer(text, width_cells)
}

/// Convenience text chip constructor.
pub fn textchip(text: impl Into<String>, tone: Tone) -> UiComponent {
    UiComponent::textchip(text, tone)
}

fn wrapped_height(text: &str, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    let mut rows = 1usize;
    let mut col = 0usize;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if col > 0 && col + 1 + len > width {
            rows += 1;
            col = len;
        } else {
            col += if col == 0 { len } else { len + 1 };
        }
    }
    rows as u16
}

fn card_chrome(tone: Tone) -> Chrome {
    let p = Palette::for_tone(tone);
    Chrome::default()
        .background(Background::Linear {
            direction: kittui::Direction::Vertical,
            start: p.bg_top,
            end: p.bg_bottom,
        })
        .border(Border::rounded(p.rail, 1.0, 6.0))
        .shadow(Shadow {
            dx_px: 2.0,
            dy_px: 2.0,
            color: Rgba::rgba(0, 0, 0, 90),
        })
}

fn bar_chrome(tone: Tone) -> Chrome {
    let p = Palette::for_tone(tone);
    Chrome::default()
        .background(Background::Linear {
            direction: kittui::Direction::Horizontal,
            start: p.rail,
            end: p.bg_top,
        })
        .border(Border::rounded(p.rail, 1.0, 4.0))
        .padding(Padding::trbl(0, 1, 0, 1))
}

fn chip_chrome(tone: Tone) -> Chrome {
    let p = Palette::for_tone(tone);
    Chrome::default()
        .background(Background::Solid(p.bg_top))
        .border(Border::rounded(p.rail, 1.0, 7.0))
        .padding(Padding::trbl(0, 1, 0, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_have_expected_kinds_and_sizes() {
        assert_eq!(h1("Hello", 40).kind, ComponentKind::H1);
        assert_eq!(h2("Hello", 40).height_cells, 2);
        assert_eq!(h3("Hello", 40).kind, ComponentKind::H3);
        assert_eq!(title("Doc", 80).height_cells, 3);
        assert_eq!(header("top", 80).kind, ComponentKind::Header);
        assert_eq!(footer("bottom", 80).kind, ComponentKind::Footer);
        assert_eq!(banner("note", 80, Tone::User).kind, ComponentKind::Banner);
        let chip = textchip("link", Tone::Tool);
        assert_eq!(chip.kind, ComponentKind::TextChip);
        assert!(chip.width_cells >= 8);
    }

    #[test]
    fn textbox_wraps_height() {
        let boxy = textbox("one two three four five six", 12, Tone::Assistant);
        assert_eq!(boxy.kind, ComponentKind::TextBox);
        assert!(boxy.height_cells > 3);
    }
}
