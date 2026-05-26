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
    pub type GhosttyRenderState = *mut c_void;
    pub type GhosttyRenderStateRowIterator = *mut c_void;
    pub type GhosttyRenderStateRowCells = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyTerminalOptions {
        pub cols: u16,
        pub rows: u16,
        pub max_scrollback: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union GhosttyTerminalScrollViewportValue {
        pub delta: isize,
        pub _padding: [u64; 2],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyTerminalScrollViewport {
        pub tag: i32,
        pub value: GhosttyTerminalScrollViewportValue,
    }

    pub const GHOSTTY_SCROLL_VIEWPORT_TOP: i32 = 0;
    pub const GHOSTTY_SCROLL_VIEWPORT_BOTTOM: i32 = 1;

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

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct GhosttyColorRgb {
        pub r: u8,
        pub g: u8,
        pub b: u8,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union GhosttyStyleColorValue {
        pub palette: u8,
        pub rgb: GhosttyColorRgb,
        pub _padding: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyStyleColor {
        pub tag: i32,
        pub value: GhosttyStyleColorValue,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GhosttyStyle {
        pub size: usize,
        pub fg_color: GhosttyStyleColor,
        pub bg_color: GhosttyStyleColor,
        pub underline_color: GhosttyStyleColor,
        pub bold: bool,
        pub italic: bool,
        pub faint: bool,
        pub blink: bool,
        pub inverse: bool,
        pub invisible: bool,
        pub strikethrough: bool,
        pub overline: bool,
        pub underline: i32,
    }

    pub const GHOSTTY_TERMINAL_DATA_COLS: i32 = 1;
    pub const GHOSTTY_TERMINAL_DATA_ROWS: i32 = 2;
    pub const GHOSTTY_TERMINAL_DATA_CURSOR_X: i32 = 3;
    pub const GHOSTTY_TERMINAL_DATA_CURSOR_Y: i32 = 4;
    pub const GHOSTTY_TERMINAL_DATA_TITLE: i32 = 12;

    pub const GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR: i32 = 4;
    pub const GHOSTTY_RENDER_STATE_ROW_DATA_CELLS: i32 = 3;
    pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE: i32 = 2;
    pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN: i32 = 3;
    pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF: i32 = 4;
    pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR: i32 = 5;
    pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR: i32 = 6;

    extern "C" {
        pub fn ghostty_terminal_new(
            allocator: *const GhosttyAllocator,
            terminal: *mut GhosttyTerminal,
            options: GhosttyTerminalOptions,
        ) -> GhosttyResult;
        pub fn ghostty_terminal_free(terminal: GhosttyTerminal);
        pub fn ghostty_terminal_vt_write(terminal: GhosttyTerminal, data: *const u8, len: usize);
        pub fn ghostty_terminal_scroll_viewport(
            terminal: GhosttyTerminal,
            behavior: GhosttyTerminalScrollViewport,
        );
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

        pub fn ghostty_render_state_new(
            allocator: *const GhosttyAllocator,
            state: *mut GhosttyRenderState,
        ) -> GhosttyResult;
        pub fn ghostty_render_state_free(state: GhosttyRenderState);
        pub fn ghostty_render_state_update(
            state: GhosttyRenderState,
            terminal: GhosttyTerminal,
        ) -> GhosttyResult;
        pub fn ghostty_render_state_get(
            state: GhosttyRenderState,
            data: i32,
            out: *mut c_void,
        ) -> GhosttyResult;
        pub fn ghostty_render_state_row_iterator_new(
            allocator: *const GhosttyAllocator,
            out_iterator: *mut GhosttyRenderStateRowIterator,
        ) -> GhosttyResult;
        pub fn ghostty_render_state_row_iterator_free(iterator: GhosttyRenderStateRowIterator);
        pub fn ghostty_render_state_row_iterator_next(
            iterator: GhosttyRenderStateRowIterator,
        ) -> bool;
        pub fn ghostty_render_state_row_get(
            iterator: GhosttyRenderStateRowIterator,
            data: i32,
            out: *mut c_void,
        ) -> GhosttyResult;
        pub fn ghostty_render_state_row_cells_new(
            allocator: *const GhosttyAllocator,
            out_cells: *mut GhosttyRenderStateRowCells,
        ) -> GhosttyResult;
        pub fn ghostty_render_state_row_cells_free(cells: GhosttyRenderStateRowCells);
        pub fn ghostty_render_state_row_cells_next(cells: GhosttyRenderStateRowCells) -> bool;
        pub fn ghostty_render_state_row_cells_get(
            cells: GhosttyRenderStateRowCells,
            data: i32,
            out: *mut c_void,
        ) -> GhosttyResult;
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

/// One terminal cell extracted from libghostty-vt render state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhosttyCellSnapshot {
    /// Text grapheme for this cell, if any.
    pub text: String,
    /// Resolved foreground color when libghostty-vt reports one.
    pub fg: Option<[u8; 3]>,
    /// Resolved background color when libghostty-vt reports one.
    pub bg: Option<[u8; 3]>,
    /// Whether the cell style is bold.
    pub bold: bool,
    /// Whether the cell style is italic.
    pub italic: bool,
    /// Underline style value from libghostty-vt; 0 means none.
    pub underline: i32,
}

/// Render-state rows/cells extracted from libghostty-vt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhosttyRenderSnapshot {
    /// Terminal width in cells.
    pub cols: u16,
    /// Terminal height in cells.
    pub rows: u16,
    /// Cursor x position in cells.
    pub cursor_x: u16,
    /// Cursor y position in cells.
    pub cursor_y: u16,
    /// Row-major cell data.
    pub cells: Vec<Vec<GhosttyCellSnapshot>>,
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

    /// Scroll the viewport to the top of available scrollback.
    pub fn scroll_top(&mut self) {
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw,
                ffi::GhosttyTerminalScrollViewport {
                    tag: ffi::GHOSTTY_SCROLL_VIEWPORT_TOP,
                    value: ffi::GhosttyTerminalScrollViewportValue { _padding: [0; 2] },
                },
            )
        };
    }

    /// Scroll the viewport to the bottom/current active area.
    pub fn scroll_bottom(&mut self) {
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw,
                ffi::GhosttyTerminalScrollViewport {
                    tag: ffi::GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
                    value: ffi::GhosttyTerminalScrollViewportValue { _padding: [0; 2] },
                },
            )
        };
    }

    /// Extract rows/cells from libghostty-vt render state.
    pub fn render_snapshot(&self) -> Result<GhosttyRenderSnapshot> {
        let mut state = ptr::null_mut();
        check(
            unsafe { ffi::ghostty_render_state_new(ptr::null(), &mut state) },
            "ghostty_render_state_new",
        )?;
        if state.is_null() {
            bail!("ghostty_render_state_new returned a null state");
        }
        let state = RenderStateGuard(state);
        check(
            unsafe { ffi::ghostty_render_state_update(state.0, self.raw) },
            "ghostty_render_state_update",
        )?;

        let mut rows_iter = ptr::null_mut();
        check(
            unsafe { ffi::ghostty_render_state_row_iterator_new(ptr::null(), &mut rows_iter) },
            "ghostty_render_state_row_iterator_new",
        )?;
        if rows_iter.is_null() {
            bail!("ghostty_render_state_row_iterator_new returned null");
        }
        let mut rows_iter = RowIteratorGuard(rows_iter);
        check(
            unsafe {
                ffi::ghostty_render_state_get(
                    state.0,
                    ffi::GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                    (&mut rows_iter.0 as *mut ffi::GhosttyRenderStateRowIterator).cast::<c_void>(),
                )
            },
            "ghostty_render_state_get(row_iterator)",
        )?;

        let mut rows = Vec::new();
        while unsafe { ffi::ghostty_render_state_row_iterator_next(rows_iter.0) } {
            let mut cells = ptr::null_mut();
            check(
                unsafe { ffi::ghostty_render_state_row_cells_new(ptr::null(), &mut cells) },
                "ghostty_render_state_row_cells_new",
            )?;
            if cells.is_null() {
                bail!("ghostty_render_state_row_cells_new returned null");
            }
            let mut cells = RowCellsGuard(cells);
            check(
                unsafe {
                    ffi::ghostty_render_state_row_get(
                        rows_iter.0,
                        ffi::GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                        (&mut cells.0 as *mut ffi::GhosttyRenderStateRowCells).cast::<c_void>(),
                    )
                },
                "ghostty_render_state_row_get(cells)",
            )?;
            let mut row = Vec::new();
            while unsafe { ffi::ghostty_render_state_row_cells_next(cells.0) } {
                row.push(read_render_cell(cells.0)?);
            }
            rows.push(row);
        }

        Ok(GhosttyRenderSnapshot {
            cols: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_COLS, "cols")?,
            rows: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_ROWS, "rows")?,
            cursor_x: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_CURSOR_X, "cursor_x")?,
            cursor_y: self.get_u16(ffi::GHOSTTY_TERMINAL_DATA_CURSOR_Y, "cursor_y")?,
            cells: rows,
        })
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

