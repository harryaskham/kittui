# Session summary — RuntimeBuilder terminal detection

## Goal

Make the kittui Rust library default path as reliable as the CLI for shell/external-platform use by detecting terminal transport/capabilities when callers do not provide an explicit `TerminalInfo`.

## Bead(s)

- `bd-b986b1` — kittui: default RuntimeBuilder to detected terminal info

## Before state

- Failing tests: none known.
- Relevant gap: `RuntimeBuilder::build()` used `TerminalInfo::default_kitty()` via `unwrap_or_default()`, so Rust/library callers did not get environment-driven transport detection unless they manually supplied a terminal descriptor.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui runtime_builder -- --nocapture` passed.
  - `cargo build -p kittui` passed.
  - `git diff --check` passed.
- Context: `RuntimeBuilder::build()` now uses `TerminalInfo::detect()` by default. Explicit `.terminal(...)` overrides still win.

## Diff summary

- Code/content commit: `5f2210e`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`
- Behavioural delta: default Rust library runtimes now pick up tmux passthrough and other detected terminal capability details automatically.

## Operator-takeaway

This closes another library/platform gap: external Rust hosts no longer need to duplicate CLI detection just to get correct transport defaults.
