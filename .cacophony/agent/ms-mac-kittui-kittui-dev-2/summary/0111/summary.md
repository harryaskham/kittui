# Session summary — Centralized standard animation contract

## Goal

Centralize the standard kitty-native animation contract so affordance defaults do not drift.

## Bead(s)

- `bd-939fc8` — kittui core: centralize standard animation contract

## Before state

- Multiple modules independently spelled the standard animation defaults:
  - 60fps
  - 180 frames
  - 3000ms period
- Loop math existed separately in CLI, controls, components, panel, table glyph, and overlay helpers.

## After state

- Added canonical constants and helpers in `kittui-core`:
  - `STANDARD_ANIMATION_FPS`
  - `STANDARD_ANIMATION_FRAMES`
  - `STANDARD_ANIMATION_CYCLE_MS`
  - `Animation::pulse_fps(frames, fps)`
  - `Animation::standard_loop()`
- Re-exported constants through `kittui`.
- Updated representative callers to use the shared constants/helper:
  - inline CLI animation args
  - primitive CLI legacy parsing helper
  - affordance controls
  - affordance components
  - affordance panel
  - table box glyph animation
  - overlay animation
- Added core tests covering the shared contract and clamping behavior.
- Reverted unrelated formatting-only diffs from rustfmt in files outside this slice.

## Diff summary

- Code/content commits: `cf47fe8` (`bd-939fc8: centralize animation defaults`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-core/src/animation.rs`
  - `crates/kittui-core/src/lib.rs`
  - `crates/kittui/src/lib.rs`
  - `crates/kittui-cli/src/main.rs`
  - `crates/kittui-affordances/src/controls.rs`
  - `crates/kittui-affordances/src/components.rs`
  - `crates/kittui-affordances/src/panel.rs`
  - `crates/kittui-affordances/src/table.rs`
  - `crates/kittui-overlay/src/lib.rs`
- Validation:
  - `cargo test -p kittui-core standard_loop -- --test-threads=1`
  - `cargo test -p kittui-core pulse_fps -- --test-threads=1`
  - `cargo test -p kittui-cli --bin kittui inline_animation_defaults_to_three_second_looping_period -- --test-threads=1`
  - `cargo test -p kittui-affordances animated_controls_emit_looping_scene_animation -- --test-threads=1`
  - `cargo test -p kittui-affordances components_can_attach_native_animation_metadata -- --test-threads=1`
  - `cargo test -p kittui-affordances animated_glyph_scene_uses_default_loop_contract -- --test-threads=1`
  - `cargo test -p kittui-affordances panel_animation -- --test-threads=1`
  - `cargo test -p kittui-overlay overlay_animation -- --test-threads=1`
  - `cargo check -p kittui-cli`
  - `cargo check -p kittui-affordances`
  - `cargo check -p kittui-overlay`
  - `git diff --check`

## Operator-takeaway

The 60fps / 180-frame / 3-second animation period is now a core shared contract instead of duplicated per component.
