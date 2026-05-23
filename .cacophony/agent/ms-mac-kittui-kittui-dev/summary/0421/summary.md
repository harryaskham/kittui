# Session summary — SDK scrollback/wait docs

## Goal

Document the typed SDK scrollback and wait helpers that landed for native automation.

## Bead(s)

- `bd-d371cc` — docs: SDK scrollback and wait helper status

## Before state

- Failing tests: none known.
- Relevant context: `ScrollbackSnapshot`, `SurfaceHandle::read_scrollback`, `wait_text[_ms]`, and `wait_output[_ms]` landed, but docs still only described socket/CLI read/wait automation paths.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context:
  - Updated `docs/kittwm-sdk-plan.md` typed helper summary to include screen text, scrollback snapshots, and visible/output waits.
  - Updated `docs/wm.md` automation paragraph with typed SDK helper names and read-capability gating.
  - Updated `docs/README.md` artifacts/status list with the SDK read automation helper coverage.

## Parallel coordination

- Assigned `bd-f26180` to `kittui-dev-2`: SDK exact bytes, paste bytes, and mouse helpers.

## Diff summary

- Code/content commit: `bfa3d70f`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/kittwm-sdk-plan.md`, `docs/wm.md`, `docs/README.md`
- Behavioural delta: docs only.

## Operator-takeaway

Docs now describe SDK read/wait automation parity with the existing socket/CLI paths.
