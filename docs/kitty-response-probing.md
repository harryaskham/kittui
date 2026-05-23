# Kitty response reading and capability probing

`bd-02ef7b` planned the interactive kitty graphics conformance gap: reading terminal responses and using kitty `a=q` capability queries without destabilizing normal rendering or stealing input from hosted applications. The first stack is now landed: pure `a=q` encoder/parser helpers in `kittui-kitty`, a bounded response-reader helper in `kittui-core`, and opt-in `kittwm doctor --probe-kitty` diagnostics.

Today normal kittui rendering still relies on environment detection (`TerminalInfo::detect`) plus explicit overrides. That is intentionally safe: probing is available for diagnostics, but render loops do not block waiting for terminal responses by default.

## Current landed surface

- `kittui_kitty::query_capabilities(query_id, transport)` emits kitty `a=q` grammar and supports tmux passthrough wrapping.
- `kittui_kitty::parse_response(...)` parses collected graphics responses into `KittyResponse`, `KittyResponseStatus`, and `KittyResponseParseError` without doing any terminal I/O.
- `kittui_core::terminal::read_kitty_response(...)` reads from an already-prepared foreground stream until a caller predicate matches, timeout/EOF occurs, or a byte limit is exceeded.
- `kittwm doctor --probe-kitty` and `KITTUI_KITTY_PROBE=1 kittwm doctor` opt into a bounded interactive probe and report status through transport diagnostics.
- Normal render paths remain non-probing by default and continue using static detection/overrides.

## Goals

- Keep kitty graphics probing opt-in and diagnostics-first.
- Use kitty `a=q` queries to annotate `TransportDiagnostics` when probing is safe.
- Never consume normal application input from kittwm child panes or the operator's shell.
- Avoid blocking render loops: all reads must be timeout-bounded and failure-tolerant.
- Preserve the existing environment/override detection path as the default fallback.

## Non-goals

- Do not make every render path synchronously wait for a terminal response.
- Do not probe through arbitrary nested PTYs by default.
- Do not require response reading for basic image placement; existing static heuristics and overrides must keep working.
- Do not treat `a=q` as a security boundary. It is a capability hint, not an authorization mechanism.

## Response-reading constraints

### Non-blocking and timeout behavior

The reader is bounded by short deadlines. Current/target budgets:

- current doctor probe budget: 500 ms for `kittwm doctor --probe-kitty` / `KITTUI_KITTY_PROBE=1`;
- response reader default timeout: 250 ms with a bounded byte limit;
- render path budget: zero by default; render code does not probe.

Timeouts should be ordinary “unknown” results, not hard failures. Diagnostics should distinguish timeout, malformed response, negative kitty response, and probing disabled.

### No TTY theft from apps

Response reading can only happen when the process owns the terminal input stream being read. Safe initial contexts:

- `kittui`/`kittwm doctor` style commands run directly in the foreground terminal;
- explicit diagnostic commands that document they will briefly read terminal responses;
- test harnesses using a controlled pseudo-terminal.

Unsafe/default-off contexts:

- inside a kittwm-managed child PTY where reads could consume bytes intended for the nested app;
- while a fullscreen TUI/browser/native app is active;
- background render loops that do not own stdin.

For kittwm, response reading should live in the host shell/runtime side, not in arbitrary pane apps. Child panes should receive their own PTY bytes normally.

### Multiplexer behavior

Under tmux, kitty graphics commands are wrapped in DCS passthrough. Response routing differs by terminal and tmux version, and responses may be delayed, dropped, or rewritten.

Initial policy:

- keep probing disabled by default inside tmux except for explicit diagnostics;
- when explicitly enabled, emit through the same `Transport::TmuxPassthrough` wrapper used for graphics commands;
- read only from the outer terminal/foreground diagnostic process;
- report tmux as a separate diagnostic dimension so “timeout under tmux” does not imply “no kitty support”.

