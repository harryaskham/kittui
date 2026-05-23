# Session summary — document render-many platform APIs

## Goal

Update docs so the newly added render-many platform APIs are part of the documented Rust/FFI/TS/Python renderer story.

## Bead(s)

- `bd-2f12a0` — docs: document render-many platform APIs

## Before state

- Failing tests: none known.
- Relevant gap: DESIGN's ABI shape still listed only single-scene `kittui_render_json`, and README did not mention binding-level render-many manifests despite Rust/FFI/Python/TS/CLI support.

## After state

- Failing tests: none.
- Relevant metrics:
  - `git diff --check` passed.
- Context: DESIGN ABI shape now lists `kittui_render_many_json` and explains the JSON manifest (`count`, `images[]`, `index`, `bytes`, `footprint`, `png_base64`). Python binding section now includes `render`/`render_many`. README platform binding bullet now mentions render-only PNG bytes and render-many manifests with base64 PNG entries.

## Diff summary

- Code/content commit: `fa226a4`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `README.md`, `DESIGN.md`
- Behavioural delta: documentation now reflects the full render-only batch platform API surface.

## Operator-takeaway

External platform users can discover the one-call render-many manifest contract from the primary docs.
