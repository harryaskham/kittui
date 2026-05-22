# Session summary — Add heading anchors

## Goal

Add stable heading anchors to kittui-md outline metadata so outlines are useful for navigation and downstream tooling, not just display text.

## Bead(s)

- `bd-9094ff` — Add heading anchors to kittui-md outlines

## Before state

- Failing tests: none known.
- Relevant metrics: `HeadingOutline` carried only heading level and text; `--outline`, plain metadata sections, and metadata JSON could not expose anchor slugs.
- Context: kittui-md already exposes rich metadata for links/images/tables/frontmatter, and headings should include stable anchors for navigation.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-affordances heading_anchors -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittui-md outline -- --nocapture` passed.
  - `cargo test -p kittui-cli --bin kittui-md metadata_json_mode_reports_stable_shape -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --outline docs/examples/kittui-md-proof.md | rg '#kittui-md-proof-gallery|#components'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --metadata-json docs/examples/kittui-md-proof.md | rg '"anchor": "kittui-md-proof-gallery"|"anchor": "components"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: headings now get deterministic slug anchors, duplicate headings receive numeric suffixes, and punctuation-only headings fall back to `section`.

## Diff summary

- Code/content commits: `903f292`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-affordances/src/markdown.rs`, `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added affordances coverage for unique heading anchors and updated CLI outline/plain/JSON expectations.
- Behavioural delta: `--outline`, plain outline metadata, and metadata JSON now include stable heading anchors.

## Operator-takeaway

kittui-md outlines now carry stable `#anchor` slugs, making them suitable for navigation and downstream document indexing.
