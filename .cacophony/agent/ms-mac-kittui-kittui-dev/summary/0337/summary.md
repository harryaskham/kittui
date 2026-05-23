# Session summary — bracketed-paste aware native paste

## Goal

Add a native paste primitive for kittwm automation that respects terminal bracketed-paste mode, reducing accidental line execution and improving shell/editor semantics.

## Bead(s)

- `bd-01bf37` — kittwm: add bracketed-paste aware native paste

## Before state

- Failing tests: none known.
- Relevant gap: kittwm supported exact byte injection and file sending, but not a paste operation that honored DEC private `?2004` bracketed-paste mode. Modern shells/editors enable bracketed paste, and automation should wrap paste payloads with `ESC[200~` / `ESC[201~` when enabled.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_tracks_bracketed_paste_mode -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_parses_focus_close_layout_and_rename_commands -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo test -p kittui-cli session::native_pane_tests::native_paste_payload_wraps_when_bracketed -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Native PTY state now tracks `CSI ? 2004 h/l` bracketed-paste mode. Native pane status publishes optional `bracketed_paste`. Added socket `PASTE_BYTES_B64 <window|focused> <base64>` and CLI `kittwm --paste-file WINDOW PATH|-`. The session loop wraps paste bytes with bracketed-paste markers only when the target pane has mode enabled; otherwise it sends raw bytes. README/docs/help were updated.

## Diff summary

- Code/content commit: `ab01c58`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: controllers can paste payloads safely into shells/editors with bracketed-paste-aware behavior.

## Operator-takeaway

Use `kittwm --paste-file focused payload.txt` for multiline paste automation; it automatically wraps payloads when the pane has bracketed paste enabled.
