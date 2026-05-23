# Session summary — first-party kittwm app docs

## Goal

Update docs for matured first-party `kittwm-terminal` and `kittwm-launch` SDK app capabilities.

## Bead(s)

- `bd-a35cb2` — docs: update first-party kittwm app docs

## Before state

- Failing tests: none known.
- Relevant context: `kittwm-terminal` now has `--status`/`--events-ms`; `kittwm-launch` matured with URL/browser auto-selection, `--dry-run`, and `--status`, but docs did not summarize that current behavior.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context:
  - Updated `docs/README.md` examples/artifacts list with `kittwm-terminal` and `kittwm-launch` current capabilities.
  - Updated `docs/wm.md` native socket/session area to mention first-party SDK apps.
  - Coordinated with kittui-dev-2: they landed `bd-a74d58` launcher maturity.

## Diff summary

- Code/content commit: `e69f6aec`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/README.md`, `docs/wm.md`
- Behavioural delta: docs only.

## Operator-takeaway

Docs now call out `kittwm-terminal` and `kittwm-launch` as useful first-party SDK apps, not just skeletons.
