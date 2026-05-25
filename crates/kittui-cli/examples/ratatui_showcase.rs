//! kittui visual lab — interactive fullscreen TUI showcasing every kittui
//! chrome variant and every ratakittui widget wrapper, with live tunable
//! controls and an htop-style performance panel so we can watch frame
//! cost as the WM substrate scales up.
//!
//! Run with `cargo run --release -p kittui-cli --example ratatui_showcase`.
//! Keys:
//!   q / Esc   quit
//!   space     toggle pause for animated chrome
//!   tab       cycle the active control
//!   left/right adjust the active control
//!   r         reset all controls
//!   g         toggle the perf grid (htop-style panel)
//!   1..6      preset themes (assistant/tool/user/cyan/violet/lime)
//!
//! This intentionally does a lot in one screen: every panel is decorated
//! by a different combination of chrome (gradient + border + glow + pulse +
//! scanlines + shadow + clip), every ratatui widget wrapper is exercised at
//! least once, and a live perf panel reports frame time and bytes-per-frame
//! so we have a baseline for the WM workload to come.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction as LayoutDir, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine, Rectangle};
use ratatui::widgets::{
    BarChart, Block, Borders, Cell as TableCell, Chart, Dataset, Gauge, GraphType, LineGauge, List,
    ListItem, Paragraph, Row, Sparkline, Table, Tabs,
};
use ratatui::{Frame, Terminal};

use kittui::{Direction as KittuiDir, Rgba, Runtime, TerminalInfo};
use ratakittui::{
    inline::{KittuiChip, KittuiDivider, KittuiLine, KittuiTitle},
    Background, Border, Chrome, EffectsSink, Glow, JoinGroup, KittuiBarChart, KittuiBlock,
    KittuiCanvas, KittuiChart, KittuiClear, KittuiGauge, KittuiLineGauge, KittuiList,
    KittuiParagraph, KittuiSparkline, KittuiTable, KittuiTabs, LifecycleTracker, Padding, Pulse,
    Scanlines, Shadow,
};

const TOTAL_PERF_SAMPLES: usize = 240;
const SHOWCASE_ANIMATION_FPS: u32 = 60;
const SHOWCASE_ANIMATION_FRAMES: u16 = 180;
const SHOWCASE_ANIMATION_CYCLE_MS: u32 =
    (SHOWCASE_ANIMATION_FRAMES as u32 * 1000) / SHOWCASE_ANIMATION_FPS;

#[derive(Copy, Clone, Debug, PartialEq)]
enum Tone {
    Assistant,
    Tool,
    User,
    Cyan,
    Violet,
    Lime,
}

impl Tone {
    fn cycle(self) -> Self {
        match self {
            Self::Assistant => Self::Tool,
            Self::Tool => Self::User,
            Self::User => Self::Cyan,
            Self::Cyan => Self::Violet,
            Self::Violet => Self::Lime,
            Self::Lime => Self::Assistant,
        }
    }

    fn palette(self) -> Palette {
        let parse = |s: &str| Rgba::parse(s).unwrap();
        match self {
            Self::Assistant => Palette {
                bg_top: parse("#07111fff"),
                bg_bottom: parse("#11192cff"),
                rail: parse("#00d8ff"),
                glow: parse("#00d8ffaa"),
                accent: parse("#72fbd6"),
                ratatui_fg: Color::Rgb(0x00, 0xd8, 0xff),
            },
            Self::Tool => Palette {
                bg_top: parse("#080d1bff"),
                bg_bottom: parse("#171326ff"),
                rail: parse("#b48cff"),
                glow: parse("#b48cffaa"),
                accent: parse("#ffe0a8"),
                ratatui_fg: Color::Rgb(0xb4, 0x8c, 0xff),
            },
            Self::User => Palette {
                bg_top: parse("#061817ff"),
                bg_bottom: parse("#0e202cff"),
                rail: parse("#72fbd6"),
                glow: parse("#72fbd6aa"),
                accent: parse("#a8ffd6"),
                ratatui_fg: Color::Rgb(0x72, 0xfb, 0xd6),
            },
            Self::Cyan => Palette {
                bg_top: parse("#06101eff"),
                bg_bottom: parse("#091a30ff"),
                rail: parse("#00ffff"),
                glow: parse("#00ffffaa"),
                accent: parse("#00d8ff"),
                ratatui_fg: Color::Cyan,
            },
            Self::Violet => Palette {
                bg_top: parse("#0a0820ff"),
                bg_bottom: parse("#1a1130ff"),
                rail: parse("#c896ff"),
                glow: parse("#c896ffaa"),
                accent: parse("#ff8cf0"),
                ratatui_fg: Color::Magenta,
            },
            Self::Lime => Palette {
                bg_top: parse("#091e10ff"),
                bg_bottom: parse("#0e2c14ff"),
                rail: parse("#c0ff5a"),
                glow: parse("#c0ff5aaa"),
                accent: parse("#f4ffaa"),
                ratatui_fg: Color::Rgb(0xc0, 0xff, 0x5a),
            },
        }
    }
}

