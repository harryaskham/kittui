//! Portable libghostty-vt proof bindings for kittui/kittwm.
//!
//! This crate intentionally targets the portable `libghostty-vt` package from
//! nixpkgs via `pkg-config`; it does **not** link against macOS `Ghostty.app` or
//! any AppKit/Metal surface symbols. The library gives us Ghostty's VT parser,
//! terminal state, formatter, render-state iterator, and kitty graphics metadata
//! APIs. Pixel-perfect headless Ghostty screenshots still require a renderer on
//! top of the render state (or a future upstream headless surface API), but this
//! proves the core VT state can be driven from Rust in a portable way.

use anyhow::{anyhow, bail, Result};
use std::ffi::c_void;
use std::ptr;

#[allow(non_camel_case_types)]
mod ffi {
    use super::c_void;

    pub type GhosttyResult = i32;
    pub const GHOSTTY_SUCCESS: GhosttyResult = 0;
    pub const GHOSTTY_OUT_OF_SPACE: GhosttyResult = -3;

    #[repr(C)]
    pub struct GhosttyAllocator {
        _private: [u8; 0],
    }

    pub type GhosttyTerminal = *mut c_void;
    pub type GhosttyFormatter = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyTerminalOptions {
        pub cols: u16,
        pub rows: u16,
        pub max_scrollback: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyFormatterScreenExtra {
        pub size: usize,
        pub cursor: bool,
        pub style: bool,
        pub hyperlink: bool,
        pub protection: bool,
        pub kitty_keyboard: bool,
        pub charsets: bool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyFormatterTerminalExtra {
        pub size: usize,
        pub palette: bool,
        pub modes: bool,
        pub scrolling_region: bool,
        pub tabstops: bool,
        pub pwd: bool,
        pub keyboard: bool,
        pub screen: GhosttyFormatterScreenExtra,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyFormatterTerminalOptions {
        pub size: usize,
        pub emit: i32,
        pub unwrap: bool,
        pub trim: bool,
        pub extra: GhosttyFormatterTerminalExtra,
        pub selection: *const c_void,
    }

    pub const GHOSTTY_FORMATTER_FORMAT_PLAIN: i32 = 0;
    pub const GHOSTTY_FORMATTER_FORMAT_VT: i32 = 1;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyString {
        pub ptr: *const u8,
        pub len: usize,
    }

    pub const GHOSTTY_TERMINAL_DATA_COLS: i32 = 1;
    pub const GHOSTTY_TERMINAL_DATA_ROWS: i32 = 2;
    pub const GHOSTTY_TERMINAL_DATA_CURSOR_X: i32 = 3;
    pub const GHOSTTY_TERMINAL_DATA_CURSOR_Y: i32 = 4;
    pub const GHOSTTY_TERMINAL_DATA_TITLE: i32 = 12;

    extern "C" {
        pub fn ghostty_terminal_new(
            allocator: *const GhosttyAllocator,
            terminal: *mut GhosttyTerminal,
            options: GhosttyTerminalOptions,
        ) -> GhosttyResult;
        pub fn ghostty_terminal_free(terminal: GhosttyTerminal);
        pub fn ghostty_terminal_vt_write(terminal: GhosttyTerminal, data: *const u8, len: usize);
        pub fn ghostty_terminal_get(
            terminal: GhosttyTerminal,
            data: i32,
            out: *mut c_void,
        ) -> GhosttyResult;

        pub fn ghostty_formatter_terminal_new(
            allocator: *const GhosttyAllocator,
            formatter: *mut GhosttyFormatter,
            terminal: GhosttyTerminal,
            options: GhosttyFormatterTerminalOptions,
        ) -> GhosttyResult;
        pub fn ghostty_formatter_format_buf(
            formatter: GhosttyFormatter,
            buf: *mut u8,
            buf_len: usize,
            out_written: *mut usize,
        ) -> GhosttyResult;
        pub fn ghostty_formatter_free(formatter: GhosttyFormatter);
    }
}

/// Snapshot of terminal state after feeding bytes through libghostty-vt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhosttyVtSnapshot {
    /// Terminal width in cells.
    pub cols: u16,
    /// Terminal height in cells.
    pub rows: u16,
    /// Cursor x position in cells.
    pub cursor_x: u16,
    /// Cursor y position in cells.
    pub cursor_y: u16,
    /// Title set via OSC 0/2, if any.
    pub title: String,
    /// Plain-text formatted active screen.
    pub plain_text: String,
    /// VT-formatted active screen, preserving SGR/style state where possible.
    pub vt_text: String,
}

/// Owned libghostty-vt terminal.
pub struct GhosttyVtTerminal {
    raw: ffi::GhosttyTerminal,
}

impl GhosttyVtTerminal {
    /// Create a terminal with the requested cell dimensions and scrollback.
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self> {
        if cols == 0 || rows == 0 {
            bail!("terminal dimensions must be non-zero");
        }
        let mut raw = ptr::null_mut();
        let result = unsafe {
            ffi::ghostty_terminal_new(
                ptr::null(),
                &mut raw,
                ffi::GhosttyTerminalOptions {
                    cols,
                    rows,
                    max_scrollback,
                },
            )
        };
        check(result, "ghostty_terminal_new")?;
        if raw.is_null() {
            bail!("ghostty_terminal_new returned a null terminal");
        }
        Ok(Self { raw })
    }

    /// Feed raw terminal bytes into Ghostty's VT parser.
    pub fn write(&mut self, bytes: impl AsRef<[u8]>) {
        let bytes = bytes.as_ref();
        if bytes.is_empty() {
            return;
        }
        unsafe { ffi::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
    }

    /// Capture a text/style snapshot via libghostty-vt's formatter APIs.
    pub fn snapshot(&self) -> Result<GhosttyVtSnapshot> {
        Ok(GhosttyVtSnapshot {
            cols: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_COLS, "cols")?,
            rows: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_ROWS, "rows")?,
            cursor_x: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_CURSOR_X, "cursor_x")?,
            cursor_y: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_CURSOR_Y, "cursor_y")?,
            title: self.title()?,
            plain_text: self.format(ffi::GHOSTTY_FORMATTER_FORMAT_PLAIN, true, true)?,
            vt_text: self.format(ffi::GHOSTTY_FORMATTER_FORMAT_VT, false, false)?,
        })
    }

    fn get_u16(&self, data: i32, label: &str) -> Result<u16> {
        let mut out = 0u16;
        let result = unsafe {
            ffi::ghostty_terminal_get(self.raw, data, (&mut out as *mut u16).cast::<c_void>())
        };
        check(result, label)?;
        Ok(out)
    }

    fn title(&self) -> Result<String> {
        let mut out = ffi::GhosttyString {
            ptr: ptr::null(),
            len: 0,
        };
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.raw,
                ffi::GHOSTTY_TERMINAL_DATA_TITLE,
                (&mut out as *mut ffi::GhosttyString).cast::<c_void>(),
            )
        };
        check(result, "title")?;
        if out.ptr.is_null() || out.len == 0 {
            return Ok(String::new());
        }
        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn format(&self, emit: i32, unwrap: bool, trim: bool) -> Result<String> {
        let mut formatter = ptr::null_mut();
        let opts = formatter_options(emit, unwrap, trim);
        let result = unsafe {
            ffi::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.raw, opts)
        };
        check(result, "ghostty_formatter_terminal_new")?;
        if formatter.is_null() {
            bail!("ghostty_formatter_terminal_new returned a null formatter");
        }
        let formatter = FormatterGuard(formatter);

        let mut needed = 0usize;
        let result = unsafe {
            ffi::ghostty_formatter_format_buf(formatter.0, ptr::null_mut(), 0, &mut needed)
        };
        if result != ffi::GHOSTTY_OUT_OF_SPACE && result != ffi::GHOSTTY_SUCCESS {
            check(result, "ghostty_formatter_format_buf(size)")?;
        }
        let mut buf = vec![0u8; needed.max(1)];
        let mut written = 0usize;
        let result = unsafe {
            ffi::ghostty_formatter_format_buf(
                formatter.0,
                buf.as_mut_ptr(),
                buf.len(),
                &mut written,
            )
        };
        check(result, "ghostty_formatter_format_buf")?;
        buf.truncate(written);
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

