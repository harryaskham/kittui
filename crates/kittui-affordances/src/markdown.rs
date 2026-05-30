//! Markdown-to-kittui component rendering.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, MetadataBlockKind, Options, Parser, Tag, TagEnd,
};

use crate::components::{banner, h1, h2, h3, textbox, textchip, UiComponent};
use crate::palette::Tone;
use crate::table::{MarkdownTable, MarkdownTableAlignment};

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
    /// Footnote reference labels in encounter order.
    pub footnote_references: Vec<String>,
    /// Definition-list entries in document order.
    pub definitions: Vec<MarkdownDefinition>,
    /// Math expressions in encounter order.
    pub math: Vec<MarkdownMath>,
    /// HTML placeholders in encounter order.
    pub html: Vec<MarkdownHtml>,
    /// Code blocks in encounter order.
    pub code_blocks: Vec<MarkdownCodeBlock>,
    /// Metadata blocks in encounter order.
    pub metadata_blocks: Vec<MarkdownMetadataBlock>,
}

/// Link rendered as a highlighted chip plus accessible URL metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkChip {
    /// Visible label.
    pub label: String,
    /// Target URL.
    pub url: String,
    /// Optional Markdown title attribute.
    pub title: Option<String>,
}

/// Image rendered as an inline placeholder plus accessible URL metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownImage {
    /// Alt text, or the URL when the alt text is empty.
    pub alt: String,
    /// Target image URL/path.
    pub url: String,
    /// Optional Markdown title attribute.
    pub title: Option<String>,
}

/// One entry in a Markdown heading outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadingOutline {
    /// Heading level, 1 through 6.
    pub level: u8,
    /// Plain rendered heading text.
    pub text: String,
    /// Stable slug anchor derived from the heading text.
    pub anchor: String,
}

/// One rendered Markdown footnote definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownFootnote {
    /// Footnote label without surrounding `[^...]`.
    pub label: String,
    /// Rendered footnote text.
    pub text: String,
}

/// One rendered Markdown definition-list entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownDefinition {
    /// Definition term/title.
    pub term: String,
    /// Rendered definition body.
    pub definition: String,
}

/// Markdown math expression kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownMathKind {
    /// Inline math delimited with `$...$`.
    Inline,
    /// Display math delimited with `$$...$$`.
    Display,
}

impl MarkdownMathKind {
    /// Stable lowercase metadata string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Display => "display",
        }
    }
}

/// One rendered Markdown math expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownMath {
    /// Inline or display math.
    pub kind: MarkdownMathKind,
    /// Math source text.
    pub source: String,
}

/// Markdown HTML placeholder kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownHtmlKind {
    /// Inline HTML.
    Inline,
    /// Block HTML.
    Block,
}

impl MarkdownHtmlKind {
    /// Stable lowercase metadata string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Block => "block",
        }
    }
}

/// One preserved Markdown HTML fragment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownHtml {
    /// Inline or block HTML.
    pub kind: MarkdownHtmlKind,
    /// Source HTML text.
    pub source: String,
}

/// One rendered Markdown code block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownCodeBlock {
    /// Optional language/info label from fenced code.
    pub language: Option<String>,
    /// Code block source text.
    pub text: String,
}

/// Metadata/frontmatter block kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownMetadataBlockKind {
    /// YAML-style frontmatter (`---`).
    Yaml,
    /// Pluses-delimited metadata (`+++`).
    Pluses,
}

impl MarkdownMetadataBlockKind {
    /// Stable lowercase metadata string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yaml => "yaml",
            Self::Pluses => "pluses",
        }
    }
}

/// One preserved Markdown metadata/frontmatter block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownMetadataBlock {
    /// Metadata block delimiter style.
    pub kind: MarkdownMetadataBlockKind,
    /// Raw metadata source text.
    pub source: String,
}

#[derive(Clone, Debug)]
struct ListState {
    next_number: Option<u64>,
}

