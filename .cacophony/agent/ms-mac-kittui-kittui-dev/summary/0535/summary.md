# Session summary — graphical kittwm launcher preview

## Bead

- `bd-9f5367` — make kittwm launcher preview kittui/kitty-native

## Changes

- Added `kittwm launcher-scene-json [--filter Q] [--limit N] [--select N]`.
- Added `kittwm launcher-kitty [--filter Q] [--limit N] [--select N]` / `launcher-graphics`.
- Existing `kittwm launcher` text preview and `--launch-selection` behavior remains unchanged.
- Launcher scene is built from existing launcher candidate discovery and labels query, selected row, candidate count, selected candidate, and per-candidate rows.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated help/known command catalog/local command catalog with the new graphical launcher surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` launcher preview inspection.
- Avoided SDK placement/ArchitectureContract/NativeSurfaceContract helper work, browser/capabilities, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm launcher_scene_labels_selected_candidate -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
