# Session summary — inline chip full-width themed background

## Goal

Fix inline chips rendering as a 1x1 placeholder-bound image and improve default styling/theme controls.

## Bead(s)

- `bd-96d04a` — kittui inline chip full-width graphics and theme

## Before state

- `kittui inline chip --text abcdef` rendered graphics confined to the single unicode placeholder cell.
- The visible text appeared beside it, but the pill background did not span the text width.
- Default styling was still the earlier palette and lacked easy inline theme/style/color overrides.

## After state

- Kitty inline mode now renders a full-width background image spanning `text width + padding + 1` cells.
- The image is placed at the current cursor position without `U=1`, with `z=-1`, so visible text is printed over the full-width background instead of being constrained to one placeholder cell.
- Embed output is now visible styled text plus trailing padding, not a kitty placeholder cell.
- Default graphics style is Nord/glass:
  - translucent fill,
  - high-contrast stroked border,
  - subtle highlight.
- Added `--theme nord`, `--style glass|chrome|metal|neon`, and `--bg-color` / `--border-color` / `--fg-color` overrides. Color overrides accept hex, simple Nord names, or Nord palette indices.
- `plain`, `ansi`, and `tmux` fallback formats remain available.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- `target/debug/kittui --dry-run --json-bytes inline chip --text abcdef | python3 -m json.tool` shows placement as full `c=8,r=1,z=-1` without `U=1`, and upload bytes for the full chip scene.
- `git diff --check` passed.

## Files touched

- `crates/kittui-cli/src/main.rs`
