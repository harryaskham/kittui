# Session summary — kittwm clipboard JSON CLI wrapper

## Goal

Expose runtime `CLIPBOARD_JSON` through a stable `kittwm` CLI flag so users do not need raw `--attach -c` protocol strings.

## Bead(s)

- `bd-c431b0` — kittwm: CLI wrapper for cached clipboard JSON

## Before state

- Failing tests: none known.
- Relevant context: runtime `CLIPBOARD_JSON` and docs existed, but the CLI had no direct wrapper.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --bin kittwm normalize_daemon_command_preserves_json_inspection_verbs -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `--clipboard-json`, mapped directly to native socket `CLIPBOARD_JSON`.
  - Help text notes cache-only/default-deny behavior and `KITTWM_CLIPBOARD_READ=allow`.
  - Extended JSON inspection verb normalization test for `clipboard_json`.
  - No daemon policy or SDK behavior changed.

## Parallel coordination

- Assigned `bd-e7240d` to `kittui-dev-2` because they were already fixing the nix PTY shell resolver issue.
- `kittui-dev-2` also retains `bd-d582b7` for typed SDK `PaneFramePresented` docs/parsing.

## Diff summary

- Code/content commit: `1eb67d90`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/bin/kittwm.rs`

## Operator-takeaway

The clipboard read-policy surface is now reachable through a first-party CLI wrapper, consistent with the project preference for stable wrappers over raw protocol strings.
