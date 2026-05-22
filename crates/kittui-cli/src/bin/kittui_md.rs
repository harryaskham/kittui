//! `kittui-md` — standalone rich kittui Markdown viewer.

use std::io::{Read, Write};
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use kittui::scene::{background_linear, rounded_rect, scene};
use kittui::{CellRect, CellSize, Direction, RendererKind, Rgba, Runtime, Scene, Transport};
use kittui_affordances::{
    box_glyph_scene, render_markdown, ComponentKind, MarkdownDocument, MarkdownTable,
    MarkdownTableAlignment, TableGlyphLayout, UiComponent,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Mode {
    Rich,
    Plain,
    Components,
    Outline,
    References,
    Links,
    Footnotes,
    Images,
    Tables,
    CodeBlocks,
    Definitions,
    Math,
    Html,
    Stats,
    MetadataJson,
}

#[derive(Clone, Debug)]
struct Config {
    mode: Mode,
    width: u16,
    offset_rows: u16,
    height_rows: Option<u16>,
    interactive: bool,
    path: Option<String>,
}

#[derive(Clone, Debug)]
struct LaidOutComponent<'a> {
    component: &'a UiComponent,
    rect: CellRect,
    table_index: Option<usize>,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kittui-md: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let cfg = parse_args(std::env::args().skip(1))?;
    let markdown = if let Some(path) = &cfg.path {
        std::fs::read_to_string(path)?
    } else {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    };
    let doc = render_markdown(&markdown, cfg.width);
    match cfg.mode {
        Mode::Plain => write_plain(&doc, cfg.width, &mut std::io::stdout().lock()),
        Mode::Components => write_components(&doc, &mut std::io::stdout().lock()),
        Mode::Outline => write_outline(&doc, &mut std::io::stdout().lock()),
        Mode::References => write_references(&doc, &mut std::io::stdout().lock()),
        Mode::Links => write_links(&doc, &mut std::io::stdout().lock()),
        Mode::Footnotes => write_footnotes(&doc, &mut std::io::stdout().lock()),
        Mode::Images => write_images(&doc, &mut std::io::stdout().lock()),
        Mode::Tables => write_tables(&doc, &mut std::io::stdout().lock()),
        Mode::CodeBlocks => write_code_blocks(&doc, &mut std::io::stdout().lock()),
        Mode::Definitions => write_definitions(&doc, &mut std::io::stdout().lock()),
        Mode::Math => write_math(&doc, &mut std::io::stdout().lock()),
        Mode::Html => write_html(&doc, &mut std::io::stdout().lock()),
        Mode::Stats => write_stats(&doc, &markdown, &mut std::io::stdout().lock()),
        Mode::MetadataJson => write_metadata_json(
            &doc,
            &markdown,
            cfg.width,
            cfg.path.as_deref(),
            &mut std::io::stdout().lock(),
        ),
        Mode::Rich if cfg.interactive => run_interactive(&doc, cfg),
        Mode::Rich => write_rich(&doc, &cfg, &mut std::io::stdout().lock()),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut mode = Mode::Rich;
    let mut mode_flag: Option<&'static str> = None;
    let mut width = terminal_cols().unwrap_or(80).clamp(20, 120);
    let mut offset_rows = 0;
    let mut height_rows = None;
    let mut interactive = false;
    let mut path = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--plain" => set_mode(&mut mode, &mut mode_flag, "--plain", Mode::Plain)?,
            "--rich" => set_mode(&mut mode, &mut mode_flag, "--rich", Mode::Rich)?,
            "--components" => {
                set_mode(&mut mode, &mut mode_flag, "--components", Mode::Components)?
            }
            "--outline" => set_mode(&mut mode, &mut mode_flag, "--outline", Mode::Outline)?,
            "--references" => {
                set_mode(&mut mode, &mut mode_flag, "--references", Mode::References)?
            }
            "--links" => set_mode(&mut mode, &mut mode_flag, "--links", Mode::Links)?,
            "--footnotes" => set_mode(&mut mode, &mut mode_flag, "--footnotes", Mode::Footnotes)?,
            "--images" => set_mode(&mut mode, &mut mode_flag, "--images", Mode::Images)?,
            "--tables" => set_mode(&mut mode, &mut mode_flag, "--tables", Mode::Tables)?,
            "--code-blocks" => {
                set_mode(&mut mode, &mut mode_flag, "--code-blocks", Mode::CodeBlocks)?
            }
            "--definitions" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--definitions",
                Mode::Definitions,
            )?,
            "--math" => set_mode(&mut mode, &mut mode_flag, "--math", Mode::Math)?,
            "--html" => set_mode(&mut mode, &mut mode_flag, "--html", Mode::Html)?,
            "--stats" => set_mode(&mut mode, &mut mode_flag, "--stats", Mode::Stats)?,
            "--metadata-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-json",
                Mode::MetadataJson,
            )?,
            "--width" => {
                width = args
                    .next()
                    .ok_or_else(|| anyhow!("--width requires a value"))?
                    .parse()?
            }
            "--offset" => {
                offset_rows = args
                    .next()
                    .ok_or_else(|| anyhow!("--offset requires a value"))?
                    .parse()?
            }
            "--interactive" | "-i" => interactive = true,
            "--height" => {
                height_rows = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--height requires a value"))?
                        .parse()?,
                )
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => return Err(anyhow!("unknown flag {arg}")),
            _ => {
                if path.replace(arg).is_some() {
                    return Err(anyhow!("expected at most one input path"));
                }
            }
        }
    }
    Ok(Config {
        mode,
        width: width.clamp(20, 200),
        offset_rows,
        height_rows,
        interactive,
        path,
    })
}

fn set_mode(
    mode: &mut Mode,
    seen: &mut Option<&'static str>,
    flag: &'static str,
    next: Mode,
) -> Result<()> {
    if let Some(prev) = *seen {
        return Err(anyhow!(
            "output modes are mutually exclusive: {prev} and {flag}"
        ));
    }
    *mode = next;
    *seen = Some(flag);
    Ok(())
}

fn print_help() {
    println!("kittui-md [--rich|--plain|--components|--outline|--references|--links|--footnotes|--images|--tables|--code-blocks|--definitions|--math|--html|--stats|--metadata-json] [--interactive] [--width N] [--offset ROWS] [--height ROWS] [file]");
    println!(
        "Render Markdown as kittui/kitty graphics components. Reads stdin when file is omitted."
    );
}

