# Session summary — Triple-Ctrl-C exit guard

## Goal

Operator safety: single Ctrl-C should pass through to the focused window (so embedded shells still get SIGINT-like behaviour), but three Ctrl-C presses within one second exit the WM cleanly — a panic kill-switch for when an embedded app eats keys.

## Bead(s)

- `bd-2776ad` — Triple-Ctrl-C exit guard; single Ctrl-C passes through
- parent: `bd-031e54` — kittui-wm v2

## Before state

- Failing tests: none.
- Context: single Ctrl-C was already forwarded to the focused window via `compositor.route_key`, but there was no triple-press exit and the footer simply read `q to quit` even when an embedded app was hosted.

## After state

- Failing tests: none. `cargo test --workspace --lib --bins --tests --features sck -- --test-threads=2` is green; +4 new tests in `session::ctrl_c_guard_tests` (single-press, three-within-window, decay outside window, footer hint switch).
- Relevant metrics: WM run-loop input drain now installs a `CtrlCGuard` (`VecDeque<Instant>`, 1-second window, trigger=3). First Ctrl-C: forwarded + logged + returned to the input loop. Third within the window: `quit=true`, terminal restore via the existing RAII guard.
- Context: footer string now switches from `q to quit` to `q or Ctrl-C×3 to quit` whenever the WM is actually hosting a window (`last_window_count > 0`).

## Diff summary

- Code/content commits: pending final squash SHA from reintegration receipt
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-cli/src/session.rs` (CtrlCGuard struct + record_press + quit_hint, input-drain Ctrl-C interception, footer string)
- Tests: +4 new, 0 skipped, 0 removed.
- Behavioural delta: visible footer text changes when hosting; triple Ctrl-C within 1s exits cleanly; single Ctrl-C still forwards.

## Embedded artefacts

- `screenshots/bd-2776ad-ctrl-c-footer.png` — full-display tendril capture of pane 2 with kittwm running; pane content confirmed via tmux-cli capture: `... — 2 windows — 30 fps (peak 31, cap 60) — q or Ctrl-C×3 to quit (log: /tmp/kittui-wm.log)`.

## Operator-takeaway

`kittwm` now has the same kill-switch ergonomic as tmux/vim — embedded apps see single Ctrl-C, but the WM itself is recoverable with three fast presses. Footer is honest about it whenever a window is actually being hosted.
