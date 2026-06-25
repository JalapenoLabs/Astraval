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
//! engines arrive in later milestones.

use astraval_proto::TARGET_PROTOCOL_LEVEL;

/// Returns the Perforce protocol level the client currently targets.
///
/// This forwards the constant from [`astraval_proto`] so callers can read the
/// negotiated level through the core crate without depending on the protocol
/// crate directly. It is a scaffold seam; richer client APIs replace it later.
///
/// # Examples
/// ```
/// assert_eq!(astraval_core::target_protocol_level(), 0);
/// ```
#[must_use]
pub fn target_protocol_level() -> u32 {
    TARGET_PROTOCOL_LEVEL
}
