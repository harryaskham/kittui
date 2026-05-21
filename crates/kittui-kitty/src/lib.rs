//! kittui-kitty
//!
//! Encoder for the kitty graphics protocol. Owns escape sequence assembly,
//! unicode placeholder generation, and the upload/animation/delete control
//! flow. Knows nothing about rasterization or caching.
//!
//! The protocol reference is <https://sw.kovidgoyal.net/kitty/graphics-protocol/>;
//! every grammar choice here is pinned by exact-grammar regression tests in
//! `tests` modules so silent drift is detected by `cargo test`.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use std::path::Path;

use base64::Engine;

use kittui_core::geom::CellRect;
use kittui_core::terminal::Transport;

mod diacritics;

use diacritics::{ROWCOLUMN_DIACRITICS, ROWCOLUMN_DIACRITICS_COUNT};

/// Codepoint reserved by the kitty protocol for unicode image placeholders.
pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';
const ESC: &str = "\x1b";

/// Quietness for control responses. Mirrors the kitty `q=` field.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum Quiet {
    /// Default: kitty responds to every command (`q` is omitted).
    Verbose,
    /// Suppress success responses only (`q=1`).
    SuppressOk,
    /// Suppress all responses including errors (`q=2`).
    #[default]
    SuppressAll,
}

impl Quiet {
    fn field(self) -> &'static str {
        match self {
            Quiet::Verbose => "",
            Quiet::SuppressOk => ",q=1",
            Quiet::SuppressAll => ",q=2",
        }
    }
}

/// Optional pixel-space offset within the anchor cell. Spec field `X=`/`Y=`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct SubcellOffset {
    /// Horizontal pixel offset within the anchor cell.
    pub x_px: u32,
    /// Vertical pixel offset within the anchor cell.
    pub y_px: u32,
}

/// Options for a placement command. Default is unicode-anchored placement at
/// the cursor with no placement id and no subcell offset.
#[derive(Clone, Debug, Default)]
pub struct PlacementOptions {
    /// Placement id (`p=`). Defaults to no id (single placement).
    pub placement_id: Option<u32>,
    /// Subcell offset (`X=`/`Y=`). Defaults to (0,0).
    pub offset: SubcellOffset,
    /// Quietness for the placement response.
    pub quiet: Quiet,
    /// Anchor via the unicode placeholder mechanism (`U=1`). Default true.
    pub unicode_placeholder: bool,
    /// Z-index for the placement (`z=`). Default 0.
    pub z_index: i32,
}

impl PlacementOptions {
    /// Construct unicode-anchored options with default quietness.
    pub fn unicode() -> Self {
        Self {
            unicode_placeholder: true,
            ..Self::default()
        }
    }

    /// Construct cursor-anchored absolute placement without the unicode placeholder.
    pub fn absolute() -> Self {
        Self {
            unicode_placeholder: false,
            ..Self::default()
        }
    }
}

/// Upload medium selection. Mirrors the kitty `t=` field.
#[derive(Clone, Debug)]
pub enum UploadMedium<'a> {
    /// Direct base64 streaming over the escape (`t=d`, default).
    Direct {
        /// Raw payload bytes (e.g. PNG).
        bytes: &'a [u8],
    },
    /// Path to a regular file readable by the terminal (`t=f`).
    File {
        /// Path the terminal will read.
        path: &'a Path,
    },
    /// POSIX shared-memory name the terminal will `shm_open` (`t=s`).
    SharedMemory {
        /// Shared-memory object name (e.g. `/kittui-<id>`).
        name: &'a str,
    },
    /// Path to a temp file the terminal should consume and delete (`t=t`).
    TempFile {
        /// Path the terminal will read and unlink.
        path: &'a Path,
    },
}

/// Wrap a payload in tmux passthrough quoting when required.
fn wrap_transport(payload: String, transport: Transport) -> String {
    match transport {
        Transport::Direct | Transport::File | Transport::Memory => payload,
        Transport::TmuxPassthrough => {
            let escaped = payload.replace(ESC, "\x1b\x1b");
            format!("\x1bPtmux;{escaped}\x1b\\")
        }
    }
}

/// Build the escape sequence that uploads a still image to the terminal.
///
/// `medium` controls the upload mechanism (`t=`); `quiet` controls the `q=`
/// field.
pub fn upload_still_ex(
    image_id: u32,
    medium: UploadMedium<'_>,
    quiet: Quiet,
    transport: Transport,
) -> String {
    upload(image_id, None, medium, quiet, transport, /*first_frame=*/ true)
}

