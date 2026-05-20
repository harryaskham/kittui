//! kittui-kitty
//!
//! Encoder for the kitty graphics protocol. Owns escape sequence assembly,
//! unicode placeholder generation, and the upload/animation/delete control
//! flow. Knows nothing about rasterization or caching.
//!
//! Animated scenes follow the protocol's native animation control: each
//! frame is uploaded exactly once (with frame index `r=`), the per-frame
//! delay is set with `z=`, and playback loops via `s=` and `c=`. The
//! renderer never re-uploads frames between cycles.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use base64::Engine;

use kittui_core::geom::CellRect;
use kittui_core::terminal::Transport;

const PLACEHOLDER_CHAR: char = '\u{10EEEE}';
const ESC: &str = "\x1b";

/// Wrap a payload in tmux passthrough quoting when required.
fn wrap_transport(payload: String, transport: Transport) -> String {
    match transport {
        Transport::Direct | Transport::File | Transport::Memory => payload,
        Transport::TmuxPassthrough => {
            // `\ePtmux;` + payload with `\e` doubled + `\e\\`.
            let escaped = payload.replace(ESC, "\x1b\x1b");
            format!("\x1bPtmux;{escaped}\x1b\\")
        }
    }
}

/// Build the escape sequence that uploads a still PNG to the terminal with
/// the supplied image id. Subsequent placements can reference the id without
/// re-uploading.
pub fn upload_still(image_id: u32, png: &[u8], transport: Transport) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    encode_chunked(image_id, &b64, None, transport)
}

/// Build the escape sequences that upload an animated scene. Each frame is
/// uploaded once at its own frame index; after the last frame is sent, the
/// animation control command is emitted and the kitty terminal will play
/// the loop natively without further escape traffic.
pub fn upload_animation(
    image_id: u32,
    frames: &[Vec<u8>],
    frame_delays_ms: &[u32],
    loops: u32,
    transport: Transport,
) -> String {
    assert_eq!(frames.len(), frame_delays_ms.len());
    let mut out = String::new();
    for (i, frame) in frames.iter().enumerate() {
        let frame_index = (i as u32).saturating_add(1);
        let b64 = base64::engine::general_purpose::STANDARD.encode(frame);
        out.push_str(&encode_chunked(image_id, &b64, Some(frame_index), transport));
    }
    let control = format!(
        "{ESC}_Ga=a,i={id},s={loops_field},c={count}{terminator}{ESC}\\",
        id = image_id,
        loops_field = loops,
        count = frames.len(),
        terminator = ""
    );
    out.push_str(&wrap_transport(control, transport));
    // Per-frame delays via the control command form documented for kitty.
    for (i, delay) in frame_delays_ms.iter().enumerate() {
        let frame_index = (i as u32).saturating_add(1);
        let cmd = format!(
            "{ESC}_Ga=a,i={id},r={frame},z={delay}{ESC}\\",
            id = image_id,
            frame = frame_index,
            delay = delay
        );
        out.push_str(&wrap_transport(cmd, transport));
    }
    out
}

/// Build the placement escape sequence that anchors the previously-uploaded
/// image at the current cursor with the supplied cell footprint. The host
/// must position the cursor before emitting placeholder rows.
pub fn placement_command(image_id: u32, footprint: CellRect, transport: Transport) -> String {
    let payload = format!(
        "{ESC}_Ga=p,U=1,i={id},c={cols},r={rows}{ESC}\\",
        id = image_id,
        cols = footprint.cols,
        rows = footprint.rows
    );
    wrap_transport(payload, transport)
}

/// Build the unicode-placeholder text grid that should be rendered into the
/// terminal cell footprint reserved for `image_id`. Hosts print this string
/// at the placement origin to make the kitty graphics protocol stamp the
/// image into the corresponding cells.
pub fn placeholder_text(image_id: u32, footprint: CellRect) -> String {
    // Encode the image id into the foreground color of the placeholder so
    // kitty knows which image to anchor under each placeholder cell. We
    // follow the documented `\x1b[38:2:r:g:b]` form using bytes from the id.
    let r = ((image_id >> 16) & 0xff) as u8;
    let g = ((image_id >> 8) & 0xff) as u8;
    let b = (image_id & 0xff) as u8;
    let mut out = String::new();
    for _ in 0..footprint.rows {
        out.push_str(&format!("\x1b[38:2:{r}:{g}:{b}m"));
        for _ in 0..footprint.cols {
            out.push(PLACEHOLDER_CHAR);
        }
        out.push_str("\x1b[39m\n");
    }
    out
}

/// Build the escape sequence that deletes an image (and all of its
/// placements) by id.
pub fn delete(image_id: u32, transport: Transport) -> String {
    wrap_transport(
        format!("{ESC}_Ga=d,d=I,i={id}{ESC}\\", id = image_id),
        transport,
    )
}

fn encode_chunked(
    image_id: u32,
    base64_body: &str,
    frame_index: Option<u32>,
    transport: Transport,
) -> String {
    const CHUNK: usize = 4096;
    let mut out = String::new();
    let bytes = base64_body.as_bytes();
    let mut offset = 0;
    let frame_field = frame_index
        .map(|i| format!(",r={i}"))
        .unwrap_or_default();
    while offset < bytes.len() {
        let end = (offset + CHUNK).min(bytes.len());
        let more = if end < bytes.len() { 1 } else { 0 };
        let header = if offset == 0 {
            format!(
                "a=t,f=100,i={id},m={more}{frame_field}",
                id = image_id,
                more = more,
                frame_field = frame_field
            )
        } else {
            format!("m={more}", more = more)
        };
        let body = std::str::from_utf8(&bytes[offset..end]).unwrap_or("");
        let payload = format!("{ESC}_G{header};{body}{ESC}\\");
        out.push_str(&wrap_transport(payload, transport));
        offset = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_still_includes_image_id_and_base64() {
        let escapes = upload_still(0xABCD, b"hello kittui", Transport::Direct);
        assert!(escapes.contains("\x1b_Ga=t,f=100,i=43981"));
        assert!(!escapes.contains("\x1b_G,"));
        assert!(escapes.contains("aGVsbG8ga2l0dHVp"));
    }

    #[test]
    fn placeholder_grid_has_unicode_placeholder() {
        let text = placeholder_text(0xABCDEF, CellRect::new(0, 0, 3, 2));
        assert_eq!(text.matches(PLACEHOLDER_CHAR).count(), 6);
    }

    #[test]
    fn animated_upload_emits_one_frame_per_index() {
        let frames = vec![vec![1u8; 8], vec![2u8; 8], vec![3u8; 8]];
        let delays = vec![100, 100, 100];
        let escapes = upload_animation(0x42, &frames, &delays, 0, Transport::Direct);
        assert!(escapes.contains("r=1"));
        assert!(escapes.contains("r=2"));
        assert!(escapes.contains("r=3"));
        assert!(escapes.contains("a=a,i=66,s=0,c=3"));
    }

    #[test]
    fn tmux_passthrough_wraps_each_payload() {
        let escapes = upload_still(1, b"hi", Transport::TmuxPassthrough);
        assert!(escapes.contains("\x1bPtmux;"));
        assert!(escapes.ends_with("\x1b\\"));
    }
}
