// SPDX-License-Identifier: Apache-2.0

//! The Perforce RPC framing codec: messages of fields to bytes and back.
//!
//! This is the lowest layer of the Perforce wire protocol, the envelope that
//! turns a [`Message`] (an ordered list of name/value [`Field`]s) into the exact
//! bytes a real `p4d` expects, and parses those bytes back into a [`Message`].
//! It is intentionally I/O-agnostic: [`encode`] writes into a caller-owned
//! buffer and [`decode`] reads from a caller-owned slice, so the same code drives
//! a blocking socket today and an async one later, and every path is unit
//! testable with no network.
//!
//! # Wire format
//!
//! A message is a fixed 5-byte preamble followed by a payload of fields:
//!
//! ```text
//! MESSAGE := PREAMBLE PAYLOAD
//!
//! PREAMBLE (5 bytes):
//!   [0]      check = L0 ^ L1 ^ L2 ^ L3        (XOR of the four length bytes)
//!   [1..4]   payload length as little-endian u32 (L0 = LSB ... L3 = MSB)
//!
//! PAYLOAD (exactly `payload length` bytes): one or more FIELDs concatenated
//!          back to back, with no separator and no stored field count.
//!
//! FIELD:
//!   <name bytes> 0x00                          (name, null-terminated)
//!   <4-byte little-endian u32 value length>    (length of value, excludes \0)
//!   <value bytes> 0x00                          (value, then a terminating null)
//! ```
//!
//! The receiver knows the payload's extent from the preamble length, then
//! consumes fields until that payload is exhausted; the field count is never
//! transmitted. The leading field of a request is conventionally `func`, naming
//! the RPC, but that convention belongs to higher layers; this codec treats all
//! fields uniformly.
//!
//! # Why bytes, not strings
//!
//! Perforce field values are arbitrary byte strings (paths in any encoding,
//! binary tokens), so [`Field`] stores `Vec<u8>` rather than `String`. Callers
//! that know a value is text decode it themselves.
//!
//! # Provenance (clean-room)
//!
//! The format is derived only from the open-source `P4Java` client
//! (`RpcPacketPreamble`, `RpcPacketField`, `RpcPacket`, `RpcStreamConnection` in
//! `com.perforce.p4java.impl.mapbased.rpc`) and from public packet analysis of a
//! real `p4d`. See `protocol-lab/captures/02-rpc-framing.md` for the byte-level
//! note and the golden frame the tests assert against. Nothing here derives from
//! the closed server binary.

use std::fmt::{self, Debug, Display, Formatter};

/// The fixed size in bytes of an RPC preamble: one check byte plus a u32 length.
///
/// Derived from `P4Java` `RpcPacketPreamble` (`RPC_PREAMBLE_SIZE = 4 + 1`). A
/// complete message is always at least this many bytes.
pub const PREAMBLE_LEN: usize = 5;

/// The size in bytes of every little-endian length field on the wire.
///
/// Both the preamble's payload length and each field's value length use this
/// width (`P4Java` `RpcPacket.RPC_LENGTH_FIELD_LENGTH = 4`).
const LENGTH_LEN: usize = 4;

/// One name/value pair within a [`Message`].
///
/// Both halves are raw byte strings: Perforce values are not guaranteed to be
/// UTF-8 (they carry paths, tokens, and occasionally binary), so we keep them as
/// `Vec<u8>` and leave any text decoding to the caller.
#[derive(Clone, PartialEq, Eq)]
pub struct Field {
    name: Vec<u8>,
    value: Vec<u8>,
}

impl Field {
    /// Creates a field from a `name` and `value`, each any byte-slice-like value.
    ///
    /// # Examples
    /// ```
    /// use astraval_proto::Field;
    /// let f = Field::new("func", "release2");
    /// assert_eq!(f.name(), b"func");
    /// assert_eq!(f.value(), b"release2");
    /// ```
    pub fn new(name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        Self {
            name: name.as_ref().to_vec(),
            value: value.as_ref().to_vec(),
        }
    }

