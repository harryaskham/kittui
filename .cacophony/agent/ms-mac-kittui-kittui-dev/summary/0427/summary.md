# Session summary — SDK capability profile presets

## Goal

Improve local SDK capability-scope ergonomics with additive preset builders.

## Bead(s)

- `bd-3faaa5` — kittwm-sdk: capability profile presets

## Before state

- Failing tests: none known.
- Relevant context: `ClientCapabilities::restricted()` only allowed `ReadText`; richer low-risk inspection and automation profiles required hand-assembling capability lists.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk capability -- --nocapture` passed before and after rebase.
  - `git diff --check` passed.
- Context:
  - Added `ClientCapabilities::none()`.
  - Added `ClientCapabilities::inspect_only()` for read/status/help/app catalog/session read/events/semantic reads (`ReadText`, `SubscribeEvents`, `ReadSemanticTree`).
  - Added `ClientCapabilities::automation()` for existing-surface automation (`ControlWindow`, `SendInput`, `ReadText`, `SubscribeEvents`, `ReadSemanticTree`) without create/replace/raw/semantic mutation.
  - Kept `restricted()` compatible as `ReadText` only and documented `inspect_only()` as preferred for new inspection clients.
  - Added `allowed()` and `iter()` accessors for introspection/tests.
  - No daemon/runtime enforcement changed.
  - Rebased after `kittui-dev-2` landed `bd-8b1d5b` at `9c587db`.

## Parallel coordination

- `kittui-dev-2` completed `bd-8b1d5b`: additive `NativePaneDetail` convenience accessors.

## Diff summary

- Code/content commit after rebase: `b3fa81e9`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Behavioural delta: SDK clients can choose clearer local capability profiles.

## Operator-takeaway

SDK capability scoping is easier to use while preserving prior restricted behavior and not changing daemon enforcement.
