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

use std::io::Write;
use std::path::Path;

use base64::Engine;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use kittui_core::geom::CellRect;
use kittui_core::terminal::Transport;

mod diacritics;

use diacritics::{ROWCOLUMN_DIACRITICS, ROWCOLUMN_DIACRITICS_COUNT};

/// Codepoint reserved by the kitty protocol for unicode image placeholders.
pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';
const ESC: &str = "\x1b";

/// Compression mode for direct graphics payloads. Mirrors kitty's `o=` field.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum CompressionMode {
    /// Send the payload uncompressed.
    #[default]
    None,
    /// Compress with zlib and mark the transfer with `o=z`.
    Zlib,
    /// Choose compression based on payload size/heuristics.
    Auto,
}

impl CompressionMode {
    fn field(self) -> &'static str {
        match self {
            CompressionMode::None | CompressionMode::Auto => "",
            CompressionMode::Zlib => ",o=z",
        }
    }
}

const DEFAULT_ZLIB_MIN_BYTES: usize = 16 * 1024;

/// Return the minimum payload size where `KITTUI_KITTY_COMPRESSION=auto` uses zlib.
pub fn zlib_min_bytes_from_env() -> usize {
    std::env::var("KITTUI_ZLIB_MIN_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_ZLIB_MIN_BYTES)
}

/// Select kitty graphics compression from `KITTUI_KITTY_COMPRESSION`.
pub fn compression_from_env() -> CompressionMode {
    match std::env::var("KITTUI_KITTY_COMPRESSION")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "z" | "zlib" | "deflate" => CompressionMode::Zlib,
        "auto" => CompressionMode::Auto,
        _ => CompressionMode::None,
    }
}

/// Resolve [`CompressionMode::Auto`] for a payload length using the current env threshold.
pub fn resolve_compression_for_len(mode: CompressionMode, payload_len: usize) -> CompressionMode {
    match mode {
        CompressionMode::Auto if payload_len >= zlib_min_bytes_from_env() => CompressionMode::Zlib,
        CompressionMode::Auto => CompressionMode::None,
        other => other,
    }
}

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

/// Parsed kitty graphics terminal response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KittyResponse {
    /// Action (`a=`) reported by the response header, when present.
    pub action: Option<String>,
    /// Image/query id (`i=`), when present.
    pub image_id: Option<u32>,
    /// Placement id (`p=`), when present.
    pub placement_id: Option<u32>,
    /// Parsed response status/body.
    pub status: KittyResponseStatus,
    /// Raw response body after the `;` separator.
    pub raw_body: String,
}

/// Known kitty graphics response body classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KittyResponseStatus {
    /// Successful response (`OK`).
    Ok,
    /// Error/status token such as `ENOENT` or `EINVAL`.
    Error(String),
    /// Capability-query response body for `a=q`.
    Capability(String),
    /// Unknown response body preserved for callers.
    Other(String),
}

/// Parse errors for kitty graphics terminal responses.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum KittyResponseParseError {
    /// No complete graphics response escape was found.
    #[error("missing kitty graphics response escape")]
    MissingEscape,
    /// The response header is malformed.
    #[error("malformed kitty graphics response header")]
    MalformedHeader,
    /// A numeric field could not be parsed.
    #[error("invalid kitty graphics response field {field}={value}")]
    InvalidField {
        /// Field name.
        field: String,
        /// Field value.
        value: String,
    },
}

/// Optional pixel-space offset within the anchor cell. Spec field `X=`/`Y=`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct SubcellOffset {
    /// Horizontal pixel offset within the anchor cell.
    pub x_px: u32,
    /// Vertical pixel offset within the anchor cell.
    pub y_px: u32,
}

/// Relative placement anchor fields (`P=`, `Q=`, `H=`, `V=`).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct RelativePlacement {
    /// Parent/reference image id (`P=`).
    pub image_id: u32,
    /// Parent/reference placement id (`Q=`), when anchoring to a specific placement.
    pub placement_id: Option<u32>,
    /// Horizontal offset from the reference (`H=`).
    pub x_offset_px: i32,
    /// Vertical offset from the reference (`V=`).
    pub y_offset_px: i32,
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
    /// Relative placement anchor (`P=`, `Q=`, `H=`, `V=`).
    pub relative: Option<RelativePlacement>,
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

    /// Construct cursor-anchored absolute placement with an explicit stable
    /// placement id (`p=`). Window-manager renderers should prefer this when
    /// repeatedly moving/redrawing the same logical surface so kitty updates a
    /// known placement instead of relying on implicit/default placement
    /// semantics.
    pub fn absolute_with_id(placement_id: u32) -> Self {
        Self {
            placement_id: Some(placement_id),
            unicode_placeholder: false,
            ..Self::default()
        }
    }

    /// Alias for [`Self::absolute_with_id`] with a name that emphasizes the
    /// stable-placement contract.
    pub fn stable_absolute(placement_id: u32) -> Self {
        Self::absolute_with_id(placement_id)
    }

    /// Return these options with a placement z-index (`z=`).
    pub fn with_z_index(mut self, z_index: i32) -> Self {
        self.z_index = z_index;
        self
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
    upload(
        image_id, None, medium, quiet, transport, /*first_frame=*/ true, None,
    )
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
    upload_still_rgba_ex(image_id, rgba, width, height, Quiet::SuppressAll, transport)
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
    upload_still_rgba_compressed(
        image_id,
        rgba,
        width,
        height,
        quiet,
        transport,
        compression_from_env(),
    )
}