/// Back-compat: upload still bytes directly via base64 with the default
/// quietness (`q=2`). Newer callers should prefer [`upload_still_ex`].
pub fn upload_still(image_id: u32, png: &[u8], transport: Transport) -> String {
    upload_still_ex(
        image_id,
        UploadMedium::Direct { bytes: png },
        Quiet::SuppressAll,
        transport,
    )
}

/// Upload a raw 32-bit RGBA frame using the kitty `f=32` format. Skips PNG
/// encoding entirely — callers (e.g. kittui-wm's per-frame WM hot path)
/// supply the tight RGBA bytes plus `(width, height)` in pixels. The kitty
/// terminal interprets the body as `width*height*4` raw RGBA bytes, base64
/// encoded over the wire.
///
/// Chunked exactly like the PNG path; the first chunk header carries
/// `f=32,s=W,v=H` instead of `f=100`.
pub fn upload_still_rgba(
    image_id: u32,
    rgba: &[u8],
    width: u32,
    height: u32,
    transport: Transport,
) -> String {
    upload_still_rgba_ex(
        image_id,
        rgba,
        width,
        height,
        Quiet::SuppressAll,
        transport,
    )
}

/// Variant of [`upload_still_rgba`] with explicit quiet selector.
pub fn upload_still_rgba_ex(
    image_id: u32,
    rgba: &[u8],
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(rgba);
    encode_chunked_rgba(image_id, &b64, width, height, quiet, transport)
}

