# kittwm semantic component surfaces

Tracking bead: `bd-911832`

## Why this exists

For a runnable workflow, see [`kittwm-semantic-quickstart.md`](kittwm-semantic-quickstart.md).

`kittwm` already has useful surface forms:

- terminal/cell surfaces from the native PTY engine;
- pixel surfaces from browser, X11/Xvfb, XQuartz, or RGBA capture;
- kittui primitive scenes for deterministic rendering and shell previews.

Those forms are enough to run programs, but they do not preserve the semantic
shape of an application. A captured Qt dialog, GTK preferences page, browser
form, or SDK-built panel may contain labels, buttons, radio groups, text inputs,
menus, split panes, tabs, tables, and accessibility metadata. If kittwm only sees
pixels, it cannot route focus semantically, expose accessible names, render a
pure-terminal equivalent, automate by action id, or remap the same app into a
kittui-native scene.

Semantic component surfaces add a fourth surface form: an application can publish
a structured component tree plus typed actions/events. kittwm can then choose the
best presentation for the host terminal and policy while retaining a pixel or
terminal fallback when semantics are unavailable.

## Responsibility split

The split must stay consistent with the broader kittui/kittwm architecture:

- `kittui-core` remains primitive-only: geometry, colors, text/image primitives,
  and rendering substrate types.
- `kittui-affordances` owns reusable high-level UI builders: buttons,
  checkboxes, radio groups, text inputs, selects, progress bars, tabs, menus,
  tables, split panes, and kittwm chrome pieces.
- `kittwm` owns runtime semantics: windows, surfaces, focus, capabilities,
  event routing, actions, lifecycle, fallback policy, and the DISPLAY/socket-like
  control plane.
- Toolkit/browser/accessibility adapters translate native app semantics into the
  kittwm semantic surface protocol. They do not draw kittwm chrome directly.

Cairo or other raster APIs fit below this layer as drawing backends or pixel
surface engines. They do not by themselves provide semantic UI; a Cairo-rendered
widget still needs an adapter or accessibility tree if kittwm is to understand it
as a button, radio group, or input.

## Surface hierarchy and fallback

A surface may advertise several frame forms at once. kittwm should prefer the
highest-fidelity safe representation for the current host and policy:

1. **Semantic component surface**: component tree, roles, labels, values, focus,
   actions, events, and optional preferred layout. This enables pure terminal,
   kitty graphics, kittui scene, remote/headless, and accessibility-aware
   rendering from the same source.
2. **Terminal/cell surface**: styled cell grid and cursor/mode state from a PTY
   or terminal-like adapter. This preserves TUI behavior and text automation.
3. **Kittui primitive scene**: already-renderer-independent drawing primitives,
   useful for SDK apps and shell/chrome composition.
4. **Pixel capture**: RGBA/PNG frame capture from arbitrary GUI apps, browser
   screenshots, Xvfb/XQuartz, or user-provided streams. This is the universal
   fallback when no semantics exist.

Arbitrary GUI apps still require pixel capture unless a framework adapter,
accessibility bridge, DOM/DevTools bridge, or native SDK integration exposes
semantic structure.

## Protocol objects

The wire/protocol model should be versioned and JSON-compatible first, even if a
future SDK uses a binary/framed transport.

```rust
struct SemanticSurfaceSnapshot {
    schema_version: u32,
    surface_id: SurfaceId,
    revision: u64,
    root: ComponentNode,
    focus: Option<ComponentId>,
    cursor: Option<TextCursor>,
    viewport: Option<Viewport>,
    capabilities: SemanticCapabilities,
}

struct ComponentNode {
    id: ComponentId,
    role: ComponentRole,
    label: Option<String>,
    description: Option<String>,
    value: Option<ComponentValue>,
    state: ComponentState,
    actions: Vec<ComponentAction>,
    layout: ComponentLayout,
    style: ComponentStyleHints,
    children: Vec<ComponentNode>,
}
```

### Stable identifiers

`ComponentId` must be stable across revisions whenever the underlying app object
is the same. Automation and focus restoration should not depend on tree index.
Adapters can derive ids from framework object ids, DOM node ids, accessibility
ids, or SDK-supplied stable keys. If an adapter cannot produce stable ids, it
must mark ids as ephemeral so kittwm avoids persisting them.

### Roles

Initial roles should cover common controls without overfitting one toolkit:

- structural: `Window`, `Panel`, `Group`, `Toolbar`, `StatusBar`, `MenuBar`,
  `Menu`, `MenuItem`, `Separator`, `ScrollArea`, `SplitPane`, `TabList`, `Tab`,
  `Table`, `Row`, `Cell`, `List`, `ListItem`, `Tree`, `TreeItem`;
- text: `Label`, `Heading`, `Paragraph`, `Code`, `Link`;
- controls: `Button`, `ToggleButton`, `Checkbox`, `Radio`, `RadioGroup`,
  `TextInput`, `TextArea`, `SearchBox`, `Select`, `ComboBox`, `Slider`,
  `Progress`, `Spinner`;
