//! Single-line affordances: chips, dividers, titles. Each returns a
//! ratakittui `Chrome` that hosts pair with `KittuiChip` / `KittuiTitle`
//! / `KittuiDivider`.

use kittui::Direction;
use kittui::Rgba;
use ratakittui::{Background, Border, Chrome, Padding};

/// Pill-shaped chip with `bg` background and a high-contrast border.
pub fn chip_chrome(bg: Rgba, border: Rgba) -> Chrome {
    Chrome::default()
        .background(Background::Solid(bg))
        .border(Border::rounded(border, 1.0, 7.0))
}

/// Two-color horizontal gradient divider, one cell tall.
pub fn divider_chrome(left: Rgba, right: Rgba) -> Chrome {
    Chrome::default().background(Background::Linear {
        direction: Direction::Horizontal,
        start: left,
        end: right,
    })
}

/// Title bar with a left-to-right gradient and left/right padding.
pub fn title_chrome(left: Rgba, right: Rgba) -> Chrome {
    Chrome::default()
        .background(Background::Linear {
            direction: Direction::Horizontal,
            start: left,
            end: right,
        })
        .padding(Padding::trbl(0, 1, 0, 1))
}
