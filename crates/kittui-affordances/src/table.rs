//! Markdown table helpers using kittui component metadata and kitty placement anchors.

use kittui::scene::scene;
use kittui::{
    Animation, CellRect, CellSize, Corners, Layer, Node, Paint, PxRect, Rgba, Scene,
    STANDARD_ANIMATION_FPS, STANDARD_ANIMATION_FRAMES,
};
use kittui_kitty::{PlacementOptions, Quiet, RelativePlacement, SubcellOffset};

/// Markdown table column alignment.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MarkdownTableAlignment {
    /// No explicit alignment marker.
    None,
    /// Left-aligned column (`:---`).
    Left,
    /// Center-aligned column (`:---:`).
    Center,
    /// Right-aligned column (`---:`).
    Right,
}

impl MarkdownTableAlignment {
    /// Stable lowercase string for metadata output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// Parsed markdown table data in document order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTable {
    /// Rows of cell text. The first row is the header when the source table has one.
    pub rows: Vec<Vec<String>>,
    /// Per-column alignment metadata in source order.
    pub alignments: Vec<MarkdownTableAlignment>,
}

impl MarkdownTable {
    /// Construct from rows.
    pub fn new(rows: Vec<Vec<String>>) -> Self {
        Self {
            rows,
            alignments: Vec::new(),
        }
    }

    /// Construct from rows and per-column alignment metadata.
    pub fn with_alignments(
        rows: Vec<Vec<String>>,
        alignments: Vec<MarkdownTableAlignment>,
    ) -> Self {
        Self { rows, alignments }
    }

    /// Per-column display widths, including a minimum width of one cell.
    pub fn column_widths(&self) -> Vec<u16> {
        let cols = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![1u16; cols];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count() as u16);
            }
        }
        widths
    }

    /// Text-grid footprint for a box-drawn table.
    pub fn footprint(&self) -> CellRect {
        let widths = self.column_widths();
        let cols = if widths.is_empty() {
            2
        } else {
            widths.iter().map(|w| w.saturating_add(2)).sum::<u16>() + widths.len() as u16 + 1
        };
        let rows = (self.rows.len() as u16).saturating_mul(2).saturating_add(1);
        CellRect::new(0, 0, cols.max(2), rows.max(2))
    }
}

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
        Self::from_footprint(
            anchor_image_id,
            CellRect::new(0, 0, cols.max(2), rows.max(2)),
        )
    }

    /// Build a connected glyph grid sized to a parsed markdown table.
    pub fn from_table(anchor_image_id: u32, table: &MarkdownTable) -> Self {
        let footprint = table.footprint();
        let widths = table.column_widths();
        let mut verticals = vec![0u16, footprint.cols.saturating_sub(1)];
        let mut x = 0u16;
        for width in widths {
            x = x.saturating_add(width).saturating_add(3);
            if x < footprint.cols {
                verticals.push(x);
            }
        }
        let horizontals = (0..footprint.rows)
            .filter(|row| row % 2 == 0)
            .collect::<Vec<_>>();
        Self::from_grid_lines(anchor_image_id, footprint, &verticals, &horizontals)
    }

    fn from_footprint(anchor_image_id: u32, footprint: CellRect) -> Self {
        let verticals = vec![0, footprint.cols.saturating_sub(1)];
        let horizontals = vec![0, footprint.rows.saturating_sub(1)];
        Self::from_grid_lines(anchor_image_id, footprint, &verticals, &horizontals)
    }

    fn from_grid_lines(
        anchor_image_id: u32,
        footprint: CellRect,
        verticals: &[u16],
        horizontals: &[u16],
    ) -> Self {
        let mut cells = Vec::new();
        let mut next_id = anchor_image_id.saturating_add(1);
        for row in 0..footprint.rows {
            for col in 0..footprint.cols {
                let has_h = horizontals.contains(&row);
                let has_v = verticals.contains(&col);
                let glyph = match (
                    has_h,
                    has_v,
                    row == 0,
                    row + 1 == footprint.rows,
                    col == 0,
                    col + 1 == footprint.cols,
                ) {
                    (true, true, true, _, true, _) => '┌',
                    (true, true, true, _, _, true) => '┐',
                    (true, true, _, true, true, _) => '└',
                    (true, true, _, true, _, true) => '┘',
                    (true, true, true, _, _, _) => '┬',
                    (true, true, _, true, _, _) => '┴',
                    (true, true, _, _, true, _) => '├',
                    (true, true, _, _, _, true) => '┤',
                    (true, true, _, _, _, _) => '┼',
                    (true, false, _, _, _, _) => '─',
                    (false, true, _, _, _, _) => '│',
                    _ => continue,
                };
                cells.push(BoxGlyphCell {
                    glyph,
                    col,
                    row,
                    image_id: next_id,
                    placement: relative_cell_options(anchor_image_id, None, col, row, 1),
                });
                next_id += 1;
            }
        }
        Self {
            anchor_image_id,
            anchor_placement_id: None,
            footprint,
            cells,
            background_image_id: None,
        }
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
    let cell = CellSize::default();
    PlacementOptions {
        placement_id: None,
        offset: SubcellOffset::default(),
        quiet: Quiet::SuppressAll,
        unicode_placeholder: false,
        z_index,
        relative: Some(RelativePlacement {
            image_id: anchor_image_id,
            placement_id: anchor_placement_id,
            x_offset_px: i32::from(col) * i32::from(cell.width_px),
            y_offset_px: i32::from(row) * i32::from(cell.height_px),
        }),
    }
}

