# Session summary — kittwm text JSON read CLI wrappers

## Goal

Expose existing JSON pane text read socket verbs through stable `kittwm` CLI wrappers.

## Bead(s)

- `bd-2a18a3` — kittwm: CLI wrappers for text JSON reads

## Before state

- Failing tests: none known.
- Relevant context: daemon and SDK exposed `READ_TEXT_JSON` / `READ_SCROLLBACK_JSON`, but CLI wrappers only covered text output variants.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `kittwm --read-text-json WINDOW` mapping to `READ_TEXT_JSON <window>`.
  - Added `kittwm --read-scrollback-json WINDOW` mapping to `READ_SCROLLBACK_JSON <window>`.
  - Added CLI help text.
  - Existing text wrappers are unchanged.
  - No daemon/session/SDK changes.

## Parallel coordination

- Assigned `bd-5c73cb` to `kittui-dev-2` as docs-only follow-up after this source bead lands.
- `kittui-dev-2` also has `bd-74cccd` docs-only for `--help-json`.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Users can now inspect pane text and scrollback JSON with stable CLI wrappers instead of raw `--attach -c READ_*_JSON ...` protocol strings.
