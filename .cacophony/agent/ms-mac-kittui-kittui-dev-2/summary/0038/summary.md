# Session summary — NativeSurface adapter docs

## Goal

Complete bd-ca24f6 as a docs-only follow-up for the newly landed capture-only `NativeSurface` adapters: `KittuiSceneSurface`, `RgbaFrameSurface`, and `CompositeFrameSurface`.

## Bead(s)

- `bd-ca24f6` — docs: document new NativeSurface adapters

## Before state

- Failing tests: none known for this docs-only slice.
- Relevant metrics: docs described PTY/browser/XQuartz/Xvfb NativeSurface progress, but did not mention the newer capture-only scene/RGBA/composite surface adapters or clarify their live-runtime wiring status.
- Context: kittui-dev took a separate code follow-up for `CompositeFrameSurface::push_surface_frame`, so this slice stayed docs-only and did not touch `native.rs`.

## After state

- Failing tests: none observed; validation was source-only docs diff checking.
- Relevant metrics: `docs/kittwm-sdk-plan.md`, `docs/wm.md`, and `docs/README.md` now mention `KittuiSceneSurface`, `RgbaFrameSurface`, and `CompositeFrameSurface`. Docs state that scene surfaces render through the CPU renderer to PNG, RGBA/composite surfaces expose raw RGBA frames, and default live kittwm runtime wiring remains follow-up work.
- Context: docs-only; no runtime, SDK, or daemon behavior changed.

## Diff summary

- Code/content commits: `ee0dd6a` (`bd-ca24f6: document NativeSurface adapters`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/README.md`, `docs/kittwm-sdk-plan.md`, `docs/wm.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: documentation-only; docs now reflect current capture-only NativeSurface adapter status.
- Validation: `git diff --check`.

## Operator-takeaway

The docs now distinguish the landed scene/RGBA/composite NativeSurface building blocks from the still-open task of wiring them into the default live kittwm runtime.
