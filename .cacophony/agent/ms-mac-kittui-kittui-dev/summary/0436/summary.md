# Session summary — raw RGB kitty transport gap docs

## Goal

Explain the remaining raw RGB (`f=24`) kitty transport gap and why it is lower priority than response/capability probing.

## Bead(s)

- `bd-bf4e82` — docs: explain raw RGB kitty transport gap

## Before state

- Failing tests: none known.
- Relevant context: protocol conformance listed raw RGB helper coverage as open but did not explain trade-offs versus current PNG/raw RGBA paths.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context:
  - Added a `Raw RGB (f=24) priority` section to `docs/protocol-conformance.md`.
  - Clarified current hot paths use PNG (`f=100`) and raw RGBA (`f=32`).
  - Explained raw RGB saves one byte/pixel only when callers already own three-channel data, while current renderer/WM captures produce RGBA and would need conversion.
  - Sketched a future additive helper shape without changing current defaults.

## Parallel coordination

- Assigned `bd-02ef7b` to `kittui-dev-2`: docs plan for kitty response reading and `a=q` capability probing.

## Diff summary

- Code/content commit: `a3992921`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/protocol-conformance.md`
- Behavioural delta: docs only.

## Operator-takeaway

The raw RGB gap is now documented as a lower-priority additive helper, with response/capability probing remaining the more important kitty protocol gap.
