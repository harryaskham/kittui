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
    ComponentsJson,
    Outline,
    OutlineJson,
    Anchors,
    AnchorsJson,
    References,
    ReferencesJson,
    Links,
    LinksJson,
    Footnotes,
    FootnotesJson,
    Images,
    ImagesJson,
    Tables,
    TablesJson,
    CodeBlocks,
    CodeBlocksJson,
    MetadataBlocks,
    MetadataBlocksJson,
    Definitions,
    DefinitionsJson,
    Math,
    MathJson,
    Html,
    HtmlJson,
    Counts,
    CountsJson,
    Stats,
    StatsJson,
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
        Mode::ComponentsJson => write_components_json(&doc, &mut std::io::stdout().lock()),
        Mode::Outline => write_outline(&doc, &mut std::io::stdout().lock()),
        Mode::OutlineJson => write_outline_json(&doc, &mut std::io::stdout().lock()),
        Mode::Anchors => write_anchors(&doc, &mut std::io::stdout().lock()),
        Mode::AnchorsJson => write_anchors_json(&doc, &mut std::io::stdout().lock()),
        Mode::References => write_references(&doc, &mut std::io::stdout().lock()),
        Mode::ReferencesJson => write_references_json(&doc, &mut std::io::stdout().lock()),
        Mode::Links => write_links(&doc, &mut std::io::stdout().lock()),
        Mode::LinksJson => write_links_json(&doc, &mut std::io::stdout().lock()),
        Mode::Footnotes => write_footnotes(&doc, &mut std::io::stdout().lock()),
        Mode::FootnotesJson => write_footnotes_json(&doc, &mut std::io::stdout().lock()),
        Mode::Images => write_images(&doc, &mut std::io::stdout().lock()),
        Mode::ImagesJson => write_images_json(&doc, &mut std::io::stdout().lock()),
        Mode::Tables => write_tables(&doc, &mut std::io::stdout().lock()),
        Mode::TablesJson => write_tables_json(&doc, &mut std::io::stdout().lock()),
        Mode::CodeBlocks => write_code_blocks(&doc, &mut std::io::stdout().lock()),
        Mode::CodeBlocksJson => write_code_blocks_json(&doc, &mut std::io::stdout().lock()),
        Mode::MetadataBlocks => write_metadata_blocks(&doc, &mut std::io::stdout().lock()),
        Mode::MetadataBlocksJson => write_metadata_blocks_json(&doc, &mut std::io::stdout().lock()),
        Mode::Definitions => write_definitions(&doc, &mut std::io::stdout().lock()),
        Mode::DefinitionsJson => write_definitions_json(&doc, &mut std::io::stdout().lock()),
        Mode::Math => write_math(&doc, &mut std::io::stdout().lock()),
        Mode::MathJson => write_math_json(&doc, &mut std::io::stdout().lock()),
        Mode::Html => write_html(&doc, &mut std::io::stdout().lock()),
        Mode::HtmlJson => write_html_json(&doc, &mut std::io::stdout().lock()),
        Mode::Counts => write_counts(&doc, &mut std::io::stdout().lock()),
        Mode::CountsJson => write_counts_json(&doc, &mut std::io::stdout().lock()),
        Mode::Stats => write_stats(
            &doc,
            &markdown,
            cfg.path.as_deref(),
            cfg.width,
            &mut std::io::stdout().lock(),
        ),
        Mode::StatsJson => write_stats_json(
            &doc,
            &markdown,
            cfg.path.as_deref(),
            cfg.width,
            &mut std::io::stdout().lock(),
        ),
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
            "--widgets" => set_mode(&mut mode, &mut mode_flag, "--widgets", Mode::Components)?,
            "--components-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--components-json",
                Mode::ComponentsJson,
            )?,
            "--outline" => set_mode(&mut mode, &mut mode_flag, "--outline", Mode::Outline)?,
            "--toc" => set_mode(&mut mode, &mut mode_flag, "--toc", Mode::Outline)?,
            "--headings" => set_mode(&mut mode, &mut mode_flag, "--headings", Mode::Outline)?,
            "--outline-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--outline-json",
                Mode::OutlineJson,
            )?,
            "--anchors" => set_mode(&mut mode, &mut mode_flag, "--anchors", Mode::Anchors)?,
            "--slugs" => set_mode(&mut mode, &mut mode_flag, "--slugs", Mode::Anchors)?,
            "--anchors-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--anchors-json",
                Mode::AnchorsJson,
            )?,
            "--references" => {
                set_mode(&mut mode, &mut mode_flag, "--references", Mode::References)?
            }
            "--refs" => set_mode(&mut mode, &mut mode_flag, "--refs", Mode::References)?,
            "--references-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--references-json",
                Mode::ReferencesJson,
            )?,
            "--links" => set_mode(&mut mode, &mut mode_flag, "--links", Mode::Links)?,
            "--urls" => set_mode(&mut mode, &mut mode_flag, "--urls", Mode::Links)?,
            "--links-json" => set_mode(&mut mode, &mut mode_flag, "--links-json", Mode::LinksJson)?,
            "--footnotes" => set_mode(&mut mode, &mut mode_flag, "--footnotes", Mode::Footnotes)?,
            "--notes" => set_mode(&mut mode, &mut mode_flag, "--notes", Mode::Footnotes)?,
            "--footnotes-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--footnotes-json",
                Mode::FootnotesJson,
            )?,
            "--images" => set_mode(&mut mode, &mut mode_flag, "--images", Mode::Images)?,
            "--pictures" => set_mode(&mut mode, &mut mode_flag, "--pictures", Mode::Images)?,
            "--images-json" => {
                set_mode(&mut mode, &mut mode_flag, "--images-json", Mode::ImagesJson)?
            }
            "--tables" => set_mode(&mut mode, &mut mode_flag, "--tables", Mode::Tables)?,
            "--grid" => set_mode(&mut mode, &mut mode_flag, "--grid", Mode::Tables)?,
            "--tables-json" => {
                set_mode(&mut mode, &mut mode_flag, "--tables-json", Mode::TablesJson)?
            }
            "--code-blocks" => {
                set_mode(&mut mode, &mut mode_flag, "--code-blocks", Mode::CodeBlocks)?
            }
            "--snippets" => set_mode(&mut mode, &mut mode_flag, "--snippets", Mode::CodeBlocks)?,
            "--code-blocks-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--code-blocks-json",
                Mode::CodeBlocksJson,
            )?,
            "--metadata-blocks" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-blocks",
                Mode::MetadataBlocks,
            )?,
            "--metadata" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata",
                Mode::MetadataBlocks,
            )?,
            "--frontmatter" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--frontmatter",
                Mode::MetadataBlocks,
            )?,
            "--metadata-blocks-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-blocks-json",
                Mode::MetadataBlocksJson,
            )?,
            "--definitions" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--definitions",
                Mode::Definitions,
            )?,
            "--glossary" => set_mode(&mut mode, &mut mode_flag, "--glossary", Mode::Definitions)?,
            "--definitions-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--definitions-json",
                Mode::DefinitionsJson,
            )?,
            "--math" => set_mode(&mut mode, &mut mode_flag, "--math", Mode::Math)?,
            "--equations" => set_mode(&mut mode, &mut mode_flag, "--equations", Mode::Math)?,
            "--math-json" => set_mode(&mut mode, &mut mode_flag, "--math-json", Mode::MathJson)?,
            "--html" => set_mode(&mut mode, &mut mode_flag, "--html", Mode::Html)?,
            "--markup" => set_mode(&mut mode, &mut mode_flag, "--markup", Mode::Html)?,
            "--html-json" => set_mode(&mut mode, &mut mode_flag, "--html-json", Mode::HtmlJson)?,
            "--counts" => set_mode(&mut mode, &mut mode_flag, "--counts", Mode::Counts)?,
            "--counts-json" => {
                set_mode(&mut mode, &mut mode_flag, "--counts-json", Mode::CountsJson)?
            }
            "--stats" => set_mode(&mut mode, &mut mode_flag, "--stats", Mode::Stats)?,
            "--summary" => set_mode(&mut mode, &mut mode_flag, "--summary", Mode::Stats)?,
            "--stats-json" => set_mode(&mut mode, &mut mode_flag, "--stats-json", Mode::StatsJson)?,
            "--metadata-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-json",
                Mode::MetadataJson,
            )?,
            "--json" => set_mode(&mut mode, &mut mode_flag, "--json", Mode::MetadataJson)?,
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
    println!("kittui-md [--rich|--plain|--components|--widgets|--components-json|--outline|--toc|--headings|--outline-json|--anchors|--slugs|--anchors-json|--references|--refs|--references-json|--links|--urls|--links-json|--footnotes|--notes|--footnotes-json|--images|--pictures|--images-json|--tables|--grid|--tables-json|--code-blocks|--snippets|--code-blocks-json|--metadata-blocks|--metadata|--frontmatter|--metadata-blocks-json|--definitions|--glossary|--definitions-json|--math|--equations|--math-json|--html|--markup|--html-json|--counts|--counts-json|--stats|--summary|--stats-json|--metadata-json|--json] [--interactive] [--width N] [--offset ROWS] [--height ROWS] [file]");
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

