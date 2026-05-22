//! `kittui-md` — standalone rich kittui Markdown viewer.

use std::io::{Read, Write};
use std::process::ExitCode;

use anyhow::Result;
use kittui_affordances::render_markdown;

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
    let mut args = std::env::args().skip(1);
    let markdown = if let Some(path) = args.next() {
        std::fs::read_to_string(path)?
    } else {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    };
    let width = terminal_cols().unwrap_or(80).min(120);
    let doc = render_markdown(&markdown, width);
    let mut out = std::io::stdout().lock();
    writeln!(out, "kittui-md — {} components, {} links", doc.components.len(), doc.links.len())?;
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

fn terminal_cols() -> Option<u16> {
    let mut ws = libc::winsize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 { Some(ws.ws_col) } else { None }
}
