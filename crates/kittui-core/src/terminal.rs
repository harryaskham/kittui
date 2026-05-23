//! Terminal capability descriptor used by every kittui layer.

use serde::{Deserialize, Serialize};

use crate::geom::CellSize;

/// Transport hint kittui-kitty uses to wrap escape sequences.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Direct kitty graphics escape sequences.
    Direct,
    /// Tmux passthrough (`\ePtmux;...\e\\`).
    TmuxPassthrough,
    /// File-based transfer (`a=t, t=f`).
    File,
    /// Shared-memory transfer (`a=t, t=s`).
    Memory,
}

impl Transport {
    fn from_override(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "direct" => Some(Self::Direct),
            "tmux" | "tmux_passthrough" | "tmux-passthrough" => Some(Self::TmuxPassthrough),
            "file" => Some(Self::File),
            "memory" | "shm" | "shared-memory" | "shared_memory" => Some(Self::Memory),
            _ => None,
        }
    }
}

/// Compression decision reported by transport diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsCompressionMode {
    /// Compression is disabled.
    Off,
    /// Compression is forced on.
    Zlib,
    /// Compression is delegated to the adaptive selector.
    Auto,
}

impl GraphicsCompressionMode {
    fn from_env_value(value: Option<String>) -> Self {
        match value.unwrap_or_default().to_ascii_lowercase().as_str() {
            "z" | "zlib" | "deflate" => Self::Zlib,
            "auto" => Self::Auto,
            _ => Self::Off,
        }
    }
}

/// Human/debug-facing explanation of graphics transport selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransportDiagnostics {
    /// Transport selected after applying simple explicit overrides.
    pub selected_transport: Transport,
    /// Compression mode requested for kitty graphics payloads.
    pub compression_mode: GraphicsCompressionMode,
    /// Whether the environment looks like tmux or another tmux-compatible wrapper.
    pub tmux: bool,
    /// Whether the process appears remote from the terminal.
    pub remote: bool,
    /// Whether kitty graphics are believed to be available.
    pub supports_kitty: bool,
    /// Whether unicode placeholders are believed to be available.
    pub supports_unicode_placeholders: bool,
    /// Environment/config variable that forced the transport, if any.
    pub override_source: Option<String>,
    /// Human-readable reason for fallback/conservative behavior.
    pub fallback_reason: Option<String>,
}

impl TransportDiagnostics {
    /// Build diagnostics from terminal info and the current process environment.
    pub fn detect(info: &TerminalInfo) -> Self {
        Self::detect_with_env(info, |key| std::env::var(key).ok())
    }

    /// Build diagnostics from terminal info plus a caller-supplied environment
    /// lookup. This keeps the policy selector directly unit-testable and lets
    /// hosts report diagnostics for a probed/remote terminal without mutating
    /// the process environment.
    pub fn detect_with_env<F>(info: &TerminalInfo, env: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let tmux = env("TMUX").is_some()
            || env("TERM_PROGRAM")
                .map(|v| v.to_ascii_lowercase().contains("tmux"))
                .unwrap_or(false);
        let remote = match env("KITTUI_REMOTE").as_deref() {
            Some("1") | Some("true") | Some("yes") => true,
            Some("0") | Some("false") | Some("no") => false,
            _ => env("SSH_CONNECTION").is_some() || env("SSH_CLIENT").is_some(),
        };
        let compression_mode =
            GraphicsCompressionMode::from_env_value(env("KITTUI_KITTY_COMPRESSION"));
        let override_raw = env("KITTUI_TRANSPORT");
        let selected_transport = override_raw
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("auto"))
            .and_then(Transport::from_override)
            .unwrap_or(info.transport);
        let override_source = override_raw
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("auto") && Transport::from_override(v).is_some())
            .map(|_| "KITTUI_TRANSPORT".to_string());
        let fallback_reason = if !info.supports_kitty {
            Some("kitty graphics unsupported; use text/pure-terminal fallback".to_string())
        } else if tmux && matches!(selected_transport, Transport::TmuxPassthrough) {
            Some(
                "tmux detected; high-rate kittwm surfaces should prefer pure-terminal fallback unless graphics is forced"
                    .to_string(),
            )
        } else if remote && matches!(selected_transport, Transport::File | Transport::Memory) {
            Some(
                "remote terminal detected; file/shared-memory transports may be unreadable by the terminal"
                    .to_string(),
            )
        } else {
            None
        };

        Self {
            selected_transport,
            compression_mode,
            tmux,
            remote,
            supports_kitty: info.supports_kitty,
            supports_unicode_placeholders: info.supports_unicode_placeholders,
            override_source,
            fallback_reason,
        }
    }
}

