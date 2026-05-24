# Session summary — SDK JSON wait helpers

## Goal

Expose the JSON wait-match daemon commands through typed `kittwm-sdk` `SurfaceHandle` helpers.

## Bead(s)

- `bd-6e3c54` — kittwm-sdk: JSON wait match helpers

## Before state

- Failing tests: none known.
- Relevant context: daemon/CLI JSON wait commands had landed, but SDK typed wait helpers still consumed the text `MATCH_*` commands only.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk scrollback_and_wait_helpers_send_expected_commands -- --nocapture` passed.
  - `cargo test -p kittwm-sdk scrollback_and_wait_helpers_deny_before_io -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `SurfaceHandle::wait_text_match_json_ms`.
  - Added `SurfaceHandle::wait_output_match_json_ms`.
  - Added `SurfaceHandle::wait_text_match_json`.
  - Added `SurfaceHandle::wait_output_match_json`.
  - Helpers return existing `WaitMatch` from JSON daemon replies and are `ReadText`-gated.
  - Existing text-reply wait helpers remain unchanged.
  - No daemon/CLI changes.

## Parallel coordination

- `kittui-dev-2` has docs-only follow-ups:
  - `bd-0a7e9f` JSON wait wrapper docs.
  - `bd-472268` SDK JSON wait helper docs after this lands.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittwm-sdk/src/lib.rs`

## Operator-takeaway

SDK automation can now use JSON wait-match commands directly while receiving the same typed `WaitMatch` metadata as the existing parsed text helpers.
