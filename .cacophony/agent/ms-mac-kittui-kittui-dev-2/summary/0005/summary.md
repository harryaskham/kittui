# Session summary — Adaptive graphics transport plan

## Goal

Produce the architecture plan requested by bd-3c0dd1 for choosing graphics transport automatically across kittui and kittwm, especially for high-rate window-manager surfaces where direct kitty escapes, tmux passthrough, file/shared-memory transfer, zlib, and pure-terminal fallback have different safety and performance tradeoffs.

## Bead(s)

- `bd-3c0dd1` — kittui-kitty: plan adaptive graphics transport selection

## Before state

- Failing tests: none known for this doc/planning bead.
- Relevant metrics: `TerminalInfo::detect()` documented only environment-based transport probing; kittwm docs mentioned tmux-safe renderer fallback and zlib env toggles but did not define a broader selection policy.
- Context: kittui-dev asked me to take this remaining open bead while they worked a separate SDK semantic-protocol-types slice; I rebased before claiming and again before preparing reintegration.

## After state

- Failing tests: none observed.
- Relevant metrics: new design note defines policy inputs, priority order, overrides/defaults, raw RGBA and zlib safeguards, and tracked follow-up implementation beads.
- Context: docs now link the adaptive policy from both `DESIGN.md` and `docs/wm.md` so future implementation work has an in-repo baseline.

## Diff summary

- Code/content commits: `74b1e88` (`bd-3c0dd1: plan adaptive graphics transport`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/adaptive-graphics-transport.md`, `DESIGN.md`, `docs/wm.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: no runtime behavior changed; the project now has a concrete adaptive graphics transport plan covering kitty support, local-vs-SSH topology, tmux safety, payload size, frame cadence, file/shared-memory availability, zlib thresholds, and explicit overrides.
- Validation: `git diff --check` passed before commit.
- Follow-up beads filed: `bd-67a477` (local shared-memory/file raw-frame transport), `bd-e15ef8` (threshold-based zlib auto compression), `bd-883864` (transport diagnostics).

## Operator-takeaway

This was intentionally a planning slice: it turns the transport performance concern into a durable policy document plus three tracked implementation beads, so the next workers can implement shared-memory/file transport, compression thresholds, and diagnostics without rediscovering the design constraints.
