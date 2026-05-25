use std::env;
use std::process::ExitCode;

use kittwm_sdk::{Kittwm, SurfaceSpec, WindowSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Auto,
    Terminal,
    App,
    Browser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchArgs {
    replace: bool,
    backend: Backend,
    title: Option<String>,
    dry_run: bool,
    status: bool,
    query: String,
}

impl LaunchArgs {
    fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut replace = false;
        let mut backend = Backend::Auto;
        let mut title = None;
        let mut dry_run = false;
        let mut status = false;
        let mut query = None;
        let mut iter = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => return Err(help_text()),
                "--replace" => replace = true,
                "--new-window" => replace = false,
                "--dry-run" => dry_run = true,
                "--status" => status = true,
                "--terminal" => backend = Backend::Terminal,
                "--app" => backend = Backend::App,
                "--browser" => backend = Backend::Browser,
                "--backend" => {
                    let value = iter.next().ok_or_else(|| {
                        "--backend requires auto|terminal|app|browser".to_string()
                    })?;
                    backend = parse_backend(&value)?;
                }
                "--title" => {
                    title = Some(
                        iter.next()
                            .ok_or_else(|| "--title requires a value".to_string())?,
                    );
                }
                "--" => {
                    let rest = iter.collect::<Vec<_>>();
                    if !rest.is_empty() {
                        query = Some(shell_words(&rest));
                    }
                    break;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other}\n\n{}", help_text()));
                }
                other => {
                    let mut rest = vec![other.to_string()];
                    rest.extend(iter);
                    query = Some(shell_words(&rest));
                    break;
                }
            }
        }
        let query = query.ok_or_else(|| format!("missing launch query\n\n{}", help_text()))?;
        Ok(Self {
            replace,
            backend,
            title,
            dry_run,
            status,
            query,
        })
    }

    fn effective_backend(&self) -> Backend {
        match self.backend {
            Backend::Auto if looks_like_browser_target(&self.query) => Backend::Browser,
            Backend::Auto if looks_like_shell_command(&self.query) => Backend::Terminal,
            Backend::Auto => Backend::App,
            other => other,
        }
    }
}

fn parse_backend(value: &str) -> Result<Backend, String> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(Backend::Auto),
        "terminal" | "term" | "pty" => Ok(Backend::Terminal),
        "app" | "native" => Ok(Backend::App),
        "browser" | "web" => Ok(Backend::Browser),
        _ => Err(format!(
            "unknown backend {value}; expected auto|terminal|app|browser"
        )),
    }
}

fn looks_like_shell_command(query: &str) -> bool {
    query.contains(' ')
        || query.contains('/')
        || query.starts_with("./")
        || query.starts_with("~/")
        || query.starts_with('$')
}

fn looks_like_browser_target(query: &str) -> bool {
    let q = query.to_ascii_lowercase();
    q.starts_with("http://")
        || q.starts_with("https://")
        || q.starts_with("data:")
        || q.starts_with("about:")
}

fn shell_words(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.' | ':'))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn help_text() -> String {
    "kittwm-launch — SDK app/surface launcher for kittwm\n\n\
Usage:\n  kittwm-launch [--replace|--new-window] [--dry-run|--status] [--backend auto|terminal|app|browser] [--title TITLE] QUERY\n  kittwm-launch --terminal [--title TITLE] -- PROGRAM [ARGS...]\n\n\
Backends:\n  auto      choose browser for URLs, terminal for shell-like commands, app discovery otherwise\n  terminal  spawn a PTY terminal surface through kittwm-sdk\n  app      ask kittwm app discovery to launch the first matching app\n  browser  launch the first-party kittwm-browser app in a PTY surface\n"
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchPlan {
    backend: Backend,
    command: String,
    surface: Option<SurfaceSpec>,
    status: String,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Terminal => "terminal",
            Backend::App => "app",
            Backend::Browser => "browser",
        }
    }
}

fn surface_spec_for_backend(backend: Backend, args: &LaunchArgs) -> Option<SurfaceSpec> {
    match backend {
        Backend::Terminal => {
            let mut spec = SurfaceSpec::terminal(args.query.clone());
            if let Some(title) = args.title.clone() {
                spec = spec.titled(title);
            }
            Some(spec)
        }
        Backend::Browser => {
            let mut spec = SurfaceSpec::browser(browser_target_from_query(&args.query));
            if let Some(title) = args.title.clone() {
                spec = spec.titled(title);
            }
            Some(spec)
        }
        Backend::App | Backend::Auto => None,
    }
}

fn browser_target_from_query(query: &str) -> String {
    query
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .map(|inner| inner.replace("'\\''", "'"))
        .unwrap_or_else(|| query.to_string())
}

fn build_launch_plan(args: &LaunchArgs) -> LaunchPlan {
    let backend = args.effective_backend();
    let mode = if args.replace {
        "replace"
    } else {
        "new-window"
    };
    let surface = surface_spec_for_backend(backend, args);
    let command = match &surface {
        Some(spec) => format!(
            "SPAWN_PTY {}",
            spec.native_pty_command()
                .expect("terminal/browser surface specs are supported")
        ),
        None => format!("APPS_LAUNCH_FIRST {}", args.query),
    };
    let status = format!(
        "kittwm-launch: backend={} mode={} title={} query={}",
        backend.label(),
        mode,
        args.title.as_deref().unwrap_or("-"),
        args.query
    );
    LaunchPlan {
        backend,
        command,
        surface,
        status,
    }
}

