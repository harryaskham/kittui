//! ratakittui showcase — drives a ratatui `Frame` with kittui-decorated
//! widgets and prints the resulting upload / placement / delete bytes.
//!
//! This example does not own a live terminal; it composes a ratatui
//! buffer in-memory and then prints the kittui side-channel a real host
//! would emit around the buffer flush. The shape of the integration
//! exactly matches what `draw_with_kittui` does in the live ratatui
//! adapter.
//!
//! Run with `cargo run -p kittui-cli --example ratatui_showcase`.

use std::io::Write;

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

use kittui::{Rgba, Runtime};
use ratakittui::{
    inline::{KittuiChip, KittuiDivider, KittuiTitle},
    join, Background, Border, Chrome, EffectsSink, Glow, JoinGroup, KittuiGauge,
    KittuiParagraph, LifecycleTracker, Padding, Pulse,
};

fn assistant_chrome() -> Chrome {
    Chrome::default()
        .background(Background::Linear {
            direction: kittui::Direction::Vertical,
            start: Rgba::parse("#07111fff").unwrap(),
            end: Rgba::parse("#11192cff").unwrap(),
        })
        .border(Border::rounded(Rgba::parse("#00d8ff").unwrap(), 1.5, 8.0))
        .glow(Glow {
            color: Rgba::parse("#00d8ffaa").unwrap(),
            cx: 0.5,
            cy: 0.5,
            radius: 0.5,
            intensity: 0.55,
            pulse: Some(Pulse {
                frames: 8,
                cycle_ms: 800,
            }),
        })
        .padding(Padding::trbl(1, 2, 1, 2))
}

fn tool_chrome() -> Chrome {
    Chrome::default()
        .background(Background::Linear {
            direction: kittui::Direction::Vertical,
            start: Rgba::parse("#080d1bff").unwrap(),
            end: Rgba::parse("#171326ff").unwrap(),
        })
        .border(Border::rounded(Rgba::parse("#b48cff").unwrap(), 1.5, 8.0))
        .padding(Padding::trbl(1, 2, 1, 2))
}

fn user_chrome() -> Chrome {
    Chrome::default()
        .background(Background::Linear {
            direction: kittui::Direction::Vertical,
            start: Rgba::parse("#061817ff").unwrap(),
            end: Rgba::parse("#0e202cff").unwrap(),
        })
        .border(Border::rounded(Rgba::parse("#72fbd6").unwrap(), 1.5, 8.0))
        .padding(Padding::trbl(1, 2, 1, 2))
}

fn chip_chrome(color: &str) -> Chrome {
    Chrome::default()
        .background(Background::Solid(Rgba::parse(color).unwrap()))
        .border(Border::rounded(Rgba::parse("#08111f").unwrap(), 1.0, 7.0))
}

fn title_chrome() -> Chrome {
    Chrome::default()
        .background(Background::Linear {
            direction: kittui::Direction::Horizontal,
            start: Rgba::parse("#00d8ff").unwrap(),
            end: Rgba::parse("#72fbd6").unwrap(),
        })
        .padding(Padding::trbl(0, 1, 0, 1))
}

fn divider_chrome() -> Chrome {
    Chrome::default().background(Background::Linear {
        direction: kittui::Direction::Horizontal,
        start: Rgba::parse("#00d8ff").unwrap(),
        end: Rgba::parse("#b48cff").unwrap(),
    })
}

