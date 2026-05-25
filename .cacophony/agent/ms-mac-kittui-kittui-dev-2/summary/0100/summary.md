# Session summary — Top-level affordance animation

## Goal

Continue animation coverage beyond inline elements by adding the shared kitty-native animation contract to top-level kittui affordance scene commands.

## Bead(s)

- `bd-7f95f5` — kittui affordances: animate top-level panel/chip/divider/title-bar

## Inventory

Top-level affordance commands that needed the same `--animated`, `--fps`, `--frames` contract:
- `kittui panel`
- `kittui chip`
- `kittui divider`
- `kittui title-bar`

Legacy note:
- `kittui panel --animate` remains accepted as a compatibility alias for enabling animation, while the new `--animated --fps --frames` controls define the standard contract.

## Before state

- Inline chip/badge/segment/divider/row already supported all-frames-up-front kitty-native animation.
- Top-level `chip`, `divider`, and `title-bar` were static.
- Top-level `panel` had only legacy `--animate` with fixed 8@800 behavior.

## After state

- Added flattened animation flags to top-level panel/chip/divider/title-bar.
- Default top-level affordance animation is the same as inline: 60fps, 180 frames, 3000ms period, infinite loop.
- Added named phase-reactive glow layers:
  - `affordance-panel-animation`
  - `affordance-chip-animation`
  - `affordance-divider-animation`
  - `affordance-title-bar-animation`
- Updated `docs/inline-animation.md` to cover top-level affordance commands too.
- Extended CLI smoke tests to cover top-level animated scene JSON.

## Diff summary

- Code/content commits: `53916f8` (`bd-7f95f5: animate top-level affordances`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/main.rs`
  - `crates/kittui-cli/tests/inline_animation_commands.rs`
  - `docs/inline-animation.md`
- Validation:
  - `cargo test -p kittui-cli --test inline_animation_commands -- --test-threads=1`
  - `cargo check -p kittui-cli`
  - `git diff --check`

## Operator-takeaway

The same animation contract now covers both inline prompt/statusline elements and top-level first-party affordance scene commands.
