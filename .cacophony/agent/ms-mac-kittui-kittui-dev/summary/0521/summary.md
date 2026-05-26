# Session summary — kittwm-terminal graphical events

## Bead

- `bd-d8c6cc` — make kittwm-terminal events kittui/kitty-native

## Changes

- Added bounded event output modes to `kittwm-terminal`:
  - `--events-scene-json MS`
  - `--events-kitty MS` / `--events-graphics MS`
- Existing `--events-ms MS` text behavior remains unchanged.
- Event modes use typed SDK event batches, summarize count/timeout/kinds, and render a labelled kittui scene.
- Kitty mode places the scene through `kittui::Runtime::place_at_with_options` at chrome z-index 20.
- Added parser/model/scene tests without requiring a live kittwm socket.

## Coordination

- Scope limited to `crates/kittui-cli/src/bin/kittwm_terminal.rs`.
- Avoided dev-2 areas: native-surfaces CLI/ArchitectureContract helpers, Runtime/browser, kittwm-bar, kittwm-launch, and live session internals.

## Validation

- `cargo test -p kittui-cli --bin kittwm-terminal -- --nocapture`
- `cargo check -p kittui-cli --bin kittwm-terminal`
- `git diff --check`