struct Palette {
    bg_top: Rgba,
    bg_bottom: Rgba,
    rail: Rgba,
    glow: Rgba,
    accent: Rgba,
    ratatui_fg: Color,
}

#[derive(Copy, Clone, Debug)]
enum Control {
    BorderWidth,
    BorderRadius,
    GlowIntensity,
    PulseFramesPer,
    PulseCycleMs,
    ScanlineAlpha,
    ScanlinePeriod,
    PaddingTop,
    PaddingSides,
    ShadowOffset,
}

impl Control {
    const ALL: [Control; 10] = [
        Control::BorderWidth,
        Control::BorderRadius,
        Control::GlowIntensity,
        Control::PulseFramesPer,
        Control::PulseCycleMs,
        Control::ScanlineAlpha,
        Control::ScanlinePeriod,
        Control::PaddingTop,
        Control::PaddingSides,
        Control::ShadowOffset,
    ];
    fn label(self) -> &'static str {
        match self {
            Control::BorderWidth => "border width (px)",
            Control::BorderRadius => "border radius (px)",
            Control::GlowIntensity => "glow intensity",
            Control::PulseFramesPer => "pulse frames",
            Control::PulseCycleMs => "pulse cycle (ms)",
            Control::ScanlineAlpha => "scanline alpha",
            Control::ScanlinePeriod => "scanline period (px)",
            Control::PaddingTop => "padding top (cells)",
            Control::PaddingSides => "padding sides (cells)",
            Control::ShadowOffset => "shadow offset (px)",
        }
    }
}

struct Controls {
    border_width: f32,
    border_radius: f32,
    glow_intensity: f32,
    pulse_frames: u16,
    pulse_cycle_ms: u32,
    scanline_alpha: u8,
    scanline_period: u8,
    padding_top: u16,
    padding_sides: u16,
    shadow_offset: f32,
}

impl Controls {
    fn defaults() -> Self {
        Self {
            border_width: 1.5,
            border_radius: 8.0,
            glow_intensity: 0.55,
            pulse_frames: SHOWCASE_ANIMATION_FRAMES,
            pulse_cycle_ms: SHOWCASE_ANIMATION_CYCLE_MS,
            scanline_alpha: 0x22,
            scanline_period: 3,
            padding_top: 1,
            padding_sides: 2,
            shadow_offset: 3.0,
        }
    }

    fn bump(&mut self, c: Control, dir: i32) {
        let d = dir as f32;
        match c {
            Control::BorderWidth => {
                self.border_width = (self.border_width + 0.5 * d).clamp(0.0, 6.0)
            }
            Control::BorderRadius => {
                self.border_radius = (self.border_radius + 1.0 * d).clamp(0.0, 24.0)
            }
            Control::GlowIntensity => {
                self.glow_intensity = (self.glow_intensity + 0.05 * d).clamp(0.0, 1.0)
            }
            Control::PulseFramesPer => {
                let next = (self.pulse_frames as i32 + dir * 10).clamp(2, 360);
                self.pulse_frames = next as u16;
            }
            Control::PulseCycleMs => {
                let next = (self.pulse_cycle_ms as i32 + dir * 100).clamp(200, 4000);
                self.pulse_cycle_ms = next as u32;
            }
            Control::ScanlineAlpha => {
                let next = (self.scanline_alpha as i32 + dir * 16).clamp(0, 255);
                self.scanline_alpha = next as u8;
            }
            Control::ScanlinePeriod => {
                let next = (self.scanline_period as i32 + dir).clamp(1, 16);
                self.scanline_period = next as u8;
            }
            Control::PaddingTop => {
                let next = (self.padding_top as i32 + dir).clamp(0, 6);
                self.padding_top = next as u16;
            }
            Control::PaddingSides => {
                let next = (self.padding_sides as i32 + dir).clamp(0, 6);
                self.padding_sides = next as u16;
            }
            Control::ShadowOffset => {
                self.shadow_offset = (self.shadow_offset + 1.0 * d).clamp(0.0, 16.0)
            }
        }
    }

