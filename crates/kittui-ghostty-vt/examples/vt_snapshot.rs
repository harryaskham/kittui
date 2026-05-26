use kittui_ghostty_vt::GhosttyVtTerminal;

fn main() -> anyhow::Result<()> {
    let mut term = GhosttyVtTerminal::new(40, 8, 200)?;
    term.write(b"kittui ghostty-vt proof\n\x1b[32mportable VT parser\x1b[0m\n");
    let snapshot = term.snapshot()?;
    println!(
        "cols={} rows={} cursor=({}, {})",
        snapshot.cols, snapshot.rows, snapshot.cursor_x, snapshot.cursor_y
    );
    println!("title={:?}", snapshot.title);
    println!("plain:\n{}", snapshot.plain_text);
    Ok(())
}
