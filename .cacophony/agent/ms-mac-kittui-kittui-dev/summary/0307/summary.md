# Session summary — kittwm session save/restore CLI

## Goal

Make native kittwm session persistence usable from shell scripts without manually quoting JSON through `kittwm --attach -c`.

## Bead(s)

- `bd-caa205` — kittwm: add save and restore session CLI flags

## Before state

- Failing tests: none known.
- Relevant gap: `SESSION_JSON` and `RESTORE_SESSION_JSON` existed as socket commands, but users/controllers had to manually quote JSON in shell commands to restore sessions.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittwm normalize_daemon_command_uppercases_only_verb -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittwm restore_session_request -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: Added first-class flags:
  - `kittwm --save-session PATH|-` reads `SESSION_JSON` from the running socket and writes pretty JSON to a file or stdout.
  - `kittwm --restore-session PATH|-` reads JSON from a file or stdin, validates/compacts it, and sends `RESTORE_SESSION_JSON <json>` to the running socket.
  Help text, README, and docs/wm were updated. Added focused tests for restore request generation/rejection.

## Diff summary

- Code/content commit: `cf8f677`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`, `README.md`, `docs/wm.md`
- Behavioural delta: kittwm session save/restore is scriptable without fragile shell JSON quoting.

## Operator-takeaway

Users can now do `kittwm --save-session session.json` and `kittwm --restore-session session.json` against a running native kittwm socket.
