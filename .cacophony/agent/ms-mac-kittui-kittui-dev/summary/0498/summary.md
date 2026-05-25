# Session summary — inline chip cursor rewind

## Goal

Fix full-width inline chips displacing text instead of drawing underneath it.

## Bead(s)

- `bd-9b56b3` — fix inline chip text overlay cursor positioning

## Before state

- `kittui inline chip --text abcdef` rendered a full-width chip, but the terminal cursor advanced by the chip width before visible text was emitted.
- Result looked like chip cells followed by `abcdef `, instead of `abcdef` drawn over the chip.

## After state

- Inline kitty placement now emits the full-width z=-1 image placement and then a cursor-back escape for the chip width before writing visible text.
- The cursor-back escape is normal terminal output rather than tmux passthrough so tmux can update its cursor model before fallback text is printed.
- Visible embed now has symmetric padding around text (` abcdef ` for default padding).
- Placement remains full-width (`c=<cols>,r=1,z=-1`) and does not use `U=1`.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- `target/debug/kittui --dry-run --json-bytes inline chip --text abcdef | python3 -m json.tool` shows placement ending in raw `CSI 8D` and no `U=1`.
- `git diff --check` passed.

## Notes

- dev-2 filed additional kittwm QA beads; this bead is limited to the kittui inline primitive.
