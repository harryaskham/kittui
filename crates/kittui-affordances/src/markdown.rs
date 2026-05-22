//! Markdown-to-kittui component rendering.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::components::{banner, h1, h2, h3, textbox, textchip, UiComponent};
use crate::palette::Tone;
use crate::table::MarkdownTable;

/// Rendered markdown document as semantic kittui UI components.
#[derive(Clone, Debug, Default)]
pub struct MarkdownDocument {
    /// Components in document order.
    pub components: Vec<UiComponent>,
    /// Link targets discovered while rendering.
    pub links: Vec<LinkChip>,
    /// Parsed markdown tables in document order.
    pub tables: Vec<MarkdownTable>,
    /// Image placeholders discovered while rendering.
    pub images: Vec<MarkdownImage>,
    /// Heading outline entries in document order.
    pub outline: Vec<HeadingOutline>,
    /// Footnote definitions in document order.
    pub footnotes: Vec<MarkdownFootnote>,
}

/// Link rendered as a highlighted chip plus accessible URL metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkChip {
    /// Visible label.
    pub label: String,
    /// Target URL.
    pub url: String,
}

/// Image rendered as an inline placeholder plus accessible URL metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownImage {
    /// Alt text, or the URL when the alt text is empty.
    pub alt: String,
    /// Target image URL/path.
    pub url: String,
}

/// One entry in a Markdown heading outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadingOutline {
    /// Heading level, 1 through 6.
    pub level: u8,
    /// Plain rendered heading text.
    pub text: String,
}

/// One rendered Markdown footnote definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownFootnote {
    /// Footnote label without surrounding `[^...]`.
    pub label: String,
    /// Rendered footnote text.
    pub text: String,
}

#[derive(Clone, Debug)]
struct ListState {
    next_number: Option<u64>,
}

impl ListState {
    fn next_marker(&mut self) -> String {
        if let Some(next) = &mut self.next_number {
            let marker = format!("{next}.");
            *next = next.saturating_add(1);
            marker
        } else {
            "•".to_string()
        }
    }
}

