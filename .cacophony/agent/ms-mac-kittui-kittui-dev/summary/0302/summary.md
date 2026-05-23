# Session summary — native socket key injection

## Goal

Let external controllers send non-text terminal keys into native kittwm panes through the socket control plane.

## Bead(s)

- `bd-b9b864` — kittwm: add native socket key injection

## Before state

- Failing tests: none known.
- Relevant gap: native socket text injection existed, but controllers could not send control/navigation keys such as Ctrl-C, Escape, arrows, or PageUp/PageDown without host TTY synthesis.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_parses_focus_close_layout_and_rename_commands -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added native socket command `SEND_KEY <window|focused> <key>`.
  Supported key names include enter/return, tab, escape/esc, backspace, delete, left/right/up/down, home/end, pageup/pagedown, and `ctrl-a` through `ctrl-z` / `C-a` through `C-z`. The daemon validates key names, queues `NativePaneCommand::SendBytes`, and the native PTY loop resolves `focused` or a window token before sending bytes to the pane. HELP/HELP_JSON and README/docs now mention `SEND_KEY`.

## Diff summary

- Code/content commit: `14063d8`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/session.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: external socket clients can drive interactive terminal control/navigation keys directly.

## Operator-takeaway

Native kittwm controllers can now spawn, inspect, focus, resize, move, inject text, and inject named control keys entirely over the socket.
