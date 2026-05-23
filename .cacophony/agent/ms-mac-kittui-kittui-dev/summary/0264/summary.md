# Session summary — channelized batch FFI output

## Goal

Expose channelized batch placement output to platform bindings so hosts can separately inspect/write upload, placement, and embed channels instead of receiving only one concatenated byte string.

## Bead(s)

- `bd-2c3439` — kittui-ffi: return channelized batch placement JSON

## Before state

- Failing tests: none known.
- Relevant gap: CLI JSON output had `upload`/`placement`/`embed` channels and metadata, but FFI/TypeScript batch placement only returned concatenated bytes. Platform hosts could not inspect image ids, footprints, channel sizes, or schedule channels separately.

## After state

- Failing tests: none in targeted checks.
- Relevant metrics:
  - `cargo test -p kittui-ffi place_many_json_channels -- --nocapture` passed.
  - `cargo test -p kittui-ffi --test abi_snapshot -- --nocapture` passed.
  - `cargo build -p kittui-ffi` passed.
  - `npm test --prefix bindings/ts` passed.
  - `git diff --check` passed.
- Context: Added additive C ABI `kittui_place_many_json_channels(runtime, scenes_json, x, y, out)` returning JSON with `count`, `image_ids`, `footprints`, byte counts, and raw `upload`/`placement`/`embed` strings. Bumped ABI minor to 7 and updated `kittui.h` plus ABI snapshot tests. TypeScript now exposes `Kittui.placeManyChannels(scenes, x, y)` returning a parsed object.

## Diff summary

- Code/content commit: `8988d9c`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-ffi/src/lib.rs`, `crates/kittui-ffi/kittui.h`, `crates/kittui-ffi/tests/abi_snapshot.rs`, `bindings/ts/src/index.js`, `bindings/ts/src/index.d.ts`, `bindings/ts/test/koffi.test.js`
- Behavioural delta: C/TypeScript hosts can request structured channelized batch placement output.

## Operator-takeaway

kittui is now a better renderer substrate for external platforms: hosts can batch render at an origin and decide when/how to write each terminal byte channel.