fn encode_chunked_rgba(
    image_id: u32,
    base64_body: &str,
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    const CHUNK: usize = 4096;
    let mut out = String::new();
    let bytes = base64_body.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + CHUNK).min(bytes.len());
        let more = if end < bytes.len() { 1 } else { 0 };
        let header = if offset == 0 {
            format!(
                "a=t,f=32,s={s},v={v},i={id},m={more}{q}",
                s = width,
                v = height,
                id = image_id,
                more = more,
                q = quiet.field(),
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

/// Build the escape sequences that upload an animated scene.
///
/// Per kitty's protocol, the first frame is uploaded with `a=t,f=100,i=<id>`
/// and each subsequent frame appends with `a=f,i=<id>,r=<index>`. The
/// animation control command (`a=a`) is emitted last and sets loop count and
/// per-frame gap defaults; per-frame delays use `a=a,i=<id>,r=<n>,z=<delay>`.
pub fn upload_animation(
    image_id: u32,
    frames: &[Vec<u8>],
    frame_delays_ms: &[u32],
    loops: u32,
    transport: Transport,
) -> String {
    upload_animation_ex(
        image_id,
        frames,
        frame_delays_ms,
        loops,
        Quiet::SuppressAll,
        transport,
    )
}

/// `upload_animation` variant with an explicit `quiet` selector.
pub fn upload_animation_ex(
    image_id: u32,
    frames: &[Vec<u8>],
    frame_delays_ms: &[u32],
    loops: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    assert_eq!(frames.len(), frame_delays_ms.len());
    let mut out = String::new();
    for (i, frame) in frames.iter().enumerate() {
        let medium = UploadMedium::Direct { bytes: frame };
        out.push_str(&upload(
            image_id,
            Some((i as u32).saturating_add(1)),
            medium,
            quiet,
            transport,
            /*first_frame=*/ i == 0,
        ));
    }
    let control = format!(
        "{ESC}_Ga=a,i={id},s={loops_field},c={count}{q}{ESC}\\",
        id = image_id,
        loops_field = loops,
        count = frames.len(),
        q = quiet.field(),
    );
    out.push_str(&wrap_transport(control, transport));
    for (i, delay) in frame_delays_ms.iter().enumerate() {
        let frame_index = (i as u32).saturating_add(1);
        let cmd = format!(
            "{ESC}_Ga=a,i={id},r={frame},z={delay}{q}{ESC}\\",
            id = image_id,
            frame = frame_index,
            delay = delay,
            q = quiet.field(),
        );
        out.push_str(&wrap_transport(cmd, transport));
    }
    out
}

/// Build a placement escape sequence with explicit options.
pub fn placement_command_ex(
    image_id: u32,
    footprint: CellRect,
    options: &PlacementOptions,
    transport: Transport,
) -> String {
    let mut fields = format!("a=p,i={id},c={cols},r={rows}",
        id = image_id,
        cols = footprint.cols,
        rows = footprint.rows,
    );
    if options.unicode_placeholder {
        fields.push_str(",U=1");
    }
    if let Some(p) = options.placement_id {
        fields.push_str(&format!(",p={p}"));
    }
    if options.offset.x_px != 0 {
        fields.push_str(&format!(",X={}", options.offset.x_px));
    }
    if options.offset.y_px != 0 {
        fields.push_str(&format!(",Y={}", options.offset.y_px));
    }
    if options.z_index != 0 {
        fields.push_str(&format!(",z={}", options.z_index));
    }
    fields.push_str(options.quiet.field());
    let payload = format!("{ESC}_G{fields}{ESC}\\");
    wrap_transport(payload, transport)
}

/// Back-compat: default unicode-anchored placement with `q=2`.
pub fn placement_command(image_id: u32, footprint: CellRect, transport: Transport) -> String {
    placement_command_ex(image_id, footprint, &PlacementOptions::unicode(), transport)
}

/// Build the unicode-placeholder text grid that should be rendered into the
/// terminal cell footprint reserved for `image_id`.
///
/// Per the kitty spec each placeholder cell carries combining diacritics
/// `(row-diacritic, column-diacritic, msb-of-image-id-diacritic)` selected
/// from `rowcolumn-diacritics.txt`. The low 24 bits of the image id travel
/// in the foreground color (`\x1b[38:2:r:g:b]`); the most significant byte
/// of the image id travels in the third diacritic.
pub fn placeholder_text(image_id: u32, footprint: CellRect) -> String {
    placeholder_text_ex(image_id, None, footprint)
}

/// Same as [`placeholder_text`] but lets the caller specify a placement id.
/// The placement id is encoded into the cell's underline color
/// (`\x1b[58:2:r:g:b]`) so kitty/Ghostty can disambiguate placements that
/// share an image id.
pub fn placeholder_text_ex(
    image_id: u32,
    placement_id: Option<u32>,
    footprint: CellRect,
) -> String {
    let r = ((image_id >> 16) & 0xff) as u8;
    let g = ((image_id >> 8) & 0xff) as u8;
    let b = (image_id & 0xff) as u8;
    let msb = ((image_id >> 24) & 0xff) as u8;
    let mut out = String::new();
    let underline = placement_id.map(|p| {
        let pr = ((p >> 16) & 0xff) as u8;
        let pg = ((p >> 8) & 0xff) as u8;
        let pb = (p & 0xff) as u8;
        format!("\x1b[58:2:{pr}:{pg}:{pb}m")
    });
    for row in 0..footprint.rows {
        out.push_str(&format!("\x1b[38:2:{r}:{g}:{b}m"));
        if let Some(u) = &underline {
            out.push_str(u);
        }
        for col in 0..footprint.cols {
            out.push(PLACEHOLDER_CHAR);
            out.push(diacritic_for(row as u32));
            out.push(diacritic_for(col as u32));
            if msb != 0 {
                out.push(diacritic_for(msb as u32));
            }
        }
        if placement_id.is_some() {
            out.push_str("\x1b[59m");
        }
        out.push_str("\x1b[39m\n");
    }
    out
}

/// Build the escape sequence that deletes an image (and all of its
/// placements) by id.
pub fn delete(image_id: u32, transport: Transport) -> String {
    wrap_transport(
        format!("{ESC}_Ga=d,d=I,i={id},q=2{ESC}\\", id = image_id),
        transport,
    )
}

/// Delete a single placement by `(image_id, placement_id)`.
pub fn delete_placement(image_id: u32, placement_id: u32, transport: Transport) -> String {
    wrap_transport(
        format!(
            "{ESC}_Ga=d,d=I,i={id},p={p},q=2{ESC}\\",
            id = image_id,
            p = placement_id
        ),
        transport,
    )
}

/// Internal: emit one upload command (single chunked image or one animation
/// frame), respecting the medium, quiet field, and animation verb selection.
fn upload(
    image_id: u32,
    frame_index: Option<u32>,
    medium: UploadMedium<'_>,
    quiet: Quiet,
    transport: Transport,
    first_frame: bool,
) -> String {
    let verb = if frame_index.is_some() && !first_frame {
        "a=f"
    } else {
        "a=t"
    };
    match medium {
        UploadMedium::Direct { bytes } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            encode_chunked(image_id, verb, &b64, frame_index, quiet, transport)
        }
        UploadMedium::File { path } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(path_bytes(path));
            single_payload(image_id, verb, "f", &b64, frame_index, quiet, transport)
        }
        UploadMedium::TempFile { path } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(path_bytes(path));
            single_payload(image_id, verb, "t", &b64, frame_index, quiet, transport)
        }
        UploadMedium::SharedMemory { name } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(name.as_bytes());
            single_payload(image_id, verb, "s", &b64, frame_index, quiet, transport)
        }
    }
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