struct RenderStateGuard(ffi::GhosttyRenderState);

impl Drop for RenderStateGuard {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_free(self.0) };
    }
}

struct RowIteratorGuard(ffi::GhosttyRenderStateRowIterator);

impl Drop for RowIteratorGuard {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_iterator_free(self.0) };
    }
}

struct RowCellsGuard(ffi::GhosttyRenderStateRowCells);

impl Drop for RowCellsGuard {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_cells_free(self.0) };
    }
}

fn read_render_cell(cells: ffi::GhosttyRenderStateRowCells) -> Result<GhosttyCellSnapshot> {
    let mut len = 0u32;
    check(
        unsafe {
            ffi::ghostty_render_state_row_cells_get(
                cells,
                ffi::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                (&mut len as *mut u32).cast::<c_void>(),
            )
        },
        "ghostty_render_state_row_cells_get(graphemes_len)",
    )?;
    let mut graphemes = vec![0u32; len as usize];
    if len > 0 {
        check(
            unsafe {
                ffi::ghostty_render_state_row_cells_get(
                    cells,
                    ffi::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                    graphemes.as_mut_ptr().cast::<c_void>(),
                )
            },
            "ghostty_render_state_row_cells_get(graphemes_buf)",
        )?;
    }
    let text = graphemes
        .into_iter()
        .filter_map(char::from_u32)
        .collect::<String>();
    let style = read_cell_style(cells);
    Ok(GhosttyCellSnapshot {
        text,
        fg: read_cell_color(cells, ffi::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR),
        bg: read_cell_color(cells, ffi::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR),
        bold: style.map(|style| style.bold).unwrap_or(false),
        italic: style.map(|style| style.italic).unwrap_or(false),
        underline: style.map(|style| style.underline).unwrap_or(0),
    })
}