/// Kitty-native animation options for one-cell box glyph scenes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BoxGlyphAnimation {
    /// Frames per second.
    pub fps: u16,
    /// Frames in one seamless loop.
    pub frames: u16,
}

impl Default for BoxGlyphAnimation {
    fn default() -> Self {
        Self {
            fps: STANDARD_ANIMATION_FPS,
            frames: STANDARD_ANIMATION_FRAMES,
        }
    }
}

impl BoxGlyphAnimation {
    /// Convert to the kittui core animation descriptor.
    pub fn to_animation(self) -> Animation {
        Animation::pulse_fps(self.frames, self.fps)
    }
}

/// Render one box-drawing glyph as a one-cell kittui scene.
pub fn box_glyph_scene(glyph: char, fg: Rgba, cell: CellSize) -> Scene {
    box_glyph_scene_with_animation(glyph, fg, cell, None)
}

/// Render one box-drawing glyph as a one-cell kittui scene with optional native animation.
pub fn box_glyph_scene_with_animation(
    glyph: char,
    fg: Rgba,
    cell: CellSize,
    animation: Option<BoxGlyphAnimation>,
) -> Scene {
    let footprint = CellRect::new(0, 0, 1, 1);
    let mut layers = Vec::new();
    let w = f32::from(cell.width_px);
    let h = f32::from(cell.height_px);
    let t = 2.0_f32.max((w.min(h) / 8.0).round());
    let mid_x = (w - t) / 2.0;
    let mid_y = (h - t) / 2.0;
    let paint = Paint::Solid { color: fg };
    let segments = glyph_segments(glyph);
    if segments.top {
        layers.push(rect_layer(
            "top",
            PxRect::new(mid_x, 0.0, t, h / 2.0 + t / 2.0),
            paint.clone(),
        ));
    }
    if segments.bottom {
        layers.push(rect_layer(
            "bottom",
            PxRect::new(mid_x, h / 2.0 - t / 2.0, t, h / 2.0 + t / 2.0),
            paint.clone(),
        ));
    }
    if segments.left {
        layers.push(rect_layer(
            "left",
            PxRect::new(0.0, mid_y, w / 2.0 + t / 2.0, t),
            paint.clone(),
        ));
    }
    if segments.right {
        layers.push(rect_layer(
            "right",
            PxRect::new(w / 2.0 - t / 2.0, mid_y, w / 2.0 + t / 2.0, t),
            paint,
        ));
    }
    if let Some(animation) = animation {
        layers.push(Layer::new(
            "box_glyph_animation",
            Node::Glow {
                rect: footprint.to_pixels(cell),
                center_x_frac: 0.5,
                center_y_frac: 0.5,
                radius_frac: 1.4,
                color: fg,
                intensity: 0.5,
            },
        ));
        let mut scene = scene(footprint, cell, layers);
        scene.animation = Some(animation.to_animation());
        scene
    } else {
        scene(footprint, cell, layers)
    }
}

