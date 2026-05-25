# Session summary — inline chip graphics visibility

## Goal

Fix `kittui inline chip --text abcdef` printing placeholder/text without visible graphics.

## Bead(s)

- `bd-bc131a` — fix inline chip graphics visibility

## Root cause

The default kitty inline chip reused `Runtime::place`, whose placement bytes include an absolute cursor move to the scene footprint (`1;1H`). For inline prompt/status use, placement should be anchored by the unicode placeholder at the current output position instead of moving to the top-left before placing.

## After state

- Inline kitty mode strips the cursor-move prefix from placement output and emits only the unicode-placeholder anchored placement command.
- Actual output order remains upload + placement + placeholder/text embed.
- Dry-run/json-bytes output now reports the inline placement bytes that will actually be emitted.
- Existing `plain`, `ansi`, and `tmux` fallback formats remain unchanged.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- `target/debug/kittui --dry-run --json-bytes inline chip --text abcdef | python3 -m json.tool` shows placement without `CSI 1;1H` and embed as placeholder + `abcdef `.
- `git diff --check` passed.

## Note

If running inside tmux, visible kitty graphics still require tmux passthrough support/config. This fix makes direct/placeholder-anchored placement correct; tmux may still show only placeholders if passthrough is disabled.
