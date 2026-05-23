# Session summary — POSIX shm raw-frame transport

## Goal

Implement bd-4edcb2 as a separate follow-up to bd-67a477: add a safe local shared-memory allocation path for raw RGBA kitty frame uploads when `Transport::Memory` is selected, while keeping unsupported platforms and allocation failures on safe fallback paths.

## Bead(s)

- `bd-4edcb2` — kittui-kitty: add POSIX shm allocator for raw-frame transport
- (follow-up from `bd-67a477` — kittui-kitty: implement local shared-memory/file raw-frame transport)

## Before state

- Failing tests: none known for this bead.
- Relevant metrics: raw-frame `f=32` grammar supported `t=s`, and `Runtime::place_raw_frame` supported tempfile-backed file transport, but `Transport::Memory` still fell back to tempfile/direct streaming and kept no allocator-owned cleanup state.
- Context: kittui-dev assigned this transport slice to me and took a separate native shell chrome/kittui-affordance scene renderer bead to avoid overlap.

## After state

- Failing tests: none observed in targeted validation.
- Relevant metrics: `Runtime` now tracks raw-frame shared-memory backing files per image id, writes Linux `/dev/shm`-backed POSIX shm-compatible objects for `Transport::Memory`, emits kitty `t=s,f=32`, and removes stale backing files on reupload, `unplace`, and runtime drop.
- Context: unsupported hosts without `/dev/shm` still fall back to the existing tempfile/direct-safe path.

## Diff summary

- Code/content commits: `9d040b7` (`bd-4edcb2: add shm raw-frame transport path`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui/src/lib.rs`, `docs/adaptive-graphics-transport.md`, `docs/wm.md`
- Tests: +2 unit tests / -0 / flipped 0
- Behavioural delta: `KITTUI_TRANSPORT=memory` can now use a safe local `/dev/shm` backing file and kitty `t=s` raw-frame grammar where available; cleanup is tied to image lifecycle and runtime lifetime.
- Validation: `cargo test -p kittui raw_frame`; `cargo test -p kittui-kitty raw_rgba`; `cargo test -p kittui-core detect_`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

The invasive part of raw-frame transport is now contained: memory transport is implemented without adding unsafe code to `kittui`, cleans up its own backing files, and degrades safely on non-Linux or restricted hosts.
