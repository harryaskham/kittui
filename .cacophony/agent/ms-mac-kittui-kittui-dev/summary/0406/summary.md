# Session summary — semantic documentation map

## Goal

Add a central docs map so contributors can find the semantic surface architecture, quickstarts, browser/accessibility adapter plans, transport notes, and SDK plan.

## Bead(s)

- `bd-915c05` — docs: add semantic surfaces documentation map

## Before state

- Failing tests: none known.
- Relevant context: semantic documentation was spread across multiple docs files, and README did not point to a consolidated docs map.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context:
  - Added `docs/README.md`.
  - The map links core kittwm docs, graphics transport/frame performance docs, semantic surface docs, browser/accessibility adapter docs, and examples.
  - It summarizes current implementation status for graphics transport and semantic surfaces.
  - Updated top-level `README.md` to point to the docs map alongside `DESIGN.md`.
  - Coordinated with kittui-dev-2: they are assigned the macOS AX semantic adapter spike.

## Diff summary

- Code/content commit: `38441ae8`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/README.md`, `README.md`
- Behavioural delta: docs only.

## Operator-takeaway

There is now a single entry point for the growing kittui/kittwm docs, including semantic architecture and implementation status.
