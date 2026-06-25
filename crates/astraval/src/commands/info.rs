// SPDX-License-Identifier: Apache-2.0

//! The `info` command: report connection and client details.
//!
//! In Perforce, `p4 info` prints server address, user, client, and similar facts
//! about the current connection. Astraval mirrors that command. This module's job
//! in issue #5 is narrow: prove the output rendering layer end to end by emitting
//! a single representative record through all three modes (human, `-ztag`, `-G`).
//!
//! The field *values* here are placeholders, not real server facts. Producing the
//! genuine `info` content needs a live server connection and lands in issue #14;
//! that issue replaces [`representative_record`] with the real fields while
//! reusing this exact rendering path and human layout. The record shape and field
//! names are modeled on real `p4 info` output (captured in
//! `protocol-lab/captures/`) so #14 is a content swap, not a rewrite.

use std::io::{self, Write};

use astraval_core::Result;
use astraval_core::config::ResolvedConfig;
use astraval_core::output::{HumanReport, OutputMode, Record, render};

/// Runs the `info` command, rendering its record in the selected output `mode`.
///
/// Builds the (currently representative) record, then renders it to stdout through
/// the shared output layer: human text, `-ztag` lines, or `-G` marshalled bytes.
/// The resolved `_config` is accepted now so the signature matches every other
/// command and so issue #14 can read the connection details straight from it to
/// fill the real fields. It is borrowed, not consumed.
///
/// # Errors
///
/// Returns a command-class [`astraval_core::Error`] if writing the rendered output
/// to stdout fails, so the failure flows through the shared error path
/// (`crate::exit::report`) like every other command.
pub fn run(_config: &ResolvedConfig, mode: OutputMode) -> Result<()> {
    let records = [representative_record()];

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    render(mode, &records, &InfoHuman, &mut handle).map_err(|error| {
        astraval_core::Error::command("info: failed to write output").with_source(error)
    })
}

/// Builds the placeholder `info` record proving the renderer (issue #14 fills it).
///
/// The field names mirror real `p4 info`'s tagged keys (`userName`,
/// `serverVersion`, ...) so issue #14 only has to supply real values. The values
/// are obvious placeholders so no one mistakes this stub for live server output.
fn representative_record() -> Record {
    let mut record = Record::new();
    record.push("userName", "placeholder-user");
    record.push("clientName", "placeholder-client");
    record.push(
        "serverVersion",
        "Astraval (info not implemented yet, see issue #14)",
    );
    record.push("serverLicense", "none");
    record
}

/// The human-readable rendering of `info`, mirroring `p4 info`'s labelled lines.
///
/// `p4 info` prints `Label: value` lines (`User name: alice`). This stub follows
/// that shape for the few representative fields; issue #14 extends the label map
/// as it adds real fields. Tagged and marshalled modes never reach here, they use
/// the generic renderings in [`astraval_core::output`].
struct InfoHuman;

impl HumanReport for InfoHuman {
    fn write_human(&self, out: &mut dyn Write, records: &[Record]) -> io::Result<()> {
        for record in records {
            for (key, value) in record.fields() {
                writeln!(out, "{}: {value}", human_label(key))?;
            }
        }
        Ok(())
    }
}

/// Maps a tagged field key to the human label `p4 info` prints for it.
///
/// Unknown keys fall back to the raw key, so a field added in issue #14 still
/// renders something readable before its label is added here.
fn human_label(key: &str) -> &str {
    match key {
        "userName" => "User name",
        "clientName" => "Client name",
        "serverVersion" => "Server version",
        "serverLicense" => "Server license",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The representative record carries the placeholder fields issue #14 replaces.
    #[test]
    fn representative_record_has_placeholder_fields() {
        let record = representative_record();
        let fields: Vec<_> = record.fields().collect();
        assert_eq!(fields[0], ("userName", "placeholder-user"));
        assert_eq!(fields[3], ("serverLicense", "none"));
    }

    /// The human view renders labelled lines, mapping known keys to `p4` labels.
    #[test]
    fn human_view_renders_labelled_lines() {
        let mut out = Vec::new();
        InfoHuman
            .write_human(&mut out, &[representative_record()])
            .expect("writing to a Vec cannot fail");
        let text = String::from_utf8(out).expect("output is UTF-8");

        assert!(text.starts_with("User name: placeholder-user\n"));
        assert!(text.contains("Server license: none\n"));
    }

    /// An unmapped key falls back to its raw name rather than being dropped.
    #[test]
    fn unknown_key_falls_back_to_its_raw_name() {
        assert_eq!(human_label("peerAddress"), "peerAddress");
    }
}
