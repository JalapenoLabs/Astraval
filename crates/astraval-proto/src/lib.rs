// SPDX-License-Identifier: Apache-2.0

//! The Perforce-compatible wire protocol spoken by Astraval.
//!
//! Astraval connects to the tools studios already use (P4V, the `p4` CLI,
//! Unreal Engine's built-in source control) by speaking the Perforce wire
//! protocol. This crate is the clean-room implementation of that protocol:
//! the packet framing, message encoding, and (in later milestones) the command
//! vocabulary that travel over the connection. It carries no client policy of
//! its own; higher layers such as [`astraval-core`] decide what to send and how
//! to react.
//!
//! # What is here today
//!
//! The lowest transport layer:
//!
//! - [`frame`]: the I/O-agnostic framing codec. [`encode`] turns a [`Message`]
//!   of [`Field`]s into wire bytes and [`decode`] turns wire bytes back into a
//!   [`Message`], with full unit and golden-frame coverage and no network.
//! - [`connection`]: a blocking [`Connection`] that frames messages over any
//!   `Read + Write` transport, with [`Connection::connect`] opening a real TCP
//!   socket to a server.
//!
//! The login handshake (negotiation) and command dispatch are separate layers
//! built on top of this one in later milestones.
//!
//! # Example
//! ```
//! use astraval_proto::{Message, encode, decode};
//!
//! let request = Message::new().with("func", "user-info").with("prog", "p4");
//! let bytes = encode(&request);
//!
//! let decoded = decode(&bytes).unwrap().expect("a complete frame");
//! assert_eq!(decoded.message, request);
//! ```
//!
//! [`astraval-core`]: https://docs.rs/astraval-core

pub mod connection;
pub mod frame;

#[doc(inline)]
pub use connection::{Connection, ConnectionError};
#[doc(inline)]
pub use frame::{Decoded, Field, FrameError, Message, PREAMBLE_LEN, decode, encode};

/// The protocol level this crate currently targets.
///
/// Astraval negotiates one Perforce protocol level first and expands coverage
/// from there. The value is a placeholder until real negotiation lands; it
/// exists so downstream crates can already reference the constant by name.
pub const TARGET_PROTOCOL_LEVEL: u32 = 0;