/// What kittui knows about the active terminal. Hosts can either let kittui
/// probe and fill this in or supply it explicitly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalInfo {
    /// Number of columns in the terminal, if known.
    pub columns: Option<u16>,
    /// Number of rows in the terminal, if known.
    pub rows: Option<u16>,
    /// Pixels per cell, if known. Defaults to a typical monospace metric.
    pub cell_size: CellSize,
    /// Whether the terminal supports the kitty graphics protocol.
    pub supports_kitty: bool,
    /// Whether the terminal supports kitty's unicode placeholders.
    pub supports_unicode_placeholders: bool,
    /// Selected transport.
    pub transport: Transport,
}

impl TerminalInfo {
    /// Construct a sane default: assume kitty graphics + unicode placeholders
    /// + direct transport with a standard 8x16 cell.
    pub fn default_kitty() -> Self {
        Self {
            columns: None,
            rows: None,
            cell_size: CellSize::default(),
            supports_kitty: true,
            supports_unicode_placeholders: true,
            transport: Transport::Direct,
        }
    }

    /// Detect transport and graphics-protocol capabilities from environment
    /// variables that terminals are conventionally expected to expose:
    ///
    /// | Variable | Effect |
    /// |---|---|
    /// | `TMUX` set | `Transport::TmuxPassthrough` |
    /// | `KITTY_WINDOW_ID` or `KITTY_PUBLIC_KEY` set | `supports_kitty=true`, `supports_unicode_placeholders=true` |
    /// | `TERM_PROGRAM=ghostty` or `iTerm.app` or `WezTerm` | `supports_kitty=true` |
    /// | `WT_SESSION` set (Windows Terminal) | `supports_kitty=false` |
    /// | `TERM` containing `kitty` or `xterm-kitty` | `supports_kitty=true` |
    ///
    /// Detection is intentionally optimistic: unknown terminals default to
    /// `supports_kitty=true` so the well-behaved majority just works, with
    /// hosts overriding when they know better.
    pub fn detect() -> Self {
        let mut info = Self::default_kitty();
        let env = |k: &str| std::env::var(k).ok();

        if env("TMUX").is_some() {
            info.transport = Transport::TmuxPassthrough;
        }
        if let Some(value) = env("KITTUI_TRANSPORT") {
            if !value.eq_ignore_ascii_case("auto") {
                if let Some(transport) = Transport::from_override(&value) {
                    info.transport = transport;
                }
            }
        }

        // Known kitty-family terminals.
        if env("KITTY_WINDOW_ID").is_some()
            || env("KITTY_PUBLIC_KEY").is_some()
            || env("TERM").map(|t| t.contains("kitty")).unwrap_or(false)
        {
            info.supports_kitty = true;
            info.supports_unicode_placeholders = true;
        }
        if let Some(prog) = env("TERM_PROGRAM") {
            let prog_l = prog.to_ascii_lowercase();
            if prog_l.contains("ghostty")
                || prog_l.contains("iterm")
                || prog_l.contains("wezterm")
                || prog_l.contains("kitty")
            {
                info.supports_kitty = true;
                info.supports_unicode_placeholders = true;
            }
        }
        // Windows Terminal (no kitty support today).
        if env("WT_SESSION").is_some() && env("TERM_PROGRAM").is_none() {
            info.supports_kitty = false;
            info.supports_unicode_placeholders = false;
        }
        info
    }

    /// Construct a host-supplied override. Library users that already know
    /// the terminal capabilities (because Pi or another wrapper has already
    /// probed) can build this directly and skip kittui's own probing.
    pub fn override_with(
        columns: Option<u16>,
        rows: Option<u16>,
        cell_size: CellSize,
        supports_kitty: bool,
        supports_unicode_placeholders: bool,
        transport: Transport,
    ) -> Self {
        Self {
            columns,
            rows,
            cell_size,
            supports_kitty,
            supports_unicode_placeholders,
            transport,
        }
    }
}

impl Default for TerminalInfo {
    fn default() -> Self {
        Self::default_kitty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce()>(pairs: &[(&str, Option<&str>)], f: F) {
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let saved: Vec<(String, Option<String>)> = pairs
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in pairs {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(&k, val),
                None => std::env::remove_var(&k),
            }
        }
        if let Err(p) = result {
            std::panic::resume_unwind(p);
        }
    }

