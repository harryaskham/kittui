# Session summary — kittwm-launch graphical plan

## Bead

- `bd-f22c20` — make kittwm-launch plan kittui/kitty-native

## Changes

- Added `kittwm-launch --plan-scene-json`.
- Added `kittwm-launch --plan-kitty` / `--plan-graphics`.
- Plan modes imply dry-run and require no live kittwm socket.
- Launch plan scene labels expose backend/status/command for inspection artifacts.
- Kitty mode places the plan scene via `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Existing `--dry-run` / `--status` text behavior remains unchanged.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm_launch.rs`.
- Avoided dev-2 areas: native-surfaces CLI/ArchitectureContract helpers, Runtime/browser, kittwm-bar, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm-launch -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm-launch`
- `git diff --check`