    /// Returns the field name as raw bytes.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// Returns the field value as raw bytes.
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// Returns the number of payload bytes this field occupies once encoded.
    ///
    /// That is the name, its terminating null, the 4-byte length, the value, and
    /// the value's terminating null.
    fn encoded_len(&self) -> usize {
        self.name.len() + 1 + LENGTH_LEN + self.value.len() + 1
    }

    /// Appends this field's wire bytes to `out` in the canonical field layout.
    fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.name);
        out.push(0);
        out.extend_from_slice(&encode_u32_le(value_len_u32(self.value.len())));
        out.extend_from_slice(&self.value);
        out.push(0);
    }
}

/// A `Debug` view that renders byte strings readably, escaping non-text bytes.
///
/// Field names and values are usually ASCII, so showing them as text aids
/// debugging far more than a raw byte vector would; non-printable bytes fall back
/// to escapes via [`std::ascii::escape_default`].
impl Debug for Field {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Field")
            .field("name", &DisplayBytes(&self.name))
            .field("value", &DisplayBytes(&self.value))
            .finish()
    }
}

/// A single framed RPC message: an ordered list of [`Field`]s.
///
/// Field order is significant on the wire (the leading field names the RPC), so
/// a `Message` preserves insertion order and never reorders or deduplicates.
/// Build one with [`Message::new`] and [`Message::with`], encode it with
/// [`encode`], and recover it with [`decode`].
///
/// # Examples
/// ```
/// use astraval_proto::{Message, encode, decode};
///
/// let msg = Message::new().with("func", "release2");
/// let bytes = encode(&msg);
///
/// let decoded = decode(&bytes).unwrap().expect("a complete frame");
/// assert_eq!(decoded.message, msg);
/// assert_eq!(decoded.consumed, bytes.len());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    fields: Vec<Field>,
}

impl Message {
    /// Creates an empty message with no fields.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a field and returns the message, for fluent construction.
    ///
    /// # Examples
    /// ```
    /// use astraval_proto::Message;
    /// let msg = Message::new().with("func", "user-info").with("prog", "p4");
    /// assert_eq!(msg.fields().len(), 2);
    /// ```
    #[must_use]
    pub fn with(mut self, name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        self.fields.push(Field::new(name, value));
        self
    }

    /// Appends an already-built [`Field`] in place.
    pub fn push(&mut self, field: Field) {
        self.fields.push(field);
    }

    /// Returns the message's fields in wire order.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Returns the value of the first field named `name`, if present.
    ///
    /// Fields are not deduplicated; this returns the first match in wire order,
    /// which is the right choice for the single-valued fields the protocol uses.
    ///
    /// # Examples
    /// ```
    /// use astraval_proto::Message;
    /// let msg = Message::new().with("func", "release2");
    /// assert_eq!(msg.value(b"func"), Some(&b"release2"[..]));
    /// assert_eq!(msg.value(b"missing"), None);
    /// ```
    #[must_use]
    pub fn value(&self, name: impl AsRef<[u8]>) -> Option<&[u8]> {
        let name = name.as_ref();
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(Field::value)
    }

    /// Returns the number of payload bytes this message's fields occupy.
    fn payload_len(&self) -> usize {
        self.fields.iter().map(Field::encoded_len).sum()
    }
}

/// A message decoded from a buffer, paired with how many bytes it consumed.
///
/// [`decode`] returns this so a caller draining a stream can advance its buffer
/// by [`consumed`](Self::consumed) and decode the next frame from what remains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// The message parsed from the buffer.
    pub message: Message,

    /// How many leading bytes of the input the frame occupied (preamble +
    /// payload). The next frame, if any, begins at this offset.
    pub consumed: usize,
}

