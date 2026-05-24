# Session summary — Text JSON read wrapper docs

## Goal

Complete bd-5c73cb by documenting the stable `kittwm --read-text-json` and `--read-scrollback-json` CLI wrappers after the source wrappers landed.

## Bead(s)

- `bd-5c73cb` — docs: kittwm text JSON read wrappers
- source context: `bd-2a18a3` — kittwm: CLI wrappers for text JSON reads

## Before state

- Failing tests: none known for this docs-only slice.
- Relevant metrics: docs mentioned raw `READ_TEXT_JSON` / `READ_SCROLLBACK_JSON` protocol and plain `--read-text` / `--read-scrollback` wrappers, but not the JSON CLI wrappers.
- Context: waited for bd-2a18a3 to land, then changed docs only.

## After state

- Failing tests: none observed; validation was source-only docs diff checking.
- Relevant metrics: `docs/wm.md` now lists `--read-text-json` and `--read-scrollback-json` alongside the other automation wrappers and states they map to `READ_TEXT_JSON` / `READ_SCROLLBACK_JSON`. `docs/README.md` summarizes the JSON text read wrappers in the SDK/automation helper inventory.
- Context: docs-only; no CLI/daemon/SDK source code changed in this bead.

## Diff summary

- Code/content commits: `46de1d6` (`bd-5c73cb: document text JSON read wrappers`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/README.md`, `docs/wm.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: documentation-only; docs now match the landed text JSON read CLI wrappers.
- Validation: `git diff --check`.

## Operator-takeaway

Automation now has documented stable CLI entry points for structured pane text and scrollback snapshots, without requiring callers to spell raw socket commands.
