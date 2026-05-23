# Session summary — cursor metadata in READ_TEXT replies

## Goal

Include native pane cursor position in automation-focused text snapshot replies so controllers do not need a second status request to pair text with caret location.

## Bead(s)

- `bd-772614` — kittwm: include cursor in read-text replies

## Before state

- Failing tests: none known.
- Relevant gap: `PANES_JSON` / `STATUS_JSON` exposed `cursor_col` / `cursor_row`, but `READ_TEXT` and `READ_TEXT_JSON` returned only text. Automation clients reading pane text needed a separate status call to know prompt/caret position.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_read_text_round_trip_over_socket -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `READ_TEXT` now includes `cursor=col,row` in its header when known, while preserving the existing text body and `END` framing. `READ_TEXT_JSON` now includes `cursor_col` and `cursor_row`. docs/wm now describes text plus cursor metadata.

## Diff summary

- Code/content commit: `7cdf366`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `docs/wm.md`
- Behavioural delta: text snapshot APIs now carry cursor metadata directly.

## Operator-takeaway

Use `kittwm --read-text focused` / `READ_TEXT_JSON focused` to get both pane text and cursor position in one request.
