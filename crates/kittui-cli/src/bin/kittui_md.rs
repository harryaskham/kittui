//! `kittui-md` — standalone rich kittui Markdown viewer.

use std::io::{Read, Write};
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use kittui::scene::{background_linear, rounded_rect, scene};
use kittui::{CellRect, CellSize, Direction, RendererKind, Rgba, Runtime, Scene, Transport};
use kittui_affordances::{render_markdown, ComponentKind, MarkdownDocument, UiComponent};

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
    path: Option<String>,
}

#[derive(Clone, Debug)]
struct LaidOutComponent<'a> {
    component: &'a UiComponent,
    rect: CellRect,
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
        Mode::Rich => write_rich(&doc, &cfg, &mut std::io::stdout().lock()),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut mode = Mode::Rich;
    let mut width = terminal_cols().unwrap_or(80).clamp(20, 120);
    let mut offset_rows = 0;
    let mut height_rows = None;
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
        path,
    })
}

fn print_help() {
    println!("kittui-md [--rich|--plain] [--width N] [--offset ROWS] [--height ROWS] [file]");
    println!(
        "Render Markdown as kittui/kitty graphics components. Reads stdin when file is omitted."
    );
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
    let layout = layout_components(&doc.components, cfg.width);
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

fn layout_components(components: &[UiComponent], width: u16) -> Vec<LaidOutComponent<'_>> {
    let mut y = 0;
    let mut out = Vec::with_capacity(components.len());
    for component in components {
        let rows = component.height_cells.max(1);
        let cols = component.width_cells.min(width).max(1);
        out.push(LaidOutComponent {
            component,
            rect: CellRect::new(0, y, cols, rows),
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
    use kittui_affordances::h1;

    #[test]
    fn layout_stacks_components_with_gaps() {
        let comps = vec![h1("Title", 40), h1("Next", 40)];
        let layout = layout_components(&comps, 80);
        assert_eq!(layout[0].rect.y, 0);
        assert_eq!(layout[1].rect.y, 4);
    }

    #[test]
    fn viewport_filters_by_offset_and_height() {
        let comps = vec![h1("One", 40), h1("Two", 40), h1("Three", 40)];
        let layout = layout_components(&comps, 80);
        let visible = visible_components(&layout, 4, Some(3));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].component.text, "Two");
    }
}
