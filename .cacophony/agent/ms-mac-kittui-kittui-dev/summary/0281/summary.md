# Session summary — TypeScript render_json bytes API

## Goal

Expose the FFI render-only PNG path to TypeScript/JavaScript hosts so they can render scene previews/artifacts without terminal placement escapes.

## Bead(s)

- `bd-fe41b2` — bindings-ts: expose render_json PNG bytes

## Before state

- Failing tests: none known.
- Relevant gap: Rust, FFI, Python, and CLI exposed render-only PNG APIs, but the TypeScript koffi binding only exposed terminal placement APIs.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `npm test --prefix bindings/ts` passed.
  - `git diff --check` passed.
- Context: The TS binding now wires `kittui_render_json` and `kittui_bytes_free`, adds `Kittui.render(scene): Uint8Array`, frees returned byte buffers, and includes render failure details through the shared last-error helper. Declarations and README were updated, and fake-lib tests cover render success/free and failure paths.

## Diff summary

- Code/content commit: `7906459`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `bindings/ts/src/index.js`, `bindings/ts/src/index.d.ts`, `bindings/ts/test/koffi.test.js`, `bindings/ts/README.md`
- Behavioural delta: JS/TS platform hosts can call kittui as a render-only PNG producer.

## Operator-takeaway

Render-only platform support is now consistent across Rust, CLI, C ABI, Python, and TypeScript.