/// Encodes a [`Message`] into a fresh `Vec<u8>` ready to write to the wire.
///
/// The result is the 5-byte preamble followed by the concatenated field
/// payload, exactly as a real `p4d` expects. This is the inverse of [`decode`].
///
/// # Examples
/// ```
/// use astraval_proto::{Message, encode};
/// let bytes = encode(&Message::new().with("func", "release2"));
/// // preamble (5) + "func\0" (5) + len (4) + "release2\0" (9) = 23 bytes
/// assert_eq!(bytes.len(), 23);
/// ```
///
/// # Panics
/// Panics if the message payload exceeds [`u32::MAX`] bytes, which the
/// little-endian length field cannot represent. Real RPC messages are far
/// smaller, so reaching this is a programming error, not a runtime condition.
#[must_use]
pub fn encode(message: &Message) -> Vec<u8> {
    let payload_len = message.payload_len();
    let length = payload_u32(payload_len);

    let mut out = Vec::with_capacity(PREAMBLE_LEN + payload_len);
    out.extend_from_slice(&encode_preamble(length));
    for field in &message.fields {
        field.encode_into(&mut out);
    }

    debug_assert_eq!(out.len(), PREAMBLE_LEN + payload_len);
    out
}

/// Decodes a single framed message from the front of `input`.
///
/// Returns `Ok(Some(Decoded))` with the message and the byte count it consumed
/// when `input` holds at least one complete frame. Returns `Ok(None)` when the
/// buffer holds only a partial frame and the caller should read more bytes and
/// retry; this is the normal way to drive a stream, not an error. Returns
/// `Err(FrameError)` only when the bytes present are not a valid frame.
///
/// Decoding never reads past the one frame at the front, so a buffer holding
/// several coalesced frames is drained by decoding, advancing by
/// [`Decoded::consumed`], and decoding again.
///
/// # Errors
/// Returns a [`FrameError`] if the preamble's check byte does not match its
/// length, if a field's value length runs past the declared payload, or if the
/// null terminators required by the field layout are missing. A truncated buffer
/// is not an error; it yields `Ok(None)`.
///
/// # Examples
/// ```
/// use astraval_proto::{Message, encode, decode};
/// let bytes = encode(&Message::new().with("func", "release2"));
///
/// // A partial buffer asks for more bytes rather than erroring.
/// assert_eq!(decode(&bytes[..4]).unwrap(), None);
///
/// // The full buffer yields the message and its length.
/// let decoded = decode(&bytes).unwrap().unwrap();
/// assert_eq!(decoded.message.value(b"func"), Some(&b"release2"[..]));
/// ```
pub fn decode(input: &[u8]) -> Result<Option<Decoded>, FrameError> {
    let Some(payload_len) = decode_preamble(input)? else {
        return Ok(None);
    };

    let frame_len = PREAMBLE_LEN + payload_len;
    if input.len() < frame_len {
        // The header promised more payload than has arrived yet; wait for it.
        return Ok(None);
    }

    let payload = &input[PREAMBLE_LEN..frame_len];
    let message = decode_payload(payload)?;
    Ok(Some(Decoded {
        message,
        consumed: frame_len,
    }))
}

/// Builds the 5-byte preamble for a payload of the given little-endian `length`.
fn encode_preamble(length: [u8; LENGTH_LEN]) -> [u8; PREAMBLE_LEN] {
    // The check byte is the XOR of the four length bytes (P4Java
    // `RpcPacketPreamble`); the server rejects a frame whose check disagrees.
    let check = length[0] ^ length[1] ^ length[2] ^ length[3];
    [check, length[0], length[1], length[2], length[3]]
}

/// Reads and validates a preamble, returning the payload length it declares.
///
/// Returns `Ok(None)` if fewer than [`PREAMBLE_LEN`] bytes are available, so a
/// caller can wait for the rest of the header.
fn decode_preamble(input: &[u8]) -> Result<Option<usize>, FrameError> {
    let Some(header) = input.get(..PREAMBLE_LEN) else {
        return Ok(None);
    };

    let check = header[0];
    let length: [u8; LENGTH_LEN] = header[1..PREAMBLE_LEN]
        .try_into()
        .expect("slice of PREAMBLE_LEN - 1 is exactly LENGTH_LEN bytes");

    let expected = length[0] ^ length[1] ^ length[2] ^ length[3];
    if check != expected {
        return Err(FrameError::checksum(expected, check));
    }

    Ok(Some(decode_u32_le(length) as usize))
}

