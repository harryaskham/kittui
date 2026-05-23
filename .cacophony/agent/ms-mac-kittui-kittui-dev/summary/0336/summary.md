# Session summary — wait for output across screen and scrollback

## Goal

Make native kittwm automation waits robust for fast/scrolled output by adding wait commands that search both the visible screen and scrollback.

## Bead(s)

- `bd-af7b18` — kittwm: wait for text across screen and scrollback

## Before state

- Failing tests: none known.
- Relevant gap: after adding scrollback, `WAIT_TEXT` still searched only the current screen. Fast commands could print a sentinel and scroll it off before controllers waited, causing automation misses despite scrollback being available.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::client_read_timeout_tracks_wait_text_ms -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added socket commands `WAIT_OUTPUT <window|focused> <needle>` and `WAIT_OUTPUT_MS <window|focused> <ms> <needle>`. These search current text plus native scrollback and return `MATCH_OUTPUT`. Existing `WAIT_TEXT` remains screen-only and returns `MATCH_TEXT`. Client read timeout calculation now honors `WAIT_OUTPUT_MS`. Added CLI wrappers `--wait-output` and `--wait-output-ms`; README/docs/help updated.

## Diff summary

- Code/content commit: `488bbfc`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: automation can block on text that may appear on-screen or have already scrolled into native scrollback.

## Operator-takeaway

Use `kittwm --wait-output focused 'needle'` or `--wait-output-ms MS focused 'needle'` for robust command-output waits across screen and scrollback; keep `--wait-text` when only the active screen should count.