    #[test]
    fn detect_tmux_picks_tmux_passthrough() {
        with_env(
            &[
                ("TMUX", Some("/tmp/x,123,0")),
                ("KITTUI_TRANSPORT", None),
                ("KITTY_WINDOW_ID", None),
                ("KITTY_PUBLIC_KEY", None),
                ("TERM_PROGRAM", None),
                ("WT_SESSION", None),
                ("TERM", Some("xterm-256color")),
            ],
            || {
                let info = TerminalInfo::detect();
                assert_eq!(info.transport, Transport::TmuxPassthrough);
                assert!(info.supports_kitty);
            },
        );
    }

    #[test]
    fn detect_transport_override_picks_file_transport() {
        with_env(
            &[
                ("TMUX", None),
                ("KITTUI_TRANSPORT", Some("file")),
                ("KITTY_WINDOW_ID", None),
                ("KITTY_PUBLIC_KEY", None),
                ("TERM_PROGRAM", None),
                ("WT_SESSION", None),
                ("TERM", Some("xterm-256color")),
            ],
            || {
                let info = TerminalInfo::detect();
                assert_eq!(info.transport, Transport::File);
            },
        );
    }

    #[test]
    fn detect_kitty_window_id_marks_kitty_supported() {
        with_env(
            &[
                ("TMUX", None),
                ("KITTUI_TRANSPORT", None),
                ("KITTY_WINDOW_ID", Some("1")),
                ("KITTY_PUBLIC_KEY", None),
                ("TERM_PROGRAM", None),
                ("WT_SESSION", None),
                ("TERM", Some("xterm-256color")),
            ],
            || {
                let info = TerminalInfo::detect();
                assert_eq!(info.transport, Transport::Direct);
                assert!(info.supports_kitty);
                assert!(info.supports_unicode_placeholders);
            },
        );
    }

    #[test]
    fn detect_ghostty_term_program_marks_kitty_supported() {
        with_env(
            &[
                ("TMUX", None),
                ("KITTUI_TRANSPORT", None),
                ("KITTY_WINDOW_ID", None),
                ("KITTY_PUBLIC_KEY", None),
                ("TERM_PROGRAM", Some("ghostty")),
                ("WT_SESSION", None),
                ("TERM", Some("xterm-256color")),
            ],
            || {
                let info = TerminalInfo::detect();
                assert!(info.supports_kitty);
            },
        );
    }

    #[test]
    fn transport_diagnostics_report_override_remote_and_compression() {
        let info = TerminalInfo::default_kitty();
        let diag = TransportDiagnostics::detect_with_env(&info, |key| match key {
            "KITTUI_TRANSPORT" => Some("memory".to_string()),
            "KITTUI_KITTY_COMPRESSION" => Some("auto".to_string()),
            "SSH_CONNECTION" => Some("client server".to_string()),
            _ => None,
        });
        assert_eq!(diag.selected_transport, Transport::Memory);
        assert_eq!(diag.compression_mode, GraphicsCompressionMode::Auto);
        assert!(diag.remote);
        assert_eq!(diag.override_source.as_deref(), Some("KITTUI_TRANSPORT"));
        assert!(diag
            .fallback_reason
            .as_deref()
            .unwrap()
            .contains("remote terminal"));
    }

    #[test]
    fn transport_diagnostics_report_tmux_fallback_reason() {
        let mut info = TerminalInfo::default_kitty();
        info.transport = Transport::TmuxPassthrough;
        let diag = TransportDiagnostics::detect_with_env(&info, |key| match key {
            "TMUX" => Some("/tmp/tmux,123,0".to_string()),
            _ => None,
        });
        assert!(diag.tmux);
        assert_eq!(diag.selected_transport, Transport::TmuxPassthrough);
        assert!(diag
            .fallback_reason
            .as_deref()
            .unwrap()
            .contains("high-rate kittwm surfaces"));
    }

    #[test]
    fn detect_windows_terminal_marks_kitty_unsupported() {
        with_env(
            &[
                ("TMUX", None),
                ("KITTUI_TRANSPORT", None),
                ("KITTY_WINDOW_ID", None),
                ("KITTY_PUBLIC_KEY", None),
                ("TERM_PROGRAM", None),
                ("WT_SESSION", Some("abc")),
                ("TERM", Some("xterm-256color")),
            ],
            || {
                let info = TerminalInfo::detect();
                assert!(!info.supports_kitty);
                assert!(!info.supports_unicode_placeholders);
            },
        );
    }
}
