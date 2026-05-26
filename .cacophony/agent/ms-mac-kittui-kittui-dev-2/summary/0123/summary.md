# Session summary — SDK native surface coverage lookup helpers

## Goal

Continue improving kittwm's SDK/kittui-native platform contract by making first-party native surface coverage easy for app authors and tests to query.

## Bead(s)

- `bd-1146b7` — kittwm-sdk: add native surface coverage lookup helpers

## Coordination

- Checked in via `caco msg speak`.
- Coordinated directly with `ms-mac:kittui:ms-mac-kittui-kittui-dev`.
- Kept the slice SDK-only; no kittwm-bar implementation/docs, Runtime/browser, or live session reservation/control-plane changes.

## Before state

- The architecture contract carried first-party native surface coverage fields, but callers had to manually scan `first_party_native_surfaces`.
- There was no shared readiness predicate for “SDK-backed + kitty-graphics-native + kittui entry present”.

## After state

- Added `NativeSurfaceContract::is_native_ready()`.
- Added `ArchitectureContract::native_surface(name)`.
- Added `ArchitectureContract::native_ready_surfaces()`.
- Strengthened the existing SDK architecture test to use the helpers and assert the ready surface list is:
  - `kittwm-terminal`
  - `kittwm-browser`
  - `kittwm-bar`

## Diff summary

- Code/content commits: `bdc7967` (`bd-1146b7: add SDK native surface coverage helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`
- Validation:
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo test -p kittwm-sdk architecture_contract_exposes_wm_boundaries_for_apps -- --test-threads=1`
  - `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 cargo check -p kittwm-sdk`
  - `git diff --check`

## Operator-takeaway

SDK clients can now query the architecture contract as a native surface coverage matrix instead of manually inspecting JSON-like fields.
