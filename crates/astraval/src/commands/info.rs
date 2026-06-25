// SPDX-License-Identifier: Apache-2.0

//! The `info` command: report connection and client details.
//!
//! In Perforce, `p4 info` prints server address, user, client, and similar
//! facts about the current connection. Astraval mirrors that command, but the
//! real implementation (which needs a live server connection) lands in a later
//! milestone. For now this is a deliberate stub: it explains that the command
//! is not wired up yet and signals that clearly through the process exit code.

use std::process::ExitCode;

use crate::cli::GlobalOptions;

/// Exit code returned by commands that are declared but not yet implemented.
///
/// It is intentionally distinct from `0` (success) and from clap's `2` (usage
/// error), so scripts and the test suite can tell "this command is a stub" apart
/// from "this command failed" or "you typed the arguments wrong". The value is
/// otherwise arbitrary within the conventional 1..=125 range for exit codes.
pub const NOT_IMPLEMENTED_EXIT_CODE: u8 = 64;

/// Runs the `info` stub and returns the not-implemented exit code.
///
/// The `_global` options are accepted now so the signature matches every other
/// command once the dispatcher passes resolved configuration through. They are
/// borrowed, not consumed, until the real implementation (issue #14) reads them.
pub fn run(_global: &GlobalOptions) -> ExitCode {
    eprintln!(
        "astraval info: not implemented yet. Connecting to a server and reporting \
         its details arrives in a later milestone."
    );
    ExitCode::from(NOT_IMPLEMENTED_EXIT_CODE)
}
