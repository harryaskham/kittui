# Session summary — Add summary alias

## Goal

Improve kittui-md CLI ergonomics by adding `--summary` as a friendly alias for the concise stats output.

## Bead(s)

- `bd-6a3b0d` — Add kittui-md summary alias for stats

## Before state

- Failing tests: none known.
- Relevant metrics: concise document summary output was available only as `--stats`, even though users may naturally look for `--summary`.
- Context: recent kittui-md work added aliases such as `--toc`, `--frontmatter`, and `--json`; stats should have a similarly discoverable spelling.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md summary -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --summary docs/examples/kittui-md-proof.md | rg 'kittui-md stats|source.path=docs/examples/kittui-md-proof.md|render.width_cells='` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--summary` maps to the same `Mode::Stats` output as `--stats`, and conflict detection rejects using both output flags together.

## Diff summary

- Code/content commits: `725e363`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parse coverage for alias acceptance and alias/conflict behavior.
- Behavioural delta: users can run `kittui-md --summary` for the existing stats output.

## Operator-takeaway

The concise kittui-md document summary is now available through both `--stats` and the more discoverable `--summary` alias.
