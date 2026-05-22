# Session summary — Add headings alias

## Goal

Improve kittui-md CLI ergonomics by adding `--headings` as another friendly alias for the outline/table-of-contents view.

## Bead(s)

- `bd-b34a3c` — Add kittui-md headings alias for outline

## Before state

- Failing tests: none known.
- Relevant metrics: heading outline inspection was available through `--outline` and `--toc`, but not the direct `--headings` term.
- Context: kittui-md now has several focused inspection modes; multiple intuitive spellings reduce friction for users exploring Markdown structure.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md headings -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --headings docs/examples/kittui-md-proof.md | rg 'kittui-md outline|kittui-md proof gallery'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--headings` maps to the same `Mode::Outline` output, and conflict detection rejects using it with `--outline`.

## Diff summary

- Code/content commits: `caf8d50`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser coverage for alias acceptance and alias/mode conflict behavior.
- Behavioural delta: users can run `kittui-md --headings` for heading outline output.

## Operator-takeaway

The outline view is now discoverable via `--outline`, `--toc`, and `--headings`.
