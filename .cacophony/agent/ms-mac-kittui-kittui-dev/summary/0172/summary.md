# Session summary — Add counts mode

## Goal

Add a focused `kittui-md --counts` mode for quick document structure counts without source/render provenance or full metadata JSON details.

## Bead(s)

- `bd-601b9f` — Add kittui-md counts inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: counts were available in `--stats` and metadata JSON, but there was no minimal text-only counts output.
- Context: tooling and humans sometimes need just structural counts without path/render fields or detailed arrays.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md counts -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --counts docs/examples/kittui-md-proof.md | rg 'kittui-md counts|components=|heading_anchors=7|metadata_blocks=1'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--counts` prints the shared component/metadata count lines and intentionally omits `source.*` and `render.*` fields.

## Diff summary

- Code/content commits: `72bb39b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/conflict coverage and output coverage for counts mode.
- Behavioural delta: users can run `kittui-md --counts` for a compact count-only summary.

## Operator-takeaway

kittui-md now has a minimal count-only inspection mode for fast structural checks.
