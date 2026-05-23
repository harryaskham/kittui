# Kitty response reading and capability probing plan

`bd-02ef7b` tracks the planning slice for the remaining interactive kitty graphics conformance gap: reading terminal responses and using kitty `a=q` capability queries without destabilizing normal rendering or stealing input from hosted applications.

Today kittui mostly relies on environment detection (`TerminalInfo::detect`) plus explicit overrides. That is intentionally safe, but it means the runtime cannot distinguish “kitty-compatible but unknown terminal” from “optimistically assumed kitty”, and it cannot surface actual `OK` / `ENOENT` / capability-query responses in diagnostics. This plan defines a conservative implementation path.

## Goals

- Add an opt-in, bounded response reader for kitty graphics replies.
- Use kitty `a=q` queries to refine `TerminalInfo::detect` and `TransportDiagnostics` when probing is safe.
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

The reader must be bounded by short deadlines. Suggested defaults:

- probe timeout: 50-150 ms per query in interactive startup/diagnostic contexts;
- total probe budget: <= 500 ms for `kittwm doctor` or explicit probe commands;
- render path budget: zero by default; render code consumes cached probe results only.

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

Add pure encoder helpers first:

- `query_capabilities(query_id) -> String` emitting kitty `a=q` grammar;
- optional typed request id wrapper so probes can ignore unrelated replies;
- exact grammar tests in `crates/kittui-kitty`.

This crate should remain I/O-free. It only builds escape sequences and parses known response fragments.

### 2. `kittui-core` / terminal probe model

Add small data types near `TerminalInfo`:

```rust
struct KittyProbeResult {
    attempted: bool,
    supported: Option<bool>,
    response: Option<String>,
    error: Option<KittyProbeError>,
    elapsed_ms: u64,
}
```

Then extend diagnostics rather than replacing detection:

- `TerminalInfo::detect()` remains environment-only and fast.
- A new opt-in function such as `TerminalInfo::detect_with_probe(...)` or `KittyProbe::run(...)` can refine `supports_kitty` / placeholder support.
- `TransportDiagnostics` includes whether probe data was used or unavailable.

### 3. Foreground response reader

Implement a small Unix foreground-terminal reader that can:

- temporarily put stdin in nonblocking/raw-compatible read mode only when requested;
- write a query to stdout/stderr-selected terminal output;
- read until a matching kitty response, timeout, or EOF;
- restore terminal mode even on errors.

Prefer a testable abstraction so parser/timeout behavior can be validated with a pseudo-terminal or in-memory stream. Avoid global background reader threads in the first implementation.

### 4. Diagnostics integration

Expose probe results first through diagnostics surfaces:

- `kittwm doctor`: include static detection, explicit overrides, tmux/remote state, whether a probe was attempted, and the result.
- Optional CLI flag/env such as `KITTUI_KITTY_PROBE=1` or `kittwm doctor --probe-kitty`.
- Do not auto-enable probes in normal rendering until diagnostics have proven stable across terminals.

### 5. Transport policy integration

After the response reader is stable, adaptive transport can consume probe results:

- if probe positively says unsupported: prefer pure-terminal fallback despite optimistic env heuristics;
- if probe positively says supported: allow `supports_kitty=true` in unknown-but-compatible terminals;
- if probe is unknown/timeout: keep existing environment heuristics and explicit overrides.

## `a=q` query details

Implementation should start with one minimal query:

1. emit a kitty graphics capability query with a unique image/query id;
2. request a verbose response;
3. parse known success/error tokens into a typed result;
4. ignore unrelated bytes until timeout.

The parser should be deliberately permissive around terminal-specific fields but strict enough to avoid treating arbitrary app output as a positive probe. A matching id/token should be required.

## Security and privacy

- Do not include terminal response payloads in durable logs by default; diagnostics may show compact status/reason.
- Never read from stdin behind an app without explicit opt-in.
- Avoid probing remote/SSH sessions unless the operator explicitly asks. In remote topologies, local file/shm capability is separate from kitty support.
- Respect explicit environment overrides. If `KITTUI_TRANSPORT` or `KITTWM_NATIVE_RENDERER` forces a path, probing should be advisory only.

## Follow-up implementation beads

- `bd-f9730c`: add pure `a=q` query encoder/parser helpers in `kittui-kitty` with exact grammar/parser tests.
- `bd-049875`: add a timeout-bounded foreground terminal response reader abstraction with pseudo-terminal/in-memory tests and no render-loop integration.
- `bd-11e67a`: add opt-in `kittwm doctor --probe-kitty` / environment-gated probe diagnostics and extend `TransportDiagnostics` with probe status.

These should land separately: encoder/parser is low-risk; terminal I/O needs careful validation; diagnostics integration should remain opt-in until stable.
