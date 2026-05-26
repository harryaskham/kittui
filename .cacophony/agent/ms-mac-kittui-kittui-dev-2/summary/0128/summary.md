# Session summary — kittwm-browser native capability JSON

## Goal

Make the first-party browser surface's SDK/kittui/kitty-native contract discoverable without launching Chrome or requiring a live kittwm session.

## Bead(s)

- `bd-523fe6` — kittwm-browser: expose native capability JSON

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept the slice to `crates/kittui-cli/src/bin/kittwm_browser.rs` tests/CLI only.
- Avoided kittwm.rs shortcut scene work, SDK ArchitectureContract helpers, terminal/launch/bar/session internals.

## Before state

- Browser native coverage was visible through global kittwm architecture/native-surface artifacts.
- `kittwm-browser` itself had no lightweight capabilities probe describing its SDK entry and kittui/kitty rendering paths.

## After state

- Added `kittwm-browser --capabilities-json` / `--native-capabilities-json`.
- The JSON reports:
  - `kind: kittwm-browser-native-capabilities`
  - `surface_kind: browser`
  - `sdk_entry: SurfaceSpec::browser`
  - `sdk_backed: true`
  - `kitty_graphics_native: true`
  - kittui entries for PNG frame placement and semantic rendering
  - semantic output modes
  - render output modes
- Added parser/help and JSON-shape tests.

## Diff summary

- Code/content commits: `95097ac` (`bd-523fe6: expose browser native capabilities JSON`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm_browser.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm-browser parses_semantic_scene_and_kitty_modes -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm-browser browser_capabilities_json_reports_sdk_and_kittui_paths -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittui-cli --bin kittwm-browser`
  - `cargo run -q -p kittui-cli --bin kittwm-browser -- --capabilities-json` parsed and verified as JSON
  - `git diff --check`

## Operator-takeaway

`kittwm-browser --capabilities-json` is now a quick per-app probe for browser SDK/kittui-native readiness.
