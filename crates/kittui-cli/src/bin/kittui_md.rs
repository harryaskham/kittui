//! `kittui-md` — standalone rich kittui Markdown viewer.

use std::io::{Read, Write};
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use kittui::scene::{background_linear, rounded_rect, scene};
use kittui::{CellRect, CellSize, Direction, RendererKind, Rgba, Runtime, Scene, Transport};
use kittui_affordances::{
    box_glyph_scene, render_markdown, ComponentKind, MarkdownDocument, MarkdownTable,
    TableGlyphLayout, UiComponent,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Mode {
    Rich,
    Plain,
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
        Mode::Rich if cfg.interactive => run_interactive(&doc, cfg),
        Mode::Rich => write_rich(&doc, &cfg, &mut std::io::stdout().lock()),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut mode = Mode::Rich;
    let mut width = terminal_cols().unwrap_or(80).clamp(20, 120);
    let mut offset_rows = 0;
    let mut height_rows = None;
    let mut interactive = false;
    let mut path = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--plain" => mode = Mode::Plain,
            "--rich" => mode = Mode::Rich,
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

fn print_help() {
    println!("kittui-md [--rich|--plain] [--interactive] [--width N] [--offset ROWS] [--height ROWS] [file]");
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

fn write_plain(doc: &MarkdownDocument, width: u16, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md — {} components, {} links",
        doc.components.len(),
        doc.links.len()
    )?;
    writeln!(out, "{}", "═".repeat(width as usize))?;
    for comp in &doc.components {
        writeln!(out, "[{:?}] {}", comp.kind, comp.text)?;
    }
    if !doc.links.is_empty() {
        writeln!(out, "\nlinks:")?;
        for link in &doc.links {
            writeln!(out, "  [{}] {}", link.label, link.url)?;
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
        "\x1b[0m\x1b[?25hkittui-md rich view — {} components, {} links; offset={} rows",
        doc.components.len(),
        doc.links.len(),
        cfg.offset_rows
    )?;
    if !doc.links.is_empty() {
        for link in &doc.links {
            writeln!(out, "  🔗 {} — {}", link.label, link.url)?;
        }
    }
    Ok(())
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
                truncate_cells(cell, widths.get(col_idx).copied().unwrap_or(1) as usize)
            )?;
            x = x
                .saturating_add(widths.get(col_idx).copied().unwrap_or(1))
                .saturating_add(3);
        }
    }
    Ok(())
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
    let y = rect.y.saturating_add(rect.rows / 2);
    write!(
        out,
        "{}",
        kittui_kitty::cursor_move(rect.x.saturating_add(x), y, Transport::Direct)
    )?;
    let style = match component.kind {
        ComponentKind::H1 | ComponentKind::Title => "\x1b[1;97m",
        ComponentKind::H2 | ComponentKind::Header => "\x1b[1;96m",
        ComponentKind::H3 | ComponentKind::Footer => "\x1b[1;95m",
        ComponentKind::TextChip => "\x1b[1;94m",
        ComponentKind::Banner => "\x1b[1;93m",
        ComponentKind::TextBox => "\x1b[37m",
    };
    let max = rect.cols.saturating_sub(x + 1) as usize;
    write!(
        out,
        "{style}{}\x1b[0m",
        truncate_cells(&component.text, max)
    )?;
    Ok(())
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
    use kittui_affordances::{h1, textbox, Tone};

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
        };
        assert_eq!(document_rows(&doc, 80), 7);
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
