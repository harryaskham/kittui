use std::io::Read;
use std::path::PathBuf;

use kittui_ghostty_vt::{render_snapshot_preview_png, GhosttyVtTerminal, PreviewOptions};

#[derive(Debug)]
struct Args {
    out: PathBuf,
    cols: u16,
    rows: u16,
    demo: bool,
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if args.demo || input.is_empty() {
        input = demo_bytes();
    }

    let mut terminal = GhosttyVtTerminal::new(args.cols, args.rows, 1_000)?;
    terminal.write(&input);
    let snapshot = terminal.render_snapshot()?;
    let png = render_snapshot_preview_png(&snapshot, &PreviewOptions::default())?;
    std::fs::write(&args.out, png)?;
    println!(
        "kittui-ghostty wrote {} ({}x{} cells, cursor={}, {})",
        args.out.display(),
        snapshot.cols,
        snapshot.rows,
        snapshot.cursor_x,
        snapshot.cursor_y
    );
    Ok(())
}

fn parse_args() -> anyhow::Result<Args> {
    let mut out = PathBuf::from("/tmp/kittui-ghostty.png");
    let mut cols = 64u16;
    let mut rows = 12u16;
    let mut demo = false;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                out = iter
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("--out PATH"))?;
            }
            "--cols" => {
                cols = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--cols N"))?
                    .parse()?;
            }
            "--rows" => {
                rows = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--rows N"))?
                    .parse()?;
            }
            "--demo" => demo = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other:?}; try --help"),
        }
    }
    Ok(Args {
        out,
        cols,
        rows,
        demo,
    })
}

fn print_help() {
    println!(
        "kittui-ghostty — portable headless libghostty-vt PNG preview\n\n\
         Usage: kittui-ghostty [--out PATH] [--cols N] [--rows N] [--demo]\n\n\
         Reads VT bytes from stdin. If stdin is empty or --demo is passed, renders demo content."
    );
}

fn demo_bytes() -> Vec<u8> {
    b"kittui-ghostty CLI\n\
      \x1b[32mportable libghostty-vt\x1b[0m render-state preview\n\
      stdin -> Ghostty VT state -> kittui-owned PNG\n\
      \x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[36mcolor\x1b[0m\n"
        .to_vec()
}
