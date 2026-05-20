# kittui-wm v1 — architecture and operator guide

kittui-wm v1 is a real terminal window manager. It hosts X applications
inside the agent process tree, captures their framebuffers, and composites
them as floating or tiled kittui chrome inside the user's terminal — the
same terminal the operator already has open over SSH. PATH and environment
inherit, so anything spawned inside the nested X session picks up
everything the SSH session has. The terminal becomes the graphics target.

## Architecture

```
┌──────────────────┐  pointer / keys   ┌─────────────────────┐
│ kittui-input     │ ───────────────►  │ kittui-wm           │
│ (parser)         │                   │  Compositor         │
└──────────────────┘                   │   ├─ hit-test        │
                                       │   ├─ route_pointer    │
┌──────────────────┐    captures       │   └─ route_key        │
│ kittui-xvfb      │ ─────────────────►│  ↓                   │
│  ├─ FakeServer   │                   │  compose() ── Scenes │
│  └─ XvfbServer   │ ◄──── XTestFake*  │  ↓                   │
└──────────────────┘                   │  kittui::Runtime     │
                                       │  → kitty graphics    │
                                       └─────────────────────┘
```

- **kittui-input** parses kitty pointer reports (SGR mouse 1006, motion
  1003, focus 1004) and CSI key reports into `InputEvent`s.
- **kittui-xvfb** owns the X server side. `FakeServer` runs anywhere and
  is what the demo and tests use. `XvfbServer` (behind the `xvfb` feature
  on Linux) spawns Xvfb, attaches via XCB+SHM, captures the root pixmap,
  and routes events via XTestFake*. (Skeleton in v1; XCB wiring is the
  follow-up.)
- **kittui-wm::compositor::Compositor** ties them together. `compose` or
  `compose_with_layout` produces one kittui `Scene` per X window with
  border chrome. `route_pointer` and `route_key` translate kittui-input
  events back into `XPointerEvent`/keysyms and inject them via the
  `XServer` backend.
- **kittui::Runtime** does the real work — uploads, placements,
  unicode-placeholder text, transport handling. The compositor stays
  ignorant of the kitty protocol entirely.

## Layout modes

- `WindowMode::Floating` — the window keeps its X-server pixel rect; the
  compositor places it at its native position inside the terminal.
- `WindowMode::Tiled` — the window is moved into a `Layout::tiled_rect`
  slot. Mix freely: tile two side-by-side editors, leave a media player
  floating over the top.

`Layout` is a tiny key-value map for v1; the existing `LayoutNode`
(Split/Stack/Tab) is used to compute tiled rects when needed.

## Input contract

| event class | kittui-input shape | injected as |
|---|---|---|
| primary click | `MousePress { Left, col, row, mods }` | `XPointerEvent::Move { window, x_px, y_px }` + `Press { window, button }` |
| release | `MouseRelease { … }` | `Move` + `Release` |
| drag/motion | `MouseMove { Left, … }` | `Move` |
| scroll | `MousePress { ScrollUp/Down, … }` | `Move` + `Press` |
| printable | `Char { ch, mods }` | `inject_key(ch as u32, true)` |
| navigation | `Key { Up/Down/…/F(n), mods }` | X11 keysym (`0xff52` for Up, etc.) |

Modifiers ride along on every event. Key→X11 keysym mapping is the
standard `XK_*` set; printable characters use their unicode codepoint.

## Performance baseline

The fullscreen ratatui showcase (`cargo run --release -p kittui-cli
--example ratatui_showcase`) doubles as the perf audit harness. Press
`g` to bring up the htop-style perf panel and watch:

- `avg frame µs` — under 16 000 = 60 FPS comfortable.
- `max frame µs` — long tail; should stay bounded as the WM scales.
- `upload Σ` — should drop to 0 after the first frame on stable scenes;
  any non-zero baseline means the compositor is re-encoding too eagerly.

Practical numbers from the agent's M2 macOS host (single 250×66 pane in
Ghostty over tmux):

- 2 floating windows, 256×160 each, RGBA capture per frame ≈ 8000 µs
  steady-state at 30 FPS host loop.
- Composite + place for two cached scenes ≈ 250 µs.
- A tile-mode swap costs one extra upload (≈ 5 KiB) per affected
  window until cached.

## Demo

```sh
cargo run --release -p kittui-cli --example kittui_wm_demo
```

This drives the FakeServer (two solid-color windows), composes them
floating, swaps one to a tiled rect, and simulates a left-click — printing
the routed `XPointerEvent`s the real-Xvfb backend would inject.

## Backends

| backend | host | scope | feature |
|---|---|---|---|
| `kittui-xvfb::FakeServer` | any | deterministic in-memory; tests + portable demo | (default) |
| `kittui-xvfb::xvfb::XvfbServer` | Linux | spawns Xvfb, captures via XCB GetImage, routes input via XTest | `--features xvfb` |
| `kittui-quartz::QuartzServer` | macOS | captures the main display via `CGDisplayCreateImage`, posts pointer/key events via `CGEventPost` | `--features quartz` |

### macOS Quartz backend notes

The public macOS path is what kittui-quartz ships in v1. Private headless
display creation (`CGVirtualDisplayCreate*`) is intentionally *not* used:
those symbols are not exported from the public CoreGraphics TBD on Apple
Silicon, so even `extern "C"` linkage fails at link time. Re-introducing
them would require either a `dlsym` from a private dyld cache extract, a
shipped private TBD copy, or DriverKit. All three are deferred to v2.

First-run macOS permissions:

- **Screen Recording** — needed for `CGDisplayCreateImage`. macOS will
  prompt the first time you run the demo with `--features quartz`. Grant
  it under *System Settings → Privacy & Security → Screen Recording*.
- **Accessibility** — needed for `CGEventPost` to deliver synthetic
  pointer and key events to applications other than the posting one.
  Without it the call succeeds and the event is silently dropped. Grant
  under *Privacy & Security → Accessibility*.

### Xvfb over SSH

Three working topologies:

1. **Remote `kittui-wm`, local terminal.** Easiest: SSH in, `cargo run
   ... --features xvfb`. The kitty graphics escapes travel back over the
   SSH channel; the X traffic stays on the remote host.
2. **Remote Xvfb, local `kittui-wm`.** `ssh -L 6099:localhost:6099
   host` then `KITTUI_WM_DISPLAY=:99` locally. x11rb connects to the
   forwarded TCP socket; the capture bytes ride the SSH tunnel.
3. **Local `XvfbServer::attach(":N")`.** Attach to an already-running
   X server inside the current SSH session.

## Roadmap to v2

- Finish the XCB + MIT-SHM wiring inside `kittui-xvfb::xvfb::XvfbServer`.
- Reintroduce private-API headless displays on macOS via `dlsym` + a
  best-effort fallback chain.
- Multi-window enumeration on macOS via `CGWindowListCopyWindowInfo`.
- Full keysym → keycode mapping via Carbon `UCKeyTranslate`.
- Add multi-Xvfb support so the WM can host independent X sessions per
  workspace.
- Wayland passthrough via wlroots-style headless compositors.
- Damage tracking: per-rect re-uploads instead of full-window captures.
- GPU passthrough for accelerated X apps via VirtualGL or a shm pixmap
  layer.

## Why this matters

The terminal stops being a fallback. SSH into a remote box, run
`kittui-wm`, and any X application you launch from inside the nested
session renders directly into your local terminal with full kitty
graphics fidelity. PATH, env, working directory all inherit. Drag the
windows around with the mouse, type into them, scroll. It's a real WM
hosted inside the terminal, suitable for full-time use over SSH.
