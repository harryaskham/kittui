# Session summary — Native pane status convenience accessors

## Goal

Implement bd-8b1d5b by adding pure additive `kittwm-sdk` convenience methods on `NativePaneDetail` for common status/mode fields, without changing daemon behavior or existing JSON shapes.

## Bead(s)

- `bd-8b1d5b` — kittwm-sdk: native pane status convenience accessors

## Before state

- Failing tests: none known for this bead.
- Relevant metrics: `NativePaneDetail` exposed rich optional fields for pane/app bounds, cursor state, terminal modes, dirty-frame metrics, and transport diagnostics, but SDK consumers had to inspect raw `Option` fields directly.
- Context: kittui-dev took docs for the control helpers, so this slice stayed narrowly in additive SDK ergonomics.

## After state

- Failing tests: none observed in targeted validation.
- Relevant metrics: `NativePaneDetail` now has `bounds`, `app_bounds`, `cursor_position`, `is_cursor_visible`, `has_bracketed_paste`, `has_application_cursor_keys`, `has_mouse_reporting`, `has_mouse_button_motion`, `has_mouse_all_motion`, `has_mouse_sgr`, `has_dirty_frame`, and `has_transport_diagnostics`.
- Context: methods are pure accessors over existing decoded fields; no daemon behavior or protocol changed.

## Diff summary

- Code/content commits: `956d86f` (`bd-8b1d5b: add native pane status accessors`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittwm-sdk/src/lib.rs`
- Tests: existing rich status fixture extended with accessor assertions
- Behavioural delta: SDK consumers can read common pane status and mode data without repetitive `Option` plumbing.
- Validation: `cargo test -p kittwm-sdk native_pane_detail -- --test-threads=1`; `cargo check -p kittwm-sdk`; `git diff --check`.

## Operator-takeaway

Native pane status remains the same on the wire, but the SDK is easier to consume: common geometry, cursor, mouse, paste, dirty-frame, and transport checks are now simple typed methods.
