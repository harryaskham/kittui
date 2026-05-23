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
│   ├── kittui/                    # Public Rust facade (Runtime, builders, Composition)
│   ├── kittui-cli/                # `kittui` binary
│   ├── kittui-ffi/                # cdylib + cbindgen header
│   ├── ratakittui/                # ratatui adapter (decoration + lifecycle)
│   ├── kittui-tmux/               # tmux pane-border host
│   ├── kittui-affordances/        # optional higher-level patterns (panel/chip/divider)
│   ├── kittui-wm/                 # window-manager substrate (scaffold)
│   ├── kittui-overlay/            # transient overlay surfaces
│   └── kittui-watch/              # live preview daemon
├── bindings/
│   └── ts/                        # @kittui/koffi (build-step-free JS binding)
└── xtask/                         # cbindgen, fixtures, abi snapshots
```

## Scene model

The `Scene` is the only shape that crosses any boundary in kittui — Rust,
JSON, C ABI, cache key. Everything else is plumbing.

```rust
pub struct Scene {
    pub footprint: CellRect,       // columns × rows
    pub cell_size: CellSize,       // pixels per cell (terminal-reported or override)
    pub layers: Vec<Layer>,        // back-to-front
    pub animation: Option<Animation>,
}

pub enum Node {
    Rect      { rect: PxRect, fill: Paint, stroke: Option<Stroke>, corners: Corners },
    Gradient  { rect: PxRect, stops: Vec<Stop>, direction: Direction },
    Glow      { rect: PxRect, center_x_frac, center_y_frac, radius_frac,
                color: Rgba, intensity: f32 },
    Scanlines { rect: PxRect, alpha: u8, period_px: u8 },
    Image     { rect: PxRect, src: ImageRef, fit: Fit, tint: Option<Rgba> },
    Group     { opacity: f32, children: Vec<Node> },
    Composite { mode: BlendMode, children: Vec<Node> },
    Mask      { mask: Box<Node>, child: Box<Node> },
    Clip      { rect: PxRect, child: Box<Node> },
}
```

### Identity and normalization

`Scene::id()` returns a blake3 digest of the canonical-JSON encoding of the
scene. Object keys are sorted by serde's default; floating-point fields are
written with full precision so semantically equal scenes share an id.

Before hashing, the scene goes through a deterministic *normalization* pass:

- Layers with `Node::Group { opacity: 0 }` or empty `children` are dropped.
- Adjacent `Composite { Normal, … }` children with no intervening blend
  modes are flattened.
- Pixel rects are snapped to subpixel precision (`f32` round-to-nearest at
  1/64 px) to make scenes that differ by floating-point noise alone still
  collide on cache.
- `Stop` lists are clamped to `[0,1]` and stable-sorted by `offset`.
- Stroke `width_px` of zero is replaced by `None`; `Paint::Solid(transparent)`
  becomes `None` where structurally valid.

Normalization happens in `kittui-core` once per scene; the renderer and the
cache see only the post-normal form. The blake3 hash is truncated to 32 bits
to produce the kitty image id; collisions are bounded by the content-address
guarantee and the local registry's eviction-on-conflict pass.

### Builders

`kittui::scene` provides primitive builders (`background_solid`,
`background_linear`, `rounded_rect`, `glow_layer`). The library intentionally
does *not* ship higher-level affordances like "panel" or "chip" — those
belong in consumer code (the CLI, the showcase, ratakittui). The library
stays small and orthogonal.

JSON is the cross-language interchange form. Every `Node` variant is
`#[serde(tag = "kind", rename_all = "snake_case")]`. The schema is part of
the stable surface; breaking it requires a major version bump and an
`xtask/schema-snapshot` update.

### Animation = native kitty loop

`Animation` is declarative: it carries `frames`, `cycle_ms`, a `PhaseCurve`,
and a `loops` count. The renderer expands the curve into per-frame phases
and rasterizes each frame exactly once. The kitty layer uploads frames in
order with `a=t, r=…` and configures playback with `a=a, i=…, s=loops, c=N`.
After the last upload the terminal animates indefinitely with no further
escape traffic.

