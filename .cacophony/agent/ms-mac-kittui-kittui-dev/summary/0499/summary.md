# Session summary — reusable inline theme tokens

## Goal

Move inline component theme/style/color resolution out of the kittui CLI so prompts, shell scripts, kittwm chrome, kittwm-bar, and future inline components can share the same visual defaults.

## Bead(s)

- `bd-2949e9` — kittui inline theme tokens reusable library

## Before state

- Nord/glass/chrome/metal/neon inline chip color logic lived inside `crates/kittui-cli/src/main.rs`.
- kittwm graphical chrome work could not reuse the same defaults without copying values.

## After state

- Added `crates/kittui-affordances/src/inline_theme.rs`.
- Exposed:
  - `InlineTheme`
  - `InlineStyle`
  - `InlineChipColors`
  - `parse_nord_inline_color`
- `InlineChipColors::resolve(theme, style)` provides shared defaults.
- `InlineChipColors::with_overrides(...)` applies fill/border/foreground overrides.
- `kittui inline chip` now consumes those library tokens through CLI enum conversions.
- Current CLI behavior/flags are preserved.

## Validation

- `cargo test -p kittui-affordances inline_theme -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- `git diff --check` passed.

## Coordination

- dev-2 landed `bd-f32b5c` (kittui-rendered split borders/gutters) and noted temporary inline colors pending this extraction. This bead supplies the reusable token module for follow-up dogfooding.
