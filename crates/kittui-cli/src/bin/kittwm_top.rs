//! `kittwm-top` — SDK-backed kittwm process viewer.
//!
//! Outside kittwm this is a regular terminal command that connects to the
//! current/default kittwm socket. Inside kittwm it is just another first-party
//! SDK app: launch it through `SurfaceSpec::terminal("kittwm-top")` or
//! `kittwm spawn kittwm-top` and it renders in the hosted terminal surface
//! without any WM hardcoding.

use std::fmt::Write as FmtWrite;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use kittwm_sdk::{display_to_socket_path, Kittwm, KittwmProcessInfo, KittwmProcessSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TopArgs {
    json: bool,
    socket: Option<String>,
    display: Option<String>,
}

impl TopArgs {
    fn parse_from<I, S>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut json = false;
        let mut socket = None;
        let mut display = None;
        let mut iter = args.into_iter().map(Into::into);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--socket" => {
                    socket = Some(
                        iter.next()
                            .ok_or_else(|| anyhow!("--socket requires a path"))?,
                    )
                }
                "--display" => {
                    display = Some(
                        iter.next()
                            .ok_or_else(|| anyhow!("--display requires a display token"))?,
                    )
                }
                "-h" | "--help" => return Err(anyhow!(help_text())),
                other => return Err(anyhow!(unknown_option_error(other))),
            }
        }
        Ok(Self {
            json,
            socket,
            display,
        })
    }
}

fn unknown_option_error(option: &str) -> String {
    let help = help_text();
    let mut out =
        String::with_capacity("unknown kittwm-top option \n\n".len() + option.len() + help.len());
    out.push_str("unknown kittwm-top option ");
    out.push_str(option);
    out.push_str("\n\n");
    out.push_str(help);
    out
}

fn help_text() -> &'static str {
    "kittwm-top — SDK-backed process viewer for kittwm\n\n\
Usage:\n  kittwm-top [--json] [--socket PATH|--display DISPLAY]\n\n\
Shows panes/processes for the current kittwm session, or the default :0\n\
session when no KITTWM_* environment is present. Inside kittwm it runs as a\n\
normal hosted first-party terminal surface; outside kittwm it prints to the\n\
current terminal.\n\n\
Options:\n  --json             Print process snapshot JSON\n  --socket PATH      Connect to a specific kittwm socket\n  --display DISPLAY  Resolve DISPLAY to its kittwm socket path\n  -h, --help         Show this help text\n"
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("kittwm-top: {err}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let args = match TopArgs::parse_from(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(err) if err.to_string().starts_with("kittwm-top —") => {
            print!("{}", err);
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let client = connect_for_args(&args);
    let snapshot = client.process_snapshot()?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        print!("{}", render_process_snapshot(&snapshot));
    }
    Ok(())
}

fn connect_for_args(args: &TopArgs) -> Kittwm {
    if let Some(socket) = &args.socket {
        return Kittwm::connect_path(socket);
    }
    if let Some(display) = &args.display {
        return Kittwm::connect_path(display_to_socket_path(display));
    }
    Kittwm::connect_from_env()
        .unwrap_or_else(|_| Kittwm::connect_path(display_to_socket_path(":0")))
}

fn render_process_snapshot(snapshot: &KittwmProcessSnapshot) -> String {
    let mut out = String::new();
    out.push_str("kittwm-top — session processes\n");
    let _ = writeln!(out, "socket: {}", snapshot.socket.display());
    out.push_str("panes: ");
    let _ = write!(out, "{}", snapshot.processes.len());
    out.push_str("\n\n");
    out.push_str("FOC WINDOW     PID     PPID    CPU%   RSS(KiB) STATE NAME/TITLE COMMAND\n");
    out.push_str("─── ────────── ─────── ─────── ────── ──────── ───── ────────── ───────\n");
    if snapshot.processes.is_empty() {
        out.push_str(" -  <none>     -       -       -      -        -     no panes reported\n");
        return out;
    }
    for process in &snapshot.processes {
        out.push_str(&render_process_row(process));
        out.push('\n');
    }
    out
}

