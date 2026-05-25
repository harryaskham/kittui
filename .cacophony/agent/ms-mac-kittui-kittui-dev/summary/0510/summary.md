# Session summary — stale socket cleanup on bind

## Goal

Improve kittwm crash recovery so a stale socket path left by a forced kill or crash is removed before the next daemon/native queue bind, while still refusing to steal a live socket.

## Bead(s)

- `bd-cc03ea` — kittwm: crash recovery and daemon/socket stale state cleanup

## Changes

- Added shared `cleanup_stale_socket_for_bind(path, owner)` in `crates/kittui-cli/src/daemon.rs`.
- `DaemonServer::bind` now uses the helper.
- `NativeSpawnQueue::bind` now uses the same helper instead of blindly unlinking existing paths.
- Existing live-daemon double-bind behavior is preserved: a socket that replies `PONG` fails with “already listening”.
- Added tests for stale regular-file cleanup for both standalone daemon and native spawn queue bind paths.

## Validation

- `cargo test -p kittui-cli --lib stale_socket_file -- --nocapture` passed.
- `cargo build -p kittui-cli --bin kittwm` passed.
- `git diff --check` passed.
