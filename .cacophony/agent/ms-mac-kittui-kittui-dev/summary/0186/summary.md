# Session summary — Add outline-json mode

## Goal

Add a compact machine-readable `kittui-md --outline-json` mode for tools that need Markdown heading outline records without the full metadata JSON payload.

## Bead(s)

- `bd-0894f0` — Add kittui-md outline-json inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: heading outlines were available as text through `--outline`/`--toc`/`--headings`, anchors were available through `--anchors-json`, and full metadata JSON included outline entries. There was no focused JSON output matching the human outline surface.
- Context: outline/toc tooling may need indexed level/text/anchor records without parsing text or full document metadata.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md outline_json -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --outline-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"outline"|"text": "Kittui Markdown Proof"|"anchor": "kittui-markdown-proof"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--outline-json` emits `schema_version: 1` plus indexed heading outline records with `level`, `text`, and `anchor`.

## Diff summary

- Code/content commits: `6fb2460`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict tests and JSON output coverage for outline JSON.
- Behavioural delta: users and tools can request compact JSON for heading outline records.

## Operator-takeaway

kittui-md now has both human-readable and machine-readable focused outline/table-of-contents inspection modes.
