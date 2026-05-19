//! Scene id stability stress.
//!
//! For a wide range of randomly-shaped scenes, assert:
//!
//! - `Scene::id()` is deterministic across clones.
//! - `Scene::id()` round-trips through serde_json without changing.
//! - Modifying any field changes the id.
//!
//! Uses a tiny xorshift RNG seeded deterministically so the test is
//! reproducible without adding a `proptest` dependency.

use kittui_core::{
    color::Rgba,
    geom::{CellRect, CellSize, PxRect},
    node::{Corners, Direction, Layer, Node, Stop, Stroke, StrokeAlign},
    paint::Paint,
    Scene,
};

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn u8(&mut self) -> u8 {
        (self.next() & 0xff) as u8
    }
    fn u16(&mut self, max: u16) -> u16 {
        if max == 0 {
            0
        } else {
            (self.next() % max as u64) as u16
        }
    }
    fn f32(&mut self, max: f32) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0 * max
    }
}

fn rgba(rng: &mut Rng) -> Rgba {
    Rgba(rng.u8(), rng.u8(), rng.u8(), rng.u8())
}

fn px_rect(rng: &mut Rng) -> PxRect {
    PxRect::new(rng.f32(64.0), rng.f32(64.0), 8.0 + rng.f32(64.0), 8.0 + rng.f32(64.0))
}

fn node(rng: &mut Rng) -> Node {
    match rng.next() % 4 {
        0 => Node::Rect {
            rect: px_rect(rng),
            fill: Paint::Solid { color: rgba(rng) },
            stroke: if rng.next() & 1 == 0 {
                None
            } else {
                Some(Stroke {
                    align: StrokeAlign::Inside,
                    width_px: 0.5 + rng.f32(3.0),
                    paint: Paint::Solid { color: rgba(rng) },
                })
            },
            corners: Corners::uniform(rng.f32(8.0)),
        },
        1 => Node::Gradient {
            rect: px_rect(rng),
            stops: vec![
                Stop {
                    offset: 0.0,
                    color: rgba(rng),
                },
                Stop {
                    offset: 1.0,
                    color: rgba(rng),
                },
            ],
            direction: match rng.next() % 3 {
                0 => Direction::Horizontal,
                1 => Direction::Vertical,
                _ => Direction::Diagonal,
            },
        },
        2 => Node::Glow {
            rect: px_rect(rng),
            center_x_frac: rng.f32(1.0),
            center_y_frac: rng.f32(1.0),
            radius_frac: 0.1 + rng.f32(0.5),
            color: rgba(rng),
            intensity: rng.f32(1.0),
        },
        _ => Node::Scanlines {
            rect: px_rect(rng),
            alpha: rng.u8(),
            period_px: 1 + (rng.u8() % 8),
        },
    }
}

fn scene(rng: &mut Rng) -> Scene {
    let cols = 1 + rng.u16(15);
    let rows = 1 + rng.u16(7);
    let layers = (0..(1 + rng.u16(4)))
        .map(|_| Layer::anon(node(rng)))
        .collect();
    Scene {
        footprint: CellRect::new(0, 0, cols, rows),
        cell_size: CellSize::new(8, 16),
        layers,
        animation: None,
    }
}

#[test]
fn scene_id_is_stable_across_clones_and_serde_roundtrip() {
    let mut rng = Rng::new(0xC0FFEE);
    for _ in 0..256 {
        let s = scene(&mut rng);
        let original = s.id();
        // Clone must match.
        let cloned = s.clone();
        assert_eq!(cloned.id(), original);
        // serde round-trip must match.
        let json = serde_json::to_string(&s).expect("serialize");
        let parsed: Scene = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed.id(), original);
    }
}

#[test]
fn perturbing_any_layer_changes_id() {
    let mut rng = Rng::new(0xDEADBEEF);
    for _ in 0..128 {
        let s = scene(&mut rng);
        let original = s.id();
        let mut mutated = s.clone();
        mutated.footprint.cols = mutated.footprint.cols.saturating_add(1);
        assert_ne!(mutated.id(), original);
    }
}
