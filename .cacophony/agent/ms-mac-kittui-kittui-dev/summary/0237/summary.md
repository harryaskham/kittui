# Session summary — kittui place subcommand

## Goal

Close a shell-renderer gap by implementing the DESIGN-documented `kittui place` workflow for re-placing an existing/cached kitty image id without re-rendering.

## Bead(s)

- `bd-fb87d3` — kittui-cli: implement cached image place subcommand

## Before state

- Failing tests: none known.
- Relevant gap: scripts could obtain image ids from render commands, but there was no `kittui place --id ...` CLI to move/re-anchor an uploaded image from shell workflows.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui parse_image_id -- --nocapture` passed.
  - `cargo test -p kittui-cli --test place_command -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui -- place --id 0x1234 --x 2 --y 3 --cols 4 --rows 2 --json --json-bytes | python3 -c '...'` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `kittui place --id ID --x X --y Y --cols C --rows R` supports decimal and `0x...` image ids, emits placement+embed by default, and honors `--json`, `--json-bytes`, `--dry-run`, `--placement-only`, and `--embed-only`.

## Diff summary

- Code/content commit: `7407f94`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/place_command.rs`, `README.md`
- Behavioural delta: shell scripts can now re-place existing kitty image ids without invoking Rust APIs.

## Operator-takeaway

The kittui CLI now supports a complete render/place shell loop: render once to get an id, then move/re-place it by id with channelized output controls.
