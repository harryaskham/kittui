# Session summary — exact bytes and paste wrapping coverage

## Goal

Add targeted verification that kittwm byte-input paths preserve exact bytes and that bracketed paste wrapping occurs only when enabled.

## Bead(s)

- `bd-683b78` — kittwm: verify bracketed paste and exact bytes through aliases and SDK

## Changes

- `crates/kittui-cli/src/session.rs`
  - Added coverage that `native_paste_payload` preserves NUL/ESC/non-UTF8 bytes unchanged when bracketed paste is disabled.
  - Added coverage that bracketed mode wraps exactly with `ESC [ 200 ~` / `ESC [ 201 ~` and preserves inner bytes.

- `crates/kittui-cli/src/daemon.rs`
  - Added coverage that `SEND_BYTES_B64` and `PASTE_BYTES_B64` decode and queue exact bytes including NUL, 0xff, and ESC sequences.

- `crates/kittui-cli/src/bin/kittwm.rs`
  - Extended alias/request tests to cover exact non-UTF8 byte payloads for send and paste request construction.

- `crates/kittwm-sdk/src/lib.rs`
  - Extended SDK socket helper test to cover `paste_bytes` with NUL/0xff/ESC payload.
  - Updated mock server expected request count accordingly.

## Validation

- `cargo test -p kittui-cli --lib native_spawn_queue_preserves_exact_decoded_bytes_for_send_and_paste -- --nocapture` passed.
- `cargo test -p kittui-cli --lib native_paste_payload_preserves_exact_bytes_and_wraps_only_when_enabled -- --nocapture` passed.
- `cargo test -p kittui-cli --bin kittwm automation_request_preserves_payload_case_and_spaces -- --nocapture` passed.
- `cargo test -p kittwm-sdk input_helpers_send_expected_socket_commands -- --nocapture` passed.
- `git diff --check` passed.
