# kittwm semantic surfaces quickstart

Tracking bead: `bd-829e64`

This quickstart shows the current end-to-end semantic surface workflow. The
semantic stack is intentionally incremental: kittwm can expose a terminal pane as
a semantic fallback, SDK apps can build/publish component snapshots, and scripts
can inspect the current tree. Published snapshots support basic in-memory focus/value actions; browser DOM
snapshot publishing exists, while toolkit/browser action routing remains future
adapter work.

## Current pieces

- Protocol model: `docs/kittwm-semantic-surfaces.md`.
- SDK types: `kittwm_sdk::{SemanticSurfaceSnapshot, ComponentNode,
  ComponentRole, ComponentValue, ComponentAction, ...}`.
- SDK methods:
  - `SurfaceHandle::semantic_snapshot()`;
  - `SurfaceHandle::semantic_publish(&snapshot)`;
  - `SurfaceHandle::semantic_action(component, action, payload)`;
  - `SurfaceHandle::semantic_focus(component)`.
- Native socket commands:
  - `SEMANTIC_SNAPSHOT <window|focused>`;
  - `SEMANTIC_PUBLISH <window|focused> <snapshot-json>`;
  - `SEMANTIC_ACTION <window|focused> <component> <action> <json>`;
  - `SEMANTIC_FOCUS <window|focused> <component>`.
- Example app: `crates/kittui-cli/examples/kittwm_semantic_app.rs`.
- Browser adapter plan/current state: `docs/kittwm-browser-semantic-adapter.md`.

## Start kittwm

Run native kittwm in one terminal:

```sh
cargo run -p kittui-cli --bin kittwm
```

Inside a kittwm-managed pane, children inherit `KITTWM_SOCKET`,
`KITTWM_DISPLAY`, and `KITTWM_WINDOW`. Those environment variables let SDK apps
connect back to the runtime.

## Print a synthetic semantic snapshot

The semantic example can generate a standalone settings/form component tree:

```sh
cargo run -p kittui-cli --example kittwm_semantic_app -- \
  --surface synthetic-settings
```

The output is JSON shaped like:

```json
{
  "schema_version": 1,
  "surface": "synthetic-settings",
  "revision": 1,
  "root": {
    "id": "settings.root",
    "role": "group",
    "label": "Settings",
    "children": [
      { "id": "settings.tabs", "role": "tabs" },
      { "id": "settings.name", "role": "text_input" },
      { "id": "settings.notifications", "role": "checkbox" }
    ]
  },
  "focus": "settings.name"
}
```

The actual example includes tabs, text input, checkbox, radio group,
select/list, progress, split pane, and button nodes.

## Publish a semantic tree into kittwm

When run inside a kittwm pane, publish the generated tree to the focused surface:

```sh
cargo run -p kittui-cli --example kittwm_semantic_app -- --publish-current
```

Or publish to an explicit window id:

```sh
cargo run -p kittui-cli --example kittwm_semantic_app -- --publish native-1
```

The example uses:

```rust
let wm = kittwm_sdk::Kittwm::connect_from_env()?;
let snapshot = synthetic_settings_snapshot("focused");
wm.focused_surface().semantic_publish(&snapshot)?;
```

A published snapshot is stored as the latest semantic tree for that pane/window.
Subsequent `SEMANTIC_SNAPSHOT` reads prefer the published tree over the terminal
text fallback.

## Read a semantic snapshot back

Using SDK:

```rust
let wm = kittwm_sdk::Kittwm::connect_from_env()?;
let snapshot = wm.focused_surface().semantic_snapshot()?;
println!("{}", serde_json::to_string_pretty(&snapshot)?);
```

Using the example:

```sh
cargo run -p kittui-cli --example kittwm_semantic_app -- --query-current
```

Using CLI wrappers:

```sh
cargo run -p kittui-cli --bin kittwm -- --semantic-snapshot focused
cargo run -p kittui-cli --bin kittwm -- --semantic-publish focused snapshot.json
cargo run -p kittui-cli --bin kittwm -- --semantic-publish focused - < snapshot.json
```

The raw socket escape hatch remains available:

```sh
cargo run -p kittui-cli --bin kittwm -- --attach -c 'SEMANTIC_SNAPSHOT focused'
```

## Fallback behavior

If no semantic snapshot has been published for a terminal pane, kittwm exposes a
safe fallback tree:

- root role: `group`;
- child role: `text_area`;
- value: current terminal text snapshot;
- focus: the text area when the pane is focused.

This makes every native pane inspectable through the same semantic API while
clearly preserving that a PTY is still a terminal/cell surface, not a true form
or toolkit adapter.

## Action and focus status

For **published** semantic snapshots, the daemon supports a conservative in-memory
action/focus subset:

- `SEMANTIC_FOCUS` updates the snapshot focus and focused/focusable flags;
- `SEMANTIC_ACTION ... focus {}` delegates to the same focus path;
- `toggle` flips boolean/checked state;
- `set`, `set_value`, and `insert_text` update text, number, or bool scalar values;
- `select` updates selection values and child selected/checked flags.

Using SDK helpers:

```rust
let surface = wm.focused_surface();
surface.semantic_focus_component("settings.name")?;
surface.semantic_set_text("settings.name", "Grace")?;
surface.semantic_toggle("settings.notifications")?;
surface.semantic_select_one("settings.profile", "settings.profile.ops")?;
```

For fallback PTY text-area snapshots, action/focus still returns explicit
unsupported errors. This is intentional: kittwm should not pretend to mutate
arbitrary terminal or pixel UI semantics by synthesizing fragile coordinates.

Browser semantic action routing through DevTools is tracked separately and should
return stale-component errors when ids no longer resolve.

## Rendering semantic trees

`kittui-wm::semantic::render_semantic_surface(...)` can turn a synthetic
semantic component tree into primitive kittui scenes via `kittui-affordances`.
This is the renderer bridge used for tests and future presentation paths:

- pure terminal renderer can draw text/Unicode controls;
- kitty graphics renderer can place rasterized kittui scenes;
- headless renderers can emit JSON/PNG artifacts;
- remote clients can choose semantics or pixels based on capability.

High-level controls remain in `kittui-affordances`; `kittui-core` remains
primitive-only.

## Current limitations

- Arbitrary GUI apps still need pixel capture unless a framework, accessibility,
  DOM/DevTools, or native SDK adapter exposes semantics.
- Published snapshots are the first runtime storage path, not a full lifecycle
  model for standalone semantic surfaces.
- Basic semantic action/focus mutation is in-memory for published snapshots; real
  adapter-backed mutation is still adapter-specific.
- Browser DOM/ARIA snapshot extraction and best-effort publishing exist; browser
  action routing through DevTools is follow-up work.
- Qt/GTK/accessibility adapters are future work.
- Published semantic trees do not replace terminal input routing by default.

## Next useful work

- Add a real standalone semantic surface/app lifecycle around SDK publishing.
- Route browser semantic actions through DevTools.
- Add Qt/GTK/accessibility-tree adapter spikes.
- Make semantic renderer output selectable in live kittwm views where useful.