fn run_interactive(doc: &MarkdownDocument, mut cfg: Config) -> Result<()> {
    if cfg.path.is_none() {
        return Err(anyhow!(
            "--interactive requires an input file so stdin can be used for keys"
        ));
    }
    let _raw = RawTerminal::enter()?;
    let mut stdout = std::io::stdout().lock();
    let mut stdin = std::io::stdin().lock();
    let viewport = cfg
        .height_rows
        .unwrap_or_else(|| terminal_rows().unwrap_or(24).saturating_sub(2).max(1));
    cfg.height_rows = Some(viewport);
    let total_rows = document_rows(doc, cfg.width);
    loop {
        write!(stdout, "\x1b[2J\x1b[H")?;
        write_rich(doc, &cfg, &mut stdout)?;
        writeln!(
            stdout,
            "j/k scroll • space/page down • b/page up • g/G ends • q quit"
        )?;
        stdout.flush()?;
        let action = read_pager_action(&mut stdin)?;
        if action == PagerAction::Quit {
            break;
        }
        cfg.offset_rows = apply_pager_action(cfg.offset_rows, viewport, total_rows, action);
    }
    write!(stdout, "\x1b[0m\x1b[?25h\x1b[2J\x1b[H")?;
    stdout.flush()?;
    Ok(())
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PagerAction {
    Noop,
    Quit,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

fn read_pager_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(match buf[0] {
        b'q' | 3 => PagerAction::Quit,
        b'k' | b'w' => PagerAction::Up,
        b'j' | b's' | b'\n' | b'\r' => PagerAction::Down,
        b' ' => PagerAction::PageDown,
        b'b' => PagerAction::PageUp,
        b'g' => PagerAction::Home,
        b'G' => PagerAction::End,
        27 => read_escape_action(input)?,
        _ => PagerAction::Noop,
    })
}

fn read_escape_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    if input.read(&mut buf)? == 0 {
        return Ok(PagerAction::Quit);
    }
    match buf[0] {
        b'[' => read_csi_action(input),
        b'O' => read_ss3_action(input),
        _ => Ok(PagerAction::Noop),
    }
}

fn read_csi_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(match buf[0] {
        b'A' => PagerAction::Up,
        b'B' => PagerAction::Down,
        b'H' => PagerAction::Home,
        b'F' => PagerAction::End,
        b'1' | b'7' => {
            consume_optional_tilde(input)?;
            PagerAction::Home
        }
        b'4' | b'8' => {
            consume_optional_tilde(input)?;
            PagerAction::End
        }
        b'5' => {
            consume_optional_tilde(input)?;
            PagerAction::PageUp
        }
        b'6' => {
            consume_optional_tilde(input)?;
            PagerAction::PageDown
        }
        _ => PagerAction::Noop,
    })
}

fn read_ss3_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(match buf[0] {
        b'H' => PagerAction::Home,
        b'F' => PagerAction::End,
        _ => PagerAction::Noop,
    })
}

fn consume_optional_tilde(input: &mut impl Read) -> Result<()> {
    let mut buf = [0u8; 1];
    if input.read(&mut buf)? == 0 || buf[0] == b'~' {
        return Ok(());
    }
    Ok(())
}

fn apply_pager_action(
    offset: u16,
    viewport_rows: u16,
    total_rows: u16,
    action: PagerAction,
) -> u16 {
    let max_offset = total_rows.saturating_sub(viewport_rows);
    match action {
        PagerAction::Noop => offset.min(max_offset),
        PagerAction::Quit => offset,
        PagerAction::Up => offset.saturating_sub(1),
        PagerAction::Down => offset.saturating_add(1).min(max_offset),
        PagerAction::PageUp => offset.saturating_sub(viewport_rows.max(1)),
        PagerAction::PageDown => offset.saturating_add(viewport_rows.max(1)).min(max_offset),
        PagerAction::Home => 0,
        PagerAction::End => max_offset,
    }
}

fn document_rows(doc: &MarkdownDocument, width: u16) -> u16 {
    layout_components(&doc.components, &doc.tables, width)
        .last()
        .map(|item| item.rect.y.saturating_add(item.rect.rows))
        .unwrap_or(0)
}

struct RawTerminal {
    original: libc::termios,
}

