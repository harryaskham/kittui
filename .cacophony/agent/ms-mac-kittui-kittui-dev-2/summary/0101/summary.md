# Session summary — Primitive scene animation flags

## Goal

Continue animation coverage by adding the standard kitty-native animation contract to primitive `kittui` scene commands.

## Bead(s)

- `bd-f3cabd` — kittui primitives: add standard animation flags to box/gradient/glow

## Inventory

Primitive commands covered in this slice:
- `kittui box`
- `kittui gradient`
- `kittui glow`

Compatibility note:
- `kittui box --animate frames@cycle_ms` remains supported as the legacy spelling.
- New standard flags are `--animated`, `--fps`, and `--frames` with 60fps/180-frame/3s defaults.

## Before state

- Inline prompt/statusline elements and top-level affordance commands had the standard animation contract.
- `kittui box` only had legacy `--animate frames@cycle_ms`.
- `kittui gradient` and `kittui glow` were static scene commands.

## After state

- Added shared flattened animation flags to box/gradient/glow.
- Added labelled phase-reactive glow layers:
  - `primitive-box-animation`
  - `primitive-gradient-animation`
  - `primitive-glow-animation`
- Updated `docs/inline-animation.md` to mention primitive commands and effect labels.
- Extended `inline_animation_commands` smoke tests to cover primitive animated scene JSON.

## Diff summary

- Code/content commits: `731ab86` (`bd-f3cabd: animate primitive scene commands`)
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

The same `--animated --fps --frames` contract now spans inline elements, top-level affordance commands, and primitive box/gradient/glow scene commands.
