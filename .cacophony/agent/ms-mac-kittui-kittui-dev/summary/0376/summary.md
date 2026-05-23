# Session summary — kitty frame memory guard and zlib upload mode

## Goal

Address high memory/wire use from native kittwm raw kitty graphics frames, especially inside tmux, and add kitty `o=z` zlib compression support for direct uploads.

## Bead(s)

- `bd-e9457f` — kittwm: guard raw kitty frame memory and tmux graphics
- Follow-up filed: `bd-3c0dd1` — kittui-kitty: plan adaptive graphics transport selection
- Follow-up filed: `bd-510a36` — kittwm: investigate dirty-grid kitty frame updates

## Before state

- Failing tests: none known.
- User-visible gap: running kittui/kittwm graphics inside tmux could consume huge memory. Native kittwm uploaded full raw RGBA frames each tick through the kitty protocol, and the raw-frame hot path relied on same image-id replacement rather than explicitly deleting prior terminal image payloads.

## After state

- Failing tests: none in targeted checks.
- Validation:
  - `cargo test -p kittui-kitty --lib -- --nocapture` passed.
  - `cargo test -p kittui --lib raw_frame_reupload_deletes_previous_image_payload -- --nocapture` passed.
  - `cargo test -p kittui-cli --lib session::native_pane_tests::native_renderer_defaults_to_terminal_inside_tmux_unless_overridden -- --nocapture` passed.
  - `cargo build -p kittui-cli --bin kittwm` passed.
  - `git diff --check` passed.
- Context:
  - `Runtime::place_raw_frame` now emits a kitty delete for an already-uploaded raw-frame image id before uploading its replacement, allowing terminals to reclaim prior image payload memory promptly.
  - Native kittwm now defaults to the pure terminal renderer when `$TMUX` is present, unless `KITTWM_NATIVE_RENDERER` explicitly requests another mode. `KITTWM_NATIVE_RENDERER=kitty`/`graphics` forces graphics; `terminal`/`text`/`ansi`/`dec` force the terminal renderer.
  - Added `CompressionMode` and `KITTUI_KITTY_COMPRESSION=zlib|auto|deflate|z` support in `kittui-kitty`.
  - Direct PNG uploads and raw RGBA (`f=32`) uploads can now be zlib-compressed and marked with kitty `o=z`.
  - Raw RGBA zlib grammar and decode round-trip are tested.
  - docs/wm documents tmux fallback, graphics override, direct escape transport, and `KITTUI_KITTY_COMPRESSION`.

## Diff summary

- Code/content commit: `d6db5e6b`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `Cargo.lock`, `crates/kittui-cli/src/session.rs`, `crates/kittui-kitty/Cargo.toml`, `crates/kittui-kitty/src/lib.rs`, `crates/kittui/src/lib.rs`, `docs/wm.md`
- Behavioural delta: tmux defaults to safer pure terminal rendering; graphics mode reuploads now delete prior raw image payloads; zlib compression is available for direct kitty graphics uploads.

## Operator-takeaway

Outside tmux, graphics are still sent as kitty graphics escape sequences by default. Inside tmux, native kittwm now avoids graphics passthrough by default. For direct graphics, try `KITTUI_KITTY_COMPRESSION=zlib` to reduce wire bytes; future adaptive transport/shm and dirty-grid updates are tracked separately.
