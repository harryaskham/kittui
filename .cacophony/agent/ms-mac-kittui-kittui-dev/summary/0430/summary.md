# Session summary — SDK typed wait and event accessor docs

## Goal

Document typed wait-match results and event envelope/detail accessors.

## Bead(s)

- `bd-57f571` — docs: SDK typed wait results and event accessors

## Before state

- Failing tests: none known.
- Relevant context: `kittui-dev-2` landed `WaitMatchKind`, `WaitMatch`, `wait_text_match[_ms]`, and `wait_output_match[_ms]`; this agent landed `KittwmEvent::envelope`, `unknown_raw`, and `EventEnvelope::detail_*`. Docs did not describe either ergonomic layer.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context:
  - Updated `docs/kittwm-sdk-plan.md` typed helper summary with typed wait-match helpers and event accessors.
  - Updated `docs/wm.md` automation paragraph to explain typed wait-match helpers parse existing `MATCH_TEXT` / `MATCH_OUTPUT` replies while preserving raw wait helpers.
  - Updated `docs/wm.md` event stream paragraph with event envelope/detail accessors.
  - Updated `docs/README.md` current SDK helper coverage.

## Parallel coordination

- Assigned `bd-443ae5` to `kittui-dev-2`: bounded event iterator helper.

## Diff summary

- Code/content commit: `25443e77`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/kittwm-sdk-plan.md`, `docs/wm.md`, `docs/README.md`
- Behavioural delta: docs only.

## Operator-takeaway

Docs now reflect the SDK ergonomics around typed wait results and event envelope/detail accessors.
