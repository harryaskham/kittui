# Session summary — graphical kittwm keymap

## Bead

- `bd-204d9a` — make kittwm keymap kittui/kitty-native

## Changes

- Added `kittwm keymap-scene-json [--keymap PATH]`.
- Added `kittwm keymap-kitty [--keymap PATH]` / `kittwm keymap-graphics [--keymap PATH]`.
- Existing `kittwm keymap`, `--keymap PATH`, and `--check` behavior remains unchanged.
- Keymap scene is built from the resolved `kittui_cli::keymap::Keymap` and labels binding count, prefix, duplicate count, and per-binding chord/action rows.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated help/known command catalog/local command catalog with the new graphical keymap surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` keymap inspection.
- Avoided dev-2 areas: SDK composition-plane helpers, kittwm_browser.rs capabilities, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm keymap_scene_labels_prefix_bindings_and_actions -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
