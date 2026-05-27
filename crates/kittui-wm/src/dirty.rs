//! Dirty-grid helpers for future kittwm frame transport policy.
//!
//! The helper is deliberately terminal-agnostic: it detects changed RGBA tiles
//! but does not emit kitty escape codes. Runtime code can use it to skip
//! unchanged frames or to drive experimental transports behind explicit flags.

/// Pixel rectangle for a dirty tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirtyTile {
    /// Tile column in the dirty grid.
    pub col: u32,
    /// Tile row in the dirty grid.
    pub row: u32,
    /// Pixel x origin.
    pub x: u32,
    /// Pixel y origin.
    pub y: u32,
    /// Pixel width, clipped at the frame edge.
    pub width: u32,
    /// Pixel height, clipped at the frame edge.
    pub height: u32,
}

/// Result of diffing one RGBA frame against the previous frame seen by a grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirtyFrameDiff {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Whether this was the first valid frame for the grid or dimensions changed.
    pub first_frame: bool,
    /// Total tiles in the grid for this frame.
    pub tiles: u32,
    /// Changed tile rectangles.
    pub changed_tiles: Vec<DirtyTile>,
}

impl DirtyFrameDiff {
    /// Number of changed tiles.
    pub fn changed_count(&self) -> u32 {
        self.changed_tiles.len() as u32
    }

    /// Fraction of tiles that changed in `[0, 1]`.
    pub fn changed_fraction(&self) -> f32 {
        if self.tiles == 0 {
            0.0
        } else {
            self.changed_count() as f32 / self.tiles as f32
        }
    }

    /// Whether no pixel tile changed.
    pub fn is_clean(&self) -> bool {
        self.changed_tiles.is_empty()
    }
}

/// Stateful dirty-grid diff for tightly packed RGBA8 frames.
#[derive(Clone, Debug)]
pub struct DirtyGrid {
    tile_width: u32,
    tile_height: u32,
    previous_width: u32,
    previous_height: u32,
    previous_hashes: Vec<u64>,
}

impl DirtyGrid {
    /// Create a grid. Tile dimensions are clamped to at least one pixel.
    pub fn new(tile_width: u32, tile_height: u32) -> Self {
        Self {
            tile_width: tile_width.max(1),
            tile_height: tile_height.max(1),
            previous_width: 0,
            previous_height: 0,
            previous_hashes: Vec::new(),
        }
    }

    /// Tile width in pixels.
    pub fn tile_width(&self) -> u32 {
        self.tile_width
    }

    /// Tile height in pixels.
    pub fn tile_height(&self) -> u32 {
        self.tile_height
    }

    /// Clear remembered frame state.
    pub fn reset(&mut self) {
        self.previous_width = 0;
        self.previous_height = 0;
        self.previous_hashes.clear();
    }

    /// Diff a tight RGBA8 frame and remember it as the new previous frame.
    ///
    /// Returns `None` when `rgba.len() != width * height * 4`.
    pub fn diff_rgba(&mut self, width: u32, height: u32, rgba: &[u8]) -> Option<DirtyFrameDiff> {
        if rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        let cols = width.div_ceil(self.tile_width);
        let rows = height.div_ceil(self.tile_height);
        let tile_count = cols.saturating_mul(rows) as usize;
        let first_frame = self.previous_width != width
            || self.previous_height != height
            || self.previous_hashes.len() != tile_count;
        if first_frame {
            self.previous_hashes.clear();
        }
        let mut changed_tiles = Vec::new();
        self.previous_hashes
            .reserve(tile_count.saturating_sub(self.previous_hashes.len()));
        for row in 0..rows {
            for col in 0..cols {
                let idx = (row * cols + col) as usize;
                let rect = tile_rect(width, height, self.tile_width, self.tile_height, col, row);
                let hash = hash_tile(width, rgba, rect);
                let changed = first_frame || self.previous_hashes.get(idx) != Some(&hash);
                if changed {
                    changed_tiles.push(rect);
                }
                if idx < self.previous_hashes.len() {
                    self.previous_hashes[idx] = hash;
                } else {
                    self.previous_hashes.push(hash);
                }
            }
        }
        self.previous_width = width;
        self.previous_height = height;
        Some(DirtyFrameDiff {
            width,
            height,
            first_frame,
            tiles: cols.saturating_mul(rows),
            changed_tiles,
        })
    }
}