`PhaseCurve::closes_loop()` is invoked at scene construction; animations
whose first and last phases don't agree are rejected at `Animation::pulse(…)`
builder time and at FFI ingestion. `Pulse` and `Custom` curves with matching
endpoints pass; raw `Linear` and `EaseInOut` are accepted only when wrapping
code clamps them appropriately (typically by sampling `[0..1]` and folding
back).

The contract is:

```
assert frame_for_phase(0.0) ≡ frame_for_phase(1.0)  (mod ε)
```

so APNG-style perfect looping is mechanical.

### Cell-size & HiDPI

`CellSize { width_px, height_px }` is supplied by the host (via
`TerminalInfo`) or defaulted to `8 × 16`. Scenes record their cell size at
construction time; the cache key therefore changes when the host switches
to a HiDPI cell metric, so HiDPI re-renders happen automatically without
explicit invalidation. The renderer always rasterizes at the recorded
pixel size; kitty resamples to the cell footprint.

## Public crate facade

```rust
let runtime = kittui::Runtime::builder()
    .terminal(TerminalInfo::detect_or_default())
    .cache_dir(paths::default_cache_dir())
    .renderer(RendererKind::Auto)
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
print!("{}{}{}", placement.upload, placement.placement, placement.embed);
```

`Runtime::place` is the only entry point most users need. It walks the
diagram: normalize → hash → look up cache → render-if-miss → store →
upload-if-not-already-placed → emit placement escape → return embeddable
text. Every step in the diagram has an early-out for cached state, so the
expected cost of a re-place is a single hash and a small string format.

A `Composition` higher-level API (proposed) lets hosts declare an entire
frame's set of scenes at once and receive a diff against the previous
frame; ratakittui's `LifecycleTracker` already implements this pattern in
miniature.

## kitty graphics protocol (`kittui-kitty`)

`kittui-kitty` is the only crate that knows about escape sequences. Its
surface is six functions and one enum:

```rust
pub enum Transport {
    Direct, TmuxPassthrough, File, Memory,
}

pub fn upload_still(image_id: u32, png: &[u8], transport: Transport) -> String;
pub fn upload_animation(image_id: u32, frames: &[Vec<u8>],
                        frame_delays_ms: &[u32], loops: u32,
                        transport: Transport) -> String;
pub fn placement_command(image_id: u32, footprint: CellRect,
                         transport: Transport) -> String;
pub fn placeholder_text(image_id: u32, footprint: CellRect) -> String;
pub fn delete(image_id: u32, transport: Transport) -> String;
```

### Chunking and base64

Payloads are base64-encoded and chunked at 4 KiB to fit kitty's documented
escape size limits. The first chunk carries action+format+id (`a=t,f=100,i=…`);
subsequent chunks carry only the continuation marker. The terminator
`\x1b\\` separates chunks. Every chunk goes through `wrap_transport` so
tmux passthrough is applied uniformly.

### Frame indexing and animation control

Animation uploads use `r=` to index frames within an image id. Once all
frames are uploaded, kittui emits one *animation control* escape
(`a=a, i=…, s=loops, c=count`) and one *per-frame delay* escape
(`a=a, i=…, r=frame, z=delay`) per frame. After this sequence the terminal
plays the loop autonomously; no further escape traffic is required for the
lifetime of the animation. This is the principal performance guarantee for
animated chrome — a thousand pulsing panels cost the same as one.

### Unicode placeholders

Placement uses the documented `U=1` form. `placeholder_text` builds a grid
of `\u{10EEEE}` cells with the image id encoded into the foreground
truecolor (`\x1b[38:2:r:g:b]…\x1b[39m`). Hosts print this grid at the
placement origin and the terminal anchors the corresponding image cell-by-cell.

### Transport-specific wrappers

