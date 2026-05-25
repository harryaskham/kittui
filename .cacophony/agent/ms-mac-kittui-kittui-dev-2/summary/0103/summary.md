# Session summary — Image scene animation flags

## Goal

Continue animation coverage by adding the standard kitty-native animation contract to `kittui image`.

## Bead(s)

- `bd-c4c3de` — kittui image: add standard animation flags

## Inventory

Remaining visual element command covered:
- `kittui image`

Utility commands intentionally left unchanged:
- `kittui compose` preserves animation from input scene JSON.
- `kittui render`, `place`, `delete`, `cache`, `probe`, and `proof` are utility/protocol commands rather than visual element builders needing new animation flags.

## Before state

- Inline elements, top-level affordances, primitive box/gradient/glow, and WM chrome scene commands had the standard animation contract.
- `kittui image` rendered static `Node::Image` scenes.

## After state

- Added `--animated`, `--fps`, and `--frames` to `kittui image`.
- Default remains 60fps, 180 frames, 3000ms period, infinite loop.
- Added a labelled phase-reactive overlay layer: `image-animation`.
- Updated `docs/inline-animation.md` and smoke tests.

## Diff summary

- Code/content commits: `a631958` (`bd-c4c3de: animate image scenes`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/main.rs`
  - `crates/kittui-cli/tests/inline_animation_commands.rs`
  - `docs/inline-animation.md`
- Validation:
  - `cargo test -p kittui-cli --test inline_animation_commands image_scene_json_reports_animation_contract -- --test-threads=1`
  - `cargo test -p kittui-cli --test inline_animation_commands -- --test-threads=1`
  - `cargo check -p kittui-cli`
  - `git diff --check`

## Operator-takeaway

`kittui image` now participates in the same all-frames-up-front animation contract as the rest of the visual scene element commands.
