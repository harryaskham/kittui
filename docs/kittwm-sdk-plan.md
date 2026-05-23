# kittwm SDK and surface architecture plan

## Why this exists

`kittwm` is growing from a native terminal-window experiment into a terminal-hosted window manager/compositor. The default shell already owns pane layout, focus, socket commands, native PTY lifecycle, browser capture demos, session manifests, and automation. As this grows, the core risk is coupling every feature directly to the built-in session loop.

The target architecture is a small `kittwm` runtime with first-class surface/window primitives, plus first-party apps and shells built on the same SDK that external apps can use.

## Core principle

Separate these responsibilities:

1. **kittwm core/runtime** owns windows, focus, placement, session state, permissions, app lifecycle, frame scheduling, and the DISPLAY/socket-like control plane.
2. **Surface engines** own I/O and renderable state for one captured thing: PTY terminal, X11/Xvfb app, Quartz app, browser/DevTools app, kittui scene, RGBA stream, etc.
3. **kittwm shell** owns the default tiling/floating UX, keybindings, launcher/picker overlays, chrome policy, and restore policy.
4. **Standalone apps** such as `kittwm-terminal`, `kittwm-launch`, or custom composite apps connect through the SDK rather than being baked into the shell.

The built-in shell should dogfood the same primitives wherever practical.

## Current state

Already present:

- Native `kittwm` starts real PTY panes using `portable-pty`.
- PTY output is parsed with `vte` into a custom `TerminalState` with screen cells, style, cursor, scrollback, alt screen, DEC modes, OSC title, OSC 52 forwarding, mouse/focus/bracketed-paste modes, and readback snapshots.
- `TerminalSurface` now owns the reusable PTY terminal engine (PTY read/write, parsing, responses, snapshots, resize state, and RGBA rendering), while `PtyTerminalApp` keeps process lifecycle as a thin adapter.
- `PtyTerminalApp` and `HeadlessBrowserApp` implement a small `NativeApp` shape: title, resize, send text/input, capture frame.
- Xvfb/XQuartz/browser capture support exists in project crates; native terminal/browser adapters now share a first-pass `NativeSurface` metadata/capture/input/resize/focus/event-drain interface, and XQuartz/Xvfb metadata adapter proofs exist. Capture-only scene/frame adapters also exist: `KittuiSceneSurface` renders a `kittui` scene through the CPU renderer to PNG, while `RgbaFrameSurface` and `CompositeFrameSurface` expose raw RGBA `SurfaceFrame`s for runtime/composite experiments. `NativeSurface::take_surface_events` gives PTY surfaces a common path for title/bell/OSC52/notification side effects, while capture-only adapters default to no events. The focus hook notifies surfaces about pane focus-in/out; the PTY implementation only emits terminal focus-reporting sequences when the nested app requested focus reporting. These adapters are not yet wired into the default live kittwm session path.
- Native socket commands expose panes, control, app discovery, save/restore, automation input, text/scrollback reads, waits, semantic snapshot/publish/action/focus, policy-gated cached clipboard reads, a JSON `EVENTS [ms]` stream with status/pane/focus/layout/semantic plus surface side-effect events, and machine-readable help.
- `kittui wm-chrome` and `kittui wm-session` provide static chrome/session previews. Live shell chrome can opt into kittui-affordance scene rendering, but default behavior still keeps the conservative ANSI/kitty placement path.

Missing or immature:

