# kittui-wm v3 — native apps, X backends, and operator guide

kittui-wm is a terminal-native window manager. Native backends now expose common surface metadata/capabilities, focus notification, and side-effect event draining through `NativeSurface`, while the older `NativeApp` adapter remains for existing session code. `SurfaceCapabilities` now distinguishes coarse text input from exact byte input, focus notifications, and side-effect event draining so controllers can tell which hooks a surface advertises. PTY surfaces advertise exact-byte input, focus notifications, and surface events; capture-only scene/RGBA/composite adapters leave those extended flags false. `NativeSurface::send_surface_pointer` adds a surface-level pointer hook for native adapters; today `XWindowSurface` translates move/press/release events into `XPointerEvent`s, while PTY mouse routing still uses the separate socket/SGR path and live session defaults are unchanged. The focus hook is a pane/surface notification: PTY surfaces use it to send terminal focus-in/out sequences only when the nested app has enabled focus reporting. It is separate from socket `FOCUS_PANE` (window focus control) and `SEMANTIC_FOCUS` (semantic component focus). `NativeSurface::take_surface_events` gives PTY surfaces a common path for title, bell, OSC 52 clipboard, and notification side effects; capture-only adapters default to an empty event batch, and daemon `EVENTS` publication semantics remain unchanged. Capture-only `NativeSurface` building blocks now include `KittuiSceneSurface` (renders kittui scenes through the CPU renderer to PNG), `RgbaFrameSurface` (caller-provided raw RGBA), and `CompositeFrameSurface` (composes positioned RGBA children into one raw RGBA frame), but these are runtime/composite primitives rather than default live-session surfaces yet. Set `KITTWM_NATIVE_RENDERER=terminal` to use the presentation-agnostic shell view's pure ANSI/text renderer instead of kitty graphics frame blitting for native PTY panes; kittwm also defaults to this pure terminal renderer inside tmux to avoid unbounded tmux memory growth from high-rate raw kitty graphics passthrough payloads. Use `KITTWM_NATIVE_RENDERER=kitty` or `graphics` outside tmux-safe workflows to force graphics rendering. Set `KITTWM_NATIVE_CHROME_RENDERER=affordance-scene` to opt into kittui-affordance-rendered native pane/footer chrome while preserving the default ANSI chrome path. In that live kittui scene chrome path, the top-bar scene carries the same state/text as the pure terminal bar through labelled scene layers (for example active/empty state plus the trimmed top-bar text), so future render-artifact consumers can inspect the live chrome without scraping ANSI; the pure terminal renderer remains the ANSI fallback. Outside tmux, kittui/kittwm graphics are sent as kitty graphics escape sequences by default; set `KITTUI_KITTY_COMPRESSION=zlib` (or `auto`) to add kitty `o=z` zlib compression to direct PNG/raw-RGBA uploads and reduce wire bytes (tracked as `bd-562644`). The broader automatic policy for choosing direct, tmux passthrough, file, shared-memory, zlib, or pure-terminal fallback is captured in [`adaptive-graphics-transport.md`](adaptive-graphics-transport.md). Pane resize reports a new logical cell size to surfaces (PTY panes receive the equivalent PTY resize) and the WM also enforces the allocated frame bounds by cropping/padding captured frames before placement; implicit pixel scaling/zoom is intentionally separate future behavior. Its default launch is now a clean empty workspace rather than an automatic shell pane: running `kittwm` with no backend flags shows a stable one-line top bar (`kittui-bar`, workspace id, empty/active state, time, display) and a shortcut hint. Set `KITTWM_WORKSPACE=<label>` to override the displayed/reported workspace label in the live top bar, `STATUS_JSON`, `PANES_JSON`, `CHROME_JSON`, and SDK chrome metadata; this is label/config metadata for the current single-workspace runtime, not full multi-workspace switching. Set `KITTWM_STARTUP_TERMINAL=1` (or `true`/`yes`) to opt back into the compatibility behavior that starts the default terminal immediately. From the empty workspace, press `Ctrl-A Enter` to launch the default terminal command as a native terminal pane; `Ctrl-A t` toggles floating mode, `Ctrl-A f` toggles fullscreen, and `Ctrl-A e` toggles the current split between vertical and horizontal. In tiled mode, drag a pane title and release over another pane to reorder the tiled stack; in floating mode, click a pane title to focus/raise it and drag that title row to reposition the pane within the terminal bounds. Spawned panes receive `KITTWM_SOCKET`, `KITTWM_DISPLAY`, `KITTUI_WM_DISPLAY`, and `KITTWM_WINDOW` in the child environment. `KITTWM_TERMINAL_CMD` remains the shell/app command to run, while `KITTWM_TERMINAL_BINARY` is an equivalent config-system handoff key for the default command. The YAML config defaults `terminal.backend` to `ghostty`; `libghostty.theme`, `libghostty.background`, `libghostty.background_opacity`, `libghostty.foreground`, `libghostty.cursor`, `libghostty.enable_ghostty_features`, and `libghostty.kitty_graphics` tune the inner terminal renderer. Set `KITTWM_TERMINAL_BACKEND` (or `KITTWM_TERMINAL_APP`) to override the configured backend at runtime. Kittwm uses a shared virtual cell metric for unprobed surfaces so PTY, libghostty, browser, X11, and Quartz panes scale consistently under the same chrome/layout. HiDPI rendering is enabled by default (`KITTWM_HIDPI=1`), so the default metric is 16×32 px on Retina/4K-friendly setups; set `KITTWM_HIDPI=0` to use the legacy 8×16 px density, or override exact sizing with `KITTWM_NATIVE_CELL_WIDTH_PX` / `KITTWM_NATIVE_CELL_HEIGHT_PX`. Tiling gaps can be configured in pixels with `KITTWM_TILE_GAP_PX`, and spacing between the tiling area and header/footer can be configured with `KITTWM_HEADER_GAP_PX` and `KITTWM_FOOTER_GAP_PX`; pixel gaps are rounded up to whole terminal cells for the current density. For non-empty workspaces, the live footer starts with `mode:<label> · panes:<n> · focus:<window>(<title>)` (for example `mode:columns · panes:2 · focus:native-1(editor)`, `mode:floating · panes:3 · focus:native-2(shell)`, or `mode:fullscreen · panes:1 · focus:native-1`) before shortcut hints and the debug log path, and adds `drag:move:<window>` or `drag:reorder:<window>` while a title drag is active, so narrow terminals keep the current WM state visible without opening JSON status. Press `Ctrl-A ?` to toggle the native shortcut help overlay; the same catalog is available without entering the TUI as text via `kittwm shortcuts` / `kittwm --shortcuts` and as machine-readable JSON via `kittwm shortcuts-json` / `kittwm --shortcuts-json`, socket `SHORTCUTS_JSON`, or SDK `Kittwm::shortcuts()` / `Kittwm::shortcuts_json()` returning a typed shortcut catalog. The catalog also includes mouse title-drag hints for tiled reorder/floating reposition plus the pane-title marker legend so `▶`, `◆`, `⇄`, `↔`, `▣`, `≡`, `▲`, and `●` can be decoded without reading the long-form docs; SDK clients can use `ShortcutCatalog::tiled_title_drag_shortcut()`, `floating_title_drag_shortcut()`, `title_marker_legend()`, and related `has_*` helpers rather than hard-coding catalog entry ids. Press `Ctrl-A %` (or `Ctrl-A |` / `Ctrl-A v`) to split side-by-side, `Ctrl-A -` (or `Ctrl-A h` / `Ctrl-A "`) to split into stacked rows, `Ctrl-A e` to toggle the current split between vertical and horizontal, `Ctrl-A t` to toggle floating mode, `Ctrl-A f` to toggle fullscreen, `Ctrl-A +/-` to grow the focused pane weight, `Ctrl-A _` (or `Ctrl-A <`) to shrink it, `Ctrl-A [` / `Ctrl-A ,` and `Ctrl-A ]` / `Ctrl-A .` to move the focused pane, `Ctrl-A b` to balance all pane weights, `Ctrl-A Tab` (or `Ctrl-A n`) to cycle focus, and `Ctrl-A x` to close the focused pane; closing the last pane returns to the empty workspace, and keyboard input is routed only to the focused pane.


