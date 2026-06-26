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
//! The transport and the opening handshake:
//!
//! - [`frame`]: the I/O-agnostic framing codec. [`encode`] turns a [`Message`]
//!   of [`Field`]s into wire bytes and [`decode`] turns wire bytes back into a
//!   [`Message`], with full unit and golden-frame coverage and no network.
//! - [`connection`]: a blocking [`Connection`] that frames messages over any
//!   `Read + Write` transport, with [`Connection::connect`] opening a real TCP
//!   socket to a server.
//! - [`handshake`]: the protocol-negotiation [`negotiate`] exchange that opens a
//!   session, building the client [`Protocol`] opener and parsing the server's
//!   reply into a [`Negotiated`] result.
//!
//! Command dispatch over the negotiated session is a separate layer built on top
//! of this one in a later milestone.
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
pub mod handshake;

#[doc(inline)]
pub use connection::{Connection, ConnectionError};
#[doc(inline)]
pub use frame::{Decoded, Field, FrameError, Message, PREAMBLE_LEN, decode, encode};
#[doc(inline)]
pub use handshake::{
    HandshakeError, Negotiated, Protocol, REQUEST_MAX_SERVER_API_LEVEL, TARGET_CLIENT_API_LEVEL,
    negotiate,
};

/// The client protocol level this crate negotiates first.
///
/// Astraval targets one Perforce protocol level before expanding coverage. This
/// is the client API level we advertise in the `protocol` opener; the server
/// reports its own level in reply (see [`Negotiated::server_api_level`]). It
/// aliases [`TARGET_CLIENT_API_LEVEL`] so downstream crates can refer to the
/// targeted level by either name.
pub const TARGET_PROTOCOL_LEVEL: u32 = TARGET_CLIENT_API_LEVEL;