fn main() -> Result<()> {
    let runtime = Runtime::builder()
        .terminal(kittui::TerminalInfo::detect())
        .build()?;
    let tracker = LifecycleTracker::new();
    tracker.begin_frame();

    let area = Rect::new(0, 0, 70, 24);
    let mut buf = Buffer::empty(area);
    let sink = EffectsSink::new();

    // Title bar
    sink.push(
        KittuiTitle::new("kittui — ratakittui showcase", title_chrome())
            .render_with(Rect::new(0, 0, 70, 1), &mut buf, &runtime),
    );

    // Divider
    sink.push(
        KittuiDivider::new(divider_chrome())
            .render_with(Rect::new(0, 1, 70, 1), &mut buf, &runtime),
    );

    // Two joined message panels: assistant (left) abuts tool (right)
    let group: JoinGroup = join![
        (assistant_chrome(), Rect::new(0, 3, 35, 9)),
        (tool_chrome(), Rect::new(35, 3, 35, 9)),
    ];
    for (i, scene) in group.resolve().into_iter().enumerate() {
        if let Some(scene) = scene {
            // Render chrome through runtime by hand because the JoinGroup
            // emitted a precomposed scene that includes the masked border.
            let id = scene.id();
            let placement = runtime.place(&scene)?;
            sink.push(ratakittui::RenderEffects::from_placement(&placement, id));
            // Inner widgets render text on top:
            let area = if i == 0 {
                Rect::new(2, 4, 31, 7)
            } else {
                Rect::new(37, 4, 31, 7)
            };
            let title = if i == 0 { "assistant" } else { "tool" };
            Paragraph::new(vec![
                Line::from(Span::styled(title, Style::default().fg(Color::Cyan))),
                Line::raw(""),
                Line::raw("joined-border panel: the inner edge"),
                Line::raw("between these two chrome rects is"),
                Line::raw("masked at the scene level so the"),
                Line::raw("border draws exactly once."),
            ])
            .render_into(area, &mut buf);
        }
    }

    // KittuiBlock with a wrapped ratatui paragraph
    sink.push(
        KittuiParagraph::new(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "settling",
                    Style::default().fg(Color::Rgb(0x72, 0xfb, 0xd6)),
                )),
                Line::raw("config-hash probe pending; live telemetry available"),
            ]),
            user_chrome(),
        )
        .render_with(Rect::new(0, 13, 70, 5), &mut buf, &runtime),
    );

    // KittuiGauge with chrome
    sink.push(
        KittuiGauge::new(
            Gauge::default()
                .ratio(0.62)
                .gauge_style(Style::default().fg(Color::Cyan)),
            Chrome::default()
                .border(Border::rounded(Rgba::parse("#00d8ff").unwrap(), 1.0, 4.0))
                .padding(Padding::trbl(1, 2, 1, 2)),
        )
        .render_with(Rect::new(0, 19, 50, 3), &mut buf, &runtime),
    );

    // KittuiChip alongside
    sink.push(
        KittuiChip::new(" 62% ", chip_chrome("#00d8ffcc"))
            .render_with(Rect::new(52, 20, 8, 1), &mut buf, &runtime),
    );

    // Drain effects through the tracker (would normally happen inside
    // `draw_with_kittui`).
    let flush = ratakittui::finalize_frame(&sink, &tracker, &runtime);

    // Print the kittui side-channel: upload first, then placements, then
    // any deletes from last frame. A real host would interleave these with
    // the ratatui buffer flush.
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    // Clear screen first so the alternate ratatui buffer doesn't overlap.
    handle.write_all(b"\x1b[2J\x1b[H")?;
    handle.write_all(flush.upload.as_bytes())?;
    handle.write_all(flush.placement.as_bytes())?;
    handle.write_all(flush.deletes.as_bytes())?;
    // Park the cursor below the last placed widget.
    handle.write_all(b"\x1b[24;1H")?;
    writeln!(
        handle,
        "\nratakittui showcase: {} placements, {} bytes of placement, {} bytes of upload",
        sink_placement_count(&buf),
        flush.placement.len(),
        flush.upload.len()
    )?;

    Ok(())
}

// Helper: a crude proxy for "how many widgets we drew" — counts non-empty
// cells in the simulated ratatui buffer.
fn sink_placement_count(buf: &Buffer) -> usize {
    let mut count = 0;
    for cell in buf.content() {
        if cell.symbol() != " " && !cell.symbol().is_empty() {
            count += 1;
        }
    }
    count
}

// Local helper trait that lets us render a `Paragraph` into a `Buffer`
// without depending on a ratatui `Frame` — useful for example code.
trait RenderIntoExt {
    fn render_into(self, area: Rect, buf: &mut Buffer);
}

impl<'a> RenderIntoExt for Paragraph<'a> {
    fn render_into(self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::Widget;
        self.render(area, buf);
    }
}

#[allow(dead_code)]
fn _unused_helpers(_: Block<'_>, _: Borders) {}