- The SDK surface is now broad: typed socket client, handles/specs, status/pane accessors, events/iterators, semantic helpers, app discovery, sessions, automation input/read/wait helpers, browser spawn via the first-party app, command catalog, and local capability presets.
- The native event stream now emits status/pane/focus/layout, semantic snapshot/focus/action/value, title/bell/OSC52 clipboard-set/notification side-effect, resize, input, and graphics-frame presentation events. Typed SDK coverage exists for the earlier event families, with resize/input/frame SDK follow-ups landing separately.
- Terminal engine extraction has started with `TerminalSurface`, but it still lives inside `kittui-wm::native` rather than a standalone `kittui-term`/`kittwm-terminal` crate.
- GUI/capture backends are partially expressed through the new native surface abstraction; XQuartz/Xvfb proofs and capture-only kittui scene/RGBA/composite frame surfaces exist, while default live-runtime integration remains immature.
- External apps can now dogfood the SDK with terminal/browser launchers, semantic examples, sessions, automation, and a composite example that spawns child surfaces and composes text snapshots side-by-side. True child-surface frame capture/present is still immature.
- Clipboard/bell/notification side effects are modeled as events and OSC 52 set-clipboard forwarding exists; PTY surfaces expose those side effects through the common `NativeSurface::take_surface_events` hook, and capture-only adapters return an empty event batch. `CLIPBOARD_JSON` adds a default-deny, cache-only read policy for the latest nested-app OSC52 write via `KITTWM_CLIPBOARD_READ=allow|1|true|yes`. It does not read the host OS clipboard and is not yet wrapped by the SDK.
- Capability/security policy has an initial client-side SDK shape with `none`, `restricted`, `inspect_only`, and `automation` local presets, but runtime-issued credentials and per-client enforcement are still immature.
- Semantic component surfaces are documented in [`kittwm-semantic-surfaces.md`](kittwm-semantic-surfaces.md) and [`kittwm-semantic-quickstart.md`](kittwm-semantic-quickstart.md); SDK/native/browser/accessibility proofs now exist, but durable standalone semantic surface lifecycle and platform bindings are still maturing.

## Target object model

### Runtime objects

```rust
struct WindowId(String);
struct SurfaceId(String);

struct WindowSpec {
    title: String,
    app_id: String,
    mode: WindowMode,
    preferred_size: Option<CellSize>,
}

struct SurfaceSpec {
    kind: SurfaceKind,
    title: Option<String>,
    env: Vec<(String, String)>,
}

enum SurfaceKind {
    Terminal { command: String, cwd: Option<PathBuf>, profile: Option<String> },
    X11 { command: String, display_policy: DisplayPolicy },
    Quartz { command: String, app_bundle: Option<String> },
    Browser { target: BrowserTarget },
    KittuiScene,
    RgbaStream,
    Composite,
}
```

### Surface capabilities

Surfaces should advertise capabilities instead of every surface pretending to support everything:

- `CaptureSurface`: emits frames/cells/scenes.
- `InputSurface`: accepts key/mouse/text/paste/focus input.
- `ResizableSurface`: accepts cell/pixel resize.
- `ClipboardSurface`: emits set/read clipboard events.
- `NotificationSurface`: emits bell/notification/title events.
- `RestorableSurface`: can serialize/restore stable manifest state.

### Frame types

A surface may present one or more frame forms:

- RGBA frame.
- PNG frame.
- terminal cell grid with styles.
- kittui primitive scene.
- host-terminal escape stream for pure terminal fallback.

The renderer chooses the best output path for the current host and policy.

## SDK shape

A Rust SDK should begin with a narrow synchronous API, then grow an async/event API.

```rust
let wm = kittwm_sdk::connect_from_env()?;
let window = wm.replace_current(WindowSpec { ... })?;
let term = wm.spawn_surface(SurfaceSpec::terminal("$SHELL -l"))?;

loop {
    for event in wm.poll_events()? {
        match event {
            Event::WindowInput { window, input } => term.send_input(input)?,
            Event::WindowResize { size, .. } => term.resize(size)?,
            Event::SurfaceFrameReady { surface } => window.present(term.capture()?)?,
            Event::SurfaceClipboardSet { selection, bytes } => wm.set_clipboard(selection, bytes)?,
            _ => {}
        }
    }
}
```

Initial transport wraps the existing socket protocol; later it can switch to a structured framed protocol without changing app-facing types. Current typed helpers cover status/panes plus `NativePaneDetail` accessors for bounds/cursor/modes/dirty-frame/transport diagnostics, bounded events plus `KittwmEventIter`, `events_iter_ms` / `event_iter_ms`, `KittwmEvent::envelope`, `unknown_raw`, and `EventEnvelope::detail_*` accessors, terminal surface spawn/replace, first-party browser surface spawn via `SurfaceSpec::browser(...)`, semantic snapshot/publish/action/focus, first-class semantic roles for common document/browser/accessibility structures, screen text and scrollback snapshots, visible/output text waits plus typed `WaitMatchKind` / `WaitMatch` helpers (`wait_text_match[_ms]`, `wait_output_match[_ms]`) over the existing raw wait replies, control/input helpers including exact bytes, bracketed paste bytes, and mouse events, layout/focus/move/balance helpers through `Kittwm::focus_next`, `focus_prev`, `layout(LayoutMode)`, `balance_panes`, and `SurfaceHandle::move_pane(MoveDirection)`, session save/restore through `SessionManifest`, `SessionPane`, `Kittwm::session`, and `Kittwm::restore_session`, app discovery through `Kittwm::apps`, `Kittwm::app_first`, and `Kittwm::app_launch_first`, and command catalog introspection through `HelpCatalog`, `HelpCommand`, `Kittwm::help_catalog`, and `Kittwm::help`.

