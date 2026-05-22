# Session summary — Add stats-json mode

## Goal

Add a compact machine-readable `kittui-md --stats-json` mode for tools that need source/render/count summaries without the full metadata JSON payload.

## Bead(s)

- `bd-876806` — Add kittui-md stats-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: `--stats`/`--summary` printed concise human-readable source path/size, render width, and structural counts. `--counts-json` omitted source/render provenance, while `--metadata-json` included much more detail than quick checks need.
- Context: automation may need the same compact stats surface as JSON without paying for or parsing the full metadata payload.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md stats_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --stats-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"mode": "stats-json"|"source"|"render"|"counts"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--stats-json` emits `schema_version: 1`, `source`, `render`, and `counts` objects.

## Diff summary

- Code/content commits: `d6eb98b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON output coverage for stats JSON.
- Behavioural delta: users and tools can request compact JSON for source/render/count summaries.

## Operator-takeaway

kittui-md now has a middle-ground machine-readable summary: richer than `--counts-json`, much smaller than `--metadata-json`.
