//! kittui-affordances
//!
//! Higher-level visual patterns over the kittui primitives. This crate
//! exists because DESIGN.md commits the library to staying primitive-only
//! — but real consumers (the CLI, the showcase, the Pi plugin layer)
//! keep needing the same handful of compositions: tonal panels, chips,
//! dividers, titles, the assistant/tool/user palette family.
//!
//! Anything here is *opt-in*. The `kittui` crate never imports this. If
//! you want primitives, use `kittui`. If you want batteries, depend on
//! `kittui-affordances`.
//!
//! v0.1 ships the small affordance set used by the showcase example.
//! Hosts adding their own affordances should follow the convention:
//! each affordance is a free function that returns either a `Scene` or
//! a ratakittui `Chrome`, never a custom widget type.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

pub mod components;
pub mod inline;
pub mod markdown;
pub mod palette;
pub mod panel;
pub mod table;

pub use components::{
    banner, footer, h1, h2, h3, header, textbox, textchip, title, ComponentKind, UiComponent,
};
pub use inline::{chip_chrome, divider_chrome, title_chrome};
pub use markdown::{
    render_markdown, HeadingOutline, LinkChip, MarkdownDefinition, MarkdownDocument,
    MarkdownFootnote, MarkdownImage, MarkdownMath, MarkdownMathKind,
};
pub use palette::{Palette, Tone};
pub use panel::{panel_chrome, PanelOptions};
pub use table::{
    box_glyph_scene, relative_cell_options, BoxGlyphCell, MarkdownTable, MarkdownTableAlignment,
    TableGlyphLayout,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_cover_three_tones_and_lerp_endpoints() {
        for tone in [Tone::Assistant, Tone::Tool, Tone::User] {
            let p = Palette::for_tone(tone);
            assert!(p.bg_top.3 > 0);
            assert!(p.bg_bottom.3 > 0);
            assert!(p.rail.3 > 0);
        }
    }
}
