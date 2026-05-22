# Session summary — Add anchors-json mode

## Goal

Add a focused machine-readable `kittui-md --anchors-json` mode for tools that need heading anchor records without the full metadata JSON payload.

## Bead(s)

- `bd-6a7086` — Add kittui-md anchors-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: anchors were available as text through `--anchors`/`--slugs` and inside full metadata JSON, but there was no compact JSON anchor-only output.
- Context: navigation/indexing tools may need just heading anchor records and should not need to parse text or full document metadata.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md anchors_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --anchors-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"anchor": "kittui-md-proof-gallery"|"anchor": "components"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--anchors-json` emits `schema_version: 1` plus indexed anchor records with heading level, slug, and text.

## Diff summary

- Code/content commits: `1a37232`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON output coverage for anchors JSON.
- Behavioural delta: users and tools can request compact JSON heading anchor records.

## Operator-takeaway

kittui-md now has both human-readable and machine-readable focused anchor inspection modes.