- media/custom: `Image`, `Canvas`, `Terminal`, `BrowserDocument`, `Custom`.

`kittwm-sdk::ComponentRole` now includes first-class variants for the common
roles used by the first semantic adapters: `Link`, `Heading`, `Paragraph`,
`Code`, `Row`, `Cell`, `List`, `ListItem`, `Tree`, `TreeItem`, `Image`,
`Canvas`, `Terminal`, and `BrowserDocument` in addition to the existing control
roles. `Custom` remains available for vendor-specific roles and must include a
namespaced type string and fallback role so renderers can still choose a generic
representation. Browser and accessibility adapters now remap obvious link,
canvas/media, heading, list/tree, row/cell, and image roles to these first-class
variants; `Custom("browser.*")` or `Custom("accessibility.*")` remains reserved
for vendor-specific or still-unsupported roles.

### Values and state

`ComponentValue` should be typed:

- boolean checked/pressed state;
- string text value;
- numeric value/range/step for sliders/progress;
- selected item ids for list/select/table/tree;
- active tab id;
- optional validation status and error text.

`ComponentState` should include at least:

- focused, focusable;
- hovered/active/pressed;
- selected/checked/expanded/collapsed;
- disabled/read-only/required;
- invalid/busy/loading;
- visible/hidden;
- dirty/stale flag if a subtree is only partially refreshed.

### Layout

Semantic surfaces should publish enough layout hints to support both faithful and
adaptive rendering:

```rust
enum LayoutKind {
    FixedRect,     // adapter knows pixel/cell bounds
    Flow,          // renderer may wrap/reflow
    Row,
    Column,
    Grid,
    Stack,
    Overlay,
    Absolute,
}
```

Each node can include:

- optional source rectangle in logical surface coordinates;
- preferred/min/max size in cells and/or pixels;
- grow/shrink weights;
- padding/margin/gap;
- alignment;
- clipping/scroll region metadata;
- z-index for overlays/menus/tooltips.

The renderer may ignore exact pixels in pure terminal mode, but should preserve
focus order, labels, action affordances, and grouping.

## Actions and event routing

Actions are the semantic equivalent of input injection. They let automation and
renderers activate controls without synthesizing fragile mouse coordinates.

```rust
struct ComponentAction {
    id: ActionId,
    kind: ActionKind,
    label: Option<String>,
    enabled: bool,
    input: Option<ActionInputSpec>,
}

enum ActionKind {
    Activate,
    Toggle,
    SetValue,
    InsertText,
    DeleteText,
    Select,
    Expand,
    Collapse,
    OpenMenu,
    Close,
    Scroll,
    Focus,
    Custom(String),
}
```

Runtime command examples:

```text
SEMANTIC_SNAPSHOT <surface|focused>
SEMANTIC_ACTION <surface|focused> <component_id> <action_id> <json_payload>
SEMANTIC_FOCUS <surface|focused> <component_id>
SEMANTIC_HIT_TEST <surface|focused> <x> <y>
```

SDK examples:

```rust
let tree = surface.semantic_snapshot()?;
surface.focus_component("settings.notifications.email")?;
surface.invoke("settings.notifications.email", "toggle", json!(true))?;
```

Input routing order:

1. If a kittwm renderer has semantic focus on a component and receives an
   activation/editing key, dispatch a semantic action.
2. If the component exposes text-edit semantics, translate text input to
   `InsertText`, cursor movement, selection, or delete actions.
3. If the surface lacks semantic handling for the event, fall back to terminal or
   pixel coordinate input if the surface advertises that capability.
4. If no route exists, report a denied/unsupported event rather than injecting
   ambiguous input.

## Focus model

There are three nested focus levels:

1. kittwm window focus;
2. surface focus within a composite/window;
3. semantic component focus within a semantic surface.

The runtime owns the global focus chain and may ask surfaces to set component
focus. Surfaces remain authoritative about whether focus succeeded. Snapshot
revisions should report the current focused component so renderers can draw focus
rings/cursors consistently.

Tab order should be either explicitly supplied by the adapter or derived from the
component tree and layout order. Hidden/disabled nodes are skipped.

## Events

Semantic surfaces should emit JSON-lines/SDK events alongside existing surface
events:

```rust
enum SemanticSurfaceEvent {
    SnapshotReady { surface: SurfaceId, revision: u64 },
    TreeChanged { surface: SurfaceId, revision: u64, changed: Vec<ComponentId> },
    FocusChanged { surface: SurfaceId, component: Option<ComponentId> },
    ValueChanged { surface: SurfaceId, component: ComponentId, value: ComponentValue },
    ActionInvoked { surface: SurfaceId, component: ComponentId, action: ActionId },
    Announcement { surface: SurfaceId, message: String, politeness: AnnouncementPoliteness },
}
```

`TreeChanged` can start coarse-grained. Fine-grained subtree diffs are useful but
should not be required for initial correctness; a full snapshot by revision is
simpler and safer.

