# Session summary — Include schemas in mode-search-json

## Goal

Refine `kittui-md --mode-search-json` so search results for JSON output modes include the same schema-summary information exposed by targeted `--mode-info-json`.

## Bead(s)

- `bd-96455e` — Include schema summaries in kittui-md mode-search-json

## Before state

- Failing tests: none known.
- Relevant metrics: `--mode-search-json` returned matching mode flags, aliases, and descriptions, but callers had to follow up with `--mode-info-json` or `--schemas-json` to learn top-level keys for matching JSON outputs.
- Context: search results are more useful for tooling when they carry schema summaries inline.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md mode_search_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --mode-search-json table | rg '"flag": "--tables-json"|"json_schema"|"top_level_keys"|"tables"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: each search match now includes `json_schema`; non-JSON modes get `null`, JSON modes get `top_level_keys` and schema description.

## Diff summary

- Code/content commits: `3dc3661`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: updated mode-search JSON coverage to assert schema summaries for JSON modes and null schema for text modes.
- Behavioural delta: JSON mode search is now directly actionable for tooling that needs output-shape hints.

## Operator-takeaway

Mode search JSON now closes the discovery loop: a single query can find relevant modes and expose schema hints for machine-readable outputs.
