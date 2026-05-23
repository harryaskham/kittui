# Session summary — Raw-frame file transport path

## Goal

Implement the next isolated transport slice for bd-67a477: make raw RGBA kitty frame uploads capable of using local file-style transfer rather than only direct base64 streaming, while preserving remote/tmux-safe defaults and leaving the more invasive POSIX shared-memory allocator for later.

## Bead(s)

- `bd-67a477` — kittui-kitty: implement local shared-memory/file raw-frame transport
- (follow-up from `bd-3c0dd1` — kittui-kitty: plan adaptive graphics transport selection)

## Before state

- Failing tests: none known for this bead.
- Relevant metrics: `upload_still_rgba` only emitted direct chunked `f=32` payloads; `UploadMedium::{File,TempFile,SharedMemory}` existed for PNG/still uploads but not raw RGBA frame grammar; `TerminalInfo::detect()` ignored `KITTUI_TRANSPORT` overrides.
- Context: kittui-dev asked me to take this transport slice while they worked a separate semantic SDK/event-stream wrapper slice and avoided kitty raw-frame transport files.

## After state

- Failing tests: none observed in targeted validation.
- Relevant metrics: kittui-kitty now exposes `upload_still_rgba_medium` for `f=32` raw-frame file/tempfile/shared-memory grammar; `Runtime::place_raw_frame` uses a local tempfile transfer when `Transport::File` is selected and safely falls back to direct streaming if tempfile creation fails.
- Context: `Transport::Memory` currently falls back to the tempfile path rather than allocating shared memory directly, because the `kittui` facade forbids unsafe code and needs a dedicated safe shm allocator follow-up.

## Diff summary

- Code/content commits: `2f8aa35` (`bd-67a477: add raw-frame file transport path`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/kittui-core/src/terminal.rs`, `crates/kittui-kitty/src/lib.rs`, `crates/kittui/src/lib.rs`, `docs/adaptive-graphics-transport.md`, `docs/wm.md`
- Tests: +4 unit tests / -0 / flipped 0
- Behavioural delta: `KITTUI_TRANSPORT=file` is now honored by terminal detection, and raw frame placement can emit kitty `t=t,f=32` transfer commands with a local temp file containing the raw RGBA bytes. kittui-kitty also has tested `t=s,f=32` grammar for callers that manage POSIX shm objects themselves.
- Validation: `cargo test -p kittui-kitty raw_rgba`; `cargo test -p kittui raw_frame_file_transport_uses_tempfile_medium`; `cargo test -p kittui-core detect_`; `cargo check -p kittui-cli`; `git diff --check`.

## Operator-takeaway

This lands the safe local file-backed raw-frame transport step without overreaching into platform-specific shared-memory allocation; the kitty grammar and runtime file path are in place, and future work can focus narrowly on a safe shm allocator/cleanup story.
