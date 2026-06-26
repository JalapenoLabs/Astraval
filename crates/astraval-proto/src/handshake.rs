// SPDX-License-Identifier: Apache-2.0

//! The Perforce protocol-negotiation handshake that opens every session.
//!
//! Before any command runs, a Perforce client and server agree on capabilities
//! and API/protocol levels by exchanging a `protocol` message. This module owns
//! that exchange: it builds the client `protocol` opener ([`Protocol`]), drives
//! it over a [`Connection`](crate::connection::Connection), and parses the
//! server's `protocol` reply into a typed [`Negotiated`] result the higher
//! layers consume. The RPC framing underneath comes from the [`frame`] module;
//! command dispatch on top of the negotiated session is a later layer.
//!
//! # The handshake is an opener, not a request/response
//!
//! A bare `protocol` frame does not by itself elicit a reply: a real `p4d`
//! waits for the command frame that follows before it answers. So the wire
//! sequence is `protocol` opener, then the first command, then the server's
//! response stream which begins with its own `protocol` reply. [`negotiate`]
//! models exactly this: it sends the opener, sends a caller-chosen command
//! frame, then reads frames until the server's `func = protocol` reply arrives
//! and returns it parsed. It stops there; consuming the rest of the response
//! (the server-invokes-client dispatch loop) is a separate layer.
//!
//! # Targeted level
//!
//! Per the project rule, Astraval negotiates one protocol level first. We target
//! the level a current `p4` client negotiates against the oracle: client API
//! level `100`, asking the server for its maximum (`api = 99999`) and accepting
//! the server level it returns (`server2`, here `62`). The defaults on
//! [`Protocol`] reproduce that exchange byte for byte.
//!
//! # Provenance (clean-room)
//!
//! Field names and semantics derive only from the open-source `P4Java` client
//! (`ProtocolCommand` for the client keys `client`, `api`, `cmpfile`, `sndbuf`,
//! `rcvbuf`; `RpcFunctionMapKey` for the server keys `server`, `server2`,
//! `xfiles`, ...) and from observed wire bytes. See
//! `protocol-lab/captures/03-protocol-handshake.md` for the byte-level note and
//! the two golden frames the tests assert against. Nothing here derives from the
//! closed server binary.
//!
//! # Examples
//! ```no_run
//! use astraval_proto::{Connection, Message};
//! use astraval_proto::handshake::{Protocol, negotiate};
//!
//! let mut conn = Connection::connect("localhost:1666")?;
//!
//! // The opener advertises our capabilities and asks the server for its level.
//! let opener = Protocol::new("my-host", "tcp:localhost:1666");
//!
//! // The first real command rides right behind the opener; the server's
//! // `protocol` reply arrives on the front of the response to it.
//! let command = Message::new().with("func", "user-info").with("prog", "astraval");
//!
//! let negotiated = negotiate(&mut conn, &opener, &command)?;
//! println!("server level {}", negotiated.server_api_level());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt::{self, Debug, Display, Formatter};
use std::io::{Read, Write};

use crate::connection::{Connection, ConnectionError};
use crate::frame::Message;

/// The client API level Astraval claims in the `protocol` opener.
///
/// This is the single level we target first (see the module docs). It mirrors
/// the `client` field a current `p4` client sends in
/// `protocol-lab/captures/03-protocol-handshake.md`. Changing it changes the
/// capabilities the server expects of us, so it is a deliberate, tested choice.
pub const TARGET_CLIENT_API_LEVEL: u32 = 100;

/// The server API level the opener requests: the server's maximum.
///
/// A real `p4` client sends `99999` in `api` to mean "use your highest level",
/// then accepts whatever the server reports back (`P4Java` `ProtocolCommand`
/// `RPC_ARGNAME_PROTOCOL_SERVER_API = "api"`). We do the same so the server,
/// not the client, picks the level.
pub const REQUEST_MAX_SERVER_API_LEVEL: u32 = 99999;