/// Upload a raw RGBA frame through a file/shared-memory medium.
///
/// The caller owns creating the file or shared-memory object and writing exactly
/// `width * height * 4` bytes into it. This emits kitty's `f=32` raw-frame
/// grammar together with `t=f`, `t=t`, or `t=s`; direct byte payloads should use
/// [`upload_still_rgba`] instead.
pub fn upload_still_rgba_medium(
    image_id: u32,
    medium: UploadMedium<'_>,
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    match medium {
        UploadMedium::Direct { bytes } => {
            upload_still_rgba_ex(image_id, bytes, width, height, quiet, transport)
        }
        UploadMedium::File { path } => single_payload_raw(
            image_id,
            32,
            "f",
            &base64::engine::general_purpose::STANDARD.encode(path_bytes(path)),
            width,
            height,
            quiet,
            transport,
        ),
        UploadMedium::TempFile { path } => single_payload_raw(
            image_id,
            32,
            "t",
            &base64::engine::general_purpose::STANDARD.encode(path_bytes(path)),
            width,
            height,
            quiet,
            transport,
        ),
        UploadMedium::SharedMemory { name } => single_payload_raw(
            image_id,
            32,
            "s",
            &base64::engine::general_purpose::STANDARD.encode(name.as_bytes()),
            width,
            height,
            quiet,
            transport,
        ),
    }
}

/// Upload a raw 24-bit RGB frame using the kitty `f=24` format.
///
/// This is additive for callers that already own tightly packed RGB bytes.
/// Current kittui renderers and kittwm hot paths generally produce RGBA and
/// should keep using [`upload_still_rgba`] unless they can avoid conversion.
pub fn upload_still_rgb(
    image_id: u32,
    rgb: &[u8],
    width: u32,
    height: u32,
    transport: Transport,
) -> String {
    upload_still_rgb_ex(image_id, rgb, width, height, Quiet::SuppressAll, transport)
}

/// Variant of [`upload_still_rgb`] with explicit quiet selector.
pub fn upload_still_rgb_ex(
    image_id: u32,
    rgb: &[u8],
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    upload_still_rgb_compressed(
        image_id,
        rgb,
        width,
        height,
        quiet,
        transport,
        compression_from_env(),
    )
}

/// Upload a raw RGB frame through a file/shared-memory medium.
///
/// The caller owns creating the file or shared-memory object and writing exactly
/// `width * height * 3` bytes into it. This emits kitty's `f=24` raw-frame
/// grammar together with `t=f`, `t=t`, or `t=s`; direct byte payloads should use
/// [`upload_still_rgb`] instead.
pub fn upload_still_rgb_medium(
    image_id: u32,
    medium: UploadMedium<'_>,
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    match medium {
        UploadMedium::Direct { bytes } => {
            upload_still_rgb_ex(image_id, bytes, width, height, quiet, transport)
        }
        UploadMedium::File { path } => single_payload_raw(
            image_id,
            24,
            "f",
            &base64::engine::general_purpose::STANDARD.encode(path_bytes(path)),
            width,
            height,
            quiet,
            transport,
        ),
        UploadMedium::TempFile { path } => single_payload_raw(
            image_id,
            24,
            "t",
            &base64::engine::general_purpose::STANDARD.encode(path_bytes(path)),
            width,
            height,
            quiet,
            transport,
        ),
        UploadMedium::SharedMemory { name } => single_payload_raw(
            image_id,
            24,
            "s",
            &base64::engine::general_purpose::STANDARD.encode(name.as_bytes()),
            width,
            height,
            quiet,
            transport,
        ),
    }
}

