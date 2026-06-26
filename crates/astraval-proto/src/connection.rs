// SPDX-License-Identifier: Apache-2.0

//! A blocking client connection that exchanges framed RPC [`Message`]s.
//!
//! [`Connection`] is the thin synchronous transport the Astraval client uses to
//! talk to a Perforce server: it sends a [`Message`] by framing it (see the
//! [`frame`](crate::frame) module) and writing the bytes, and receives one by
//! reading until a full frame is buffered and decoding it. The framing itself is
//! I/O-agnostic; this type only adds the read/write loop and the buffer that
//! reassembles frames split across TCP reads.
//!
//! # Sans-I/O by construction
//!
//! `Connection` is generic over any `Read + Write`, so it drives a real
//! [`TcpStream`](std::net::TcpStream) in production and an in-memory buffer in
//! tests, with no network required (M-IMPL-IO). [`Connection::connect`] is the
//! one convenience that does touch the network, opening a TCP stream to a host.
//!
//! Async transport is intentionally out of scope here; because the codec is pure
//! and the buffering logic is small, an async wrapper can reuse
//! [`frame::decode`](crate::frame::decode) and [`frame::encode`](crate::frame::encode)
//! later without changing this module.
//!
//! # Examples
//! ```no_run
//! use astraval_proto::{Connection, Message};
//!
//! let mut conn = Connection::connect("localhost:1666")?;
//! conn.send(&Message::new().with("func", "user-info"))?;
//! let reply = conn.receive()?;
//! println!("{:?}", reply.value(b"func"));
//! # Ok::<(), astraval_proto::ConnectionError>(())
//! ```

use std::fmt::{self, Debug, Display, Formatter};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

use crate::frame::{self, FrameError, Message};

/// How many bytes to request per read while reassembling a frame.
///
/// Sized to comfortably hold a typical RPC packet in one read without being
/// wasteful; the buffer still grows transparently for larger messages. The value
/// is a throughput hint only and has no effect on correctness.
const READ_CHUNK: usize = 8192;

/// A blocking transport that sends and receives framed RPC [`Message`]s.
///
/// Wraps any `Read + Write` transport and owns a read buffer that reassembles
/// frames spanning multiple reads. Create one over a real socket with
/// [`Connection::connect`], or over any stream (including in-memory test
/// streams) with [`Connection::new`].
pub struct Connection<T> {
    /// The underlying byte transport (a TCP stream in production).
    transport: T,

    /// Bytes received but not yet consumed by a completed [`Connection::receive`].
    ///
    /// A single read can deliver part of a frame or several frames at once, so
    /// leftover bytes stay here and seed the next receive (see the `frame`
    /// module's note on TCP coalescing).
    pending: Vec<u8>,
}

impl Connection<TcpStream> {
    /// Connects to a Perforce server at `addr` over plaintext TCP.
    ///
    /// Accepts anything addressable, for example `"localhost:1666"` or a
    /// [`SocketAddr`](std::net::SocketAddr). The returned connection speaks the
    /// raw RPC framing; the login handshake and command dispatch are higher
    /// layers built on top of [`send`](Self::send) and [`receive`](Self::receive).
    ///
    /// # Errors
    /// Returns [`ConnectionError`] if no address resolves or the TCP connection
    /// cannot be established.
    ///
    /// # Examples
    /// ```no_run
    /// use astraval_proto::Connection;
    /// let conn = Connection::connect("localhost:1666")?;
    /// # Ok::<(), astraval_proto::ConnectionError>(())
    /// ```
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Self, ConnectionError> {
        let transport = TcpStream::connect(addr).map_err(ConnectionError::io)?;
        Ok(Self::new(transport))
    }
}

impl<T: Read + Write> Connection<T> {
    /// Wraps an existing `Read + Write` transport as a connection.
    ///
    /// This is the sans-I/O entry point: pass a [`TcpStream`], or any in-memory
    /// stream in tests, and the connection adds only framing and buffering.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            pending: Vec::new(),
        }
    }

    /// Frames `message` and writes it to the transport, flushing before return.
    ///
    /// # Errors
    /// Returns [`ConnectionError`] if the underlying write or flush fails.
    pub fn send(&mut self, message: &Message) -> Result<(), ConnectionError> {
        let bytes = frame::encode(message);
        self.transport
            .write_all(&bytes)
            .map_err(ConnectionError::io)?;
        self.transport.flush().map_err(ConnectionError::io)?;
        Ok(())
    }

    /// Reads until one complete frame is buffered and returns its [`Message`].
    ///
    /// Any bytes received beyond the returned frame are retained for the next
    /// call, so coalesced frames are handled transparently.
    ///
    /// # Errors
    /// Returns [`ConnectionError`] if the transport errors, if it closes before a
    /// full frame arrives (reported via [`is_disconnected`](ConnectionError::is_disconnected)),
    /// or if the bytes received are not a valid frame.
    pub fn receive(&mut self) -> Result<Message, ConnectionError> {
        loop {
            // Try to satisfy the request from already-buffered bytes first, so a
            // read that delivered several frames is drained without more reads.
            if let Some(decoded) = frame::decode(&self.pending).map_err(ConnectionError::frame)? {
                self.pending.drain(..decoded.consumed);
                return Ok(decoded.message);
            }

            let mut chunk = [0u8; READ_CHUNK];
            let read = self
                .transport
                .read(&mut chunk)
                .map_err(ConnectionError::io)?;
            if read == 0 {
                // EOF: the peer closed the connection. If we were mid-frame this
                // is a truncated message; either way there is nothing more to read.
                return Err(ConnectionError::disconnected());
            }
            self.pending.extend_from_slice(&chunk[..read]);
        }
    }

    /// Returns a shared reference to the underlying transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }
}

