//! kittui-tmux
//!
//! Replace tmux's ASCII pane separators with kittui chrome. The v0.1
//! deliverable is the deterministic core: parse `tmux list-panes -F`
//! output, build the join-group of pane chromes, and produce the
//! kittui escape stream that paints the separators.
//!
//! Live integration (a `tmux` hook that re-runs this on every pane
//! geometry change) is intentionally not in scope for v0.1 — it depends
//! on a tmux server-side mechanism that varies per host install and is
//! best driven by a thin shell wrapper that reads stdout from this
//! crate's binary helpers.
//!
//! See DESIGN.md `## Future ideas` for the long-form rationale.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

mod compose;
mod parse;

pub use compose::{compose_pane_chrome, ComposeOptions, ComposeOutput};
pub use parse::{parse_list_panes, Pane, ParseError};

/// Errors produced by the high-level helpers in this crate.
#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    /// Parse failure.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// kittui facade failure.
    #[error("kittui: {0}")]
    Kittui(String),
}