/// Upload a raw RGB frame with an explicit compression mode.
pub fn upload_still_rgb_compressed(
    image_id: u32,
    rgb: &[u8],
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
    compression: CompressionMode,
) -> String {
    let compression = resolve_compression_for_len(compression, rgb.len());
    let payload = compress_payload(rgb, compression).unwrap_or_else(|| rgb.to_vec());
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    encode_chunked_raw(
        image_id,
        &b64,
        24,
        width,
        height,
        quiet,
        transport,
        compression,
    )
}

/// Upload a raw RGBA frame with an explicit compression mode.
pub fn upload_still_rgba_compressed(
    image_id: u32,
    rgba: &[u8],
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
    compression: CompressionMode,
) -> String {
    let compression = resolve_compression_for_len(compression, rgba.len());
    let payload = compress_payload(rgba, compression).unwrap_or_else(|| rgba.to_vec());
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    encode_chunked_raw(
        image_id,
        &b64,
        32,
        width,
        height,
        quiet,
        transport,
        compression,
    )
}

fn compress_payload(bytes: &[u8], compression: CompressionMode) -> Option<Vec<u8>> {
    match resolve_compression_for_len(compression, bytes.len()) {
        CompressionMode::None => Some(bytes.to_vec()),
        CompressionMode::Auto => unreachable!("auto compression must resolve before encoding"),
        CompressionMode::Zlib => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(bytes).ok()?;
            encoder.finish().ok()
        }
    }
}

fn encode_chunked_raw(
    image_id: u32,
    base64_body: &str,
    format: u8,
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
    compression: CompressionMode,
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
                "a=t,f={format},s={s},v={v},i={id},m={more}{compression}{q}",
                format = format,
                s = width,
                v = height,
                id = image_id,
                more = more,
                compression = compression.field(),
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

/// Playback state for a kitty animation control command (`a=a`).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AnimationState {
    /// Stop playback.
    Stop,
    /// Play, typically looping until another control command changes state.
    Play,
    /// Play a finite number of loops, then stop.
    PlayAndStop,
}

impl AnimationState {
    fn field(self) -> u32 {
        match self {
            Self::Stop => 1,
            Self::Play => 2,
            Self::PlayAndStop => 3,
        }
    }
}

/// Upload one PNG animation frame with explicit frame index and delay.
///
/// Frame index `1` is uploaded with `a=t` and no redundant `r=1`; later frames
/// use `a=f,r=<index>`. This is the typed primitive that full-frame animation
/// or ring-buffer experiments can use without constructing a whole animation at
/// once.
pub fn upload_animation_frame(
    image_id: u32,
    frame_index: u32,
    png: &[u8],
    frame_delay_ms: Option<u32>,
    transport: Transport,
) -> String {
    upload_animation_frame_ex(
        image_id,
        frame_index,
        png,
        frame_delay_ms,
        Quiet::SuppressAll,
        transport,
    )
}

/// Variant of [`upload_animation_frame`] with explicit quiet selector.
pub fn upload_animation_frame_ex(
    image_id: u32,
    frame_index: u32,
    png: &[u8],
    frame_delay_ms: Option<u32>,
    quiet: Quiet,
    transport: Transport,
) -> String {
    let normalized = frame_index.max(1);
    upload(
        image_id,
        (normalized > 1).then_some(normalized),
        UploadMedium::Direct { bytes: png },
        quiet,
        transport,
        /*first_frame=*/ normalized == 1,
        frame_delay_ms,
    )
}

/// Emit a typed kitty animation control command (`a=a`).
///
/// `current_frame` maps to kitty's `c=` field. `loops` maps to `v=` and is most
/// useful with [`AnimationState::PlayAndStop`].
pub fn animation_control(
    image_id: u32,
    state: AnimationState,
    loops: Option<u32>,
    current_frame: Option<u32>,
    transport: Transport,
) -> String {
    animation_control_ex(
        image_id,
        state,
        loops,
        current_frame,
        Quiet::SuppressAll,
        transport,
    )
}

/// Variant of [`animation_control`] with explicit quiet selector.
pub fn animation_control_ex(
    image_id: u32,
    state: AnimationState,
    loops: Option<u32>,
    current_frame: Option<u32>,
    quiet: Quiet,
    transport: Transport,
) -> String {
    let current = current_frame
        .map(|frame| format!(",c={}", frame.max(1)))
        .unwrap_or_default();
    let loops = loops.map(|v| format!(",v={v}")).unwrap_or_default();
    let control = format!(
        "{ESC}_Ga=a,i={id},s={state}{current}{loops}{q}{ESC}\\",
        id = image_id,
        state = state.field(),
        current = current,
        loops = loops,
        q = quiet.field(),
    );
    wrap_transport(control, transport)
}

