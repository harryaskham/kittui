//! Markdown-to-kittui component rendering.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::components::{banner, h1, h2, h3, textbox, textchip, UiComponent};
use crate::palette::Tone;

/// Rendered markdown document as semantic kittui UI components.
#[derive(Clone, Debug, Default)]
pub struct MarkdownDocument {
    /// Components in document order.
    pub components: Vec<UiComponent>,
    /// Link targets discovered while rendering.
    pub links: Vec<LinkChip>,
}

/// Link rendered as a highlighted chip plus accessible URL metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkChip {
    /// Visible label.
    pub label: String,
    /// Target URL.
    pub url: String,
}

/// Render markdown into semantic kittui components.
pub fn render_markdown(src: &str, width_cells: u16) -> MarkdownDocument {
    let parser = Parser::new_ext(src, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    let mut out = MarkdownDocument::default();
    let mut buf = String::new();
    let mut heading: Option<HeadingLevel> = None;
    let mut in_code = false;
    let mut link_target: Option<String> = None;
    let mut link_label = String::new();
    let mut in_table = false;
    let mut table_row = String::new();

    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(level)) => {
                let text = take_trimmed(&mut buf);
                let comp = match level {
                    HeadingLevel::H1 => h1(text, width_cells),
                    HeadingLevel::H2 => h2(text, width_cells),
                    _ => h3(text, width_cells),
                };
                out.components.push(comp);
                heading = None;
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => flush_paragraph(&mut out, &mut buf, width_cells),
            Event::Start(Tag::BlockQuote(_)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                let text = take_trimmed(&mut buf);
                if !text.is_empty() {
                    out.components.push(banner(text, width_cells, Tone::Tool));
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                in_code = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                let text = take_trimmed(&mut buf);
                if !text.is_empty() {
                    out.components.push(textbox(text, width_cells, Tone::Tool));
                }
                in_code = false;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_target = Some(dest_url.to_string());
                link_label.clear();
            }
            Event::End(TagEnd::Link) => {
                if let Some(url) = link_target.take() {
                    let label = if link_label.trim().is_empty() { url.clone() } else { link_label.trim().to_string() };
                    out.links.push(LinkChip { label: label.clone(), url: url.clone() });
                    out.components.push(textchip(label, Tone::User));
                }
            }
            Event::Start(Tag::Table(_)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                in_table = true;
                buf.push_str("table\n");
            }
            Event::End(TagEnd::Table) => {
                if !table_row.is_empty() {
                    buf.push_str(&table_row);
                    table_row.clear();
                }
                let text = take_trimmed(&mut buf);
                if !text.is_empty() {
                    out.components.push(textbox(text, width_cells, Tone::Assistant));
                }
                in_table = false;
            }
            Event::End(TagEnd::TableRow) => {
                buf.push_str(table_row.trim_end_matches(" | "));
                buf.push('\n');
                table_row.clear();
            }
            Event::End(TagEnd::TableCell) => table_row.push_str(" | "),
            Event::Text(t) => {
                if link_target.is_some() {
                    link_label.push_str(&t);
                } else if in_table {
                    table_row.push_str(&t);
                } else {
                    buf.push_str(&t);
                }
            }
            Event::Code(t) => {
                buf.push('`');
                buf.push_str(&t);
                buf.push('`');
            }
            Event::SoftBreak | Event::HardBreak => buf.push('\n'),
            Event::Rule => out.components.push(banner("—", width_cells, Tone::Tool)),
            _ => {
                let _ = heading;
                let _ = in_code;
            }
        }
    }
    flush_paragraph(&mut out, &mut buf, width_cells);
    out
}

fn flush_paragraph(out: &mut MarkdownDocument, buf: &mut String, width_cells: u16) {
    let text = take_trimmed(buf);
    if !text.is_empty() {
        out.components.push(textbox(text, width_cells, Tone::Assistant));
    }
}

fn take_trimmed(buf: &mut String) -> String {
    let text = buf.trim().to_string();
    buf.clear();
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ComponentKind;

    #[test]
    fn markdown_renders_headings_paragraphs_and_link_chips() {
        let doc = render_markdown("# Title\n\nhello [site](https://example.com) world", 60);
        assert_eq!(doc.components[0].kind, ComponentKind::H1);
        assert!(doc.components.iter().any(|c| c.kind == ComponentKind::TextChip && c.text == "site"));
        assert_eq!(doc.links[0].url, "https://example.com");
    }

    #[test]
    fn markdown_renders_table_as_textbox() {
        let doc = render_markdown("| a | b |\n|---|---|\n| 1 | 2 |", 60);
        assert!(doc.components.iter().any(|c| c.text.contains("1 | 2")));
    }
}
