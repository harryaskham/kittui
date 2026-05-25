# Session summary — Triple Ctrl-C exit guard

## Goal

Complete bd-8c5078 by restoring Ctrl-C forwarding to focused apps while adding a guarded kittwm exit path only after three Ctrl-C presses in a short window.

## Bead(s)

- `bd-8c5078` — kittwm: restore Ctrl-C handling with triple-Ctrl-C exit guard

## Before state

- Failing tests: none known for this slice.
- Relevant metrics: byte input processing forwarded ordinary bytes to focused panes and exited on Ctrl-], but there was no explicit Ctrl-C exit guard. Ctrl-C behavior was ambiguous for nested apps needing one or two interrupts.
- Context: scoped to native session input loop; no command catalog or graphical chrome changes.

## After state

- Failing tests: none observed; targeted validation passed.
- Relevant metrics: added `NativeCtrlCExitGuard` with a 3-press threshold and 2-second window. Ctrl-C (`0x03`) is forwarded to the focused pane first, then the guard is updated; the third Ctrl-C within the window exits kittwm. Non-Ctrl-C bytes and prefix entry reset the guard. Ctrl-] remains immediate exit.
- Context: changed only `crates/kittui-cli/src/session.rs`.

## Diff summary

- Code/content commits: `0c43a06` (`bd-8c5078: add triple ctrl-c exit guard`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`
- Tests: added `native_ctrl_c_exit_guard_requires_three_presses_in_window` covering third-press exit, timeout reset, and manual reset.
- Behavioural delta: single/double Ctrl-C remain app-delivered; triple Ctrl-C within 2s exits kittwm.
- Validation: `cargo test -p kittui-cli native_ctrl_c_exit_guard_requires_three_presses_in_window -- --test-threads=1`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

Nested terminal apps can receive one or two Ctrl-C interrupts normally; kittwm only exits after the third rapid Ctrl-C.