/// Default socket send-buffer hint, in bytes, advertised in `sndbuf`.
///
/// Matches the real client's value in the capture. It is a transport hint the
/// server echoes back; it has no effect on message semantics.
const DEFAULT_SEND_BUFFER: u32 = 524_288;

/// Default socket receive-buffer hint, in bytes, advertised in `rcvbuf`.
///
/// Matches the real client's value in the capture; a transport hint only.
const DEFAULT_RECEIVE_BUFFER: u32 = 65_536;

/// The RPC selector value naming the protocol message: `func = protocol`.
///
/// In the opener this is the *last* field, closing the capability spec; the
/// server's reply also ends with it. Both are confirmed in the capture note.
const FUNC_PROTOCOL: &[u8] = b"protocol";

/// The client `protocol` opener: capability flags plus client and server levels.
///
/// `Protocol` builds the first message a client sends, declaring what it can do
/// and which API levels it speaks. Its [`Default`]-derived field values
/// reproduce a real `p4` client's opener exactly (verified byte for byte by the
/// golden test); only [`host`](Self::host) and [`port`](Self::port) are
/// environment-specific and must be supplied. Turn it into a [`Message`] with
/// [`to_message`](Self::to_message), or pass it to [`negotiate`].
///
/// The capability flags are advertised empty, meaning "not enabled": Astraval
/// does not yet opt into streams, graph, chunking, and similar. They are kept as
/// explicit fields rather than omitted so the emitted message matches the real
/// client's field set position for position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protocol {
    /// The client host name, sent as `host`.
    host: Vec<u8>,

    /// The server port the client dialed, sent as `port` (e.g. `tcp:host:1666`).
    port: Vec<u8>,

    /// The client API level, sent as `client`.
    client_api_level: u32,

    /// The maximum server API level requested, sent as `api`.
    server_api_request: u32,

    /// The send-buffer hint in bytes, sent as `sndbuf`.
    send_buffer: u32,

    /// The receive-buffer hint in bytes, sent as `rcvbuf`.
    receive_buffer: u32,
}

impl Protocol {
    /// Creates an opener for the given client `host` and server `port`.
    ///
    /// All other fields take the targeted defaults that reproduce a real `p4`
    /// client's opener: client level [`TARGET_CLIENT_API_LEVEL`], a request for
    /// the server's maximum ([`REQUEST_MAX_SERVER_API_LEVEL`]), the captured
    /// buffer sizes, and every optional capability advertised as disabled.
    ///
    /// `port` is the server endpoint string the client dialed, conventionally
    /// `tcp:host:port`; it is sent verbatim in the `port` field.
    ///
    /// # Examples
    /// ```
    /// use astraval_proto::handshake::Protocol;
    /// let opener = Protocol::new("Titan", "tcp:localhost:1666");
    /// assert_eq!(opener.host(), b"Titan");
    /// ```
    pub fn new(host: impl AsRef<[u8]>, port: impl AsRef<[u8]>) -> Self {
        Self {
            host: host.as_ref().to_vec(),
            port: port.as_ref().to_vec(),
            client_api_level: TARGET_CLIENT_API_LEVEL,
            server_api_request: REQUEST_MAX_SERVER_API_LEVEL,
            send_buffer: DEFAULT_SEND_BUFFER,
            receive_buffer: DEFAULT_RECEIVE_BUFFER,
        }
    }

    /// Returns the client host name sent as `host`.
    #[must_use]
    pub fn host(&self) -> &[u8] {
        &self.host
    }

    /// Returns the server port string sent as `port`.
    #[must_use]
    pub fn port(&self) -> &[u8] {
        &self.port
    }

    /// Returns the client API level sent as `client`.
    #[must_use]
    pub const fn client_api_level(&self) -> u32 {
        self.client_api_level
    }

