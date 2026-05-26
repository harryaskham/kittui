# Session summary — graphical kittwm config

## Bead

- `bd-10a2f7` — make kittwm config kittui/kitty-native

## Changes

- Added `kittwm config-scene-json [--keymap PATH]`.
- Added `kittwm config-kitty [--keymap PATH]` / `kittwm config-graphics [--keymap PATH]`.
- Existing `kittwm config` text behavior remains unchanged.
- Config scene is built from the existing config/keymap readiness data and labels keymap source, launcher env, prefix, binding count, duplicate chords, and readiness status.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated help/known command catalog/local command catalog with the new graphical config surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` config inspection.
- Avoided dev-2 areas: SDK composition-plane/ArchitectureContract/NativeSurfaceContract helpers, kittwm_browser.rs capabilities, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm config_scene_labels_readiness_summary -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
