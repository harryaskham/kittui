# Session summary — inline row composition

## Goal

Let prompts/status scripts render multiple kittui inline components in one process invocation instead of spawning `kittui` once per chip/segment/divider.

## Bead(s)

- `bd-f98ff4` — kittui inline row composition command

## Changes

- Added `kittui inline row`.
- Supports repeated `--item` values:
  - `chip:TEXT`
  - `badge:TEXT`
  - `segment:TEXT`
  - `divider:WIDTH`
  - `divider:WIDTH:GLYPH`
- Supports shared row flags:
  - `--format` with kitty/prompt/plain/ansi/tmux modes
  - `--tone`
  - `--theme`
  - `--style`
  - `--padding`
  - `--gap`
- Kitty/prompt modes compose one scene/upload/placement for the whole row and one visible embed string.
- Text fallback modes concatenate existing component fallback outputs in item order.
- Dry-run JSON reports `inline_component: row`, `inline_items`, footprint, placement, upload, and embed.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_row_composes_multiple_items_in_order -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui inline_badge_segment_and_divider_have_one_line_outputs -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- Manual smoke:
  - `target/debug/kittui inline row --format plain --item chip:main --item badge:ok --item divider:3:= --item segment:dev --gap 1`
  - `target/debug/kittui --dry-run --json-bytes inline row --item chip:main --item badge:ok --item divider:3:= --item segment:dev --gap 1 | python3 -m json.tool`
- `git diff --check` passed.
