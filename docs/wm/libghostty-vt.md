# Portable libghostty-vt integration notes

`kittui-ghostty-vt` is a narrow proof crate for using Ghostty terminal logic from kittui/kittwm without depending on the macOS `Ghostty.app` bundle.

## What is wired

- Dependency source: `nixpkgs#libghostty-vt`.
- Build discovery: Cargo `build.rs` uses `pkg-config` for `libghostty-vt`.
- Nix flake wiring: the dev shell and Rust package builds include `pkg-config` and `libghostty-vt`.
- Rust proof surface: `GhosttyVtTerminal` can create a terminal, feed VT bytes, and format a text/style snapshot using libghostty-vt's formatter API.

This is portable across platforms supported by nixpkgs `libghostty-vt`; it does not link to AppKit, Metal, Swift, or Ghostty application symbols.

## What libghostty-vt gives us

The packaged headers expose:

- full terminal state (`ghostty_terminal_*`),
- formatter output (`ghostty_formatter_*`) for plain/VT/HTML snapshots,
- render-state iteration (`ghostty_render_state_*`) for row/cell/style/color data,
- kitty graphics storage and placement metadata (`ghostty_kitty_graphics_*`),
- input encoders for keyboard/mouse/focus events.

That is enough to avoid reimplementing VT100/xterm parsing in kittwm's terminal app path.

## What it does not give us yet

`libghostty-vt` is the VT/state library, not the full Ghostty GUI renderer. It does not directly expose a stable headless pixel canvas API. A robust screenshot pipeline should either:

1. build a small kittui renderer over libghostty-vt render-state rows/cells plus kitty image placement metadata, or
2. upstream/consume a future Ghostty headless surface API if one becomes available.

The first option is immediately portable and keeps kittwm in control of compositing. The second option would be closer to “virtual Ghostty canvas over time” but depends on upstream API stability.

## Proposed next steps

1. Extend `kittui-ghostty-vt` from formatter proof to render-state extraction: rows, cells, graphemes, fg/bg colors, cursor state, dirty rows.
2. Add a CPU raster adapter that converts render-state snapshots into a kittui `Scene` or RGBA frame.
3. Teach a `kittui-ghostty` utility to run a child PTY, feed output into libghostty-vt, and emit frame PNG/manifest sequences for deterministic screenshot evidence.
4. Wrap that utility as a first-party kittwm native app surface so libghostty-vt owns terminal emulation while kittwm owns IO forwarding, layout, decoration, and kitty/kittui compositing.

This keeps the architecture clean: libghostty-vt handles terminal semantics, kittui handles rendering primitives/frames, and kittwm handles surface lifecycle, placement, decorations, and control-plane contracts.
