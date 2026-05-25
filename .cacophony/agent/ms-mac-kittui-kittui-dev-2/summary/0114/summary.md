# Session summary — Animated render export docs

## Goal

Document the offline animated frame export workflow for kittui animation artifacts.

## Bead(s)

- `bd-4f8638` — kittui docs: document animated render frame export

## Before state

- `kittui render` could export animated single-scene JSON to frame PNGs, but `docs/inline-animation.md` only covered scene JSON/dry-run inspection.

## After state

- Added an `Offline frame export` section to `docs/inline-animation.md`.
- Documented:
  - generating animated scene JSON
  - `kittui render <scene.json> --out-dir <dir>`
  - `frame-00000.png` naming
  - manifest contents including frame count, pixel dimensions, loop count, byte sizes, `delay_ms`, and output paths
  - `--json` / `--json-bytes` use
  - static single-scene `--out FILE` behavior and clear `--out-dir` error for non-animated single scenes

## Diff summary

- Code/content commits: `881caee` (`bd-4f8638: document animated render export`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `docs/inline-animation.md`
- Validation:
  - `git diff --check`
  - `rg -n "Offline frame export|frame-00000|--out-dir" docs/inline-animation.md`

## Operator-takeaway

The docs now explain how to turn animated kittui elements into offline per-frame PNG artifacts for QA/golden workflows.
