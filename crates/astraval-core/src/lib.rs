// SPDX-License-Identifier: Apache-2.0

//! Client logic for Astraval, built on top of the Perforce wire protocol.
//!
//! Where [`astraval_proto`] knows how to talk to a server, this crate knows
//! what to say: managing local workspaces, syncing files, taking and releasing
//! exclusive locks, and merging changes. It is the reusable core shared by the
//! `astraval` CLI and any third-party tool that wants Astraval's client
//! behavior without reimplementing it.
//!
//! This crate is currently a scaffold. The workspace, sync, lock, and merge
//! engines arrive in later milestones. Three pieces are already in place: the
//! [`error`] module, the shared error spine every client operation returns, whose
//! [`ErrorClass`] the `astraval` binary maps to Perforce-compatible exit codes;
//! the [`config`] module, which resolves Perforce-compatible connection and
//! identity settings into the single [`config::ResolvedConfig`] the CLI reads; and
//! the [`output`] module, which renders command results in `p4`'s human, `-ztag`,
//! and `-G` (marshalled) modes from one tagged-record model.

pub mod config;
mod error;
pub mod output;

#[doc(inline)]
pub use error::{Error, ErrorClass};

use astraval_proto::TARGET_PROTOCOL_LEVEL;

/// The result type returned by fallible Astraval client operations.
///
/// A thin alias over [`std::result::Result`] fixing the error half to this
/// crate's [`Error`], so client APIs read as `Result<T>` and callers get one
/// consistent error type to handle.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Returns the Perforce protocol level the client currently targets.
///
/// This forwards the constant from [`astraval_proto`] so callers can read the
/// negotiated level through the core crate without depending on the protocol
/// crate directly. It is a scaffold seam; richer client APIs replace it later.
///
/// # Examples
/// ```
/// // The targeted client API level Astraval negotiates first.
/// assert_eq!(astraval_core::target_protocol_level(), 100);
/// ```
#[must_use]
pub fn target_protocol_level() -> u32 {
    TARGET_PROTOCOL_LEVEL
}