fn render_process_row(process: &KittwmProcessInfo) -> String {
    let window = clip(&process.window, 10);
    let name = clip(
        process
            .process_name
            .as_deref()
            .unwrap_or(process.title.as_str()),
        10,
    );
    let mut out = String::with_capacity(
        64usize
            .saturating_add(window.len())
            .saturating_add(name.len())
            .saturating_add(process.command.as_deref().unwrap_or("-").len()),
    );
    let _ = write!(
        out,
        " {}  {:<10} ",
        if process.focused { '*' } else { '-' },
        window
    );
    push_top_optional_u32(&mut out, process.pid, 7, false);
    out.push(' ');
    push_top_optional_u32(&mut out, process.ppid, 7, false);
    out.push(' ');
    push_top_optional_f32(&mut out, process.cpu_percent, 6, true);
    out.push(' ');
    push_top_optional_u64(&mut out, process.rss_kib, 8, true);
    let _ = write!(
        out,
        " {:<5} {:<10} {}",
        process.state.as_deref().unwrap_or("-"),
        name,
        process.command.as_deref().unwrap_or("-"),
    );
    out
}

fn push_top_optional_u32(out: &mut String, value: Option<u32>, width: usize, right: bool) {
    if let Some(value) = value {
        push_top_padded(out, &value.to_string(), width, right);
    } else {
        push_top_padded(out, "-", width, right);
    }
}

fn push_top_optional_u64(out: &mut String, value: Option<u64>, width: usize, right: bool) {
    if let Some(value) = value {
        push_top_padded(out, &value.to_string(), width, right);
    } else {
        push_top_padded(out, "-", width, right);
    }
}

fn push_top_optional_f32(out: &mut String, value: Option<f32>, width: usize, right: bool) {
    if let Some(value) = value {
        let mut number = String::with_capacity(8);
        let _ = write!(number, "{value:.1}");
        push_top_padded(out, &number, width, right);
    } else {
        push_top_padded(out, "-", width, right);
    }
}

fn push_top_padded(out: &mut String, value: &str, width: usize, right: bool) {
    let len = value.chars().count();
    if right && len < width {
        for _ in 0..(width - len) {
            out.push(' ');
        }
    }
    out.push_str(value);
    if !right && len < width {
        for _ in 0..(width - len) {
            out.push(' ');
        }
    }
}

fn clip(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let mut out = String::new();
    for _ in 0..max {
        let Some(ch) = chars.next() else {
            return value.to_string();
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_top_args() {
        let args = TopArgs::parse_from(["--json", "--display", ":7"]).unwrap();
        assert!(args.json);
        assert_eq!(args.display.as_deref(), Some(":7"));
    }

    #[test]
    fn unknown_option_error_includes_help() {
        let err = TopArgs::parse_from(["--wat"]).unwrap_err().to_string();
        assert!(err.starts_with("unknown kittwm-top option --wat"), "{err}");
        assert!(
            err.contains("kittwm-top — SDK-backed process viewer"),
            "{err}"
        );
    }

    #[test]
    fn help_text_lists_operator_options() {
        let help = help_text();
        assert!(help.contains("Options:"), "{help}");
        assert!(help.contains("--json"), "{help}");
        assert!(help.contains("--socket PATH"), "{help}");
        assert!(help.contains("--display DISPLAY"), "{help}");
        assert!(help.contains("-h, --help"), "{help}");
    }

    #[test]
    fn renders_process_snapshot_table() {
        let snapshot = KittwmProcessSnapshot {
            socket: PathBuf::from("/tmp/kittui-wm-0.sock"),
            processes: vec![KittwmProcessInfo {
                window: "native-1".to_string(),
                title: "shell".to_string(),
                focused: true,
                pid: Some(42),
                ppid: Some(1),
                command: Some("zsh -l".to_string()),
                state: Some("S".to_string()),
                rss_kib: Some(2048),
                cpu_percent: Some(1.5),
                process_name: Some("zsh".to_string()),
            }],
        };
        let row = render_process_row(&snapshot.processes[0]);
        assert_eq!(
            row,
            " *  native-1   42      1          1.5     2048 S     zsh        zsh -l"
        );
        assert!(row.capacity() >= row.len());

        let rendered = render_process_snapshot(&snapshot);
        assert!(rendered.contains("kittwm-top"), "{rendered}");
        assert!(
            rendered.contains("socket: /tmp/kittui-wm-0.sock"),
            "{rendered}"
        );
        assert!(rendered.contains("panes: 1"), "{rendered}");
        assert!(rendered.contains("native-1"), "{rendered}");
        assert!(rendered.contains("zsh -l"), "{rendered}");
        assert!(rendered.contains("*"), "{rendered}");
    }
}