/// A `Debug` view that omits the transport and buffered bytes.
///
/// The transport is not `Debug`-bound and the pending buffer may carry user
/// data, so we surface only the buffered byte count and mark the rest omitted
/// with `finish_non_exhaustive`.
impl<T> Debug for Connection<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("pending_bytes", &self.pending.len())
            .finish_non_exhaustive()
    }
}

/// The failure class behind a [`ConnectionError`], kept private for future-proofing.
#[derive(Debug)]
enum ConnectionErrorKind {
    /// The underlying transport reported an I/O error.
    Io(std::io::Error),

    /// The peer closed the connection before a full frame arrived.
    Disconnected,

    /// The bytes received did not form a valid frame.
    Frame(FrameError),
}

/// An error from sending or receiving an RPC message over a connection.
///
/// Inspect the cause with [`is_io`](Self::is_io),
/// [`is_disconnected`](Self::is_disconnected), or [`is_frame`](Self::is_frame).
/// The wrapped source, when any, is reachable through [`std::error::Error::source`].
pub struct ConnectionError {
    kind: ConnectionErrorKind,
}

impl ConnectionError {
    /// Wraps an underlying I/O error.
    fn io(error: std::io::Error) -> Self {
        Self {
            kind: ConnectionErrorKind::Io(error),
        }
    }

    /// Creates a disconnected error for an unexpected peer close.
    fn disconnected() -> Self {
        Self {
            kind: ConnectionErrorKind::Disconnected,
        }
    }

    /// Wraps a framing error encountered while decoding received bytes.
    fn frame(error: FrameError) -> Self {
        Self {
            kind: ConnectionErrorKind::Frame(error),
        }
    }

    /// Returns `true` if the failure came from the underlying transport I/O.
    #[must_use]
    pub const fn is_io(&self) -> bool {
        matches!(self.kind, ConnectionErrorKind::Io(_))
    }

    /// Returns `true` if the peer closed the connection before a full frame.
    #[must_use]
    pub const fn is_disconnected(&self) -> bool {
        matches!(self.kind, ConnectionErrorKind::Disconnected)
    }

    /// Returns `true` if received bytes did not form a valid frame.
    #[must_use]
    pub const fn is_frame(&self) -> bool {
        matches!(self.kind, ConnectionErrorKind::Frame(_))
    }
}

/// Renders the connection error as a single diagnostic line.
impl Display for ConnectionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ConnectionErrorKind::Io(error) => write!(f, "connection i/o error: {error}"),
            ConnectionErrorKind::Disconnected => {
                write!(
                    f,
                    "server closed the connection before a complete message arrived"
                )
            }
            ConnectionErrorKind::Frame(error) => write!(f, "received a malformed frame: {error}"),
        }
    }
}

/// A `Debug` view forwarding to the private kind for a readable dump.
impl Debug for ConnectionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl std::error::Error for ConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ConnectionErrorKind::Io(error) => Some(error),
            ConnectionErrorKind::Frame(error) => Some(error),
            ConnectionErrorKind::Disconnected => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::encode;
    use std::io::Cursor;

    /// An in-memory transport: reads drain `inbound`, writes collect into `outbound`.
    ///
    /// Lets the connection's send and receive paths be exercised with no socket,
    /// satisfying the sans-I/O design (M-MOCKABLE-SYSCALLS).
    struct MemTransport {
        inbound: Cursor<Vec<u8>>,
        outbound: Vec<u8>,
    }

    impl MemTransport {
        fn with_inbound(bytes: Vec<u8>) -> Self {
            Self {
                inbound: Cursor::new(bytes),
                outbound: Vec::new(),
            }
        }
    }

    impl Read for MemTransport {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inbound.read(buf)
        }
    }

    impl Write for MemTransport {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.outbound.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn send_writes_the_framed_bytes() {
        let message = Message::new().with("func", "release2");
        let mut conn = Connection::new(MemTransport::with_inbound(Vec::new()));

        conn.send(&message).unwrap();
        assert_eq!(conn.transport().outbound, encode(&message));
    }

    #[test]
    fn receive_decodes_a_single_frame() {
        let message = Message::new().with("func", "user-info").with("prog", "p4");
        let mut conn = Connection::new(MemTransport::with_inbound(encode(&message)));

        assert_eq!(conn.receive().unwrap(), message);
    }

    #[test]
    fn receive_drains_two_coalesced_frames() {
        let first = Message::new().with("func", "release2");
        let second = Message::new().with("func", "user-info");
        let mut stream = encode(&first);
        stream.extend_from_slice(&encode(&second));

        let mut conn = Connection::new(MemTransport::with_inbound(stream));
        assert_eq!(conn.receive().unwrap(), first);
        assert_eq!(conn.receive().unwrap(), second);
    }

    #[test]
    fn receive_reports_a_clean_close_as_disconnected() {
        // No bytes at all: the first read returns EOF.
        let mut conn = Connection::new(MemTransport::with_inbound(Vec::new()));
        let err = conn.receive().unwrap_err();
        assert!(err.is_disconnected());
    }

    #[test]
    fn receive_reports_a_truncated_frame_as_disconnected() {
        // A preamble promising payload that never arrives, then EOF.
        let bytes = encode(&Message::new().with("func", "release2"));
        let mut conn = Connection::new(MemTransport::with_inbound(bytes[..6].to_vec()));
        let err = conn.receive().unwrap_err();
        assert!(err.is_disconnected());
    }

    #[test]
    fn receive_surfaces_a_framing_error() {
        let mut bytes = encode(&Message::new().with("func", "release2"));
        bytes[0] ^= 0xff; // corrupt the preamble check byte
        let mut conn = Connection::new(MemTransport::with_inbound(bytes));

        let err = conn.receive().unwrap_err();
        assert!(err.is_frame());
    }
}
