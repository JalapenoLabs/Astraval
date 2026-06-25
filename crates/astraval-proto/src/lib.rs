// SPDX-License-Identifier: Apache-2.0

//! The Perforce-compatible wire protocol spoken by Astraval.
//!
//! Astraval connects to the tools studios already use (P4V, the `p4` CLI,
//! Unreal Engine's built-in source control) by speaking the Perforce wire
//! protocol. This crate is the clean-room implementation of that protocol:
//! the packet framing, message encoding, and command vocabulary that travel
//! over the connection. It carries no client policy of its own; higher layers
//! such as [`astraval-core`] decide what to send and how to react.
//!
//! This crate is currently a scaffold. The protocol surface arrives in later
//! milestones, starting with a single negotiated protocol level.
//!
//! [`astraval-core`]: https://docs.rs/astraval-core

/// The protocol level this crate currently targets.
///
/// Astraval negotiates one Perforce protocol level first and expands coverage
/// from there. The value is a placeholder until real negotiation lands; it
/// exists so downstream crates can already reference the constant by name.
pub const TARGET_PROTOCOL_LEVEL: u32 = 0;