/// Parses a payload slice into a [`Message`], requiring it to consume exactly.
fn decode_payload(mut payload: &[u8]) -> Result<Message, FrameError> {
    let mut message = Message::new();
    while !payload.is_empty() {
        let (field, rest) = decode_field(payload)?;
        message.push(field);
        payload = rest;
    }
    Ok(message)
}

/// Parses one field from the front of `payload`, returning it and the remainder.
fn decode_field(payload: &[u8]) -> Result<(Field, &[u8]), FrameError> {
    // Name: bytes up to and including the first null terminator.
    let name_end = find_null(payload).ok_or(FrameError::truncated_field())?;
    let name = payload[..name_end].to_vec();
    let after_name = &payload[name_end + 1..];

    // Value length: the next four little-endian bytes.
    let length_bytes: [u8; LENGTH_LEN] = after_name
        .get(..LENGTH_LEN)
        .ok_or(FrameError::truncated_field())?
        .try_into()
        .expect("slice of LENGTH_LEN is exactly LENGTH_LEN bytes");
    let value_len = decode_u32_le(length_bytes) as usize;
    let after_length = &after_name[LENGTH_LEN..];

    // Value: exactly `value_len` bytes, then a single null terminator.
    let value = after_length
        .get(..value_len)
        .ok_or(FrameError::truncated_field())?
        .to_vec();
    let after_value = &after_length[value_len..];
    match after_value.first() {
        Some(0) => Ok((Field { name, value }, &after_value[1..])),
        Some(byte) => Err(FrameError::missing_terminator(*byte)),
        None => Err(FrameError::truncated_field()),
    }
}

/// Returns the index of the first null byte in `bytes`, if any.
fn find_null(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&byte| byte == 0)
}

/// Encodes a `u32` as four little-endian bytes (`P4Java` `RpcPacket.encodeInt4`).
fn encode_u32_le(value: u32) -> [u8; LENGTH_LEN] {
    value.to_le_bytes()
}

/// Decodes four little-endian bytes into a `u32` (`P4Java` `RpcPacket.decodeInt4`).
fn decode_u32_le(bytes: [u8; LENGTH_LEN]) -> u32 {
    u32::from_le_bytes(bytes)
}

/// Narrows a field value's byte length to the `u32` the wire format requires.
///
/// # Panics
/// Panics if a single value exceeds [`u32::MAX`] bytes. Such a value cannot be
/// framed and indicates a programming error upstream, not recoverable input.
fn value_len_u32(len: usize) -> u32 {
    u32::try_from(len).expect("field value length exceeds u32::MAX, cannot be framed")
}

/// Narrows a payload's byte length to the `u32` the preamble requires.
///
/// # Panics
/// Panics if the payload exceeds [`u32::MAX`] bytes, which the length field
/// cannot represent. See [`encode`].
fn payload_u32(len: usize) -> [u8; LENGTH_LEN] {
    let len = u32::try_from(len).expect("message payload exceeds u32::MAX, cannot be framed");
    encode_u32_le(len)
}

/// The failure class behind a [`FrameError`], kept private for future-proofing.
///
/// Exposing the enum directly would lock every caller into matching all of our
/// internal failure modes; instead [`FrameError`] surfaces only `is_*` helpers
/// (M-ERRORS-CANONICAL-STRUCTS).
#[derive(Debug)]
enum FrameErrorKind {
    /// The preamble's check byte disagreed with its length bytes.
    Checksum { expected: u8, found: u8 },

    /// A field ended before its name, length, or value was complete.
    TruncatedField,

    /// A field's value was not followed by its required null terminator.
    MissingTerminator { found: u8 },
}

/// An error from decoding malformed RPC frame bytes.
///
/// A `FrameError` means the bytes present are not a valid frame, as opposed to a
/// merely incomplete buffer (which [`decode`] reports as `Ok(None)`). Inspect the
/// cause with [`is_checksum`](Self::is_checksum),
/// [`is_truncated_field`](Self::is_truncated_field), or
/// [`is_missing_terminator`](Self::is_missing_terminator).
pub struct FrameError {
    kind: FrameErrorKind,
}

