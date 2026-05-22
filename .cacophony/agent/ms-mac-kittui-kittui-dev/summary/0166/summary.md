# Session summary — Show heading anchor count in stats

## Goal

Make the recently added heading-anchor feature visible in concise `kittui-md --stats` / `--summary` output.

## Bead(s)

- `bd-5497c6` — Show heading anchor count in kittui-md stats

## Before state

- Failing tests: none known.
- Relevant metrics: stats output reported `headings=<n>`, but did not explicitly report heading-anchor coverage after anchors were added.
- Context: stats mode is used for quick metadata checks, so anchor count should be included alongside other document-structure counts.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md stats_mode -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --stats docs/examples/kittui-md-proof.md | rg 'headings=7|heading_anchors=7'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: stats now prints `heading_anchors=<n>` immediately after `headings=<n>`.

## Diff summary

- Code/content commits: `f5688d5`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: updated stats-mode expectations and docs for heading-anchor count.
- Behavioural delta: concise stats output now surfaces heading-anchor coverage.

## Operator-takeaway

Stats/summary output now makes heading anchors visible without requiring full JSON or anchor-only output.
