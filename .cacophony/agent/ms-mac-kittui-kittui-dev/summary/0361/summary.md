# Session summary — OSC 52 clipboard forwarding

## Goal

Let nested native kittwm terminal apps set the host clipboard through OSC 52 even though kittwm consumes and renders the inner PTY stream itself.

## Bead(s)

- `bd-402bf0` — kittwm: forward OSC 52 clipboard writes from native panes

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm intercepted OSC sequences for its own terminal model. OSC 52 clipboard writes from inner apps did not reach Ghostty/the host terminal clipboard.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_forwards_osc52_clipboard_writes -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_ignores_osc52_queries_and_invalid_payloads -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_preserves_osc_title_across_resize -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now handles OSC through a dispatcher, preserving OSC 0/1/2 titles and recognizing OSC 52 clipboard writes. Valid base64 non-query OSC 52 payloads are queued as sanitized host-terminal sequences. Queries/empty/invalid payloads or suspicious selectors are ignored. `PtyTerminalApp::take_host_sequences()` exposes drained host sequences, and the native session loop writes them to stdout before frame rendering.

## Diff summary

- Code/content commit: `68b1e6a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs`, `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: nested terminal apps can set the host clipboard via OSC 52 while kittwm keeps ownership of pane rendering/state.

## Operator-takeaway

kittwm now mediates a first host-terminal side effect from inner PTY apps: sanitized clipboard writes pass through to Ghostty/host terminal instead of being swallowed.
