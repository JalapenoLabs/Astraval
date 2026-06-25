// SPDX-License-Identifier: Apache-2.0

//! The output rendering layer: one tagged record, three renderings.
//!
//! Underneath, every Astraval command produces the same thing `p4` does: tagged
//! data, an ordered set of key/value fields per record. This module captures that
//! as [`Record`] and renders a sequence of records in whichever of `p4`'s three
//! output modes the user selected:
//!
//! - [`OutputMode::Human`] is the default, friendly text a person reads. Its
//!   shape is bespoke per command, so each command supplies its own formatting by
//!   implementing [`HumanReport`]; this module does not guess a generic layout.
//! - [`OutputMode::Tagged`] is `p4 -ztag`: every field printed as a
//!   `... <key> <value>` line, with a blank line between records.
//! - [`OutputMode::Marshalled`] is `p4 -G`: each record as one Python `marshal`
//!   dictionary of strings, the format scripting clients consume.
//!
//! The tagged and marshalled renderings are generic over the fields and live
//! here; only the human view is delegated. A command therefore writes its records
//! once and calls [`render`], which picks the mode and, for human mode, calls back
//! into the command's own formatting.
//!
//! # Why the byte format is pinned to real `p4`
//!
//! Protocol and output compatibility is the product: scripts and tools built for
//! Perforce must read Astraval's output unchanged. Both the `-ztag` line shape and
//! the `-G` marshal byte stream were captured from the real client against a live
//! `p4d` oracle (see `protocol-lab/captures/`) and are reproduced here exactly.
//! The marshal encoding details, including the type tags, live in [`marshal`].
//!
//! # Examples
//!
//! ```
//! use astraval_core::output::{HumanReport, OutputMode, Record, render};
//! use std::io::{self, Write};
//!
//! // A command builds its records, then describes its own human view.
//! struct Greeting;
//! impl HumanReport for Greeting {
//!     fn write_human(&self, out: &mut dyn Write, records: &[Record]) -> io::Result<()> {
//!         for record in records {
//!             for (key, value) in record.fields() {
//!                 writeln!(out, "{key}: {value}")?;
//!             }
//!         }
//!         Ok(())
//!     }
//! }
//!
//! let mut record = Record::new();
//! record.push("userName", "alice");
//!
//! let mut out = Vec::new();
//! render(OutputMode::Tagged, &[record], &Greeting, &mut out)?;
//! assert_eq!(out, b"... userName alice\r\n\r\n");
//! # Ok::<(), std::io::Error>(())
//! ```

mod marshal;

use std::io::{self, Write};

/// One tagged record: an ordered set of key/value string fields.
///
/// This is the unit every command emits and every output mode renders. Order is
/// significant and preserved: `p4` prints fields in a fixed, meaningful order, and
/// both the tagged and marshalled renderings reproduce that order, so [`Record`]
/// keeps fields in insertion order rather than sorting or deduplicating them.
///
/// Fields are plain `String` pairs. The tagged data model `p4` exposes is
/// untyped on the wire (every field is text), so a stringly-typed record matches
/// it directly and renders to all three modes without per-type handling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    /// Key/value fields in insertion order; see the type docs on why order matters.
    fields: Vec<(String, String)>,
}

impl Record {
    /// Creates an empty record with no fields.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a `key`/`value` field, preserving insertion order.
    ///
    /// Duplicate keys are kept as distinct fields rather than overwritten, because
    /// `p4`'s tagged data can legitimately repeat a key (for example a list field
    /// emitted as `key0`, `key1`); this method stays faithful to that by never
    /// collapsing fields.
    pub fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.fields.push((key.into(), value.into()));
    }

    /// Returns the fields in insertion order as `(key, value)` string slices.
    ///
    /// Commands use this to drive their own human formatting in
    /// [`HumanReport::write_human`].
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

/// Which of `p4`'s three output renderings to produce.
///
/// Selected from the global flags by [`OutputMode::from_flags`], which encodes
/// `p4`'s precedence: marshalled output (`-G`) wins over tagged (`-ztag`), which
/// wins over the human default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Friendly, command-specific text for a person to read (the default).
    Human,

    /// `p4 -ztag`: each field as a `... <key> <value>` line, records blank-separated.
    Tagged,

    /// `p4 -G`: each record as one Python `marshal` dictionary of strings.
    Marshalled,
}

