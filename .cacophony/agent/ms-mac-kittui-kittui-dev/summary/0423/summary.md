# Session summary — SDK input helper docs

## Goal

Document SDK exact byte, paste byte, and mouse input helpers landed by `kittui-dev-2`.

## Bead(s)

- `bd-5ef731` — docs: SDK exact bytes paste and mouse helper status

## Before state

- Failing tests: none known.
- Relevant context: `bd-f26180` landed at `96c51b7`; SDK `SurfaceHandle` now has `send_bytes`, `send_bytes_b64`, `paste_bytes`, `paste_bytes_b64`, and `send_mouse(MouseEvent, col, row)`, gated by `SendInput`. Docs still mostly described socket/CLI input paths.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context:
  - Updated `docs/kittwm-sdk-plan.md` typed helper summary with exact bytes, bracketed paste bytes, and mouse events.
  - Updated `docs/wm.md` automation paragraph with helper names and `SendInput` gating.
  - Updated `docs/README.md` current SDK automation helper coverage.

## Parallel coordination

- Assigned `bd-59af9e` to `kittui-dev-2`: typed SDK layout/focus/move/balance control helpers.

## Diff summary

- Code/content commit: `ce02cad1`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/kittwm-sdk-plan.md`, `docs/wm.md`, `docs/README.md`
- Behavioural delta: docs only.

## Operator-takeaway

Docs now reflect SDK input automation parity for exact bytes, paste bytes, and mouse events.
