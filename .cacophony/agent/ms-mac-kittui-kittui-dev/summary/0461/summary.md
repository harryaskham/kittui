# Session summary — NativeSurface exact-byte input hook

## Goal

Add a common exact-byte input hook to the `NativeSurface` abstraction while preserving PTY raw byte behavior and explicit unsupported errors elsewhere.

## Bead(s)

- `bd-3b80da` — kittui-wm: add NativeSurface exact-byte input hook

## Before state

- Failing tests: none known.
- Relevant context: `NativeSurface` had text input only, while PTY surfaces and kittwm socket automation already support exact bytes.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-wm native_surface_exact_byte_hook -- --nocapture` passed.
  - `cargo test -p kittui-wm byte_hook -- --nocapture` passed.
  - `git diff --check` passed.
- Context:
  - Added `NativeSurface::send_surface_bytes(&mut self, bytes: &[u8]) -> Result<()>`.
  - Default implementation delegates UTF-8 bytes to `send_surface_text` and rejects non-UTF8 bytes explicitly.
  - `PtyTerminalApp` overrides it and delegates to `TerminalSurface::send_bytes` for raw/exact bytes.
  - Capture-only adapters retain unsupported behavior through their text path.
  - Added tests for PTY exact bytes, default non-UTF8 rejection, and capture-only unsupported text input.
  - No daemon/socket behavior changed.

## Parallel coordination

- `kittui-dev-2` landed docs-only `bd-69359f` at `4cf77ad`, documenting `NativeSurface::take_surface_events`.

## Diff summary

- Code/content commit: `10364442`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`

## Operator-takeaway

The common NativeSurface abstraction can now represent exact-byte input where supported, closing another gap between runtime surfaces and socket automation semantics.
