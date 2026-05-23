# Session summary — Accessibility-tree semantic adapter plan

## Goal

Complete bd-4a49aa by documenting how kittwm can use platform accessibility trees (macOS AX and Linux AT-SPI) as semantic sources for arbitrary GUI apps, while preserving pixel capture fallback and treating permissions/security as first-class constraints.

## Bead(s)

- `bd-4a49aa` — kittwm: plan accessibility-tree semantic adapter

## Before state

- Failing tests: none known for this docs/planning bead.
- Relevant metrics: semantic docs listed accessibility tree adapters as a future source, but there was no architecture for platform association, role mapping, events, action routing, sensitive values, or permission diagnostics.
- Context: kittui-dev claimed SDK semantic snapshot renderer via affordances, so this stayed a non-overlapping docs/planning slice.

## After state

- Failing tests: none observed; validation was source-only docs diff checking.
- Relevant metrics: new `docs/kittwm-accessibility-semantic-adapter.md` covers AX/AT-SPI source shape, role/action mapping, snapshot extraction policy, event/update loop, focus/action routing, security/privacy, fallback behavior, and implementation follow-ups.
- Context: semantic surfaces and quickstart docs now link to the accessibility adapter plan.

## Diff summary

- Code/content commits: `cb00d49` (`bd-4a49aa: plan accessibility semantic adapter`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/kittwm-accessibility-semantic-adapter.md`, `docs/kittwm-semantic-surfaces.md`, `docs/kittwm-semantic-quickstart.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: no runtime behavior changed; the project now has a concrete accessibility-tree semantic adapter plan for arbitrary GUI apps.
- Validation: `git diff --check`.
- Follow-up beads filed: `bd-a17062` (macOS AX semantic adapter spike), `bd-dcb522` (Linux AT-SPI semantic adapter spike), `bd-eabe22` (accessibility semantic action routing).

## Operator-takeaway

The non-browser/native-app semantic path is now scoped: kittwm can pursue platform accessibility adapters as opt-in side-band semantic sources over pixel capture, with explicit permission, redaction, fallback, and action-routing constraints.
