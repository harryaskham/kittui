# Session summary — pane frame presented runtime events

## Goal

Emit conservative graphics-frame presentation events from native kittwm so automation clients can observe frame presentation without polling status/dirty metrics or reading frame payloads.

## Bead(s)

- `bd-7f1f9d` — kittwm: emit pane frame presented events

## Before state

- Failing tests: none known.
- Relevant context: native graphics rendering captured/placed RGBA frames and updated dirty-frame status, but `EVENTS [ms]` subscribers had no explicit frame-presented event.

## After state

- Failing tests: none in targeted validation.
- Validation:
  - `cargo test -p kittui-cli --lib native_frame_presented_event_reports_metadata_without_payload -- --nocapture` passed.
  - `git diff --check` passed before commit.
- Context:
  - Added `NativeFramePresented` and `NativeSpawnQueue::publish_frame_presented`.
  - Native graphics render path now emits `pane_frame_presented` after RGBA placement/embed.
  - Event detail includes non-payload metadata:
    - renderer (`kitty`), format (`rgba`), pixel width/height
    - app cell bounds
    - uploaded / skipped_upload
    - changed_tiles / total_tiles when available
    - elapsed_us for capture/present path
  - Raw frame bytes/pixel payloads are never included.
  - Pure terminal renderer behavior is unchanged.

## Parallel coordination

- Assigned `bd-d582b7` to `kittui-dev-2`: SDK/docs typed `PaneFramePresented` event; daemon/session runtime out of scope for them.

## Diff summary

- Code/content commit: `3bd62c58`
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched:
  - `crates/kittui-cli/src/daemon.rs`
  - `crates/kittui-cli/src/session.rs`
- Behavioural delta: native graphics-rendered panes now publish frame presentation metadata into the socket event stream.

## Operator-takeaway

The resize/input/frame event-model gap is now covered at runtime; SDK typing/docs are split to dev-2.