    /// Renders this opener as the exact `protocol` [`Message`] sent on the wire.
    ///
    /// The field order matches a real client's opener: the capability flags and
    /// levels first, then `host`, `port`, the buffer hints, and finally the
    /// `func = protocol` selector that closes the message. This ordering is part
    /// of the wire contract and is asserted against the captured golden frame.
    ///
    /// # Examples
    /// ```
    /// use astraval_proto::handshake::Protocol;
    /// let msg = Protocol::new("Titan", "tcp:localhost:1670").to_message();
    /// assert_eq!(msg.value(b"client"), Some(&b"100"[..]));
    /// assert_eq!(msg.value(b"func"), Some(&b"protocol"[..]));
    /// ```
    #[must_use]
    pub fn to_message(&self) -> Message {
        // Empty capability flags advertise "not enabled". They are emitted in
        // the same positions a real `p4` client uses so the byte stream matches.
        Message::new()
            .with("cmpfile", "")
            .with("altSync", "")
            .with("client", self.client_api_level.to_string())
            .with("api", self.server_api_request.to_string())
            .with("enableStreams", "")
            .with("enableGraph", "")
            .with("expandAndmaps", "")
            .with("chunking", "")
            .with("host", &self.host)
            .with("port", &self.port)
            .with("sndbuf", self.send_buffer.to_string())
            .with("rcvbuf", self.receive_buffer.to_string())
            .with("autoTune", "1")
            .with("func", FUNC_PROTOCOL)
    }
}

/// The protocol levels and capabilities settled during the handshake.
///
/// `Negotiated` is the parsed server `protocol` reply: the levels both sides
/// agreed on plus the server-reported capabilities the client needs to behave
/// correctly. Obtain one from [`negotiate`], or parse a received reply directly
/// with [`Negotiated::from_reply`].
///
/// The negotiated **server API level** is the server's `server2` value when
/// present, falling back to `server`: `server2` is the extended level that
/// supersedes the base `server` level (`P4Java` `RpcFunctionMapKey.SERVER` /
/// `SERVER2`). The targeted exchange returns `server2 = 62`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negotiated {
    /// The negotiated server API level (`server2`, or `server` if no `server2`).
    server_api_level: u32,

    /// The base server protocol level reported in `server`.
    base_server_level: u32,

    /// The server's security level reported in `security`, if any.
    security_level: Option<u32>,

    /// The maximum file type the server supports, reported in `xfiles`.
    max_file_type: Option<u32>,

    /// Whether the server is case-insensitive (the `nocase` field is present).
    case_insensitive: bool,
}

impl Negotiated {
    /// Parses a server `protocol` reply into the negotiated session parameters.
    ///
    /// The negotiated server API level is taken from `server2` when present,
    /// otherwise from `server`; at least one must be present and numeric.
    /// Case-insensitivity is signalled by the mere presence of `nocase`
    /// (its value is empty on the wire). The optional `security` and `xfiles`
    /// levels are parsed when present and numeric.
    ///
    /// # Errors
    /// Returns [`HandshakeError`] if the reply is not a `protocol` reply
    /// (`func != protocol`), if neither `server` nor `server2` is present, or if
    /// a level field that is present does not parse as a number.
    ///
    /// # Examples
    /// ```
    /// use astraval_proto::Message;
    /// use astraval_proto::handshake::Negotiated;
    ///
    /// let reply = Message::new()
    ///     .with("server", "3")
    ///     .with("server2", "62")
    ///     .with("security", "4")
    ///     .with("func", "protocol");
    /// let negotiated = Negotiated::from_reply(&reply).unwrap();
    /// assert_eq!(negotiated.server_api_level(), 62);
    /// assert_eq!(negotiated.security_level(), Some(4));
    /// ```
    pub fn from_reply(reply: &Message) -> Result<Self, HandshakeError> {
        if reply.value(b"func") != Some(FUNC_PROTOCOL) {
            return Err(HandshakeError::not_protocol());
        }

        let base = parse_level(reply, b"server")?;
        let extended = parse_level(reply, b"server2")?;
        let (base_server_level, server_api_level) = match (base, extended) {
            // `server2` supersedes `server` as the effective negotiated level.
            (Some(base), Some(ext)) => (base, ext),
            (Some(base), None) => (base, base),
            (None, Some(ext)) => (ext, ext),
            (None, None) => return Err(HandshakeError::missing_level()),
        };

        Ok(Self {
            server_api_level,
            base_server_level,
            security_level: parse_level(reply, b"security")?,
            max_file_type: parse_level(reply, b"xfiles")?,
            // `nocase` carries an empty value; its presence alone is the signal.
            case_insensitive: reply.value(b"nocase").is_some(),
        })
    }