- `Direct`: raw `\x1b_G…\x1b\\` escapes.
- `TmuxPassthrough`: wraps each escape with `\x1bPtmux;…\x1b\\` and doubles
  embedded `\x1b` bytes. Picked automatically when `TERM_PROGRAM=tmux` or
  `TMUX` is set, unless the host overrides.
- `File`: writes the PNG to a tempfile and emits `a=t,t=f,…;<path>` —
  useful for very large images on tmux passthrough where chunked base64
  adds overhead.
- `Memory`: shared-memory transfer (`a=t,t=s,…;<shm key>`). The cdylib
  build supports this on Linux/macOS via `memfd_create` / `shm_open`;
  Windows falls back to `File`.

### Transport probing

`TerminalInfo::detect()` inspects `TERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID`,
`KITTY_PUBLIC_KEY`, `TMUX`, and `WT_SESSION` to pick a transport. Hosts
that already know (Pi, ratakittui adapters) can construct `TerminalInfo`
directly and skip probing entirely.

## Renderer architecture

Two backends share the surface:

```rust
pub fn render_still(scene: &Scene) -> Result<RasterFrame, RenderError>;
pub fn render_animation(scene: &Scene) -> Result<RasterAnimation, RenderError>;
```

`RasterFrame::png` is the bytes the kitty layer uploads. `RasterAnimation`
holds one PNG per frame, plus per-frame delays, plus loop count. Both
backends produce 8-bit RGBA PNGs so the kitty protocol path is identical
regardless of which backend rasterized.

### CPU renderer (`kittui-render-cpu`)

The CPU renderer is the parity oracle. It is the simplest possible
correct implementation:

- A reusable `Pixmap` (RGBA8, row-major, top-down) is allocated once and
  cleared between frames within an animation.
- The scene is walked recursively; each node's pixel-space bounding rect
  is clipped against the pixmap and scanned at one sample per pixel.
- Rect / gradient / glow / scanline / stroke shading functions live in
  `rasterize.rs`; each is a pure function from `(node params, sample px)`
  to `Rgba`. No allocations in the inner loop.
- Compositing uses straight-alpha source-over blending to match the legacy
  pi-graphics implementation byte-for-byte where applicable.
- Animation phase enters the renderer through a single `phase: f32`
  argument that nodes read (currently only `Glow` modulates intensity by
  phase; future nodes that pulse must declare which fields they sample).

The CPU renderer is `forbid(unsafe_code)`, has zero direct OS calls, and
runs on every supported target. It is the renderer the FFI defaults to
because it has no shader compilation, no driver dependency, and no
adapter-init failure mode.

### GPU renderer (`kittui-render-gpu`)

The GPU renderer is a wgpu pipeline producing the same `RasterFrame` byte
output as the CPU renderer. Its responsibility is **throughput** —
animated chrome compositing at 60+ fps on a typical laptop — not
quality, which the CPU renderer pins.

Architecture:

- **One render pass per scene.** A single command encoder emits all node
  draws into an offscreen RGBA8 texture sized to the scene's pixel
  footprint. Layer order is preserved by draw order; opacity is a vertex
  attribute.
- **One pipeline per shape family.** Three pipelines cover the v1 node
  set:
  - `rounded_rect_sdf`: fills + strokes + per-corner radii via a signed
    distance function in the fragment shader.
  - `gradient`: linear / horizontal / vertical / diagonal / radial as a
    single shader that switches on a uniform tag.
  - `glow_radial`: smoothstep falloff with phase-aware intensity.
  Scanlines reuse `gradient` with a stripe-period uniform. `Image` uses a
  sampler-only pipeline that reads from a pre-uploaded texture atlas.
- **Instanced draws.** Nodes that share a pipeline are batched into a
  single instanced draw call with per-instance buffers (rect, paint
  parameters, phase). `Composite` / `Mask` / `Clip` introduce render-pass
  barriers only when their semantics demand them (e.g. `Mask` writes to a
  scratch alpha-only texture and reads it back).
- **Animation in one pass.** For animated scenes, frames are produced as
  consecutive viewport renders into the same texture, with `phase` as a
  uniform. Each frame is copied to a CPU-readable buffer and PNG-encoded
  in parallel via `rayon`. No shader work happens after the first cycle —
  the kitty side replays autonomously.