impl OutputMode {
    /// Selects the output mode from the marshalled and tagged global flags.
    ///
    /// `marshalled` is the `-G` flag; `tagged` is whether `-z`'s argument turned
    /// tagged mode on. The precedence matches `p4`: `-G` wins if set, otherwise
    /// `-ztag` if tagged mode is on, otherwise the human default. Passing both
    /// flags therefore yields [`OutputMode::Marshalled`], exactly as real `p4`
    /// resolves the same combination.
    #[must_use]
    pub const fn from_flags(marshalled: bool, tagged: bool) -> Self {
        if marshalled {
            Self::Marshalled
        } else if tagged {
            Self::Tagged
        } else {
            Self::Human
        }
    }
}

/// A command's own human-readable rendering of its records.
///
/// Human output is bespoke per command (`p4 info` prints `User name: ...`, while
/// other commands look nothing alike), so the generic renderer cannot produce it.
/// Each command implements this trait to format the very same [`Record`] set it
/// emits; [`render`] calls it only when the selected mode is [`OutputMode::Human`].
pub trait HumanReport {
    /// Writes the human-readable view of `records` to `out`.
    ///
    /// The implementation owns its layout entirely: which fields to show, their
    /// labels, and their order. It typically reads each [`Record::fields`].
    ///
    /// # Errors
    ///
    /// Returns any [`io::Error`] from writing to `out`.
    fn write_human(&self, out: &mut dyn Write, records: &[Record]) -> io::Result<()>;
}

/// The literal prefix `p4 -ztag` writes before every tagged field.
///
/// Each tagged line is `... <key> <value>`; the marker is three dots and a space.
/// Captured verbatim from `p4 -p 1666 -ztag info`
/// (`protocol-lab/captures/raw-ztag-info.txt`).
const TAGGED_FIELD_MARKER: &str = "... ";

/// The line terminator `p4 -ztag` writes, including on the blank record separator.
///
/// The real client emits CRLF (`\r\n`) line endings in tagged mode, observed in
/// the capture above where every line ends `\r\n`. Matching this exactly is the
/// point of compatibility, so we do not substitute the platform newline.
const TAGGED_LINE_ENDING: &str = "\r\n";

/// Renders `records` in the selected `mode`, writing to `out`.
///
/// For [`OutputMode::Human`] this defers to `human`'s [`HumanReport::write_human`];
/// for [`OutputMode::Tagged`] and [`OutputMode::Marshalled`] it uses the generic
/// renderings pinned to real `p4`'s bytes. This is the single entry point a
/// command calls after building its records.
///
/// # Errors
///
/// Returns any [`io::Error`] from writing to `out`, including an error propagated
/// from the command's own [`HumanReport::write_human`].
pub fn render(
    mode: OutputMode,
    records: &[Record],
    human: &dyn HumanReport,
    out: &mut dyn Write,
) -> io::Result<()> {
    match mode {
        OutputMode::Human => human.write_human(out, records),
        OutputMode::Tagged => write_tagged(out, records),
        OutputMode::Marshalled => write_marshalled(out, records),
    }
}

/// Writes records in `p4 -ztag` form: `... <key> <value>` lines, blank-separated.
///
/// Every field becomes one `... key value` line; a blank line follows each record,
/// matching the trailing blank line real `p4` emits after its single `info`
/// record. Line endings are CRLF to match the captured client output.
fn write_tagged(out: &mut (impl Write + ?Sized), records: &[Record]) -> io::Result<()> {
    for record in records {
        for (key, value) in record.fields() {
            write!(
                out,
                "{TAGGED_FIELD_MARKER}{key} {value}{TAGGED_LINE_ENDING}"
            )?;
        }
        // `p4` separates records with a blank line and also emits one after the
        // final record, so the separator is written unconditionally per record.
        write!(out, "{TAGGED_LINE_ENDING}")?;
    }
    Ok(())
}

