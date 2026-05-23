# kittui / kittwm docs map

This directory contains design notes, runtime references, and implementation
plans for kittui and kittwm.

## Core kittwm docs

- [`wm.md`](wm.md) — native kittwm runtime, socket commands, pane lifecycle,
  renderer modes, transport controls, and operator-facing behavior.
- [`kittwm-sdk-plan.md`](kittwm-sdk-plan.md) — long-range SDK/surface runtime
  architecture: surfaces, windows, first-party apps, events, capabilities, and
  renderer split.
- [`protocol-conformance.md`](protocol-conformance.md) — terminal/protocol
  behavior notes and conformance tracking.

## Graphics transport and frame performance

- [`adaptive-graphics-transport.md`](adaptive-graphics-transport.md) — transport
  selection plan for direct kitty streams, zlib, file/tempfile/shared-memory,
  tmux safety, local/remote detection, and diagnostics.
- [`kitty-response-probing.md`](kitty-response-probing.md) — status and design
  notes for opt-in kitty terminal response reading and `a=q` capability probing
  without blocking render loops or consuming app input.
- [`kittwm-dirty-frame-updates.md`](kittwm-dirty-frame-updates.md) — dirty-grid
  frame diff model, safe unchanged-frame skipping, and why partial/overlay kitty
  updates remain experimental.

Current implementation status:

- tmux defaults to pure terminal rendering unless explicitly overridden;
- direct raw RGBA uploads support zlib and threshold-based `auto` compression;
- file/tempfile and shared-memory raw-frame grammar/path exist with safe fallback;
- dirty-grid unchanged-frame skipping and dirty-frame status metrics exist;
- typed kitty animation/frame control helpers exist for future experiments;
- pure `a=q` query encoder/parser helpers, a bounded response reader, and
  opt-in `kittwm doctor --probe-kitty` / `KITTUI_KITTY_PROBE=1` diagnostics
  exist, while normal rendering remains non-probing by default.

## Semantic surface docs

Semantic surfaces let kittwm represent labels, buttons, inputs, selection state,
focus, actions, and events as structured component trees instead of pixels only.

Read in this order:

1. [`kittwm-semantic-surfaces.md`](kittwm-semantic-surfaces.md) — protocol and
   architecture plan: component tree model, roles, values, layout, actions,
   events, renderer mapping, capabilities, and adapter sources.
2. [`kittwm-semantic-quickstart.md`](kittwm-semantic-quickstart.md) — runnable
   workflow: print/publish/read semantic snapshots, use SDK/CLI helpers, inspect
   fallback PTY snapshots, and understand current action behavior.
3. [`kittwm-browser-semantic-adapter.md`](kittwm-browser-semantic-adapter.md) —
   browser DOM/ARIA adapter design and current browser implementation status.
4. [`kittwm-accessibility-semantic-adapter.md`](kittwm-accessibility-semantic-adapter.md)
   — platform accessibility-tree adapter plan for macOS AX / Linux AT-SPI style
   semantics.

Current semantic implementation status:

- `kittwm-sdk` owns public semantic protocol types, first-class roles for common
  document/browser/accessibility structures, snapshot/action/focus/publish
  wrappers, typed semantic events, and convenience action helpers.
- Native kittwm socket exposes `SEMANTIC_SNAPSHOT`, `SEMANTIC_PUBLISH`,
  `SEMANTIC_ACTION`, and `SEMANTIC_FOCUS`.
- Terminal panes expose a fallback semantic text-area tree when no published
  snapshot exists.
- Published semantic snapshots support in-memory focus, toggle, set/insert text,
  set number/bool, and select actions.
- Semantic publish/focus/action/value changes and native surface side effects
  (title, bell, OSC52 clipboard set, notification) appear in the native bounded
  `EVENTS [ms]` stream and parse as typed SDK events, including pane lifecycle,
  `pane_resized`, `pane_input_sent`, focus/layout, semantic, and side-effect variants; SDK callers
  can collect events as a batch or iterate the bounded batch with
  `KittwmEventIter`.
- `kittui-affordances` owns the high-level form/control builders and gallery;
  `kittui-core` remains primitive-only.
- `kittui-wm` can render both internal and public SDK semantic snapshots to
  primitive kittui scenes via shared affordance controls.
- `kittwm-browser` can extract DOM/ARIA semantic snapshots, print one-shot
  snapshots with `--semantic-snapshot` / `--print-semantic`, best-effort publish
  changed snapshots when running with `KITTWM_SOCKET`/`KITTWM_WINDOW`, and route
  supported browser semantic actions through DevTools/DOM.
- Accessibility-tree adapter foundations have landed: the safe adapter core has
  macOS AX and Linux AT-SPI-style node mapping, redaction/action descriptors,
  permission/unavailable diagnostics, platform-neutral action routing through an
  `AccessibilityActionBackend` trait, and browser/accessibility semantic role
  remaps use first-class SDK roles where available. Direct macOS AX / Linux
  AT-SPI platform bindings remain follow-ups.

## Examples and artifacts

- [`examples/`](examples/) — docs/examples assets and proof inputs.
- `crates/kittui-cli/examples/kittwm_semantic_app.rs` — synthetic semantic SDK
  app that prints, queries, and publishes a settings/form component tree.
- `crates/kittui-cli/examples/kittwm_composite_app.rs` — composite SDK example
  spanning terminal plus browser/placeholder surfaces.
- `kittwm-terminal` — first-party SDK terminal client for spawn/replace plus
  typed `--status` and `--events-ms` inspection.
- `kittwm-sdk` automation helpers now include screen text, scrollback,
  visible-text waits, screen-plus-scrollback output waits, typed wait-match
  results, exact byte sends, bracketed paste byte payloads, and typed
  `MouseEvent` injection through `SurfaceHandle` methods; event helpers expose
  bounded iterators and common envelope/detail accessors; control helpers cover
  focus cycling, layout, balance, and pane movement; `NativePaneDetail` has
  convenience accessors for bounds/cursor/modes/dirty/transport status; local
  capability presets cover none/restricted/inspection/automation scopes; SDK
  introspection includes typed `HELP_JSON` catalog helpers. Remaining SDK/runtime
  gaps are mostly stable frame-capture/present surfaces, resize/input/frame
  event modeling, clipboard read policy, and runtime-issued credentials.
- `kittwm-launch` — first-party SDK launcher with backend planning, typed app
  discovery helpers, URL/browser auto-selection through `kittwm-browser`,
  `--dry-run`, and `--status`.
- `kittwm-sdk::SurfaceSpec::browser(...)` — browser surface requests currently
  spawn the first-party `kittwm-browser` app through the PTY transport; dedicated
  browser/X/Quartz surface protocols remain future work.
- `crates/kittui-affordances/examples/control_gallery.rs` — first-party control
  palette summary over affordance builders.
