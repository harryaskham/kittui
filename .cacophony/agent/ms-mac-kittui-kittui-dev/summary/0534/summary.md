# Session summary — graphical kittwm apps inspection

## Bead

- `bd-922b18` — make kittwm apps inspection kittui/kitty-native

## Changes

- Added `kittwm apps-scene-json [--filter Q] [--limit N]`.
- Added `kittwm apps-kitty [--filter Q] [--limit N]` / `kittwm apps-graphics [--filter Q] [--limit N]`.
- Existing `kittwm apps`, `--json`, `--first`, and `--launch-first` behavior remains unchanged.
- Apps scene is built from existing launcher candidate discovery and labels default command, default path resolution, filter, limit, PATH candidate count, macOS candidate count, and per-candidate rows.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated help/known command catalog/local command catalog with the new graphical apps surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` apps inspection.
- Avoided dev-2 SDK placement iterator/helper work, browser/capabilities, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm apps_scene_labels_launcher_candidates -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