/// Render markdown into semantic kittui components.
pub fn render_markdown(src: &str, width_cells: u16) -> MarkdownDocument {
    let parser = Parser::new_ext(
        src,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES,
    );
    let mut out = MarkdownDocument::default();
    let mut buf = String::new();
    let mut heading: Option<HeadingLevel> = None;
    let mut in_code = false;
    let mut code_label: Option<String> = None;
    let mut link_target: Option<String> = None;
    let mut link_label = String::new();
    let mut image_target: Option<String> = None;
    let mut image_alt = String::new();
    let mut footnote_definition: Option<String> = None;
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut table_row: Vec<String> = Vec::new();
    let mut table_cell = String::new();
    let mut list_stack: Vec<ListState> = Vec::new();
    let mut in_list_item = false;
    let mut blockquote_depth = 0usize;

    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(level)) => {
                let text = take_trimmed(&mut buf);
                out.outline.push(HeadingOutline {
                    level: heading_level_number(level),
                    text: text.clone(),
                });
                let comp = match level {
                    HeadingLevel::H1 => h1(text, width_cells),
                    HeadingLevel::H2 => h2(text, width_cells),
                    _ => h3(text, width_cells),
                };
                out.components.push(comp);
                heading = None;
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !in_list_item && blockquote_depth == 0 && footnote_definition.is_none() {
                    flush_paragraph(&mut out, &mut buf, width_cells);
                }
            }
            Event::Start(Tag::List(start)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                list_stack.push(ListState { next_number: start });
            }
            Event::End(TagEnd::List(_)) => {
                if in_list_item {
                    flush_list_item(&mut out, &mut buf, width_cells, &mut list_stack);
                    in_list_item = false;
                }
                let _ = list_stack.pop();
            }
            Event::Start(Tag::Item) => {
                buf.clear();
                in_list_item = true;
            }
            Event::End(TagEnd::Item) => {
                flush_list_item(&mut out, &mut buf, width_cells, &mut list_stack);
                in_list_item = false;
            }
            Event::Start(Tag::FootnoteDefinition(label)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                footnote_definition = Some(label.to_string());
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                if let Some(label) = footnote_definition.take() {
                    let text = take_trimmed(&mut buf);
                    if !text.is_empty() {
                        out.footnotes.push(MarkdownFootnote {
                            label: label.clone(),
                            text: text.clone(),
                        });
                        out.components.push(textbox(
                            format!("footnote [^{label}]: {text}"),
                            width_cells,
                            Tone::Tool,
                        ));
                    }
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                blockquote_depth = blockquote_depth.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                blockquote_depth = blockquote_depth.saturating_sub(1);
                let text = take_trimmed(&mut buf);
                if !text.is_empty() {
                    out.components.push(banner(text, width_cells, Tone::Tool));
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                code_label = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_string)
                    }
                    CodeBlockKind::Indented => None,
                };
                in_code = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                let text = take_trimmed(&mut buf);
                if !text.is_empty() {
                    let rendered = code_label
                        .take()
                        .filter(|label| !label.is_empty())
                        .map(|label| format!("code:{label}\n{text}"))
                        .unwrap_or(text);
                    out.components
                        .push(textbox(rendered, width_cells, Tone::Tool));
                }
                in_code = false;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_target = Some(dest_url.to_string());
                link_label.clear();
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                image_target = Some(dest_url.to_string());
                image_alt.clear();
            }
            Event::End(TagEnd::Image) => {
                if let Some(url) = image_target.take() {
                    let alt = if image_alt.trim().is_empty() {
                        url.clone()
                    } else {
                        image_alt.trim().to_string()
                    };
                    let placeholder = format!("image: {alt} -> {url}");
                    out.images.push(MarkdownImage { alt, url });
                    if in_table {
                        table_cell.push_str(&placeholder);
                    } else {
                        buf.push_str(&placeholder);
                    }
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some(url) = link_target.take() {
                    let label = if link_label.trim().is_empty() {
                        url.clone()
                    } else {
                        link_label.trim().to_string()
                    };
                    out.links.push(LinkChip {
                        label: label.clone(),
                        url: url.clone(),
                    });
                    out.components.push(textchip(label, Tone::User));
                }
            }
            Event::Start(Tag::Table(_)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                in_table = true;
                table_rows.clear();
            }
            Event::Start(Tag::TableHead) => table_row.clear(),
            Event::End(TagEnd::Table) => {
                let table = MarkdownTable::new(table_rows.clone());
                let text = table_text(&table);
                out.components
                    .push(textbox(text, width_cells, Tone::Assistant));
                out.tables.push(table);
                in_table = false;
            }
            Event::End(TagEnd::TableHead) => {
                if !table_row.is_empty() {
                    table_rows.push(table_row.clone());
                }
                table_row.clear();
            }
            Event::Start(Tag::TableRow) => table_row.clear(),
            Event::End(TagEnd::TableRow) => {
                if !table_row.is_empty() {
                    table_rows.push(table_row.clone());
                }
                table_row.clear();
            }
            Event::Start(Tag::TableCell) => table_cell.clear(),
            Event::End(TagEnd::TableCell) => {
                table_row.push(table_cell.trim().to_string());
                table_cell.clear();
            }
            Event::Start(Tag::Emphasis) | Event::End(TagEnd::Emphasis) => {
                push_inline_marker(
                    "*",
                    in_table,
                    link_target.is_some(),
                    &mut table_cell,
                    &mut buf,
                    &mut link_label,
                );
            }
            Event::Start(Tag::Strong) | Event::End(TagEnd::Strong) => {
                push_inline_marker(
                    "**",
                    in_table,
                    link_target.is_some(),
                    &mut table_cell,
                    &mut buf,
                    &mut link_label,
                );
            }
            Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough) => {
                push_inline_marker(
                    "~~",
                    in_table,
                    link_target.is_some(),
                    &mut table_cell,
                    &mut buf,
                    &mut link_label,
                );
            }
            Event::Text(t) => {
                if image_target.is_some() {
                    image_alt.push_str(&t);
                    continue;
                }
                if link_target.is_some() {
                    link_label.push_str(&t);
                }
                if in_table {
                    table_cell.push_str(&t);
                } else {
                    buf.push_str(&t);
                }
            }
            Event::Code(t) => {
                if image_target.is_some() {
                    image_alt.push_str(&t);
                    continue;
                }
                if link_target.is_some() {
                    link_label.push_str(&t);
                }
                if in_table {
                    table_cell.push('`');
                    table_cell.push_str(&t);
                    table_cell.push('`');
                } else {
                    buf.push('`');
                    buf.push_str(&t);
                    buf.push('`');
                }
            }
            Event::InlineHtml(html) => {
                let html = html.trim();
                let placeholder = if html.starts_with("</") {
                    html.to_string()
                } else {
                    format!("html:{html}")
                };
                if link_target.is_some() {
                    link_label.push_str(&placeholder);
                }
                if in_table {
                    table_cell.push_str(&placeholder);
                } else {
                    buf.push_str(&placeholder);
                }
            }
            Event::Html(html) => {
                if in_table {
                    table_cell.push_str(&format!("html:{}", html.trim()));
                } else {
                    flush_paragraph(&mut out, &mut buf, width_cells);
                    let text = html.trim();
                    if !text.is_empty() {
                        out.components.push(textbox(
                            format!("html:\n{text}"),
                            width_cells,
                            Tone::Tool,
                        ));
                    }
                }
            }
            Event::FootnoteReference(label) => {
                let marker = format!("[^{label}]");
                if link_target.is_some() {
                    link_label.push_str(&marker);
                }
                if in_table {
                    table_cell.push_str(&marker);
                } else {
                    buf.push_str(&marker);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_table {
                    table_cell.push(' ');
                } else {
                    buf.push('\n');
                }
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                if in_table {
                    table_cell.push_str(marker);
                } else {
                    buf.push_str(marker);
                }
            }
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

fn heading_level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn push_inline_marker(
    marker: &str,
    in_table: bool,
    in_link: bool,
    table_cell: &mut String,
    buf: &mut String,
    link_label: &mut String,
) {
    if in_link {
        link_label.push_str(marker);
    }
    if in_table {
        table_cell.push_str(marker);
    } else {
        buf.push_str(marker);
    }
}

fn table_text(table: &MarkdownTable) -> String {
    let mut out = String::from("table\n");
    for row in &table.rows {
        out.push_str(&row.join(" | "));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn flush_paragraph(out: &mut MarkdownDocument, buf: &mut String, width_cells: u16) {
    let text = take_trimmed(buf);
    if !text.is_empty() {
        out.components
            .push(textbox(text, width_cells, Tone::Assistant));
    }
}

fn flush_list_item(
    out: &mut MarkdownDocument,
    buf: &mut String,
    width_cells: u16,
    list_stack: &mut [ListState],
) {
    let text = take_trimmed(buf);
    if text.is_empty() {
        return;
    }
    let marker = list_stack
        .last_mut()
        .map(ListState::next_marker)
        .unwrap_or_else(|| "•".to_string());
    out.components.push(textbox(
        format!("{marker} {text}"),
        width_cells,
        Tone::Assistant,
    ));
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
        let doc = render_markdown(
            "# Title\n\n## Section\n\nhello [site](https://example.com) world",
            60,
        );
        assert_eq!(doc.components[0].kind, ComponentKind::H1);
        assert_eq!(
            doc.outline[0],
            HeadingOutline {
                level: 1,
                text: "Title".to_string()
            }
        );
        assert_eq!(
            doc.outline[1],
            HeadingOutline {
                level: 2,
                text: "Section".to_string()
            }
        );
        assert!(doc
            .components
            .iter()
            .any(|c| c.kind == ComponentKind::TextChip && c.text == "site"));
        assert!(doc
            .components
            .iter()
            .any(|c| c.kind == ComponentKind::TextBox && c.text.contains("hello site world")));
        assert_eq!(doc.links[0].url, "https://example.com");
    }

    #[test]
    fn markdown_renders_table_as_textbox_and_metadata() {
        let doc = render_markdown("| a | b |\n|---|---|\n| 1 | 2 |", 60);
        assert!(doc.components.iter().any(|c| c.text.contains("1 | 2")));
        assert_eq!(doc.tables.len(), 1);
        assert_eq!(doc.tables[0].rows[0], vec!["a", "b"]);
        assert_eq!(doc.tables[0].rows[1], vec!["1", "2"]);
    }

    #[test]
    fn markdown_renders_unordered_and_ordered_list_markers() {
        let doc = render_markdown("- alpha\n- beta\n\n3. gamma\n4. delta", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("• alpha"), "{text}");
        assert!(text.contains("• beta"), "{text}");
        assert!(text.contains("3. gamma"), "{text}");
        assert!(text.contains("4. delta"), "{text}");
    }

    #[test]
    fn markdown_renders_task_list_markers() {
        let doc = render_markdown("- [ ] todo\n- [x] done", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("• [ ] todo"), "{text}");
        assert!(text.contains("• [x] done"), "{text}");
    }

    #[test]
    fn markdown_renders_code_fence_language_label() {
        let doc = render_markdown("```rust\nfn main() {}\n```\n\n```\nplain\n```", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(text.contains("code:rust\nfn main() {}"), "{text}");
        assert!(text.contains("\n---\nplain"), "{text}");
    }

    #[test]
    fn markdown_preserves_emphasis_strong_and_strikethrough_markers() {
        let doc = render_markdown("This is *em* and **strong** and ~~gone~~.", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("*em*"), "{text}");
        assert!(text.contains("**strong**"), "{text}");
        assert!(text.contains("~~gone~~"), "{text}");
    }

    #[test]
    fn markdown_renders_image_placeholders_and_metadata() {
        let doc = render_markdown("Logo: ![kittui logo](assets/logo.png)", 60);
        assert_eq!(doc.images.len(), 1);
        assert_eq!(doc.images[0].alt, "kittui logo");
        assert_eq!(doc.images[0].url, "assets/logo.png");
        assert!(doc.components.iter().any(|c| c
            .text
            .contains("Logo: image: kittui logo -> assets/logo.png")));
    }

    #[test]
    fn markdown_renders_blockquote_as_banner_not_textbox() {
        let doc = render_markdown("> quoted callout", 60);
        assert_eq!(doc.components.len(), 1);
        assert_eq!(doc.components[0].kind, ComponentKind::Banner);
        assert_eq!(doc.components[0].text, "quoted callout");
    }

    #[test]
    fn markdown_preserves_inline_and_block_html_placeholders() {
        let doc = render_markdown("hello <kbd>x</kbd>\n\n<div>block</div>", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(text.contains("hello html:<kbd>x</kbd>"), "{text}");
        assert!(text.contains("html:\n<div>block</div>"), "{text}");
    }

    #[test]
    fn markdown_preserves_footnote_references() {
        let doc = render_markdown("see this[^note]\n\n[^note]: details", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("see this[^note]"), "{text}");
    }

    #[test]
    fn markdown_renders_footnote_definitions() {
        let doc = render_markdown("see this[^note]\n\n[^note]: details here", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("footnote [^note]: details here"), "{text}");
        assert_eq!(
            doc.footnotes[0],
            MarkdownFootnote {
                label: "note".to_string(),
                text: "details here".to_string(),
            }
        );
        assert!(doc
            .components
            .iter()
            .any(|c| c.kind == ComponentKind::TextBox && c.text.starts_with("footnote")));
    }
}
