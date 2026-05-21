//! Minimal Unix-socket daemon protocol for kittwm.
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

/// Default socket path for the kittwm daemon. Honors `KITTWM_SOCK`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("KITTWM_SOCK") {
        return PathBuf::from(p);
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "anon".to_string());
    PathBuf::from(format!("/tmp/kittwm-{user}.sock"))
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
                        "another kittwm daemon is already listening on {}",
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
    let reply = if let Some(query) = cmd.strip_prefix("APPS_FIRST ") {
        apps_first_reply(query, false)
    } else if let Some(query) = cmd.strip_prefix("APPS_LAUNCH_FIRST ") {
        apps_first_reply(query, true)
    } else {
        match cmd {
        "PING" => "PONG\n".to_string(),
        "STATUS" => format!(
            "pid={} uptime_s={} sock={}\n",
            std::process::id(),
            started.elapsed().as_secs(),
            path.display()
        ),
        "WINDOWS" => windows_reply(),
        "DISPLAYS" => displays_reply(),
        "APPS" => apps_reply(50),
        "APPS_JSON" => apps_json_reply(50),
        "HELP" | "?" => {
            "PING | STATUS | WINDOWS | DISPLAYS | APPS | APPS_JSON | APPS_FIRST <query> | APPS_LAUNCH_FIRST <query> | QUIT | HELP\n".to_string()
        }
        "QUIT" => {
            quit.store(true, Ordering::SeqCst);
            "BYE\n".to_string()
        }
        other => format!("ERR unknown: {other}\n"),
        }
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
            "kittwm-test-{}.sock",
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
            "kittwm-test-status-{}.sock",
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
            "kittwm-test-quit-{}.sock",
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
            "kittwm-test-dup-{}.sock",
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
        if !first.starts_with("WINDOWS ") && !first.starts_with("DISPLAYS ") && !first.starts_with("APPS ") {
            break;
        }
    }
    Ok(out)
}

fn apps_reply(limit: usize) -> String {
    use std::fmt::Write;
    let default_cmd = crate::session::launcher_command();
    let default_prog = default_cmd.split_whitespace().next().unwrap_or("xterm");
    let default_path = find_on_path(default_prog)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<not found on PATH>".to_string());
    let path_cmds = path_commands(limit);
    #[cfg(target_os = "macos")]
    let mac_apps = macos_apps(limit);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();

    let mut out = String::new();
    let _ = writeln!(out, "APPS default={default_cmd:?} resolved={default_path:?}");
    let _ = writeln!(out, "PATH_COMMANDS {}", path_cmds.len());
    for cmd in path_cmds {
        let _ = writeln!(out, "  {cmd}");
    }
    let _ = writeln!(out, "MACOS_APPS {}", mac_apps.len());
    for app in mac_apps {
        let _ = writeln!(out, "  {app}");
    }
    out.push_str("END\n");
    out
}

fn apps_json_reply(limit: usize) -> String {
    let default_cmd = crate::session::launcher_command();
    let default_prog = default_cmd.split_whitespace().next().unwrap_or("xterm");
    let default_path = find_on_path(default_prog);
    let path_cmds = path_commands(limit);
    #[cfg(target_os = "macos")]
    let mac_apps = macos_apps(limit);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();
    format!(
        "{{\"default_command\": {:?}, \"default_resolved\": {}, \"path_commands\": [{}], \"macos_apps\": [{}]}}\n",
        default_cmd,
        default_path
            .as_ref()
            .map(|p| format!("{:?}", p.display().to_string()))
            .unwrap_or_else(|| "null".to_string()),
        json_string_array(&path_cmds),
        json_string_array(&mac_apps),
    )
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.exists().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(program);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn path_commands(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(read) = std::fs::read_dir(dir) else { continue };
            for ent in read.flatten() {
                let path = ent.path();
                if !path.is_file() { continue; }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
                if name.starts_with('.') { continue; }
                out.insert(name.to_string());
                if out.len() >= limit { break; }
            }
            if out.len() >= limit { break; }
        }
    }
    out.into_iter().take(limit).collect()
}

#[cfg(target_os = "macos")]
fn macos_apps(limit: usize) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for root in ["/Applications", "/System/Applications"] {
        let Ok(read) = std::fs::read_dir(root) else { continue };
        for ent in read.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("app") { continue; }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
            out.insert(name.trim_end_matches(".app").to_string());
            if out.len() >= limit { break; }
        }
        if out.len() >= limit { break; }
    }
    out.into_iter().take(limit).collect()
}

fn json_string_array(items: &[String]) -> String {
    items
        .iter()
        .map(|s| format!("{:?}", s))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone)]
struct AppCandidate {
    kind: &'static str,
    name: String,
}

fn apps_first_reply(query: &str, launch: bool) -> String {
    let query = query.trim();
    if query.is_empty() {
        return "ERR APPS_FIRST requires a query\n".to_string();
    }
    let path_cmds = filter_candidates(path_commands(5000), Some(query), 1);
    #[cfg(target_os = "macos")]
    let mac_apps = filter_candidates(macos_apps(5000), Some(query), 1);
    #[cfg(not(target_os = "macos"))]
    let mac_apps: Vec<String> = Vec::new();
    let Some(candidate) = first_app_candidate(&path_cmds, &mac_apps) else {
        return format!("ERR no app candidates matched {query:?}\n");
    };
    if launch {
        match launch_app_candidate(&candidate) {
            Ok(pid) => format!(
                "APPS_LAUNCH_FIRST pid={} kind={} name={}\n",
                pid, candidate.kind, candidate.name
            ),
            Err(e) => format!("ERR launch {}:{}: {e}\n", candidate.kind, candidate.name),
        }
    } else {
        format!("APPS_FIRST kind={} name={}\n", candidate.kind, candidate.name)
    }
}

fn first_app_candidate(path_cmds: &[String], mac_apps: &[String]) -> Option<AppCandidate> {
    path_cmds
        .first()
        .map(|name| AppCandidate { kind: "path", name: name.clone() })
        .or_else(|| {
            mac_apps
                .first()
                .map(|name| AppCandidate { kind: "macos", name: name.clone() })
        })
}

fn launch_app_candidate(candidate: &AppCandidate) -> Result<u32> {
    let mut cmd = if candidate.kind == "macos" {
        let mut c = std::process::Command::new("open");
        c.arg("-a").arg(&candidate.name);
        c
    } else {
        std::process::Command::new(&candidate.name)
    };
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(child.id())
}

fn filter_candidates(items: Vec<String>, query: Option<&str>, limit: usize) -> Vec<String> {
    let Some(query) = query else {
        return items.into_iter().take(limit).collect();
    };
    let q = query.to_ascii_lowercase();
    items
        .into_iter()
        .filter(|item| item.to_ascii_lowercase().contains(&q))
        .take(limit)
        .collect()
}
