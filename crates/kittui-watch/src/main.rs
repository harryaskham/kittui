//! `kittui-watch` — live preview daemon.
//!
//! Watches a scene-JSON file via stat polling and re-emits the kittui
//! placement on change. Authors can iterate on chrome JSON in their
//! editor without rebuilding any host code; saving the file triggers a
//! re-render in whatever terminal `kittui-watch` is attached to.
//!
//! Usage:
//!
//! ```sh
//! kittui-watch panel.json
//! ```
//!
//! Press Ctrl-C to stop. The daemon exits cleanly after deleting the
//! placement so the terminal isn't left with an orphan image.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use clap::Parser;

use kittui::{Composer, Composition, CompositionEntry, Runtime, Scene};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Path to a scene-JSON file.
    path: PathBuf,
    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = 200)]
    interval_ms: u64,
    /// Cache directory override.
    #[arg(long, env = "KITTUI_CACHE_DIR")]
    cache_dir: Option<PathBuf>,
}

fn read_scene(path: &PathBuf) -> Result<Scene> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let scene: Scene = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(scene)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut builder = Runtime::builder();
    if let Some(dir) = cli.cache_dir.as_ref() {
        builder = builder.cache_dir(dir.clone());
    }
    let runtime = builder.build()?;
    let composer = Composer::new();
    let stdout = std::io::stdout();

    // Drain placements on Ctrl-C so the terminal is left clean.
    let _guard = scopeguard_drain(&composer, &runtime);

    let mut last_mtime: Option<SystemTime> = None;
    let interval = Duration::from_millis(cli.interval_ms);
    eprintln!("kittui-watch: watching {} every {interval:?}", cli.path.display());

    loop {
        let mtime = fs::metadata(&cli.path).ok().and_then(|m| m.modified().ok());
        if mtime != last_mtime {
            last_mtime = mtime;
            match read_scene(&cli.path) {
                Ok(scene) => {
                    let footprint = scene.footprint;
                    let mut comp = Composition::new();
                    comp.push(CompositionEntry {
                        key: Some("preview".to_owned()),
                        footprint,
                        scene,
                    });
                    match composer.apply(&comp, &runtime) {
                        Ok(diff) => {
                            let mut h = stdout.lock();
                            h.write_all(diff.upload.as_bytes())?;
                            h.write_all(diff.placement.as_bytes())?;
                            h.write_all(diff.deletes.as_bytes())?;
                            h.flush()?;
                            eprintln!(
                                "kittui-watch: applied diff (upload={}, place={}, deletes={})",
                                diff.upload.len(),
                                diff.placement.len(),
                                diff.deletes.len()
                            );
                        }
                        Err(e) => eprintln!("kittui-watch: apply error: {e}"),
                    }
                }
                Err(e) => eprintln!("kittui-watch: read error: {e:#}"),
            }
        }
        thread::sleep(interval);
    }
}

/// Tiny scope guard that drains placements on drop. Avoids pulling in
/// the `scopeguard` crate for one closure.
fn scopeguard_drain<'a>(composer: &'a Composer, runtime: &'a Runtime) -> DrainGuard<'a> {
    DrainGuard { composer, runtime }
}

struct DrainGuard<'a> {
    composer: &'a Composer,
    runtime: &'a Runtime,
}

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        let bytes = self.composer.drain(self.runtime);
        if !bytes.is_empty() {
            let _ = std::io::stdout().write_all(bytes.as_bytes());
        }
    }
}