impl ListState {
    fn next_marker(&mut self) -> String {
        if let Some(next) = &mut self.next_number {
            let marker = ordered_list_marker(*next);
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
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_DEFINITION_LIST
            | Options::ENABLE_MATH
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS,
    );
    let mut out = MarkdownDocument::default();
    let mut buf = String::new();
    let mut heading: Option<HeadingLevel> = None;
    let mut heading_anchors: HashMap<String, usize> = HashMap::new();
    let mut in_code = false;
    let mut code_label: Option<String> = None;
    let mut metadata_block: Option<MarkdownMetadataBlockKind> = None;
    let mut link_target: Option<String> = None;
    let mut link_title: Option<String> = None;
    let mut link_label = String::new();
    let mut image_target: Option<String> = None;
    let mut image_title: Option<String> = None;
    let mut image_alt = String::new();
    let mut footnote_definition: Option<String> = None;
    let mut definition_term: Option<String> = None;
    let mut in_definition_title = false;
    let mut in_definition_body = false;
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut table_alignments: Vec<MarkdownTableAlignment> = Vec::new();
    let mut table_row: Vec<String> = Vec::new();
    let mut table_cell = String::new();
    let mut list_stack: Vec<ListState> = Vec::new();
    let mut in_list_item = false;
    let mut blockquote_depth = 0usize;

    for ev in parser {
        match ev {
            Event::Start(Tag::MetadataBlock(kind)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                metadata_block = Some(markdown_metadata_kind(kind));
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                if let Some(kind) = metadata_block.take() {
                    let source = take_trimmed(&mut buf);
                    if !source.is_empty() {
                        out.metadata_blocks.push(MarkdownMetadataBlock {
                            kind: kind.clone(),
                            source: source.clone(),
                        });
                        out.components.push(textbox(
                            metadata_block_text(kind.as_str(), &source),
                            width_cells,
                            Tone::Tool,
                        ));
                    }
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(level)) => {
                let text = take_trimmed(&mut buf);
                let anchor = unique_heading_anchor(&text, &mut heading_anchors);
                out.outline.push(HeadingOutline {
                    level: heading_level_number(level),
                    text: text.clone(),
                    anchor,
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
                if !in_list_item
                    && blockquote_depth == 0
                    && footnote_definition.is_none()
                    && metadata_block.is_none()
                    && !in_definition_title
                    && !in_definition_body
                {
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
            Event::Start(Tag::DefinitionList) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
            }
            Event::Start(Tag::DefinitionListTitle) => {
                buf.clear();
                in_definition_title = true;
            }
            Event::End(TagEnd::DefinitionListTitle) => {
                definition_term = Some(take_trimmed(&mut buf));
                in_definition_title = false;
            }
            Event::Start(Tag::DefinitionListDefinition) => {
                buf.clear();
                in_definition_body = true;
            }
            Event::End(TagEnd::DefinitionListDefinition) => {
                let definition = take_trimmed(&mut buf);
                let term = definition_term.take().unwrap_or_default();
                if !term.is_empty() || !definition.is_empty() {
                    out.definitions.push(MarkdownDefinition {
                        term: term.clone(),
                        definition: definition.clone(),
                    });
                    out.components.push(textbox(
                        definition_block_text(&term, &definition),
                        width_cells,
                        Tone::Assistant,
                    ));
                }
                in_definition_body = false;
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
                            footnote_definition_text(&label, &text),
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
                    let language = code_label.take().filter(|label| !label.is_empty());
                    out.code_blocks.push(MarkdownCodeBlock {
                        language: language.clone(),
                        text: text.clone(),
                    });
                    let rendered = language
                        .map(|label| code_block_text(&label, &text))
                        .unwrap_or(text);
                    out.components
                        .push(textbox(rendered, width_cells, Tone::Tool));
                }
                in_code = false;
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                link_target = Some(dest_url.to_string());
                link_title = non_empty_title(&title);
                link_label.clear();
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                image_target = Some(dest_url.to_string());
                image_title = non_empty_title(&title);
                image_alt.clear();
            }
            Event::End(TagEnd::Image) => {
                if let Some(url) = image_target.take() {
                    let alt = if image_alt.trim().is_empty() {
                        url.clone()
                    } else {
                        image_alt.trim().to_string()
                    };
                    let placeholder = image_placeholder_text(&alt, &url);
                    let title = image_title.take();
                    out.images.push(MarkdownImage { alt, url, title });
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
                    let title = link_title.take();
                    out.links.push(LinkChip {
                        label: label.clone(),
                        url: url.clone(),
                        title,
                    });
                    out.components.push(textchip(label, Tone::User));
                }
            }
            Event::Start(Tag::Table(alignments)) => {
                flush_paragraph(&mut out, &mut buf, width_cells);
                in_table = true;
                table_rows.clear();
                table_alignments = alignments.into_iter().map(markdown_alignment).collect();
            }
            Event::Start(Tag::TableHead) => table_row.clear(),
            Event::End(TagEnd::Table) => {
                let table =
                    MarkdownTable::with_alignments(table_rows.clone(), table_alignments.clone());
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
            Event::InlineMath(math) => {
                let source = math.to_string();
                out.math.push(MarkdownMath {
                    kind: MarkdownMathKind::Inline,
                    source: source.clone(),
                });
                let placeholder = prefixed_text("math:", &source);
                if link_target.is_some() {
                    link_label.push_str(&placeholder);
                }
                if in_table {
                    table_cell.push_str(&placeholder);
                } else {
                    buf.push_str(&placeholder);
                }
            }
            Event::DisplayMath(math) => {
                let math = math.trim();
                out.math.push(MarkdownMath {
                    kind: MarkdownMathKind::Display,
                    source: math.to_string(),
                });
                if in_table {
                    table_cell.push_str(&prefixed_text("math:", math));
                } else {
                    flush_paragraph(&mut out, &mut buf, width_cells);
                    out.components
                        .push(textbox(math_block_text(math), width_cells, Tone::Tool));
                }
            }
            Event::InlineHtml(html) => {
                let html = html.trim();
                out.html.push(MarkdownHtml {
                    kind: MarkdownHtmlKind::Inline,
                    source: html.to_string(),
                });
                let placeholder = if html.starts_with("</") {
                    html.to_string()
                } else {
                    html_inline_text(html)
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
                let text = html.trim();
                if !text.is_empty() {
                    out.html.push(MarkdownHtml {
                        kind: MarkdownHtmlKind::Block,
                        source: text.to_string(),
                    });
                }
                if in_table {
                    table_cell.push_str(&html_inline_text(text));
                } else {
                    flush_paragraph(&mut out, &mut buf, width_cells);
                    if !text.is_empty() {
                        out.components.push(textbox(
                            html_block_text(text),
                            width_cells,
                            Tone::Tool,
                        ));
                    }
                }
            }
            Event::FootnoteReference(label) => {
                out.footnote_references.push(label.to_string());
                let marker = footnote_marker(&label);
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

fn non_empty_title(title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn markdown_metadata_kind(kind: MetadataBlockKind) -> MarkdownMetadataBlockKind {
    match kind {
        MetadataBlockKind::YamlStyle => MarkdownMetadataBlockKind::Yaml,
        MetadataBlockKind::PlusesStyle => MarkdownMetadataBlockKind::Pluses,
    }
}

fn markdown_alignment(alignment: Alignment) -> MarkdownTableAlignment {
    match alignment {
        Alignment::None => MarkdownTableAlignment::None,
        Alignment::Left => MarkdownTableAlignment::Left,
        Alignment::Center => MarkdownTableAlignment::Center,
        Alignment::Right => MarkdownTableAlignment::Right,
    }
}

fn unique_heading_anchor(text: &str, seen: &mut HashMap<String, usize>) -> String {
    let base = heading_anchor(text);
    let count = seen.entry(base.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base
    } else {
        duplicate_heading_anchor(&base, *count)
    }
}

fn duplicate_heading_anchor(base: &str, count: usize) -> String {
    let mut anchor = String::with_capacity(base.len() + 1 + decimal_len(count as u64));
    anchor.push_str(base);
    anchor.push('-');
    write!(anchor, "{count}").expect("write to string");
    anchor
}

fn heading_anchor(text: &str) -> String {
    let mut anchor = String::new();
    let mut pending_dash = false;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            if pending_dash && !anchor.is_empty() {
                anchor.push('-');
            }
            anchor.push(ch);
            pending_dash = false;
        } else {
            pending_dash = true;
        }
    }
    if anchor.is_empty() {
        "section".to_string()
    } else {
        anchor
    }
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
    let widths = table.column_widths();
    for row in &table.rows {
        let cells = row
            .iter()
            .enumerate()
            .map(|(idx, cell)| {
                align_table_cell_text(
                    cell,
                    widths.get(idx).copied().unwrap_or(1) as usize,
                    table
                        .alignments
                        .get(idx)
                        .copied()
                        .unwrap_or(MarkdownTableAlignment::None),
                )
            })
            .collect::<Vec<_>>();
        out.push_str(&cells.join(" | "));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn align_table_cell_text(text: &str, width: usize, alignment: MarkdownTableAlignment) -> String {
    let truncated = text.chars().take(width).collect::<String>();
    let len = truncated.chars().count();
    if len >= width {
        return truncated;
    }
    let pad = width - len;
    match alignment {
        MarkdownTableAlignment::Right => format!("{}{}", " ".repeat(pad), truncated),
        MarkdownTableAlignment::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), truncated, " ".repeat(right))
        }
        MarkdownTableAlignment::None | MarkdownTableAlignment::Left => {
            format!("{}{}", truncated, " ".repeat(pad))
        }
    }
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
        list_item_text(&marker, &text),
        width_cells,
        Tone::Assistant,
    ));
}

fn html_inline_text(html: &str) -> String {
    prefixed_text("html:", html)
}

fn html_block_text(html: &str) -> String {
    prefixed_text("html:\n", html)
}

fn prefixed_text(prefix: &str, text: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + text.len());
    out.push_str(prefix);
    out.push_str(text);
    out
}

fn math_block_text(math: &str) -> String {
    prefixed_text("math:\n", math)
}

fn image_placeholder_text(alt: &str, url: &str) -> String {
    let mut placeholder = String::with_capacity("image:  -> ".len() + alt.len() + url.len());
    placeholder.push_str("image: ");
    placeholder.push_str(alt);
    placeholder.push_str(" -> ");
    placeholder.push_str(url);
    placeholder
}

fn code_block_text(label: &str, text: &str) -> String {
    let mut block = String::with_capacity("code:\n".len() + label.len() + text.len());
    block.push_str("code:");
    block.push_str(label);
    block.push('\n');
    block.push_str(text);
    block
}

fn footnote_definition_text(label: &str, text: &str) -> String {
    let mut definition = String::with_capacity("footnote [^]: ".len() + label.len() + text.len());
    definition.push_str("footnote [^");
    definition.push_str(label);
    definition.push_str("]: ");
    definition.push_str(text);
    definition
}

fn definition_block_text(term: &str, definition: &str) -> String {
    let mut text = String::with_capacity("definition: \n: ".len() + term.len() + definition.len());
    text.push_str("definition: ");
    text.push_str(term);
    text.push_str("\n: ");
    text.push_str(definition);
    text
}

fn metadata_block_text(kind: &str, source: &str) -> String {
    let mut text = String::with_capacity("metadata:".len() + kind.len() + 1 + source.len());
    text.push_str("metadata:");
    text.push_str(kind);
    text.push('\n');
    text.push_str(source);
    text
}

fn footnote_marker(label: &str) -> String {
    let mut marker = String::with_capacity("[^]".len() + label.len());
    marker.push_str("[^");
    marker.push_str(label);
    marker.push(']');
    marker
}

fn ordered_list_marker(next: u64) -> String {
    let mut marker = String::with_capacity(decimal_len(next) + 1);
    write!(marker, "{next}").expect("write to string");
    marker.push('.');
    marker
}

fn list_item_text(marker: &str, text: &str) -> String {
    let mut item = String::with_capacity(marker.len() + 1 + text.len());
    item.push_str(marker);
    item.push(' ');
    item.push_str(text);
    item
}

fn decimal_len(value: u64) -> usize {
    if value == 0 {
        return 1;
    }
    let mut n = value;
    let mut len = 0;
    while n > 0 {
        len += 1;
        n /= 10;
    }
    len
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
                text: "Title".to_string(),
                anchor: "title".to_string(),
            }
        );
        assert_eq!(
            doc.outline[1],
            HeadingOutline {
                level: 2,
                text: "Section".to_string(),
                anchor: "section".to_string(),
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
        assert_eq!(doc.links[0].title, None);
    }

    #[test]
    fn markdown_preserves_link_title_metadata() {
        let doc = render_markdown("See [site](https://example.com \"Example title\")", 60);
        assert_eq!(doc.links.len(), 1);
        assert_eq!(doc.links[0].label, "site");
        assert_eq!(doc.links[0].url, "https://example.com");
        assert_eq!(doc.links[0].title.as_deref(), Some("Example title"));
    }

    #[test]
    fn duplicate_heading_anchor_builds_directly() {
        let anchor = duplicate_heading_anchor("hello-world", 12);
        assert_eq!(anchor, "hello-world-12");
        assert!(anchor.capacity() >= anchor.len());
    }

    #[test]
    fn markdown_generates_stable_unique_heading_anchors() {
        let doc = render_markdown("# Hello, World!\n\n## Hello World\n\n## !!!", 60);
        assert_eq!(doc.outline[0].anchor, "hello-world");
        assert_eq!(doc.outline[1].anchor, "hello-world-2");
        assert_eq!(doc.outline[2].anchor, "section");
    }

    #[test]
    fn markdown_renders_table_as_textbox_and_metadata() {
        let doc = render_markdown(
            "| aa | b | c | dd |\n|---|:---|:---:|---:|\n| 1 | 2 | 3 | 4 |",
            60,
        );
        let table_component = doc
            .components
            .iter()
            .find(|c| c.text.starts_with("table\n"))
            .expect("table component");
        assert!(
            table_component.text.contains("1  | 2 | 3 |  4"),
            "{}",
            table_component.text
        );
        assert_eq!(doc.tables.len(), 1);
        assert_eq!(doc.tables[0].rows[0], vec!["aa", "b", "c", "dd"]);
        assert_eq!(doc.tables[0].rows[1], vec!["1", "2", "3", "4"]);
        assert_eq!(
            doc.tables[0].alignments,
            vec![
                MarkdownTableAlignment::None,
                MarkdownTableAlignment::Left,
                MarkdownTableAlignment::Center,
                MarkdownTableAlignment::Right,
            ]
        );
    }

    #[test]
    fn html_placeholder_text_builds_directly() {
        let inline = html_inline_text("<kbd>x</kbd>");
        assert_eq!(inline, "html:<kbd>x</kbd>");
        assert!(inline.capacity() >= inline.len());
        let block = html_block_text("<div>block</div>");
        assert_eq!(block, "html:\n<div>block</div>");
        assert!(block.capacity() >= block.len());
    }

    #[test]
    fn math_placeholder_text_builds_directly() {
        let inline = prefixed_text("math:", "x + y");
        assert_eq!(inline, "math:x + y");
        assert!(inline.capacity() >= inline.len());
        let block = math_block_text("a^2 + b^2");
        assert_eq!(block, "math:\na^2 + b^2");
        assert!(block.capacity() >= block.len());
    }

    #[test]
    fn image_placeholder_text_builds_directly() {
        let placeholder = image_placeholder_text("kittui logo", "assets/logo.png");
        assert_eq!(placeholder, "image: kittui logo -> assets/logo.png");
        assert!(placeholder.capacity() >= placeholder.len());
    }

    #[test]
    fn code_block_text_builds_directly() {
        let text = code_block_text("rust", "fn main() {}");
        assert_eq!(text, "code:rust\nfn main() {}");
        assert!(text.capacity() >= text.len());
    }

    #[test]
    fn footnote_definition_text_builds_directly() {
        let text = footnote_definition_text("note", "details here");
        assert_eq!(text, "footnote [^note]: details here");
        assert!(text.capacity() >= text.len());
    }

    #[test]
    fn definition_block_text_builds_directly() {
        let text = definition_block_text("Term", "Definition text");
        assert_eq!(text, "definition: Term\n: Definition text");
        assert!(text.capacity() >= text.len());
    }

    #[test]
    fn metadata_block_text_builds_directly() {
        let text = metadata_block_text("yaml", "title: Proof");
        assert_eq!(text, "metadata:yaml\ntitle: Proof");
        assert!(text.capacity() >= text.len());
    }

    #[test]
    fn footnote_marker_builds_directly() {
        let marker = footnote_marker("note");
        assert_eq!(marker, "[^note]");
        assert!(marker.capacity() >= marker.len());
    }

    #[test]
    fn list_marker_helpers_build_directly() {
        let marker = ordered_list_marker(42);
        assert_eq!(marker, "42.");
        assert!(marker.capacity() >= marker.len());
        let item = list_item_text(&marker, "answer");
        assert_eq!(item, "42. answer");
        assert!(item.capacity() >= item.len());
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
        assert_eq!(
            doc.code_blocks,
            vec![
                MarkdownCodeBlock {
                    language: Some("rust".to_string()),
                    text: "fn main() {}".to_string(),
                },
                MarkdownCodeBlock {
                    language: None,
                    text: "plain".to_string(),
                },
            ]
        );
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
        assert_eq!(doc.images[0].title, None);
        assert!(doc.components.iter().any(|c| c
            .text
            .contains("Logo: image: kittui logo -> assets/logo.png")));
    }

    #[test]
    fn markdown_preserves_image_title_metadata() {
        let doc = render_markdown("![kittui logo](assets/logo.png \"Logo title\")", 60);
        assert_eq!(doc.images.len(), 1);
        assert_eq!(doc.images[0].alt, "kittui logo");
        assert_eq!(doc.images[0].url, "assets/logo.png");
        assert_eq!(doc.images[0].title.as_deref(), Some("Logo title"));
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
        assert_eq!(
            doc.html,
            vec![
                MarkdownHtml {
                    kind: MarkdownHtmlKind::Inline,
                    source: "<kbd>".to_string(),
                },
                MarkdownHtml {
                    kind: MarkdownHtmlKind::Inline,
                    source: "</kbd>".to_string(),
                },
                MarkdownHtml {
                    kind: MarkdownHtmlKind::Block,
                    source: "<div>block</div>".to_string(),
                },
            ]
        );
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
        assert_eq!(doc.footnote_references, vec!["note".to_string()]);
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

    #[test]
    fn markdown_renders_definition_lists() {
        let doc = render_markdown("Term\n: Definition text", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(
            text.contains("definition: Term\n: Definition text"),
            "{text}"
        );
        assert_eq!(
            doc.definitions[0],
            MarkdownDefinition {
                term: "Term".to_string(),
                definition: "Definition text".to_string(),
            }
        );
    }

    #[test]
    fn markdown_preserves_inline_and_display_math_placeholders() {
        let doc = render_markdown("Inline $x + y$\n\n$$\na^2 + b^2\n$$", 60);
        let text = doc
            .components
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(text.contains("Inline math:x + y"), "{text}");
        assert!(text.contains("math:\na^2 + b^2"), "{text}");
        assert_eq!(
            doc.math,
            vec![
                MarkdownMath {
                    kind: MarkdownMathKind::Inline,
                    source: "x + y".to_string(),
                },
                MarkdownMath {
                    kind: MarkdownMathKind::Display,
                    source: "a^2 + b^2".to_string(),
                },
            ]
        );
    }

    #[test]
    fn markdown_preserves_metadata_blocks() {
        let doc = render_markdown("---\ntitle: Proof\n---\n\n# Body", 60);
        assert_eq!(
            doc.metadata_blocks,
            vec![MarkdownMetadataBlock {
                kind: MarkdownMetadataBlockKind::Yaml,
                source: "title: Proof".to_string(),
            }]
        );
        assert!(doc
            .components
            .iter()
            .any(|c| c.text.contains("metadata:yaml\ntitle: Proof")));
    }

    #[test]
    fn markdown_preserves_pluses_metadata_blocks() {
        let doc = render_markdown("+++\ntitle = \"Proof\"\n+++\n\n# Body", 60);
        assert_eq!(
            doc.metadata_blocks,
            vec![MarkdownMetadataBlock {
                kind: MarkdownMetadataBlockKind::Pluses,
                source: "title = \"Proof\"".to_string(),
            }]
        );
        assert!(doc
            .components
            .iter()
            .any(|c| c.text.contains("metadata:pluses\ntitle = \"Proof\"")));
    }
}
