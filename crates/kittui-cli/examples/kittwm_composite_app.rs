//! Composite kittwm SDK example.
//!
//! Run inside a native kittwm pane so `KITTWM_SOCKET` is inherited:
//!
//! ```text
//! cargo run -p kittui-cli --example kittwm_composite_app -- --browser https://example.com
//! ```
//!
//! The current SDK transport can spawn terminal surfaces today. Browser/GUI
//! surface spawning is represented as a typed request and falls back to a
//! terminal placeholder until the runtime exposes browser/X/Quartz spawning over
//! the socket. The composition/routing logic here is deliberately first-party
//! and reusable: child surfaces are read, composed side-by-side, and input is
//! routed by coordinate offset.

use std::env;

use kittwm_sdk::{Kittwm, SurfaceHandle, SurfaceKind, SurfaceSpec, TextSnapshot};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompositeRegion {
    name: &'static str,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
}

impl CompositeRegion {
    fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x
            && x < self.x.saturating_add(self.cols)
            && y >= self.y
            && y < self.y.saturating_add(self.rows)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompositeLayout {
    left: CompositeRegion,
    right: CompositeRegion,
}

impl CompositeLayout {
    fn side_by_side(cols: u16, rows: u16) -> Self {
        let left_cols = cols / 2;
        Self {
            left: CompositeRegion {
                name: "terminal",
                x: 0,
                y: 0,
                cols: left_cols,
                rows,
            },
            right: CompositeRegion {
                name: "browser",
                x: left_cols,
                y: 0,
                cols: cols.saturating_sub(left_cols),
                rows,
            },
        }
    }

    fn route(&self, x: u16, y: u16) -> Option<&CompositeRegion> {
        if self.left.contains(x, y) {
            Some(&self.left)
        } else if self.right.contains(x, y) {
            Some(&self.right)
        } else {
            None
        }
    }
}

fn compose_side_by_side(left: &str, right: &str, width: usize) -> String {
    let left_width = (width / 2).max(1);
    let right_width = width.saturating_sub(left_width).max(1);
    let left_lines = left.lines().collect::<Vec<_>>();
    let right_lines = right.lines().collect::<Vec<_>>();
    let rows = left_lines.len().max(right_lines.len()).max(1);
    let mut out = String::new();
    for row in 0..rows {
        let l = truncate_pad(left_lines.get(row).copied().unwrap_or(""), left_width);
        let r = truncate_pad(right_lines.get(row).copied().unwrap_or(""), right_width);
        out.push_str(&l);
        out.push_str(&r);
        out.push('\n');
    }
    out
}

fn truncate_pad(text: &str, width: usize) -> String {
    let mut out = text.chars().take(width).collect::<String>();
    let len = out.chars().count();
    if len < width {
        out.push_str(&" ".repeat(width - len));
    }
    out
}

fn spawn_terminal(wm: &Kittwm, command: &str, title: &str) -> Result<SurfaceHandle> {
    let spawn = wm.spawn_surface(&SurfaceSpec::terminal(command).titled(title))?;
    Ok(spawn.handle)
}

fn spawn_browser_or_placeholder(wm: &Kittwm, target: &str) -> Result<SurfaceHandle> {
    let browser = SurfaceSpec {
        kind: SurfaceKind::Browser,
        command: target.to_string(),
        title: Some(format!("browser: {target}")),
    };
    match wm.spawn_surface(&browser) {
        Ok(spawn) => Ok(spawn.handle),
        Err(err) => {
            let escaped = target.replace('"', "\\\"");
            spawn_terminal(
                wm,
                &format!(
                    "printf 'browser surface pending in SDK transport\\nrequested: {escaped}\\n'; exec ${{SHELL:-/bin/sh}} -l"
                ),
                "browser placeholder",
            )
            .map_err(|fallback| {
                format!("browser spawn failed ({err}); placeholder spawn failed ({fallback})").into()
            })
        }
    }
}

fn read_or_placeholder(surface: &SurfaceHandle, label: &str) -> TextSnapshot {
    surface.read_text().unwrap_or_else(|err| TextSnapshot {
        window: surface.id.clone(),
        text: format!("{label}: read_text unavailable: {err}"),
        cursor_col: None,
        cursor_row: None,
    })
}

fn main() -> Result<()> {
    let mut browser = "https://example.com".to_string();
    let mut cols = 100u16;
    let mut rows = 30u16;
    let mut route: Option<(u16, u16, String)> = None;

    let args = env::args().skip(1).collect::<Vec<_>>();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--browser" => {
                i += 1;
                browser = args.get(i).cloned().unwrap_or(browser);
            }
            "--cols" => {
                i += 1;
                cols = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(cols);
            }
            "--rows" => {
                i += 1;
                rows = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(rows);
            }
            "--route-text" => {
                let x = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let y = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0);
                let text = args.get(i + 3).cloned().unwrap_or_default();
                route = Some((x, y, text));
                i += 3;
            }
            "--help" | "-h" => {
                println!("usage: kittwm_composite_app [--browser URL] [--cols N] [--rows N] [--route-text X Y TEXT]");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    let wm = Kittwm::connect_from_env()?;
    let terminal = spawn_terminal(&wm, "${SHELL:-/bin/sh} -l", "composite terminal")?;
    let browser = spawn_browser_or_placeholder(&wm, &browser)?;
    let layout = CompositeLayout::side_by_side(cols, rows);

    if let Some((x, y, text)) = route {
        if let Some(region) = layout.route(x, y) {
            let target = if region.name == "terminal" {
                &terminal
            } else {
                &browser
            };
            let _ = target.focus();
            let _ = target.send_text(text);
        }
    }

    let left = read_or_placeholder(&terminal, "terminal");
    let right = read_or_placeholder(&browser, "browser");
    println!(
        "{}",
        compose_side_by_side(&left.text, &right.text, cols as usize)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_by_side_layout_routes_by_coordinate() {
        let layout = CompositeLayout::side_by_side(100, 20);
        assert_eq!(layout.route(0, 0).unwrap().name, "terminal");
        assert_eq!(layout.route(49, 19).unwrap().name, "terminal");
        assert_eq!(layout.route(50, 0).unwrap().name, "browser");
        assert_eq!(layout.route(99, 19).unwrap().name, "browser");
        assert!(layout.route(100, 0).is_none());
    }

    #[test]
    fn composition_pads_and_truncates_columns() {
        let out = compose_side_by_side("left-long\nL2", "right\nR2", 10);
        let lines = out.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "left-right");
        assert_eq!(lines[1], "L2   R2   ");
    }
}