impl RawTerminal {
    fn enter() -> Result<Self> {
        let fd = libc::STDIN_FILENO;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        let rc = unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) };
        if rc != 0 {
            return Err(anyhow!(
                "tcgetattr failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        let rc = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
        if rc != 0 {
            return Err(anyhow!(
                "tcsetattr raw failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self { original })
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original) };
    }
}

fn write_components(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md components — {} components",
        doc.components.len()
    )?;
    if doc.components.is_empty() {
        writeln!(out, "<empty>")?;
    } else {
        for component in &doc.components {
            write_plain_component(out, component)?;
        }
    }
    Ok(())
}

fn write_outline(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md outline — {} headings", doc.outline.len())?;
    if doc.outline.is_empty() {
        writeln!(out, "<empty>")?;
    } else {
        for line in outline_lines(doc) {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

fn write_stats(doc: &MarkdownDocument, source: &str, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md stats")?;
    writeln!(out, "source.bytes={}", source.len())?;
    writeln!(out, "source.lines={}", source.lines().count())?;
    writeln!(out, "components={}", doc.components.len())?;
    writeln!(out, "headings={}", doc.outline.len())?;
    writeln!(out, "links={}", doc.links.len())?;
    writeln!(out, "images={}", doc.images.len())?;
    writeln!(out, "tables={}", doc.tables.len())?;
    writeln!(out, "footnote_references={}", doc.footnote_references.len())?;
    writeln!(out, "footnotes={}", doc.footnotes.len())?;
    writeln!(out, "definitions={}", doc.definitions.len())?;
    writeln!(out, "math={}", doc.math.len())?;
    writeln!(out, "html={}", doc.html.len())?;
    writeln!(out, "code_blocks={}", doc.code_blocks.len())?;
    Ok(())
}

fn write_links(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md links — {} links", doc.links.len())?;
    if doc.links.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, link) in doc.links.iter().enumerate() {
        writeln!(out, "link #{}", i + 1)?;
        writeln!(out, "  label={}", link.label)?;
        writeln!(out, "  url={}", link.url)?;
    }
    Ok(())
}

fn write_footnotes(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let total = doc.footnote_references.len() + doc.footnotes.len();
    writeln!(out, "kittui-md footnotes — {total} entries")?;
    if total == 0 {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    if !doc.footnote_references.is_empty() {
        writeln!(out, "references:")?;
        for label in &doc.footnote_references {
            writeln!(out, "  [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        writeln!(out, "definitions:")?;
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    Ok(())
}

fn write_images(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md images — {} images", doc.images.len())?;
    if doc.images.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, image) in doc.images.iter().enumerate() {
        writeln!(out, "image #{}", i + 1)?;
        writeln!(out, "  alt={}", image.alt)?;
        writeln!(out, "  url={}", image.url)?;
    }
    Ok(())
}

fn write_tables(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md tables — {} tables", doc.tables.len())?;
    if doc.tables.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, table) in doc.tables.iter().enumerate() {
        let footprint = table.footprint();
        writeln!(out, "table #{}", i + 1)?;
        writeln!(out, "  rows={}", table.rows.len())?;
        writeln!(out, "  columns={}", table.column_widths().len())?;
        writeln!(out, "  column_widths={:?}", table.column_widths())?;
        writeln!(
            out,
            "  alignments={:?}",
            table
                .alignments
                .iter()
                .map(|alignment| alignment.as_str())
                .collect::<Vec<_>>()
        )?;
        writeln!(out, "  footprint={}x{}", footprint.cols, footprint.rows)?;
        for row in &table.rows {
            writeln!(out, "  | {} |", row.join(" | "))?;
        }
    }
    Ok(())
}

fn write_html(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md html — {} fragments", doc.html.len())?;
    if doc.html.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, html) in doc.html.iter().enumerate() {
        writeln!(out, "html #{}", i + 1)?;
        writeln!(out, "  kind={}", html.kind.as_str())?;
        writeln!(out, "  source={}", html.source)?;
    }
    Ok(())
}

fn write_math(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md math — {} expressions", doc.math.len())?;
    if doc.math.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, math) in doc.math.iter().enumerate() {
        writeln!(out, "math #{}", i + 1)?;
        writeln!(out, "  kind={}", math.kind.as_str())?;
        writeln!(out, "  source={}", math.source)?;
    }
    Ok(())
}

fn write_definitions(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md definitions — {} definitions",
        doc.definitions.len()
    )?;
    if doc.definitions.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, definition) in doc.definitions.iter().enumerate() {
        writeln!(out, "definition #{}", i + 1)?;
        writeln!(out, "  term={}", definition.term)?;
        writeln!(out, "  definition={}", definition.definition)?;
    }
    Ok(())
}

fn write_code_blocks(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md code blocks — {} code blocks",
        doc.code_blocks.len()
    )?;
    if doc.code_blocks.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, block) in doc.code_blocks.iter().enumerate() {
        writeln!(out, "code block #{}", i + 1)?;
        writeln!(
            out,
            "  language={}",
            block.language.as_deref().unwrap_or("<plain>")
        )?;
        writeln!(out, "---")?;
        writeln!(out, "{}", block.text)?;
        writeln!(out, "---")?;
    }
    Ok(())
}

