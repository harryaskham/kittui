# Session summary — Shortcut JSON catalog docs

## Goal

Complete bd-736c85 by documenting the machine-readable kittwm shortcut catalog after the source surfaces landed.

## Bead(s)

- `bd-736c85` — docs: kittwm shortcuts JSON catalog
- source context: `bd-605ca4` — kittwm: machine-readable shortcut catalog

## Before state

- Failing tests: none known for this docs-only slice.
- Relevant metrics: docs described `Ctrl-A ?` shortcut help but did not mention `SHORTCUTS_JSON` or CLI JSON shortcut catalog entry points.
- Context: waited for bd-605ca4 to land, then changed docs only.

## After state

- Failing tests: none observed; validation was source-only docs diff checking.
- Relevant metrics: `docs/wm.md` now states that the same shortcut catalog exposed in the TUI help overlay is available as text via `kittwm shortcuts` / `kittwm --shortcuts` and as JSON via `kittwm shortcuts-json` / `kittwm --shortcuts-json` / socket `SHORTCUTS_JSON`. `docs/README.md` summarizes the machine-readable shortcut catalog in the current kittwm status list.
- Context: docs-only; no CLI/daemon/runtime code changed in this bead.

## Diff summary

- Code/content commits: `abe8694` (`bd-736c85: document shortcuts JSON catalog`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/README.md`, `docs/wm.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: documentation-only; docs now match the landed shortcut catalog surfaces.
- Validation: `git diff --check`.

## Operator-takeaway

Shortcut help is now documented as both an interactive overlay and an inspectable catalog: automation can read `SHORTCUTS_JSON` without scraping the `Ctrl-A ?` display.