fn read_cell_style(cells: ffi::GhosttyRenderStateRowCells) -> Option<ffi::GhosttyStyle> {
    let mut style = ffi::GhosttyStyle {
        size: std::mem::size_of::<ffi::GhosttyStyle>(),
        fg_color: ffi::GhosttyStyleColor {
            tag: 0,
            value: ffi::GhosttyStyleColorValue { _padding: 0 },
        },
        bg_color: ffi::GhosttyStyleColor {
            tag: 0,
            value: ffi::GhosttyStyleColorValue { _padding: 0 },
        },
        underline_color: ffi::GhosttyStyleColor {
            tag: 0,
            value: ffi::GhosttyStyleColorValue { _padding: 0 },
        },
        bold: false,
        italic: false,
        faint: false,
        blink: false,
        inverse: false,
        invisible: false,
        strikethrough: false,
        overline: false,
        underline: 0,
    };
    let result = unsafe {
        ffi::ghostty_render_state_row_cells_get(
            cells,
            ffi::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
            (&mut style as *mut ffi::GhosttyStyle).cast::<c_void>(),
        )
    };
    (result == ffi::GHOSTTY_SUCCESS).then_some(style)
}

fn read_cell_color(cells: ffi::GhosttyRenderStateRowCells, data: i32) -> Option<[u8; 3]> {
    let mut color = ffi::GhosttyColorRgb { r: 0, g: 0, b: 0 };
    let result = unsafe {
        ffi::ghostty_render_state_row_cells_get(
            cells,
            data,
            (&mut color as *mut ffi::GhosttyColorRgb).cast::<c_void>(),
        )
    };
    (result == ffi::GHOSTTY_SUCCESS).then_some([color.r, color.g, color.b])
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

    #[test]
    fn scroll_top_and_bottom_are_safe() {
        let mut term = GhosttyVtTerminal::new(10, 2, 10).unwrap();
        term.write(b"one\ntwo\nthree\nfour\n");
        term.scroll_top();
        let top = term.render_snapshot().unwrap();
        term.scroll_bottom();
        let bottom = term.render_snapshot().unwrap();
        assert_eq!(top.cols, 10);
        assert_eq!(bottom.rows, 2);
    }

    #[test]
    fn extracts_render_state_cells_and_colors() {
        let mut term = GhosttyVtTerminal::new(20, 4, 100).unwrap();
        term.write(b"hello\n\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m \x1b[4mul\x1b[0m");
        let render = term.render_snapshot().unwrap();
        assert_eq!(render.cols, 20);
        assert_eq!(render.rows, 4);
        let text = render
            .cells
            .iter()
            .flat_map(|row| row.iter())
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        assert!(text.contains("hello"), "{text:?}");
        assert!(text.contains("red"), "{text:?}");
        assert!(
            render
                .cells
                .iter()
                .flat_map(|row| row.iter())
                .any(|cell| cell.text == "r" && cell.fg.is_some()),
            "{render:?}"
        );
        assert!(
            render
                .cells
                .iter()
                .flat_map(|row| row.iter())
                .any(|cell| cell.text == "b" && cell.bold),
            "{render:?}"
        );
        assert!(
            render
                .cells
                .iter()
                .flat_map(|row| row.iter())
                .any(|cell| cell.text == "u" && cell.underline != 0),
            "{render:?}"
        );
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
    x_offset: u32,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            cell_width: 8,
            cell_height: 12,
            background: [7, 17, 31, 255],
            foreground: [216, 222, 233, 255],
            cursor: [235, 203, 139, 255],
            x_offset: 0,
        }
    }
}

