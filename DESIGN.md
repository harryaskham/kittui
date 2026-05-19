# kittui — design

`kittui` is a Rust-native renderer for terminal graphics via the kitty graphics
protocol. It is structured as a small set of crates so it can ship as:

- a Rust crate (`kittui`),
- a C ABI shared library / staticlib (`kittui-ffi`, producing `libkittui.so` /
  `.dylib` / `.dll` with `kittui.h`),
- a standalone binary (`kittui` from `kittui-cli`) for shell and script use,
- a ratatui adapter (`ratakittui`) that decorates and lifecycle-binds widgets.

It is the eventual substrate for a terminal window manager, so the design is
careful to keep the renderer agnostic of any UI toolkit and to avoid the kinds
of inefficient redraws that the existing `pi-graphics` JS implementation in
`agent-utils` falls into. The throughput target is **60+ fps for cached frames
and incremental updates** on a typical laptop terminal.

## Performance principles (carried forward from lessons learned)

These are the non-negotiables that everything below is shaped to support.

1. **Content-addressed image ids.** A scene's kitty image id is derived from a
   stable hash of its rasterized contents (after normalization). Identical
   scenes never produce a second upload anywhere in the system.

2. **No animation re-uploads.** Animated scenes are decomposed into N frames,
   each uploaded **once** with `a=t, m=1` and chained via kitty's native
   animation control. The renderer asserts that frame 0 and frame N agree at
   phase 1.0, so loops are perfect by construction. The phase curve
   (`Linear`, `EaseInOut`, `Pulse`, `Custom`) is part of the scene; the
   renderer never spins re-rasterizing.

3. **Diff-driven placements.** A `Composition` is a set of scenes the host
   wants on-screen this frame. `Composition::diff(previous)` returns the
   minimal set of uploads, placements, and deletions. Hosts (ratakittui, CLI,
   FFI consumers) drive the system by mutating compositions, not by calling
   draw primitives.

4. **Single allocator-friendly hot path.** Scene → raster goes through one
   render encoder with reusable scratch buffers. No per-call allocations in the
   inner loop. The CPU renderer uses a tile-based approach; the GPU renderer
   uses a single render pass with batched SDF shaders.

5. **Zero work on cache hit.** A `Runtime::place(scene)` call that hits the
   cache does no rasterization at all; it only re-emits placement escapes if
   the image id is not currently placed at the target footprint.

6. **Decoupled UI lifecycle.** The renderer holds no opinions on widget trees.
   `ratakittui` bridges to ratatui in a separate crate. The Rust library, CLI
   and FFI can be used without ratatui at all.

7. **One renderer, configurable backend.** A single fat `libkittui` ships with
   the `wgpu` GPU backend linked in by default; the CPU renderer is the
   reference oracle and is always available. Backend selection is by env / API
   override; CPU-only slim builds are a feature flag to add later, not a
   separate crate.

## Workspace layout

```
kittui/
├── Cargo.toml                     # workspace
├── DESIGN.md
├── README.md
├── crates/
│   ├── kittui-core/               # Scene types, geometry, color, hashing, animation
│   ├── kittui-render-cpu/         # CPU renderer (parity oracle, no GPU deps)
│   ├── kittui-render-gpu/         # wgpu renderer (feature: gpu, default on)
│   ├── kittui-kitty/              # kitty graphics protocol + transports
│   ├── kittui-cache/              # content-addressed PNG/APNG cache
│   ├── kittui/                    # Public Rust facade (Runtime, builders)
│   ├── kittui-cli/                # `kittui` binary
│   ├── kittui-ffi/                # cdylib + cbindgen header
│   └── ratakittui/                # ratatui adapter (decoration + lifecycle)
└── xtask/                         # cbindgen, fixtures, abi snapshots
```

## Scene model

```rust
pub struct Scene {
    pub footprint: CellRect,       // columns × rows
    pub cell_size: CellSize,       // pixels per cell (terminal-reported or override)
    pub layers: Vec<Layer>,        // back-to-front
    pub animation: Option<Animation>,
}

pub enum Node {
    Rect      { rect: Px, fill: Paint, stroke: Option<Stroke>, corners: Corners },
    Gradient  { rect: Px, stops: Vec<Stop>, direction: Direction },
    Glow      { center: Px, radius_px: f32, color: Rgba, intensity: f32 },
    Scanlines { rect: Px, alpha: u8, period_px: u8 },
    Image     { rect: Px, src: ImageRef, fit: Fit, tint: Option<Rgba> },
    Group     { transform: Affine2, opacity: u8, children: Vec<Node> },
    Composite { mode: BlendMode, children: Vec<Node> },
    Mask      { mask: Box<Node>, child: Box<Node> },
    Clip      { rect: Px, child: Box<Node> },
}

pub struct Animation {
    pub frames: u16,               // ≥ 2
    pub cycle_ms: u32,             // full loop duration
    pub curve: PhaseCurve,         // Linear, EaseInOut, Pulse{harmonics}, Custom
    pub loops: u32,                // 0 = forever
}
```

`Scene` is `serde`-serializable. Its content hash (blake3) is the image id
source. JSON form is the cross-language interchange between CLI, FFI and TS.

### Animation = native kitty loop

