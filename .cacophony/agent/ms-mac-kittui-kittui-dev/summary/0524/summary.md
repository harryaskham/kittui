# Session summary — graphical kittwm info

## Bead

- `bd-1bd73b` — make kittwm info kittui/kitty-native

## Changes

- Added `kittwm info-scene-json`.
- Added `kittwm info-kitty` / `kittwm info-graphics`.
- Existing `kittwm info` text behavior remains unchanged.
- Info scene is built from the same STATUS_JSON / CHROME_JSON / PANES_JSON snapshot and labels workspace, focus, layout, chrome rows, and panes.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` info inspection.
- Avoided dev-2 areas: browser/capabilities, native-surfaces/ArchitectureContract helpers, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm info_output_formats_daily_driver_snapshot -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