impl Drop for GhosttyVtTerminal {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.raw) };
    }
}

struct FormatterGuard(ffi::GhosttyFormatter);

impl Drop for FormatterGuard {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.0) };
    }
}

fn formatter_options(emit: i32, unwrap: bool, trim: bool) -> ffi::GhosttyFormatterTerminalOptions {
    ffi::GhosttyFormatterTerminalOptions {
        size: std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>(),
        emit,
        unwrap,
        trim,
        extra: ffi::GhosttyFormatterTerminalExtra {
            size: std::mem::size_of::<ffi::GhosttyFormatterTerminalExtra>(),
            palette: false,
            modes: false,
            scrolling_region: false,
            tabstops: false,
            pwd: false,
            keyboard: false,
            screen: ffi::GhosttyFormatterScreenExtra {
                size: std::mem::size_of::<ffi::GhosttyFormatterScreenExtra>(),
                cursor: false,
                style: emit == ffi::GHOSTTY_FORMATTER_FORMAT_VT,
                hyperlink: false,
                protection: false,
                kitty_keyboard: false,
                charsets: false,
            },
        },
        selection: ptr::null(),
    }
}

fn check(result: ffi::GhosttyResult, context: &str) -> Result<()> {
    if result == ffi::GHOSTTY_SUCCESS {
        Ok(())
    } else {
        Err(anyhow!(
            "{context} failed with libghostty-vt result {result}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_vt_bytes_and_formats_snapshot() {
        let mut term = GhosttyVtTerminal::new(20, 4, 100).unwrap();
        term.write(b"hello\n\x1b[31mred\x1b[0m\x1b]0;kittui-title\x07");
        let snap = term.snapshot().unwrap();
        assert_eq!(snap.cols, 20);
        assert_eq!(snap.rows, 4);
        assert_eq!(snap.title, "kittui-title");
        assert!(snap.plain_text.contains("hello"), "{:?}", snap.plain_text);
        assert!(snap.plain_text.contains("red"), "{:?}", snap.plain_text);
        assert!(snap.vt_text.contains("\u{1b}[31m") || snap.vt_text.contains("red"));
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(GhosttyVtTerminal::new(0, 24, 0).is_err());
        assert!(GhosttyVtTerminal::new(80, 0, 0).is_err());
    }
}

/// Styling options for deterministic PNG previews produced from a
/// libghostty-vt text snapshot.
#[derive(Debug, Clone)]
pub struct PreviewOptions {
    /// Width of one terminal cell in pixels.
    pub cell_width: u32,
    /// Height of one terminal cell in pixels.
    pub cell_height: u32,
    /// Background RGBA.
    pub background: [u8; 4],
    /// Foreground RGBA.
    pub foreground: [u8; 4],
    /// Accent RGBA for the cursor.
    pub cursor: [u8; 4],
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            cell_width: 8,
            cell_height: 12,
            background: [7, 17, 31, 255],
            foreground: [216, 222, 233, 255],
            cursor: [235, 203, 139, 255],
        }
    }
}