`kittwm doctor` reports the running executable path plus its canonical realpath when different, along with active graphics transport diagnostics, including selected transport, compression mode, tmux/remote classification, explicit override source, and any conservative fallback reason. Add `--probe-kitty` (or set `KITTUI_KITTY_PROBE=1`) to opt into a bounded interactive kitty `a=q` capability probe; this annotates diagnostics but normal rendering remains non-probing by default. The same diagnostic model is available to library callers as `kittui::TransportDiagnostics`. For raw RGBA frame uploads, `KITTUI_TRANSPORT=file` makes `Runtime::place_raw_frame` write a local tempfile and emit kitty `t=t,f=32` transfer grammar; `KITTUI_TRANSPORT=memory` uses a safe Linux `/dev/shm` POSIX shared-memory backing file and emits `t=s,f=32` when available, falling back to tempfile/direct streaming otherwise. kittui-kitty also exposes `f=32` file/shared-memory grammar for callers that manage their own files or POSIX shm objects.

The WM can also host kittwm-native apps (for example `kittwm-browser`, backed by headless Chrome screenshots + DevTools input) and X/Quartz capture backends. The long-term model is DISPLAY-like: native apps are ordinary binaries that inherit a kittwm socket/window context, can `kittwm replace ...` their current container, or can ask the socket to spawn a new app when not already inside a window.

