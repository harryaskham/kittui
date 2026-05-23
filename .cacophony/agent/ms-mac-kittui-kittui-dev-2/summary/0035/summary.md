# Session summary — Typed pane_input_sent event

## Goal

Implement bd-c0e84a by adding SDK typed parsing and documentation for the native event stream’s `pane_input_sent` event, while leaving daemon runtime emission behavior to kittui-dev’s separate slice.

## Bead(s)

- `bd-c0e84a` — kittwm-sdk: typed pane input event

## Before state

- Failing tests: none known for this bead.
- Relevant metrics: the SDK parsed pane lifecycle, resize, focus/layout, semantic, and surface side-effect events, but did not yet have a `KittwmEvent::PaneInputSent` variant for the planned socket-injected-input event.
- Context: kittui-dev is implementing daemon-only `pane_input_sent` emission for conservative non-sensitive socket input metadata. This slice stayed limited to SDK parsing/tests and docs.

## After state

- Failing tests: none observed in targeted validation.
- Relevant metrics: `KittwmEvent::PaneInputSent(EventEnvelope)` now parses `kind: "pane_input_sent"`, participates in `kind()` and `envelope()`, and has a parser test with non-sensitive detail fields. Docs now list `pane_input_sent` in the bounded `EVENTS [ms]` stream and describe it as conservative non-sensitive socket-injected input metadata.
- Context: no daemon behavior was changed.

## Diff summary

- Code/content commits: `33853ba` (`bd-c0e84a: add typed pane input event`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`, `docs/wm.md`, `docs/README.md`
- Tests: updated `event_parser_handles_known_and_unknown_events` with `pane_input_sent`
- Behavioural delta: SDK clients can now match `KittwmEvent::PaneInputSent`; event emission remains daemon-owned.
- Validation: `cargo test -p kittwm-sdk event_parser -- --test-threads=1`; `cargo check -p kittwm-sdk`; `git diff --check`.

## Operator-takeaway

The SDK/docs are ready for socket-injected input events: consumers can parse `pane_input_sent` as a first-class event as soon as the daemon emits it.
