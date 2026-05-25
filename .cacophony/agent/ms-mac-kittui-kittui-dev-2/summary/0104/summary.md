# Session summary — Control affordance animation

## Goal

Continue animation coverage into reusable `kittui-affordances` control components.

## Bead(s)

- `bd-e80d71` — kittui affordance controls: add animation option to ControlComponent

## Inventory

Control kinds covered by the shared `ControlComponent` API:
- button
- checkbox
- radio
- radio_group
- text_input
- text_area
- select_list
- menu
- slider
- progress
- tabs
- split_pane

## Before state

- CLI visual commands had the standard animation contract.
- `ControlComponent::to_scene` lowered controls to static primitive scenes.

## After state

- Added `ControlAnimation` with defaults matching the broader contract:
  - 60fps
  - 180 frames
  - 3000ms period
  - infinite looping pulse curve
- Added `ControlKind::as_str()` for stable animation layer labels.
- Added builder methods:
  - `ControlComponent::animated(bool)`
  - `ControlComponent::animation(ControlAnimation)`
- `ControlComponent::to_scene` now emits `Scene.animation` and a labelled phase-reactive glow layer when animation is enabled.
- Layer labels follow `control_animation_<kind>`, e.g. `control_animation_button` and `control_animation_slider`.

## Diff summary

- Code/content commits: `4ee3c18` (`bd-e80d71: animate control affordance scenes`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-affordances/src/controls.rs`
- Validation:
  - `cargo test -p kittui-affordances animated_controls_emit_looping_scene_animation -- --test-threads=1`
  - `cargo check -p kittui-affordances`
  - `git diff --check`

## Operator-takeaway

Reusable high-level controls can now opt into the same all-frames-up-front kitty-native animation model as the CLI scene builders.
