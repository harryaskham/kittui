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

1. Replace the interim pseudo-glyph preview renderer with render-state extraction: rows, cells, graphemes, fg/bg colors, cursor state, dirty rows.
2. Add a CPU raster adapter that converts render-state snapshots into a kittui `Scene` or RGBA frame with real cell colors/styles and kitty placement metadata.
3. Teach a `kittui-ghostty` utility to run a child PTY, feed output into libghostty-vt, and emit frame PNG/manifest sequences for deterministic screenshot evidence.
4. Wrap that utility as a first-party kittwm native app surface so libghostty-vt owns terminal emulation while kittwm owns IO forwarding, layout, decoration, and kitty/kittui compositing.

This keeps the architecture clean: libghostty-vt handles terminal semantics, kittui handles rendering primitives/frames, and kittwm handles surface lifecycle, placement, decorations, and control-plane contracts.


## Interim headless preview

`snapshot_preview_png` and `examples/headless_preview.rs` emit a deterministic PNG from the libghostty-vt formatter snapshot. This uses a tiny bundled bitmap font and avoids platform text APIs, so it is suitable as a first CI/headless proof artifact. It proves Ghostty-owned VT state can drive an image artifact without desktop capture. It is not yet pixel-identical to Ghostty because it does not consume render-state cell style/color data.


## Render-state extraction

`GhosttyVtTerminal::render_snapshot` now updates a `GhosttyRenderState` and extracts rows/cells/graphemes plus resolved foreground/background colors where libghostty-vt reports them. `render_snapshot_preview_png` uses those cells for a deterministic PNG artifact. This moves the proof beyond formatter text while staying portable and independent of desktop capture.

The preview is still a kittui-owned bitmap-font renderer. It is not intended to match Ghostty's GPU text shaping pixel-for-pixel yet. The next step is to carry more render-state metadata (style flags, cursor style, dirty rows, kitty image placements) and map that to kittui scene/RGBA primitives.


## Style extraction

`GhosttyCellSnapshot` now carries basic style flags from libghostty-vt render-state cells: bold, italic, and underline. The headless preview visualizes these in a simple portable way (bold overdraw/brightening, italic hint, underline stroke). This confirms style metadata is available through the portable VT layer and can be mapped into kittui rendering without using the Ghostty GUI application.


## Timelapse artifacts

`examples/headless_timelapse.rs` demonstrates deterministic frame-sequence evidence. It feeds VT bytes into one libghostty-vt terminal over several steps, extracts render-state after each step, writes PNG frames, and emits a small manifest. This gives us a portable substitute for flaky desktop screenshots while we build toward a full `kittui-ghostty` terminal-surface runner.


## `kittui-ghostty` CLI

The `kittui-ghostty` binary in `kittui-ghostty-vt` is a portable headless preview utility. It reads VT bytes from stdin (or `--demo` content), feeds libghostty-vt, extracts render-state cells/styles, and writes a PNG via the kittui-owned preview renderer:

```sh
printf 'hello\n\033[32mgreen\033[0m\n' | kittui-ghostty --out /tmp/frame.png --cols 64 --rows 12
```

This is the first reusable command-line path for deterministic headless Ghostty evidence.