## Same-machine SSH latency proof

The soft target for same-machine SSH interaction is **<200 ms** from an already-established SSH session to the next kittwm frame/control payload. Measure this without including SSH handshake time; cold one-shot `ssh localhost ...` timings are useful for startup diagnostics but are not the interactive-session RTT.

A reproducible loopback check is:

```sh
ssh -o BatchMode=yes localhost sh <<'EOF'
cd /path/to/kittui
for i in 1 2 3 4 5 6 7; do
  target/debug/kittwm showcase-composition-json >/tmp/kittwm-ssh-frame.json
  wc -c /tmp/kittwm-ssh-frame.json
 done
EOF
```

For `bd-84c3f5` on `ms-mac` (2026-05-28), a persistent `ssh localhost sh` session repeatedly running `target/debug/kittwm showcase-composition-json` measured:

- payload: **1634 bytes** per frame/composition JSON sample;
- warm median command-to-complete RTT: **7.1 ms**;
- warm p95 RTT: **25.1 ms**;
- median effective payload rate: **224 KiB/s**.

The first command on a fresh persistent shell was **214 ms**, so automation should either warm the session or record first-sample startup separately. Since warm RTT is well below 200 ms, no follow-up perf bead was filed from this proof.

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
kittwm --display :7 --serve
kittwm --display :7 --status
kittwm --display :7 --attach -c HELP_JSON
kittwm --display :7 --status-json
kittwm --display :7 --panes-json