#[derive(Copy, Clone)]
struct Segments {
    top: bool,
    right: bool,
    bottom: bool,
    left: bool,
}

fn glyph_segments(glyph: char) -> Segments {
    match glyph {
        '─' => Segments {
            top: false,
            right: true,
            bottom: false,
            left: true,
        },
        '│' => Segments {
            top: true,
            right: false,
            bottom: true,
            left: false,
        },
        '┌' => Segments {
            top: false,
            right: true,
            bottom: true,
            left: false,
        },
        '┐' => Segments {
            top: false,
            right: false,
            bottom: true,
            left: true,
        },
        '└' => Segments {
            top: true,
            right: true,
            bottom: false,
            left: false,
        },
        '┘' => Segments {
            top: true,
            right: false,
            bottom: false,
            left: true,
        },
        '┬' => Segments {
            top: false,
            right: true,
            bottom: true,
            left: true,
        },
        '┴' => Segments {
            top: true,
            right: true,
            bottom: false,
            left: true,
        },
        '├' => Segments {
            top: true,
            right: true,
            bottom: true,
            left: false,
        },
        '┤' => Segments {
            top: true,
            right: false,
            bottom: true,
            left: true,
        },
        '┼' => Segments {
            top: true,
            right: true,
            bottom: true,
            left: true,
        },
        _ => Segments {
            top: false,
            right: false,
            bottom: false,
            left: false,
        },
    }
}

fn rect_layer(label: &'static str, rect: PxRect, fill: Paint) -> Layer {
    Layer::new(
        label,
        Node::Rect {
            rect,
            fill,
            stroke: None,
            corners: Corners::default(),
        },
    )
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
        let relative = first.placement.relative.unwrap();
        assert_eq!(relative.image_id, 100);
        assert_eq!(relative.x_offset_px, 0);
        assert_eq!(relative.y_offset_px, 0);
    }

    #[test]
    fn table_layout_from_rows_adds_intersections() {
        let table = MarkdownTable::new(vec![
            vec!["A".into(), "B".into()],
            vec!["1".into(), "2".into()],
        ]);
        let layout = TableGlyphLayout::from_table(200, &table);
        assert!(layout.cells.iter().any(|c| c.glyph == '┬'));
        assert!(layout.cells.iter().any(|c| c.glyph == '┼'));
        assert!(layout.footprint.cols >= 9);
    }

    #[test]
    fn animated_glyph_scene_uses_default_loop_contract() {
        let scene = box_glyph_scene_with_animation(
            '┼',
            Rgba::rgba(255, 255, 255, 255),
            CellSize::default(),
            Some(BoxGlyphAnimation::default()),
        );
        let animation = scene.animation.as_ref().unwrap();
        assert_eq!(animation.frames, 180);
        assert_eq!(animation.cycle_ms, 3000);
        assert!(animation.curve.closes_loop());
        assert!(scene
            .layers
            .iter()
            .any(|layer| layer.label.as_deref() == Some("box_glyph_animation")));
    }

    #[test]
    fn glyph_scene_has_line_layers() {
        let scene = box_glyph_scene('┼', Rgba::rgba(255, 255, 255, 255), CellSize::default());
        assert_eq!(scene.footprint, CellRect::new(0, 0, 1, 1));
        assert_eq!(scene.layers.len(), 4);
    }
}