fn write_references(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let total =
        doc.links.len() + doc.images.len() + doc.footnote_references.len() + doc.footnotes.len();
    writeln!(out, "kittui-md references — {total} entries")?;
    if total == 0 {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    if !doc.links.is_empty() {
        writeln!(out, "links:")?;
        for link in &doc.links {
            writeln!(out, "  [{}] {}", link.label, link.url)?;
        }
    }
    if !doc.images.is_empty() {
        writeln!(out, "images:")?;
        for image in &doc.images {
            writeln!(out, "  [{}] {}", image.alt, image.url)?;
        }
    }
    if !doc.footnote_references.is_empty() {
        writeln!(out, "footnote references:")?;
        for label in &doc.footnote_references {
            writeln!(out, "  [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        writeln!(out, "footnotes:")?;
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    Ok(())
}

fn write_metadata_json(
    doc: &MarkdownDocument,
    source: &str,
    width_cells: u16,
    source_path: Option<&str>,
    out: &mut impl Write,
) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "source": {
            "bytes": source.len(),
            "lines": source.lines().count(),
            "path": source_path,
        },
        "render": {
            "mode": "metadata-json",
            "width_cells": width_cells,
        },
        "components": doc.components.len(),
        "components_detail": doc.components.iter().map(|component| serde_json::json!({
            "kind": format!("{:?}", component.kind),
            "text": component.text,
            "width_cells": component.width_cells,
            "height_cells": component.height_cells,
        })).collect::<Vec<_>>(),
        "links": doc.links.iter().map(|link| serde_json::json!({
            "label": link.label,
            "url": link.url,
        })).collect::<Vec<_>>(),
        "images": doc.images.iter().map(|image| serde_json::json!({
            "alt": image.alt,
            "url": image.url,
        })).collect::<Vec<_>>(),
        "footnote_references": doc.footnote_references,
        "footnotes": doc.footnotes.iter().map(|footnote| serde_json::json!({
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
        "definitions": doc.definitions.iter().map(|definition| serde_json::json!({
            "term": definition.term,
            "definition": definition.definition,
        })).collect::<Vec<_>>(),
        "math": doc.math.iter().map(|math| serde_json::json!({
            "kind": math.kind.as_str(),
            "source": math.source,
        })).collect::<Vec<_>>(),
        "html": doc.html.iter().map(|html| serde_json::json!({
            "kind": html.kind.as_str(),
            "source": html.source,
        })).collect::<Vec<_>>(),
        "code_blocks": doc.code_blocks.iter().map(|code| serde_json::json!({
            "language": code.language,
            "text": code.text,
        })).collect::<Vec<_>>(),
        "outline": doc.outline.iter().map(|heading| serde_json::json!({
            "level": heading.level,
            "text": heading.text,
        })).collect::<Vec<_>>(),
        "tables": doc.tables.iter().map(|table| {
            let footprint = table.footprint();
            serde_json::json!({
                "rows": table.rows,
                "alignments": table.alignments.iter().map(|alignment| alignment.as_str()).collect::<Vec<_>>(),
                "column_widths": table.column_widths(),
                "footprint": {
                    "cols": footprint.cols,
                    "rows": footprint.rows,
                },
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_plain(doc: &MarkdownDocument, width: u16, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md — {} components, {} links, {} images",
        doc.components.len(),
        doc.links.len(),
        doc.images.len()
    )?;
    writeln!(out, "{}", "═".repeat(width as usize))?;
    for comp in &doc.components {
        write_plain_component(out, comp)?;
    }
    write_metadata_sections(doc, out)?;
    Ok(())
}

fn write_plain_component(out: &mut impl Write, comp: &UiComponent) -> Result<()> {
    let prefix = format!("[{:?}] ", comp.kind);
    let continuation = " ".repeat(prefix.len());
    let mut lines = comp.text.lines();
    if let Some(first) = lines.next() {
        writeln!(out, "{prefix}{first}")?;
        for line in lines {
            writeln!(out, "{continuation}{line}")?;
        }
    } else {
        writeln!(out, "{prefix}")?;
    }
    Ok(())
}

fn write_metadata_sections(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    if !doc.outline.is_empty() {
        writeln!(out, "\noutline:")?;
        for line in outline_lines(doc) {
            writeln!(out, "  {line}")?;
        }
    }
    if !doc.links.is_empty() {
        writeln!(out, "\nlinks:")?;
        for link in &doc.links {
            writeln!(out, "  [{}] {}", link.label, link.url)?;
        }
    }
    if !doc.images.is_empty() {
        writeln!(out, "\nimages:")?;
        for image in &doc.images {
            writeln!(out, "  [{}] {}", image.alt, image.url)?;
        }
    }
    if !doc.footnote_references.is_empty() {
        writeln!(out, "\nfootnote references:")?;
        for label in &doc.footnote_references {
            writeln!(out, "  [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        writeln!(out, "\nfootnotes:")?;
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    if !doc.definitions.is_empty() {
        writeln!(out, "\ndefinitions:")?;
        for definition in &doc.definitions {
            writeln!(out, "  {} — {}", definition.term, definition.definition)?;
        }
    }
    if !doc.math.is_empty() {
        writeln!(out, "\nmath:")?;
        for math in &doc.math {
            writeln!(out, "  {} {}", math.kind.as_str(), math.source)?;
        }
    }
    if !doc.html.is_empty() {
        writeln!(out, "\nhtml:")?;
        for html in &doc.html {
            writeln!(out, "  {} {}", html.kind.as_str(), html.source)?;
        }
    }
    if !doc.code_blocks.is_empty() {
        writeln!(out, "\ncode blocks:")?;
        for code in &doc.code_blocks {
            writeln!(
                out,
                "  {} {}",
                code.language.as_deref().unwrap_or("<plain>"),
                code.text.lines().next().unwrap_or("")
            )?;
        }
    }
    Ok(())
}

fn write_rich(doc: &MarkdownDocument, cfg: &Config, out: &mut impl Write) -> Result<()> {
    let layout = layout_components(&doc.components, &doc.tables, cfg.width);
    let visible = visible_components(&layout, cfg.offset_rows, cfg.height_rows);
    let cell = CellSize::default();
    let runtime = Runtime::builder().renderer(RendererKind::Cpu).build()?;

    write!(out, "\x1b[?25l")?;
    for item in &visible {
        let local_rect = CellRect::new(
            0,
            item.rect.y.saturating_sub(cfg.offset_rows),
            item.rect.cols,
            item.rect.rows,
        );
        let scene = component_scene(item.component, local_rect, cell);
        let placed = runtime.place(&scene)?;
        write!(
            out,
            "{}{}{}",
            placed.upload,
            placed.placement,
            kittui_kitty::cursor_move(local_rect.x, local_rect.y, Transport::Direct)
        )?;
        write!(out, "{}", placed.embed)?;
        if let Some(table_index) = item.table_index {
            if let Some(table) = doc.tables.get(table_index) {
                write_table_glyphs(out, &runtime, table, placed.image_id, local_rect, cell)?;
                write_table_text(out, table, local_rect)?;
                continue;
            }
        }
        write_component_text(out, item.component, local_rect)?;
    }
    let footer_y = visible
        .last()
        .map(|item| {
            item.rect
                .y
                .saturating_sub(cfg.offset_rows)
                .saturating_add(item.rect.rows)
                .saturating_add(1)
        })
        .unwrap_or(0);
    write!(
        out,
        "{}",
        kittui_kitty::cursor_move(0, footer_y, Transport::Direct)
    )?;
    writeln!(
        out,
        "\x1b[0m\x1b[?25h{}",
        rich_status_line(doc, cfg, document_rows(doc, cfg.width))
    )?;
    if !doc.outline.is_empty() {
        writeln!(out, "  outline:")?;
        for line in outline_lines(doc) {
            writeln!(out, "    {line}")?;
        }
    }
    if !doc.links.is_empty() {
        for link in &doc.links {
            writeln!(out, "  🔗 {} — {}", link.label, link.url)?;
        }
    }
    if !doc.images.is_empty() {
        for image in &doc.images {
            writeln!(out, "  🖼  {} — {}", image.alt, image.url)?;
        }
    }
    if !doc.footnote_references.is_empty() {
        for label in &doc.footnote_references {
            writeln!(out, "  ↩ [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    if !doc.definitions.is_empty() {
        for definition in &doc.definitions {
            writeln!(out, "  📖 {} — {}", definition.term, definition.definition)?;
        }
    }
    if !doc.math.is_empty() {
        for math in &doc.math {
            writeln!(out, "  ∑ {} — {}", math.kind.as_str(), math.source)?;
        }
    }
    if !doc.html.is_empty() {
        for html in &doc.html {
            writeln!(out, "  HTML {} — {}", html.kind.as_str(), html.source)?;
        }
    }
    if !doc.code_blocks.is_empty() {
        for code in &doc.code_blocks {
            writeln!(
                out,
                "  code {} — {}",
                code.language.as_deref().unwrap_or("<plain>"),
                code.text.lines().next().unwrap_or("")
            )?;
        }
    }
    Ok(())
}

fn outline_lines(doc: &MarkdownDocument) -> Vec<String> {
    doc.outline
        .iter()
        .map(|heading| {
            format!(
                "{}{}",
                "  ".repeat(heading.level.saturating_sub(1) as usize),
                heading.text
            )
        })
        .collect()
}

fn rich_status_line(doc: &MarkdownDocument, cfg: &Config, total_rows: u16) -> String {
    let viewport = cfg.height_rows.unwrap_or(total_rows);
    let max_offset = total_rows.saturating_sub(viewport);
    format!(
        "kittui-md rich view — {} components, {} headings, {} links, {} images, {} footnote refs, {} footnotes, {} definitions, {} math, {} html, {} code blocks; offset={}/{} rows; viewport={}; total_rows={}",
        doc.components.len(),
        doc.outline.len(),
        doc.links.len(),
        doc.images.len(),
        doc.footnote_references.len(),
        doc.footnotes.len(),
        doc.definitions.len(),
        doc.math.len(),
        doc.html.len(),
        doc.code_blocks.len(),
        cfg.offset_rows.min(max_offset),
        max_offset,
        viewport,
        total_rows,
    )
}

fn layout_components<'a>(
    components: &'a [UiComponent],
    tables: &'a [MarkdownTable],
    width: u16,
) -> Vec<LaidOutComponent<'a>> {
    let mut y = 0;
    let mut table_index = 0usize;
    let mut out = Vec::with_capacity(components.len());
    for component in components {
        let is_table =
            component.kind == ComponentKind::TextBox && component.text.starts_with("table\n");
        let current_table = if is_table {
            let idx = table_index;
            table_index += 1;
            Some(idx)
        } else {
            None
        };
        let table_rows = current_table
            .and_then(|idx| tables.get(idx))
            .map(|table| table.footprint().rows.saturating_add(2));
        let rows = table_rows.unwrap_or_else(|| component.height_cells.max(1));
        let cols = component.width_cells.min(width).max(1);
        out.push(LaidOutComponent {
            component,
            rect: CellRect::new(0, y, cols, rows),
            table_index: current_table,
        });
        y = y.saturating_add(rows).saturating_add(1);
    }
    out
}

fn visible_components<'a>(
    layout: &'a [LaidOutComponent<'a>],
    offset_rows: u16,
    height_rows: Option<u16>,
) -> Vec<&'a LaidOutComponent<'a>> {
    let end = height_rows.map(|h| offset_rows.saturating_add(h));
    layout
        .iter()
        .filter(|item| {
            let bottom = item.rect.y.saturating_add(item.rect.rows);
            bottom > offset_rows && end.map_or(true, |end| item.rect.y < end)
        })
        .collect()
}

fn component_scene(component: &UiComponent, rect: CellRect, cell: CellSize) -> Scene {
    let mut layers = Vec::new();
    match component.kind {
        ComponentKind::H1 | ComponentKind::Title => layers.push(background_linear(
            rect,
            cell,
            Direction::Horizontal,
            Rgba::rgba(102, 92, 255, 235),
            Rgba::rgba(18, 214, 196, 220),
        )),
        ComponentKind::H2 | ComponentKind::Header => layers.push(background_linear(
            rect,
            cell,
            Direction::Horizontal,
            Rgba::rgba(60, 130, 255, 220),
            Rgba::rgba(27, 32, 54, 210),
        )),
        ComponentKind::H3 | ComponentKind::Footer => layers.push(background_linear(
            rect,
            cell,
            Direction::Horizontal,
            Rgba::rgba(145, 105, 255, 210),
            Rgba::rgba(27, 32, 54, 205),
        )),
        ComponentKind::TextChip => layers.push(rounded_rect(
            rect.to_pixels(cell),
            Rgba::rgba(65, 76, 125, 230),
            Rgba::rgba(176, 196, 255, 240),
            1.0,
            8.0,
        )),
        ComponentKind::Banner => layers.push(rounded_rect(
            rect.to_pixels(cell),
            Rgba::rgba(80, 62, 35, 230),
            Rgba::rgba(255, 195, 92, 245),
            1.0,
            6.0,
        )),
        ComponentKind::TextBox => layers.push(rounded_rect(
            rect.to_pixels(cell),
            Rgba::rgba(20, 24, 38, 220),
            Rgba::rgba(95, 116, 170, 230),
            1.0,
            6.0,
        )),
    }
    scene(rect, cell, layers)
}

fn write_table_glyphs(
    out: &mut impl Write,
    runtime: &Runtime,
    table: &MarkdownTable,
    anchor_image_id: u32,
    rect: CellRect,
    cell: CellSize,
) -> Result<()> {
    let layout =
        TableGlyphLayout::from_table(anchor_image_id, table).with_background(anchor_image_id);
    let fg = Rgba::rgba(176, 220, 255, 245);
    for (i, glyph_cell) in layout.cells.iter().enumerate() {
        let scene = box_glyph_scene(glyph_cell.glyph, fg, cell);
        let placed = runtime.place(&scene)?;
        let mut options = glyph_cell.placement.clone();
        options.placement_id = Some(10_000 + i as u32);
        if let Some(relative) = &mut options.relative {
            relative.image_id = anchor_image_id;
        }
        let command = kittui_kitty::placement_command_ex(
            placed.image_id,
            CellRect::new(rect.x, rect.y, 1, 1),
            &options,
            Transport::Direct,
        );
        write!(out, "{}{}", placed.upload, command)?;
    }
    Ok(())
}

fn write_table_text(out: &mut impl Write, table: &MarkdownTable, rect: CellRect) -> Result<()> {
    let widths = table.column_widths();
    for (row_idx, row) in table.rows.iter().enumerate() {
        let y = rect.y.saturating_add(1 + row_idx as u16 * 2);
        let mut x = rect.x.saturating_add(2);
        for (col_idx, cell) in row.iter().enumerate() {
            write!(
                out,
                "{}\x1b[37m{}\x1b[0m",
                kittui_kitty::cursor_move(x, y, Transport::Direct),
                align_table_cell_text(
                    cell,
                    widths.get(col_idx).copied().unwrap_or(1) as usize,
                    table
                        .alignments
                        .get(col_idx)
                        .copied()
                        .unwrap_or(MarkdownTableAlignment::None),
                )
            )?;
            x = x
                .saturating_add(widths.get(col_idx).copied().unwrap_or(1))
                .saturating_add(3);
        }
    }
    Ok(())
}

fn align_table_cell_text(text: &str, width: usize, alignment: MarkdownTableAlignment) -> String {
    let truncated = truncate_cells(text, width);
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

fn write_component_text(
    out: &mut impl Write,
    component: &UiComponent,
    rect: CellRect,
) -> Result<()> {
    let x = if matches!(component.kind, ComponentKind::TextChip) {
        1
    } else {
        2
    };
    let style = match component.kind {
        ComponentKind::H1 | ComponentKind::Title => "\x1b[1;97m",
        ComponentKind::H2 | ComponentKind::Header => "\x1b[1;96m",
        ComponentKind::H3 | ComponentKind::Footer => "\x1b[1;95m",
        ComponentKind::TextChip => "\x1b[1;94m",
        ComponentKind::Banner => "\x1b[1;93m",
        ComponentKind::TextBox => "\x1b[37m",
    };
    let max_cols = rect.cols.saturating_sub(x + 1) as usize;
    let max_rows = if matches!(component.kind, ComponentKind::TextChip) {
        1
    } else {
        rect.rows.saturating_sub(1).max(1) as usize
    };
    let start_y = if max_rows == 1 {
        rect.y.saturating_add(rect.rows / 2)
    } else {
        rect.y.saturating_add(1)
    };
    for (i, line) in wrap_text_lines(&component.text, max_cols, max_rows)
        .iter()
        .enumerate()
    {
        write!(
            out,
            "{}{style}{}\x1b[0m",
            kittui_kitty::cursor_move(
                rect.x.saturating_add(x),
                start_y.saturating_add(i as u16),
                Transport::Direct
            ),
            line
        )?;
    }
    Ok(())
}

fn wrap_text_lines(text: &str, max_cols: usize, max_rows: usize) -> Vec<String> {
    if max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut current = String::new();
        for word in raw.split_whitespace() {
            let word_len = word.chars().count();
            let current_len = current.chars().count();
            if current_len == 0 {
                current = truncate_cells(word, max_cols);
            } else if current_len + 1 + word_len <= max_cols {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                if lines.len() == max_rows {
                    return lines;
                }
                current = truncate_cells(word, max_cols);
            }
        }
        if !current.is_empty() || raw.is_empty() {
            lines.push(current);
            if lines.len() == max_rows {
                return lines;
            }
        }
    }
    lines
}

fn truncate_cells(s: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in s.chars().take(max) {
        out.push(ch);
    }
    out
}

fn terminal_rows() -> Option<u16> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_row > 0 {
        Some(ws.ws_row)
    } else {
        None
    }
}

fn terminal_cols() -> Option<u16> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 {
        Some(ws.ws_col)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui_affordances::{
        h1, h2, textbox, HeadingOutline, MarkdownDefinition, MarkdownFootnote, MarkdownImage, Tone,
    };

    #[test]
    fn parse_args_rejects_multiple_output_modes() {
        let err = parse_args(["--plain".to_string(), "--outline".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--plain"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
    }

    #[test]
    fn parse_args_accepts_single_output_mode() {
        let cfg = parse_args(["--components".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Components);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn layout_stacks_components_with_gaps() {
        let comps = vec![h1("Title", 40), h1("Next", 40)];
        let layout = layout_components(&comps, &[], 80);
        assert_eq!(layout[0].rect.y, 0);
        assert_eq!(layout[1].rect.y, 4);
    }

    #[test]
    fn layout_marks_table_components() {
        let tables = vec![MarkdownTable::new(vec![vec!["A".into(), "B".into()]])];
        let comps = vec![textbox("table\nA | B", 40, Tone::Assistant)];
        let layout = layout_components(&comps, &tables, 80);
        assert_eq!(layout[0].table_index, Some(0));
        assert!(layout[0].rect.rows > 2);
    }

    #[test]
    fn pager_actions_clamp_to_document() {
        assert_eq!(apply_pager_action(0, 10, 30, PagerAction::Down), 1);
        assert_eq!(apply_pager_action(3, 10, 30, PagerAction::PageUp), 0);
        assert_eq!(apply_pager_action(0, 10, 30, PagerAction::PageDown), 10);
        assert_eq!(apply_pager_action(19, 10, 30, PagerAction::Down), 20);
        assert_eq!(apply_pager_action(20, 10, 30, PagerAction::Down), 20);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Home), 0);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::End), 20);
    }

    #[test]
    fn pager_reads_arrow_and_page_key_escape_sequences() {
        let cases = [
            (b"\x1b[A".as_slice(), PagerAction::Up),
            (b"\x1b[B".as_slice(), PagerAction::Down),
            (b"\x1b[5~".as_slice(), PagerAction::PageUp),
            (b"\x1b[6~".as_slice(), PagerAction::PageDown),
            (b"\x1b[H".as_slice(), PagerAction::Home),
            (b"\x1b[F".as_slice(), PagerAction::End),
            (b"\x1bOH".as_slice(), PagerAction::Home),
            (b"\x1bOF".as_slice(), PagerAction::End),
        ];
        for (bytes, action) in cases {
            let mut cursor = std::io::Cursor::new(bytes);
            assert_eq!(read_pager_action(&mut cursor).unwrap(), action);
        }
    }

    #[test]
    fn document_rows_reports_bottom_edge() {
        let doc = MarkdownDocument {
            components: vec![h1("One", 40), h1("Two", 40)],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        assert_eq!(document_rows(&doc, 80), 7);
    }

    #[test]
    fn rich_status_line_reports_offset_viewport_and_total_rows() {
        let doc = MarkdownDocument {
            components: vec![h1("One", 40), h1("Two", 40)],
            links: vec![],
            tables: vec![],
            images: vec![MarkdownImage {
                alt: "logo".to_string(),
                url: "logo.png".to_string(),
            }],
            outline: vec![HeadingOutline {
                level: 1,
                text: "Title".to_string(),
            }],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        let cfg = Config {
            mode: Mode::Rich,
            width: 80,
            offset_rows: 99,
            height_rows: Some(3),
            interactive: true,
            path: Some("proof.md".to_string()),
        };
        let status = rich_status_line(&doc, &cfg, document_rows(&doc, 80));
        assert!(status.contains("offset=4/4 rows"), "{status}");
        assert!(status.contains("viewport=3"), "{status}");
        assert!(status.contains("total_rows=7"), "{status}");
        assert!(
            status.contains("1 headings, 0 links, 1 images, 0 footnote refs, 0 footnotes, 0 definitions, 0 math, 0 html, 0 code blocks"),
            "{status}"
        );
    }

    #[test]
    fn html_mode_writes_kind_and_source() {
        let doc = render_markdown("hello <kbd>x</kbd>\n\n<div>block</div>", 80);
        let mut out = Vec::new();
        write_html(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md html — 3 fragments"),
            "{rendered}"
        );
        assert!(
            rendered.contains("html #1\n  kind=inline\n  source=<kbd>"),
            "{rendered}"
        );
        assert!(
            rendered.contains("html #2\n  kind=inline\n  source=</kbd>"),
            "{rendered}"
        );
        assert!(
            rendered.contains("html #3\n  kind=block\n  source=<div>block</div>"),
            "{rendered}"
        );
    }

    #[test]
    fn html_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_html(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md html — 0 fragments\n<empty>\n"
        );
    }

    #[test]
    fn math_mode_writes_kind_and_source() {
        let doc = render_markdown("inline $x + y$\n\n$$\na^2\n$$", 80);
        let mut out = Vec::new();
        write_math(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md math — 2 expressions"),
            "{rendered}"
        );
        assert!(
            rendered.contains("math #1\n  kind=inline\n  source=x + y"),
            "{rendered}"
        );
        assert!(
            rendered.contains("math #2\n  kind=display\n  source=a^2"),
            "{rendered}"
        );
    }

    #[test]
    fn math_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_math(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md math — 0 expressions\n<empty>\n"
        );
    }

    #[test]
    fn definitions_mode_writes_terms_and_definitions() {
        let doc = render_markdown("Term\n: Definition text", 80);
        let mut out = Vec::new();
        write_definitions(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md definitions — 1 definitions"),
            "{rendered}"
        );
        assert!(rendered.contains("definition #1"), "{rendered}");
        assert!(rendered.contains("term=Term"), "{rendered}");
        assert!(
            rendered.contains("definition=Definition text"),
            "{rendered}"
        );
    }

    #[test]
    fn definitions_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_definitions(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md definitions — 0 definitions\n<empty>\n"
        );
    }

    #[test]
    fn code_blocks_mode_writes_language_and_source() {
        let doc = render_markdown("```rust\nfn main() {}\n```\n\n```\nplain\n```", 80);
        let mut out = Vec::new();
        write_code_blocks(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md code blocks — 2 code blocks"),
            "{rendered}"
        );
        assert!(
            rendered.contains("code block #1\n  language=rust"),
            "{rendered}"
        );
        assert!(rendered.contains("fn main() {}"), "{rendered}");
        assert!(
            rendered.contains("code block #2\n  language=<plain>"),
            "{rendered}"
        );
        assert!(rendered.contains("plain"), "{rendered}");
    }

    #[test]
    fn code_blocks_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_code_blocks(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md code blocks — 0 code blocks\n<empty>\n"
        );
    }

    #[test]
    fn links_mode_writes_label_and_url() {
        let doc = render_markdown("See [site](https://example.com)", 80);
        let mut out = Vec::new();
        write_links(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md links — 1 links"), "{rendered}");
        assert!(rendered.contains("link #1"), "{rendered}");
        assert!(rendered.contains("label=site"), "{rendered}");
        assert!(rendered.contains("url=https://example.com"), "{rendered}");
    }

    #[test]
    fn links_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_links(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md links — 0 links\n<empty>\n"
        );
    }

    #[test]
    fn footnotes_mode_writes_references_and_definitions() {
        let doc = render_markdown("see[^n]\n\n[^n]: note text", 80);
        let mut out = Vec::new();
        write_footnotes(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md footnotes — 2 entries"),
            "{rendered}"
        );
        assert!(rendered.contains("references:\n  [^n]"), "{rendered}");
        assert!(
            rendered.contains("definitions:\n  [^n] note text"),
            "{rendered}"
        );
    }

    #[test]
    fn footnotes_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_footnotes(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md footnotes — 0 entries\n<empty>\n"
        );
    }

    #[test]
    fn images_mode_writes_alt_and_url() {
        let doc = render_markdown("![logo](logo.png)", 80);
        let mut out = Vec::new();
        write_images(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md images — 1 images"),
            "{rendered}"
        );
        assert!(rendered.contains("image #1"), "{rendered}");
        assert!(rendered.contains("alt=logo"), "{rendered}");
        assert!(rendered.contains("url=logo.png"), "{rendered}");
    }

    #[test]
    fn images_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_images(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md images — 0 images\n<empty>\n"
        );
    }

    #[test]
    fn tables_mode_reports_table_metrics_and_rows() {
        let doc = render_markdown("| a | b |\n|:---|---:|\n| 1 | 22 |", 80);
        let mut out = Vec::new();
        write_tables(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md tables — 1 tables"),
            "{rendered}"
        );
        assert!(rendered.contains("table #1"), "{rendered}");
        assert!(rendered.contains("column_widths=[1, 2]"), "{rendered}");
        assert!(
            rendered.contains("alignments=[\"left\", \"right\"]"),
            "{rendered}"
        );
        assert!(rendered.contains("| 1 | 22 |"), "{rendered}");
    }

    #[test]
    fn tables_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_tables(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md tables — 0 tables\n<empty>\n"
        );
    }

    #[test]
    fn stats_mode_reports_document_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md stats\n"), "{rendered}");
        assert!(rendered.contains("source.bytes="), "{rendered}");
        assert!(rendered.contains("source.lines=3"), "{rendered}");
        assert!(rendered.contains("headings=1"), "{rendered}");
        assert!(rendered.contains("links=1"), "{rendered}");
        assert!(rendered.contains("images=1"), "{rendered}");
    }

    #[test]
    fn references_mode_writes_links_images_and_footnotes() {
        let doc = render_markdown(
            "See [site](https://example.com) and ![logo](logo.png)[^n].\n\n[^n]: note text",
            80,
        );
        let mut out = Vec::new();
        write_references(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md references — 4 entries"),
            "{rendered}"
        );
        assert!(
            rendered.contains("links:\n  [site] https://example.com"),
            "{rendered}"
        );
        assert!(
            rendered.contains("images:\n  [logo] logo.png"),
            "{rendered}"
        );
        assert!(
            rendered.contains("footnote references:\n  [^n]"),
            "{rendered}"
        );
        assert!(
            rendered.contains("footnotes:\n  [^n] note text"),
            "{rendered}"
        );
    }

    #[test]
    fn references_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_references(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md references — 0 entries\n<empty>\n"
        );
    }

    #[test]
    fn metadata_json_mode_reports_stable_shape() {
        let doc = render_markdown(
            "# Title\n\nSee [site](https://example.com) and note[^n] plus $x + y$ and <kbd>x</kbd>.\n\n```rust\nfn main() {}\n```\n\n![logo](logo.png)\n\n| a | b | c |\n|:---|:---:|---:|\n| 1 | 2 | 3 |\n\nTerm\n: Definition text\n\n[^n]: note text",
            80,
        );
        let mut out = Vec::new();
        let source = "# Title\n\nSee [site](https://example.com) and note[^n] plus $x + y$ and <kbd>x</kbd>.\n\n```rust\nfn main() {}\n```\n\n![logo](logo.png)\n\n| a | b | c |\n|:---|:---:|---:|\n| 1 | 2 | 3 |\n\nTerm\n: Definition text\n\n[^n]: note text";
        write_metadata_json(&doc, source, 80, Some("proof.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["source"]["bytes"], source.len());
        assert_eq!(value["source"]["lines"], source.lines().count());
        assert_eq!(value["source"]["path"], "proof.md");
        assert_eq!(value["render"]["mode"], "metadata-json");
        assert_eq!(value["render"]["width_cells"], 80);
        assert_eq!(
            value["components"].as_u64().unwrap(),
            doc.components.len() as u64
        );
        assert_eq!(value["components_detail"][0]["kind"], "H1");
        assert_eq!(value["components_detail"][0]["text"], "Title");
        assert_eq!(value["components_detail"][0]["width_cells"], 80);
        assert!(
            value["components_detail"][0]["height_cells"]
                .as_u64()
                .unwrap()
                >= 1
        );
        assert_eq!(value["outline"][0]["level"], 1);
        assert_eq!(value["outline"][0]["text"], "Title");
        assert_eq!(value["links"][0]["url"], "https://example.com");
        assert_eq!(value["images"][0]["url"], "logo.png");
        assert_eq!(value["footnote_references"][0], "n");
        assert_eq!(value["footnotes"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["text"], "note text");
        assert_eq!(value["definitions"][0]["term"], "Term");
        assert_eq!(value["definitions"][0]["definition"], "Definition text");
        assert_eq!(value["math"][0]["kind"], "inline");
        assert_eq!(value["math"][0]["source"], "x + y");
        assert_eq!(value["html"][0]["kind"], "inline");
        assert_eq!(value["html"][0]["source"], "<kbd>");
        assert_eq!(value["code_blocks"][0]["language"], "rust");
        assert_eq!(value["code_blocks"][0]["text"], "fn main() {}");
        assert_eq!(value["tables"][0]["rows"][1][0], "1");
        assert_eq!(value["tables"][0]["alignments"][0], "left");
        assert_eq!(value["tables"][0]["alignments"][1], "center");
        assert_eq!(value["tables"][0]["alignments"][2], "right");
        assert_eq!(
            value["tables"][0]["column_widths"],
            serde_json::json!([1, 1, 1])
        );
        assert!(value["tables"][0]["footprint"]["cols"].as_u64().unwrap() >= 10);
        assert_eq!(value["tables"][0]["footprint"]["rows"], 5);
    }

    #[test]
    fn components_mode_writes_only_component_records() {
        let doc = render_markdown("# Title\n\nSee [site](https://example.com)", 40);
        let mut out = Vec::new();
        write_components(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.starts_with("kittui-md components — "),
            "{rendered}"
        );
        assert!(rendered.contains("[H1] Title"), "{rendered}");
        assert!(rendered.contains("[TextChip] site"), "{rendered}");
        assert!(!rendered.contains("links:"), "{rendered}");
        assert!(!rendered.contains("outline:"), "{rendered}");
    }

    #[test]
    fn components_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_components(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md components — 0 components\n<empty>\n"
        );
    }

    #[test]
    fn outline_mode_writes_only_heading_outline() {
        let doc = MarkdownDocument {
            components: vec![h1("Title", 40), h2("Section", 40)],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_outline(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(
            rendered,
            "kittui-md outline — 2 headings\nTitle\n  Section\n"
        );
        assert!(!rendered.contains("[H1]"));
    }

    #[test]
    fn outline_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_outline(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered, "kittui-md outline — 0 headings\n<empty>\n");
    }

    #[test]
    fn plain_component_indents_multiline_text() {
        let comp = textbox("code:rust\nfn main() {}", 40, Tone::Tool);
        let mut out = Vec::new();
        write_plain_component(&mut out, &comp).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered, "[TextBox] code:rust\n          fn main() {}\n");
    }

    #[test]
    fn plain_metadata_sections_include_images() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![MarkdownImage {
                alt: "logo".to_string(),
                url: "logo.png".to_string(),
            }],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("0 links, 1 images"), "{rendered}");
        assert!(
            rendered.contains("images:\n  [logo] logo.png"),
            "{rendered}"
        );
    }

    #[test]
    fn plain_metadata_sections_include_footnotes() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![MarkdownFootnote {
                label: "note".to_string(),
                text: "details".to_string(),
            }],
            footnote_references: vec!["note".to_string()],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("footnote references:\n  [^note]"),
            "{rendered}"
        );
        assert!(
            rendered.contains("footnotes:\n  [^note] details"),
            "{rendered}"
        );
    }

    #[test]
    fn plain_metadata_sections_include_definitions() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![MarkdownDefinition {
                term: "Term".to_string(),
                definition: "Definition text".to_string(),
            }],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("definitions:\n  Term — Definition text"),
            "{rendered}"
        );
    }

    #[test]
    fn plain_metadata_sections_include_math() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![kittui_affordances::MarkdownMath {
                kind: kittui_affordances::MarkdownMathKind::Inline,
                source: "x + y".to_string(),
            }],
            html: vec![],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("math:\n  inline x + y"), "{rendered}");
    }

    #[test]
    fn plain_metadata_sections_include_html() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![kittui_affordances::MarkdownHtml {
                kind: kittui_affordances::MarkdownHtmlKind::Inline,
                source: "<kbd>".to_string(),
            }],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("html:\n  inline <kbd>"), "{rendered}");
    }

    #[test]
    fn plain_metadata_sections_include_code_blocks() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![kittui_affordances::MarkdownCodeBlock {
                language: Some("rust".to_string()),
                text: "fn main() {}".to_string(),
            }],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("code blocks:\n  rust fn main() {}"),
            "{rendered}"
        );
    }

    #[test]
    fn rich_outline_lines_mirror_plain_indentation() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                },
                HeadingOutline {
                    level: 3,
                    text: "Deep".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        assert_eq!(
            outline_lines(&doc),
            vec!["Title".to_string(), "    Deep".to_string()]
        );
    }

    #[test]
    fn plain_metadata_sections_include_heading_outline() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("outline:\n  Title\n    Section"),
            "{rendered}"
        );
    }

    #[test]
    fn align_table_cell_text_uses_markdown_alignment() {
        assert_eq!(
            align_table_cell_text("x", 3, MarkdownTableAlignment::Left),
            "x  "
        );
        assert_eq!(
            align_table_cell_text("x", 3, MarkdownTableAlignment::Center),
            " x "
        );
        assert_eq!(
            align_table_cell_text("x", 3, MarkdownTableAlignment::Right),
            "  x"
        );
        assert_eq!(
            align_table_cell_text("abcd", 2, MarkdownTableAlignment::Right),
            "ab"
        );
    }

    #[test]
    fn wrap_text_lines_wraps_and_respects_row_limit() {
        assert_eq!(
            wrap_text_lines("one two three four", 9, 3),
            vec![
                "one two".to_string(),
                "three".to_string(),
                "four".to_string()
            ]
        );
        assert_eq!(
            wrap_text_lines("one two three four", 9, 2),
            vec!["one two".to_string(), "three".to_string()]
        );
        assert_eq!(
            wrap_text_lines("abcdefghij", 4, 2),
            vec!["abcd".to_string()]
        );
    }

    #[test]
    fn viewport_filters_by_offset_and_height() {
        let comps = vec![h1("One", 40), h1("Two", 40), h1("Three", 40)];
        let layout = layout_components(&comps, &[], 80);
        let visible = visible_components(&layout, 4, Some(3));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].component.text, "Two");
    }
}
