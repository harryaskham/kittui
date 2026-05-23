# Session summary — SDK scrollback and wait helpers

## Goal

Expose existing native automation socket operations through typed `kittwm-sdk` helpers so clients do not need raw protocol strings for scrollback reads and waits.

## Bead(s)

- `bd-b9b860` — kittwm-sdk: typed scrollback and wait helpers

## Before state

- Failing tests: none known.
- Relevant context: daemon/CLI already exposed `READ_SCROLLBACK_JSON`, `WAIT_TEXT[_MS]`, and `WAIT_OUTPUT[_MS]`; SDK only wrapped `READ_TEXT_JSON`.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk scrollback -- --nocapture` passed.
  - `cargo test -p kittwm-sdk wait -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `ScrollbackSnapshot` typed response.
  - Added `SurfaceHandle::read_scrollback()`.
  - Added wait helpers:
    - `wait_text_ms(ms, needle)`
    - `wait_output_ms(ms, needle)`
    - `wait_text(needle)`
    - `wait_output(needle)`
  - Helpers are gated by the existing `ReadText` capability.
  - Added JSON decode, capability-denial, and Unix socket command-format tests.
  - Rebased after `kittui-dev-2` landed `bd-d9ccc5` at `26a07cb`.

## Parallel coordination

- `kittui-dev-2` completed docs for typed SDK session helpers.

## Diff summary

- Code/content commit: `326361c0`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Behavioural delta: SDK automation clients can now read scrollback and wait for visible/output text without raw socket commands.

## Operator-takeaway

The SDK now covers the core read/wait automation path: screen text, scrollback, visible-text waits, and output waits.
