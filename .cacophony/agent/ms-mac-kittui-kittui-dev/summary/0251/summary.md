# Session summary — native socket app discovery

## Goal

Make the live native kittwm session socket expose app discovery commands, aligning it with the standalone daemon and making DISPLAY-like attach workflows more useful from inside/outside native sessions.

## Bead(s)

- `bd-27aac2` — kittwm: add app discovery commands to native socket

## Before state

- Failing tests: none known.
- Relevant gap: native session socket supported spawn/focus/close/layout/status, but not `APPS`, `APPS_JSON`, `APPS_FIRST`, or `APPS_LAUNCH_FIRST` discovery commands available on the standalone daemon.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli native_spawn_queue_serves_app_discovery_commands -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `NativeSpawnQueue` now handles `APPS`, `APPS_JSON`, `APPS_FIRST <query>`, and `APPS_LAUNCH_FIRST <query>` by reusing the existing daemon app discovery helpers. `docs/wm.md` now lists APPS_JSON/APPS_FIRST attach examples.

## Diff summary

- Code/content commit: `103ded0`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`, `docs/wm.md`
- Behavioural delta: native kittwm session sockets can discover launchable commands before scripts enqueue `SPAWN_PTY`.

## Operator-takeaway

The native socket is now closer to parity with the standalone daemon for app discovery, improving scriptable terminal-WM workflows.
