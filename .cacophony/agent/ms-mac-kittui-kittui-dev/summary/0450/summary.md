# Session summary — clipboard read policy docs

## Goal

Refresh docs after runtime `CLIPBOARD_JSON` landed so docs no longer state clipboard read policy is absent.

## Bead(s)

- `bd-1fbb94` — docs: document kittwm clipboard read policy

## Before state

- Failing tests: none known.
- Relevant context: docs still described clipboard read policy as future/absent after runtime cache-only policy landed.

## After state

- Failing tests: none in docs validation.
- Validation:
  - `git diff --check` passed.
- Context:
  - Updated `docs/wm.md` with `CLIPBOARD_JSON`, default-deny behavior, `KITTWM_CLIPBOARD_READ=allow|1|true|yes`, cache-only semantics, and no host-OS clipboard reads.
  - Updated `docs/kittwm-sdk-plan.md` current-state/backlog language to reflect runtime clipboard policy and remaining SDK helper/docs gap.

## Parallel coordination

- `kittui-dev-2` remains assigned to `bd-d582b7` for SDK/docs typed `PaneFramePresented` events, but reported they are investigating a nix build failure / PTY child availability issue.

## Diff summary

- Code/content commit: `f18592c9`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `docs/wm.md`
  - `docs/kittwm-sdk-plan.md`

## Operator-takeaway

Docs now match the runtime clipboard policy foundation landed in `bd-6957a0`.