/// Render the snapshot's plain-text screen into a deterministic PNG preview.
///
/// This is an interim evidence renderer: it visualizes the terminal text/state
/// driven by libghostty-vt, but it does not yet consume Ghostty render-state
/// cell colors/styles or kitty image placement metadata.
pub fn snapshot_preview_png(
    snapshot: &GhosttyVtSnapshot,
    options: &PreviewOptions,
) -> Result<Vec<u8>> {
    use image::{ImageBuffer, ImageEncoder, Rgba};

    let width = u32::from(snapshot.cols).max(1) * options.cell_width;
    let height = u32::from(snapshot.rows).max(1) * options.cell_height;
    let mut img = ImageBuffer::from_pixel(width, height, Rgba(options.background));
    for (row, line) in snapshot
        .plain_text
        .lines()
        .take(snapshot.rows as usize)
        .enumerate()
    {
        for (col, ch) in line.chars().take(snapshot.cols as usize).enumerate() {
            draw_ascii_cell(&mut img, col as u32, row as u32, ch, options);
        }
    }
    draw_cursor(&mut img, snapshot.cursor_x, snapshot.cursor_y, options);

    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes).write_image(
        img.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(bytes)
}

fn draw_ascii_cell(
    img: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    col: u32,
    row: u32,
    ch: char,
    options: &PreviewOptions,
) {
    use font8x8::UnicodeFonts;

    if ch == ' ' || ch == '\0' {
        return;
    }
    let Some(glyph) = font8x8::BASIC_FONTS.get(ch) else {
        return;
    };
    let x0 = col * options.cell_width;
    let y0 = row * options.cell_height;
    let color = image::Rgba(options.foreground);
    let x_pad = options.cell_width.saturating_sub(8) / 2;
    let y_pad = options.cell_height.saturating_sub(8) / 2;
    for (gy, row_bits) in glyph.iter().enumerate() {
        for gx in 0..8u32 {
            if (row_bits >> gx) & 1 == 1 {
                let x = x0 + x_pad + gx;
                let y = y0 + y_pad + gy as u32;
                if x < img.width() && y < img.height() {
                    img.put_pixel(x, y, color);
                }
            }
        }
    }
}

fn draw_cursor(
    img: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    cursor_x: u16,
    cursor_y: u16,
    options: &PreviewOptions,
) {
    let x0 = u32::from(cursor_x) * options.cell_width;
    let y0 = u32::from(cursor_y) * options.cell_height;
    let color = image::Rgba(options.cursor);
    for y in y0..(y0 + options.cell_height).min(img.height()) {
        for x in x0..(x0 + options.cell_width).min(img.width()) {
            if y == y0
                || y + 1 == (y0 + options.cell_height).min(img.height())
                || x == x0
                || x + 1 == (x0 + options.cell_width).min(img.width())
            {
                img.put_pixel(x, y, color);
            }
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn snapshot_preview_png_emits_png_bytes() {
        let snapshot = GhosttyVtSnapshot {
            cols: 12,
            rows: 3,
            cursor_x: 2,
            cursor_y: 1,
            title: "demo".to_string(),
            plain_text: "hello\nworld".to_string(),
            vt_text: String::new(),
        };
        let png = snapshot_preview_png(&snapshot, &PreviewOptions::default()).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 100);
    }
}