impl FrameError {
    /// Creates a checksum-mismatch error from the expected and found check bytes.
    fn checksum(expected: u8, found: u8) -> Self {
        Self {
            kind: FrameErrorKind::Checksum { expected, found },
        }
    }

    /// Creates a truncated-field error.
    fn truncated_field() -> Self {
        Self {
            kind: FrameErrorKind::TruncatedField,
        }
    }

    /// Creates a missing-terminator error from the byte found where a null was due.
    fn missing_terminator(found: u8) -> Self {
        Self {
            kind: FrameErrorKind::MissingTerminator { found },
        }
    }

    /// Returns `true` if the preamble's check byte did not match its length.
    #[must_use]
    pub const fn is_checksum(&self) -> bool {
        matches!(self.kind, FrameErrorKind::Checksum { .. })
    }

    /// Returns `true` if a field ended before its name, length, or value was complete.
    #[must_use]
    pub const fn is_truncated_field(&self) -> bool {
        matches!(self.kind, FrameErrorKind::TruncatedField)
    }

    /// Returns `true` if a field value lacked its required null terminator.
    #[must_use]
    pub const fn is_missing_terminator(&self) -> bool {
        matches!(self.kind, FrameErrorKind::MissingTerminator { .. })
    }
}

/// Renders the frame error as a single diagnostic line.
impl Display for FrameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.kind {
            FrameErrorKind::Checksum { expected, found } => write!(
                f,
                "rpc preamble checksum mismatch: expected {expected:#04x}, found {found:#04x}"
            ),
            FrameErrorKind::TruncatedField => {
                write!(
                    f,
                    "rpc field ended before its name, length, or value was complete"
                )
            }
            FrameErrorKind::MissingTerminator { found } => write!(
                f,
                "rpc field value missing its null terminator, found {found:#04x}"
            ),
        }
    }
}

/// A `Debug` view that forwards to the private kind for a readable dump.
impl Debug for FrameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl std::error::Error for FrameError {}

/// A `Debug` adapter that renders a byte slice as escaped text.
///
/// Used by [`Field`]'s `Debug` so names and values read as the ASCII they almost
/// always are, with non-printable bytes shown as escapes rather than numbers.
struct DisplayBytes<'a>(&'a [u8]);

