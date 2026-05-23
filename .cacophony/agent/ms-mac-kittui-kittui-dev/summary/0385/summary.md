# Session summary — composite kittwm SDK example

## Goal

Add a first-party composite SDK example that spawns child surfaces, composes their text snapshots side-by-side, and routes input by coordinate offsets.

## Bead(s)

- `bd-57a8e5` — examples: add composite app using terminal and browser surfaces

## Before state

- Failing tests: none known.
- Relevant context: `kittwm-sdk` had typed terminal surface APIs and a reserved browser surface kind, but no example showing how an app can use multiple child surfaces and compose/reroute between them.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --example kittwm_composite_app -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `crates/kittui-cli/examples/kittwm_composite_app.rs`.
  - The example connects via inherited `KITTWM_SOCKET`, spawns a terminal surface, attempts a typed browser surface request, and falls back to a terminal placeholder because browser spawning is not yet exposed by the SDK socket transport.
  - It reads `TextSnapshot`s and composes them side-by-side.
  - It includes coordinate-based routing via `--route-text X Y TEXT`, focusing/sending to the left terminal or right browser/placeholder region based on offsets.
  - Added tests for side-by-side routing and text composition.
  - Updated `docs/kittwm-sdk-plan.md` to point at the example and clarify remaining frame-capture/present immaturity.

## Diff summary

- Code/content commit: `dc6892e4`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/examples/kittwm_composite_app.rs`, `docs/kittwm-sdk-plan.md`
- Behavioural delta: example only; no runtime behavior change.

## Operator-takeaway

The SDK now has a concrete composite-app dogfood example. It demonstrates the intended shape today while making the current browser/GUI spawn limitation explicit.
