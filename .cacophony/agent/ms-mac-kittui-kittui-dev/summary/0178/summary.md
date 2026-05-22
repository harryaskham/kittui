# Session summary — Add footnotes-json mode

## Goal

Add a compact machine-readable `kittui-md --footnotes-json` mode for tools that need Markdown footnote references and definitions without the full metadata JSON payload.

## Bead(s)

- `bd-4b5138` — Add kittui-md footnotes-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: footnote references/definitions were available as text through `--footnotes`/`--notes` and inside full metadata JSON, but there was no focused JSON footnote-only output.
- Context: footnote audit tooling may need indexed references and definitions without parsing text or full document metadata.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md footnotes_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --footnotes-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"references"|"definitions"|"label": "proof"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--footnotes-json` emits `schema_version: 1`, indexed reference label records, and indexed definition records.

## Diff summary

- Code/content commits: `75f8f5e`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON output coverage for footnotes JSON.
- Behavioural delta: users and tools can request compact JSON footnote records.

## Operator-takeaway

kittui-md now has both human-readable and machine-readable focused footnote inspection modes.