impl Debug for DisplayBytes<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "\"")?;
        for byte in self.0 {
            for escaped in std::ascii::escape_default(*byte) {
                write!(f, "{}", escaped as char)?;
            }
        }
        write!(f, "\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact 23 bytes of a real `func = release2` packet, preamble included.
    ///
    /// Captured from a live `p4 info` exchange against the `p4d` oracle and saved
    /// at `protocol-lab/captures/02-rpc-frame-release2.bin`. This is the golden
    /// frame: it proves the codec matches real wire bytes, not just itself.
    const GOLDEN_RELEASE2: &[u8] = &[
        0x12, 0x12, 0x00, 0x00, 0x00, // preamble: check 0x12, length 18 (LE)
        0x66, 0x75, 0x6e, 0x63, 0x00, // "func\0"
        0x08, 0x00, 0x00, 0x00, // value length 8 (LE)
        0x72, 0x65, 0x6c, 0x65, 0x61, 0x73, 0x65, 0x32, // "release2"
        0x00, // value terminator
    ];

    #[test]
    fn round_trip_preserves_fields() {
        let original = Message::new()
            .with("func", "user-info")
            .with("prog", "p4")
            .with("client", "Titan");

        let bytes = encode(&original);
        let decoded = decode(&bytes).unwrap().expect("a complete frame");

        assert_eq!(decoded.message, original);
        assert_eq!(decoded.consumed, bytes.len());
    }

    #[test]
    fn round_trip_preserves_empty_and_binary_values() {
        // Perforce sends empty values (e.g. `serverID`) and non-UTF-8 bytes; both
        // must survive a round trip untouched.
        let original = Message::new()
            .with("serverID", "")
            .with("blob", [0x00u8, 0xff, 0x7f, 0x80])
            .with("", "empty-name");

        let bytes = encode(&original);
        let decoded = decode(&bytes).unwrap().expect("a complete frame");
        assert_eq!(decoded.message, original);
    }

    #[test]
    fn golden_frame_encodes_to_real_wire_bytes() {
        // Encoding the known field must reproduce the captured bytes exactly.
        let message = Message::new().with("func", "release2");
        assert_eq!(encode(&message), GOLDEN_RELEASE2);
    }

    #[test]
    fn golden_frame_decodes_to_expected_field() {
        // Decoding the captured bytes must recover exactly `func = release2`.
        let decoded = decode(GOLDEN_RELEASE2).unwrap().expect("a complete frame");
        assert_eq!(decoded.consumed, GOLDEN_RELEASE2.len());
        assert_eq!(decoded.message.fields().len(), 1);
        assert_eq!(decoded.message.value(b"func"), Some(&b"release2"[..]));
    }

    #[test]
    fn decode_reports_partial_buffers_as_incomplete() {
        let bytes = encode(&Message::new().with("func", "release2"));

        // A buffer shorter than the preamble cannot even be sized.
        assert_eq!(decode(&bytes[..3]).unwrap(), None);
        // A buffer with the preamble but a partial payload also waits.
        assert_eq!(decode(&bytes[..PREAMBLE_LEN + 2]).unwrap(), None);
    }

    #[test]
    fn decode_drains_coalesced_frames_one_at_a_time() {
        // TCP may deliver several frames in one read; decoding must split them by
        // length and never read past the frame at the front.
        let first = encode(&Message::new().with("func", "release2"));
        let second = encode(&Message::new().with("func", "user-info").with("prog", "p4"));

        let mut buffer = first.clone();
        buffer.extend_from_slice(&second);

        let head = decode(&buffer).unwrap().expect("first frame");
        assert_eq!(head.consumed, first.len());
        assert_eq!(head.message.value(b"func"), Some(&b"release2"[..]));

        let tail = decode(&buffer[head.consumed..])
            .unwrap()
            .expect("second frame");
        assert_eq!(tail.consumed, second.len());
        assert_eq!(tail.message.value(b"prog"), Some(&b"p4"[..]));
    }

    #[test]
    fn decode_rejects_a_bad_checksum() {
        let mut bytes = encode(&Message::new().with("func", "release2"));
        bytes[0] ^= 0xff; // corrupt only the check byte

        let err = decode(&bytes).unwrap_err();
        assert!(err.is_checksum());
    }

    #[test]
    fn decode_rejects_a_value_running_past_the_payload() {
        // Claim a value far longer than the payload actually carries. The field
        // parser must refuse rather than read out of bounds.
        let mut bytes = encode(&Message::new().with("func", "x"));
        // Overwrite the field's value-length bytes (after preamble + "func\0").
        let length_at = PREAMBLE_LEN + b"func\0".len();
        bytes[length_at..length_at + LENGTH_LEN].copy_from_slice(&0xffff_u32.to_le_bytes());
        // The preamble check still matches its own length, so this is a payload
        // error, not a checksum error; fix the preamble to a self-consistent one.
        let payload_len = u32::try_from(bytes.len() - PREAMBLE_LEN).unwrap();
        bytes[..PREAMBLE_LEN].copy_from_slice(&encode_preamble(payload_len.to_le_bytes()));

        let err = decode(&bytes).unwrap_err();
        assert!(err.is_truncated_field());
    }

    #[test]
    fn preamble_check_is_xor_of_length_bytes() {
        // Mirror the captured preambles: check must equal the XOR of L0..L3.
        for payload_len in [0u32, 18, 235, 241, 0x0001_0203] {
            let preamble = encode_preamble(payload_len.to_le_bytes());
            let expected = preamble[1] ^ preamble[2] ^ preamble[3] ^ preamble[4];
            assert_eq!(preamble[0], expected);
            assert_eq!(
                decode_preamble(&preamble).unwrap(),
                Some(payload_len as usize)
            );
        }
    }

    #[test]
    fn debug_renders_fields_as_text() {
        let field = Field::new("func", "release2");
        let rendered = format!("{field:?}");
        assert!(rendered.contains("func"));
        assert!(rendered.contains("release2"));
    }
}