The renderer expands `Animation` into N frames at phases derived from `curve`.
Frame i is independent; each frame is uploaded once with a frame index and the
kitty protocol's animation table is configured (`a=a, r=...`). The terminal
animates with no further escape traffic. We assert frame loop closure:

```
assert frame_for_phase(0.0) ≡ frame_for_phase(1.0)  (mod ε)
```

so APNG-style perfect looping is mechanical.

## Public crate facade

```rust
let runtime = kittui::Runtime::builder()
    .terminal(TerminalInfo::detect_or_default())
    .cache_dir(paths::default_cache_dir())
    .build()?;

let scene = kittui::scene()
    .footprint_cells(60, 8)
    .layer(layer::background(Paint::linear("#0b1626", "#1a2336", Direction::V)))
    .layer(layer::rect().inset_cells(1, 1).corners(8.0)
        .stroke(Stroke::inside(1.5, Paint::solid("#72fbd6")))
        .fill(Paint::solid("#08111fcc")))
    .layer(layer::glow().center_frac(0.2, 0.5).color("#00d8ffaa").radius_frac(0.5))
    .animation(Animation::pulse(8, 800)) // 8 frames, 800ms cycle
    .build();

let placement = runtime.place(&scene)?;
print!("{}{}", placement.upload_escape(), placement.embed_text("settling"));
```

## CLI

```
kittui box -x 3 -y 5 -w 100% -h 5 \
    --fg "#72fbd6" --bg "#08111fcc" \
    --radius 6 --border 1.5 --border-color "#00d8ff" \
    --glow "#00d8ff:0.6" --scanlines 0.15 \
    --label "settling"

kittui gradient -w 100% -h 1 --left "#00d8ff" --right "#b48cff" --fade
kittui panel --tone assistant -w 60 -h 9 --animate pulse:8@800ms
kittui image --src logo.png -w 24 -h 6 --tint "#00d8ff88"
kittui compose scene.json
kittui place --id 0xABCD --rows 5 --cols 60
kittui cache info | gc | clear
kittui probe
```

All commands accept `--json` for programmatic chaining and `--cache-dir` to
override storage.

## FFI

Opaque handles + JSON-blob scenes. Strings are UTF-8 with explicit lengths.
Errors via out-param structs. Every entry point is `catch_unwind`-wrapped.
ABI versioned via `kittui_abi_version()`; cbindgen snapshot checked into the
repo; CI rejects unannounced changes.

## ratakittui

A separate crate covering full decoration of ratatui primitives. Every widget
that has chrome (border, background, title, footer, scrollbar, chip-style
inline text) gets a `KittuiDecorated<W>` wrapper. Joined borders and shared
backgrounds are computed at scene-composition time via `Composite` + `Mask`
nodes so the underlying renderer stays primitive.

Lifecycle is integrated with `ratatui::Terminal::draw`: kittui placements
participate in the draw cycle, are uploaded only when ids change, and are
deleted when their widgets leave the tree.

## Cache

Layout:

```
<cache>/
├── scenes/<sha[0..2]>/<sha>.png|apng|meta
├── images/<sha>...
└── probe.json
```

LRU by access mtime, default budget 256 MiB. Cross-process cooperative via
per-key lock files. Cache directory selection: builder → CLI flag → FFI call
→ `KITTUI_CACHE_DIR` → XDG default.

## Phasing

1. `kittui-core` + `kittui-render-cpu` + `kittui-cache` + `kittui` facade +
   `kittui-kitty` + `kittui-cli`. Achieves byte-for-byte parity with the
   subset of pi-graphics we use today. Comes with golden tests.
2. `kittui-ffi` + TS wrapper. Pi plugins can adopt.
3. `kittui-render-gpu` (wgpu). Enables 60+ fps interactive ratakittui.
4. `ratakittui` full decoration matrix.
5. `kittui-wm` (long term).

This document is the source of truth. PRs that diverge from it must update it
first.

## Future ideas

- **`kittui-tmux` pane-border showcase.** A tmux plugin / hook that replaces
  the ASCII box-drawing characters tmux paints between panes with kittui
  unicode-placeholder graphics. The outer chrome (pane separators, status
  line, message overlays) is then drawn by kittui with full gradients,
  rounded corners, joined borders, glow, and per-pane tinted backgrounds —
  effectively a graphically enhanced tmux at the outer layer. As a bonus,
  centralizing kitty graphics state at the tmux outer layer gives us a
  consistent place to translate pane-local kitty placements into absolute
  terminal coordinates, fixing the relative-placement-in-tmux problem most
  consumers hit today. This is also a good stress test of the diff-driven
  composition path because tmux pane geometry changes whenever the user
  splits, resizes, swaps, or zooms.
- **`kittui-wm` window manager substrate.** Once the tmux-border showcase
  works, the same primitives drive a from-scratch terminal window manager.
  The renderer/cache/protocol layers are unchanged; only a new host crate is
  added.
- **`kittui-shader` user shaders.** Expose a stable shader interface so users
  can drop in a fragment shader and target a `Node::Shader` rect. With the
  GPU backend the cost is essentially free per draw, and the same shader
  produces a CPU fallback by compiling through naga/wgsl-to-spirv-to-cpu.