## Renderer mapping

A semantic renderer in kittwm should translate component nodes into kittui
primitive scenes using shared `kittui-affordances` components:

- button roles -> affordance button;
- checkbox/radio roles -> affordance checkbox/radio/radio-group;
- text input/area -> affordance input with cursor/selection hints;
- select/list/tree/table -> affordance list/table/tree primitives;
- tabs/split panes -> affordance layout/chrome primitives;
- unknown/custom roles -> labeled panel with available actions.

Renderer targets:

- **Pure terminal renderer**: Unicode/text controls, focus brackets, simple color
  and keyboard hints. This is the safest tmux/remote fallback.
- **Kitty graphics renderer**: render kittui scene to pixels and place via the
  configured graphics transport.
- **Kittui scene/headless renderer**: emit primitive JSON or PNG artifacts for
  tests, shell scripts, docs, and external platforms.
- **Remote streaming renderer**: send semantic snapshots and/or rendered frames
  depending on client capability.

Fallback must be explicit: if a role cannot be represented, draw a generic label
with actions instead of silently dropping it.

## Capabilities and security

Semantic actions can mutate app state and must be capability-scoped. Suggested
capabilities:

- `ReadSemanticTree`;
- `SubscribeSemanticEvents`;
- `FocusComponent`;
- `InvokeSemanticAction`;
- `EditSemanticText`;
- `ReadSensitiveValues` for password/secret fields;
- `AutomateGlobalUi` for cross-window semantic automation.

Password/secret fields should expose role/state and focusability but redact value
unless the client has explicit permission. Clipboard actions should continue to
flow through kittwm clipboard policy.

## Adapter sources

Potential semantic adapters:

- native kittwm SDK apps that construct component trees directly;
- browser DOM/ARIA/DevTools adapter, now landed for first-party browser surfaces
  with snapshot extraction, best-effort publish/debounce, CLI one-shot
  inspection, DevTools-backed focus/action routing, stale-component errors, and
  first-class link/canvas role remaps; see [`kittwm-browser-semantic-adapter.md`](kittwm-browser-semantic-adapter.md);
- accessibility tree adapters for macOS AX, AT-SPI, UI Automation, or platform
  equivalents; the safe platform-neutral core now maps extracted macOS AX-style
  and Linux AT-SPI-style trees, reports permission/unavailable diagnostics,
  routes actions through an `AccessibilityActionBackend`, and uses first-class
  roles for obvious document/list/tree/media nodes, while direct platform
  bindings remain follow-up work; see [`kittwm-accessibility-semantic-adapter.md`](kittwm-accessibility-semantic-adapter.md);
- toolkit plugins/backends for Qt, GTK, egui, iced, Tauri, Electron, etc.;
- terminal apps with future semantic escape extensions, while classic TUIs remain
  terminal/cell surfaces.

Adapters may pair semantics with pixels. For example, a browser can expose DOM
semantics while still providing screenshots for canvas/video/custom content.

## Initial implementation path

1. Land the protocol document and keep it versioned (`bd-911832`).
2. Add reusable controls in `kittui-affordances` (`bd-0337ce`).
3. Add SDK/protocol types for semantic snapshots, nodes, actions, values, and
   events.
4. Add a synthetic in-process `SemanticSurface` test adapter with a small form:
   label, text input, checkbox, radio group, select/list, progress, tabs, and
   button.
5. Render that synthetic tree through kittui affordances in kittwm (`bd-586ce3`).
6. Expose narrow socket/SDK commands for semantic snapshot/action/focus.
7. Keep expanding real adapter proofs: native SDK publishing, browser DOM/ARIA,
   and safe accessibility-tree mapping/action routing now exist; durable platform
   bindings and richer lifecycle policy remain follow-up work.

## Follow-up bead map

Existing beads:

- `bd-0337ce` — `kittui-affordances: add first-party form and control components`.
- `bd-586ce3` — `kittwm: render semantic component surfaces via kittui affordances`.

Landed follow-ups from this plan now include:

- SDK semantic protocol types, events, snapshot/publish/action/focus wrappers,
  and convenience helpers.
- Native socket semantic snapshot/publish/action/focus commands.
- Synthetic semantic SDK example app and publish/readback workflows.
- Browser DOM/ARIA snapshot extraction, best-effort publishing, CLI inspection,
  DevTools-backed focus/action routing, and first-class link/canvas role mapping.
- Accessibility semantic adapter planning, safe macOS AX-style and Linux
  AT-SPI-style mapping core, permission/unavailable diagnostics, platform-neutral
  action routing, and first-class role remapping for obvious document/list/tree
  and media roles.

Remaining follow-ups:

- Durable standalone semantic surface lifecycle and richer app-owned event loops.
- Direct macOS AX / Linux AT-SPI / UI Automation bindings that feed the safe
  accessibility adapter core.
- Toolkit plugins/backends for Qt, GTK, egui, iced, Tauri, Electron, etc.
- Optional future terminal semantic escape extensions for TUIs.