impl PreviewOptions {
    fn with_x_offset(mut self, x_offset: u32) -> Self {
        self.x_offset = x_offset;
        self
    }
}

/// Render the snapshot's plain-text screen into a deterministic PNG preview.
///
/// This compatibility helper uses formatter text. Prefer
/// [`render_snapshot_preview_png`] when render-state cells are available.
pub fn snapshot_preview_png(
    snapshot: &GhosttyVtSnapshot,
    options: &PreviewOptions,
) -> Result<Vec<u8>> {
    let cells = snapshot
        .plain_text
        .lines()
        .take(snapshot.rows as usize)
        .map(|line| {
            line.chars()
                .take(snapshot.cols as usize)
                .map(|ch| GhosttyCellSnapshot {
                    text: ch.to_string(),
                    fg: None,
                    bg: None,
                    bold: false,
                    italic: false,
                    underline: 0,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    render_snapshot_preview_png(
        &GhosttyRenderSnapshot {
            cols: snapshot.cols,
            rows: snapshot.rows,
            cursor_x: snapshot.cursor_x,
            cursor_y: snapshot.cursor_y,
            cells,
        },
        options,
    )
}

/// Render libghostty-vt render-state cells into a deterministic PNG preview.
///
/// This uses Ghostty's extracted row/cell/grapheme data and honors per-cell
/// foreground/background colors when libghostty-vt reports them. It still uses a
/// bundled bitmap font rather than platform text APIs, keeping the artifact
/// portable and suitable for CI/headless evidence.
pub fn render_snapshot_preview_png(
    snapshot: &GhosttyRenderSnapshot,
    options: &PreviewOptions,
) -> Result<Vec<u8>> {
    use image::{ImageBuffer, ImageEncoder, Rgba};

    let width = u32::from(snapshot.cols).max(1) * options.cell_width;
    let height = u32::from(snapshot.rows).max(1) * options.cell_height;
    let mut img = ImageBuffer::from_pixel(width, height, Rgba(options.background));
    for (row, cells) in snapshot
        .cells
        .iter()
        .take(snapshot.rows as usize)
        .enumerate()
    {
        for (col, cell) in cells.iter().take(snapshot.cols as usize).enumerate() {
            if let Some(bg) = cell.bg {
                fill_cell(&mut img, col as u32, row as u32, rgba3(bg), options);
            }
            let mut cell_options = options.clone();
            if let Some(fg) = cell.fg {
                cell_options.foreground = rgba3(fg);
            }
            for ch in cell.text.chars().take(1) {
                draw_ascii_cell(&mut img, col as u32, row as u32, ch, &cell_options);
                if cell.bold {
                    let mut bold_options = cell_options.clone();
                    bold_options.foreground = brighten_rgba(bold_options.foreground);
                    draw_ascii_cell(
                        &mut img,
                        col as u32,
                        row as u32,
                        ch,
                        &bold_options.with_x_offset(1),
                    );
                }
                if cell.italic {
                    draw_italic_hint(&mut img, col as u32, row as u32, &cell_options);
                }
                if cell.underline != 0 {
                    draw_underline(&mut img, col as u32, row as u32, &cell_options);
                }
            }
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

fn rgba3(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}

fn fill_cell(
    img: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    col: u32,
    row: u32,
    color: [u8; 4],
    options: &PreviewOptions,
) {
    let x0 = col * options.cell_width;
    let y0 = row * options.cell_height;
    for y in y0..(y0 + options.cell_height).min(img.height()) {
        for x in x0..(x0 + options.cell_width).min(img.width()) {
            img.put_pixel(x, y, image::Rgba(color));
        }
    }
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
    let x0 = col * options.cell_width + options.x_offset;
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

fn brighten_rgba(mut color: [u8; 4]) -> [u8; 4] {
    color[0] = color[0].saturating_add(32);
    color[1] = color[1].saturating_add(32);
    color[2] = color[2].saturating_add(32);
    color
}

fn draw_underline(
    img: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    col: u32,
    row: u32,
    options: &PreviewOptions,
) {
    let x0 = col * options.cell_width;
    let y = ((row + 1) * options.cell_height).saturating_sub(2);
    for x in x0..(x0 + options.cell_width).min(img.width()) {
        if y < img.height() {
            img.put_pixel(x, y, image::Rgba(options.foreground));
        }
    }
}

fn draw_italic_hint(
    img: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    col: u32,
    row: u32,
    options: &PreviewOptions,
) {
    let x0 = col * options.cell_width;
    let y0 = row * options.cell_height;
    for offset in 0..options.cell_height.min(options.cell_width) {
        let x = x0 + offset;
        let y = y0 + offset;
        if x < img.width() && y < img.height() {
            img.put_pixel(x, y, image::Rgba(options.cursor));
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
