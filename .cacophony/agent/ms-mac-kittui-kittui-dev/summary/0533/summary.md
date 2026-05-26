# Session summary — graphical kittwm status

## Bead

- `bd-d3974e` — make kittwm status kittui/kitty-native

## Changes

- Added `kittwm status-scene-json` and `--status-scene-json`.
- Added `kittwm status-kitty` / `status-graphics` and `--status-kitty` / `--status-graphics`.
- Existing `kittwm status`, `--status`, and `--status-json` behavior remains unchanged.
- Status scene is built from `STATUS_JSON` and labels pid, uptime, socket, workspace, layout, focus, panes, and pending count.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated help/known command catalog/local command catalog with the new graphical status surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` status inspection.
- Avoided SDK placement/ArchitectureContract/NativeSurfaceContract helper work, browser/capabilities, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm status_scene_labels_daemon_snapshot -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
