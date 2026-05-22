# Session summary — Add counts-json mode

## Goal

Add a minimal machine-readable `kittui-md --counts-json` mode for tools that need document counts without the full metadata JSON payload.

## Bead(s)

- `bd-cfca14` — Add kittui-md counts-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: counts were available as text via `--counts` and as part of the full `--metadata-json` payload, but there was no compact JSON counts-only output.
- Context: some tooling needs only structural counts and should not have to parse detailed arrays or text lines.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md counts_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --counts-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"counts"|"heading_anchors": 7|"metadata_blocks": 1'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--counts-json` emits `{ schema_version: 1, counts: { ... } }` and deliberately omits source provenance and detailed arrays.

## Diff summary

- Code/content commits: `cc1bf73`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and output coverage for counts JSON.
- Behavioural delta: users and tools can request a compact JSON counts payload.

## Operator-takeaway

kittui-md now supports both human-readable and machine-readable count-only summaries.
