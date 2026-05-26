# Session summary — graphical kittwm session inspection

## Bead

- `bd-bbf6a9` — make kittwm session inspection kittui/kitty-native

## Changes

- Added `kittwm session-scene-json` and `--session-scene-json`.
- Added `kittwm session-kitty` / `session-graphics` and `--session-kitty` / `--session-graphics`.
- Existing `--session-json`, `--save-session`, and `--restore-session` behavior remains unchanged.
- Session scene is built from `SESSION_JSON` and labels manifest kind/schema, layout, focus, pane count, and per-pane window/title/command/weight/focus rows.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated known command catalog/local command catalog with the new graphical session surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` SESSION_JSON inspection.
- Avoided session internals, dev-2 SDK placement/coverage helpers, helper binaries, Runtime/browser/bar internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm session_scene_labels_manifest_panes -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
