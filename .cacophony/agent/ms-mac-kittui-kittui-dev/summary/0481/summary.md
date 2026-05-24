# Session summary — kittwm JSON wait wrappers

## Goal

Add JSON-returning wait-match surfaces alongside the existing text `MATCH_*` wait replies.

## Bead(s)

- `bd-3b3595` — kittwm: JSON wait match socket and CLI wrappers

## Before state

- Failing tests: none known.
- Relevant context: `WAIT_TEXT*` / `WAIT_OUTPUT*` returned text `MATCH_*` replies, while surrounding read surfaces had JSON forms.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added socket commands:
    - `WAIT_TEXT_JSON <window|focused> <needle>`
    - `WAIT_TEXT_JSON_MS <window|focused> <ms> <needle>`
    - `WAIT_OUTPUT_JSON <window|focused> <needle>`
    - `WAIT_OUTPUT_JSON_MS <window|focused> <ms> <needle>`
  - JSON success shape includes `kind`, `match`, `window`, and `bytes`.
  - Existing text `WAIT_TEXT*` / `WAIT_OUTPUT*` behavior is unchanged.
  - Added CLI wrappers:
    - `--wait-text-json`
    - `--wait-text-json-ms`
    - `--wait-output-json`
    - `--wait-output-json-ms`
  - Added HELP/HELP_JSON/error help entries.
  - No SDK changes in this bead.

## Parallel coordination

- `kittui-dev-2` claimed `bd-0a7e9f` docs-only follow-up and is waiting for this source bead to land.

## Diff summary

- Code/content commit: pending branch commit
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/daemon.rs`
  - `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

Automation can now use JSON wait match replies without parsing `MATCH_TEXT` / `MATCH_OUTPUT`, while old text replies remain stable.
