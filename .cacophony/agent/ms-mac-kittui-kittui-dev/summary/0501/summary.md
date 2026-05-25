# Session summary — prompt-safe inline chip modes

## Goal

Make `kittui inline chip` usable directly in zsh/bash prompts with kitty graphics while preserving shell prompt width accounting.

## Bead(s)

- `bd-547db1` — kittui inline chip prompt-safe zsh/bash modes

## Changes

- Added `--format prompt-zsh` and `--format prompt-bash` to `kittui inline chip`.
- These formats use the same kitty graphics rendering path as the default `kitty` mode.
- Nonprinting upload, placement, cursor rewind, and SGR color reset/prefix bytes are wrapped for shell prompt editors:
  - zsh: `%{...%}`
  - bash: `\[...\]`
- Width-bearing visible chip text remains outside prompt wrappers.
- Dry-run/json-bytes output now reports wrapped upload/placement/embed bytes for prompt modes.
- Existing `kitty`, `plain`, `ansi`, and `tmux` modes are preserved.

## Validation

- `cargo test -p kittui-cli --bin kittui inline_chip_renders_plain_ansi_tmux_and_kitty_embed_formats -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittui inline_chip_prompt_formats_wrap_only_nonprinting_bytes -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittui` passed.
- `target/debug/kittui --dry-run --json-bytes inline chip --format prompt-zsh --text abcdef | python3 -m json.tool` inspected successfully.
- `git diff --check` passed.