## First-party apps

### `kittwm-terminal` / `kittui-term`

A standalone terminal app designed for kittwm:

- Current first-party binary: `kittwm-terminal` can spawn or replace terminal surfaces through the SDK.
- It now also dogfoods typed SDK status/pane and bounded event APIs via `--status` and `--events-ms`.
- Future work: own terminal UX/config directly (profiles, shell, theme, scrollback limits, copy/paste policy, shortcuts, bell/notification behavior) and move closer to the extracted terminal engine rather than only asking the shell to spawn PTYs.
- The built-in default terminal can remain as bootstrap but should use the same engine.

### `kittwm-launch`

A standalone launcher/spawner:

- Current first-party binary: `kittwm-launch` detects/accepts backend choices and uses SDK paths for terminal/app/browser-ish launching.
- It now has clearer backend selection, dry-run/status planning, URL/browser behavior, and dogfoods the typed SDK app-discovery helpers for app backend discovery/launch.
- Browser targets can now be requested from SDK clients with `SurfaceSpec::browser("https://...")`; the current transport launches the first-party `kittwm-browser` app inside a PTY-backed surface. Future work: dedicated browser/X/Quartz surface protocols instead of PTY/app fallbacks; `SurfaceKind::Other` remains unsupported.
- Lets specialized launchers exist without bloating the shell.

### Composite apps

Custom apps should be able to spawn child surfaces and compose them. Example:

- `kittwm-vim-firefox` spawns a terminal/vim surface and a browser/GUI surface.
- It captures both, blits them side-by-side, presents a single window, and routes input by coordinate/focus.
- This validates that kittwm primitives are reusable rather than shell-private.

## Renderer split

The shell should eventually render a presentation-agnostic view model:

```text
SessionModel + WindowTree + SurfaceFrames + ChromeModel
  -> KittyGraphicsRenderer
  -> PureTerminalRenderer
  -> KittuiSceneRenderer
  -> Headless/PNG renderer
```

Today the live shell directly places kitty images and writes ANSI chrome. The next step is to extract a `NativeWmView`/`ChromeModel` and renderer trait while preserving current behavior.

## Clipboard, bell, and notifications

Terminal/app side effects should become semantic events:

```rust
enum SurfaceEvent {
    TitleChanged(String),
    Bell { visual: bool, audible: bool },
    ClipboardSet { selection: Selection, bytes: Vec<u8> },
    ClipboardRead { selection: Selection, request_id: RequestId },
    Notification { title: String, body: String },
}
```

Policy belongs to kittwm/runtime:

- forward OSC 52 set-clipboard to host terminal or OS clipboard;
- deny clipboard reads by default or require capability;
- display visual bell in chrome;
- route notifications to host/OS/user-configured UI.

## Capability model

External apps must not implicitly get all WM powers. Capabilities should include:

- create/replace windows;
- spawn surfaces;
- capture child surfaces;
- send input to child surfaces;
- read/write clipboard;
- subscribe to global events;
- request focus/raise/close;
- persist/restore session data.

Built-in shell and first-party apps can receive broader capabilities; arbitrary clients should be scoped.

## Implementation stages

### Stage 1: stabilize current primitives

- Keep host side-effect mediation such as OSC 52 set-clipboard covered by host forwarding, event reporting, and the default-deny cache-only `CLIPBOARD_JSON` policy surface.
- Keep native socket help/status comprehensive.
- Keep the native `EVENTS [ms]` stream covered while broadening it beyond current status/pane/focus/layout/semantic/side-effect events so clients do not poll status/readback.

### Stage 2: extract terminal surface engine

- Continue hardening the new `TerminalSurface` boundary by moving it toward a standalone `kittui-term`/`kittwm-terminal` crate when the SDK shape is ready.
- Keep `PtyTerminalApp` as an adapter to avoid behavior churn.
- Add direct tests at the surface boundary as it gains a public constructor independent of live PTY process spawning.

