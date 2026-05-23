# Session summary — SDK typed native pane status details

## Goal

Add typed SDK structures for rich native pane details so SDK clients can consume `PANES_JSON` / `STATUS_JSON` without hand-inspecting JSON values.

## Bead(s)

- `bd-18e1b5` — kittwm-sdk: add typed native pane status details

## Before state

- Failing tests: none known.
- Relevant context: daemon status surfaces now expose cursor/mouse modes, geometry, dirty-frame metrics, and future transport diagnostics, but `kittwm-sdk` only had a minimal `Status` model.

## After state

- Failing tests: none in validation.
- Validation:
  - `cargo test -p kittwm-sdk -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `DirtyFrameStatus`, `NativePaneDetail`, and `PanesStatus`.
  - Added optional rich fields for pid/command, geometry, app geometry, cursor state, bracketed paste, application cursor mode, mouse modes, dirty-frame metrics, and future transport diagnostics.
  - Extended `Status` with optional `focused_pane` and `panes_detail` while keeping missing fields backward-compatible.
  - Added `Kittwm::panes()` wrapper for `PANES_JSON`.
  - Added tests for rich pane detail JSON and minimal status JSON.
  - Coordinated with kittui-dev-2: they are implementing `bd-4edcb2` POSIX shm raw-frame transport.

## Diff summary

- Code/content commit: `0b06580a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Behavioural delta: SDK API/types only; no daemon/runtime behavior change.

## Operator-takeaway

SDK clients now have typed access to rich native pane metadata, including dirty-frame metrics and a forward-compatible transport diagnostics slot.