### Quiet mode interaction

Most normal upload commands use `q=2`/`Quiet::SuppressAll` to avoid leaking replies into the terminal. Capability probes need responses, so query commands must use verbose/response-producing mode and unique ids. Normal renderer uploads should continue suppressing replies unless a dedicated diagnostic mode asks for verbose response capture.

## Proposed architecture

### 1. `kittui-kitty` query encoder

Landed pure encoder/parser helpers:

- `query_capabilities(query_id, transport) -> String` emits kitty `a=q` grammar;
- `parse_response(input)` parses captured graphics response escapes and preserves ids/raw body;
- exact grammar/parser tests live in `crates/kittui-kitty`.

This crate remains I/O-free. It only builds escape sequences and parses known response fragments.

### 2. `kittui-core` / terminal probe model

Transport diagnostics now have probe fields populated by `kittwm doctor` when probing is explicitly requested:

- `TerminalInfo::detect()` remains environment-only and fast.
- `TransportDiagnostics` can carry probe status, optional support result, error/note, and elapsed time.
- `kittwm doctor --probe-kitty` / `KITTUI_KITTY_PROBE=1` annotates diagnostics without changing render policy.

### 3. Foreground response reader

The landed reader is deliberately small and testable:

- `read_kitty_response` reads from an already-prepared stream and never writes query bytes itself;
- callers provide the match predicate, timeout, byte limit, and poll interval;
- the helper returns matched/timeout/EOF/byte-limit status plus captured bytes and elapsed time;
- `kittwm doctor --probe-kitty` owns the foreground-terminal setup and restores terminal state around the probe.

There are still no global background reader threads and no render-loop integration.

### 4. Diagnostics integration

Probe results are exposed through diagnostics surfaces:

- `kittwm doctor` includes static detection, explicit overrides, tmux/remote state, whether a probe was attempted, and the result.
- `kittwm doctor --probe-kitty` or `KITTUI_KITTY_PROBE=1 kittwm doctor` performs the bounded interactive probe.
- Probes are not auto-enabled in normal rendering.

### 5. Transport policy integration

Adaptive transport still treats probe results as diagnostics-only:

- if probe positively says unsupported, `doctor` reports that result but normal rendering still follows existing policy unless a future bead wires the probe into selection;
- if probe positively says supported, `doctor` reports support for unknown-but-compatible terminals;
- if probe is unknown/timeout, diagnostics preserve the timeout/unknown state and existing environment heuristics remain authoritative.

## `a=q` query details

The current diagnostic query flow:

1. emits a kitty graphics capability query with a unique image/query id;
2. omits `q=` so the terminal can respond;
3. reads with a bounded foreground response reader;
4. parses known success/error/capability tokens into a typed result;
5. requires the matching query id before treating the response as a match.

The parser is deliberately permissive around terminal-specific fields but strict enough to avoid treating arbitrary app output as a positive probe.

## Security and privacy

- Do not include terminal response payloads in durable logs by default; diagnostics may show compact status/reason.
- Never read from stdin behind an app without explicit opt-in.
- Avoid probing remote/SSH sessions unless the operator explicitly asks. In remote topologies, local file/shm capability is separate from kitty support.
- Respect explicit environment overrides. If `KITTUI_TRANSPORT` or `KITTWM_NATIVE_RENDERER` forces a path, probing should be advisory only.

## Landed implementation beads

- `bd-f9730c`: pure `a=q` query encoder/parser helpers in `kittui-kitty` with exact grammar/parser tests.
- `bd-049875`: timeout-bounded foreground-stream response reader abstraction with tests and no render-loop integration.
- `bd-11e67a`: opt-in `kittwm doctor --probe-kitty` / `KITTUI_KITTY_PROBE=1` diagnostics and `TransportDiagnostics` probe status.

Future work should decide whether proven probe results ever feed default transport selection. Until then, probing remains an explicit diagnostic path.
