//! Markdown table helpers using kittui component metadata and kitty placement anchors.

use kittui::CellRect;
use kittui_kitty::{PlacementOptions, Quiet, RelativePlacement, SubcellOffset};

/// A single box-drawing glyph represented as a kittui image cell.
#[derive(Clone, Debug)]
pub struct BoxGlyphCell {
    /// Box drawing character.
    pub glyph: char,
    /// Column in table cell grid.
    pub col: u16,
    /// Row in table cell grid.
    pub row: u16,
    /// Stable image id to use for this glyph cell.
    pub image_id: u32,
    /// Placement options anchoring this cell to the table anchor.
    pub placement: PlacementOptions,
}

/// A table layout anchor plus glyph cells.
#[derive(Clone, Debug)]
pub struct TableGlyphLayout {
    /// Virtual/non-rendered anchor image id.
    pub anchor_image_id: u32,
    /// Optional anchor placement id.
    pub anchor_placement_id: Option<u32>,
    /// Table footprint.
    pub footprint: CellRect,
    /// Box glyph cells.
    pub cells: Vec<BoxGlyphCell>,
    /// Optional background image id that should be placed below the glyphs.
    pub background_image_id: Option<u32>,
}

impl TableGlyphLayout {
    /// Build a simple connected table border grid.
    pub fn from_dimensions(anchor_image_id: u32, cols: u16, rows: u16) -> Self {
        let footprint = CellRect::new(0, 0, cols.max(2), rows.max(2));
        let mut cells = Vec::new();
        let mut next_id = anchor_image_id.saturating_add(1);
        for row in 0..footprint.rows {
            for col in 0..footprint.cols {
                let glyph = match (row, col) {
                    (0, 0) => '┌',
                    (0, c) if c + 1 == footprint.cols => '┐',
                    (r, 0) if r + 1 == footprint.rows => '└',
                    (r, c) if r + 1 == footprint.rows && c + 1 == footprint.cols => '┘',
                    (0, _) => '─',
                    (r, _) if r + 1 == footprint.rows => '─',
                    (_, 0) => '│',
                    (_, c) if c + 1 == footprint.cols => '│',
                    _ => ' ',
                };
                if glyph == ' ' {
                    continue;
                }
                cells.push(BoxGlyphCell {
                    glyph,
                    col,
                    row,
                    image_id: next_id,
                    placement: relative_cell_options(anchor_image_id, None, col, row, 0),
                });
                next_id += 1;
            }
        }
        Self { anchor_image_id, anchor_placement_id: None, footprint, cells, background_image_id: None }
    }

    /// Set a background image id intended to render below table glyph cells.
    pub fn with_background(mut self, image_id: u32) -> Self {
        self.background_image_id = Some(image_id);
        self
    }
}

/// Build placement options for a cell image anchored relative to another image.
pub fn relative_cell_options(
    anchor_image_id: u32,
    anchor_placement_id: Option<u32>,
    col: u16,
    row: u16,
    z_index: i32,
) -> PlacementOptions {
    PlacementOptions {
        placement_id: None,
        offset: SubcellOffset::default(),
        quiet: Quiet::SuppressAll,
        unicode_placeholder: true,
        z_index,
        relative: Some(RelativePlacement {
            image_id: anchor_image_id,
            placement_id: anchor_placement_id,
            x_offset_px: i32::from(col),
            y_offset_px: i32::from(row),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_layout_builds_relative_glyph_cells() {
        let table = TableGlyphLayout::from_dimensions(100, 4, 3).with_background(99);
        assert_eq!(table.footprint.cols, 4);
        assert_eq!(table.background_image_id, Some(99));
        assert!(table.cells.iter().any(|c| c.glyph == '┌'));
        let first = &table.cells[0];
        assert_eq!(first.placement.relative.unwrap().image_id, 100);
    }
}