### Stage 3: define common surface trait/model

- Extend the new `SurfaceId`, `SurfaceFrame`, `SurfaceMetadata`, `SurfaceCapabilities`, and `NativeSurface` model into the remaining native adapters, including the common focus notification and `take_surface_events` drain hooks for terminal side-effect events.
- Keep PTY and browser surfaces adapted to the trait as the reference implementation.
- Map Xvfb/XQuartz capture/input and kittui-scene/composite surfaces to the same trait; capture-only `KittuiSceneSurface`, `RgbaFrameSurface`, and `CompositeFrameSurface` are landed building blocks, with live session wiring still follow-up work.

### Stage 4: SDK transport and handles

- Continue expanding the existing `kittwm-sdk` crate with typed requests and handles.
- Keep the initial transport backed by the existing native socket protocol.
- Typed SDK session helpers now expose `SESSION_JSON` / `RESTORE_SESSION_JSON` as `SessionManifest`, `SessionPane`, `Kittwm::session`, and `Kittwm::restore_session`; session reads use the low-risk read capability, while restore is gated as a create/control mutation.
- Typed SDK event iteration over `EVENTS [ms]` exists for current status/pane/focus/layout/semantic and surface side-effect events, including `pane_resized`, `pane_input_sent`, and `pane_frame_presented` follow-up variants as runtime sources land.
- Semantic surface snapshot/publish/action/focus APIs and common action helper methods now exist in the SDK.

### Stage 5: dogfood built-in shell

- Replace direct shell-private app manipulation with surface handles where possible.
- Extract chrome/view model from direct ANSI drawing.
- Render live chrome through kittui-compatible model or renderer trait.

### Stage 6: first-party standalone apps

- `kittwm-terminal` exists and now includes SDK status/event inspection helpers.
- `kittwm-launch` exists and is now a backend-aware SDK launcher using typed app discovery and first-party browser app paths.
- A composite SDK example exists at `crates/kittui-cli/examples/kittwm_composite_app.rs`.
- A synthetic semantic SDK app exists at `crates/kittui-cli/examples/kittwm_semantic_app.rs` and can print/query/publish component trees.

## Backlog mapping

Recommended remaining beads:

1. `kittwm: extract TerminalSurface engine from PtyTerminalApp` into a reusable terminal-engine crate or public boundary.
2. `kittwm: wire scene/RGBA/composite NativeSurface adapters into live runtime` beyond the landed capture-only building blocks.
3. `kittwm-sdk: finish model coverage for resize/input/frame events` as the runtime sources land.
4. `kittwm-sdk: add CLIPBOARD_JSON helper/capability docs` over the runtime's default-deny cached OSC52 read policy.
5. `kittwm: add runtime-issued SDK credentials/per-client enforcement` beyond local client capability presets.
6. `kittwm-sdk: add connect/window handle skeleton over native socket`
7. `kittwm-sdk: add typed surface spawn/capture/input APIs`
8. `kittwm: dogfood surface handles in built-in native session`
9. `kittwm: extract presentation-agnostic shell view/chrome model`
10. `kittwm: add pure terminal/DEC renderer backend for shell view model`
11. `kittwm-terminal: add standalone first-party terminal app skeleton` — landed; follow-up maturity includes SDK status/events.
12. `kittwm-launch: add standalone app launcher skeleton` — landed; backend/app-discovery maturity and first-party browser path are active.
13. `kittwm: model clipboard/bell/notification events and policy`
14. `kittwm: add capability scoping for SDK clients`
15. `examples: add composite app spawning terminal plus browser surfaces` — initial SDK example lives at `crates/kittui-cli/examples/kittwm_composite_app.rs`.
16. `kittwm: plan semantic component surface protocol` — see [`kittwm-semantic-surfaces.md`](kittwm-semantic-surfaces.md).
17. `kittui-affordances: add first-party form and control components`
18. `kittwm: render semantic component surfaces via kittui affordances`

## Non-goals for the near term

- Perfect xterm conformance in the custom terminal emulator.
- Replacing all socket commands at once.
- Removing the built-in default terminal before standalone `kittwm-terminal` is proven.
- Making arbitrary external clients fully trusted by default.