fn tile_rect(
    frame_width: u32,
    frame_height: u32,
    tile_width: u32,
    tile_height: u32,
    col: u32,
    row: u32,
) -> DirtyTile {
    let x = col.saturating_mul(tile_width);
    let y = row.saturating_mul(tile_height);
    DirtyTile {
        col,
        row,
        x,
        y,
        width: tile_width.min(frame_width.saturating_sub(x)),
        height: tile_height.min(frame_height.saturating_sub(y)),
    }
}

fn hash_tile(frame_width: u32, rgba: &[u8], tile: DirtyTile) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for y in tile.y..tile.y + tile.height {
        let start = ((y * frame_width + tile.x) * 4) as usize;
        let end = start + tile.width as usize * 4;
        for byte in &rgba[start..end] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_marks_all_tiles_dirty() {
        let mut grid = DirtyGrid::new(2, 2);
        let rgba = vec![0u8; 4 * 4 * 4];
        let diff = grid.diff_rgba(4, 4, &rgba).unwrap();
        assert!(diff.first_frame);
        assert_eq!(diff.tiles, 4);
        assert_eq!(diff.changed_count(), 4);
        assert_eq!(diff.changed_fraction(), 1.0);
    }

    #[test]
    fn identical_frame_is_clean_after_first_diff() {
        let mut grid = DirtyGrid::new(2, 2);
        let rgba = vec![0u8; 4 * 4 * 4];
        grid.diff_rgba(4, 4, &rgba).unwrap();
        let diff = grid.diff_rgba(4, 4, &rgba).unwrap();
        assert!(!diff.first_frame);
        assert!(diff.is_clean());
        assert_eq!(diff.changed_fraction(), 0.0);
    }

    #[test]
    fn repeated_same_size_frames_reuse_hash_buffer_capacity() {
        let mut grid = DirtyGrid::new(2, 2);
        let rgba = vec![0u8; 4 * 4 * 4];
        grid.diff_rgba(4, 4, &rgba).unwrap();
        let capacity = grid.previous_hashes.capacity();
        grid.diff_rgba(4, 4, &rgba).unwrap();
        assert_eq!(grid.previous_hashes.len(), 4);
        assert_eq!(grid.previous_hashes.capacity(), capacity);
    }

    #[test]
    fn single_pixel_change_marks_own_tile_dirty() {
        let mut grid = DirtyGrid::new(2, 2);
        let mut rgba = vec![0u8; 4 * 4 * 4];
        grid.diff_rgba(4, 4, &rgba).unwrap();
        let pixel = ((3 * 4 + 3) * 4) as usize;
        rgba[pixel] = 255;
        let diff = grid.diff_rgba(4, 4, &rgba).unwrap();
        assert_eq!(
            diff.changed_tiles,
            vec![DirtyTile {
                col: 1,
                row: 1,
                x: 2,
                y: 2,
                width: 2,
                height: 2
            }]
        );
    }

    #[test]
    fn edge_tiles_are_clipped() {
        let mut grid = DirtyGrid::new(4, 4);
        let rgba = vec![0u8; 5 * 5 * 4];
        let diff = grid.diff_rgba(5, 5, &rgba).unwrap();
        assert_eq!(diff.tiles, 4);
        assert!(diff.changed_tiles.contains(&DirtyTile {
            col: 1,
            row: 1,
            x: 4,
            y: 4,
            width: 1,
            height: 1
        }));
    }

    #[test]
    fn invalid_rgba_length_is_rejected() {
        let mut grid = DirtyGrid::new(2, 2);
        assert!(grid.diff_rgba(2, 2, &[0; 3]).is_none());
    }
}
