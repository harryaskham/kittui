# Session summary — Add anchors mode

## Goal

Add a focused `kittui-md --anchors` output mode for heading-anchor inspection after stable heading anchors were introduced.

## Bead(s)

- `bd-52160e` — Add kittui-md anchors inspection mode

## Before state

- Failing tests: none known.
- Relevant metrics: heading anchors appeared in `--outline`, plain metadata, and metadata JSON, but there was no dedicated output mode for tools that only need heading levels and anchors.
- Context: kittui-md already has focused inspection modes for links, images, tables, metadata blocks, and other structures; anchors deserve the same focused audit surface.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md anchors -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --anchors docs/examples/kittui-md-proof.md | rg 'kittui-md anchors|h1 #kittui-md-proof-gallery|h2 #components'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--anchors` prints `h<level> #<anchor> <heading text>` for every heading, with empty-document output.

## Diff summary

- Code/content commits: `be6638d`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser/output coverage for `--anchors` and documented the mode.
- Behavioural delta: users and tools can request only heading-anchor records without parsing the full outline or JSON.

## Operator-takeaway

Heading anchors now have a dedicated lightweight inspection mode, making kittui-md more useful for navigation/indexing tooling.
