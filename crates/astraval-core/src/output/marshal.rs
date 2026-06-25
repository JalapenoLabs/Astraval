// SPDX-License-Identifier: Apache-2.0

//! Encoder for the subset of Python's `marshal` format that `p4 -G` emits.
//!
//! Scripting clients (`P4Python` and friends) consume `p4 -G` output as a stream
//! of Python objects in the `marshal` serialization format. `marshal` is a
//! public, documented format: each value is a one-byte type tag followed by that
//! type's payload, and the standard library's `marshal` module is the canonical
//! reference (<https://docs.python.org/3/library/marshal.html>, and the type tags
//! in `CPython`'s `Python/marshal.c`). Reproducing it here is therefore clean-room:
//! we implement a public wire format and match the bytes the open `p4` client
//! emits, never anything derived from the closed server.
//!
//! # What `p4 -G` actually puts on the wire
//!
//! For a tagged record `p4` writes exactly one marshalled dictionary whose keys
//! and values are all strings. Observed against the real client
//! (`p4 -p 1666 -G info`, captured in `protocol-lab/captures/`), the byte stream
//! for one record is:
//!
//! ```text
//! '{'                      one TYPE_DICT byte (0x7b) opens the dict
//! ( <string> <string> )*   key then value, repeated, in insertion order
//! '0'                      one TYPE_NULL byte (0x30) terminates the dict
//! ```
//!
//! and each `<string>` is:
//!
//! ```text
//! 's'                      one TYPE_STRING byte (0x73)
//! <u32 length, LE>         four bytes, little-endian byte length
//! <bytes>                  exactly that many raw bytes
//! ```
//!
//! `p4` uses the unversioned `marshal` tags (no high `FLAG_REF` bit, no
//! interned-string `TYPE_INTERNED`/`TYPE_STRINGREF` tags), which is what
//! `marshal.load` reads back as plain `str`/`bytes`. We encode the same way so a
//! `P4Python` client deserializes our output identically to real `p4`'s.
//!
//! Only the string-valued dictionary needed by tagged command output is
//! implemented; integers and the other `marshal` types are intentionally left
//! out until a command needs them, at which point their tags are added here.

use std::io::{self, Write};

/// `marshal` type tag opening a dictionary (`'{'`, `CPython` `TYPE_DICT`).
const TYPE_DICT: u8 = b'{';

/// `marshal` type tag for a byte string (`'s'`, `CPython` `TYPE_STRING`).
///
/// `CPython` calls this `TYPE_STRING`; `marshal` loads it back as `bytes`, which is
/// exactly how `P4Python` receives `p4 -G` string fields. We use it for every key
/// and value, matching the real client.
const TYPE_STRING: u8 = b's';

/// `marshal` type tag for `None` (`'0'`, `CPython` `TYPE_NULL`).
///
/// `p4` writes a single `TYPE_NULL` to mark the end of a dictionary's entries
/// rather than prefixing a count, so the reader consumes key/value pairs until it
/// meets this terminator. We emit it once after the last value.
const TYPE_DICT_TERMINATOR: u8 = b'0';

/// Writes one `marshal` string: the `'s'` tag, a little-endian `u32` length, then the bytes.
///
/// This is the single string primitive both keys and values go through, so the
/// length-prefix encoding lives in exactly one place.
///
/// # Errors
///
/// Returns any [`io::Error`] from writing to `out`.
///
/// # Panics
///
/// Panics if `bytes` is longer than [`u32::MAX`]. `marshal`'s `TYPE_STRING`
/// carries a 32-bit length, so a longer string cannot be represented; for the
/// short, fixed field values `p4` emits this is unreachable, and a silent
/// truncation would corrupt the stream, so we fail loudly (rust.md,
/// M-PANIC-ON-BUG).
fn write_string(out: &mut (impl Write + ?Sized), bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len())
        .expect("marshal string length must fit in u32; p4 fields are short");

    out.write_all(&[TYPE_STRING])?;
    out.write_all(&length.to_le_bytes())?;
    out.write_all(bytes)
}

/// Writes `fields` as one marshalled dictionary of strings, exactly as `p4 -G`.
///
/// Emits the opening `TYPE_DICT`, each key and value as a `marshal` string in the
/// given order, and the closing `TYPE_NULL` terminator. Iteration order is the
/// caller's order, which preserves the field order `p4` clients expect.
///
/// # Errors
///
/// Returns any [`io::Error`] from writing to `out`.
pub fn write_string_dict<'a, I>(out: &mut (impl Write + ?Sized), fields: I) -> io::Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    out.write_all(&[TYPE_DICT])?;
    for (key, value) in fields {
        write_string(out, key.as_bytes())?;
        write_string(out, value.as_bytes())?;
    }
    out.write_all(&[TYPE_DICT_TERMINATOR])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-field dict matches the exact `marshal` byte layout.
    ///
    /// The expected bytes are assembled by hand from the documented tags so the
    /// test pins the wire format, not merely the encoder's own behavior. This is
    /// the same layout observed in `protocol-lab/captures/raw-G-info.bin`.
    #[test]
    fn single_field_dict_has_exact_bytes() {
        let mut buffer = Vec::new();
        write_string_dict(&mut buffer, [("code", "stat")]).expect("writing to a Vec cannot fail");

        // {  s len=4  c o d e   s len=4  s t a t   0
        let expected = [
            b'{', b's', 4, 0, 0, 0, b'c', b'o', b'd', b'e', b's', 4, 0, 0, 0, b's', b't', b'a',
            b't', b'0',
        ];
        assert_eq!(buffer, expected);
    }

    /// An empty dict is just the open tag and the terminator, back to back.
    #[test]
    fn empty_dict_is_open_then_terminator() {
        let mut buffer = Vec::new();
        write_string_dict(&mut buffer, []).expect("writing to a Vec cannot fail");
        assert_eq!(buffer, [b'{', b'0']);
    }

    /// The length prefix is little-endian and counts bytes, not characters.
    ///
    /// A multibyte UTF-8 value proves the prefix is a byte count: `"é"` is two
    /// bytes (`0xC3 0xA9`), so the length must read `2`, not `1`.
    #[test]
    fn string_length_is_little_endian_byte_count() {
        let mut buffer = Vec::new();
        write_string(&mut buffer, "é".as_bytes()).expect("writing to a Vec cannot fail");
        assert_eq!(buffer, [b's', 2, 0, 0, 0, 0xC3, 0xA9]);
    }
}
