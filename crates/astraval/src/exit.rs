// SPDX-License-Identifier: Apache-2.0

//! The single error path: mapping client errors to exit codes and stderr.
//!
//! Every `astraval` command flows its failures through one function,
//! [`report`], which renders a `p4`-style line to stderr and returns the
//! process [`ExitCode`] that matches the failure class. Centralizing this is the
//! whole point of the error spine: scripts keying on exit codes, and humans
//! reading stderr, see one consistent contract no matter which command failed.
//!
//! # Exit-code mapping
//!
//! | Code | Meaning                | Source / rationale                       |
//! |------|------------------------|------------------------------------------|
//! | 0    | success                | `p4` returns 0 on success.               |
//! | 1    | general command failure| `p4` returns 1 for a failed command.     |
//! | 2    | usage error            | clap's default; matches `p4`'s arg error.|
//! | 69   | connection failure     | `EX_UNAVAILABLE` from BSD `sysexits.h`.   |
//! | 77   | authentication failure | `EX_NOPERM` from BSD `sysexits.h`.        |
//!
//! Real `p4` collapses almost everything into `0`, `1`, and `2`: success, a
//! failed command, and a bad command line. Astraval keeps that core contract
//! exactly, because that is what existing Perforce scripts already test for.
//! For the two failure classes `p4` does not give a distinct numeric code,
//! connection and authentication, we borrow the long-standing BSD `sysexits.h`
//! conventions (`<sysexits.h>`: `EX_UNAVAILABLE = 69`, `EX_NOPERM = 77`). Those
//! values are widely understood by shell tooling, sit in the conventional
//! `1..=125` user range, and do not collide with `1` or `2`, so a script can
//! branch on "could not connect" versus "not authorized" when it wants to while
//! still treating any non-zero code as failure.

use std::process::ExitCode;

use astraval_core::{Error, ErrorClass};

/// Exit code for a general command failure (the catch-all error).
///
/// Matches `p4`, which returns `1` whenever a command runs but fails.
const EXIT_COMMAND_FAILURE: u8 = 1;

/// Exit code for a connection failure: the server could not be reached.
///
/// `EX_UNAVAILABLE` from BSD `sysexits.h`. Chosen because "the service is
/// unavailable" is exactly a refused or unreachable server, and because the
/// value is distinct from `p4`'s `1`/`2` so scripts can single out this case.
const EXIT_CONNECTION_FAILURE: u8 = 69;

/// Exit code for an authentication failure: valid credentials were missing.
///
/// `EX_NOPERM` from BSD `sysexits.h` ("permission denied"). It cleanly captures
/// "the server let me connect but would not let me in" and stays clear of the
/// `p4` core codes.
const EXIT_AUTHENTICATION_FAILURE: u8 = 77;

/// Returns the process exit code for a given error class.
///
/// This is the authoritative mapping documented in the module table. It is kept
/// separate from [`report`] so the mapping can be unit-tested directly, without
/// capturing stderr.
///
/// [`ErrorClass`] is `#[non_exhaustive]`, so a wildcard arm is required. Any
/// class this binary does not yet recognize is treated as a general command
/// failure: it is still a failure (non-zero), and falling back to the safe `1`
/// keeps an older binary working against a newer core that added a class,
/// rather than failing to compile or inventing a meaningless code.
const fn exit_code_for(class: ErrorClass) -> u8 {
    // The explicit `Command` arm and the wildcard share a body on purpose: the
    // named arm documents the intended mapping for the class we have today,
    // while the wildcard is the forward-compatible fallback the
    // `#[non_exhaustive]` enum requires. Collapsing them would hide the
    // documented `Command` mapping, so the duplicate body is deliberate.
    #[expect(
        clippy::match_same_arms,
        reason = "named Command arm documents intent; wildcard is the non_exhaustive fallback"
    )]
    match class {
        ErrorClass::Command => EXIT_COMMAND_FAILURE,
        ErrorClass::Connection => EXIT_CONNECTION_FAILURE,
        ErrorClass::Authentication => EXIT_AUTHENTICATION_FAILURE,
        // Any future, unrecognized class maps to the general command-failure
        // code; see the function-level note on this wildcard.
        _ => EXIT_COMMAND_FAILURE,
    }
}

/// Formats an error as a single `p4`-style diagnostic line.
///
/// The shape is `<severity>: <message>`, for example `error: connection
/// refused`. Perforce prefixes its diagnostics with a severity word; matching
/// that shape keeps Astraval's stderr familiar to anyone who already parses
/// `p4` output, while the exit code (not the text) remains the stable contract
/// scripts should branch on. The message includes the wrapped cause chain via
/// the error's [`std::fmt::Display`].
fn format_diagnostic(error: &Error) -> String {
    format!("{}: {error}", error.class().severity_prefix())
}

/// Renders an error to stderr and returns its matching process exit code.
///
/// This is the single sink every command's failure flows through: it writes one
/// `p4`-style line to stderr and returns the [`ExitCode`] for the error's class.
/// Routing all errors here is what guarantees a uniform contract across
/// commands, so a script sees the same exit code and message shape regardless of
/// which command produced the error.
pub fn report(error: &Error) -> ExitCode {
    eprintln!("{}", format_diagnostic(error));
    ExitCode::from(exit_code_for(error.class()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_failure_maps_to_one() {
        assert_eq!(exit_code_for(ErrorClass::Command), 1);
    }

    #[test]
    fn connection_failure_maps_to_sysexits_unavailable() {
        assert_eq!(exit_code_for(ErrorClass::Connection), 69);
    }

    #[test]
    fn authentication_failure_maps_to_sysexits_noperm() {
        assert_eq!(exit_code_for(ErrorClass::Authentication), 77);
    }

    #[test]
    fn each_class_maps_to_a_distinct_nonzero_code() {
        let command = exit_code_for(ErrorClass::Command);
        let connection = exit_code_for(ErrorClass::Connection);
        let authentication = exit_code_for(ErrorClass::Authentication);

        // Every failure is non-zero so any script treating `!= 0` as failure
        // behaves correctly, and the codes are distinct so a script can also
        // branch on the specific failure class when it wants to.
        for code in [command, connection, authentication] {
            assert_ne!(code, 0);
        }
        assert_ne!(command, connection);
        assert_ne!(command, authentication);
        assert_ne!(connection, authentication);
    }

    #[test]
    fn diagnostic_uses_the_error_severity_prefix() {
        let err = Error::command("file not found in depot");
        assert_eq!(format_diagnostic(&err), "error: file not found in depot");
    }

    #[test]
    fn diagnostic_includes_the_wrapped_cause() {
        let inner =
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        let err = Error::connection("cannot reach server").with_source(inner);

        assert_eq!(
            format_diagnostic(&err),
            "error: cannot reach server: connection refused"
        );
    }

    #[test]
    fn connection_and_authentication_diagnostics_render_their_classes() {
        let connection = Error::connection("no route to host");
        let authentication = Error::authentication("ticket expired");

        assert_eq!(format_diagnostic(&connection), "error: no route to host");
        assert_eq!(format_diagnostic(&authentication), "error: ticket expired");
    }
}
