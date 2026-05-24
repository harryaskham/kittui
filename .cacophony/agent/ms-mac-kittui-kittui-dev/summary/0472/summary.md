# Session summary — chrome reservation status JSON

## Goal

Expose native kittwm chrome/top-bar reservation metadata through status JSON so SDK/CLI clients can reason about top-clamped chrome vs tilable pane area.

## Bead(s)

- `bd-4a56aa` — kittwm: expose chrome reservation in status JSON

## Before state

- Failing tests: none known.
- Relevant context: live session reserves a top-bar band and kittwm-bar can render status, but STATUS_JSON/PANES_JSON did not expose chrome reservation metadata.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `workspace: "1"` to native `STATUS_JSON` and `PANES_JSON`.
  - Added `chrome` object to both replies:
    - `workspace`
    - `top_bar_rows`
    - `tilable_rows` when inferable from pane geometry.
  - Existing status/panes fields are preserved.
  - No live rendering behavior changed.

## Parallel coordination

- `kittui-dev-2` claimed `bd-f4c2fb` for typed SDK chrome reservation status and is waiting for this source bead to land.

## Diff summary

- Code/content commit: `e8be5833`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`

## Operator-takeaway

SDK clients now have native JSON metadata to discover the top-bar reservation and tilable area instead of inferring it from pane geometry.
