//! Tonal panel: the showcase example's "assistant / tool / user" panel
//! distilled to a free function returning a ratakittui `Chrome`.

use kittui::Direction;
use ratakittui::{Background, Border, Chrome, Glow, Padding, Pulse};

use crate::palette::{Palette, Tone};

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
    let p = Palette::for_tone(tone);
    let mut chrome = Chrome::default()
        .background(Background::Linear {
            direction: Direction::Vertical,
            start: p.bg_top,
            end: p.bg_bottom,
        })
        .border(Border::rounded(p.rail, 1.5, 8.0))
        .padding(Padding::trbl(1, 2, 1, 2));
    if opts.animated {
        chrome = chrome.glow(Glow {
            color: p.glow,
            cx: 0.5,
            cy: 0.5,
            radius: 0.5,
            intensity: 0.55,
            pulse: Some(Pulse {
                frames: 8,
                cycle_ms: 800,
            }),
        });
    }
    chrome
}
