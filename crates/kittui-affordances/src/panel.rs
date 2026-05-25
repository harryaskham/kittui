//! Tonal panel: the showcase example's "assistant / tool / user" panel
//! distilled to a free function returning a ratakittui `Chrome`.

use kittui::{Direction, STANDARD_ANIMATION_FPS, STANDARD_ANIMATION_FRAMES};
use ratakittui::{Background, Border, Chrome, Glow, Padding, Pulse};

use crate::palette::{Palette, Tone};

/// Kitty-native panel animation options.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PanelAnimation {
    /// Frames per second.
    pub fps: u16,
    /// Frames in one seamless loop.
    pub frames: u16,
}

impl Default for PanelAnimation {
    fn default() -> Self {
        Self {
            fps: STANDARD_ANIMATION_FPS,
            frames: STANDARD_ANIMATION_FRAMES,
        }
    }
}

impl PanelAnimation {
    fn pulse(self) -> Pulse {
        let fps = self.fps.max(1) as u32;
        let frames = self.frames.max(2);
        Pulse {
            frames,
            cycle_ms: (((frames as u32) * 1000) / fps).max(1),
        }
    }
}

/// Options for [`panel_chrome`].
#[derive(Clone, Debug, Default)]
pub struct PanelOptions {
    /// If `true`, the panel pulses via a kittui-side animation that
    /// uploads once and loops natively. Costs nothing per frame after
    /// the first upload.
    pub animated: bool,
}

/// Build a panel chrome for the given tone. The returned `Chrome` plugs
/// straight into any ratakittui widget wrapper or `JoinGroup`.
pub fn panel_chrome(tone: Tone, opts: &PanelOptions) -> Chrome {
    panel_chrome_with_animation(tone, opts.animated.then(PanelAnimation::default))
}

/// Build a panel chrome with explicit animation options.
pub fn panel_chrome_with_animation(tone: Tone, animation: Option<PanelAnimation>) -> Chrome {
    let p = Palette::for_tone(tone);
    let mut chrome = Chrome::default()
        .background(Background::Linear {
            direction: Direction::Vertical,
            start: p.bg_top,
            end: p.bg_bottom,
        })
        .border(Border::rounded(p.rail, 1.5, 8.0))
        .padding(Padding::trbl(1, 2, 1, 2));
    if let Some(animation) = animation {
        chrome = chrome.glow(Glow {
            color: p.glow,
            cx: 0.5,
            cy: 0.5,
            radius: 0.5,
            intensity: 0.55,
            pulse: Some(animation.pulse()),
        });
    }
    chrome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_animation_uses_standard_default_period() {
        let chrome = panel_chrome(Tone::Assistant, &PanelOptions { animated: true });
        let pulse = chrome.glow.as_ref().unwrap().pulse.unwrap();
        assert_eq!(pulse.frames, 180);
        assert_eq!(pulse.cycle_ms, 3000);
    }

    #[test]
    fn explicit_panel_animation_controls_period() {
        let chrome = panel_chrome_with_animation(
            Tone::Assistant,
            Some(PanelAnimation {
                fps: 30,
                frames: 90,
            }),
        );
        let pulse = chrome.glow.as_ref().unwrap().pulse.unwrap();
        assert_eq!(pulse.frames, 90);
        assert_eq!(pulse.cycle_ms, 3000);
    }
}
