# Adaptive graphics transport plan

`bd-3c0dd1` tracks the policy gap between kittui's current transport hints and the high-rate frame workloads produced by kittwm. Today `TerminalInfo::detect()` mostly chooses `Direct` or `TmuxPassthrough` from environment variables, while callers can override with `Transport::File` or `Transport::Memory`. That is enough for static panels, but a window-manager loop needs an explicit policy that weighs safety, terminal topology, payload size, and frame rate before choosing how each upload travels.

## Goals

- Keep the common rendering path simple: callers ask for a graphics placement; policy picks the safest fast transport.
- Preserve the tmux-safe fallback that avoids unbounded passthrough payload growth for high-rate native panes.
- Prefer zero-copy or file-backed local transfers for large local frames once those paths are implemented.
- Use zlib only when it reduces total wall-clock cost, not merely when it reduces bytes.
- Make operator/developer overrides explicit and inspectable.

## Inputs

Transport selection should be based on a small `TransportContext` derived from `TerminalInfo`, environment, and per-upload metadata:

| Input | Source | Why it matters |
|---|---|---|
| Kitty graphics support | `TerminalInfo.supports_kitty`, probes, host overrides | Non-kitty terminals must use a pure terminal/text fallback. |
| Unicode placeholder support | `TerminalInfo.supports_unicode_placeholders` | Determines whether image placement can be represented in normal cell output. |
| Multiplexer state | `TMUX`, `TERM_PROGRAM=tmux`, explicit host hint | tmux passthrough is correct but expensive for continuous large binary payloads. |
| Local vs remote | `SSH_CONNECTION`, `SSH_CLIENT`, `KITTUI_REMOTE`, host hint | Shared memory/file transfer is only safe when the terminal process can read the same local resources. Remote/SSH should prefer direct streaming. |
| Local filesystem/shm availability | platform, sandbox, `$TMPDIR`, future probe result | `t=f`/`t=s` require paths or shm names readable by the terminal, with cleanup semantics. |
| Payload kind | PNG scene, raw RGBA frame, animation frame, delete/place-only | Raw RGBA WM frames are high bandwidth; PNG scenes are already compressed. |
| Payload size | encoded byte length or `width * height * 4` | Compression and file/shm thresholds should be size-aware. |
| Frame cadence | expected FPS / recent uploads per image id | Continuous streams should avoid tmux passthrough and repeated base64 where possible. |
| Security policy | `KITTY_PUBLIC_KEY`, sandbox hints, path allowlists | File/shm modes expose local resource names and must not cross trust boundaries unexpectedly. |

## Policy order

The selector should return both a `Transport` and a reason string for diagnostics. A good first implementation can use this deterministic order:

1. **Explicit override wins.** `KITTUI_TRANSPORT=direct|tmux|file|memory|auto` (and a matching CLI/config option) bypasses automatic ranking except for impossible modes. Impossible override requests should produce a clear error or warning, not silently downgrade.
2. **No kitty support -> pure terminal fallback.** For kittwm, this means `KITTWM_NATIVE_RENDERER=terminal` behavior; for library placement APIs, continue returning the existing unsupported-terminal error.
3. **Remote/SSH -> direct stream.** If the terminal is remote from the process, local file or shm names are not usable by the terminal. Prefer `Direct` unless tmux passthrough is the only way to reach kitty graphics.
4. **tmux + high-rate/large frame -> pure terminal fallback by default.** Current kittwm defaults are correct: avoid streaming large kitty graphics through tmux unless the operator explicitly forces graphics. Low-rate static scenes may still use `TmuxPassthrough`.
5. **Local large raw frame -> memory, then file.** For local terminals with kitty support and available shm/file resources, prefer `Memory` for raw RGBA frames above the configured threshold. Fall back to `File` when shm is unavailable, then `Direct`.
6. **Local medium PNG/static scene -> direct or file.** PNG payloads are already compressed; use `Direct` for small payloads and `File` for large local payloads where base64/chunk overhead dominates.
7. **Compression is orthogonal.** After selecting transport, choose `CompressionMode::Zlib` only for raw/direct payloads where a size/cadence heuristic predicts a win. Do not zlib-compress already-compressed PNG uploads by default.

## Overrides and defaults

Suggested environment/config surface:

| Name | Values | Default | Notes |
|---|---|---|---|
| `KITTUI_TRANSPORT` | `auto`, `direct`, `tmux`, `file`, `memory` | `auto` | Library-wide transport override. |
| `KITTUI_TRANSPORT_MIN_FILE_BYTES` | integer bytes | `262144` | First-pass threshold for local file transfer. |
| `KITTUI_TRANSPORT_MIN_MEMORY_BYTES` | integer bytes | `524288` | First-pass threshold for shared memory transfer. |
| `KITTUI_KITTY_COMPRESSION` | `off`, `zlib`, `auto` | `off` initially, later `auto` for raw frames | Existing variable; `auto` should become threshold-based rather than unconditional zlib. |
| `KITTUI_ZLIB_MIN_BYTES` | integer bytes | `262144` | Only considered for raw/direct payloads. |
| `KITTWM_NATIVE_RENDERER` | `auto`, `terminal`, `kitty`, `graphics` | `auto` | `auto` remains tmux-safe; `kitty`/`graphics` explicitly opt into passthrough risk. |
| `KITTUI_REMOTE` | `auto`, `0`, `1` | `auto` | Lets tests/hosts override SSH heuristic. |

Defaults should be conservative: `Direct` for ordinary local/remote kitty sessions, pure-terminal kittwm fallback inside tmux for high-rate panes, and no automatic file/shm until cleanup and permissions are proven.

## Raw RGBA and zlib safeguards

`Runtime::place_raw_frame` is the first hot path that needs adaptive policy. It currently uses kitty `f=32` raw RGBA uploads to avoid PNG encode cost, but raw bytes can be much larger than PNG scenes. The policy should therefore evaluate `width * height * 4`, recent FPS, and whether the same image id is being re-uploaded repeatedly.

For zlib:

- `off`: preserve current uncompressed raw upload behavior.
- `zlib`: force `o=z` for raw/direct payloads.
- `auto`: compress only when raw bytes exceed `KITTUI_ZLIB_MIN_BYTES` and transport is `Direct` or `TmuxPassthrough`; skip for `File`/`Memory` unless measurement proves a win.

For tmux:

- static/low-rate scene uploads may continue through `TmuxPassthrough`.
- high-rate raw frames should default to `KITTWM_NATIVE_RENDERER=terminal` behavior unless explicitly overridden.
- diagnostics should say when this fallback was selected so users understand why graphics are not active.

## Follow-up implementation map

- `bd-67a477`: implement local shared-memory/file-backed transfer for `upload_still_rgba` and `Runtime::place_raw_frame`, including cleanup and fallback to direct streaming.
- `bd-e15ef8`: replace unconditional `KITTUI_KITTY_COMPRESSION=auto` zlib behavior with threshold-based compression for raw frames, plus tests for small/large payload decisions.
- `bd-883864`: expose transport decision diagnostics in `kittwm doctor`, re-export `TransportDiagnostics` for callers, and keep the policy selector testable with caller-supplied environment data.

These follow-ups should land as separate implementation beads because this document is the policy baseline, not the runtime selector itself.
