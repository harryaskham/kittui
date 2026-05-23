# Session summary — SDK native event stream wrapper

## Goal

Wrap native kittwm socket `EVENTS [ms]` JSON-lines stream in typed `kittwm-sdk` APIs.

## Bead(s)

- `bd-93f42e` — kittwm-sdk: wrap native socket event stream

## Before state

- Failing tests: none known.
- Relevant context: daemon/socket `EVENTS [ms]` existed, but SDK clients had no typed event API and had to use raw requests/parsing.

## After state

- Failing tests: none in validation.
- Validation:
  - `cargo test -p kittwm-sdk -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `EventEnvelope` and `KittwmEvent` for current native event kinds: `status`, `status_changed`, `pane_opened`, `pane_closed`, `pane_changed`, `focus_changed`, `layout_changed`, and unknown fallback.
  - Added `KittwmEvent::parse_line(...)` and `kind()`.
  - Added `Kittwm::events_ms(ms)` that sends bounded `EVENTS <ms>`, parses JSON lines, and stops at `END`.
  - Enforces `SubscribeEvents` capability before I/O.
  - Added parser tests, capability denial tests, and a small Unix socket server test verifying `EVENTS 250` command construction and line parsing.
  - Coordinated with kittui-dev-2: they are on `bd-67a477` local file/shared-memory raw-frame transport.

## Diff summary

- Code/content commit: `b5706b65`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Behavioural delta: SDK API only; no daemon/runtime behavior change.

## Operator-takeaway

SDK clients now have typed bounded event polling over the existing native socket event stream.
