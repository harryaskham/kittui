//! kittui showcase — affordance gallery rendered through the library.
//!
//! This example deliberately lives in the consumer side of the workspace.
//! The library crate `kittui` exposes only general primitives (Rect, Glow,
//! Gradient, Scanlines, etc.) and this file composes them into the kinds of
//! affordances callers want to ship: tonal panels, joined-border boxes,
//! chips, dividers, headers, and a pulsing assistant card.
//!
//! Run with `cargo run -p kittui-cli --example showcase`.

use std::io::Write;

use kittui::scene::{background_linear, background_solid, glow_layer, rounded_rect};
use kittui::{
    Animation, CellRect, CellSize, Direction, Layer, PhaseCurve, RendererKind, Rgba, Runtime, Scene,
};
use kittui_core::geom::PxRect;
use kittui_core::node::{Corners, Node, StrokeAlign};
use kittui_core::paint::Paint;
use kittui_core::Stroke;

const SHOWCASE_ANIMATION_FPS: u32 = 60;
const SHOWCASE_ANIMATION_FRAMES: u16 = 180;
const SHOWCASE_ANIMATION_CYCLE_MS: u32 =
    (SHOWCASE_ANIMATION_FRAMES as u32 * 1000) / SHOWCASE_ANIMATION_FPS;

#[derive(Copy, Clone)]
enum Tone {
    Assistant,
    Tool,
    User,
}

struct Palette {
    bg_top: Rgba,
    bg_bottom: Rgba,
    rail: Rgba,
    glow: Rgba,
}

fn palette(tone: Tone) -> Palette {
    let parse = |s: &str| Rgba::parse(s).unwrap();
    match tone {
        Tone::Assistant => Palette {
            bg_top: parse("#07111fff"),
            bg_bottom: parse("#11192cff"),
            rail: parse("#00d8ff"),
            glow: parse("#00d8ffaa"),
        },
        Tone::Tool => Palette {
            bg_top: parse("#080d1bff"),
            bg_bottom: parse("#171326ff"),
            rail: parse("#b48cff"),
            glow: parse("#b48cffaa"),
        },
        Tone::User => Palette {
            bg_top: parse("#061817ff"),
            bg_bottom: parse("#0e202cff"),
            rail: parse("#72fbd6"),
            glow: parse("#72fbd6aa"),
        },
    }
}

fn panel(tone: Tone, cols: u16, rows: u16, animated: bool) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, rows);
    let rect = footprint.to_pixels(cell);
    let p = palette(tone);
    let inset = PxRect::new(
        rect.origin.0 + 2.0,
        rect.origin.1 + 2.0,
        rect.width - 4.0,
        rect.height - 4.0,
    );
    let layers = vec![
        background_linear(footprint, cell, Direction::Vertical, p.bg_top, p.bg_bottom),
        rounded_rect(inset, p.bg_top, p.rail, 1.5, 8.0),
        Layer::new(
            "scanlines",
            Node::Scanlines {
                rect: inset,
                alpha: 0x22,
                period_px: 3,
            },
        ),
        glow_layer(inset, p.glow, 0.55),
    ];
    Scene {
        footprint,
        cell_size: cell,
        layers,
        animation: animated.then(|| Animation {
            frames: SHOWCASE_ANIMATION_FRAMES,
            cycle_ms: SHOWCASE_ANIMATION_CYCLE_MS,
            curve: PhaseCurve::Pulse { harmonics: 0 },
            loops: 0,
        }),
    }
}

fn chip(label_rgba: &str, bg_rgba: &str, cols: u16) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, 1);
    let rect = footprint.to_pixels(cell);
    let fg = Rgba::parse(label_rgba).unwrap();
    let bg = Rgba::parse(bg_rgba).unwrap();
    Scene {
        footprint,
        cell_size: cell,
        layers: vec![Layer::new(
            "chip",
            Node::Rect {
                rect,
                fill: Paint::Solid { color: bg },
                stroke: Some(Stroke {
                    align: StrokeAlign::Inside,
                    width_px: 1.0,
                    paint: Paint::Solid { color: fg },
                }),
                corners: Corners::uniform(rect.height * 0.5),
            },
        )],
        animation: None,
    }
}

fn divider(cols: u16) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, 1);
    Scene {
        footprint,
        cell_size: cell,
        layers: vec![background_linear(
            footprint,
            cell,
            Direction::Horizontal,
            Rgba::parse("#00d8ff").unwrap(),
            Rgba::parse("#b48cff").unwrap(),
        )],
        animation: None,
    }
}

fn header(cols: u16) -> Scene {
    let cell = CellSize::default();
    let footprint = CellRect::new(0, 0, cols, 3);
    let rect = footprint.to_pixels(cell);
    let bg = background_solid(footprint, cell, Rgba::parse("#060d17ff").unwrap());
    let band = Layer::new(
        "header_band",
        Node::Gradient {
            rect: PxRect::new(rect.origin.0, rect.bottom() - 2.0, rect.width, 2.0),
            stops: vec![
                kittui_core::Stop {
                    offset: 0.0,
                    color: Rgba::parse("#00d8ff").unwrap(),
                },
                kittui_core::Stop {
                    offset: 1.0,
                    color: Rgba::parse("#72fbd6").unwrap(),
                },
            ],
            direction: Direction::Horizontal,
        },
    );
    Scene {
        footprint,
        cell_size: cell,
        layers: vec![bg, band],
        animation: None,
    }
}

fn main() -> anyhow::Result<()> {
    let runtime = Runtime::builder()
        .renderer(RendererKind::Cpu)
        .terminal(kittui::TerminalInfo::detect())
        .build()?;

    let scenes: Vec<(&str, Scene)> = vec![
        ("header", header(60)),
        ("divider", divider(60)),
        (
            "assistant panel (animated)",
            panel(Tone::Assistant, 60, 9, true),
        ),
        ("tool panel", panel(Tone::Tool, 60, 7, false)),
        ("user panel", panel(Tone::User, 60, 5, false)),
        ("chip", chip("#08111f", "#00d8ffcc", 12)),
    ];

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    for (label, scene) in scenes {
        writeln!(handle, "\x1b[1m{label}\x1b[0m")?;
        let placement = runtime.place(&scene)?;
        handle.write_all(placement.upload.as_bytes())?;
        handle.write_all(placement.placement.as_bytes())?;
        handle.write_all(placement.embed.as_bytes())?;
        writeln!(handle)?;
    }
    Ok(())
}
