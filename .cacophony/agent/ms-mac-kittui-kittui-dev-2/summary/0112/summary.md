# Session summary — Examples use shared animation constants

## Goal

Finish the follow-up after centralizing the animation contract by removing duplicated example-local constants.

## Bead(s)

- `bd-57492c` — kittui examples: use shared animation constants

## Before state

- `crates/kittui-cli/examples/showcase.rs` and `crates/kittui-cli/examples/ratatui_showcase.rs` each defined local `SHOWCASE_ANIMATION_*` constants duplicating the shared 60fps / 180-frame / 3000ms contract.

## After state

- Examples now import and use shared kittui constants:
  - `STANDARD_ANIMATION_FRAMES`
  - `STANDARD_ANIMATION_CYCLE_MS`
- Removed duplicated local frame/FPS/cycle constants.

## Diff summary

- Code/content commits: `d89fbca` (`bd-57492c: use shared animation constants in examples`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/examples/showcase.rs`
  - `crates/kittui-cli/examples/ratatui_showcase.rs`
- Validation:
  - `cargo check -p kittui-cli --example showcase`
  - `cargo check -p kittui-cli --example ratatui_showcase`
  - `git diff --check`

## Operator-takeaway

First-party examples now consume the centralized standard animation contract instead of carrying local duplicates.
