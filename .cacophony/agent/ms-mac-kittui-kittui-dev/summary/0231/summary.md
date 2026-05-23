# Session summary — guard high-level placement on terminal support

## Goal

Make detected terminal capabilities meaningful by preventing high-level kittui scene placement from emitting kitty graphics bytes when the configured terminal descriptor says kitty graphics or unicode placeholders are unsupported.

## Bead(s)

- `bd-731ec7` — kittui: guard high-level placement on terminal capability support

## Before state

- Failing tests: none known.
- Relevant gap: `TerminalInfo` could say `supports_kitty=false` or `supports_unicode_placeholders=false`, but `Runtime::place` still rendered/uploaded and emitted kitty escape sequences/placeholders.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui unsupported_terminal -- --nocapture` passed.
  - `cargo test -p kittui supported_terminal_override -- --nocapture` passed.
  - `cargo build -p kittui` passed.
  - `git diff --check` passed.
- Context: high-level `Runtime::place` / `place_at` now return `KittuiError::UnsupportedTerminal` before rendering when kitty graphics or unicode placeholders are disabled. Supported explicit terminal overrides still work.

## Diff summary

- Code/content commit: `a3e591b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`
- Behavioural delta: hosts get an explicit error instead of invalid terminal output on unsupported terminals.

## Operator-takeaway

kittui is now safer as a library and script renderer: capability probing is enforced at placement time instead of being metadata only.
