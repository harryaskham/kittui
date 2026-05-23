# Session summary — semantic runtime events

## Goal

Emit semantic-specific events when semantic snapshots are published, focused, or mutated so SDK clients can observe semantic changes without polling snapshots.

## Bead(s)

- `bd-23b373` — kittwm: emit semantic snapshot and action events

## Before state

- Failing tests: none known.
- Relevant context: `SEMANTIC_PUBLISH`, `SEMANTIC_FOCUS`, and `SEMANTIC_ACTION` could mutate published semantic snapshots, but the bounded `EVENTS [ms]` stream only covered status/pane/focus/layout changes.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib daemon::tests::native_spawn_queue_routes_published_semantic_focus_and_actions -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib daemon::tests::native_spawn_queue_rejects_fallback_semantic_action_and_focus_until_supported -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - `SEMANTIC_PUBLISH` now emits `semantic_snapshot_ready` with revision/focus detail.
  - `SEMANTIC_FOCUS` on published snapshots emits `semantic_focus_changed`.
  - `SEMANTIC_ACTION` on published snapshots emits `semantic_action_invoked` and, when a value changes, `semantic_value_changed`.
  - Basic semantic actions increment snapshot revision.
  - Existing fallback PTY semantic snapshots remain read-only and unsupported for focus/action.
  - Existing status/pane/focus/layout event behavior remains unchanged.
  - Tests assert semantic event kinds and schema versioning.

## Diff summary

- Code/content commit: `92d2d822`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/daemon.rs`
- Behavioural delta: semantic changes now appear in the native `EVENTS [ms]` stream.

## Operator-takeaway

Semantic SDK apps can now publish/mutate snapshots and observe semantic snapshot/action/focus/value events over the existing bounded event stream.
