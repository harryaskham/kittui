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
