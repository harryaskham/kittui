# Session summary — TypeScript runtime config and placeAt

## Goal

Bring the TypeScript koffi binding up to date with the newer FFI platform surface so JS/TS hosts can configure runtime/terminal capabilities and place scene-local renders at host-supplied coordinates.

## Bead(s)

- `bd-d02826` — bindings-ts: expose runtime config and placeAt

## Before state

- Failing tests: none known.
- Relevant gap: `bindings/ts` only used `kittui_runtime_new(cache_dir)` and `kittui_place_json`. It could not call `kittui_runtime_new_config` or `kittui_place_json_at`.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `npm ci --prefix bindings/ts` completed successfully to install `koffi` locally for tests.
  - `npm test --prefix bindings/ts` passed.
  - `git diff --check` passed.
- Context: `Kittui.open({ ... })` now uses `kittui_runtime_new_config` when runtime/terminal options are present (`renderer`, `transport`, `columns`, `rows`, `cellWidthPx`, `cellHeightPx`, `supportsKitty`, `supportsUnicodePlaceholders`). `Kittui.placeAt(scene, x, y)` calls `kittui_place_json_at`. Type declarations and README were updated. Tests include fake-library coverage for constructor selection and placeAt forwarding without requiring a cdylib.

## Diff summary

- Code/content commit: `7747e7a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `bindings/ts/src/index.js`, `bindings/ts/src/index.d.ts`, `bindings/ts/test/koffi.test.js`, `bindings/ts/README.md`
- Behavioural delta: TS hosts can now configure kittui runtimes and place scenes at explicit terminal positions without mutating scene JSON.

## Operator-takeaway

The TS binding now tracks the cross-platform FFI improvements, closing a non-Rust host gap for runtime config and placement override workflows.