# While a no-arg native kittwm session is running, inspect or create panes
# through its inherited/display-style socket.
kittwm --attach -c HELP
kittwm --attach -c HELP_JSON
kittwm --attach -c STATUS
kittwm --status-json
kittwm --panes
kittwm --panes-json
kittwm-top
kittwm-top --json
kittwm --apps-json
kittwm --apps-first htop
kittwm --spawn-pty htop
kittwm split columns htop
kittwm split native-1 rows 'bash -lc top'
kittwm --layout rows
kittwm --layout grid
kittwm --focus-pane native-2
kittwm --focus-next
kittwm --focus-prev
kittwm --move-pane focused last
kittwm --resize-pane focused +2
kittwm --balance-panes
kittwm --rename-pane native-2 editor
kittwm --send-line focused 'echo hello from controller'
kittwm --send-key focused ctrl-c
kittwm --send-key focused shift-tab
kittwm --send-key focused ctrl-left
kittwm --send-key focused shift-page-down
kittwm --send-key focused f12
kittwm --send-mouse focused press-left 7 9
kittwm --send-bytes-b64 focused aGkKAA==
kittwm --send-file focused ./payload.txt
kittwm --paste-file focused ./payload.txt
kittwm --read-text focused
kittwm --read-scrollback focused
kittwm --wait-text focused ready
kittwm --wait-output focused 'previous output'
kittwm --wait-text-ms 15000 focused 'build finished'
kittwm --wait-output-ms 15000 focused 'scrolled sentinel'
kittwm --save-session session.json
kittwm --restore-session session.json
kittwm --session-json
kittwm --attach -c 'RESTORE_SESSION_JSON {"layout":"rows","panes":[{"command":"htop","title":"htop","weight":1,"focused":true}]}'
kittwm --close-pane focused
```

Use `Ctrl-]` to exit the current native PTY/browser viewer. Explicit capture-backed demos remain available with `--backend fake|quartz|xvfb`. Native PTY snapshots and rendering honor common cursor movement/save/restore, DSR/CPR responses, application cursor-key mode, DEC Special Graphics line drawing, index/reverse-index, scroll regions/origin/autowrap modes, terminal reset controls, insert mode, erase, edit, basic, 256-color, and truecolor SGR style, OSC title, tab, focus reporting, cursor visibility, mouse-reporting mode toggles, and alternate-screen sequences so terminal applications remain legible and restore shell text after full-screen TUI modes. Host SGR mouse reports, including button-drag motion, are routed into the pane app area under the pointer with pane-local coordinates when the target app has requested compatible mouse modes; in floating mode, title-row left-drag is reserved by kittwm for moving the floating pane before app input routing, while other pane chrome cells only focus/raise without starting a drag. Focus cycles forward with `C-a Tab` / `C-a n` and backward with `C-a p`. Focused floating panes can also be nudged one cell at a time with `C-a w/a/s/d`, lowered/raised in the stack with `C-a {` / `C-a }`, reset to their generated floating position with `C-a r`, `kittwm reset-position [WINDOW]`, or `--reset-pane-offset WINDOW`, and reset all floating positions at once with `C-a R`, `kittwm reset-positions`, or `--reset-all-pane-offsets`. Pane title rows use a leading `▶` marker for the focused pane so focus state remains visible even when color/style cues are unavailable; the actively dragged pane adds `◆` while kittwm is consuming title-drag motion; tiled title rows show a compact `⇄` reorder marker when they can be dragged to reorder panes, and resized tiled titles add `↔` for non-default weights. Fullscreen title rows show `▣` so the maximized layout remains visible even when footer state is clipped. In floating mode, title rows show a compact `≡` handle marker to advertise the title-drag affordance, `▲` on the topmost pane, and `●` when a pane has been dragged or nudged away from its generated floating position.

Native `PANES_JSON` includes per-pane `window`, `title`, `focused`, `weight`, visual stack metadata (`stack_index`, `stack_top`), floating offset metadata (`floating_dx`, `floating_dy`, `floating_moved`), title drag affordance metadata (`title_draggable`, `title_drag_kind`, `title_drag_col`, `title_drag_row`, `title_drag_active`) for both tiled title reorder and floating title reposition interactions, optional process metadata (`pid`, `command`), cursor metadata (`cursor_col`, `cursor_row`, `cursor_visible`), bracketed-paste mode (`bracketed_paste`), application cursor-key mode (`application_cursor_keys`), mouse-reporting modes (`mouse_reporting`, `mouse_button_motion`, `mouse_all_motion`, `mouse_sgr`), and, once a live session has rendered at least one frame, resolved title/app cell geometry: `x`, `y`, `cols`, `rows`, `app_x`, `app_y`, `app_cols`, and `app_rows`. Native `STATUS_JSON` mirrors the same detail in `focused_pane` and `panes_detail`. `CHROME_JSON` reports the current chrome/workspace reservation directly as `workspace`, `top_bar_rows`, and `tilable_rows`; `STATUS_JSON` / `PANES_JSON` also embed compatible chrome metadata. SDK clients can read the embedded metadata from typed status/panes responses or call `Kittwm::chrome()` / `Kittwm::chrome_json()` to read `CHROME_JSON` as `ChromeReservationStatus`. SDK `PanesStatus` / `Status` offer convenience accessors for layout mode (`is_floating_layout`, `is_fullscreen_layout`, `is_tiled_layout`), focused, fullscreen-pane/focused-is-fullscreen, topmost, focused-is-topmost, focused-is-resized/focused weight-chip label, focused command/pid/status-chip labels, focused-is-moved, focused-title-draggable/focused-is-title-drag-active, focused title marker prefix/active-drag/reported variants, focused title drag interaction kind (`TitleDragKind`, `NativePaneDetail::parsed_title_drag_kind`, `focused_title_drag_reorders_pane`, `focused_title_drag_repositions_pane`, backed by `title_drag_kind` when reported), focused title-drag cells, focused dirty-frame state/upload-skipped/status-label/chip-label/change-label/change-percent helpers, title-draggable/reorder/reposition panes, active title-drag panes/window/raw-kind/typed-kind/cell helpers, moved floating panes, clean/dirty frame panes, and resized/non-default-weight panes, while `NativePaneDetail` offers pure convenience accessors for outer/app bounds, visual stack index/topmost state, non-default weight state, floating offset/non-zero offset state (`floating_moved` when reported), title marker prefix/active-drag variant, active title-drag state, title-row draggability/reported or derived title drag start-and-delta cells, cursor position/visibility, bracketed paste, application cursor-key mode, mouse modes, dirty-frame clean/skipped/change metrics, pane status-chip label helpers, and transport diagnostics presence. Live pane status chips show command/process/frame state, show dirty-frame tile ratios as `frame:<changed>/<total>`, show `frame:clean` for skipped or zero-change frames, and include `wt:<n>` when a pane has a non-default resize weight. Resized tiled pane titles also show a compact `↔` marker beside the focus marker so non-default weights remain visible in the primary pane chrome. The text `PANES` reply includes process/cursor metadata plus the same geometry as `layout=x,y CxR app=x,y CxR` for simple shell inspection. `--socket PATH` and `--display DISPLAY` target an explicit socket or DISPLAY-like token for any command without exporting environment variables. CLI wrappers `--status-json`, `--panes`, `--panes-json`, and `--session-json` expose these inspection surfaces without raw protocol strings. Pane control wrappers (`--spawn-pty`, `kittwm split [WINDOW] columns|rows|grid CMD`, `--focus-pane`, `--focus-next`, `--focus-prev`, `--close-pane`, `--layout`, `--move-pane`, `kittwm raise [WINDOW]`, `kittwm lower [WINDOW]`, `kittwm nudge [WINDOW] DX DY`, `--nudge-pane`, `kittwm reset-position [WINDOW]`, `--reset-pane-offset`, `kittwm reset-positions`, `--reset-all-pane-offsets`, `--resize-pane`, `--balance-panes`, `kittwm reset-weights`, `--reset-pane-weights`, and `--rename-pane`) map to the same native socket control plane; `--layout grid` arranges four panes as a practical 2x2 grid instead of a flat all-row/all-column split. SDK clients can use `Kittwm::focus_next`, `focus_prev`, `layout(LayoutMode)`, `split_pane`, `split_focused`, `balance_panes`, `SurfaceHandle::move_pane(MoveDirection)`, `SurfaceHandle::raise()`, `SurfaceHandle::lower()`, `SurfaceHandle::nudge(dx, dy)`, `SurfaceHandle::reset_floating_offset()`, `SurfaceHandle::reset_position()`, `Kittwm::reset_all_positions()`, and `Kittwm::reset_all_floating_offsets()` for the `ControlWindow`-gated focus/layout/move/balance/stack/floating-offset subset without raw `FOCUS_NEXT`/`FOCUS_PREV`/`LAYOUT`/`SPLIT_PANE`/`BALANCE_PANES`/`MOVE_PANE`/`NUDGE_PANE`/`RESET_PANE_OFFSET` strings; `kittwm reset-weights` and `--reset-pane-weights` are CLI aliases for `BALANCE_PANES`. App discovery wrappers (`--apps-json`, `--apps-first`, and `--apps-launch-first`) expose the socket app catalog, and SDK clients can use typed `Kittwm::apps`, `app_first`, and `app_launch_first` helpers for the same read/launch path. Controllers can inject pane input with `SEND_TEXT <window|focused> <text>`, `SEND_LINE <window|focused> <text>`, `SEND_KEY <window|focused> <key>` for named keys such as `ctrl-c`, `escape`, `shift-tab`/`backtab`, arrows plus `shift-left`/`alt-left`/`ctrl-left`-style modified arrows, `page-up`/`page-down` plus modified names such as `shift-page-down`, `ctrl-home`/`shift-end`, and `f5`-`f12`, `SEND_MOUSE <window|focused> <event> <col> <row>` for SGR mouse events (`press-left`, `press-middle`, `press-right`, `release`, `move`, `move-left`, `move-middle`, `move-right`, `scroll-up`, `scroll-down`) when the pane has requested compatible mouse modes, `SEND_BYTES_B64 <window|focused> <base64>` for arbitrary bytes, or `PASTE_BYTES_B64 <window|focused> <base64>` for paste payloads that automatically wrap with bracketed-paste markers when the pane has enabled DEC `?2004` mode. First-party semantic SDK apps can publish their current component tree with `SEMANTIC_PUBLISH <window|focused> <snapshot-json>`; subsequent `SEMANTIC_SNAPSHOT` calls prefer that published tree over the PTY text-area fallback. Semantic component surfaces are documented in `docs/kittwm-semantic-surfaces.md`, with a runnable workflow in `docs/kittwm-semantic-quickstart.md`; the native socket exposes semantic snapshot/publish/action/focus commands as the first control-plane layer. Nested app side effects are also modeled as semantic `SurfaceEvent`s for the SDK/runtime: title changes, bells, OSC 52 clipboard writes, and basic OSC notification requests. OSC 52 clipboard writes from nested apps are sanitized and forwarded to the host terminal so Ghostty-compatible clipboard integration still works even though kittwm renders panes itself. They can read pane screen text plus cursor metadata with `READ_TEXT <window|focused>` or `READ_TEXT_JSON <window|focused>`, inspect lines pushed off-screen with `READ_SCROLLBACK <window|focused>` or `READ_SCROLLBACK_JSON <window|focused>`, and block automation until screen output appears with `WAIT_TEXT <window|focused> <needle>` / `WAIT_TEXT_MS <window|focused> <ms> <needle>` or screen-plus-scrollback output appears with `WAIT_OUTPUT <window|focused> <needle>` / `WAIT_OUTPUT_MS <window|focused> <ms> <needle>`. JSON wait variants `WAIT_TEXT_JSON`, `WAIT_TEXT_JSON_MS`, `WAIT_OUTPUT_JSON`, and `WAIT_OUTPUT_JSON_MS` return structured match objects while preserving the existing text wait replies. SDK clients can use typed `TextSnapshot`, `ScrollbackSnapshot`, `SurfaceHandle::read_text`, `read_scrollback`, `wait_text[_ms]`, and `wait_output[_ms]` helpers for the same read-capability-gated automation path; typed `wait_text_match[_ms]` / `wait_output_match[_ms]` variants parse the existing `MATCH_TEXT` / `MATCH_OUTPUT` daemon replies into `WaitMatchKind` and `WaitMatch`, while `wait_text_match_json[_ms]` / `wait_output_match_json[_ms]` use the JSON wait daemon commands and return the same `WaitMatch` shape. Existing raw string helpers remain available. SDK input helpers include `SurfaceHandle::send_text`, `send_line`, `send_key`, `send_bytes`, `send_bytes_b64`, `paste_bytes`, `paste_bytes_b64`, and `send_mouse(MouseEvent, col, row)`, all gated by `SendInput` and mapped to the existing text/key/`SEND_BYTES_B64`/`PASTE_BYTES_B64`/`SEND_MOUSE` socket verbs. CLI wrappers `--send-text`, `--send-line`, `--send-key`, `--send-mouse`, `--send-bytes-b64`, `--send-file`, `--paste-file`, `--read-text`, `--read-text-json`, `--read-scrollback`, `--read-scrollback-json`, `--wait-text`, `--wait-text-json`, `--wait-text-ms`, `--wait-text-json-ms`, `--wait-output`, `--wait-output-json`, `--wait-output-ms`, and `--wait-output-json-ms` provide the same automation primitives without spelling socket protocol verbs directly; the JSON read wrappers map to `READ_TEXT_JSON` and `READ_SCROLLBACK_JSON` for structured text/cursor/scrollback snapshots, and the JSON wait wrappers map to the `WAIT_*_JSON[_MS]` verbs for structured match metadata. First-party SDK apps include `kittwm-terminal` for spawning/replacing terminal surfaces plus typed `--status`/`--events-ms` inspection, `kittwm-top` for SDK-backed pane/process introspection in terminal or hosted kittwm surfaces, and `kittwm-launch` for backend-aware launching with typed app discovery, URL/browser auto-selection, `--dry-run`, and `--status`. SDK clients can also request browser surfaces with `SurfaceSpec::browser(...)`; the v0 socket command path still uses `SPAWN_PTY kittwm-browser ...`, but live kittwm recognizes that first-party command and hosts a direct headless-browser capture surface instead of a terminal PTY. Dedicated browser/X/Quartz surface protocols remain future work. `SESSION_JSON` provides a persistence-oriented manifest containing layout axis, focus, pane order, titles, commands, weights, and floating offsets; `RESTORE_SESSION_JSON <json>` replaces the current native panes from that manifest. The CLI wrappers `--save-session PATH|-` and `--restore-session PATH|-` avoid manual shell quoting for this flow, and SDK clients can use typed `SessionManifest` / `SessionPane` with `Kittwm::session()` and `Kittwm::restore_session(&manifest)` instead of raw protocol strings. Session reads are treated as low-risk read capability operations; restore is a create/control mutation. `kittui wm-session session.json -w 120 -h 30 --scene-json` turns the same manifest into kittwm chrome preview scenes for shell/external renderer workflows. `EVENTS [ms]` opens a bounded JSON-lines subscription: the first line is a `status` snapshot, subsequent lines carry `status_changed`, `pane_opened`, `pane_closed`, `pane_changed`, `pane_resized`, `pane_input_sent`, `pane_frame_presented`, `focus_changed`, `layout_changed`, semantic events, and surface side-effect events (`surface_title_changed`, `surface_bell`, `surface_clipboard_set`, `surface_notification`) with `schema_version`, monotonic `seq`, `at_ms`, optional `window`, and `detail`; `pane_resized` detail includes old/new outer and app bounds when available, `pane_input_sent` reports conservative non-sensitive metadata for socket-injected input, and `pane_frame_presented` reports frame metadata without pixel payloads; the stream ends with `END` after the requested timeout (default 5000 ms, max 60000 ms), and CLI wrappers `--events` / `--events-ms MS` expose it without raw protocol strings. SDK clients can either collect a bounded batch with `events_ms` or iterate the same batch with `KittwmEventIter` via `events_iter_ms` / `event_iter_ms`. SDK event consumers can use `KittwmEvent::envelope()`, `unknown_raw()`, and `EventEnvelope::detail_str` / `detail_bool` / `detail_u64` / `detail_i64` / `pane_detail` / `pane_bounds` / `pane_app_bounds` / pane size-delta / frame-presented render metadata/dirty-tile label convenience accessors instead of matching every variant for common metadata. `surface_clipboard_set` reports the existing OSC52 base64 payload and host OSC52 forwarding remains unchanged. `CLIPBOARD_JSON` adds a policy-gated, cache-only read of the latest nested-app OSC52 write: by default it returns `allowed:false` without payload, and only `KITTWM_CLIPBOARD_READ=allow|1|true|yes` exposes the cached selection/source/payload metadata. It does not read the host OS clipboard. `HELP_JSON` is the machine-readable catalog for the socket command set; `kittwm --help-json` prints that catalog without spelling a raw socket command, and SDK clients can read the same data through read-capability-gated `HelpCatalog` / `HelpCommand` helpers via `Kittwm::help_catalog()` or `Kittwm::help()`. `ClientCapabilities` presets (`none`, `restricted`, `inspect_only`, and `automation`) are local SDK scopes for client-side least privilege; they do not replace future daemon-issued credentials or per-client runtime enforcement.

## Architecture

`kittwm architecture-json` emits the current machine-readable architecture
contract for the kitty-graphics-backed WM. The same schema is available to Rust
apps as `kittwm_sdk::ArchitectureContract::current()`. Treat that artifact as
the boundary checklist for implementation work: the SDK/control plane names and
authorizes surface operations, the tiling engine owns drawable geometry, surface
adapters capture or synthesize app frames, the decoration renderer paints kittui
scene chrome, and `kittui-kitty` only encodes transport/placement grammar.

The intended separation of concerns is:

- **SDK/control plane (`kittwm-sdk`)**: typed app-facing vocabulary such as
  `SurfaceSpec::terminal`, `SurfaceSpec::browser`, chrome reservation requests,
  status/panes/chrome/events, and semantic snapshots. It must not decide pane
  geometry or emit kitty graphics.
- **Tiling engine (native session layout)**: consumes reported terminal
  cols/rows plus top/bottom/side/gap reservations and produces disjoint outer
  and app bounds. It must not upload images or draw decorations.
- **Surface renderer (`NativeSurface` adapters + `kittui::Runtime`)**: captures
  PTY/browser/native surfaces, fits them to allocated app cells, and places
  kitty images with explicit placement/z-plane options. It must not allocate
  tiles or consume SDK policy directly.
- **Decoration renderer (`kittui-affordances` + kittwm chrome helpers)**:
  renders top bar, pane labels/borders, footer, and overlays as labelled kittui
  scenes above app surfaces. It must not resize apps or route app input.
- **Kitty compositor (`kittui-kitty`)**: encodes upload, placement, deletion,
  and transport grammar. It must not know about workspaces, panes, or first-party
  app policy.

The standard composition order is app surfaces at z-index `0`, decorations at
z-index `20`, and overlays above them. First-party apps should enter through SDK
surface types (`kittwm-terminal` via `SurfaceSpec::terminal`, `kittwm-browser`
via `SurfaceSpec::browser`) or explicit chrome contracts (`kittwm-bar` via
`ChromeReservationRequest`/`CHROME_JSON`) so kittwm remains a usable window
manager rather than a collection of renderer special cases.

`kittwm-bar` is the first-party example of that chrome-app contract. It can
render the standard top bar as a kittui/kitty graphics scene with `kittwm-bar
--kitty` (alias `--graphics`), or emit the same model as text, JSON, or
`--scene-json`. When running inside or alongside a live kittwm session, use
`kittwm-bar --reserve` to request a one-row top reservation through the typed
SDK (`ChromeReservationRequest::top_bar(1)`); use `kittwm-bar --release` to
clear it back to the daemon default. Its JSON output includes the full drawable
reservation reported by `CHROME_JSON`: `top_bar_rows`, `bottom_bar_rows`,
`left_cols`, `right_cols`, `gap_cols`, `gap_rows`, optional `owner`, and
`tilable_rows`. That makes external/statusline bars regular SDK clients instead
of hard-coded renderer exceptions.

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
