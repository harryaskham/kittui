# Session summary — Add toc alias

## Goal

Improve kittui-md outline discoverability by adding `--toc` as a friendly table-of-contents alias for `--outline`.

## Bead(s)

- `bd-0d127c` — Add kittui-md toc alias for outline

## Before state

- Failing tests: none known.
- Relevant metrics: users could inspect headings via `--outline`, but common table-of-contents terminology was not accepted.
- Context: kittui-md has accumulated focused inspection modes; aliases improve CLI ergonomics without changing the output contract.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md toc -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --toc docs/examples/kittui-md-proof.md | rg 'kittui-md outline|kittui-md proof gallery'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--toc` maps to the same mode as `--outline`, and conflict detection treats `--outline --toc` as mutually exclusive output flags.

## Diff summary

- Code/content commits: `241595b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parse coverage for the alias and conflict behavior.
- Behavioural delta: users can run `kittui-md --toc` to inspect heading outline output.

## Operator-takeaway

Heading inspection is now available through both `--outline` and the familiar `--toc` alias.