/// Writes records in `p4 -G` form: one Python `marshal` string-dict per record.
///
/// Each record is encoded back to back with no separator between them, which is
/// how `p4` streams multiple marshalled dictionaries: the reader consumes one
/// complete dict, then the next. The encoding details live in [`marshal`].
fn write_marshalled(out: &mut (impl Write + ?Sized), records: &[Record]) -> io::Result<()> {
    for record in records {
        marshal::write_string_dict(out, record.fields())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a small two-field record used across the rendering tests.
    fn sample_record() -> Record {
        let mut record = Record::new();
        record.push("userName", "alice");
        record.push("serverLicense", "none");
        record
    }

    /// `-G` wins over `-ztag`, which wins over human: `p4`'s exact precedence.
    #[test]
    fn mode_selection_follows_p4_precedence() {
        assert_eq!(OutputMode::from_flags(true, true), OutputMode::Marshalled);
        assert_eq!(OutputMode::from_flags(true, false), OutputMode::Marshalled);
        assert_eq!(OutputMode::from_flags(false, true), OutputMode::Tagged);
        assert_eq!(OutputMode::from_flags(false, false), OutputMode::Human);
    }

    /// Tagged rendering matches `p4 -ztag`: `... key value` lines, CRLF, trailing blank.
    #[test]
    fn tagged_rendering_matches_p4_line_format() {
        let mut out = Vec::new();
        write_tagged(&mut out, &[sample_record()]).expect("writing to a Vec cannot fail");

        let expected = "... userName alice\r\n... serverLicense none\r\n\r\n";
        assert_eq!(String::from_utf8(out).expect("output is ASCII"), expected);
    }

    /// Marshalled rendering matches the exact `marshal` byte stream `p4 -G` emits.
    ///
    /// The expected bytes are built by hand from the documented `marshal` tags so
    /// the test pins the wire format. This layout is the same one captured from the
    /// real client in `protocol-lab/captures/raw-G-info.bin`: `{`, then each key
    /// and value as `s` + little-endian `u32` length + bytes, then the `0`
    /// terminator.
    #[test]
    fn marshalled_rendering_matches_p4_byte_stream() {
        let mut out = Vec::new();
        write_marshalled(&mut out, &[sample_record()]).expect("writing to a Vec cannot fail");

        let mut expected = Vec::new();
        expected.push(b'{');
        for (key, value) in [("userName", "alice"), ("serverLicense", "none")] {
            for field in [key, value] {
                expected.push(b's');
                expected.extend_from_slice(
                    &u32::try_from(field.len())
                        .expect("field fits in u32")
                        .to_le_bytes(),
                );
                expected.extend_from_slice(field.as_bytes());
            }
        }
        expected.push(b'0');

        assert_eq!(out, expected);
    }

    /// Multiple marshalled records stream back to back with no separator.
    #[test]
    fn marshalled_records_stream_without_separators() {
        let mut single = Record::new();
        single.push("k", "v");

        let mut out = Vec::new();
        write_marshalled(&mut out, &[single.clone(), single])
            .expect("writing to a Vec cannot fail");

        // One dict is `{ s..k s..v 0`; two identical dicts are that twice, joined.
        let one = [b'{', b's', 1, 0, 0, 0, b'k', b's', 1, 0, 0, 0, b'v', b'0'];
        let mut expected = Vec::new();
        expected.extend_from_slice(&one);
        expected.extend_from_slice(&one);
        assert_eq!(out, expected);
    }

    /// In human mode `render` defers entirely to the command's own formatting.
    #[test]
    fn render_human_mode_delegates_to_the_command() {
        struct Labelled;
        impl HumanReport for Labelled {
            fn write_human(&self, out: &mut dyn Write, records: &[Record]) -> io::Result<()> {
                for record in records {
                    for (key, value) in record.fields() {
                        writeln!(out, "{key} -> {value}")?;
                    }
                }
                Ok(())
            }
        }

        let mut out = Vec::new();
        render(OutputMode::Human, &[sample_record()], &Labelled, &mut out)
            .expect("writing to a Vec cannot fail");

        assert_eq!(
            String::from_utf8(out).expect("output is ASCII"),
            "userName -> alice\nserverLicense -> none\n"
        );
    }
}