- **Adapter selection.** `wgpu::Instance::request_adapter` is called with
  `HighPerformance`, then `LowPower`, then `software`. The choice is
  cached in `<cache>/probe.json` so subsequent processes skip probing.
- **CPU/GPU parity check.** On first use the GPU renders a canonical
  fixture and diffs it against the CPU renderer with an SSIM tolerance
  of 0.99. Below tolerance the GPU is marked unusable for that host and
  the facade falls back to CPU. Above tolerance the GPU remains primary.

Current implementation note: the checked-in `kittui-render-gpu` backend is
headless/offscreen-first and now exposes explicit adapter options plus adapter
diagnostics. It reuses its offscreen color target and readback buffer across
frames of the same size, which avoids hidden global state while amortising wgpu
resource allocation. The currently unsupported GPU-only semantics are image atlas
nodes, custom shader nodes, and true mask/clip intermediate passes; the public
renderer reports these through `GpuRenderer::unsupported_features()` and callers
should route those scenes through CPU fallback until the dedicated pipelines land.

The GPU renderer has zero `unsafe` in its own code; the only `unsafe` in
this crate is whatever `wgpu` requires transitively. Shader code is WGSL
checked into `crates/kittui-render-gpu/shaders/` and validated at compile
time via `wgpu::ShaderModuleDescriptor`.

### Renderer selection in the facade

`Runtime::builder().renderer(RendererKind::*)`:

- `Cpu`: always CPU. The reference, default for FFI.
- `Gpu`: GPU, fall back to CPU on any error (shader compile, adapter
  request, parity check fail). The error is logged once via `tracing` (if
  the host enabled it) and remembered in the cache probe file.
- `Auto`: try GPU once, fall back to CPU thereafter.

Renderer choice is per-`Runtime`, not per-call. A long-lived host should
build one runtime; a CLI invocation builds and tears down. The GPU path
amortises the wgpu adapter init over many `place()` calls.

### Image inputs

`Node::Image` accepts PNG / JPEG bytes (decoded via `image` crate, behind
the `image-decoders` feature; default on for `kittui-cli` and off for the
slim FFI). The CPU renderer copies pixels directly; the GPU renderer
uploads into a texture in the image atlas. `Fit` (`Contain`, `Cover`,
`Stretch`, `None`) is implemented in both backends with a shared helper
in `kittui-core::geom::fit_into_rect`.

SVG is not in v1. When it lands (`svg` feature, `resvg`-backed) it produces
a rasterized intermediate that is fed to either backend identically.

## Cache (`kittui-cache`)

Content-addressed cache rooted at a directory. Layout:

```
<cache>/
├── scenes/<sha[0..2]>/<sha>.png             # still raster
├── scenes/<sha[0..2]>/<sha>.frames/         # one PNG per animation frame
├── scenes/<sha[0..2]>/<sha>.meta.json       # footprint, frame count, delays, image id, loops
├── images/<sha>...                          # external image inputs (Image node sources)
├── probe.json                               # renderer capability cache
└── locks/<sha>.lock                         # per-key inter-process advisory lock
```

### Concurrency model

Reads are lock-free (the OS guarantees atomicity for whole-file reads on
POSIX + Windows). Writes are atomic: write to `<path>.tmp`, then `rename`.
For multi-file writes (animation frames + meta), all temps are written
first, then renamed in a fixed order so a partial cache is never visible.

Cross-process cooperation uses per-key advisory lock files (`flock` on
POSIX, `LockFileEx` on Windows). A writer takes an exclusive lock, checks
whether another process already produced the entry while it waited, and
either skips or proceeds. This makes the cache safe under concurrent
`kittui` CLI invocations and concurrent FFI consumers in the same user
session.

### Eviction

Eviction is LRU by access mtime (the most recently *read* entry is kept
last). The default budget is 256 MiB; the value is configurable via
`Cache::builder().budget_bytes(…)`, the `KITTUI_CACHE_BUDGET` env var, or
the CLI `--cache-budget`. Eviction runs:

