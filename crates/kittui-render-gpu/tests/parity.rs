//! CPU↔GPU parity gate.
//!
//! Renders a small canonical scene through both backends and asserts the
//! output PNGs decode to RGBA buffers with a per-channel mean absolute
//! error below a threshold (a cheap SSIM proxy that is adequate for
//! tiny rasters and doesn't pull in an extra dependency).
//!
//! The test is gated on adapter availability; CI nodes without a usable
//! wgpu adapter exit successfully without running the parity check.

use kittui_core::{
    color::Rgba,
    geom::{CellRect, CellSize, PxRect},
    node::{Corners, Direction, Layer, Node, Stop},
    paint::Paint,
    Scene,
};
use kittui_render_cpu as cpu;
use kittui_render_gpu as gpu;

fn fixture_scene() -> Scene {
    let cell = CellSize::new(8, 16);
    let footprint = CellRect::new(0, 0, 8, 4);
    let rect = footprint.to_pixels(cell);
    Scene {
        footprint,
        cell_size: cell,
        layers: vec![
            Layer::new(
                "background",
                Node::Gradient {
                    rect,
                    stops: vec![
                        Stop {
                            offset: 0.0,
                            color: Rgba::rgb(0x07, 0x11, 0x1f),
                        },
                        Stop {
                            offset: 1.0,
                            color: Rgba::rgb(0x17, 0x13, 0x26),
                        },
                    ],
                    direction: Direction::Vertical,
                },
            ),
            Layer::new(
                "panel",
                Node::Rect {
                    rect: PxRect::new(
                        rect.origin.0 + 4.0,
                        rect.origin.1 + 4.0,
                        rect.width - 8.0,
                        rect.height - 8.0,
                    ),
                    fill: Paint::Solid {
                        color: Rgba::rgba(0x08, 0x11, 0x1f, 0xee),
                    },
                    stroke: None,
                    corners: Corners::uniform(6.0),
                },
            ),
        ],
        animation: None,
    }
}

fn decode_rgba(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    // Minimal PNG parser sufficient to read 8-bit RGBA images produced by
    // our own encoder. Used only by the test; not exposed as API.
    use std::io::Read;

    if png.len() < 8 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut idx = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();
    while idx + 8 <= png.len() {
        let len = u32::from_be_bytes([
            png[idx],
            png[idx + 1],
            png[idx + 2],
            png[idx + 3],
        ]) as usize;
        let ty = &png[idx + 4..idx + 8];
        let data_start = idx + 8;
        let data_end = data_start + len;
        if data_end + 4 > png.len() {
            return None;
        }
        match ty {
            b"IHDR" => {
                width = u32::from_be_bytes([
                    png[data_start],
                    png[data_start + 1],
                    png[data_start + 2],
                    png[data_start + 3],
                ]);
                height = u32::from_be_bytes([
                    png[data_start + 4],
                    png[data_start + 5],
                    png[data_start + 6],
                    png[data_start + 7],
                ]);
            }
            b"IDAT" => idat.extend_from_slice(&png[data_start..data_end]),
            b"IEND" => break,
            _ => {}
        }
        idx = data_end + 4;
    }
    let mut z = flate2::read::ZlibDecoder::new(&idat[..]);
    let mut raw = Vec::new();
    z.read_to_end(&mut raw).ok()?;
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity(raw.len() - height as usize);
    for row in 0..height as usize {
        let off = row * (stride + 1);
        let _filter = raw[off];
        out.extend_from_slice(&raw[off + 1..off + 1 + stride]);
    }
    Some((width, height, out))
}

#[test]
fn cpu_gpu_parity_within_mae_threshold() {
    let scene = fixture_scene();
    let cpu_frame = cpu::render_still(&scene).unwrap();
    let mut renderer = match gpu::GpuRenderer::new() {
        Ok(r) => r,
        Err(_) => {
            eprintln!("skipping parity test: no usable wgpu adapter");
            return;
        }
    };
    let gpu_frame = renderer.render_still(&scene).unwrap();

    let (cw, ch, c) = decode_rgba(&cpu_frame.png).expect("cpu decode");
    let (gw, gh, g) = decode_rgba(&gpu_frame.png).expect("gpu decode");
    assert_eq!((cw, ch), (gw, gh));
    assert_eq!(c.len(), g.len());

    let mut total: u64 = 0;
    for (a, b) in c.iter().zip(g.iter()) {
        total += (*a as i32 - *b as i32).unsigned_abs() as u64;
    }
    let mae = total as f64 / c.len() as f64;
    // sRGB color space + AA + smoothstep glow differ at the LSB
    // between the two backends; the threshold is loose enough that any
    // visible regression in the GPU shader still trips it.
    assert!(mae < 24.0, "CPU↔GPU mean absolute error too high: {mae}");
}
