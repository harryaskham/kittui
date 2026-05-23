# Session summary — DEC auto-wrap mode in native PTY

## Goal

Improve native kittwm terminal fidelity by honoring DEC auto-wrap mode for fixed-width terminal drawing.

## Bead(s)

- `bd-861eaf` — kittwm: honor DEC auto-wrap mode in native PTY

## Before state

- Failing tests: none known.
- Relevant gap: native kittwm wrapped when printable output advanced past the right edge regardless of DEC auto-wrap mode. Apps that disable wrapping while drawing status cells/regions could corrupt the next line and `READ_TEXT` snapshots.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-wm terminal_state_honors_dec_autowrap_mode -- --nocapture` passed.
  - `cargo test -p kittui-wm terminal_state_honors_scroll_region -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context: `TerminalState` now tracks `auto_wrap`, defaults it on, and handles DEC private `?7h` / `?7l`. When disabled, printable characters at/past the last column clamp/overwrite at the last column instead of wrapping. Default wrap behavior is preserved. docs/wm now mentions autowrap mode fidelity.

## Diff summary

- Code/content commit: `a9c400a`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-wm/src/native.rs`, `docs/wm.md`
- Behavioural delta: native pane rendering/snapshots better match terminal apps that temporarily disable right-margin wrapping.

## Operator-takeaway

Native kittwm now honors another common DEC terminal mode used by TUIs/status renderers.
