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

        // Known kitty-family terminals.
        if env("KITTY_WINDOW_ID").is_some()
            || env("KITTY_PUBLIC_KEY").is_some()
            || env("TERM")
                .map(|t| t.contains("kitty"))
                .unwrap_or(false)
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
    fn detect_kitty_window_id_marks_kitty_supported() {
        with_env(
            &[
                ("TMUX", None),
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
    fn detect_windows_terminal_marks_kitty_unsupported() {
        with_env(
            &[
                ("TMUX", None),
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
