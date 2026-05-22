# Session summary — Add mode selector

## Goal

Add a `kittui-md --mode <name>` selector so users and tools can choose any output mode by canonical name, flag spelling, or alias without needing a separate dedicated flag path.

## Bead(s)

- `bd-8c5e60` — Add kittui-md --mode name selector

## Before state

- Failing tests: none known.
- Relevant metrics: each kittui-md output mode required spelling its dedicated flag. The new mode catalogs improved discovery, but scripts still needed to translate discovered names back into dedicated flags manually.
- Context: a generic selector improves scripted use across the expanding mode surface.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui-md mode_selector -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --mode components-json docs/examples/kittui-md-proof.md | rg '"schema_version": 1|"components"|"kind": "H1"'` passed.
  - `cargo run -q -p kittui-cli --bin kittui-md -- --mode --stats-json docs/examples/kittui-md-proof.md | rg '"mode": "stats-json"|"counts"'` passed.
  - `cargo build -p kittui-cli --bin kittui-md` passed.
  - `git diff --check` passed.
- Context: `--mode` accepts names with or without `--`, including aliases such as `widgets`; unknown names and conflicts report clear errors.

## Diff summary

- Code/content commits: `af9ec1d`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittui_md.rs`, `README.md`
- Tests: added parser coverage for canonical names, flag names, aliases, unknown selectors, and selector/direct-flag conflicts.
- Behavioural delta: every existing kittui-md output mode is now selectable through `--mode <name>` as well as its dedicated flag.

## Operator-takeaway

kittui-md's growing output surface is easier to drive programmatically because discovered mode names can be fed directly back through `--mode`.