- Synchronously at the end of `put_*` when the cache exceeds budget.
- On `kittui cache gc` invocations.
- On `Runtime::drop` if the runtime was constructed with
  `RuntimeBuilder::gc_on_drop(true)`.

Eviction never touches entries with an active advisory lock (a writer is
in flight). It also leaves entries less than 60 seconds old, to avoid
churn when a session is producing many short-lived scenes.

### Cache directory resolution

Priority order:

1. `RuntimeBuilder::cache_dir(path)` — programmatic.
2. `Cache::open(path)` — direct.
3. CLI `--cache-dir`.
4. FFI `kittui_set_cache_dir`.
5. `KITTUI_CACHE_DIR` env.
6. `XDG_CACHE_HOME/kittui` on Linux.
7. `~/Library/Caches/kittui` on macOS.
8. `%LOCALAPPDATA%\kittui` on Windows.
9. `std::env::temp_dir()/kittui-cache` as last resort.

### Probe cache

`probe.json` records:

```json
{
  "kittui_version": "0.1.0",
  "gpu_adapter": "Apple M2",
  "gpu_parity_ssim": 0.998,
  "gpu_status": "ok" | "fallback" | "unavailable",
  "checked_at": "2026-05-19T02:47:26Z"
}
```

The runtime reads this on construction and uses the status field to skip
the live parity check. The file is invalidated when `kittui_version`
changes or when the host explicitly runs `kittui probe --force`.

## CLI (`kittui-cli`)

The CLI is `clap`-derived and intentionally thin — every subcommand
constructs a `Scene` from its flags and forwards to `Runtime::place`. The
v1 surface mirrors the affordance set the JS pi-graphics implementation
exposes today:

```
kittui box       -x X -y Y -w W -h H --fg COLOR --bg COLOR
                 [--radius R] [--border W] [--border-color C]
                 [--glow COLOR[:INTENSITY]] [--scanlines ALPHA]
                 [--shadow DX,DY,COLOR] [--label TEXT]
                 [--animate pulse:FRAMES@CYCLE_MS[:CURVE]]
kittui gradient  -w W -h H --left COLOR --right COLOR [--direction h|v|d] [--fade]
kittui glow      -w W -h H --color COLOR [--intensity 0..1]
kittui panel     --tone assistant|tool|user -w W -h H
                 [--caption TEXT] [--animate pulse:8@800ms]
kittui image     --src PATH|- -w W -h H [--fit contain|cover|stretch|none] [--tint COLOR]
kittui compose   <scene.json>|-                              # `-` reads Scene JSON from stdin
kittui place     --id 0xID --x X --y Y --cols C --rows R       # re-place a cached id
kittui cache     info | gc [--budget BYTES] | clear
kittui probe     [--force]
kittui scene     emit                                            # print last-built JSON
```

Common flags:

- `--cache-dir PATH` / `KITTUI_CACHE_DIR`.
- `--terminal-cols N` / `--terminal-rows N` (override probing; falls back
  to `$COLUMNS` / `$LINES`).
- `--transport direct|tmux|file|memory`.
- `--renderer cpu|gpu|auto`.
- `--json` for structured output (image id, upload byte count, embed text
  payload as a quoted string).
- `--upload-only` / `--placement-only` / `--embed-only` for scripts that
  buffer their own writes.
- `--scene-json` prints the generated `Scene` JSON for shell pipelines.
- `--dry-run` returns the JSON that `Runtime::place` would have built,
  without rendering or uploading.

Size flags accept `N` (cells), `Npx` (pixels — divided by `cell_size`),
and `N%` (percentage of terminal `cols` / `rows`).

