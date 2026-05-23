# Session summary — TypeScript probe/unplace lifecycle APIs

## Goal

Expose more of the kittui FFI renderer lifecycle to TypeScript hosts so JS/TS integrations can inspect runtime metadata and clean up uploaded image ids.

## Bead(s)

- `bd-a8c9f8` — bindings-ts: expose probe and unplace lifecycle APIs

## Before state

- Failing tests: none known.
- Relevant gap: `bindings/ts` exposed placement APIs but not `kittui_probe_json` or `kittui_unplace`, leaving JS/TS hosts without runtime introspection or cleanup controls.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `npm test --prefix bindings/ts` passed.
  - `git diff --check` passed.
- Context: `Kittui.probe()` now returns parsed FFI JSON metadata. `Kittui.unplace(imageId)` accepts numbers or decimal/hex strings and returns delete bytes from `kittui_unplace`. Type declarations and README were updated, with fake-library tests for both APIs.

## Diff summary

- Code/content commit: `6a08336`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `bindings/ts/src/index.js`, `bindings/ts/src/index.d.ts`, `bindings/ts/test/koffi.test.js`, `bindings/ts/README.md`
- Behavioural delta: TypeScript hosts now cover render/place/move/probe/delete lifecycle APIs.

## Operator-takeaway

The TS binding is now much closer to a complete external-platform host for kittui: it can configure, render, move, inspect, and clean up terminal graphics resources.
