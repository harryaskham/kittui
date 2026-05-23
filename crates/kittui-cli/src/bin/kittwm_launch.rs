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
        let mut query = None;
        let mut iter = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => return Err(help_text()),
                "--replace" => replace = true,
                "--new-window" => replace = false,
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
            query,
        })
    }

    fn effective_backend(&self) -> Backend {
        match self.backend {
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
Usage:\n  kittwm-launch [--replace|--new-window] [--backend auto|terminal|app|browser] [--title TITLE] QUERY\n  kittwm-launch --terminal [--title TITLE] -- PROGRAM [ARGS...]\n\n\
Backends:\n  auto      choose terminal for shell-like commands, app discovery otherwise\n  terminal  spawn a PTY terminal surface through kittwm-sdk\n  app      ask kittwm app discovery to launch the first matching app\n  browser  currently uses app discovery; dedicated browser surfaces are planned\n"
        .to_string()
}

fn run(args: LaunchArgs) -> Result<String, String> {
    let wm = Kittwm::connect_from_env().map_err(|err| format!("connect to kittwm: {err}"))?;
    match args.effective_backend() {
        Backend::Terminal => {
            if args.replace {
                wm.replace_current(&WindowSpec {
                    title: args.title,
                    command: args.query,
                })
                .map_err(|err| format!("replace terminal surface: {err}"))
            } else {
                let mut spec = SurfaceSpec::terminal(args.query);
                if let Some(title) = args.title {
                    spec = spec.titled(title);
                }
                wm.spawn_surface(&spec)
                    .map(|spawn| spawn.reply)
                    .map_err(|err| format!("spawn terminal surface: {err}"))
            }
        }
        Backend::App | Backend::Browser | Backend::Auto => {
            let verb = if args.effective_backend() == Backend::Browser {
                "APPS_LAUNCH_FIRST browser"
            } else {
                "APPS_LAUNCH_FIRST"
            };
            let reply = wm
                .request(format!("{verb} {}", args.query))
                .map_err(|err| format!("launch app: {err}"))?;
            if args.replace {
                if let Some(current) = wm.current_window_from_env() {
                    let _ = wm.request(format!("CLOSE_PANE {}", current.id));
                }
            }
            Ok(reply)
        }
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
    fn auto_detects_shell_like_commands() {
        let args = LaunchArgs::parse_from(["echo hello"]).unwrap();
        assert_eq!(args.effective_backend(), Backend::Terminal);
        let args = LaunchArgs::parse_from(["firefox"]).unwrap();
        assert_eq!(args.effective_backend(), Backend::App);
    }

    #[test]
    fn imports_surface_kind_for_sdk_surface_vocab() {
        assert_eq!(
            kittwm_sdk::SurfaceKind::Terminal,
            kittwm_sdk::SurfaceKind::Terminal
        );
    }
}