    /// Returns the negotiated server API level (`server2`, else `server`).
    #[must_use]
    pub const fn server_api_level(&self) -> u32 {
        self.server_api_level
    }

    /// Returns the base server protocol level reported in `server`.
    #[must_use]
    pub const fn base_server_level(&self) -> u32 {
        self.base_server_level
    }

    /// Returns the server's security level, if it reported one.
    #[must_use]
    pub const fn security_level(&self) -> Option<u32> {
        self.security_level
    }

    /// Returns the maximum file type the server supports, if reported.
    #[must_use]
    pub const fn max_file_type(&self) -> Option<u32> {
        self.max_file_type
    }

    /// Returns `true` if the server reported case-insensitive path handling.
    #[must_use]
    pub const fn is_case_insensitive(&self) -> bool {
        self.case_insensitive
    }
}

/// Runs the handshake over `conn` and returns the negotiated session parameters.
///
/// This sends the `opener` (the client `protocol` message), then the `command`
/// frame the server needs before it will answer, then reads frames until the
/// server's `func = protocol` reply arrives and parses it. Frames the server
/// sends ahead of its `protocol` reply are skipped; the function stops at the
/// reply without consuming the rest of the response, which belongs to the
/// dispatch layer.
///
/// `command` is any valid command frame; the server's `protocol` reply rides on
/// the front of the response to it. A light, side-effect-free command such as
/// `user-info` is a natural choice.
///
/// # Errors
/// Returns [`HandshakeError`] if the transport fails, if the connection closes
/// before a `protocol` reply arrives, or if the reply does not parse (see
/// [`Negotiated::from_reply`]).
pub fn negotiate<T: Read + Write>(
    conn: &mut Connection<T>,
    opener: &Protocol,
    command: &Message,
) -> Result<Negotiated, HandshakeError> {
    conn.send(&opener.to_message())
        .map_err(HandshakeError::connection)?;
    conn.send(command).map_err(HandshakeError::connection)?;

    // The server may emit other frames before its `protocol` reply; read until
    // we find it. A clean close before then is a handshake failure.
    loop {
        let reply = conn.receive().map_err(HandshakeError::connection)?;
        if reply.value(b"func") == Some(FUNC_PROTOCOL) {
            return Negotiated::from_reply(&reply);
        }
    }
}

/// Reads an optional numeric level field from a message.
///
/// Returns `Ok(None)` when the field is absent, `Ok(Some(n))` when it is present
/// and parses, and an error when it is present but not a base-10 number.
fn parse_level(message: &Message, name: &[u8]) -> Result<Option<u32>, HandshakeError> {
    let Some(raw) = message.value(name) else {
        return Ok(None);
    };

    // The parse errors carry no detail worth surfacing; the raw wire value is the
    // useful diagnostic, so we record that on `BadLevel` rather than the cause.
    match std::str::from_utf8(raw)
        .ok()
        .and_then(|text| text.parse().ok())
    {
        Some(level) => Ok(Some(level)),
        None => Err(HandshakeError::bad_level(name, raw)),
    }
}

/// The failure class behind a [`HandshakeError`], kept private for future-proofing.
#[derive(Debug)]
enum HandshakeErrorKind {
    /// The reply was not a `protocol` reply (`func` was missing or different).
    NotProtocol,

    /// The reply carried neither a `server` nor a `server2` level.
    MissingLevel,

