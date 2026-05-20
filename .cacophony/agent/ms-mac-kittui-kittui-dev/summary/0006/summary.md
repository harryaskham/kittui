# Session summary — component completeness wave

## Goal

Continue past pure protocol conformance into the component layer: kill the
cache parallel test flake, finish transport detection + CLI override,
implement real Composite/Mask/Clip rendering semantics, ship batch render
entrypoints, and persist a renderer probe so Auto mode does not pay GPU
init cost on every startup.

## Bead(s)

- `bd-3581fb` — kittui-cache tests flake when run in parallel
- `bd-85e274` — Complete terminal transport probing and File/Memory transfer modes
- `bd-9ce511` — Implement real Mask, Clip, and BlendMode rendering semantics
- `bd-e5a91b` — Add batch/many render entrypoints
- `bd-6c1165` — Add renderer probe cache and GPU parity gating to Runtime Auto mode

All five claimed by `ms-mac-kittui-kittui-dev`; all landed in this session.

## Before state

- Failing tests: `kittui-cache` flaked 2/8 under parallel runs (same-nanosecond temp-dir collisions).
- Relevant metrics: `Composite { mode: Add|Multiply|Screen }` reduced to Normal, `Mask` ignored, `Clip` ignored; no batch entrypoints; `Runtime::builder()` never consulted `probe.json`; `TerminalInfo::detect` only looked at `$TMUX`.
- Context: protocol-conformance epic `bd-3dc8c7` was complete and visually proven, but the component layer underneath still had real gaps.

## After state

- Failing tests: none. Workspace 85 tests, 5 consecutive clean parallel runs.
- Relevant metrics: `kittui-render-cpu` adds `Pixmap::blend_with(mode)` and Composite renders each child to a scratch and blends the first with Normal + rest with the requested mode; `Node::Mask` renders mask and child into scratches and multiplies child alpha by mask alpha per-pixel; `Node::Clip` clamps the child draw to a rectangle. `Runtime::place_many` and `Runtime::place_batch` (+`BatchPlacement`) added. `TerminalInfo::detect` now considers `TMUX`, `KITTY_WINDOW_ID`, `KITTY_PUBLIC_KEY`, `TERM_PROGRAM` (ghostty/iterm/wezterm/kitty), `TERM`, and `WT_SESSION`. CLI gained `--transport {direct,tmux,file,memory}`. `kittui probe --force` invalidates the persisted probe. `RuntimeBuilder::build` consults `probe.json` and starts in `BackendState::GpuFailed` for Auto mode when prior probe failed.
- Context: Visual proof captured for `--transport tmux` override (`screenshots/bd-85e274-transport-tmux.png`); ratakittui showcase, affordance showcase, and `kittui proof` matrix all still pass.

## Diff summary

- Code/content commits this session:
  - `e2003f3` — cache test flake fix + transport probing/override + composite/mask/clip semantics + batch entrypoints
  - `d2759be` — probe.json persistence + Auto-mode honouring + `kittui probe --force`
- Files touched: kittui-cache (eviction.rs, lib.rs, lock.rs, probe.rs), kittui-cli/src/main.rs, kittui-core/src/terminal.rs, kittui-render-cpu (lib.rs, pixmap.rs, rasterize.rs), kittui/src/lib.rs.
- Tests: +4 terminal detect, +3 pixmap blend modes, +3 scene composite/mask/clip, +2 runtime batch, +1 runtime probe-honour, +0 cache (race fix only). Total +13 tests.
- Behavioural delta: Composite/Mask/Clip nodes now affect rendering; multi-scene hosts can call `place_many`/`place_batch`; terminal detection covers the major kitty-family terminals; Auto-mode boot is one-shot.

## Embedded artefacts

- `screenshots/bd-85e274-transport-tmux.png` — visual confirmation that `kittui --transport tmux box ...` still renders a real image through tmux passthrough.

## Operator-takeaway

The component layer under the spec-conformant protocol now matches what the
DESIGN promised: blending semantics, mask/clip semantics, batch entrypoints,
broad terminal detection, transport override, and a one-shot Auto probe.
The remaining open beads — image decoding (`bd-ca05d4`), full v1 CLI
surface (`bd-ac9d7e`), FFI completeness (`bd-d5ce20`), shader nodes
(`bd-a0bd40`), wm semantics (`bd-eaaf38`), fuzz targets (`bd-50f52a`) —
are independent and can be picked up in any order.
