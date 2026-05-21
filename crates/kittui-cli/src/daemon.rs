//! Minimal Unix-socket daemon protocol for kitwm.
//!
//! Single-line text requests; reply is one line. RAII guard removes the
//! socket file on drop. The server runs an accept loop on a worker
//! thread and exits when the main thread drops the [`DaemonServer`].

use anyhow::{anyhow, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Default socket path for the kitwm daemon. Honors `KITWM_SOCK`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("KITWM_SOCK") {
        return PathBuf::from(p);
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "anon".to_string());
    PathBuf::from(format!("/tmp/kitwm-{user}.sock"))
}

/// Accept-loop daemon that answers `PING` / `STATUS` / `QUIT`.
pub struct DaemonServer {
    path: PathBuf,
    started: Instant,
    quit: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for DaemonServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonServer")
            .field("path", &self.path)
            .field("uptime", &self.started.elapsed())
            .field("quit_requested", &self.quit.load(Ordering::SeqCst))
            .finish()
    }
}

impl DaemonServer {
    pub fn bind(path: PathBuf) -> Result<Self> {
        // If a stale socket exists, try to ping it. If a real server is
        // there we fail loudly; otherwise we unlink and rebind.
        if path.exists() {
            match client_request(&path, "PING") {
                Ok(reply) if reply.trim() == "PONG" => {
                    return Err(anyhow!(
                        "another kitwm daemon is already listening on {}",
                        path.display()
                    ));
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        let listener = UnixListener::bind(&path)
            .map_err(|e| anyhow!("bind {}: {e}", path.display()))?;
        listener
            .set_nonblocking(false)
            .map_err(|e| anyhow!("set_nonblocking: {e}"))?;
        let started = Instant::now();
        let quit = Arc::new(AtomicBool::new(false));
        let quit_t = quit.clone();
        let path_t = path.clone();
        let accept_thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if quit_t.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = handle_request(stream, started, &path_t, &quit_t);
            }
            let _ = std::fs::remove_file(&path_t);
        });
        Ok(Self {
            path,
            started,
            quit,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True if a `QUIT` request was received by the accept thread.
    pub fn quit_requested(&self) -> bool {
        self.quit.load(Ordering::SeqCst)
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }
}

impl Drop for DaemonServer {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        // Wake the accept loop by connecting once.
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.accept_thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle_request(
    stream: UnixStream,
    started: Instant,
    path: &Path,
    quit: &AtomicBool,
) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let cmd = line.trim();
    let reply = match cmd {
        "PING" => "PONG\n".to_string(),
        "STATUS" => format!(
            "pid={} uptime_s={} sock={}\n",
            std::process::id(),
            started.elapsed().as_secs(),
            path.display()
        ),
        "WINDOWS" => windows_reply(),
        "DISPLAYS" => displays_reply(),
        "HELP" | "?" => {
            "PING | STATUS | WINDOWS | DISPLAYS | QUIT | HELP\n".to_string()
        }
        "QUIT" => {
            quit.store(true, Ordering::SeqCst);
            "BYE\n".to_string()
        }
        other => format!("ERR unknown: {other}\n"),
    };
    writer.write_all(reply.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// Send a single-line request and return the reply line.
pub fn client_request(path: &Path, cmd: &str) -> Result<String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|e| anyhow!("connect {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_sock() -> PathBuf {
        std::env::temp_dir().join(format!(
            "kitwm-test-{}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn ping_pong_round_trip() {
        let p = tmp_sock();
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let reply = client_request(server.path(), "PING").unwrap();
        assert_eq!(reply.trim(), "PONG");
    }

    #[test]
    fn status_includes_pid_and_uptime() {
        let p = std::env::temp_dir().join(format!(
            "kitwm-test-status-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let reply = client_request(server.path(), "STATUS").unwrap();
        assert!(reply.contains("pid="), "{reply}");
        assert!(reply.contains("uptime_s="), "{reply}");
        assert!(reply.contains("sock="), "{reply}");
    }

    #[test]
    fn quit_sets_flag() {
        let p = std::env::temp_dir().join(format!(
            "kitwm-test-quit-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        let server = DaemonServer::bind(p.clone()).unwrap();
        let reply = client_request(server.path(), "QUIT").unwrap();
        assert_eq!(reply.trim(), "BYE");
        // Give the accept thread a moment.
        std::thread::sleep(Duration::from_millis(50));
        assert!(server.quit_requested());
    }

    #[test]
    fn double_bind_detects_existing_daemon() {
        let p = std::env::temp_dir().join(format!(
            "kitwm-test-dup-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        let _a = DaemonServer::bind(p.clone()).unwrap();
        let err = DaemonServer::bind(p.clone()).unwrap_err();
        assert!(err.to_string().contains("already listening"), "{err}");
    }
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn windows_reply() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let wins = kittui_quartz::QuartzServer::list_app_windows();
    let _ = writeln!(out, "WINDOWS {}", wins.len());
    for w in wins {
        let _ = writeln!(
            out,
            "  id={} owner={:?} title={:?} bounds=({:.0},{:.0} {:.0}x{:.0})",
            w.id,
            w.owner_name,
            w.title,
            w.bounds.origin.0,
            w.bounds.origin.1,
            w.bounds.width,
            w.bounds.height,
        );
    }
    out.push_str("END\n");
    out
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn windows_reply() -> String {
    "ERR WINDOWS requires --features quartz on macOS\n".to_string()
}

#[cfg(all(target_os = "macos", feature = "quartz"))]
fn displays_reply() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let ds = kittui_quartz::QuartzServer::displays();
    let _ = writeln!(out, "DISPLAYS {}", ds.len());
    for d in ds {
        let _ = writeln!(
            out,
            "  index={} id={} bounds=({:.0},{:.0} {:.0}x{:.0})",
            d.index, d.id, d.bounds.origin.0, d.bounds.origin.1, d.bounds.width, d.bounds.height
        );
    }
    out.push_str("END\n");
    out
}

#[cfg(not(all(target_os = "macos", feature = "quartz")))]
fn displays_reply() -> String {
    "ERR DISPLAYS requires --features quartz on macOS\n".to_string()
}

/// Multi-line client request — keeps reading until EOF, or until a line
/// containing exactly "END" arrives (so multi-line replies like WINDOWS
/// don't drop after the first line).
pub fn client_request_multi(path: &Path, cmd: &str) -> Result<String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|e| anyhow!("connect {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut out = String::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if line.trim() == "END" {
            break;
        }
        out.push_str(&line);
        // Single-line replies don't send END; break after one if it
        // doesn't look like a known multi-line header.
        let first = out.lines().next().unwrap_or("");
        if !first.starts_with("WINDOWS ") && !first.starts_with("DISPLAYS ") {
            break;
        }
    }
    Ok(out)
}
