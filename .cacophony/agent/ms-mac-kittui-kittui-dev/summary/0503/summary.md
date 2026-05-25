# Session summary — inline badge, segment, divider

## Goal

Expand `kittui inline` beyond chip so shell prompts, statuslines, scripts, and future kittwm chrome can use more one-line styled building blocks with the same kitty/default and fallback format model.

## Bead(s)

- `bd-1f98c2` — kittui inline badge segment divider components

## Changes

- Added `kittui inline badge`.
  - Reuses chip args/theme/style/color/output modes.
  - Plain fallback renders `< text >`.
  - Kitty/prompt modes render a compact rectangular badge background with visible text overlay.

- Added `kittui inline segment`.
  - Reuses chip args/theme/style/color/output modes.
  - Plain fallback renders just padded visible text.
  - Kitty/prompt modes render a squarer prompt/status segment.

- Added `kittui inline divider`.
  - Args: `--width`, `--glyph`, `--format`, `--tone`, `--theme`, `--style`, `--color`.
  - Defaults to kitty graphics with explicit plain/ansi/tmux/prompt fallbacks.
  - Kitty mode renders a one-row graphical rule and visible width-bearing glyph text.

- All new components remain deterministic and one-line.
- Dry-run JSON for kitty components reports `inline_component` plus wrapped upload/placement/embed bytes like chip.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_badge_segment_and_divider_have_one_line_outputs -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui inline_chip_prompt_formats_wrap_only_nonprinting_bytes -- --nocapture` passed before commit during this bead.
- `cargo build -p kittui-cli --bin kittui` passed.
- Manual smoke:
  - `target/debug/kittui inline badge --format plain --text ok`
  - `target/debug/kittui inline segment --format plain --text ok`
  - `target/debug/kittui inline divider --format plain --width 5 --glyph '='`
  - `target/debug/kittui --dry-run --json-bytes inline badge --text ok | python3 -m json.tool`
  - `target/debug/kittui --dry-run --json-bytes inline divider --width 5 | python3 -m json.tool`
- `git diff --check` passed.