fn run(args: LaunchArgs) -> Result<String, String> {
    let plan = build_launch_plan(&args);
    if args.dry_run {
        return Ok(format!("{}\n{}\n", plan.status, plan.command));
    }
    let wm = Kittwm::connect_from_env().map_err(|err| {
        format!(
            "connect to kittwm: {err}. Set KITTWM_SOCKET/KITTWM_SOCK or run inside a kittwm pane"
        )
    })?;
    let reply = match plan.backend {
        Backend::Terminal | Backend::Browser => {
            let spec = plan
                .surface
                .as_ref()
                .expect("terminal/browser plan carries a typed SDK surface");
            if args.replace {
                wm.replace_current(&WindowSpec {
                    title: spec.title.clone(),
                    command: spec.native_pty_command().map_err(|err| {
                        format!("prepare {} surface: {err}", plan.backend.label())
                    })?,
                })
                .map_err(|err| format!("replace {} surface: {err}", plan.backend.label()))?
            } else {
                wm.spawn_surface(spec)
                    .map(|spawn| spawn.reply)
                    .map_err(|err| format!("spawn {} surface: {err}", plan.backend.label()))?
            }
        }
        Backend::App | Backend::Auto => {
            let reply = if args.status {
                let candidate = wm
                    .app_first(&args.query)
                    .map_err(|err| format!("find app via discovery: {err}"))?;
                let launch = wm
                    .app_launch_first(&args.query)
                    .map_err(|err| format!("launch app via discovery: {err}"))?;
                format!(
                    "APPS_LAUNCH_FIRST pid={} kind={} name={}\nAPPS_FIRST kind={} name={}\n",
                    launch.pid,
                    launch.candidate.kind,
                    launch.candidate.name,
                    candidate.kind,
                    candidate.name
                )
            } else {
                let launch = wm
                    .app_launch_first(&args.query)
                    .map_err(|err| format!("launch app via discovery: {err}"))?;
                format!(
                    "APPS_LAUNCH_FIRST pid={} kind={} name={}\n",
                    launch.pid, launch.candidate.kind, launch.candidate.name
                )
            };
            if args.replace {
                if let Some(current) = wm.current_window_from_env() {
                    let _ = wm.surface(current.id).close();
                }
            }
            reply
        }
    };
    if args.status {
        Ok(format!("{}\n{}", plan.status, reply))
    } else {
        Ok(reply)
    }
}

fn main() -> ExitCode {
    let parsed = match LaunchArgs::parse_from(env::args().skip(1)) {
        Ok(args) => args,
        Err(message) if message.starts_with("kittwm-launch") => {
            print!("{message}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(parsed) {
        Ok(reply) => {
            print!("{reply}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("kittwm-launch: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terminal_command() {
        let args = LaunchArgs::parse_from([
            "--terminal",
            "--title",
            "logs",
            "--",
            "tail",
            "-f",
            "/tmp/app log.txt",
        ])
        .unwrap();
        assert_eq!(args.backend, Backend::Terminal);
        assert_eq!(args.title.as_deref(), Some("logs"));
        assert_eq!(args.query, "tail -f '/tmp/app log.txt'");
    }

    #[test]
    fn parses_backend_aliases() {
        let args = LaunchArgs::parse_from(["--backend", "term", "htop"]).unwrap();
        assert_eq!(args.backend, Backend::Terminal);
    }

    #[test]
    fn auto_detects_shell_like_commands_and_browser_urls() {
        let args = LaunchArgs::parse_from(["echo hello"]).unwrap();
        assert_eq!(args.effective_backend(), Backend::Terminal);
        let args = LaunchArgs::parse_from(["https://example.com"]).unwrap();
        assert_eq!(args.effective_backend(), Backend::Browser);
        let args = LaunchArgs::parse_from(["firefox"]).unwrap();
        assert_eq!(args.effective_backend(), Backend::App);
    }

    #[test]
    fn browser_target_strips_shell_word_quotes_before_sdk_surface_quote() {
        assert_eq!(
            browser_target_from_query("'https://example.com/a%20b'"),
            "https://example.com/a%20b"
        );
        assert_eq!(
            browser_target_from_query("'https://example.com/it'\\''s'"),
            "https://example.com/it's"
        );
    }

    #[test]
    fn builds_launch_plans_for_terminal_browser_and_app() {
        let terminal = LaunchArgs::parse_from(["--terminal", "--", "echo", "hi there"]).unwrap();
        let plan = build_launch_plan(&terminal);
        assert_eq!(plan.backend, Backend::Terminal);
        assert_eq!(plan.command, "SPAWN_PTY echo 'hi there'");
        assert!(plan.status.contains("backend=terminal"));

        let browser = LaunchArgs::parse_from(["--browser", "https://example.com/a%20b"]).unwrap();
        let plan = build_launch_plan(&browser);
        assert_eq!(plan.backend, Backend::Browser);
        assert_eq!(
            plan.command,
            "SPAWN_PTY kittwm-browser 'https://example.com/a%20b'"
        );
        assert_eq!(plan.surface.unwrap().kind, kittwm_sdk::SurfaceKind::Browser);

        let app = LaunchArgs::parse_from(["firefox"]).unwrap();
        let plan = build_launch_plan(&app);
        assert_eq!(plan.backend, Backend::App);
        assert_eq!(plan.command, "APPS_LAUNCH_FIRST firefox");
    }

    #[test]
    fn dry_run_returns_status_and_command_without_socket() {
        let args =
            LaunchArgs::parse_from(["--dry-run", "--browser", "https://example.com"]).unwrap();
        let out = run(args).unwrap();
        assert!(out.contains("backend=browser"));
        assert!(out.contains("SPAWN_PTY kittwm-browser https://example.com"));
    }

    #[test]
    fn imports_surface_kind_for_sdk_surface_vocab() {
        assert_eq!(
            kittwm_sdk::SurfaceKind::Terminal,
            kittwm_sdk::SurfaceKind::Terminal
        );
    }
}