    fn value_str(&self, c: Control) -> String {
        match c {
            Control::BorderWidth => format!("{:.1}", self.border_width),
            Control::BorderRadius => format!("{:.1}", self.border_radius),
            Control::GlowIntensity => format!("{:.2}", self.glow_intensity),
            Control::PulseFramesPer => format!("{}", self.pulse_frames),
            Control::PulseCycleMs => format!("{}", self.pulse_cycle_ms),
            Control::ScanlineAlpha => format!("{}", self.scanline_alpha),
            Control::ScanlinePeriod => format!("{}", self.scanline_period),
            Control::PaddingTop => format!("{}", self.padding_top),
            Control::PaddingSides => format!("{}", self.padding_sides),
            Control::ShadowOffset => format!("{:.1}", self.shadow_offset),
        }
    }
}

struct Perf {
    samples: Vec<u64>,         // frame microseconds
    upload_bytes: Vec<u64>,    // per-frame upload bytes
    placement_bytes: Vec<u64>, // per-frame placement bytes
    frame_count: u64,
    last_frame_at: Instant,
}

impl Perf {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(TOTAL_PERF_SAMPLES),
            upload_bytes: Vec::with_capacity(TOTAL_PERF_SAMPLES),
            placement_bytes: Vec::with_capacity(TOTAL_PERF_SAMPLES),
            frame_count: 0,
            last_frame_at: Instant::now(),
        }
    }

    fn record(&mut self, micros: u64, upload: u64, placement: u64) {
        if self.samples.len() == TOTAL_PERF_SAMPLES {
            self.samples.remove(0);
            self.upload_bytes.remove(0);
            self.placement_bytes.remove(0);
        }
        self.samples.push(micros);
        self.upload_bytes.push(upload);
        self.placement_bytes.push(placement);
        self.frame_count = self.frame_count.saturating_add(1);
        self.last_frame_at = Instant::now();
    }

    fn average_micros(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum: u64 = self.samples.iter().sum();
        sum / self.samples.len() as u64
    }

    fn max_micros(&self) -> u64 {
        *self.samples.iter().max().unwrap_or(&0)
    }

    fn fps(&self) -> f64 {
        let avg = self.average_micros();
        if avg == 0 {
            0.0
        } else {
            1_000_000.0 / avg as f64
        }
    }
}

struct App {
    tone: Tone,
    controls: Controls,
    perf: Perf,
    paused: bool,
    active_control: usize,
    show_perf: bool,
    started_at: Instant,
    tab_index: usize,
}

impl App {
    fn new() -> Self {
        Self {
            tone: Tone::Assistant,
            controls: Controls::defaults(),
            perf: Perf::new(),
            paused: false,
            active_control: 0,
            show_perf: true,
            started_at: Instant::now(),
            tab_index: 0,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Tab => self.active_control = (self.active_control + 1) % Control::ALL.len(),
            KeyCode::BackTab => {
                self.active_control =
                    (self.active_control + Control::ALL.len() - 1) % Control::ALL.len()
            }
            KeyCode::Left => self.controls.bump(Control::ALL[self.active_control], -1),
            KeyCode::Right => self.controls.bump(Control::ALL[self.active_control], 1),
            KeyCode::Char('r') => self.controls = Controls::defaults(),
            KeyCode::Char('g') => self.show_perf = !self.show_perf,
            KeyCode::Char('t') => self.tone = self.tone.cycle(),
            KeyCode::Char('1') => self.tone = Tone::Assistant,
            KeyCode::Char('2') => self.tone = Tone::Tool,
            KeyCode::Char('3') => self.tone = Tone::User,
            KeyCode::Char('4') => self.tone = Tone::Cyan,
            KeyCode::Char('5') => self.tone = Tone::Violet,
            KeyCode::Char('6') => self.tone = Tone::Lime,
            KeyCode::Char('[') => self.tab_index = self.tab_index.saturating_sub(1),
            KeyCode::Char(']') => self.tab_index = (self.tab_index + 1).min(3),
            _ => {}
        }
        true
    }
}

