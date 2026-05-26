# Session summary — graphical kittwm chrome inspection

## Bead

- `bd-4003a9` — make kittwm chrome inspection kittui/kitty-native

## Changes

- Added `kittwm chrome-scene-json` and `--chrome-scene-json`.
- Added `kittwm chrome-kitty` / `chrome-graphics` and `--chrome-kitty` / `--chrome-graphics`.
- Existing `--chrome-json` behavior remains unchanged.
- Chrome scene is built from `CHROME_JSON` and labels workspace, owner, top/bottom/left/right reservations, row/column gaps, and tilable rows.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated known command catalog/local command catalog with the new graphical chrome surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` chrome inspection.
- Avoided dev-2 SDK compositor/placement helpers, helper binaries, Runtime/browser/bar/session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm chrome_scene_labels_reservation_contract -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
