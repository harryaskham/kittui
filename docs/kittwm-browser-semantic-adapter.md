# Browser DOM/ARIA semantic adapter plan

`bd-2250e1` tracks the first browser-specific semantic adapter for kittwm. Browser surfaces today are useful but pixel-first: `HeadlessBrowserApp` drives Chrome through DevTools, captures screenshots, and injects text/mouse/key input. That keeps arbitrary web content visible, but it hides the DOM and accessibility semantics that would let kittwm expose labels, controls, links, forms, focus, and actions through `SemanticSurfaceSnapshot`.

This plan defines a narrow DOM/ARIA/DevTools adapter that can publish semantic component trees while preserving screenshot fallback for canvas, video, WebGL, and custom controls that do not expose enough structure.

## Goals

- Expose a browser page as a kittwm semantic surface using existing SDK types: `SemanticSurfaceSnapshot`, `ComponentNode`, `ComponentRole`, `ComponentValue`, `ComponentAction`, and `ActionKind`.
- Keep pixels as the source of visual truth. Semantics augment the browser screenshot; they do not replace it for unsupported content.
- Route semantic focus/actions through Chrome DevTools in a way that mirrors user input (`click`, `focus`, text edit, select) instead of inventing a parallel app model.
- Make updates incremental enough for live pages without blocking screenshot capture.
- Keep cross-origin and sensitive fields conservative by default.

## Architecture

The adapter should live next to the existing `HeadlessBrowserApp` DevTools client in `kittui-wm::native` or a small browser-semantic submodule. It should be optional and best-effort:

1. **Snapshot trigger** — on a timer, after navigation, after DOM mutation, and after focus/action routing, request a semantic snapshot from the page.
2. **Extraction** — execute a small JavaScript/DevTools routine in the page context to walk visible DOM nodes and ARIA metadata. A later version can use Chrome's Accessibility domain (`Accessibility.getFullAXTree`) when available, but DOM+ARIA is enough for the first proof.
3. **Mapping** — convert DOM/ARIA nodes into SDK component nodes with stable ids, roles, values, state, layout hints, and actions.
4. **Publish** — call the already-landed semantic publish path (`SEMANTIC_PUBLISH` / `SurfaceHandle::semantic_publish`) for the browser pane/window.
5. **Fallback** — if extraction fails, returns no meaningful controls, or the content is canvas/video/custom-only, continue serving the screenshot/pixel surface and expose either the previous valid semantic tree or a small fallback group describing the browser surface.

The browser surface still captures screenshots through the existing DevTools screenshot path. Semantic snapshots are side-band state for inspection, rendering overlays, and automation.

## DOM/ARIA to kittwm mapping

Use ARIA role first, then native element semantics, then a conservative custom role.

| DOM/ARIA source | `ComponentRole` | Value/state | Actions |
|---|---|---|---|
| `form`, `fieldset`, landmark sections, generic containers with useful children | `Group` | label/description from accessible name | none unless focusable |
| `label`, headings, static text | `Label` | `ComponentValue::Text` for visible text | none |
| `button`, `role=button`, submit/reset inputs | `Button` | `active`, `disabled` | `Activate`, `Focus` when focusable |
| `a[href]`, `role=link` | `Custom("browser.link")` until a first-class Link role exists | href in description or custom metadata later | `Activate`, `Focus` |
| `input type=text/search/email/url/tel`, contenteditable single line | `TextInput` | `ComponentValue::Text`, `sensitive=false` | `Focus`, `SetValue`, `InsertText` |
| `textarea`, multiline contenteditable | `TextArea` | `ComponentValue::Text` | `Focus`, `SetValue`, `InsertText` |
| `input type=password` | `TextInput` | omit/redact value, `sensitive=true` | `Focus`, `SetValue`, `InsertText` |
| `input type=checkbox`, `role=checkbox` | `Checkbox` | `checked`, `disabled` | `Toggle`, `Focus` |
| `input type=radio`, `role=radio` | `Radio` under a `RadioGroup` when grouping is known | `checked`, `selected` | `Select`, `Focus` |
| `select`, `role=listbox`, combobox with options | `SelectList` | `Selection([...])` where ids are known | `Select`, `OpenMenu`, `Focus` |
| `input type=range`, `role=slider` | `Slider` | numeric value if parseable | `SetValue`, `Focus` |
| `progress`, `meter` | `Progress` | numeric value if parseable | none |
| `table`, `grid`, `treegrid` | `Table` or `Custom("browser.grid")` | children for rows/cells when bounded | `Focus`, `Scroll` where useful |
| `nav/menu/menubar/menuitem` | `Menu` / `Custom("browser.menu_item")` | selected/expanded | `Activate`, `OpenMenu`, `Close`, `Focus` |
| `canvas`, `video`, WebGL, opaque widgets | `Custom("browser.pixel_region")` or omitted | description from ARIA/name if any | usually `Focus`/`Activate` only |

