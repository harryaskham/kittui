# Session summary — graphical kittwm events

## Bead

- `bd-385b65` — make kittwm events kittui/kitty-native

## Changes

- Added `kittwm events-scene-json [MS]`.
- Added `kittwm events-kitty [MS]` / `kittwm events-graphics [MS]`.
- Added flag forms `--events-scene-json MS` and `--events-kitty/--events-graphics MS`.
- Existing `events`, `--events`, and `--events-ms` text behavior remains unchanged.
- Event scene is built from bounded `EVENTS` JSONL batches and labels count, timeout, summary kinds, and per-event rows.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` event inspection.
- Avoided dev-2 areas: kittwm_browser.rs capabilities, SDK ArchitectureContract helpers, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm events_scene_labels_bounded_event_kinds -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
