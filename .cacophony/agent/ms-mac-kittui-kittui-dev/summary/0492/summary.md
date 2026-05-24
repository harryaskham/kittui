# Session summary — nix sandbox kittui-cli lib test fixes

## Goal

Fix reported Nix sandbox failures in `kittui-cli` library tests.

## Bead(s)

- `bd-39442f` — fix nix kittui-cli sandbox test failures

## Before state

Reported Nix build failures:

- `daemon::tests::native_spawn_queue_reports_live_pane_status`
- `daemon::tests::native_spawn_queue_wait_text_does_not_block_ping`
- `session::native_pane_tests::native_pane_statuses_include_dirty_frame_metrics`

Symptoms included:

- test expecting `ERR WAIT_TEXT timeout` while command now correctly reports `ERR WAIT_TEXT_MS timeout`,
- dummy PTY test helper spawning bare `true`, which may not resolve in the Nix Darwin sandbox.

## After state

- `native_spawn_queue_wait_text_does_not_block_ping` now asserts the correct `WAIT_TEXT_MS` timeout prefix.
- `dummy_native_pane_app()` now spawns the current test executable with `--help` via an absolute path instead of relying on a bare `true` lookup.
- Runtime behavior is unchanged; only tests/helpers were adjusted.

## Validation

- `cargo test -p kittui-cli --lib native_spawn_queue_wait_text_does_not_block_ping -- --nocapture` passed.
- `cargo test -p kittui-cli --lib native_pane_statuses_include_dirty_frame_metrics -- --nocapture` passed.
- `cargo test -p kittui-cli --lib native_spawn_queue_reports_live_pane_status -- --nocapture` passed.
- `cargo test -p kittui-cli --lib -- --nocapture` passed: 86 passed, 4 ignored.
- `git diff --check` passed.

## Files touched

- `crates/kittui-cli/src/daemon.rs`
- `crates/kittui-cli/src/session.rs`
