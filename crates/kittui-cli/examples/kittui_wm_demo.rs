//! kittui-wm v1 demo. Hosts a fake X server (with two solid-color windows
//! in floating mode) inside the agent process, composes them as kittui
//! scenes with chrome, and flushes them to stdout as kitty graphics for a
//! visual sanity check. The real-Xvfb path lives behind the `xvfb` feature
//! in `kittui-xvfb` and is wired into a follow-up bead.
//!
//! Run with `cargo run --release -p kittui-cli --example kittui_wm_demo`.

use std::io::{self, Write};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use kittui::{CellSize, Runtime, TerminalInfo};
use kittui_core::geom::PxRect;
use kittui_input::{InputEvent, MouseButton, Modifiers};
use kittui_wm::compositor::{Compositor, Layout, WindowMode};
use kittui_xvfb::{FakeServer, XWindowId};

fn main() -> Result<()> {
    // Two solid-colour windows: one red on the left, one green on the right.
    let server = FakeServer::with_windows(vec![
        (
            XWindowId(1),
            PxRect::new(8.0, 16.0, 256.0, 160.0),
            "alpha",
            [0xff, 0x40, 0x40, 0xff],
        ),
        (
            XWindowId(2),
            PxRect::new(320.0, 64.0, 256.0, 160.0),
            "beta",
            [0x40, 0xc0, 0xff, 0xff],
        ),
    ]);
    let cell = CellSize::default();
    let compositor = Compositor::new(server, cell);
    let runtime = Runtime::builder()
        .terminal(TerminalInfo::detect())
        .build()?;

    // Compose once with both floating, then again with the first window tiled
    // to demonstrate the layout API.
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    println!("\x1b[2J\x1b[H\x1b[1mkittui-wm v1 demo — floating layout\x1b[0m");
    for scene in compositor.compose()? {
        let p = runtime.place(&scene)?;
        handle.write_all(p.upload.as_bytes())?;
        write!(
            handle,
            "\x1b[{};{}H",
            scene.footprint.y + 1,
            scene.footprint.x + 1
        )?;
        handle.write_all(p.placement.as_bytes())?;
        handle.write_all(p.embed.as_bytes())?;
    }
    handle.flush()?;
    thread::sleep(Duration::from_millis(1500));

    println!("\x1b[2J\x1b[H\x1b[1mkittui-wm v1 demo — alpha tiled, beta floating\x1b[0m");
    let mut layout = Layout::all_floating();
    layout.tile(XWindowId(1), PxRect::new(8.0, 16.0, 320.0, 192.0));
    compositor.set_mode(XWindowId(1), WindowMode::Tiled);
    for scene in compositor.compose_with_layout(&layout)? {
        let p = runtime.place(&scene)?;
        handle.write_all(p.upload.as_bytes())?;
        write!(
            handle,
            "\x1b[{};{}H",
            scene.footprint.y + 1,
            scene.footprint.x + 1
        )?;
        handle.write_all(p.placement.as_bytes())?;
        handle.write_all(p.embed.as_bytes())?;
    }
    handle.flush()?;
    thread::sleep(Duration::from_millis(1500));

    // Simulate a left-click at the centre of the beta window and report which
    // X events get routed back. This is what would drive XTestFake* in the
    // real-Xvfb path.
    let ev = InputEvent::MousePress {
        button: MouseButton::Left,
        col: 50,
        row: 6,
        mods: Modifiers::default(),
    };
    let routed = compositor.route_pointer(&ev);
    write!(
        handle,
        "\x1b[2J\x1b[Hrouted {} pointer events to the X server: {:#?}\n",
        routed.len(),
        routed
    )?;
    handle.flush()?;
    Ok(())
}