fn write_components_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "components": doc.components.iter().enumerate().map(|(index, component)| serde_json::json!({
            "index": index,
            "kind": format!("{:?}", component.kind),
            "text": component.text,
            "width_cells": component.width_cells,
            "height_cells": component.height_cells,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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

fn write_outline_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "outline": doc.outline.iter().enumerate().map(|(index, heading)| serde_json::json!({
            "index": index,
            "level": heading.level,
            "text": heading.text,
            "anchor": heading.anchor,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_anchors(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md anchors — {} headings", doc.outline.len())?;
    if doc.outline.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for heading in &doc.outline {
        writeln!(
            out,
            "h{} #{} {}",
            heading.level, heading.anchor, heading.text
        )?;
    }
    Ok(())
}

fn write_anchors_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "anchors": doc.outline.iter().enumerate().map(|(index, heading)| serde_json::json!({
            "index": index,
            "level": heading.level,
            "anchor": heading.anchor,
            "text": heading.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_stats(
    doc: &MarkdownDocument,
    source: &str,
    source_path: Option<&str>,
    width_cells: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md stats")?;
    writeln!(out, "source.bytes={}", source.len())?;
    writeln!(out, "source.lines={}", source.lines().count())?;
    writeln!(out, "source.path={}", source_path.unwrap_or("<stdin>"))?;
    writeln!(out, "render.width_cells={width_cells}")?;
    write_count_lines(doc, out)
}

fn write_stats_json(
    doc: &MarkdownDocument,
    source: &str,
    source_path: Option<&str>,
    width_cells: u16,
    out: &mut impl Write,
) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "source": {
            "bytes": source.len(),
            "lines": source.lines().count(),
            "path": source_path.unwrap_or("<stdin>"),
        },
        "render": {
            "mode": "stats-json",
            "width_cells": width_cells,
        },
        "counts": metadata_counts(doc),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_counts(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md counts")?;
    write_count_lines(doc, out)
}

fn write_count_lines(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "components={}", doc.components.len())?;
    writeln!(out, "headings={}", doc.outline.len())?;
    writeln!(out, "heading_anchors={}", doc.outline.len())?;
    writeln!(out, "links={}", doc.links.len())?;
    writeln!(out, "images={}", doc.images.len())?;
    writeln!(out, "tables={}", doc.tables.len())?;
    writeln!(out, "footnote_references={}", doc.footnote_references.len())?;
    writeln!(out, "footnotes={}", doc.footnotes.len())?;
    writeln!(out, "definitions={}", doc.definitions.len())?;
    writeln!(out, "math={}", doc.math.len())?;
    writeln!(out, "html={}", doc.html.len())?;
    writeln!(out, "metadata_blocks={}", doc.metadata_blocks.len())?;
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
        if let Some(title) = &link.title {
            writeln!(out, "  title={title}")?;
        }
    }
    Ok(())
}

fn write_links_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "links": doc.links.iter().enumerate().map(|(index, link)| serde_json::json!({
            "index": index,
            "label": link.label,
            "url": link.url,
            "title": link.title,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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

fn write_footnotes_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "references": doc.footnote_references.iter().enumerate().map(|(index, label)| serde_json::json!({
            "index": index,
            "label": label,
        })).collect::<Vec<_>>(),
        "definitions": doc.footnotes.iter().enumerate().map(|(index, footnote)| serde_json::json!({
            "index": index,
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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
        if let Some(title) = &image.title {
            writeln!(out, "  title={title}")?;
        }
    }
    Ok(())
}

fn write_images_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "images": doc.images.iter().enumerate().map(|(index, image)| serde_json::json!({
            "index": index,
            "alt": image.alt,
            "url": image.url,
            "title": image.title,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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

fn write_tables_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "tables": doc.tables.iter().enumerate().map(|(index, table)| {
            let footprint = table.footprint();
            serde_json::json!({
                "index": index,
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

fn write_html_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "html": doc.html.iter().enumerate().map(|(index, html)| serde_json::json!({
            "index": index,
            "kind": html.kind.as_str(),
            "source": html.source,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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

fn write_math_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "math": doc.math.iter().enumerate().map(|(index, math)| serde_json::json!({
            "index": index,
            "kind": math.kind.as_str(),
            "source": math.source,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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

fn write_definitions_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "definitions": doc.definitions.iter().enumerate().map(|(index, definition)| serde_json::json!({
            "index": index,
            "term": definition.term,
            "definition": definition.definition,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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

fn write_code_blocks_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "code_blocks": doc.code_blocks.iter().enumerate().map(|(index, block)| serde_json::json!({
            "index": index,
            "language": block.language,
            "text": block.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_metadata_blocks(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md metadata blocks — {} metadata blocks",
        doc.metadata_blocks.len()
    )?;
    if doc.metadata_blocks.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, metadata) in doc.metadata_blocks.iter().enumerate() {
        writeln!(out, "metadata block #{}", i + 1)?;
        writeln!(out, "  kind={}", metadata.kind.as_str())?;
        writeln!(out, "---")?;
        writeln!(out, "{}", metadata.source)?;
        writeln!(out, "---")?;
    }
    Ok(())
}

fn write_metadata_blocks_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "metadata_blocks": doc.metadata_blocks.iter().enumerate().map(|(index, metadata)| serde_json::json!({
            "index": index,
            "kind": metadata.kind.as_str(),
            "source": metadata.source,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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
            if let Some(title) = &link.title {
                writeln!(out, "  [{}] {} \"{}\"", link.label, link.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", link.label, link.url)?;
            }
        }
    }
    if !doc.images.is_empty() {
        writeln!(out, "images:")?;
        for image in &doc.images {
            if let Some(title) = &image.title {
                writeln!(out, "  [{}] {} \"{}\"", image.alt, image.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", image.alt, image.url)?;
            }
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

fn write_references_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "links": doc.links.iter().enumerate().map(|(index, link)| serde_json::json!({
            "index": index,
            "label": link.label,
            "url": link.url,
            "title": link.title,
        })).collect::<Vec<_>>(),
        "images": doc.images.iter().enumerate().map(|(index, image)| serde_json::json!({
            "index": index,
            "alt": image.alt,
            "url": image.url,
            "title": image.title,
        })).collect::<Vec<_>>(),
        "footnote_references": doc.footnote_references.iter().enumerate().map(|(index, label)| serde_json::json!({
            "index": index,
            "label": label,
        })).collect::<Vec<_>>(),
        "footnotes": doc.footnotes.iter().enumerate().map(|(index, footnote)| serde_json::json!({
            "index": index,
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn metadata_counts(doc: &MarkdownDocument) -> serde_json::Value {
    serde_json::json!({
        "components": doc.components.len(),
        "headings": doc.outline.len(),
        "heading_anchors": doc.outline.len(),
        "links": doc.links.len(),
        "images": doc.images.len(),
        "tables": doc.tables.len(),
        "footnote_references": doc.footnote_references.len(),
        "footnotes": doc.footnotes.len(),
        "definitions": doc.definitions.len(),
        "math": doc.math.len(),
        "html": doc.html.len(),
        "metadata_blocks": doc.metadata_blocks.len(),
        "code_blocks": doc.code_blocks.len(),
    })
}

fn write_counts_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "counts": metadata_counts(doc),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
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
        "counts": metadata_counts(doc),
        "components": doc.components.len(),
        "components_detail": doc.components.iter().enumerate().map(|(index, component)| serde_json::json!({
            "index": index,
            "kind": format!("{:?}", component.kind),
            "text": component.text,
            "width_cells": component.width_cells,
            "height_cells": component.height_cells,
        })).collect::<Vec<_>>(),
        "links": doc.links.iter().enumerate().map(|(index, link)| serde_json::json!({
            "index": index,
            "label": link.label,
            "url": link.url,
            "title": link.title,
        })).collect::<Vec<_>>(),
        "images": doc.images.iter().enumerate().map(|(index, image)| serde_json::json!({
            "index": index,
            "alt": image.alt,
            "url": image.url,
            "title": image.title,
        })).collect::<Vec<_>>(),
        "footnote_references": doc.footnote_references,
        "footnotes": doc.footnotes.iter().enumerate().map(|(index, footnote)| serde_json::json!({
            "index": index,
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
        "definitions": doc.definitions.iter().enumerate().map(|(index, definition)| serde_json::json!({
            "index": index,
            "term": definition.term,
            "definition": definition.definition,
        })).collect::<Vec<_>>(),
        "math": doc.math.iter().enumerate().map(|(index, math)| serde_json::json!({
            "index": index,
            "kind": math.kind.as_str(),
            "source": math.source,
        })).collect::<Vec<_>>(),
        "html": doc.html.iter().enumerate().map(|(index, html)| serde_json::json!({
            "index": index,
            "kind": html.kind.as_str(),
            "source": html.source,
        })).collect::<Vec<_>>(),
        "metadata_blocks": doc.metadata_blocks.iter().enumerate().map(|(index, metadata)| serde_json::json!({
            "index": index,
            "kind": metadata.kind.as_str(),
            "source": metadata.source,
        })).collect::<Vec<_>>(),
        "code_blocks": doc.code_blocks.iter().enumerate().map(|(index, code)| serde_json::json!({
            "index": index,
            "language": code.language,
            "text": code.text,
        })).collect::<Vec<_>>(),
        "outline": doc.outline.iter().enumerate().map(|(index, heading)| serde_json::json!({
            "index": index,
            "level": heading.level,
            "text": heading.text,
            "anchor": heading.anchor,
        })).collect::<Vec<_>>(),
        "tables": doc.tables.iter().enumerate().map(|(index, table)| {
            let footprint = table.footprint();
            serde_json::json!({
                "index": index,
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
            if let Some(title) = &link.title {
                writeln!(out, "  [{}] {} \"{}\"", link.label, link.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", link.label, link.url)?;
            }
        }
    }
    if !doc.images.is_empty() {
        writeln!(out, "\nimages:")?;
        for image in &doc.images {
            if let Some(title) = &image.title {
                writeln!(out, "  [{}] {} \"{}\"", image.alt, image.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", image.alt, image.url)?;
            }
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
    if !doc.metadata_blocks.is_empty() {
        writeln!(out, "\nmetadata blocks:")?;
        for metadata in &doc.metadata_blocks {
            writeln!(
                out,
                "  {} {}",
                metadata.kind.as_str(),
                metadata.source.lines().next().unwrap_or("")
            )?;
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
                format!("{} #{}", heading.text, heading.anchor)
            )
        })
        .collect()
}

fn rich_status_line(doc: &MarkdownDocument, cfg: &Config, total_rows: u16) -> String {
    let viewport = cfg.height_rows.unwrap_or(total_rows);
    let max_offset = total_rows.saturating_sub(viewport);
    format!(
        "kittui-md rich view — {} components, {} headings, {} heading anchors, {} links, {} images, {} tables, {} footnote refs, {} footnotes, {} definitions, {} math, {} html, {} metadata blocks, {} code blocks; offset={}/{} rows; viewport={}; total_rows={}",
        doc.components.len(),
        doc.outline.len(),
        doc.outline.len(),
        doc.links.len(),
        doc.images.len(),
        doc.tables.len(),
        doc.footnote_references.len(),
        doc.footnotes.len(),
        doc.definitions.len(),
        doc.math.len(),
        doc.html.len(),
        doc.metadata_blocks.len(),
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
        h1, h2, textbox, HeadingOutline, MarkdownDefinition, MarkdownFootnote, MarkdownImage,
        MarkdownMetadataBlock, MarkdownMetadataBlockKind, Tone,
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
    fn parse_args_accepts_widgets_alias() {
        let cfg = parse_args(["--widgets".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Components);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_components_json_mode() {
        let cfg = parse_args(["--components-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ComponentsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_components_plus_components_json() {
        let err =
            parse_args(["--components".to_string(), "--components-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--components"), "{err}");
        assert!(err.to_string().contains("--components-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_components_plus_widgets() {
        let err = parse_args(["--components".to_string(), "--widgets".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--components"), "{err}");
        assert!(err.to_string().contains("--widgets"), "{err}");
    }

    #[test]
    fn parse_args_accepts_toc_alias() {
        let cfg = parse_args(["--toc".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Outline);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_headings_alias() {
        let cfg = parse_args(["--headings".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Outline);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_outline_json_mode() {
        let cfg = parse_args(["--outline-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::OutlineJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_outline_plus_outline_json() {
        let err = parse_args(["--outline".to_string(), "--outline-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
        assert!(err.to_string().contains("--outline-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_anchors_mode() {
        let cfg = parse_args(["--anchors".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Anchors);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_slugs_alias() {
        let cfg = parse_args(["--slugs".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Anchors);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_anchors_json_mode() {
        let cfg = parse_args(["--anchors-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::AnchorsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_anchors_plus_anchors_json() {
        let err = parse_args(["--anchors".to_string(), "--anchors-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--anchors"), "{err}");
        assert!(err.to_string().contains("--anchors-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_anchors_plus_slugs() {
        let err = parse_args(["--anchors".to_string(), "--slugs".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--anchors"), "{err}");
        assert!(err.to_string().contains("--slugs"), "{err}");
    }

    #[test]
    fn parse_args_accepts_json_alias() {
        let cfg = parse_args(["--json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_summary_alias() {
        let cfg = parse_args(["--summary".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Stats);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_stats_json_mode() {
        let cfg = parse_args(["--stats-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::StatsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_stats_plus_stats_json() {
        let err = parse_args(["--stats".to_string(), "--stats-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--stats"), "{err}");
        assert!(err.to_string().contains("--stats-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_counts_mode() {
        let cfg = parse_args(["--counts".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Counts);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_counts_json_mode() {
        let cfg = parse_args(["--counts-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CountsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_counts_plus_counts_json() {
        let err = parse_args(["--counts".to_string(), "--counts-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--counts"), "{err}");
        assert!(err.to_string().contains("--counts-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_counts_plus_stats() {
        let err = parse_args(["--counts".to_string(), "--stats".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--counts"), "{err}");
        assert!(err.to_string().contains("--stats"), "{err}");
    }

    #[test]
    fn parse_args_accepts_refs_alias() {
        let cfg = parse_args(["--refs".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::References);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_references_json_mode() {
        let cfg = parse_args(["--references-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ReferencesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_references_plus_references_json() {
        let err =
            parse_args(["--references".to_string(), "--references-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--references"), "{err}");
        assert!(err.to_string().contains("--references-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_snippets_alias() {
        let cfg = parse_args(["--snippets".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CodeBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_code_blocks_json_mode() {
        let cfg = parse_args(["--code-blocks-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CodeBlocksJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_code_blocks_plus_code_blocks_json() {
        let err = parse_args([
            "--code-blocks".to_string(),
            "--code-blocks-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--code-blocks"), "{err}");
        assert!(err.to_string().contains("--code-blocks-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_glossary_alias() {
        let cfg = parse_args(["--glossary".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Definitions);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_definitions_json_mode() {
        let cfg = parse_args(["--definitions-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::DefinitionsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_definitions_plus_definitions_json() {
        let err = parse_args([
            "--definitions".to_string(),
            "--definitions-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--definitions"), "{err}");
        assert!(err.to_string().contains("--definitions-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_markup_alias() {
        let cfg = parse_args(["--markup".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Html);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_html_json_mode() {
        let cfg = parse_args(["--html-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::HtmlJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_html_plus_html_json() {
        let err = parse_args(["--html".to_string(), "--html-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--html"), "{err}");
        assert!(err.to_string().contains("--html-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_equations_alias() {
        let cfg = parse_args(["--equations".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Math);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_math_json_mode() {
        let cfg = parse_args(["--math-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MathJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_math_plus_math_json() {
        let err = parse_args(["--math".to_string(), "--math-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--math"), "{err}");
        assert!(err.to_string().contains("--math-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_math_plus_equations() {
        let err = parse_args(["--math".to_string(), "--equations".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--math"), "{err}");
        assert!(err.to_string().contains("--equations"), "{err}");
    }

    #[test]
    fn parse_args_rejects_html_plus_markup() {
        let err = parse_args(["--html".to_string(), "--markup".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--html"), "{err}");
        assert!(err.to_string().contains("--markup"), "{err}");
    }

    #[test]
    fn parse_args_accepts_pictures_alias() {
        let cfg = parse_args(["--pictures".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Images);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_images_json_mode() {
        let cfg = parse_args(["--images-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ImagesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_images_plus_images_json() {
        let err = parse_args(["--images".to_string(), "--images-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--images"), "{err}");
        assert!(err.to_string().contains("--images-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_grid_alias() {
        let cfg = parse_args(["--grid".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Tables);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_tables_json_mode() {
        let cfg = parse_args(["--tables-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::TablesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_tables_plus_tables_json() {
        let err = parse_args(["--tables".to_string(), "--tables-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--tables"), "{err}");
        assert!(err.to_string().contains("--tables-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_tables_plus_grid() {
        let err = parse_args(["--tables".to_string(), "--grid".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--tables"), "{err}");
        assert!(err.to_string().contains("--grid"), "{err}");
    }

    #[test]
    fn parse_args_accepts_urls_alias() {
        let cfg = parse_args(["--urls".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Links);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_links_json_mode() {
        let cfg = parse_args(["--links-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::LinksJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_links_plus_links_json() {
        let err = parse_args(["--links".to_string(), "--links-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--links"), "{err}");
        assert!(err.to_string().contains("--links-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_notes_alias() {
        let cfg = parse_args(["--notes".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Footnotes);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_footnotes_json_mode() {
        let cfg = parse_args(["--footnotes-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::FootnotesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_footnotes_plus_footnotes_json() {
        let err =
            parse_args(["--footnotes".to_string(), "--footnotes-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--footnotes"), "{err}");
        assert!(err.to_string().contains("--footnotes-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_footnotes_plus_notes() {
        let err = parse_args(["--footnotes".to_string(), "--notes".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--footnotes"), "{err}");
        assert!(err.to_string().contains("--notes"), "{err}");
    }

    #[test]
    fn parse_args_rejects_links_plus_urls() {
        let err = parse_args(["--links".to_string(), "--urls".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--links"), "{err}");
        assert!(err.to_string().contains("--urls"), "{err}");
    }

    #[test]
    fn parse_args_rejects_images_plus_pictures() {
        let err = parse_args(["--images".to_string(), "--pictures".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--images"), "{err}");
        assert!(err.to_string().contains("--pictures"), "{err}");
    }

    #[test]
    fn parse_args_rejects_definitions_plus_glossary() {
        let err = parse_args(["--definitions".to_string(), "--glossary".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--definitions"), "{err}");
        assert!(err.to_string().contains("--glossary"), "{err}");
    }

    #[test]
    fn parse_args_rejects_code_blocks_plus_snippets() {
        let err = parse_args(["--code-blocks".to_string(), "--snippets".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--code-blocks"), "{err}");
        assert!(err.to_string().contains("--snippets"), "{err}");
    }

    #[test]
    fn parse_args_rejects_references_plus_refs() {
        let err = parse_args(["--references".to_string(), "--refs".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--references"), "{err}");
        assert!(err.to_string().contains("--refs"), "{err}");
    }

    #[test]
    fn parse_args_rejects_stats_plus_summary() {
        let err = parse_args(["--stats".to_string(), "--summary".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--stats"), "{err}");
        assert!(err.to_string().contains("--summary"), "{err}");
    }

    #[test]
    fn parse_args_rejects_metadata_json_plus_json() {
        let err = parse_args(["--metadata-json".to_string(), "--json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-json"), "{err}");
        assert!(err.to_string().contains("--json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_outline_plus_toc() {
        let err = parse_args(["--outline".to_string(), "--toc".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
        assert!(err.to_string().contains("--toc"), "{err}");
    }

    #[test]
    fn parse_args_rejects_outline_plus_headings() {
        let err = parse_args(["--outline".to_string(), "--headings".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
        assert!(err.to_string().contains("--headings"), "{err}");
    }

    #[test]
    fn parse_args_accepts_metadata_blocks_mode() {
        let cfg = parse_args(["--metadata-blocks".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_frontmatter_alias() {
        let cfg = parse_args(["--frontmatter".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_metadata_alias() {
        let cfg = parse_args(["--metadata".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_metadata_blocks_json_mode() {
        let cfg = parse_args(["--metadata-blocks-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocksJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_metadata_blocks_plus_metadata_blocks_json() {
        let err = parse_args([
            "--metadata-blocks".to_string(),
            "--metadata-blocks-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_frontmatter_plus_metadata_blocks() {
        let err =
            parse_args(["--metadata-blocks".to_string(), "--frontmatter".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks"), "{err}");
        assert!(err.to_string().contains("--frontmatter"), "{err}");
    }

    #[test]
    fn parse_args_rejects_metadata_plus_metadata_blocks() {
        let err =
            parse_args(["--metadata-blocks".to_string(), "--metadata".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks"), "{err}");
        assert!(err.to_string().contains("--metadata"), "{err}");
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
            metadata_blocks: vec![],
        };
        assert_eq!(document_rows(&doc, 80), 7);
    }

    #[test]
    fn rich_status_line_reports_offset_viewport_and_total_rows() {
        let doc = MarkdownDocument {
            components: vec![h1("One", 40), h1("Two", 40)],
            links: vec![],
            tables: vec![MarkdownTable::new(vec![vec!["A".into()]])],
            images: vec![MarkdownImage {
                alt: "logo".to_string(),
                url: "logo.png".to_string(),
                title: None,
            }],
            outline: vec![HeadingOutline {
                level: 1,
                text: "Title".to_string(),
                anchor: "title".to_string(),
            }],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![MarkdownMetadataBlock {
                kind: MarkdownMetadataBlockKind::Yaml,
                source: "title: Proof".to_string(),
            }],
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
            status.contains("1 headings, 1 heading anchors, 0 links, 1 images, 1 tables, 0 footnote refs, 0 footnotes, 0 definitions, 0 math, 0 html, 1 metadata blocks, 0 code blocks"),
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
    fn html_json_mode_writes_html_records() {
        let doc = render_markdown("hello <kbd>x</kbd>\n\n<div>block</div>", 80);
        let mut out = Vec::new();
        write_html_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["html"][0]["index"], 0);
        assert_eq!(value["html"][0]["kind"], "inline");
        assert_eq!(value["html"][0]["source"], "<kbd>");
        assert_eq!(value["html"][2]["index"], 2);
        assert_eq!(value["html"][2]["kind"], "block");
        assert_eq!(value["html"][2]["source"], "<div>block</div>");
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
    fn math_json_mode_writes_math_records() {
        let doc = render_markdown("inline $x + y$\n\n$$\na^2\n$$", 80);
        let mut out = Vec::new();
        write_math_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["math"][0]["index"], 0);
        assert_eq!(value["math"][0]["kind"], "inline");
        assert_eq!(value["math"][0]["source"], "x + y");
        assert_eq!(value["math"][1]["index"], 1);
        assert_eq!(value["math"][1]["kind"], "display");
        assert_eq!(value["math"][1]["source"], "a^2");
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
    fn definitions_json_mode_writes_definition_records() {
        let doc = render_markdown("Term\n: Definition text", 80);
        let mut out = Vec::new();
        write_definitions_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["definitions"][0]["index"], 0);
        assert_eq!(value["definitions"][0]["term"], "Term");
        assert_eq!(value["definitions"][0]["definition"], "Definition text");
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
    fn code_blocks_json_mode_writes_code_block_records() {
        let doc = render_markdown("```rust\nfn main() {}\n```\n\n```\nplain\n```", 80);
        let mut out = Vec::new();
        write_code_blocks_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["code_blocks"][0]["index"], 0);
        assert_eq!(value["code_blocks"][0]["language"], "rust");
        assert_eq!(value["code_blocks"][0]["text"], "fn main() {}");
        assert_eq!(value["code_blocks"][1]["index"], 1);
        assert_eq!(value["code_blocks"][1]["language"], serde_json::Value::Null);
        assert_eq!(value["code_blocks"][1]["text"], "plain");
    }

    #[test]
    fn metadata_blocks_mode_writes_kind_and_source() {
        let doc = MarkdownDocument {
            metadata_blocks: vec![MarkdownMetadataBlock {
                kind: MarkdownMetadataBlockKind::Yaml,
                source: "title: Proof".to_string(),
            }],
            ..MarkdownDocument::default()
        };
        let mut out = Vec::new();
        write_metadata_blocks(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md metadata blocks — 1 metadata blocks"),
            "{rendered}"
        );
        assert!(rendered.contains("metadata block #1"), "{rendered}");
        assert!(rendered.contains("kind=yaml"), "{rendered}");
        assert!(rendered.contains("title: Proof"), "{rendered}");
    }

    #[test]
    fn metadata_blocks_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_metadata_blocks(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md metadata blocks — 0 metadata blocks\n<empty>\n"
        );
    }

    #[test]
    fn metadata_blocks_json_mode_writes_metadata_block_records() {
        let doc = render_markdown("---\ntitle: Proof\n---\n\n# Body", 80);
        let mut out = Vec::new();
        write_metadata_blocks_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["metadata_blocks"][0]["index"], 0);
        assert_eq!(value["metadata_blocks"][0]["kind"], "yaml");
        assert_eq!(value["metadata_blocks"][0]["source"], "title: Proof");
    }

    #[test]
    fn links_mode_writes_label_url_and_title() {
        let doc = render_markdown("See [site](https://example.com \"Example title\")", 80);
        let mut out = Vec::new();
        write_links(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md links — 1 links"), "{rendered}");
        assert!(rendered.contains("link #1"), "{rendered}");
        assert!(rendered.contains("label=site"), "{rendered}");
        assert!(rendered.contains("url=https://example.com"), "{rendered}");
        assert!(rendered.contains("title=Example title"), "{rendered}");
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
    fn links_json_mode_writes_link_records() {
        let doc = render_markdown("See [site](https://example.com \"Example title\")", 80);
        let mut out = Vec::new();
        write_links_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["links"][0]["index"], 0);
        assert_eq!(value["links"][0]["label"], "site");
        assert_eq!(value["links"][0]["url"], "https://example.com");
        assert_eq!(value["links"][0]["title"], "Example title");
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
    fn footnotes_json_mode_writes_references_and_definitions() {
        let doc = render_markdown("see[^n]\n\n[^n]: note text", 80);
        let mut out = Vec::new();
        write_footnotes_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["references"][0]["index"], 0);
        assert_eq!(value["references"][0]["label"], "n");
        assert_eq!(value["definitions"][0]["index"], 0);
        assert_eq!(value["definitions"][0]["label"], "n");
        assert_eq!(value["definitions"][0]["text"], "note text");
    }

    #[test]
    fn images_mode_writes_alt_url_and_title() {
        let doc = render_markdown("![logo](logo.png \"Logo title\")", 80);
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
        assert!(rendered.contains("title=Logo title"), "{rendered}");
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
    fn images_json_mode_writes_image_records() {
        let doc = render_markdown("![logo](logo.png \"Logo title\")", 80);
        let mut out = Vec::new();
        write_images_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["images"][0]["index"], 0);
        assert_eq!(value["images"][0]["alt"], "logo");
        assert_eq!(value["images"][0]["url"], "logo.png");
        assert_eq!(value["images"][0]["title"], "Logo title");
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
    fn tables_json_mode_writes_table_records() {
        let doc = render_markdown("| a | b |\n|:---|---:|\n| 1 | 22 |", 80);
        let mut out = Vec::new();
        write_tables_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["tables"][0]["index"], 0);
        assert_eq!(value["tables"][0]["rows"][1][1], "22");
        assert_eq!(value["tables"][0]["alignments"][0], "left");
        assert_eq!(value["tables"][0]["alignments"][1], "right");
        assert_eq!(
            value["tables"][0]["column_widths"],
            serde_json::json!([1, 2])
        );
        assert!(value["tables"][0]["footprint"]["cols"].as_u64().unwrap() >= 6);
    }

    #[test]
    fn stats_mode_reports_document_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, None, 80, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md stats\n"), "{rendered}");
        assert!(rendered.contains("source.bytes="), "{rendered}");
        assert!(rendered.contains("source.lines=3"), "{rendered}");
        assert!(rendered.contains("source.path=<stdin>"), "{rendered}");
        assert!(rendered.contains("render.width_cells=80"), "{rendered}");
        assert!(rendered.contains("headings=1"), "{rendered}");
        assert!(rendered.contains("heading_anchors=1"), "{rendered}");
        assert!(rendered.contains("links=1"), "{rendered}");
        assert!(rendered.contains("images=1"), "{rendered}");
    }

    #[test]
    fn stats_mode_reports_source_path_when_present() {
        let source = "# Title";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, Some("docs/proof.md"), 72, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("source.path=docs/proof.md"), "{rendered}");
        assert!(rendered.contains("render.width_cells=72"), "{rendered}");
    }

    #[test]
    fn stats_json_mode_reports_source_render_and_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats_json(&doc, source, Some("docs/proof.md"), 72, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["source"]["bytes"], source.len());
        assert_eq!(value["source"]["lines"], 3);
        assert_eq!(value["source"]["path"], "docs/proof.md");
        assert_eq!(value["render"]["mode"], "stats-json");
        assert_eq!(value["render"]["width_cells"], 72);
        assert_eq!(value["counts"]["headings"], 1);
        assert_eq!(value["counts"]["links"], 1);
        assert_eq!(value["counts"]["images"], 1);
        assert!(value.get("components_detail").is_none());
    }

    #[test]
    fn counts_mode_reports_counts_without_source_provenance() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_counts(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.starts_with("kittui-md counts\n"), "{rendered}");
        assert!(rendered.contains("components="), "{rendered}");
        assert!(rendered.contains("headings=1"), "{rendered}");
        assert!(rendered.contains("links=1"), "{rendered}");
        assert!(rendered.contains("images=1"), "{rendered}");
        assert!(!rendered.contains("source.path="), "{rendered}");
        assert!(!rendered.contains("render.width_cells="), "{rendered}");
    }

    #[test]
    fn counts_json_mode_reports_machine_readable_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_counts_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["counts"]["headings"], 1);
        assert_eq!(value["counts"]["heading_anchors"], 1);
        assert_eq!(value["counts"]["links"], 1);
        assert_eq!(value["counts"]["images"], 1);
        assert!(value.get("source").is_none());
        assert!(value.get("components_detail").is_none());
    }

    #[test]
    fn references_mode_writes_links_images_and_footnotes() {
        let doc = render_markdown(
            "See [site](https://example.com \"Example title\") and ![logo](logo.png \"Logo title\")[^n].\n\n[^n]: note text",
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
            rendered.contains("links:\n  [site] https://example.com \"Example title\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("images:\n  [logo] logo.png \"Logo title\""),
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
    fn references_json_mode_writes_combined_reference_records() {
        let doc = render_markdown(
            "See [site](https://example.com \"Example title\") and ![logo](logo.png \"Logo title\")[^n].\n\n[^n]: note text",
            80,
        );
        let mut out = Vec::new();
        write_references_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["links"][0]["index"], 0);
        assert_eq!(value["links"][0]["label"], "site");
        assert_eq!(value["links"][0]["title"], "Example title");
        assert_eq!(value["images"][0]["index"], 0);
        assert_eq!(value["images"][0]["alt"], "logo");
        assert_eq!(value["images"][0]["title"], "Logo title");
        assert_eq!(value["footnote_references"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["text"], "note text");
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
        assert_eq!(value["counts"]["components"], doc.components.len());
        assert_eq!(value["counts"]["headings"], 1);
        assert_eq!(value["counts"]["heading_anchors"], 1);
        assert_eq!(value["counts"]["links"], 1);
        assert_eq!(value["counts"]["images"], 1);
        assert_eq!(value["counts"]["tables"], 1);
        assert_eq!(value["counts"]["footnote_references"], 1);
        assert_eq!(value["counts"]["footnotes"], 1);
        assert_eq!(value["counts"]["definitions"], 1);
        assert_eq!(value["counts"]["math"], 1);
        assert_eq!(value["counts"]["html"], 2);
        assert_eq!(value["counts"]["metadata_blocks"], 0);
        assert_eq!(value["counts"]["code_blocks"], 1);
        assert_eq!(value["components_detail"][0]["index"], 0);
        assert_eq!(value["components_detail"][0]["kind"], "H1");
        assert_eq!(value["components_detail"][0]["text"], "Title");
        assert_eq!(value["components_detail"][0]["width_cells"], 80);
        assert!(
            value["components_detail"][0]["height_cells"]
                .as_u64()
                .unwrap()
                >= 1
        );
        assert_eq!(value["outline"][0]["index"], 0);
        assert_eq!(value["outline"][0]["level"], 1);
        assert_eq!(value["outline"][0]["text"], "Title");
        assert_eq!(value["outline"][0]["anchor"], "title");
        assert_eq!(value["links"][0]["index"], 0);
        assert_eq!(value["links"][0]["url"], "https://example.com");
        assert_eq!(value["links"][0]["title"], serde_json::Value::Null);
        assert_eq!(value["images"][0]["index"], 0);
        assert_eq!(value["images"][0]["url"], "logo.png");
        assert_eq!(value["images"][0]["title"], serde_json::Value::Null);
        assert_eq!(value["footnote_references"][0], "n");
        assert_eq!(value["footnotes"][0]["index"], 0);
        assert_eq!(value["footnotes"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["text"], "note text");
        assert_eq!(value["definitions"][0]["index"], 0);
        assert_eq!(value["definitions"][0]["term"], "Term");
        assert_eq!(value["definitions"][0]["definition"], "Definition text");
        assert_eq!(value["math"][0]["index"], 0);
        assert_eq!(value["math"][0]["kind"], "inline");
        assert_eq!(value["math"][0]["source"], "x + y");
        assert_eq!(value["html"][0]["index"], 0);
        assert_eq!(value["html"][0]["kind"], "inline");
        assert_eq!(value["html"][0]["source"], "<kbd>");
        assert_eq!(value["code_blocks"][0]["index"], 0);
        assert_eq!(value["code_blocks"][0]["language"], "rust");
        assert_eq!(value["code_blocks"][0]["text"], "fn main() {}");
        assert_eq!(value["tables"][0]["index"], 0);
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
    fn metadata_json_mode_reports_metadata_blocks() {
        let source = "---\ntitle: Proof\n---\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_metadata_json(&doc, source, 80, Some("frontmatter.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["metadata_blocks"][0]["index"], 0);
        assert_eq!(value["metadata_blocks"][0]["kind"], "yaml");
        assert_eq!(value["metadata_blocks"][0]["source"], "title: Proof");
    }

    #[test]
    fn metadata_json_mode_reports_link_and_image_titles() {
        let source =
            "[site](https://example.com \"Example title\")\n\n![logo](logo.png \"Logo title\")";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_metadata_json(&doc, source, 80, Some("titles.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["links"][0]["title"], "Example title");
        assert_eq!(value["images"][0]["title"], "Logo title");
    }

    #[test]
    fn metadata_json_mode_reports_pluses_metadata_blocks() {
        let source = "+++\ntitle = \"Proof\"\n+++\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_metadata_json(&doc, source, 80, Some("frontmatter.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["metadata_blocks"][0]["kind"], "pluses");
        assert_eq!(value["metadata_blocks"][0]["source"], "title = \"Proof\"");
    }

    #[test]
    fn stats_mode_counts_metadata_blocks() {
        let source = "---\ntitle: Proof\n---\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, None, 80, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("metadata_blocks=1"), "{rendered}");
    }

    #[test]
    fn plain_metadata_sections_include_metadata_blocks() {
        let source = "---\ntitle: Proof\n---\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_plain(&doc, 80, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("metadata blocks:\n  yaml title: Proof"),
            "{rendered}"
        );
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
    fn components_json_mode_writes_component_records() {
        let doc = render_markdown("# Title\n\nSee [site](https://example.com)", 40);
        let mut out = Vec::new();
        write_components_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["components"][0]["index"], 0);
        assert_eq!(value["components"][0]["kind"], "H1");
        assert_eq!(value["components"][0]["text"], "Title");
        assert_eq!(value["components"][0]["width_cells"], 40);
        assert_eq!(value["components"][0]["height_cells"], 3);
        assert!(value["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| { component["kind"] == "TextChip" && component["text"] == "site" }));
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
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                    anchor: "section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_outline(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(
            rendered,
            "kittui-md outline — 2 headings\nTitle #title\n  Section #section\n"
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
    fn outline_json_mode_writes_heading_outline() {
        let doc = render_markdown("# Title\n\n## Section", 80);
        let mut out = Vec::new();
        write_outline_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["outline"][0]["index"], 0);
        assert_eq!(value["outline"][0]["level"], 1);
        assert_eq!(value["outline"][0]["text"], "Title");
        assert_eq!(value["outline"][0]["anchor"], "title");
        assert_eq!(value["outline"][1]["index"], 1);
        assert_eq!(value["outline"][1]["level"], 2);
        assert_eq!(value["outline"][1]["anchor"], "section");
    }

    #[test]
    fn anchors_mode_writes_heading_anchors() {
        let doc = render_markdown("# Title\n\n## Section", 80);
        let mut out = Vec::new();
        write_anchors(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(
            rendered,
            "kittui-md anchors — 2 headings\nh1 #title Title\nh2 #section Section\n"
        );
    }

    #[test]
    fn anchors_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_anchors(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered, "kittui-md anchors — 0 headings\n<empty>\n");
    }

    #[test]
    fn anchors_json_mode_writes_heading_anchors() {
        let doc = render_markdown("# Title\n\n## Section", 80);
        let mut out = Vec::new();
        write_anchors_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["anchors"][0]["index"], 0);
        assert_eq!(value["anchors"][0]["level"], 1);
        assert_eq!(value["anchors"][0]["anchor"], "title");
        assert_eq!(value["anchors"][0]["text"], "Title");
        assert_eq!(value["anchors"][1]["index"], 1);
        assert_eq!(value["anchors"][1]["anchor"], "section");
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
    fn plain_metadata_sections_include_links_and_images_with_titles() {
        let doc = render_markdown(
            "[site](https://example.com \"Example title\")\n\n![logo](logo.png \"Logo title\")",
            80,
        );
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("1 links, 1 images"), "{rendered}");
        assert!(
            rendered.contains("links:\n  [site] https://example.com \"Example title\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("images:\n  [logo] logo.png \"Logo title\""),
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
            metadata_blocks: vec![],
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
            metadata_blocks: vec![],
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
            metadata_blocks: vec![],
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
            metadata_blocks: vec![],
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
            metadata_blocks: vec![],
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
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 3,
                    text: "Deep".to_string(),
                    anchor: "deep".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        assert_eq!(
            outline_lines(&doc),
            vec!["Title #title".to_string(), "    Deep #deep".to_string()]
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
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                    anchor: "section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("outline:\n  Title #title\n    Section #section"),
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
