# Accessibility-tree semantic adapter plan

`bd-4a49aa` tracks a semantic adapter for arbitrary native GUI applications where toolkit/browser-specific semantics are not available. Today those apps enter kittwm primarily as pixels through X11/Xvfb, XQuartz, Quartz, or other capture backends. Platform accessibility trees can expose labels, roles, values, menus, and focus/action affordances for many of those apps without screen-scraping pixels.

This plan covers the first accessibility-tree adapter shape for macOS Accessibility (AX) and Linux AT-SPI, while preserving pixel capture as the visual fallback and treating permissions/security as first-class constraints.

## Goals

- Translate platform accessibility trees into existing kittwm SDK semantic types: `SemanticSurfaceSnapshot`, `ComponentNode`, `ComponentRole`, `ComponentValue`, `ComponentState`, `ComponentAction`, and `ActionKind`.
- Pair semantic trees with existing pixel surfaces instead of replacing capture. If AX/AT-SPI is unavailable or incomplete, users still see and can interact with pixels.
- Route semantic focus/actions through platform accessibility APIs where possible, with safe fallbacks to existing input injection.
- Keep sensitive values redacted unless an explicit future capability permits them.
- Avoid global automation surprises: the adapter must target the kittwm-owned/captured app/window, not arbitrary desktop windows.

## Architecture

The adapter should be a side-band semantic source attached to a captured native app/window surface:

1. **Window association** — map a kittwm surface/window handle to the platform accessibility object for the same app window.
   - macOS: CGWindowID / owning PID / AXUIElement window list.
   - Linux: X11 window id / PID / AT-SPI application tree where available.
2. **Snapshot extraction** — walk a bounded accessibility subtree and convert visible/focusable nodes into SDK component nodes.
3. **Publish** — publish the latest snapshot with `SEMANTIC_PUBLISH <window> <snapshot-json>` so existing `SEMANTIC_SNAPSHOT` consumers see accessibility semantics instead of a generic text/pixel fallback.
4. **Events** — subscribe to platform focus/value/structure notifications when available; otherwise poll with debounce.
5. **Actions** — route semantic focus/actions to AX/AT-SPI operations and emit stale-component errors when ids no longer resolve.
6. **Fallback** — if permissions, association, extraction, or action routing fail, keep the pixel surface and input-injection path alive.

The adapter should be optional at runtime and explicitly diagnosable. Missing permissions must never crash a viewer.

## Platform source shape

### macOS AX

Relevant source APIs/objects:

- `AXUIElementCreateApplication(pid)` for app root;
- `kAXWindowsAttribute`, `kAXFocusedWindowAttribute`, `kAXChildrenAttribute` for traversal;
- `AXUIElementCopyAttributeValue` for role/title/value/enabled/focused/selected/frame;
- `AXObserverCreate` / notifications for focus, value, title, selected children, and window moved/resized;
- `AXUIElementPerformAction` for press/show menu/increment/decrement and related actions.

Permissions:

- Requires Accessibility permission for the process hosting the adapter.
- If denied, publish no AX snapshot and report a diagnostic note; do not prompt-loop.
- Screen Recording remains a separate pixel-capture permission.

### Linux AT-SPI

Relevant source APIs/objects:

- D-Bus AT-SPI registry and application tree;
- object roles, names, descriptions, states, relations, interfaces;
- `Action`, `Text`, `Value`, `Selection`, `Component`, and `EditableText` interfaces;
- focus/property/children changed events.

Permissions/session caveats:

- Requires AT-SPI bus availability and desktop environment support.
- Sandboxed apps may expose partial or no trees.
- Wayland/X11 window-to-accessible-object association may need PID/app heuristics rather than a stable window id.

## Role and action mapping

Use the platform role as the first signal, then state/interfaces to refine.