    /// A level field was present but did not parse as a number.
    BadLevel { field: String, value: Vec<u8> },

    /// The underlying connection failed during the handshake.
    Connection(ConnectionError),
}

/// An error from running or parsing the protocol-negotiation handshake.
///
/// Inspect the cause with [`is_not_protocol`](Self::is_not_protocol),
/// [`is_missing_level`](Self::is_missing_level),
/// [`is_bad_level`](Self::is_bad_level), or
/// [`is_connection`](Self::is_connection). When the failure came from the
/// transport, the wrapped [`ConnectionError`] is reachable through
/// [`std::error::Error::source`].
pub struct HandshakeError {
    kind: HandshakeErrorKind,
}

impl HandshakeError {
    /// Creates a "reply was not a protocol reply" error.
    fn not_protocol() -> Self {
        Self {
            kind: HandshakeErrorKind::NotProtocol,
        }
    }

    /// Creates a "no server level present" error.
    fn missing_level() -> Self {
        Self {
            kind: HandshakeErrorKind::MissingLevel,
        }
    }

    /// Creates a "level field did not parse" error from the field and raw value.
    fn bad_level(field: &[u8], value: &[u8]) -> Self {
        Self {
            kind: HandshakeErrorKind::BadLevel {
                field: String::from_utf8_lossy(field).into_owned(),
                value: value.to_vec(),
            },
        }
    }

    /// Wraps a connection error encountered during the handshake.
    fn connection(error: ConnectionError) -> Self {
        Self {
            kind: HandshakeErrorKind::Connection(error),
        }
    }

    /// Returns `true` if the server's reply was not a `protocol` reply.
    #[must_use]
    pub const fn is_not_protocol(&self) -> bool {
        matches!(self.kind, HandshakeErrorKind::NotProtocol)
    }

    /// Returns `true` if the reply carried no `server` or `server2` level.
    #[must_use]
    pub const fn is_missing_level(&self) -> bool {
        matches!(self.kind, HandshakeErrorKind::MissingLevel)
    }

    /// Returns `true` if a level field was present but did not parse.
    #[must_use]
    pub const fn is_bad_level(&self) -> bool {
        matches!(self.kind, HandshakeErrorKind::BadLevel { .. })
    }

    /// Returns `true` if the failure came from the underlying connection.
    #[must_use]
    pub const fn is_connection(&self) -> bool {
        matches!(self.kind, HandshakeErrorKind::Connection(_))
    }
}

/// Renders the handshake error as a single diagnostic line.
impl Display for HandshakeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            HandshakeErrorKind::NotProtocol => {
                write!(f, "handshake reply was not a `protocol` message")
            }
            HandshakeErrorKind::MissingLevel => write!(
                f,
                "handshake reply carried no `server` or `server2` protocol level"
            ),
            HandshakeErrorKind::BadLevel { field, value } => write!(
                f,
                "handshake level field `{field}` is not a number: {:?}",
                String::from_utf8_lossy(value)
            ),
            HandshakeErrorKind::Connection(error) => {
                write!(f, "handshake transport failed: {error}")
            }
        }
    }
}

