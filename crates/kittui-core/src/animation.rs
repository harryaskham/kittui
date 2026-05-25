//! Animation primitives.
//!
//! Animation is intentionally declarative: a scene declares its frame count,
//! cycle duration, and phase curve. The renderer rasterizes each frame
//! exactly once and uploads them all to the terminal; the kitty graphics
//! protocol's native animation control drives playback. Re-uploading per
//! frame is never required and is treated as a bug.

use serde::{Deserialize, Serialize};

/// Standard native animation frame rate used by kittui visual affordances.
pub const STANDARD_ANIMATION_FPS: u16 = 60;
/// Standard frame count for one seamless native animation loop.
pub const STANDARD_ANIMATION_FRAMES: u16 = 180;
/// Standard cycle length in milliseconds (`180 / 60 = 3s`).
pub const STANDARD_ANIMATION_CYCLE_MS: u32 = 3000;

/// Phase curve applied across an animation cycle. Phase values are `[0,1]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PhaseCurve {
    /// Linear phase: `phase = t`.
    Linear,
    /// Smooth ease-in-out: cubic Hermite.
    EaseInOut,
    /// Sinusoidal pulse with optional higher harmonics.
    Pulse {
        /// Number of additional sinusoidal harmonics blended in. `0` is a
        /// single sine; `1` adds a half-amplitude second harmonic, etc.
        harmonics: u8,
    },
    /// Caller-supplied per-frame phases. Must be exactly the same length as
    /// `Animation::frames` and must satisfy `phases[0] == 0.0` and
    /// `phases[frames-1] == 1.0` so the loop closes.
    Custom {
        /// Explicit per-frame phase values in ascending order.
        phases: Vec<f32>,
    },
}

impl PhaseCurve {
    /// Evaluate the curve at a normalized `t in [0,1]`.
    pub fn eval(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseInOut => t * t * (3.0 - 2.0 * t),
            Self::Pulse { harmonics } => {
                let mut acc = (t * std::f32::consts::TAU).sin();
                let mut amp_sum = 1.0;
                for k in 1..=(*harmonics as u32) {
                    let amp = 1.0 / ((k + 1) as f32);
                    acc += amp * ((t * (k as f32 + 1.0) * std::f32::consts::TAU).sin());
                    amp_sum += amp;
                }
                // Normalize to [0,1].
                ((acc / amp_sum) + 1.0) * 0.5
            }
            Self::Custom { phases } => {
                if phases.is_empty() {
                    return t;
                }
                let idx = (t * (phases.len() - 1) as f32).round() as usize;
                phases[idx.min(phases.len() - 1)]
            }
        }
    }

    /// Whether the curve guarantees `eval(0) == eval(1)`. Used to validate
    /// that an animation forms a perfect loop before upload.
    pub fn closes_loop(&self) -> bool {
        match self {
            Self::Linear => false, // 0 != 1
            Self::EaseInOut => false,
            Self::Pulse { .. } => true,
            Self::Custom { phases } => phases
                .first()
                .zip(phases.last())
                .map(|(a, b)| (a - b).abs() < f32::EPSILON)
                .unwrap_or(false),
        }
    }
}

/// Animation descriptor attached to a scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    /// Number of frames in one cycle. Must be `>= 2`.
    pub frames: u16,
    /// Cycle duration in milliseconds.
    pub cycle_ms: u32,
    /// Curve evaluated to compute each frame's phase.
    pub curve: PhaseCurve,
    /// Number of full cycles to play. `0` means loop forever.
    pub loops: u32,
}

impl Animation {
    /// Construct a pulsing animation with `frames` and `cycle_ms`.
    pub fn pulse(frames: u16, cycle_ms: u32) -> Self {
        Self {
            frames: frames.max(2),
            cycle_ms,
            curve: PhaseCurve::Pulse { harmonics: 0 },
            loops: 0,
        }
    }

    /// Construct a pulsing animation from a frame count and frames-per-second.
    pub fn pulse_fps(frames: u16, fps: u16) -> Self {
        let frames = frames.max(2);
        let fps = u32::from(fps.max(1));
        Self::pulse(frames, ((u32::from(frames) * 1000) / fps).max(1))
    }

    /// Construct kittui's standard 60fps / 180-frame / 3s native loop.
    pub fn standard_loop() -> Self {
        Self::pulse(STANDARD_ANIMATION_FRAMES, STANDARD_ANIMATION_CYCLE_MS)
    }

    /// Compute the per-frame phase array used by renderers.
    pub fn phases(&self) -> Vec<f32> {
        let n = self.frames.max(2) as usize;
        (0..n)
            .map(|i| self.curve.eval(i as f32 / (n - 1) as f32))
            .collect()
    }

    /// Frame delay in milliseconds for a given frame index. Uses uniform
    /// spacing across `cycle_ms`; the kitty protocol carries this directly.
    pub fn delay_ms(&self, _frame: u16) -> u32 {
        self.cycle_ms / self.frames.max(2) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_curve_closes_the_loop() {
        let curve = PhaseCurve::Pulse { harmonics: 0 };
        let a = curve.eval(0.0);
        let b = curve.eval(1.0);
        assert!((a - b).abs() < 1e-6);
        assert!(curve.closes_loop());
    }

    #[test]
    fn custom_curve_requires_endpoints_to_match() {
        let bad = PhaseCurve::Custom {
            phases: vec![0.0, 0.5, 0.9],
        };
        assert!(!bad.closes_loop());

        let good = PhaseCurve::Custom {
            phases: vec![0.0, 0.5, 0.5, 0.0],
        };
        assert!(good.closes_loop());
    }

    #[test]
    fn standard_loop_uses_shared_affordance_contract() {
        let anim = Animation::standard_loop();
        assert_eq!(anim.frames, STANDARD_ANIMATION_FRAMES);
        assert_eq!(anim.cycle_ms, STANDARD_ANIMATION_CYCLE_MS);
        assert_eq!(anim.delay_ms(0), 1000 / u32::from(STANDARD_ANIMATION_FPS));
        assert!(anim.curve.closes_loop());
    }

    #[test]
    fn pulse_fps_clamps_inputs_and_computes_period() {
        let anim = Animation::pulse_fps(1, 0);
        assert_eq!(anim.frames, 2);
        assert_eq!(anim.cycle_ms, 2000);
    }

    #[test]
    fn phases_returns_frame_count_samples() {
        let anim = Animation::pulse(8, 800);
        let phases = anim.phases();
        assert_eq!(phases.len(), 8);
        // First and last must agree because Pulse closes its loop.
        assert!((phases[0] - phases[7]).abs() < 1e-6);
    }
}
