# Session summary — inline examples command

## Goal

Add a source-level CLI examples command for kittui inline prompt/statusline usage so supported formats are discoverable and smoke-tested in code.

## Bead(s)

- `bd-850e7f` — kittui inline prompt status examples command

## Changes

- Added `kittui inline examples`.
- It prints copy/paste examples for:
  - default kitty graphics chip,
  - zsh prompt-safe graphics mode,
  - bash prompt-safe graphics mode,
  - tmux statusline fallback,
  - plain fallback,
  - ANSI fallback,
  - style/color knobs.
- Added a unit test ensuring the examples mention key supported modes and prompt-width caveats.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_examples_cover_prompt_status_and_fallback_modes -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui inline_chip_prompt_formats_wrap_only_nonprinting_bytes -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- `target/debug/kittui inline examples | head -80` inspected successfully.
- `git diff --check` passed.
