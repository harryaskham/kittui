# Session summary — typed SDK architecture contract

## Goal

Continue improving kittwm as a well-designed kitty-graphics-backed window manager by making the architecture/separation contract available through the SDK, not just the `kittwm` CLI artifact.

## Bead(s)

- `bd-fe5771` — kittwm-sdk: type the architecture contract artifact

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev` before editing.
- Kept the slice to SDK types/tests plus the CLI artifact reusing the SDK model; no live session reservation/control-plane internals.

## Before state

- `kittwm architecture-json` emitted an inline JSON object inside the CLI.
- Rust SDK users had no typed model for the same architecture/platform boundary contract.
- The CLI artifact and any future SDK model could drift.

## After state

- Added typed SDK models:
  - `ArchitectureContract`
  - `ArchitectureLayer`
  - `CompositionPlane`
  - `NativeSurfaceContract`
- Added `ArchitectureContract::current()` and `ArchitectureContract::layer(id)`.
- `kittwm architecture-json` now serializes `kittwm_sdk::ArchitectureContract::current()`.
- Updated docs to point Rust apps at `kittwm_sdk::ArchitectureContract::current()`.
- Added SDK roundtrip/boundary tests and retained CLI artifact tests.

## Diff summary

- Code/content commits: `b035e35` (`bd-fe5771: type kittwm architecture contract in SDK`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
  - `crates/kittui-cli/src/bin/kittwm.rs`
  - `docs/wm.md`
- Validation:
  - `CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `CARGO_BUILD_JOBS=1 cargo test -p kittui-cli --bin kittwm architecture_contract_names_clean_wm_boundaries -- --test-threads=1`
  - `CARGO_BUILD_JOBS=1 cargo check -p kittui-cli --bin kittwm`
  - `git diff --check`
  - `cargo run -q -p kittui-cli --bin kittwm -- architecture-json` parsed as JSON and verified kind/layer count

## Operator-takeaway

The kittwm architecture contract is now an SDK-level typed artifact, so app authors can consume the same responsibility boundaries that the CLI publishes.
