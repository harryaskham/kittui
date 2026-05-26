# Session summary — graphical kittwm panes

## Bead

- `bd-820a30` — make kittwm panes kittui/kitty-native

## Changes

- Added `kittwm panes-scene-json` / `--panes-scene-json`.
- Added `kittwm panes-kitty` / `panes-graphics` and `--panes-kitty` / `--panes-graphics`.
- Existing `kittwm panes` / `--panes` text and `panes-json` / `--panes-json` behavior remains unchanged.
- Pane scene is built from `PANES_JSON` and labels pane count, focus, layout, per-pane title/focus/app bounds.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` panes inspection.
- Avoided dev-2 areas: kittwm_browser.rs capabilities, native-surfaces/ArchitectureContract helpers, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm panes_scene_labels_focus_layout_and_app_bounds -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
