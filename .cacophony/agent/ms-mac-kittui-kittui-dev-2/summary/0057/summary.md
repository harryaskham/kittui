# Session summary — SDK JSON wait helper docs

## Goal

Complete bd-472268 by documenting the SDK JSON wait match helpers after the source helpers landed.

## Bead(s)

- `bd-472268` — docs: SDK JSON wait helpers
- source context: `bd-6e3c54` — kittwm-sdk: JSON wait match helpers

## Before state

- Failing tests: none known for this docs-only slice.
- Relevant metrics: docs covered CLI/socket JSON wait wrappers and existing SDK `wait_text_match[_ms]` / `wait_output_match[_ms]` helpers, but did not mention the new SDK JSON-command variants.
- Context: waited for bd-6e3c54 to land, then changed docs only.

## After state

- Failing tests: none observed; validation was source-only docs diff checking.
- Relevant metrics: `docs/wm.md` now documents `wait_text_match_json[_ms]` and `wait_output_match_json[_ms]` alongside existing wait helpers, clarifying they use JSON daemon commands and return the same `WaitMatch` shape while raw string helpers remain available. `docs/README.md` and `docs/kittwm-sdk-plan.md` include the SDK JSON wait helpers in the automation/helper inventory.
- Context: docs-only; no source code changed in this bead.

## Diff summary

- Code/content commits: `829f360` (`bd-472268: document SDK JSON wait helpers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/README.md`, `docs/kittwm-sdk-plan.md`, `docs/wm.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: documentation-only; docs now match the landed SDK JSON wait helpers.
- Validation: `git diff --check`.

## Operator-takeaway

SDK automation docs now distinguish raw-reply wait parsing from JSON-command wait helpers while making clear both produce typed `WaitMatch` metadata for callers.
