# Session summary — graphical kittwm architecture contract

## Bead

- `bd-63394e` — make kittwm architecture contract kittui/kitty-native

## Changes

- Added `kittwm architecture-scene-json` / `platform-contract-scene-json`.
- Added `kittwm architecture-kitty` / `architecture-graphics` and `platform-contract-kitty` / `platform-contract-graphics`.
- Existing `architecture-json` / `platform-contract-json` behavior remains unchanged.
- Scene is built from `kittwm_sdk::ArchitectureContract::current()`.
- Labels contract schema/kind, layer count, composition-plane count, native surface count, per-layer ownership/boundary counts, per-plane z-indexes, and per-surface SDK/kitty/kittui entries.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Updated help/known command catalog/local command catalog with the new graphical architecture surfaces.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm.rs` architecture inspection.
- Avoided dev-2 active SDK placement/ArchitectureContract/NativeSurfaceContract helper work, browser/capabilities, Runtime internals, helper binaries, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm architecture_scene_labels_layers_planes_and_surfaces -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm architecture_contract_names_clean_wm_boundaries -- --nocapture`
- `cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm`
- `git diff --check`