fn main() -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;
    let tracker = LifecycleTracker::new();
    let target_frame = Duration::from_millis(33); // ~30fps host loop.

    loop {
        let frame_start = Instant::now();
        let mut last_flush_upload = 0u64;
        let mut last_flush_placement = 0u64;

        terminal.draw(|f| {
            let (upload, placement) = render_frame(f, &app, &runtime, &tracker);
            last_flush_upload = upload as u64;
            last_flush_placement = placement as u64;
        })?;

        let elapsed = frame_start.elapsed();
        app.perf.record(
            elapsed.as_micros() as u64,
            last_flush_upload,
            last_flush_placement,
        );

        let remaining = target_frame.checked_sub(elapsed).unwrap_or_default();
        if event::poll(remaining)? {
            if let Event::Key(key) = event::read()? {
                if !app.handle_key(key) {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn render_frame(
    f: &mut Frame<'_>,
    app: &App,
    runtime: &Runtime,
    tracker: &LifecycleTracker,
) -> (usize, usize) {
    let area = f.area();
    let sink = EffectsSink::new();
    tracker.begin_frame();

    let chunks = Layout::default()
        .direction(LayoutDir::Vertical)
        .constraints([
            Constraint::Length(3),                                  // header
            Constraint::Min(10),                                    // body
            Constraint::Length(if app.show_perf { 10 } else { 6 }), // footer (perf/help)
        ])
        .split(area);

    render_header(f, &sink, runtime, app, chunks[0]);
    render_body(f, &sink, runtime, app, chunks[1]);
    render_footer(f, &sink, runtime, app, chunks[2]);

    // Drain effects and emit them around the buffer flush. ratakittui's
    // `finalize_frame` positions the cursor at each chrome origin.
    let flush = ratakittui::finalize_frame(&sink, tracker, runtime);
    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"\x1b[?25l"); // hide cursor
    let _ = stdout.write_all(flush.upload.as_bytes());
    let _ = stdout.write_all(flush.placement.as_bytes());
    let _ = stdout.write_all(flush.deletes.as_bytes());
    let _ = stdout.flush();

    (flush.upload.len(), flush.placement.len())
}

fn render_header(f: &mut Frame<'_>, sink: &EffectsSink, runtime: &Runtime, app: &App, area: Rect) {
    let palette = app.tone.palette();
    let title_chrome = Chrome::default()
        .background(Background::Linear {
            direction: KittuiDir::Horizontal,
            start: palette.rail,
            end: palette.accent,
        })
        .padding(Padding::trbl(0, 1, 0, 1));
    sink.push(
        KittuiTitle::new("kittui — visual lab + perf audit", title_chrome).render_with(
            Rect::new(area.x, area.y, area.width, 1),
            f.buffer_mut(),
            runtime,
        ),
    );

    let div_chrome = Chrome::default().background(Background::Linear {
        direction: KittuiDir::Horizontal,
        start: palette.rail,
        end: palette.glow,
    });
    sink.push(KittuiDivider::new(div_chrome).render_with(
        Rect::new(area.x, area.y + 1, area.width, 1),
        f.buffer_mut(),
        runtime,
    ));

    let tabs = Tabs::new(vec![
        Line::raw("widgets"),
        Line::raw("charts"),
        Line::raw("forms"),
        Line::raw("canvas"),
    ])
    .select(app.tab_index)
    .highlight_style(
        Style::default()
            .fg(palette.ratatui_fg)
            .add_modifier(Modifier::BOLD),
    );
    let tabs_chrome = panel_chrome(&palette, app, false);
    sink.push(KittuiTabs::new(tabs, tabs_chrome).render_with(
        Rect::new(area.x, area.y + 2, area.width, 1),
        f.buffer_mut(),
        runtime,
    ));
}

fn panel_chrome(palette: &Palette, app: &App, animated: bool) -> Chrome {
    let mut chrome = Chrome::default()
        .background(Background::Linear {
            direction: KittuiDir::Vertical,
            start: palette.bg_top,
            end: palette.bg_bottom,
        })
        .border(Border::rounded(
            palette.rail,
            app.controls.border_width,
            app.controls.border_radius,
        ))
        .scanlines(Scanlines {
            alpha: app.controls.scanline_alpha,
            period_px: app.controls.scanline_period,
        })
        .shadow(Shadow {
            dx_px: app.controls.shadow_offset,
            dy_px: app.controls.shadow_offset,
            color: Rgba::parse("#000000aa").unwrap(),
        })
        .padding(Padding::trbl(
            app.controls.padding_top,
            app.controls.padding_sides,
            app.controls.padding_top,
            app.controls.padding_sides,
        ));
    let mut glow = Glow {
        color: palette.glow,
        cx: 0.5,
        cy: 0.5,
        radius: 0.5,
        intensity: app.controls.glow_intensity,
        pulse: None,
    };
    if animated && !app.paused {
        glow.pulse = Some(Pulse {
            frames: app.controls.pulse_frames,
            cycle_ms: app.controls.pulse_cycle_ms,
        });
    }
    chrome = chrome.glow(glow);
    chrome
}

fn render_body(f: &mut Frame<'_>, sink: &EffectsSink, runtime: &Runtime, app: &App, area: Rect) {
    match app.tab_index {
        0 => render_widgets_tab(f, sink, runtime, app, area),
        1 => render_charts_tab(f, sink, runtime, app, area),
        2 => render_forms_tab(f, sink, runtime, app, area),
        3 => render_canvas_tab(f, sink, runtime, app, area),
        _ => {}
    }
}

fn render_widgets_tab(
    f: &mut Frame<'_>,
    sink: &EffectsSink,
    runtime: &Runtime,
    app: &App,
    area: Rect,
) {
    let palette = app.tone.palette();
    let chunks = Layout::default()
        .direction(LayoutDir::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(area);

    // Left: joined assistant/tool panels (animated glow).
    let join_rows = chunks[0].height.saturating_sub(2);
    let group: JoinGroup = ratakittui::join![
        (
            panel_chrome(&palette, app, true),
            Rect::new(chunks[0].x, chunks[0].y, chunks[0].width, join_rows / 2)
        ),
        (
            panel_chrome(&palette, app, false),
            Rect::new(
                chunks[0].x,
                chunks[0].y + join_rows / 2,
                chunks[0].width,
                join_rows - join_rows / 2
            )
        ),
    ];
    let scenes = group.resolve();
    for (i, scene) in scenes.into_iter().enumerate() {
        if let Some(scene) = scene {
            let id = scene.id();
            if let Ok(placement) = runtime.place(&scene) {
                sink.push(ratakittui::RenderEffects::from_placement(&placement, id));
                let inner = if i == 0 {
                    Rect::new(
                        chunks[0].x + 2,
                        chunks[0].y + 1,
                        chunks[0].width.saturating_sub(4),
                        (join_rows / 2).saturating_sub(2),
                    )
                } else {
                    Rect::new(
                        chunks[0].x + 2,
                        chunks[0].y + join_rows / 2 + 1,
                        chunks[0].width.saturating_sub(4),
                        (join_rows - join_rows / 2).saturating_sub(2),
                    )
                };
                let label = if i == 0 { "assistant" } else { "tool" };
                let para = Paragraph::new(vec![
                    Line::from(Span::styled(
                        label,
                        Style::default()
                            .fg(palette.ratatui_fg)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::raw(""),
                    Line::raw("joined-border panels: inner edge masked so the"),
                    Line::raw("border draws exactly once across the seam."),
                    Line::raw(""),
                    Line::raw(format!(
                        "border={:.1}px  radius={:.1}px",
                        app.controls.border_width, app.controls.border_radius
                    )),
                    Line::raw(format!("glow={:.2}", app.controls.glow_intensity)),
                ]);
                use ratatui::widgets::Widget;
                para.render(inner, f.buffer_mut());
            }
        }
    }

    // Middle column: list + table.
    let mid = Layout::default()
        .direction(LayoutDir::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);
    let items: Vec<ListItem> = (0..10)
        .map(|i| ListItem::new(format!("item {i:02} — kittui scene")))
        .collect();
    let list = List::new(items).highlight_style(
        Style::default()
            .fg(palette.ratatui_fg)
            .add_modifier(Modifier::REVERSED),
    );
    sink.push(
        KittuiList::new(list, panel_chrome(&palette, app, false)).render_with(
            mid[0],
            f.buffer_mut(),
            runtime,
        ),
    );

    let rows: Vec<Row> = (0..6)
        .map(|i| {
            Row::new(vec![
                TableCell::from(format!("scene_{i:02}")),
                TableCell::from(format!("{}x{}", 4 + i, 1 + i / 2)),
                TableCell::from(format!("{}ms", 8 + i * 3)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec!["scene", "cells", "render"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );
    sink.push(
        KittuiTable::new(table, panel_chrome(&palette, app, false)).render_with(
            mid[1],
            f.buffer_mut(),
            runtime,
        ),
    );

    // Right column: chips + gauge + line gauge stack.
    let right = Layout::default()
        .direction(LayoutDir::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(chunks[2]);
    sink.push(
        KittuiChip::new(" READY ", chip_chrome(&palette, "#00d8ffcc")).render_with(
            Rect::new(right[0].x, right[0].y, 9, 1),
            f.buffer_mut(),
            runtime,
        ),
    );
    sink.push(
        KittuiChip::new(" BUSY ", chip_chrome(&palette, "#ff8c5acc")).render_with(
            Rect::new(right[0].x + 10, right[0].y, 8, 1),
            f.buffer_mut(),
            runtime,
        ),
    );
    sink.push(
        KittuiChip::new(" OK ", chip_chrome(&palette, "#72fbd6cc")).render_with(
            Rect::new(right[0].x + 19, right[0].y, 6, 1),
            f.buffer_mut(),
            runtime,
        ),
    );

    let ratio = ((app.started_at.elapsed().as_millis() % 4000) as f64 / 4000.0) * 0.7 + 0.15;
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(palette.ratatui_fg))
        .ratio(if app.paused { 0.42 } else { ratio });
    sink.push(
        KittuiGauge::new(gauge, panel_chrome(&palette, app, false)).render_with(
            right[1],
            f.buffer_mut(),
            runtime,
        ),
    );

    let lg = LineGauge::default()
        .filled_style(Style::default().fg(palette.ratatui_fg))
        .ratio((ratio * 1.3).min(0.99));
    sink.push(
        KittuiLineGauge::new(lg, panel_chrome(&palette, app, false)).render_with(
            right[2],
            f.buffer_mut(),
            runtime,
        ),
    );

    sink.push(
        KittuiLine::new("─ live ─".into(), line_chrome(&palette)).render_with(
            Rect::new(right[3].x, right[3].y, right[3].width, 1),
            f.buffer_mut(),
            runtime,
        ),
    );

    // Bottom of right column: sparkline.
    let spark_data: Vec<u64> = app.perf.samples.iter().copied().collect();
    if !spark_data.is_empty() {
        let s = Sparkline::default()
            .data(&spark_data)
            .style(Style::default().fg(palette.ratatui_fg));
        sink.push(
            KittuiSparkline::new(s, panel_chrome(&palette, app, false)).render_with(
                right[4],
                f.buffer_mut(),
                runtime,
            ),
        );
    }
}

fn render_charts_tab(
    f: &mut Frame<'_>,
    sink: &EffectsSink,
    runtime: &Runtime,
    app: &App,
    area: Rect,
) {
    let palette = app.tone.palette();
    let cols = Layout::default()
        .direction(LayoutDir::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // BarChart on the left.
    let bars: Vec<(&str, u64)> = vec![
        ("alpha", 8),
        ("bravo", 15),
        ("charlie", 22),
        ("delta", 11),
        ("echo", 30),
        ("foxtrot", 17),
    ];
    let bc = BarChart::default()
        .data(&bars)
        .bar_width(8)
        .bar_gap(1)
        .bar_style(Style::default().fg(palette.ratatui_fg));
    sink.push(
        KittuiBarChart::new(bc, panel_chrome(&palette, app, false)).render_with(
            cols[0],
            f.buffer_mut(),
            runtime,
        ),
    );

    // Chart on the right (two datasets).
    let data1: Vec<(f64, f64)> = (0..50)
        .map(|i| (i as f64, (i as f64 * 0.2).sin()))
        .collect();
    let data2: Vec<(f64, f64)> = (0..50)
        .map(|i| (i as f64, (i as f64 * 0.2).cos() * 0.8))
        .collect();
    let datasets = vec![
        Dataset::default()
            .name("sin")
            .marker(ratatui::symbols::Marker::Braille)
            .style(Style::default().fg(palette.ratatui_fg))
            .graph_type(GraphType::Line)
            .data(&data1),
        Dataset::default()
            .name("cos")
            .marker(ratatui::symbols::Marker::Braille)
            .style(Style::default().fg(Color::Yellow))
            .graph_type(GraphType::Line)
            .data(&data2),
    ];
    let chart = Chart::new(datasets)
        .x_axis(
            ratatui::widgets::Axis::default()
                .bounds([0.0, 50.0])
                .labels(vec![Span::raw("0"), Span::raw("50")]),
        )
        .y_axis(
            ratatui::widgets::Axis::default()
                .bounds([-1.2, 1.2])
                .labels(vec![Span::raw("-1"), Span::raw("0"), Span::raw("1")]),
        );
    sink.push(
        KittuiChart::new(chart, panel_chrome(&palette, app, true)).render_with(
            cols[1],
            f.buffer_mut(),
            runtime,
        ),
    );
}

fn render_forms_tab(
    f: &mut Frame<'_>,
    sink: &EffectsSink,
    runtime: &Runtime,
    app: &App,
    area: Rect,
) {
    let palette = app.tone.palette();
    let chunks = Layout::default()
        .direction(LayoutDir::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "kittui composer",
            Style::default()
                .fg(palette.ratatui_fg)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::raw("Compose scenes from JSON. The right panel shows the last"),
        Line::raw("placement result; the gauge tracks bytes shipped per frame."),
        Line::raw(""),
        Line::raw("Press [t] to cycle theme, [r] to reset controls."),
    ]);
    sink.push(
        KittuiParagraph::new(header, panel_chrome(&palette, app, false)).render_with(
            chunks[0],
            f.buffer_mut(),
            runtime,
        ),
    );

    // A KittuiBlock containing free text.
    let body = Block::default().borders(Borders::NONE);
    sink.push(
        KittuiBlock::new(body, panel_chrome(&palette, app, true)).render_with(
            chunks[1],
            f.buffer_mut(),
            runtime,
        ),
    );

    let footer = Paragraph::new(vec![Line::from(vec![
        Span::raw("tone "),
        Span::styled(
            format!("{:?}", app.tone),
            Style::default()
                .fg(palette.ratatui_fg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("    frames "),
        Span::raw(format!("{}", app.perf.frame_count)),
    ])]);
    sink.push(
        KittuiParagraph::new(footer, panel_chrome(&palette, app, false)).render_with(
            chunks[2],
            f.buffer_mut(),
            runtime,
        ),
    );
}

fn render_canvas_tab(
    f: &mut Frame<'_>,
    sink: &EffectsSink,
    runtime: &Runtime,
    app: &App,
    area: Rect,
) {
    let palette = app.tone.palette();
    let now = app.started_at.elapsed().as_millis() as f64;
    let phase = ((now / 600.0) % std::f64::consts::TAU) as f64;
    let canvas = Canvas::default()
        .x_bounds([-1.5, 1.5])
        .y_bounds([-1.5, 1.5])
        .paint(move |ctx| {
            for i in 0..32 {
                let a = phase + i as f64 * std::f64::consts::TAU / 32.0;
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 0.0,
                    x2: a.cos(),
                    y2: a.sin(),
                    color: Color::Cyan,
                });
            }
            ctx.draw(&Rectangle {
                x: -1.0,
                y: -1.0,
                width: 2.0,
                height: 2.0,
                color: Color::DarkGray,
            });
        });
    sink.push(
        KittuiCanvas::new(canvas, panel_chrome(&palette, app, true)).render_with(
            area,
            f.buffer_mut(),
            runtime,
        ),
    );

    // A KittuiClear in the corner to demonstrate the wrapper compiles.
    let corner = Rect::new(area.x + area.width.saturating_sub(8), area.y + 1, 8, 1);
    sink.push(
        KittuiClear::new(chip_chrome(&palette, "#08111fcc")).render_with(
            corner,
            f.buffer_mut(),
            runtime,
        ),
    );
    let _ = palette.ratatui_fg;
}

fn render_footer(f: &mut Frame<'_>, sink: &EffectsSink, runtime: &Runtime, app: &App, area: Rect) {
    let palette = app.tone.palette();
    if app.show_perf {
        render_perf(f, sink, runtime, app, area, &palette);
    } else {
        render_help(f, sink, runtime, app, area, &palette);
    }
}

fn render_perf(
    f: &mut Frame<'_>,
    sink: &EffectsSink,
    runtime: &Runtime,
    app: &App,
    area: Rect,
    palette: &Palette,
) {
    let cols = Layout::default()
        .direction(LayoutDir::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(40),
            Constraint::Percentage(25),
        ])
        .split(area);

    // Numeric summary.
    let avg = app.perf.average_micros();
    let max = app.perf.max_micros();
    let fps = app.perf.fps();
    let upload_sum: u64 = app.perf.upload_bytes.iter().sum();
    let place_sum: u64 = app.perf.placement_bytes.iter().sum();
    let summary = Paragraph::new(vec![
        Line::from(Span::styled(
            "perf",
            Style::default()
                .fg(palette.ratatui_fg)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(format!("avg frame   {:>6} µs", avg)),
        Line::raw(format!("max frame   {:>6} µs", max)),
        Line::raw(format!("est fps     {:>6.1}", fps)),
        Line::raw(format!("upload Σ    {:>6} B", upload_sum)),
        Line::raw(format!("placement Σ {:>6} B", place_sum)),
        Line::raw(format!("frames      {:>6}", app.perf.frame_count)),
    ]);
    sink.push(
        KittuiParagraph::new(summary, panel_chrome(palette, app, false)).render_with(
            cols[0],
            f.buffer_mut(),
            runtime,
        ),
    );

    // Sparkline of frame microseconds.
    let data: Vec<u64> = app.perf.samples.clone();
    if !data.is_empty() {
        let sp = Sparkline::default()
            .data(&data)
            .style(Style::default().fg(palette.ratatui_fg));
        sink.push(
            KittuiSparkline::new(sp, panel_chrome(palette, app, false)).render_with(
                cols[1],
                f.buffer_mut(),
                runtime,
            ),
        );
    }

    // Active control panel.
    let mut lines = vec![Line::from(Span::styled(
        "controls (Tab/Shift-Tab, ←/→)",
        Style::default()
            .fg(palette.ratatui_fg)
            .add_modifier(Modifier::BOLD),
    ))];
    for (i, c) in Control::ALL.iter().enumerate() {
        let style = if i == app.active_control {
            Style::default()
                .fg(palette.ratatui_fg)
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{:<22} {}", c.label(), app.controls.value_str(*c)),
            style,
        )));
    }
    let controls = Paragraph::new(lines);
    sink.push(
        KittuiParagraph::new(controls, panel_chrome(palette, app, false)).render_with(
            cols[2],
            f.buffer_mut(),
            runtime,
        ),
    );
}

fn render_help(
    f: &mut Frame<'_>,
    sink: &EffectsSink,
    runtime: &Runtime,
    app: &App,
    area: Rect,
    palette: &Palette,
) {
    let p = Paragraph::new(vec![
        Line::from(Span::styled(
            "help (press g for perf)",
            Style::default()
                .fg(palette.ratatui_fg)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw("q/Esc quit   space pause   Tab cycle control   ←/→ adjust"),
        Line::raw("r reset      g toggle perf  t cycle tone   1-6 themes"),
        Line::raw("[ / ] cycle tabs"),
    ]);
    sink.push(
        KittuiParagraph::new(p, panel_chrome(palette, app, false)).render_with(
            area,
            f.buffer_mut(),
            runtime,
        ),
    );
}

fn chip_chrome(palette: &Palette, bg: &str) -> Chrome {
    Chrome::default()
        .background(Background::Solid(Rgba::parse(bg).unwrap()))
        .border(Border::rounded(palette.accent, 1.0, 7.0))
}

fn line_chrome(palette: &Palette) -> Chrome {
    Chrome::default().background(Background::Linear {
        direction: KittuiDir::Horizontal,
        start: palette.rail,
        end: palette.glow,
    })
}
