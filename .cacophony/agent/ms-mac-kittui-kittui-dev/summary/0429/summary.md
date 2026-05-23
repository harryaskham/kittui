# Session summary — SDK event envelope accessors

## Goal

Improve SDK event ergonomics so clients can inspect common event metadata and detail fields without repetitive matches.

## Bead(s)

- `bd-a06c44` — kittwm-sdk: event envelope convenience accessors

## Before state

- Failing tests: none known.
- Relevant context: `KittwmEvent` variants carried `EventEnvelope`, but clients had to match each known variant manually and inspect raw `serde_json::Value` detail fields directly.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittwm-sdk event -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `EventEnvelope::detail_str`, `detail_bool`, and `detail_u64` helpers.
  - Added `KittwmEvent::envelope()` for all typed known event variants.
  - Added `KittwmEvent::unknown_raw()` for unknown forward-compatible raw events.
  - Added tests covering known surface event details and unknown fallback.
  - No parser behavior or daemon behavior changed.

## Parallel coordination

- `bd-f763bd` remains assigned to `kittui-dev-2`: typed SDK wait-match results.

## Diff summary

- Code/content commit: `fa5251ca`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Behavioural delta: SDK event consumers can inspect envelopes and common detail fields more easily.

## Operator-takeaway

The SDK event API is easier to consume without weakening forward compatibility for unknown events.
