# Session summary — document CLI render-only PNG command

## Goal

Update top-level docs so shell users can discover the new `kittui render` PNG artifact path.

## Bead(s)

- `bd-080401` — docs: document CLI render-only PNG command

## Before state

- Failing tests: none known.
- Relevant gap: README/DESIGN still advertised only terminal placement and compose flows, even though `kittui render` now provides render-only PNG output for previews/artifacts/non-terminal embedding.

## After state

- Failing tests: none.
- Relevant metrics:
  - `git diff --check` passed.
- Context: README CLI surface and quick start now include `kittui render scene.json --out preview.png` / pipeline usage. DESIGN CLI section now explains that most subcommands place via `Runtime::place`, while `render` uses `Runtime::render_png`; command surface and shell pipeline examples document render-only PNG output.

## Diff summary

- Code/content commit: `b0aa626`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `README.md`, `DESIGN.md`
- Behavioural delta: documentation now reflects the CLI render-only PNG workflow.

## Operator-takeaway

Shell/platform users can now find the direct PNG artifact path in the primary docs instead of inferring it from Rust/FFI/Python APIs.
