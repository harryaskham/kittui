# Session summary — kittwm architecture separation contract

## Goal

Take an architecture/design pass over kittwm toward a usable kitty-graphics-backed terminal window manager with clear separation between SDK/control plane, tiling engine, surface renderer, decoration renderer, and kitty compositor.

## Bead(s)

- `bd-bdc963` — kittwm: publish architecture separation contract

## Coordination

- Announced progress via `caco msg speak` as requested.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept this slice to docs + inspection artifacts/tests, avoiding active session reservation/control-plane internals.

## Before state

- `docs/wm.md` had historical architecture notes, but no concise machine-readable contract for current kittwm boundary responsibilities.
- There was no `kittwm` inspection command that named the intended separation between SDK surface vocabulary, tiling/layout policy, surface rendering, decoration rendering, and kitty transport.

## After state

- Added `kittwm architecture-json` / `kittwm platform-contract-json`.
- The JSON contract names:
  - `sdk-control-plane`
  - `tiling-engine`
  - `surface-renderer`
  - `decoration-renderer`
  - `kitty-compositor`
- The contract records responsibilities, must-not boundaries, key native contracts, composition z-order, first-party native surfaces, and inspection artifacts.
- Added the command to help text and `commands-json`.
- Updated `docs/wm.md` with the architecture separation checklist and composition order.
- Added deterministic tests for the architecture JSON contract and command catalog entry.

## Diff summary

- Code/content commits: `7e4f03c` (`bd-bdc963: publish kittwm architecture contract`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`
  - `docs/wm.md`
- Validation:
  - `CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm architecture_contract_names_clean_wm_boundaries -- --test-threads=1`
  - `CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm commands_catalog_lists_daily_driver_aliases -- --test-threads=1`
  - `CARGO_BUILD_JOBS=1 cargo check -p kittui-cli --bin kittwm`
  - `git diff --check`
  - `cargo run -q -p kittui-cli --bin kittwm -- architecture-json` parsed as JSON and verified layer count/kind

## Operator-takeaway

`kittwm architecture-json` is now a stable inspection artifact that future implementation work can use to keep tiling, rendering, decorations, SDK, and kitty transport responsibilities cleanly separated.
