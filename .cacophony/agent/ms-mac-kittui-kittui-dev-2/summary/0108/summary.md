# Session summary — Example animation defaults

## Goal

Continue animation coverage into first-party kittui examples/showcases.

## Bead(s)

- `bd-7e8010` — kittui examples: standardize animation defaults

## Inventory

Example/showcase files covered:
- `crates/kittui-cli/examples/showcase.rs`
- `crates/kittui-cli/examples/ratatui_showcase.rs`

## Before state

- Example panel animation used fixed 8 frames / 800ms.
- Ratatui visual lab defaults used fixed 8 frames / 800ms and limited pulse-frame adjustment to 32.
- These examples no longer matched the broader kittui animation contract.

## After state

- Added shared example constants for the standard contract:
  - 60fps
  - 180 frames
  - 3000ms period
- `showcase.rs` animated panels now use 180 frames / 3000ms.
- `ratatui_showcase.rs` default controls now start at 180 frames / 3000ms.
- Ratatui showcase pulse-frame adjustment range now allows up to 360 frames in 10-frame increments, keeping 180-frame defaults practical.

## Diff summary

- Code/content commits: `1768d45` (`bd-7e8010: standardize example animation defaults`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/examples/showcase.rs`
  - `crates/kittui-cli/examples/ratatui_showcase.rs`
- Validation:
  - `cargo check -p kittui-cli --example showcase`
  - `cargo check -p kittui-cli --example ratatui_showcase`
  - `git diff --check`

## Operator-takeaway

First-party examples now demonstrate the same standard 60fps / 180-frame / 3-second native animation period used by the CLI and affordance APIs.
