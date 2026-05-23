# Session summary — duplicate origin-mode bead documentation anchor

## Goal

Make duplicate bead `bd-8d4980` closable by adding an explicit mainline documentation anchor for already-landed DEC origin mode support.

## Bead(s)

- `bd-8d4980` — kittwm: support DEC origin mode in native PTY

## Before state

- Failing tests: none known.
- Relevant context: DEC origin mode support already landed under `bd-b145a9`, but this earlier duplicate bead id remained in-progress and close validation rejected it because the id was not in recent mainline commits.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context: README now explicitly notes that native PTY rendering handles scroll regions plus DEC origin mode for TUI body/status layouts.

## Diff summary

- Code/content commit: `a199e3d`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `README.md`
- Behavioural delta: docs only; runtime behavior already landed.

## Operator-takeaway

`bd-8d4980` should now be closable after reintegration; the actual implementation remains the previously-landed DEC origin mode work.