/// Convenience control command that selects a displayed animation frame.
pub fn set_animation_frame(
    image_id: u32,
    frame_index: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    animation_control_ex(
        image_id,
        AnimationState::Stop,
        None,
        Some(frame_index),
        quiet,
        transport,
    )
}

/// Build the escape sequences that upload an animated scene.
///
/// Per kitty's protocol, the first frame is uploaded with `a=t,f=100,i=<id>`
/// and each subsequent frame appends with `a=f,i=<id>,r=<index>`. Per-frame
/// delays are encoded as `z=<ms>` on each upload command. The animation control
/// command (`a=a`) is emitted last and sets playback state/loop count.
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
        out.push_str(&upload_animation_frame_ex(
            image_id,
            (i as u32).saturating_add(1),
            frame,
            frame_delays_ms.get(i).copied(),
            quiet,
            transport,
        ));
    }
    // Animation control: pick state + loop count per spec.
    //   s=2          => loop forever
    //   s=3,v=<N>    => play N times then stop (kitty extension: v=0 also infinite)
    // c=<frame> would force current frame; omit it so playback starts at frame 1.
    let (state, loops) = if loops == 0 {
        (AnimationState::Play, None)
    } else {
        (AnimationState::PlayAndStop, Some(loops))
    };
    out.push_str(&animation_control_ex(
        image_id, state, loops, None, quiet, transport,
    ));
    out
}

/// Build a CSI cursor-move escape that positions the cursor at the
/// 1-indexed terminal coordinate corresponding to the (0-indexed)
/// `(col_x, row_y)` cell position. Use this to anchor a placement at an
/// absolute terminal coordinate before emitting `placement_command`; the
/// kitty graphics protocol itself has no absolute-positioning verb, so the
/// cursor must be moved first. (bd-12568a)
pub fn cursor_move(col_x: u16, row_y: u16, transport: Transport) -> String {
    let row = row_y.saturating_add(1);
    let col = col_x.saturating_add(1);
    wrap_transport(format!("\x1b[{row};{col}H"), transport)
}

