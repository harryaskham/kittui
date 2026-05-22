# Session summary — Show render width in stats

## Goal

Bring `kittui-md --stats` closer to metadata JSON parity by reporting the render width used for Markdown/component layout.

## Bead(s)

- `bd-d1bb41` — Show render width in kittui-md stats

## Before state

- Failing tests: none known.
- Relevant metrics: metadata JSON exposed `render.width_cells`, but text stats only reported source provenance and document counts.
- Context: width affects wrapping/layout and should be visible in concise text logs.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md stats_mode -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --stats --width 72 docs/examples/kittui-md-proof.md | rg 'render.width_cells=72|source.path=docs/examples/kittui-md-proof.md'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: stats output now includes `render.width_cells=<n>`.

## Diff summary

- Code/content commits: `b3c5aad`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: updated stats-mode tests to assert render width for stdin/file cases.
- Behavioural delta: `kittui-md --stats` now records the render width that shaped the document summary.

## Operator-takeaway

Stats logs now carry both input provenance and render-width provenance, making them more useful for reproducing layout-dependent output.
