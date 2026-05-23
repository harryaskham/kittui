# Session summary — SDK client capability scopes

## Goal

Continue the SDK/surface plan by adding a first local capability scoping model to `kittwm-sdk` clients.

## Bead(s)

- `bd-08dcc2` — kittwm: add capability scoping for SDK clients

## Before state

- Failing tests: none known.
- Relevant gap: SDK clients had typed handles but no operation-level capability model. Any code with a `Kittwm` client could call raw requests, spawn/replace windows, control panes, send input, and read text.

## After state

- Failing tests: none in targeted checks.
- Validation:
  - `cargo test -p kittwm-sdk -- --nocapture` passed.
  - `cargo build -p kittwm-sdk` passed.
  - `cargo build -p kittui-cli --bin kittwm-launch` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - Added `Capability` enum: `RawRequest`, `CreateWindow`, `ReplaceWindow`, `ControlWindow`, `SendInput`, `ReadText`, `Clipboard`, `SubscribeEvents`.
  - Added `ClientCapabilities` with `all`, `restricted`, `only`, `allows`, and enforcement helpers.
  - `Kittwm` now carries a capability scope and exposes `with_capabilities` / `capabilities`.
  - Public raw `request` requires `RawRequest`.
  - `spawn_surface` requires `CreateWindow`.
  - `replace_current` requires `ReplaceWindow` plus the create path.
  - `SurfaceHandle` control methods require `ControlWindow`; input methods require `SendInput`; `read_text` requires `ReadText`.
  - Capability denial happens before socket I/O, covered by tests.

## Diff summary

- Code/content commit: `eee69bd`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Behavioural delta: default SDK clients remain fully capable, but callers can now create restricted clients and get deterministic local denial before transport access.

## Operator-takeaway

The SDK now has the first capability vocabulary and local enforcement layer. This is not daemon-side security yet, but it establishes the typed policy boundary for future authenticated/scoped clients.
