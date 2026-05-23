use std::env;
use std::process::ExitCode;

use kittwm_sdk::{Kittwm, SurfaceSpec, WindowSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalArgs {
    replace: bool,
    title: Option<String>,
    command: String,
}

impl TerminalArgs {
    fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut replace = false;
        let mut title = None;
        let mut command = None;
        let mut iter = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => return Err(help_text()),
                "--replace" => replace = true,
                "--new-window" => replace = false,
                "--title" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--title requires a value".to_string())?;
                    title = Some(value);
                }
                "--command" | "-c" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--command requires a value".to_string())?;
                    command = Some(value);
                }
                "--" => {
                    let rest = iter.collect::<Vec<_>>();
                    if !rest.is_empty() {
                        command = Some(shell_words(&rest));
                    }
                    break;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other}\n\n{}", help_text()));
                }
                other => {
                    let mut rest = vec![other.to_string()];
                    rest.extend(iter);
                    command = Some(shell_words(&rest));
                    break;
                }
            }
        }
        Ok(Self {
            replace,
            title,
            command: command.unwrap_or_else(default_terminal_command),
        })
    }
}

fn default_terminal_command() -> String {
    env::var("KITTWM_TERMINAL_CMD")
        .or_else(|_| env::var("SHELL").map(|shell| format!("{shell} -l")))
        .unwrap_or_else(|_| "/bin/sh -l".to_string())
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
    "kittwm-terminal — first-party terminal client for kittwm\n\n\
Usage:\n  kittwm-terminal [--replace|--new-window] [--title TITLE] [--command CMD]\n  kittwm-terminal [--replace|--new-window] [--title TITLE] -- PROGRAM [ARGS...]\n\n\
The v0 skeleton connects through KITTWM_SOCKET/KITTWM_DISPLAY using kittwm-sdk\n\
and asks the running kittwm instance to spawn a native terminal surface.\n"
        .to_string()
}

fn run(args: TerminalArgs) -> Result<String, String> {
    let wm = Kittwm::connect_from_env().map_err(|err| format!("connect to kittwm: {err}"))?;
    if args.replace {
        wm.replace_current(&WindowSpec {
            title: args.title,
            command: args.command,
        })
        .map_err(|err| format!("replace current terminal: {err}"))
    } else {
        let mut spec = SurfaceSpec::terminal(args.command);
        if let Some(title) = args.title {
            spec = spec.titled(title);
        }
        wm.spawn_surface(&spec)
            .map(|spawn| spawn.reply)
            .map_err(|err| format!("spawn terminal surface: {err}"))
    }
}

fn main() -> ExitCode {
    let parsed = match TerminalArgs::parse_from(env::args().skip(1)) {
        Ok(args) => args,
        Err(message) if message.starts_with("kittwm-terminal") => {
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
            eprintln!("kittwm-terminal: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_replace_title_and_command() {
        let args =
            TerminalArgs::parse_from(["--replace", "--title", "dev shell", "--command", "zsh -l"])
                .unwrap();
        assert_eq!(
            args,
            TerminalArgs {
                replace: true,
                title: Some("dev shell".to_string()),
                command: "zsh -l".to_string(),
            }
        );
    }

    #[test]
    fn parses_program_after_separator() {
        let args = TerminalArgs::parse_from(["--", "echo", "hello world"]).unwrap();
        assert_eq!(args.command, "echo 'hello world'");
    }

    #[test]
    fn help_is_success_path() {
        let err = TerminalArgs::parse_from(["--help"]).unwrap_err();
        assert!(err.starts_with("kittwm-terminal"));
    }
}
