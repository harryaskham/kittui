# Session summary — kittwm SDK/surface architecture plan

## Goal

Capture the long-term kittwm SDK and surface architecture durably, then file the implementation backlog so future work can separate WM runtime, surface engines, shell UX, and standalone apps.

## Bead(s)

- `bd-950fc8` — kittwm: document SDK and surface architecture plan

## Before state

- Failing tests: none known.
- Relevant gap: the architecture discussion around terminal surfaces, GUI/app surfaces, SDK handles, composite apps, standalone kittwm-terminal/kittwm-launch, and shell dogfooding existed only in conversation, not durable project docs or bead backlog.

## After state

- Failing tests: none introduced.
- Validation:
  - `git diff --check` passed.
- Context: Added `docs/kittwm-sdk-plan.md` covering kittwm core/runtime, surface engines, default shell, standalone apps, SDK objects, surface capabilities, frame types, event streams, renderer split, clipboard/bell/notification policy, capability scoping, staged implementation, and bead backlog mapping.
- Created follow-on beads:
  - `bd-c859be` event stream
  - `bd-099358` TerminalSurface extraction
  - `bd-91eb17` common Surface trait
  - `bd-6e4dcf` browser Surface adapter
  - `bd-3aca3c` Xvfb/XQuartz Surface adapters
  - `bd-8b93cf` kittwm-sdk connect/window handle skeleton
  - `bd-c1d62d` typed SDK surface APIs
  - `bd-f835b9` dogfood handles in built-in session
  - `bd-0957d6` presentation-agnostic shell/chrome model
  - `bd-1b4f3c` pure terminal renderer
  - `bd-25baac` standalone kittwm-terminal skeleton
  - `bd-b0c8d3` standalone kittwm-launch skeleton
  - `bd-ebb7bf` clipboard/bell/notification SurfaceEvents
  - `bd-08dcc2` SDK client capabilities
  - `bd-57a8e5` composite app example

## Diff summary

- Code/content commit: `96b442c`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `docs/kittwm-sdk-plan.md`
- Behavioural delta: docs/backlog only; establishes the architecture roadmap for separating kittwm's runtime, surfaces, shell, SDK, and first-party apps.

## Operator-takeaway

The SDK/surface architecture is now captured in-repo and mapped to filed beads so implementation can proceed incrementally without losing the separation goals.
