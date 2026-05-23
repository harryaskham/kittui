# kittui-wm v3 — native apps, X backends, and operator guide

kittui-wm is a terminal-native window manager. Its default surface is now a native PTY session: running `kittwm` with no backend flags starts a real shell in a nested PTY, renders it through kitty graphics, resizes it with the host terminal, and injects `KITTWM_SOCKET`, `KITTWM_DISPLAY`, `KITTUI_WM_DISPLAY`, and `KITTWM_WINDOW` into the child environment. Press `Ctrl-A %` (or `Ctrl-A |` / `Ctrl-A v`) to split side-by-side, `Ctrl-A -` (or `Ctrl-A h` / `Ctrl-A "`) to split into stacked rows, `Ctrl-A +/-` to grow the focused pane weight, `Ctrl-A _` (or `Ctrl-A <`) to shrink it, `Ctrl-A [` / `Ctrl-A ,` and `Ctrl-A ]` / `Ctrl-A .` to move the focused pane, `Ctrl-A b` to balance all pane weights, `Ctrl-A Tab` (or `Ctrl-A n`) to cycle focus, and `Ctrl-A x` to close the focused pane; keyboard input is routed only to the focused pane.

The WM can also host kittwm-native apps (for example `kittwm-browser`, backed by headless Chrome screenshots + DevTools input) and X/Quartz capture backends. The long-term model is DISPLAY-like: native apps are ordinary binaries that inherit a kittwm socket/window context, can `kittwm replace ...` their current container, or can ask the socket to spawn a new app when not already inside a window.


## Quick start

```sh
# Default: native PTY terminal sized to your current terminal.
cargo run -p kittui-cli --bin kittwm

# Run a specific terminal app in that native PTY.
KITTWM_TERMINAL_CMD=htop cargo run -p kittui-cli --bin kittwm

# Launch a first-class native browser app (headless Chrome based).
cargo run -p kittui-cli --bin kittwm-browser -- https://example.com

# Inside a kittwm PTY, replace the current container with another app.
kittwm replace browser https://example.com
kittwm replace htop

# Socket/display style context. :7 maps to /tmp/kittui-wm-7.sock.
KITTUI_WM_DISPLAY=:7 kittwm --serve
KITTUI_WM_DISPLAY=:7 kittwm --status
KITTUI_WM_DISPLAY=:7 kittwm --attach -c HELP_JSON
KITTUI_WM_DISPLAY=:7 kittwm --attach -c STATUS_JSON
KITTUI_WM_DISPLAY=:7 kittwm --attach -c PANES_JSON

# While a no-arg native kittwm session is running, inspect or create panes
# through its inherited/display-style socket.
kittwm --attach -c HELP
kittwm --attach -c HELP_JSON
kittwm --attach -c STATUS
kittwm --attach -c STATUS_JSON
kittwm --attach -c PANES
kittwm --attach -c PANES_JSON
kittwm --attach -c APPS_JSON
kittwm --attach -c 'APPS_FIRST htop'
kittwm --attach -c 'SPAWN_PTY htop'
kittwm --attach -c 'LAYOUT rows'
kittwm --attach -c 'FOCUS_PANE native-2'
kittwm --attach -c FOCUS_NEXT
kittwm --attach -c FOCUS_PREV
kittwm --attach -c 'MOVE_PANE focused last'
kittwm --attach -c 'RESIZE_PANE focused +2'
kittwm --attach -c BALANCE_PANES
kittwm --attach -c 'RENAME_PANE native-2 editor'
kittwm --attach -c 'SEND_LINE focused echo hello from controller'
kittwm --attach -c 'SEND_KEY focused ctrl-c'
kittwm --attach -c 'READ_TEXT focused'
kittwm --attach -c 'CLOSE_PANE focused'
```

Use `Ctrl-]` to exit the current native PTY/browser viewer. Explicit capture-backed demos remain available with `--backend fake|quartz|xvfb`.

Native `PANES_JSON` includes per-pane `window`, `title`, `focused`, `weight`, optional process metadata (`pid`, `command`), and, once a live session has rendered at least one frame, resolved title/app cell geometry: `x`, `y`, `cols`, `rows`, `app_x`, `app_y`, `app_cols`, and `app_rows`. Native `STATUS_JSON` mirrors the same detail in `focused_pane` and `panes_detail`. The text `PANES` reply includes process metadata plus the same geometry as `layout=x,y CxR app=x,y CxR` for simple shell inspection. Controllers can inject pane input with `SEND_TEXT <window|focused> <text>`, `SEND_LINE <window|focused> <text>`, or `SEND_KEY <window|focused> <key>` for named keys such as `ctrl-c`, `escape`, arrows, and paging keys. They can read pane screen text with `READ_TEXT <window|focused>` or `READ_TEXT_JSON <window|focused>` without scraping kitty graphics output. `HELP_JSON` is the machine-readable catalog for the socket command set.

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
  border chrome from the reusable `kittui_wm::chrome::WindowChromeTheme`.
  Hosts can build their own `WindowChromeState`/theme pair to preview or
  override focused/unfocused, tiled/floating, and title/source styling.
  `route_pointer` and `route_key` translate kittui-input
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
| `kittui_wm::native::PtyTerminalApp` | any Unix | real nested PTY; default `kittwm` surface; terminal apps like htop/vim run normally | default |
| `kittui_wm::native::HeadlessBrowserApp` / `kittwm-browser` | local Chrome/Chromium | browser surface via DevTools screenshots + mouse/keyboard input | default (requires Chrome) |
| `kittui-xvfb::FakeServer` | any | deterministic in-memory; tests + portable demo | default |
| `kittui-xvfb::xvfb::XvfbServer` | Linux | spawns Xvfb, captures via XCB GetImage, routes input via XTest | `--features xvfb` |
| `kittui-xvfb::xquartz::XQuartzServer` | macOS | spawns or attaches XQuartz and reuses x11rb capture/input | `--features xquartz` |
| `kittui-quartz::QuartzServer` | macOS | captures the main display via `CGDisplayCreateImage`/ScreenCaptureKit, posts pointer/key events via `CGEventPost` | `--features quartz` / `sck` |

### macOS XQuartz backend prerequisites

`kittui-xvfb::xquartz::XQuartzServer` is a macOS proof harness for X11 apps
inside kittwm. It is separate from the native Quartz/SCK capture backend: it
expects a real XQuartz server and X11 client tools to exist on the host.

Install the host-level prerequisites before running the `xquartz` feature lane:

```sh
brew install --cask xquartz
brew install xterm
```

After installing XQuartz, log out/in (or reboot) so launch services and the
`DISPLAY`/socket environment are refreshed. The expected binary locations on a
standard install are:

- `/opt/X11/bin/Xquartz`
- `/opt/X11/bin/xterm`

Useful smoke commands:

```sh
/opt/X11/bin/Xquartz :99 -nolisten tcp &
DISPLAY=:99 /opt/X11/bin/xterm &
cargo test -p kittui-xvfb --features xquartz xquartz -- --nocapture
```

If the binaries are missing, the XQuartz round-trip tests compile and skip
rather than failing; that means the Rust path is buildable but the host is not
ready for interactive XQuartz proof work. The Nix dev shell currently provides
Linux Xvfb/xterm tools, but macOS XQuartz itself is a host application and is
not supplied by the flake.

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

## Remaining roadmap

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
