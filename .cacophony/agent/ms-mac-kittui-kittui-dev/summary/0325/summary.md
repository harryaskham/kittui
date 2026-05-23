# Session summary — native base64 byte injection

## Goal

Add an encoded arbitrary-byte injection path to native kittwm's line-oriented socket control plane.

## Bead(s)

- `bd-b40d42` — kittwm: add base64 byte injection command

## Before state

- Failing tests: none known.
- Relevant gap: `SEND_TEXT` / `SEND_LINE` could not safely carry embedded newlines, NUL bytes, escape sequences, or arbitrary paste payloads because the socket protocol is single-line. Scripts had to work around framing.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_parses_focus_close_layout_and_rename_commands -- --nocapture` passed.
  - `cargo test -p kittui-cli daemon::tests::native_spawn_queue_serves_help_catalogs -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm automation_request -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added socket command `SEND_BYTES_B64 <window|focused> <base64>`. It decodes standard base64 and queues `NativePaneCommand::SendBytes`, reusing the existing native loop byte-send path. Invalid base64 and malformed arguments return clear errors. Added CLI wrapper `kittwm --send-bytes-b64 <window|focused> <base64>`, HELP/HELP_JSON coverage, README, and docs/wm updates.

## Diff summary

- Code/content commit: `c0908bf`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: automation clients can inject exact bytes, including multiline and NUL-containing payloads, through the native socket.

## Operator-takeaway

Use `kittwm --send-bytes-b64 focused aGkKAA==` for exact byte injection when line-oriented text commands are not enough.
