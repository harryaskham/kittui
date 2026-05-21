//! Library facade for `kittui-cli`. Hosts code shared between the
//! `kittui` and `kitwm` binaries and the example programs.
//!
//! Currently exposes the `session` module which owns the kittui-wm
//! render loop, terminal raw-mode handling, signal restoration, and
//! the file-based debug logger.

pub mod session;
pub mod daemon;
