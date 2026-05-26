# Session summary — kittwm native surface coverage JSON

## Goal

Expose the SDK architecture contract's first-party native surface coverage matrix as a focused `kittwm` inspection command so operators and app authors can verify SDK-backed + kitty-graphics-native coverage without parsing the full architecture contract.

## Bead(s)

- `bd-7f657c` — kittwm: expose native surface coverage JSON

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept the slice to `kittwm` CLI inspection/tests only; no live session, bar implementation, Runtime/browser, or docs/help work.

## Before state

- `kittwm architecture-json` exposed the full typed architecture contract.
- There was no compact CLI artifact focused only on first-party native surface coverage/readiness.

## After state

- Added `kittwm native-surfaces-json` / `kittwm surface-coverage-json`.
- The artifact emits:
  - `schema_version`
  - `kind: kittwm-native-surface-coverage`
  - `all_ready`
  - `surfaces` copied from `ArchitectureContract::current().first_party_native_surfaces`
- Added command/help catalog entries.
- Added a test asserting terminal/browser/bar coverage and key SDK/kittui entries.

## Diff summary

- Code/content commits: `3117756` (`bd-7f657c: expose native surface coverage JSON`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm native_surfaces_json_reports_sdk_and_kitty_native_coverage -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittui-cli --bin kittwm`
  - `cargo run -q -p kittui-cli --bin kittwm -- native-surfaces-json` parsed and verified as JSON
  - `git diff --check`

## Operator-takeaway

`kittwm native-surfaces-json` is now a compact coverage probe for first-party SDK + kitty-graphics-native surface readiness.