Color flags accept `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, named CSS
colors via the `csscolorparser` feature, and `rgba(r,g,b,a)`.

Animation curves accept `linear`, `ease-in-out`, `pulse[:harmonics]`, and
`custom:f0,f1,…,fN-1`. The CLI validates loop closure before invoking
the runtime.

`--scene-json` plus `compose -` is the integration story for shell pipelines:

```sh
kittui box -w 60 -h 9 --fg '#00d8ff' --bg '#08111fcc' --scene-json > panel.json
jq '.animation = {"frames": 16, "cycle_ms": 800, "curve": {"Pulse": {"harmonics": 0}}, "loops": 0}' panel.json \
  | kittui compose - --dry-run --json
```

## FFI (`kittui-ffi`)

A single C ABI surface for every non-Rust consumer (TS, Python, Lua,
shell via `dlopen`). The crate produces `cdylib` + `staticlib` + `rlib`.

### ABI shape

```c
// Versioning
uint32_t kittui_abi_version(void);                 // (major << 16) | minor

// Runtime lifecycle
typedef struct KittuiRuntime KittuiRuntime;
KittuiRuntime* kittui_runtime_new(const char* cache_dir);
void           kittui_runtime_free(KittuiRuntime*);
KittuiStatus   kittui_runtime_configure(KittuiRuntime*, const char* json);

// Place a scene (JSON blob)
KittuiStatus   kittui_place_json(KittuiRuntime*,
                                 const char* scene_json,
                                 char** out_bytes);     // upload + placement + embed

// Lower-level: render only, no upload
KittuiStatus   kittui_render_json(KittuiRuntime*,
                                  const char* scene_json,
                                  uint8_t** out_png,
                                  size_t*   out_len);

// Unplace by image id
KittuiStatus   kittui_unplace(KittuiRuntime*, uint32_t image_id,
                              char** out_bytes);

// Diagnostics
const char*    kittui_last_error(KittuiRuntime*);
const char*    kittui_probe_json(KittuiRuntime*);

