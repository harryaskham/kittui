# Session summary — native pane mouse event injection

## Goal

Add a stable native kittwm socket/CLI primitive for sending mouse events to mouse-aware PTY panes, building on the newly published mouse-reporting mode state.

## Bead(s)

- `bd-4db547` — kittwm: add native pane mouse event injection

## Before state

- Failing tests: none known.
- Relevant gap: native panes published which mouse modes they requested, but controllers still had to craft raw terminal bytes to automate TUIs that use mouse input.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_parses_focus_close_layout_and_rename_commands -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo test -p kittui-cli session::native_pane_tests::native_mouse_event_payload_requires_compatible_modes -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added socket command `SEND_MOUSE <window|focused> <event> <col> <row>` and CLI wrapper `kittwm --send-mouse WINDOW EVENT COL ROW`. Supported events are `press-left`, `press-middle`, `press-right`, `release`, `move`, `scroll-up`, and `scroll-down`. The session loop emits SGR mouse reports only when the target pane has compatible `mouse_sgr` and base mouse mode state; otherwise it logs/ignores the request rather than injecting invalid bytes. README/docs/help were updated.

## Diff summary

- Code/content commit: `2bb3604`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/session.rs`, `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: external controllers can send mouse clicks/scrolls/moves to native panes through stable wrappers instead of raw escape construction.

## Operator-takeaway

Use `kittwm --send-mouse focused press-left 7 9` or the socket `SEND_MOUSE` command for mouse-aware TUI automation; it only injects when the pane has requested compatible modes.