/// Build a placement escape sequence with explicit options.
pub fn placement_command_ex(
    image_id: u32,
    footprint: CellRect,
    options: &PlacementOptions,
    transport: Transport,
) -> String {
    let mut fields = format!(
        "a=p,i={id},c={cols},r={rows}",
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
    if let Some(relative) = options.relative {
        fields.push_str(&format!(",P={}", relative.image_id));
        if let Some(q) = relative.placement_id {
            fields.push_str(&format!(",Q={q}"));
        }
        if relative.x_offset_px != 0 {
            fields.push_str(&format!(",H={}", relative.x_offset_px));
        }
        if relative.y_offset_px != 0 {
            fields.push_str(&format!(",V={}", relative.y_offset_px));
        }
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

/// Emit a kitty graphics capability query (`a=q`) with a caller-supplied id.
///
/// This is a pure encoder: it does not write to or read from the terminal. The
/// query intentionally omits `q=` so the terminal can respond.
pub fn query_capabilities(query_id: u32, transport: Transport) -> String {
    wrap_transport(format!("{ESC}_Ga=q,i={query_id}{ESC}\\"), transport)
}

/// Parse one kitty graphics response escape from terminal output.
///
/// The parser is deliberately I/O-free and can be used by future response
/// readers after they have collected bytes from a terminal they own.
pub fn parse_response(input: &str) -> Result<KittyResponse, KittyResponseParseError> {
    let start = input
        .find("\x1b_G")
        .ok_or(KittyResponseParseError::MissingEscape)?;
    let rest = &input[start + 3..];
    let end = rest
        .find("\x1b\\")
        .ok_or(KittyResponseParseError::MissingEscape)?;
    let packet = &rest[..end];
    let (header, body) = packet.split_once(';').unwrap_or((packet, ""));
    if header.trim().is_empty() {
        return Err(KittyResponseParseError::MalformedHeader);
    }

    let mut action = None;
    let mut image_id = None;
    let mut placement_id = None;
    for field in header.split(',').filter(|field| !field.is_empty()) {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key {
            "a" => action = Some(value.to_string()),
            "i" => image_id = Some(parse_u32_field(key, value)?),
            "p" => placement_id = Some(parse_u32_field(key, value)?),
            _ => {}
        }
    }

    let body_trimmed = body.trim();
    let status = if action.as_deref() == Some("q") && !body_trimmed.is_empty() {
        KittyResponseStatus::Capability(body_trimmed.to_string())
    } else if body_trimmed == "OK" {
        KittyResponseStatus::Ok
    } else if let Some(code) = error_code(body_trimmed) {
        KittyResponseStatus::Error(code.to_string())
    } else {
        KittyResponseStatus::Other(body_trimmed.to_string())
    };

    Ok(KittyResponse {
        action,
        image_id,
        placement_id,
        status,
        raw_body: body_trimmed.to_string(),
    })
}

fn parse_u32_field(field: &str, value: &str) -> Result<u32, KittyResponseParseError> {
    value
        .parse::<u32>()
        .map_err(|_| KittyResponseParseError::InvalidField {
            field: field.to_string(),
            value: value.to_string(),
        })
}

fn error_code(body: &str) -> Option<&str> {
    let code = body
        .split(|ch: char| ch == ':' || ch.is_ascii_whitespace())
        .next()
        .unwrap_or("");
    if code.starts_with('E')
        && code
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        Some(code)
    } else {
        None
    }
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
    frame_delay_ms: Option<u32>,
) -> String {
    let verb = if frame_index.is_some() && !first_frame {
        "a=f"
    } else {
        "a=t"
    };
    match medium {
        UploadMedium::Direct { bytes } => {
            let compression = resolve_compression_for_len(compression_from_env(), bytes.len());
            let payload = compress_payload(bytes, compression).unwrap_or_else(|| bytes.to_vec());
            let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
            encode_chunked(
                image_id,
                verb,
                &b64,
                frame_index,
                frame_delay_ms,
                quiet,
                transport,
                compression,
            )
        }
        UploadMedium::File { path } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(path_bytes(path));
            single_payload(
                image_id,
                verb,
                "f",
                &b64,
                frame_index,
                frame_delay_ms,
                quiet,
                transport,
            )
        }
        UploadMedium::TempFile { path } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(path_bytes(path));
            single_payload(
                image_id,
                verb,
                "t",
                &b64,
                frame_index,
                frame_delay_ms,
                quiet,
                transport,
            )
        }
        UploadMedium::SharedMemory { name } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(name.as_bytes());
            single_payload(
                image_id,
                verb,
                "s",
                &b64,
                frame_index,
                frame_delay_ms,
                quiet,
                transport,
            )
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
    frame_delay_ms: Option<u32>,
    quiet: Quiet,
    transport: Transport,
) -> String {
    let frame_field = frame_index.map(|i| format!(",r={i}")).unwrap_or_default();
    let delay_field = frame_delay_ms
        .map(|z| format!(",z={z}"))
        .unwrap_or_default();
    let header = format!(
        "{verb},f=100,t={medium},i={id}{frame}{delay}{q}",
        verb = verb,
        medium = medium_field,
        id = image_id,
        frame = frame_field,
        delay = delay_field,
        q = quiet.field(),
    );
    let payload = format!("{ESC}_G{header};{base64_body}{ESC}\\");
    wrap_transport(payload, transport)
}

fn single_payload_raw(
    image_id: u32,
    format: u8,
    medium_field: &str,
    base64_body: &str,
    width: u32,
    height: u32,
    quiet: Quiet,
    transport: Transport,
) -> String {
    let header = format!(
        "a=t,f={format},s={width},v={height},t={medium},i={id}{q}",
        format = format,
        width = width,
        height = height,
        medium = medium_field,
        id = image_id,
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
    frame_delay_ms: Option<u32>,
    quiet: Quiet,
    transport: Transport,
    compression: CompressionMode,
) -> String {
    const CHUNK: usize = 4096;
    let mut out = String::new();
    let bytes = base64_body.as_bytes();
    let mut offset = 0;
    let frame_field = frame_index.map(|i| format!(",r={i}")).unwrap_or_default();
    let delay_field = frame_delay_ms
        .map(|z| format!(",z={z}"))
        .unwrap_or_default();
    while offset < bytes.len() {
        let end = (offset + CHUNK).min(bytes.len());
        let more = if end < bytes.len() { 1 } else { 0 };
        let header = if offset == 0 {
            format!(
                "{verb},f=100,i={id},m={more}{frame_field}{delay_field}{compression}{q}",
                verb = verb,
                id = image_id,
                more = more,
                frame_field = frame_field,
                delay_field = delay_field,
                compression = compression.field(),
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
            relative: None,
        };
        let cmd = placement_command_ex(1, CellRect::new(0, 0, 8, 4), &opts, Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=p,i=1,c=8,r=4,p=7,X=4,Y=2,z=-1,q=2\x1b\\");
    }

    #[test]
    fn absolute_with_id_sets_stable_id_without_unicode_placeholder() {
        let opts = PlacementOptions::absolute_with_id(99);
        let cmd = placement_command_ex(1, CellRect::new(0, 0, 8, 4), &opts, Transport::Direct);
        assert!(cmd.contains("p=99"), "{cmd}");
        assert!(!cmd.contains("U=1"), "{cmd}");
    }

    #[test]
    fn stable_absolute_placement_helper_sets_id_without_placeholder() {
        let opts = PlacementOptions::stable_absolute(77).with_z_index(4);
        let cmd = placement_command_ex(9, CellRect::new(0, 0, 5, 2), &opts, Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=p,i=9,c=5,r=2,p=77,z=4,q=2\x1b\\");
        assert!(!cmd.contains("U=1"), "{cmd}");
    }

    #[test]
    fn placement_command_with_relative_anchor_fields() {
        let opts = PlacementOptions {
            quiet: Quiet::SuppressAll,
            relative: Some(RelativePlacement {
                image_id: 42,
                placement_id: Some(9),
                x_offset_px: -3,
                y_offset_px: 12,
            }),
            ..PlacementOptions::unicode()
        };
        let cmd = placement_command_ex(5, CellRect::new(0, 0, 8, 4), &opts, Transport::Direct);
        assert_eq!(
            cmd,
            "\x1b_Ga=p,i=5,c=8,r=4,U=1,P=42,Q=9,H=-3,V=12,q=2\x1b\\"
        );
    }

    #[test]
    fn placeholder_grid_carries_diacritics_per_cell() {
        let text = placeholder_text(0x010203, CellRect::new(0, 0, 2, 2));
        let placeholder_count = text.matches(PLACEHOLDER_CHAR).count();
        assert_eq!(placeholder_count, 4);
        // Each placeholder is followed by row + column diacritic; image id MSB is zero.
        let mut chars = text
            .chars()
            .filter(|c| *c == PLACEHOLDER_CHAR || ROWCOLUMN_DIACRITICS.contains(c));
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
        // First frame uses a=t without redundant r=1; second/third use a=f with r=2,r=3.
        // Per-frame delays live on the frame upload commands via z=<ms>.
        assert!(escapes.contains("\x1b_Ga=t,f=100,i=66,m=0,z=100,q=2;"));
        assert!(escapes.contains("\x1b_Ga=f,f=100,i=66,m=0,r=2,z=200,q=2;"));
        assert!(escapes.contains("\x1b_Ga=f,f=100,i=66,m=0,r=3,z=300,q=2;"));
        // Loop forever => s=2, no v field, no c field. (bd-ad5957)
        assert!(escapes.contains("\x1b_Ga=a,i=66,s=2,q=2\x1b\\"));
        assert!(!escapes.contains("a=a,i=66,r=1,z="));
    }

    #[test]
    fn animation_frame_helper_uses_t_for_first_and_f_for_later_frames() {
        let first = upload_animation_frame_ex(
            5,
            1,
            b"first",
            Some(33),
            Quiet::SuppressAll,
            Transport::Direct,
        );
        assert!(first.starts_with("\x1b_Ga=t,f=100,i=5,m=0,z=33,q=2;"));
        assert!(!first.contains(",r=1,"));

        let second = upload_animation_frame_ex(
            5,
            2,
            b"second",
            Some(44),
            Quiet::SuppressAll,
            Transport::Direct,
        );
        assert!(second.starts_with("\x1b_Ga=f,f=100,i=5,m=0,r=2,z=44,q=2;"));
    }

    #[test]
    fn animation_control_helper_emits_state_frame_and_loop_fields() {
        let control = animation_control_ex(
            7,
            AnimationState::PlayAndStop,
            Some(3),
            Some(2),
            Quiet::SuppressAll,
            Transport::Direct,
        );
        assert_eq!(control, "\x1b_Ga=a,i=7,s=3,c=2,v=3,q=2\x1b\\");

        let select = set_animation_frame(7, 0, Quiet::SuppressAll, Transport::Direct);
        assert_eq!(select, "\x1b_Ga=a,i=7,s=1,c=1,q=2\x1b\\");
    }

    #[test]
    fn animated_upload_finite_loops_uses_state_3_and_v_field() {
        let frames = vec![vec![1u8; 4], vec![2u8; 4]];
        let delays = vec![50, 50];
        let escapes = upload_animation(7, &frames, &delays, 5, Transport::Direct);
        assert!(
            escapes.contains("\x1b_Ga=a,i=7,s=3,v=5,q=2\x1b\\"),
            "finite loop count must emit s=3,v=<N>: {escapes}"
        );
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
        let expected_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"/tmp/kittui-image.png");
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
    fn upload_raw_rgb_via_temp_file_medium_sends_path_and_f24_shape() {
        let path = Path::new("/tmp/kittui-raw-frame.rgb");
        let escapes = upload_still_rgb_medium(
            13,
            UploadMedium::TempFile { path },
            64,
            32,
            Quiet::SuppressAll,
            Transport::Direct,
        );
        let expected_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"/tmp/kittui-raw-frame.rgb");
        let want = format!("\x1b_Ga=t,f=24,s=64,v=32,t=t,i=13,q=2;{expected_b64}\x1b\\");
        assert_eq!(escapes, want);
    }

    #[test]
    fn upload_raw_rgb_via_shared_memory_medium_sends_name_and_f24_shape() {
        let escapes = upload_still_rgb_medium(
            14,
            UploadMedium::SharedMemory {
                name: "/kittui-rgb-14",
            },
            8,
            4,
            Quiet::SuppressAll,
            Transport::Direct,
        );
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(b"/kittui-rgb-14");
        let want = format!("\x1b_Ga=t,f=24,s=8,v=4,t=s,i=14,q=2;{expected_b64}\x1b\\");
        assert_eq!(escapes, want);
    }

    #[test]
    fn upload_raw_rgba_via_temp_file_medium_sends_path_and_f32_shape() {
        let path = Path::new("/tmp/kittui-raw-frame.rgba");
        let escapes = upload_still_rgba_medium(
            11,
            UploadMedium::TempFile { path },
            64,
            32,
            Quiet::SuppressAll,
            Transport::Direct,
        );
        let expected_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"/tmp/kittui-raw-frame.rgba");
        let want = format!("\x1b_Ga=t,f=32,s=64,v=32,t=t,i=11,q=2;{expected_b64}\x1b\\");
        assert_eq!(escapes, want);
    }

    #[test]
    fn upload_raw_rgba_via_shared_memory_medium_sends_name_and_f32_shape() {
        let escapes = upload_still_rgba_medium(
            12,
            UploadMedium::SharedMemory {
                name: "/kittui-raw-12",
            },
            8,
            4,
            Quiet::SuppressAll,
            Transport::Direct,
        );
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(b"/kittui-raw-12");
        let want = format!("\x1b_Ga=t,f=32,s=8,v=4,t=s,i=12,q=2;{expected_b64}\x1b\\");
        assert_eq!(escapes, want);
    }

    #[test]
    fn cursor_move_is_one_indexed_csi_h() {
        // (col=4, row=2) 0-indexed -> CSI 3;5 H (1-indexed). (bd-12568a)
        let s = cursor_move(4, 2, Transport::Direct);
        assert_eq!(s, "\x1b[3;5H");
    }

    #[test]
    fn cursor_move_origin_emits_csi_1_1_h() {
        let s = cursor_move(0, 0, Transport::Direct);
        assert_eq!(s, "\x1b[1;1H");
    }

    #[test]
    fn cursor_move_under_tmux_wraps_passthrough() {
        let s = cursor_move(1, 1, Transport::TmuxPassthrough);
        assert!(s.starts_with("\x1bPtmux;\x1b\x1b["));
        assert!(s.ends_with("\x1b\\"));
    }

    #[test]
    fn delete_by_placement_emits_p_field() {
        let cmd = delete_placement(0x55, 3, Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=d,d=I,i=85,p=3,q=2\x1b\\");
    }

    #[test]
    fn query_capabilities_emits_a_q_without_quiet_suppression() {
        let cmd = query_capabilities(123, Transport::Direct);
        assert_eq!(cmd, "\x1b_Ga=q,i=123\x1b\\");
        assert!(!cmd.contains(",q="));
    }

    #[test]
    fn query_capabilities_wraps_for_tmux_passthrough() {
        let cmd = query_capabilities(7, Transport::TmuxPassthrough);
        assert_eq!(cmd, "\x1bPtmux;\x1b\x1b_Ga=q,i=7\x1b\x1b\\\x1b\\");
    }

    #[test]
    fn parse_response_decodes_ok_error_and_capability_replies() {
        let ok = parse_response("noise\x1b_Gi=42,p=9;OK\x1b\\tail").unwrap();
        assert_eq!(ok.image_id, Some(42));
        assert_eq!(ok.placement_id, Some(9));
        assert_eq!(ok.status, KittyResponseStatus::Ok);

        let err = parse_response("\x1b_Gi=42;ENOENT: image not found\x1b\\").unwrap();
        assert_eq!(err.status, KittyResponseStatus::Error("ENOENT".to_string()));
        assert_eq!(err.raw_body, "ENOENT: image not found");

        let caps = parse_response("\x1b_Ga=q,i=77;OK: f=24,f=32,t=d\x1b\\").unwrap();
        assert_eq!(caps.action.as_deref(), Some("q"));
        assert_eq!(caps.image_id, Some(77));
        assert_eq!(
            caps.status,
            KittyResponseStatus::Capability("OK: f=24,f=32,t=d".to_string())
        );
    }

    #[test]
    fn parse_response_rejects_missing_escape_and_bad_numeric_fields() {
        assert_eq!(
            parse_response("not a response").unwrap_err(),
            KittyResponseParseError::MissingEscape
        );
        assert_eq!(
            parse_response("\x1b_Gi=abc;OK\x1b\\").unwrap_err(),
            KittyResponseParseError::InvalidField {
                field: "i".to_string(),
                value: "abc".to_string(),
            }
        );
    }

    #[test]
    fn upload_still_rgb_emits_f24_grammar_with_s_v_width_height() {
        let rgb: Vec<u8> = vec![
            0xff, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00,
        ];
        let escapes = upload_still_rgb(0x1234, &rgb, 2, 2, Transport::Direct);
        assert!(
            escapes.starts_with("\x1b_Ga=t,f=24,s=2,v=2,i=4660,m=0,q=2;"),
            "raw RGB upload must use f=24,s=W,v=H: prefix was {}",
            &escapes[..escapes.len().min(60)]
        );
        assert!(escapes.ends_with("\x1b\\"));
        assert!(!escapes.contains("PNG"));
        let body = escapes
            .split_once(';')
            .unwrap()
            .1
            .trim_end_matches("\x1b\\");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap();
        assert_eq!(decoded, rgb);
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
    fn compression_auto_respects_min_byte_threshold() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTUI_ZLIB_MIN_BYTES", "32");
        assert_eq!(
            resolve_compression_for_len(CompressionMode::Auto, 31),
            CompressionMode::None
        );
        assert_eq!(
            resolve_compression_for_len(CompressionMode::Auto, 32),
            CompressionMode::Zlib
        );
        std::env::remove_var("KITTUI_ZLIB_MIN_BYTES");
    }

    #[test]
    fn upload_still_rgba_auto_compresses_only_large_payloads() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KITTUI_ZLIB_MIN_BYTES", "32");
        let small = upload_still_rgba_compressed(
            4,
            &[0x55u8; 16],
            2,
            2,
            Quiet::SuppressAll,
            Transport::Direct,
            CompressionMode::Auto,
        );
        assert!(
            small.starts_with("\x1b_Ga=t,f=32,s=2,v=2,i=4,m=0,q=2;"),
            "{small:?}"
        );
        let large = upload_still_rgba_compressed(
            4,
            &[0x55u8; 64],
            4,
            4,
            Quiet::SuppressAll,
            Transport::Direct,
            CompressionMode::Auto,
        );
        assert!(
            large.starts_with("\x1b_Ga=t,f=32,s=4,v=4,i=4,m=0,o=z,q=2;"),
            "{large:?}"
        );
        std::env::remove_var("KITTUI_ZLIB_MIN_BYTES");
    }

    #[test]
    fn upload_still_rgb_supports_zlib_compression() {
        let rgb = vec![0x3fu8; 48];
        let escapes = upload_still_rgb_compressed(
            6,
            &rgb,
            4,
            4,
            Quiet::SuppressAll,
            Transport::Direct,
            CompressionMode::Zlib,
        );
        assert!(
            escapes.starts_with("\x1b_Ga=t,f=24,s=4,v=4,i=6,m=0,o=z,q=2;"),
            "compressed raw RGB upload must mark o=z and f=24: {escapes:?}"
        );
        let body = escapes
            .split_once(';')
            .and_then(|(_, rest)| rest.strip_suffix("\x1b\\"))
            .unwrap();
        let compressed = base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap();
        let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
        let mut decoded = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decoded).unwrap();
        assert_eq!(decoded, rgb);
    }

    #[test]
    fn upload_still_rgba_supports_zlib_compression() {
        let rgba = vec![0x7fu8; 64];
        let escapes = upload_still_rgba_compressed(
            3,
            &rgba,
            4,
            4,
            Quiet::SuppressAll,
            Transport::Direct,
            CompressionMode::Zlib,
        );
        assert!(
            escapes.starts_with("\x1b_Ga=t,f=32,s=4,v=4,i=3,m=0,o=z,q=2;"),
            "compressed raw RGBA upload must mark o=z: {escapes:?}"
        );
        let body = escapes
            .split_once(';')
            .and_then(|(_, rest)| rest.strip_suffix("\x1b\\"))
            .unwrap();
        let compressed = base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap();
        let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
        let mut decoded = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decoded).unwrap();
        assert_eq!(decoded, rgba);
    }

    #[test]
    fn diacritic_table_is_exactly_297_entries() {
        assert_eq!(ROWCOLUMN_DIACRITICS_COUNT, 297);
        assert_eq!(ROWCOLUMN_DIACRITICS.len(), 297);
    }
}