Stable ids should be generated from, in priority order:

1. author-provided `id` when unique;
2. stable DOM path plus role/name hash;
3. DevTools backend node id / AX node id when available.

Ids must be stable across ordinary text/value updates so action routing can target the same component after a refresh.

## Accessible names and descriptions

The adapter should approximate the browser accessibility name algorithm enough for common controls:

- `aria-label`, `aria-labelledby`, `aria-describedby`;
- associated `<label for=...>` / label-wrapped inputs;
- button/link text content;
- `alt` for images that become semantic nodes;
- `title` as a fallback only when no better name exists.

Sensitive values (passwords, fields with `autocomplete=current-password/new-password`, or explicit redaction hints) should not publish raw text values. Expose role, focusability, disabled state, and a redacted/sensitive flag instead.

## Event and update loop

The first proof can use a polling loop, but the architecture should allow event-driven updates:

1. Subscribe via DevTools to navigation/lifecycle/runtime events.
2. Inject a `MutationObserver` and focus/input/change/click listeners that bump a page-side revision counter.
3. On each kittwm frame or a bounded timer, ask only whether the revision changed.
4. If changed, extract a new snapshot, increment `SemanticSurfaceSnapshot.revision`, and publish.
5. Coalesce rapid changes (for example typing) to avoid spamming the daemon; 30-100 ms debounce is enough for a first pass.

Extraction must have a deadline. If DevTools hangs or the page is cross-origin-isolated in a way that blocks script evaluation, skip semantic update and keep the screenshot path healthy.

## Hit-testing, focus, and action routing

Semantic action routing should reuse the same target ids generated for snapshots. The browser adapter needs a reverse map from component id to DOM locator / backend node id.

Suggested first actions:

- `Focus`: call `element.focus()` or DevTools focus command, then publish.
- `Activate`: call `element.click()` for buttons/links/menu items, or dispatch a click at the node's center when DOM click is unsuitable.
- `SetValue`: set value/contenteditable text, dispatch `input` and `change`, then publish.
- `InsertText`: focus the element and use DevTools `Input.insertText` or DOM insertion, preserving browser editing behavior where possible.
- `Select`: update selected option/radio/listbox item and dispatch relevant events.
- `Scroll`: call `scrollIntoView` / adjust scroll containers.

If an action target no longer exists after a DOM update, return an explicit stale-component error and ask the caller to refresh `SEMANTIC_SNAPSHOT`.

## Fallback and limitations

- Canvas/video/WebGL/custom-rendered apps remain primarily pixel surfaces. Publish only a named `browser.pixel_region` node when ARIA metadata exists; do not synthesize fake controls from pixels in this adapter.
- Cross-origin iframes may expose limited DOM. Treat each accessible iframe as a child surface/group when possible; otherwise expose its frame title/URL if allowed and rely on screenshot fallback.
- Shadow DOM should be traversed for open roots; closed roots are opaque unless the browser accessibility tree exposes them.
- Virtualized lists may only publish visible items plus a scroll action/range; do not pretend the full list is present.
- Passwords and secrets are redacted by default.
- DOM order and visual order may differ. The adapter should include layout hints from bounding boxes so renderers can choose.

## Minimal proof follow-ups

- `bd-22195b`: implement a DevTools DOM/ARIA snapshot extractor for `HeadlessBrowserApp` that maps common controls (label, button, link, text input, checkbox, select) into `SemanticSurfaceSnapshot`.
- `bd-fea819`: wire the browser adapter to publish snapshots through the kittwm semantic publish path on navigation/focus/change, with screenshot fallback intact.
- `bd-15cde5`: route `SEMANTIC_ACTION`/`SEMANTIC_FOCUS` for browser semantic nodes to DevTools/DOM operations for focus, activate, set value, insert text, and select.

These should be separate implementation beads because extraction, publishing cadence, and action routing each carry different risk.