| Platform role/interface | kittwm role | Values/state | Actions |
|---|---|---|---|
| window/dialog/panel/group | `Group` | title/description | `Focus` when focusable |
| static text/heading/label | `Label` | `Text` from accessible name/value | none |
| push button/menu button | `Button` | `disabled`, `focused` | `Activate`, `Focus`, maybe `OpenMenu` |
| check box/toggle button | `Checkbox` | `checked`, `disabled` | `Toggle`, `Focus` |
| radio button/radio group | `Radio` / `RadioGroup` | `selected`/`checked` | `Select`, `Focus` |
| text field/search field/password field | `TextInput` | `Text` unless sensitive; `sensitive=true` for passwords | `Focus`, `SetValue`, `InsertText` |
| multiline text/editable text | `TextArea` | `Text` unless sensitive | `Focus`, `SetValue`, `InsertText`, `Scroll` |
| combo box/list/select | `SelectList` | `Selection([...])` where ids known | `OpenMenu`, `Select`, `Focus` |
| slider/spin button/value control | `Slider` | `Number` normalized or raw value | `SetValue`, platform increment/decrement custom actions |
| progress indicator | `Progress` | `Number` when exposed | none |
| menu/menu item/menu bar | `Menu` / `Custom("ax.menu_item")` or `Custom("atspi.menu_item")` | selected/expanded | `Activate`, `OpenMenu`, `Close`, `Focus` |
| table/grid/tree | `Table` or `Custom("accessibility.tree")` | selected rows/cells | `Select`, `Expand`, `Collapse`, `Scroll`, `Focus` |
| image/canvas/custom view | `Custom("accessibility.pixel_region")` | label/description if exposed | `Focus`/`Activate` only when the platform exposes it |

IDs should be stable but not leak private paths unnecessarily. Suggested id source order:

1. platform persistent identifier if available and scoped to the app/window;
2. role + accessible name + sibling index path hash;
3. backend node path index as a fallback.

The id should include an adapter prefix (`ax:` or `atspi:`) to avoid collisions with browser DOM ids.

## Snapshot extraction policy

- Bound traversal by depth and node count (for example depth 12, 2,000 nodes) to avoid freezing on huge app trees.
- Include nodes that are visible, focusable, actionable, carry a meaningful label/value, or have included descendants.
- Collapse purely decorative containers where possible.
- Preserve enough layout hints (frame bounds mapped to kittwm/app-local coordinates) for semantic renderers and hit testing.
- Keep revision numbers monotonic per surface.
- Redact sensitive values; expose the component and state, not the secret.

## Event/update loop

Preferred event-driven path:

1. Subscribe to platform focus/value/title/selection/children/window-geometry notifications for the associated app/window.
2. Coalesce notifications with a short debounce (30-100 ms).
3. Re-extract and publish `SemanticSurfaceSnapshot` when structure/value/focus changed.
4. If notification setup fails, fall back to bounded polling (for example 500 ms to 1 s) while the surface is visible/focused.

The adapter must not block the pixel capture/render loop. Extraction should happen on a worker or with a deadline; a timeout means skip semantic update and keep the last good snapshot or pixel fallback.

## Focus and action routing

Action routing resolves a semantic component id back to the latest platform object:

- `Focus`: platform focus/set-focused call, or platform action where focus is exposed.
- `Activate`: AX press / AT-SPI action named click/press/activate/default.
- `Toggle`: checkbox/toggle action or value flip through platform API.
- `SetValue`: AX value set / AT-SPI Value or EditableText where permitted.
- `InsertText`: focus then platform editable text insertion; fallback to existing keyboard/text injection only for the focused target and only when safe.
- `Select`: platform selection interface.
- `OpenMenu` / `Close` / `Expand` / `Collapse`: platform actions when advertised.
- `Scroll`: platform component scroll or fallback to input injection over the target bounds.

If the object no longer resolves, return a stale-component error and require the caller to refresh the snapshot. If the platform denies the action, return an unsupported/permission error rather than silently injecting global input.

## Security and privacy

- Accessibility permissions grant broad desktop visibility. The adapter should remain opt-in until the operator explicitly enables it for kittwm/browser/native app surfaces.
- Never publish password/secure text values by default. Use `ComponentState.sensitive=true` and omit `ComponentValue`.
- Avoid publishing full filesystem paths, window titles from other apps, or unrelated desktop tree nodes outside the target app/window.
- Prefer per-surface association over global tree dumps.
- Expose diagnostics for permission denied, no matching accessibility object, timeout, and partial tree extraction.

## Fallback behavior

Pixel capture remains authoritative for visuals. The accessibility adapter can publish:

- no snapshot, leaving existing pixel/text fallback active;
- a partial semantic tree for known controls;
- a root group with a diagnostic/custom pixel-region child if the app is opaque.

Automation and renderers must tolerate partial semantics. They should not assume every visible control has an accessibility node.

## Minimal implementation follow-ups

- `bd-a17062`: macOS AX proof that associates a captured app/window with an AX window, extracts a bounded snapshot, maps common controls, and reports permission diagnostics.
- `bd-dcb522`: Linux AT-SPI proof that finds an app/window tree, extracts roles/names/states/actions, and degrades cleanly when AT-SPI is unavailable.
- `bd-eabe22`: route focus/activate/set value/insert text/select through resolved AX/AT-SPI objects with stale-component and permission errors.

These should stay separate because platform association/extraction and action routing have different risk profiles.
