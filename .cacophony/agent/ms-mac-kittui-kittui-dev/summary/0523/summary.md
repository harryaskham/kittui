# Session summary — graphical kittwm shortcuts

## Bead

- `bd-02184f` — make kittwm shortcuts kittui/kitty-native

## Changes

- Added `kittwm shortcuts-scene-json`.
- Added `kittwm shortcuts-kitty` / `kittwm shortcuts-graphics`.
- Existing `kittwm shortcuts` text and `shortcuts-json` behavior remains unchanged.
- The scene is built from the shared `NATIVE_SHORTCUT_ENTRIES` catalog and labels each shortcut row with id/keys/description.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at overlay z-index 30.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` shortcut inspection.
- Avoided dev-2 areas: browser semantic output, native-surfaces/ArchitectureContract helpers, Runtime/browser internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm shortcuts_scene_labels_shared_shortcut_catalog -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
