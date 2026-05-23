# Session summary — inline affordance CLI commands

## Goal

Expose existing `kittui-affordances` inline chrome patterns through the `kittui` CLI so shell scripts and kittwm chrome preview workflows can generate chips, dividers, and title bars without Rust/FFI code.

## Bead(s)

- `bd-941d19` — kittui-cli: expose inline affordance chrome commands

## Before state

- Failing tests: none known.
- Relevant gap: `kittui-affordances` had `chip_chrome`, `divider_chrome`, and `title_chrome`, but the CLI only exposed panel among the higher-level affordance commands.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-cli --bin kittui inline_affordance -- --nocapture` passed.
  - `cargo test -p kittui-cli --test inline_affordance_commands -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittui` passed.
  - `git diff --check` passed.
- Context: added `kittui chip --bg COLOR --border COLOR -w W [-h H]`, `kittui divider --left COLOR --right COLOR -w W`, and `kittui title-bar --left COLOR --right COLOR -w W [-h H]`. Each command builds scenes through the shared affordance chrome helpers and supports existing emit modes including `--scene-json`, `--json`, dry-run, and channel filters.

## Diff summary

- Code/content commit: `d0176b5`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/main.rs`, `crates/kittui-cli/tests/inline_affordance_commands.rs`, `README.md`, `DESIGN.md`
- Behavioural delta: shell users can generate common inline chrome affordance scenes directly from the CLI.

## Operator-takeaway

The CLI now exposes more of the reusable chrome layer needed for scriptable kittui/kittwm UI construction.
