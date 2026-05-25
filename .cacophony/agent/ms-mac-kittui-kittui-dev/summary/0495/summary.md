# Session summary — inline chip kitty graphics default

## Goal

Make `kittui inline chip` behave like a kittui graphical inline component by default while keeping explicit text/statusline fallbacks.

## Bead(s)

- `bd-62f542` — kittui inline chip defaults to kitty graphics

## Before state

- `kittui inline chip` defaulted to ANSI text output.
- User expected default kittui behavior to emit a kitty graphics-backed inline chip with width-bearing fallback text, with ASCII/ANSI/tmux available explicitly.

## After state

- `kittui inline chip --text TEXT` now defaults to `--format kitty`.
- Kitty mode builds a one-row chip scene using existing chip chrome sized to text width + padding + one placeholder cell.
- It emits upload + placement + custom inline embed text by default.
- The inline embed begins with a kitty unicode placeholder cell and then prints the visible text plus trailing padding, e.g. placeholder + `abcdef `.
- `--format plain`, `--format ansi`, and `--format tmux` remain explicit text/statusline fallbacks.
- `--dry-run --json-bytes` exposes the upload/placement/embed strings for validation.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- Foreground smoke:
  - `target/debug/kittui inline chip --text abcdef --format plain`
  - `target/debug/kittui inline chip --text abcdef --format tmux`
  - `target/debug/kittui --dry-run --json-bytes inline chip --text abcdef | python3 -m json.tool`
- `git diff --check` passed.

## Files touched

- `crates/kittui-cli/src/main.rs`
