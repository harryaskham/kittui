# Session summary — real kittui probe capabilities

## Goal

Make `kittui probe` useful for shell/external-platform hosts by reporting the actual detected terminal descriptor instead of hard-coded kitty support booleans.

## Bead(s)

- `bd-20d3e8` — kittui-cli: make probe report detected terminal capabilities

## Before state

- Failing tests: none known.
- Relevant gap: `kittui probe` always emitted `supports_kitty: true` and `supports_unicode_placeholders: true`, regardless of `TerminalInfo::detect()`.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui probe_payload -- --nocapture` passed.
  - `cargo run -q -p kittui-cli --bin kittui -- --terminal-cols 123 --terminal-rows 45 probe --json | python3 -c '...'` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: `probe` now includes top-level `supports_kitty`, `supports_unicode_placeholders`, `transport`, `columns`, `rows`, `cell_size`, and a nested `terminal` object matching `TerminalInfo`, while preserving renderer/version/config provenance and `--force` invalidation.

## Diff summary

- Code/content commit: `900c9e2`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`
- Behavioural delta: probe output follows detected terminal capabilities and CLI terminal-size overrides.

## Operator-takeaway

External callers can now rely on `kittui probe` as the first step in deciding whether to use kitty/unicode placeholders, tmux passthrough, and what terminal dimensions to target.
