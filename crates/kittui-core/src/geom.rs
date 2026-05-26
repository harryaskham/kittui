//! Cell-grid and pixel-space geometry primitives.

use serde::{Deserialize, Serialize};

/// A pixel-space point. Origin is the top-left of the scene; `+x` is right,
/// `+y` is down. Float so subpixel rasterization is well-defined.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Px(pub f32, pub f32);

/// An axis-aligned pixel-space rectangle.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PxRect {
    /// Top-left corner.
    pub origin: Px,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
}

impl PxRect {
    /// Construct from `x, y, w, h` quadruple.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            origin: Px(x, y),
            width: w,
            height: h,
        }
    }

    /// `x`-coordinate of the right edge.
    pub fn right(&self) -> f32 {
        self.origin.0 + self.width
    }

    /// `y`-coordinate of the bottom edge.
    pub fn bottom(&self) -> f32 {
        self.origin.1 + self.height
    }

    /// Whether the rectangle contains the given pixel point. Right and bottom
    /// edges are exclusive so adjacent rectangles tile without overlap.
    pub fn contains(&self, point: Px) -> bool {
        point.0 >= self.origin.0
            && point.0 < self.right()
            && point.1 >= self.origin.1
            && point.1 < self.bottom()
    }
}

/// A cell-grid rectangle. Coordinates are integer cell positions.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRect {
    /// Column of the top-left cell.
    pub x: u16,
    /// Row of the top-left cell.
    pub y: u16,
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
}

impl CellRect {
    /// Construct from `x, y, cols, rows`.
    pub const fn new(x: u16, y: u16, cols: u16, rows: u16) -> Self {
        Self { x, y, cols, rows }
    }

    /// Convert to pixel-space using the supplied [`CellSize`].
    pub fn to_pixels(self, cell: CellSize) -> PxRect {
        PxRect::new(
            self.x as f32 * cell.width_px as f32,
            self.y as f32 * cell.height_px as f32,
            self.cols as f32 * cell.width_px as f32,
            self.rows as f32 * cell.height_px as f32,
        )
    }
}

/// Default virtual cell width used for kittui/kittwm surfaces that do not
/// have a host-probed pixel metric yet.
pub const DEFAULT_CELL_WIDTH_PX: u16 = 8;

/// Default virtual cell height used for kittui/kittwm surfaces that do not
/// have a host-probed pixel metric yet.
pub const DEFAULT_CELL_HEIGHT_PX: u16 = 16;

/// Pixel dimensions of a single terminal cell. Defaults reflect kittwm's
/// shared virtual surface metric (8×16) so PTY, libghostty, browser, X11, and
/// Quartz adapters have a consistent scale baseline until the host provides a
/// measured cell size.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellSize {
    /// Width of a cell in pixels.
    pub width_px: u16,
    /// Height of a cell in pixels.
    pub height_px: u16,
}

impl CellSize {
    /// Construct an explicit cell size.
    pub const fn new(width_px: u16, height_px: u16) -> Self {
        Self {
            width_px,
            height_px,
        }
    }

    /// Total pixels in a cell.
    pub fn area_px(self) -> u32 {
        self.width_px as u32 * self.height_px as u32
    }
}

impl Default for CellSize {
    fn default() -> Self {
        Self::new(DEFAULT_CELL_WIDTH_PX, DEFAULT_CELL_HEIGHT_PX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_rect_to_pixels_uses_cell_size() {
        let rect = CellRect::new(3, 5, 4, 2);
        let pixels = rect.to_pixels(CellSize::new(8, 16));
        assert_eq!(pixels, PxRect::new(24.0, 80.0, 32.0, 32.0));
    }

    #[test]
    fn px_rect_contains_is_half_open() {
        let rect = PxRect::new(0.0, 0.0, 2.0, 2.0);
        assert!(rect.contains(Px(0.0, 0.0)));
        assert!(rect.contains(Px(1.9, 1.9)));
        assert!(!rect.contains(Px(2.0, 0.0)));
        assert!(!rect.contains(Px(0.0, 2.0)));
    }
}