/// A `Debug` view forwarding to the private kind for a readable dump.
impl Debug for HandshakeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandshakeError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl std::error::Error for HandshakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            HandshakeErrorKind::Connection(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Field, encode};

    /// The exact 240 bytes of a real client `protocol` opener, preamble included.
    ///
    /// Captured from a live `p4 info` exchange against the oracle (provenance in
    /// `protocol-lab/captures/03-protocol-handshake.md`) and vendored into the
    /// crate at `tests/fixtures/` so it builds standalone. This is the golden
    /// frame: it proves the opener we build matches real wire bytes, not just
    /// itself. The host is `Titan` and the port is `tcp:localhost:1670`, the
    /// values the captured client used.
    const GOLDEN_CLIENT_PROTOCOL: &[u8] =
        include_bytes!("../tests/fixtures/03-client-protocol.bin");

    /// The exact 246 bytes of the real server `protocol` reply, preamble included.
    ///
    /// Vendored at `tests/fixtures/03-server-protocol.bin` from the same captured
    /// exchange; the parsing tests decode it and assert the negotiated levels.
    const GOLDEN_SERVER_PROTOCOL: &[u8] =
        include_bytes!("../tests/fixtures/03-server-protocol.bin");

    #[test]
    fn opener_matches_the_real_wire_bytes() {
        // The captured client dialed as host `Titan`, port `tcp:localhost:1670`.
        // Building the opener with those values must reproduce the bytes exactly.
        let opener = Protocol::new("Titan", "tcp:localhost:1670");
        assert_eq!(encode(&opener.to_message()), GOLDEN_CLIENT_PROTOCOL);
    }

    #[test]
    fn opener_carries_the_targeted_levels() {
        let opener = Protocol::new("host", "tcp:localhost:1666");
        let msg = opener.to_message();
        assert_eq!(msg.value(b"client"), Some(b"100".as_slice()));
        assert_eq!(msg.value(b"api"), Some(b"99999".as_slice()));
        // `func = protocol` must be the closing field, mirroring the capture.
        assert_eq!(
            msg.fields().last().map(Field::name),
            Some(b"func".as_slice())
        );
        assert_eq!(msg.value(b"func"), Some(FUNC_PROTOCOL));
    }

    #[test]
    fn parses_the_real_server_reply() {
        // Decode the golden server frame, then parse the negotiated parameters.
        let decoded = crate::frame::decode(GOLDEN_SERVER_PROTOCOL)
            .unwrap()
            .expect("a complete frame");
        let negotiated = Negotiated::from_reply(&decoded.message).unwrap();

        // From the capture: server=3, server2=62, security=4, xfiles=7, nocase set.
        assert_eq!(negotiated.server_api_level(), 62);
        assert_eq!(negotiated.base_server_level(), 3);
        assert_eq!(negotiated.security_level(), Some(4));
        assert_eq!(negotiated.max_file_type(), Some(7));
        assert!(negotiated.is_case_insensitive());
    }

    #[test]
    fn server2_supersedes_server_when_both_present() {
        let reply = Message::new()
            .with("server", "3")
            .with("server2", "62")
            .with("func", "protocol");
        let negotiated = Negotiated::from_reply(&reply).unwrap();
        assert_eq!(negotiated.server_api_level(), 62);
        assert_eq!(negotiated.base_server_level(), 3);
    }

    #[test]
    fn falls_back_to_server_without_server2() {
        let reply = Message::new().with("server", "41").with("func", "protocol");
        let negotiated = Negotiated::from_reply(&reply).unwrap();
        assert_eq!(negotiated.server_api_level(), 41);
        assert_eq!(negotiated.base_server_level(), 41);
    }

    #[test]
    fn case_sensitive_when_nocase_absent() {
        let reply = Message::new()
            .with("server2", "62")
            .with("func", "protocol");
        let negotiated = Negotiated::from_reply(&reply).unwrap();
        assert!(!negotiated.is_case_insensitive());
        assert_eq!(negotiated.security_level(), None);
        assert_eq!(negotiated.max_file_type(), None);
    }

    #[test]
    fn rejects_a_non_protocol_reply() {
        let reply = Message::new().with("server2", "62").with("func", "release");
        let err = Negotiated::from_reply(&reply).unwrap_err();
        assert!(err.is_not_protocol());
    }

    #[test]
    fn rejects_a_reply_without_any_server_level() {
        let reply = Message::new()
            .with("security", "4")
            .with("func", "protocol");
        let err = Negotiated::from_reply(&reply).unwrap_err();
        assert!(err.is_missing_level());
    }

    #[test]
    fn rejects_a_non_numeric_level() {
        let reply = Message::new()
            .with("server2", "not-a-number")
            .with("func", "protocol");
        let err = Negotiated::from_reply(&reply).unwrap_err();
        assert!(err.is_bad_level());
    }
}
