# Session summary — Typed SDK byte/paste/mouse input helpers

## Goal

Implement bd-f26180 by adding typed `kittwm-sdk` helpers for the remaining input automation paths: exact byte input, bracketed paste byte payloads, and pane-local mouse events.

## Bead(s)

- `bd-f26180` — kittwm-sdk: typed exact bytes paste and mouse helpers

## Before state

- Failing tests: none known for this bead.
- Relevant metrics: the socket and CLI exposed `SEND_BYTES_B64`, `PASTE_BYTES_B64`, and `SEND_MOUSE`, but SDK clients had to use raw protocol strings for those paths; only text/line/key helpers were typed.
- Context: kittui-dev took scrollback/wait helper docs, so this slice stayed narrowly inside SDK typed input helpers and tests.

## After state

- Failing tests: none observed in targeted validation.
- Relevant metrics: `SurfaceHandle` now has `send_bytes`, `send_bytes_b64`, `paste_bytes`, `paste_bytes_b64`, and `send_mouse`. `MouseEvent` models daemon-supported event labels such as press/move/release/scroll variants. Byte helpers base64-encode internally via the workspace `base64` crate.
- Context: all helpers are gated by the existing `SendInput` capability and no daemon behavior changed.

## Diff summary

- Code/content commits: `4b1ca4f` (`bd-f26180: add typed SDK input helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`, `crates/kittwm-sdk/Cargo.toml`, `Cargo.lock`
- Tests: +3 targeted SDK input/mouse tests / -0 / flipped 0
- Behavioural delta: SDK clients can now send exact bytes, paste exact bytes, and inject typed mouse events without spelling raw socket verbs.
- Validation: `cargo test -p kittwm-sdk input -- --test-threads=1`; `cargo test -p kittwm-sdk mouse_event -- --test-threads=1`; `cargo check -p kittwm-sdk`; `git diff --check`.

## Operator-takeaway

The SDK input surface is now much closer to the CLI/socket automation surface: text, lines, keys, exact bytes, paste bytes, and mouse events are all available through typed helpers with capability checks.