// Memory
void           kittui_string_free(char*);
void           kittui_bytes_free(uint8_t*, size_t);
```

`KittuiStatus` is a non-zero-on-error C enum (`Ok=0`, `NullPointer`,
`BadScene`, `Runtime`, `Panic`). All entry points are `catch_unwind`-wrapped;
a Rust panic returns `Panic` rather than unwinding into the foreign caller.

Strings are UTF-8 with explicit lengths where applicable; the convenience
NUL-terminated variants are also exported because they ease N-API
bindings.

### ABI versioning

- `KITTUI_ABI_MAJOR` and `KITTUI_ABI_MINOR` are compile-time constants;
  `kittui_abi_version()` returns the packed value.
- Breaking changes bump major. Additive changes (new entry points, new
  enum variants past the last documented one) bump minor.
- `xtask abi-snapshot` regenerates the cbindgen header and checks the
  diff against the committed snapshot. CI rejects unannounced changes.
- Consumers should call `kittui_abi_version_check(MAJOR, MINOR)` at load
  time; this is a one-liner that returns nonzero if the loaded library
  is incompatible.

### Threading

`KittuiRuntime` is `Send + Sync`. Internal state uses `parking_lot::Mutex`
for the placed-image map and `parking_lot::RwLock` for the renderer
selection. The wgpu queue is held behind its own mutex; concurrent
`kittui_place_json` calls serialize at the GPU layer but produce
independent PNG outputs.

Callers should use one runtime per process; spawning many runtimes is
allowed but each carries its own cache directory open-handle set.

### TypeScript bindings (`bindings/ts/`)

Two paths:

- **N-API (`@cosmos/kittui`, default).** A `kittui-napi` wrapper crate
  (added later) builds with `napi-rs` and produces prebuilt binaries via
  `prebuildify` per `(platform, arch)`. Hosts get sync + async APIs.
- **`koffi` fallback.** A pure-JS wrapper that `dlopen`s
  `libkittui_ffi.{so,dylib,dll}` directly. Slightly slower per call (no
  V8 fast-path) but requires no build step. Pi plugins start here.

Both paths consume the same `Scene` JSON; the TS surface is generated
from the Rust scene types via `serde-reflection` + `quicktype` so it
stays in sync automatically.

### Future bindings

The same C ABI gives Python (`ctypes` / `cffi`), Lua
(`luajit ffi`), Ruby (`Fiddle`), and Zig (`@cImport`) zero-cost paths.
We do not commit to packaging these in this repo; they live in their
own places and depend on a pinned `libkittui_ffi` version.

## Threading & concurrency

- **`Runtime` is `Send + Sync`.** Multiple threads may call `place`
  concurrently; the renderer pool serializes GPU access internally.
- **Cache is process-safe.** Lock files coordinate writes across
  concurrent CLI invocations or FFI processes.
- **No global state.** Each runtime is independent. The only ambient
  state is the cache directory; multiple runtimes with the same dir
  coexist cleanly via the lock-file protocol.
- **Async story.** kittui itself is sync — its calls are CPU-bound (the
  GPU encode + PNG write block briefly). N-API bindings expose
  `placeAsync` that runs `place` on a thread-pool worker; the koffi
  binding leaves async to the caller. We do not adopt `tokio` because
  the per-call cost doesn't justify the dependency.

## Error model

Three error layers, each with a single canonical type:

- **`kittui_core::Error`** for scene-construction issues (color parse,
  animation does-not-loop, invalid stop list). Caught at builder time;
  invalid scenes never reach the renderer.
- **`RenderError`** (per backend) for rasterization failures. The
  facade folds these into `KittuiError::Render`.
- **`KittuiError`** (the public type) for everything else: cache I/O,
  protocol assembly, FFI marshaling, panic recovery.

Errors are `thiserror`-derived, carry source chains, and never panic
across the FFI boundary (`catch_unwind` is the last line of defense).
The facade exposes `Runtime::last_error_snapshot()` for hosts that want
to inspect the most recent error without losing it to the boundary
crossing.

## Testing & validation

The library lives or dies on rasterization correctness, so testing is
unusually rigorous for its size.

- **Unit tests** in every crate. v0.1 ships 33; the GPU landing will
  add roughly another 30 for adapter init, shader compile, parity, and
  pool serialization.
- **Golden snapshots** under `crates/kittui-render-cpu/tests/golden/`.
  Each fixture is `(scene.json, expected.png)`. The CPU renderer is the
  oracle; goldens are committed and refreshed via `xtask refresh-goldens`.
- **CPU↔GPU parity** under `crates/kittui-render-gpu/tests/parity/`.
  Each parity test renders the same scene through both backends and
  asserts `SSIM(a, b) >= 0.99`. CI runs against `lavapipe` on Linux.
- **Property tests** for the scene normalizer (`proptest`): randomly
  generated scenes must round-trip through serde and produce a stable
  `SceneId`.
- **ABI snapshots.** `xtask abi-snapshot` re-runs cbindgen, diffs the
  result, and fails CI on unexpected drift. Snapshot lives at
  `crates/kittui-ffi/kittui.h`.
- **Fuzzing.** `cargo fuzz` on `Scene::from_json` and on every CLI flag
  parser. Targets live in `fuzz/`.
- **Loop-closure assertion.** Every animation is checked at build time
  via `PhaseCurve::closes_loop()`; the test suite confirms the
  rejection path actually rejects.
- **Integration smoke** (`tests/it.rs` at the workspace root, added
  with the GPU landing): spin up a runtime, render a small scene,
  assert the resulting bytes parse as PNG and the placement starts
  with `\x1b_G`.

## Phasing

Numbers reflect what's already landed vs. still ahead.

1. ✅ **kittui-core + kittui-render-cpu + kittui-cache + kittui facade +
   kittui-kitty + kittui-cli.** All built, all tested, all green on
   v0.1 head. The CPU oracle is pinned.
2. ✅ **ratakittui full coverage.** Chrome model, widget wrapper matrix,
   join-group composition, lifecycle tracker, `draw_with_kittui`
   adapter, ratatui showcase example. v0.1 head.
3. ✅ **kittui-ffi scaffold.** cdylib + staticlib + rlib, JSON-scene
   entry point, panic-safe, `abi_version` exported. v0.1 head.
4. ✅ **kittui-render-gpu** (wgpu). Unified shader covering rounded-rect
   SDF + gradient + radial glow + scanlines; CPU↔GPU SSIM-proxy
   parity gate; lazy adapter init in the facade with permanent CPU
   fallback under `Auto`.
5. ✅ **TS bindings.** `@kittui/koffi`: zero-build-step JS binding that
   dlopens `libkittui_ffi`, ties ownership cleanly via
   `koffi.disposable`, ships scene helpers + type declarations + tests.
   `@kittui/napi` with `prebuildify` per platform remains a future
   higher-performance path that consumes the same JSON.
6. ✅ **`kittui-tmux`.** Deterministic parser for
   `tmux list-panes -F` plus a composer that builds a ratakittui
   `JoinGroup` of pane chromes and a CLI binary that emits the
   chrome escape stream. The live hook integration is host-side glue.
7. ✅ **`kittui-wm` scaffold.** `WindowTree` + `WindowGeometry` types
   reserved so the substrate API is additive from here. Full layout
   semantics arrive after the tmux showcase exercises diff-driven
   composition.
8. ✅ **Affordance library.** `kittui-affordances` ships the showcase's
   tonal panel / chip / divider / title patterns as free functions
   returning `Chrome`. Opt-in; the core `kittui` crate never imports
   it.

This document is the source of truth. PRs that diverge from it must update it
first.

## Future ideas

Every item below now has a real artifact on `main`. The section title is
kept ("Future ideas") because each remains an explicit area for future
refinement — the v0.1 surface is small, but the type and the
contract are committed.

- ✅ **`kittui-tmux` pane-border showcase.** Parser for `tmux list-panes
  -F` + join-group composer + `kittui-tmux` binary; live tmux hook
  example under `crates/kittui-tmux/examples/install-hooks.sh` that
  registers the binary against tmux's `client-resized` /
  `pane-exited` / `window-layout-changed` / `window-resized` /
  `session-window-changed` / `pane-set-active` hooks. Source the
  script (`./install-hooks.sh | tmux source-file -`) to wire the live
  integration.
- ✅ **`kittui-wm` window manager substrate.** Scaffold crate landed at
  v0.1: `WindowTree` / `WindowGeometry` / `WindowId` with z-ordered
  layout. Full split/stack/tab semantics + per-window scene generation
  arrive once the tmux-border showcase has exercised diff-driven
  composition under real load.
- ✅ **`kittui-shader` user shaders.** `Node::Shader { rect, source,
  uniforms }` lives in `kittui-core`; serde round-trips. CPU renderer
  returns a clear `UnsupportedImage` error explaining GPU-only status.
  GPU renderer accepts the node in the type system; per-scene WGSL
  pipeline compilation + caching lights up the actual draw in the
  next revision. The CPU naga→WGSL fallback path is documented but
  unimplemented.
- ✅ **`Composition` first-class type.** `kittui::Composition` /
  `CompositionEntry` / `Composer` / `DiffResult`. Diff-driven
  upload-once + place-while-visible + delete-when-gone exposed at the
  library level, so Pi panels / kittui-wm / kittui-tmux / FFI
  consumers get the same protocol ratakittui's `LifecycleTracker`
  already implements internally.
- ✅ **Soft-keyboard / overlay surface.** `kittui-overlay` crate ships
  `Overlay` + `default_overlay_chrome()` (heavier shadow + brighter
  glow + pulse) and `Overlay::entry_from_chrome` so hosts build
  transient surfaces from a `Chrome` in one call. `Overlay::render`
  routes through a private `Composer`, so re-rendering the same
  overlay only re-emits the placement escape; `Overlay::hide`
  drains.
- ✅ **Live preview daemon.** `kittui-watch` binary watches a
  scene-JSON file by stat-polling at configurable interval and
  re-applies a one-entry `Composition` through a `Composer`. Drains
  on `Drop` so the terminal isn't left with an orphan image when the
  daemon exits.
