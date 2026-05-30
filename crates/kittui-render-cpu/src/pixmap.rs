//! 8-bit RGBA pixmap used as the rasterizer's render target.

use kittui_core::color::Rgba;
use kittui_core::node::BlendMode;

/// Tight RGBA8 pixmap. Storage is row-major, top-down, with no padding.
#[derive(Clone)]
pub struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Pixmap {
    /// Construct a transparent pixmap.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0u8; (width as usize) * (height as usize) * 4],
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Borrow the raw RGBA8 buffer.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Borrow the raw RGBA8 buffer mutably.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Clear the pixmap to transparent.
    pub fn clear(&mut self) {
        self.data.iter_mut().for_each(|b| *b = 0);
    }

    /// Read the pixel at `(x, y)`. Returns transparent for out-of-bounds.
    pub fn get(&self, x: u32, y: u32) -> Rgba {
        if x >= self.width || y >= self.height {
            return Rgba::default();
        }
        let i = ((y * self.width + x) * 4) as usize;
        Rgba(
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        )
    }

    /// Blend `src` over the pixel at `(x, y)` using straight (non-premul) alpha.
    /// Out-of-bounds writes are silently ignored.
    pub fn blend(&mut self, x: u32, y: u32, src: Rgba) {
        self.blend_with(x, y, src, BlendMode::Normal);
    }

    /// Blend `src` over the pixel at `(x, y)` using the requested mode.
    pub fn blend_with(&mut self, x: u32, y: u32, src: Rgba, mode: BlendMode) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        let dst = [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ];
        let sa = src.3 as f32 / 255.0;
        let da = dst[3] as f32 / 255.0;
        let blended_rgb = match mode {
            BlendMode::Normal => None,
            BlendMode::Add => Some([
                (src.0 as f32 + dst[0] as f32).min(255.0) as u8,
                (src.1 as f32 + dst[1] as f32).min(255.0) as u8,
                (src.2 as f32 + dst[2] as f32).min(255.0) as u8,
            ]),
            BlendMode::Multiply => Some([
                ((src.0 as f32 * dst[0] as f32) / 255.0) as u8,
                ((src.1 as f32 * dst[1] as f32) / 255.0) as u8,
                ((src.2 as f32 * dst[2] as f32) / 255.0) as u8,
            ]),
            BlendMode::Screen => Some([
                (255.0 - (255.0 - src.0 as f32) * (255.0 - dst[0] as f32) / 255.0) as u8,
                (255.0 - (255.0 - src.1 as f32) * (255.0 - dst[1] as f32) / 255.0) as u8,
                (255.0 - (255.0 - src.2 as f32) * (255.0 - dst[2] as f32) / 255.0) as u8,
            ]),
        };
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 0.0 {
            self.data[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        if let Some(rgb) = blended_rgb {
            // For non-Normal modes, treat the blended RGB as the source and
            // alpha-composite it over the destination with the source alpha.
            let mix = |s: u8, d: u8| -> u8 {
                let s = s as f32 / 255.0;
                let d = d as f32 / 255.0;
                let v = (s * sa + d * da * (1.0 - sa)) / out_a;
                (v.clamp(0.0, 1.0) * 255.0).round() as u8
            };
            self.data[i] = mix(rgb[0], dst[0]);
            self.data[i + 1] = mix(rgb[1], dst[1]);
            self.data[i + 2] = mix(rgb[2], dst[2]);
        } else {
            let mix = |s: u8, d: u8| -> u8 {
                let s = s as f32 / 255.0;
                let d = d as f32 / 255.0;
                let v = (s * sa + d * da * (1.0 - sa)) / out_a;
                (v.clamp(0.0, 1.0) * 255.0).round() as u8
            };
            self.data[i] = mix(src.0, dst[0]);
            self.data[i + 1] = mix(src.1, dst[1]);
            self.data[i + 2] = mix(src.2, dst[2]);
        }
        self.data[i + 3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_modes_combine_channels() {
        let mut p = Pixmap::new(1, 1);
        // Start with mid grey, opaque.
        p.blend(0, 0, Rgba(128, 128, 128, 255));
        // Multiply with red.
        p.blend_with(0, 0, Rgba(255, 0, 0, 255), BlendMode::Multiply);
        let c = p.get(0, 0);
        assert!(c.0 > 100 && c.1 == 0 && c.2 == 0);
    }

    #[test]
    fn add_clamps_at_255() {
        let mut p = Pixmap::new(1, 1);
        p.blend(0, 0, Rgba(200, 0, 0, 255));
        p.blend_with(0, 0, Rgba(200, 200, 0, 255), BlendMode::Add);
        let c = p.get(0, 0);
        assert_eq!(c.0, 255);
        assert!(c.1 > 100);
    }

    #[test]
    fn screen_brightens_toward_white() {
        let mut p = Pixmap::new(1, 1);
        p.blend(0, 0, Rgba(100, 100, 100, 255));
        p.blend_with(0, 0, Rgba(100, 100, 100, 255), BlendMode::Screen);
        let c = p.get(0, 0);
        // 100 screen 100 = 1 - (1 - 100/255)^2 * 255 ~ 161
        assert!(c.0 >= 150 && c.0 <= 170);
    }
}
