# Session summary — typed SDK cached clipboard helper

## Goal

Add a typed kittwm-sdk helper for the runtime `CLIPBOARD_JSON` policy surface so SDK clients do not need raw protocol strings.

## Bead(s)

- `bd-f65815` — kittwm-sdk: typed cached clipboard helper

## Before state

- Failing tests: none known.
- Relevant context: daemon `CLIPBOARD_JSON`, docs, and CLI wrapper existed; SDK clients still needed raw protocol commands.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk clipboard -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `ClipboardStatus` typed response shape.
  - Added `ClipboardStatus::has_payload()` convenience helper.
  - Added `Kittwm::clipboard()` and `Kittwm::clipboard_json()`.
  - Helper is gated by existing `Capability::Clipboard` and denies before socket I/O when unavailable.
  - Denied, allowed-empty, and allowed-cached daemon replies parse cleanly.
  - Unix socket test verifies the helper sends `CLIPBOARD_JSON`.
  - No daemon or CLI behavior changed.

## Parallel coordination

- `kittui-dev-2` remains assigned to `bd-d582b7` for typed SDK `PaneFramePresented` event parsing/docs.

## Diff summary

- Code/content commit: `5702310f`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`

## Operator-takeaway

The clipboard policy surface now has runtime, docs, CLI, and SDK coverage, while still preserving default-deny/cache-only semantics.