fn single_payload(
    image_id: u32,
    verb: &str,
    medium_field: &str,
    base64_body: &str,
    frame_index: Option<u32>,
    quiet: Quiet,
    transport: Transport,
) -> String {
    let frame_field = frame_index.map(|i| format!(",r={i}")).unwrap_or_default();
    let header = format!(
        "{verb},f=100,t={medium},i={id}{frame}{q}",
        verb = verb,
        medium = medium_field,
        id = image_id,
        frame = frame_field,
        q = quiet.field(),
    );
    let payload = format!("{ESC}_G{header};{base64_body}{ESC}\\");
    wrap_transport(payload, transport)
}

fn encode_chunked(
    image_id: u32,
    verb: &str,
    base64_body: &str,
    frame_index: Option<u32>,
    quiet: Quiet,
    transport: Transport,
) -> String {
    const CHUNK: usize = 4096;
    let mut out = String::new();
    let bytes = base64_body.as_bytes();
    let mut offset = 0;
    let frame_field = frame_index.map(|i| format!(",r={i}")).unwrap_or_default();
    while offset < bytes.len() {
        let end = (offset + CHUNK).min(bytes.len());
        let more = if end < bytes.len() { 1 } else { 0 };
        let header = if offset == 0 {
            format!(
                "{verb},f=100,i={id},m={more}{frame_field}{q}",
                verb = verb,
                id = image_id,
                more = more,
                frame_field = frame_field,
                q = quiet.field(),
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

/// Diacritic codepoint for a 0-indexed row/column/msb value. Saturates at the
/// last spec diacritic for out-of-range inputs (the kitty spec table has 297
/// entries; rows/cols above that are clamped rather than dropped).
pub fn diacritic_for(index_zero_based: u32) -> char {
    let clamped = (index_zero_based as usize).min(ROWCOLUMN_DIACRITICS_COUNT - 1);
    ROWCOLUMN_DIACRITICS[clamped]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_still_emits_exact_grammar() {
        let escapes = upload_still(0xABCD, b"hello kittui", Transport::Direct);
        // Single chunk: complete from `\x1b_G` to `\x1b\\`.
        assert_eq!(
            escapes,
            format!("\x1b_Ga=t,f=100,i=43981,m=0,q=2;aGVsbG8ga2l0dHVp\x1b\\"),
        );
    }

    #[test]
    fn upload_still_multi_chunk_starts_first_with_verb_and_rest_with_m() {
        let big = vec![0u8; 4096]; // base64 → 5464 chars, two chunks
        let escapes = upload_still(1, &big, Transport::Direct);
        assert!(escapes.starts_with("\x1b_Ga=t,f=100,i=1,m=1,q=2;"));
        // Second chunk header is the bare `m=0` continuation.
        assert!(escapes.contains("\x1b\\\x1b_Gm=0;"));
        assert!(escapes.ends_with("\x1b\\"));
    }

    #[test]
    fn placement_command_unicode_default_includes_required_fields() {
        let cmd = placement_command(0x42, CellRect::new(0, 0, 4, 3), Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=p,i=66,c=4,r=3,U=1,q=2\x1b\\");
    }

    #[test]
    fn placement_command_with_id_and_subcell_offset() {
        let opts = PlacementOptions {
            placement_id: Some(7),
            offset: SubcellOffset { x_px: 4, y_px: 2 },
            quiet: Quiet::SuppressAll,
            unicode_placeholder: false,
            z_index: -1,
        };
        let cmd = placement_command_ex(1, CellRect::new(0, 0, 8, 4), &opts, Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=p,i=1,c=8,r=4,p=7,X=4,Y=2,z=-1,q=2\x1b\\");
    }

    #[test]
    fn placeholder_grid_carries_diacritics_per_cell() {
        let text = placeholder_text(0x010203, CellRect::new(0, 0, 2, 2));
        let placeholder_count = text.matches(PLACEHOLDER_CHAR).count();
        assert_eq!(placeholder_count, 4);
        // Each placeholder is followed by row + column diacritic; image id MSB is zero.
        let mut chars = text.chars().filter(|c| *c == PLACEHOLDER_CHAR || ROWCOLUMN_DIACRITICS.contains(c));
        let first = chars.next().unwrap();
        let row0 = chars.next().unwrap();
        let col0 = chars.next().unwrap();
        assert_eq!(first, PLACEHOLDER_CHAR);
        assert_eq!(row0, diacritic_for(0));
        assert_eq!(col0, diacritic_for(0));
    }

    #[test]
    fn placeholder_grid_encodes_msb_when_nonzero() {
        let text = placeholder_text(0xAB010203, CellRect::new(0, 0, 1, 1));
        let combining: Vec<char> = text
            .chars()
            .filter(|c| ROWCOLUMN_DIACRITICS.contains(c))
            .collect();
        // Single cell → one row diacritic + one column diacritic + msb diacritic.
        assert_eq!(combining.len(), 3);
        assert_eq!(combining[2], diacritic_for(0xAB));
    }

    #[test]
    fn animated_upload_uses_a_t_then_a_f_then_control() {
        let frames = vec![vec![1u8; 8], vec![2u8; 8], vec![3u8; 8]];
        let delays = vec![100, 200, 300];
        let escapes = upload_animation(0x42, &frames, &delays, 0, Transport::Direct);
        // First frame uses a=t with r=1; second/third use a=f with r=2,r=3.
        assert!(escapes.contains("\x1b_Ga=t,f=100,i=66,m=0,r=1,q=2;"));
        assert!(escapes.contains("\x1b_Ga=f,f=100,i=66,m=0,r=2,q=2;"));
        assert!(escapes.contains("\x1b_Ga=f,f=100,i=66,m=0,r=3,q=2;"));
        // Animation control + per-frame delay commands.
        assert!(escapes.contains("\x1b_Ga=a,i=66,s=0,c=3,q=2\x1b\\"));
        assert!(escapes.contains("\x1b_Ga=a,i=66,r=1,z=100,q=2\x1b\\"));
        assert!(escapes.contains("\x1b_Ga=a,i=66,r=2,z=200,q=2\x1b\\"));
        assert!(escapes.contains("\x1b_Ga=a,i=66,r=3,z=300,q=2\x1b\\"));
    }

    #[test]
    fn tmux_passthrough_wraps_each_payload_and_doubles_escapes() {
        let escapes = upload_still(1, b"hi", Transport::TmuxPassthrough);
        assert!(escapes.starts_with("\x1bPtmux;\x1b\x1b_Ga=t,f=100,i=1,m=0,q=2;"));
        assert!(escapes.ends_with("\x1b\\"));
    }

    #[test]
    fn upload_via_file_medium_sends_path_in_t_field() {
        let path = Path::new("/tmp/kittui-image.png");
        let escapes = upload_still_ex(
            7,
            UploadMedium::File { path },
            Quiet::SuppressAll,
            Transport::Direct,
        );
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(b"/tmp/kittui-image.png");
        let want = format!("\x1b_Ga=t,f=100,t=f,i=7,q=2;{expected_b64}\x1b\\");
        assert_eq!(escapes, want);
    }

    #[test]
    fn upload_via_shared_memory_medium_sends_name_in_t_field() {
        let escapes = upload_still_ex(
            9,
            UploadMedium::SharedMemory { name: "/kittui-9" },
            Quiet::SuppressAll,
            Transport::Direct,
        );
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(b"/kittui-9");
        let want = format!("\x1b_Ga=t,f=100,t=s,i=9,q=2;{expected_b64}\x1b\\");
        assert_eq!(escapes, want);
    }

    #[test]
    fn delete_by_placement_emits_p_field() {
        let cmd = delete_placement(0x55, 3, Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=d,d=I,i=85,p=3,q=2\x1b\\");
    }

    #[test]
    fn upload_still_rgba_emits_f32_grammar_with_s_v_width_height() {
        // 2x2 RGBA, alternating red/green pixels.
        let rgba: Vec<u8> = vec![
            0xff, 0x00, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0xff, 0x00,
            0x00, 0xff,
        ];
        let escapes = upload_still_rgba(0xABCD, &rgba, 2, 2, Transport::Direct);
        assert!(
            escapes.starts_with("\x1b_Ga=t,f=32,s=2,v=2,i=43981,m=0,q=2;"),
            "raw RGBA upload must use f=32,s=W,v=H: prefix was {}",
            &escapes[..escapes.len().min(60)]
        );
        assert!(escapes.ends_with("\x1b\\"));
        // No PNG signature in the body — must be base64 of raw RGBA only.
        assert!(!escapes.contains("PNG"));
    }

    #[test]
    fn diacritic_table_is_exactly_297_entries() {
        assert_eq!(ROWCOLUMN_DIACRITICS_COUNT, 297);
        assert_eq!(ROWCOLUMN_DIACRITICS.len(), 297);
    }
}
