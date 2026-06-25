// SPDX-License-Identifier: Apache-2.0

//! The error spine shared by every Astraval client operation.
//!
//! This module defines [`Error`], the single library error type that client
//! code returns, and [`ErrorClass`], the small set of failure *classes* that
//! callers act on. The CLI maps each class to a Perforce-compatible process
//! exit code and a `p4`-style stderr line, so scripts keying on exit codes keep
//! working against Astraval as they would against a real Perforce server.
//!
//! # Why a class, not just a message
//!
//! A bare message can be printed but not *acted on*. By carrying an
//! [`ErrorClass`], an error stays structured all the way to the process
//! boundary: the binary inspects [`Error::class`] to choose the exit code,
//! a script inspects that code to branch, and only the human-facing text is
//! rendered from the message. This keeps behavior driven by structure, not by
//! string matching.
//!
//! # Design
//!
//! Following the Astraval Rust guidelines (M-ERRORS-CANONICAL-STRUCTS), the
//! public surface is a single struct rather than an exposed enum. The struct
//! captures a [`Backtrace`] and an optional upstream cause, and hides its
//! discriminant behind [`ErrorClass`] plus `is_*` helpers so adding internal
//! failure modes later never breaks callers. This is the error *spine*; command
//! issues add richer construction helpers and causes as they need them.

use std::backtrace::Backtrace;
use std::error::Error as StdError;
use std::fmt::{self, Debug, Display, Formatter};

/// The broad failure classes Astraval distinguishes for callers and exit codes.
///
/// Real `p4` collapses most failures into a single error code, but it does
/// separate the cases scripts care about most: could the client even *reach*
/// the server, and was it *allowed* in once it did. Astraval keeps those
/// distinctions first-class so the CLI can map them to stable exit codes
/// (see the binary's exit-code module) and so library callers can branch on
/// them without parsing messages.
///
/// The enum is `#[non_exhaustive]`: later milestones may introduce new classes
/// (for example a dedicated "file conflict" class), and existing matches must
/// keep compiling. Callers should prefer the [`Error`] `is_*` helpers and a
/// catch-all arm over matching every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// A general command failure: the request reached the server and was
    /// understood, but could not be completed (the catch-all error class).
    Command,

    /// The client could not establish or maintain a connection to the server.
    ///
    /// This covers DNS failure, refused connections, timeouts, and dropped
    /// sockets: anything that prevents the request from being delivered at all.
    Connection,

    /// The server was reached but rejected the request for lack of valid
    /// credentials (no ticket, expired ticket, or wrong password).
    Authentication,
}

impl ErrorClass {
    /// Returns the `p4`-style severity prefix used when rendering this class.
    ///
    /// Perforce prefixes diagnostic lines with a severity word. Every class
    /// Astraval models today is a hard failure, so each renders as the
    /// uppercase `"error"` prefix that `p4` uses for failed commands. Keeping
    /// this on the class (rather than hard-coding it at the print site) means a
    /// future warning-level class can choose a different prefix in one place.
    #[must_use]
    pub const fn severity_prefix(self) -> &'static str {
        match self {
            Self::Command | Self::Connection | Self::Authentication => "error",
        }
    }
}

/// The single error type returned by Astraval client operations.
///
/// An `Error` pairs a human-readable `message` with an [`ErrorClass`] that
/// drives behavior, a captured [`Backtrace`] for debugging, and an optional
/// upstream `source` it wraps. Construct one with [`Error::command`],
/// [`Error::connection`], or [`Error::authentication`], optionally attaching a
/// cause with [`Error::with_source`].
///
/// # Examples
/// ```
/// use astraval_core::{Error, ErrorClass};
///
/// let err = Error::connection("cannot reach tcp:localhost:1666");
/// assert_eq!(err.class(), ErrorClass::Connection);
/// assert!(err.is_connection());
/// ```
pub struct Error {
    /// The failure class, used to choose exit codes and to branch on behavior.
    class: ErrorClass,

    /// The human-facing summary of what went wrong, rendered to the user.
    message: String,

    /// An optional upstream cause this error wraps, surfaced via [`StdError`].
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,

    /// A backtrace captured at construction, empty unless `RUST_BACKTRACE` is set.
    ///
    /// Capturing is near-free when backtraces are disabled (the default), so we
    /// always capture and let the runtime decide whether to walk the stack.
    backtrace: Backtrace,
}

impl Error {
    /// Creates an error of the given `class` with a human-readable `message`.
    ///
    /// This is the shared constructor the class-specific helpers forward to. It
    /// captures a backtrace and leaves the upstream cause unset; attach one with
    /// [`Error::with_source`].
    fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
            source: None,
            backtrace: Backtrace::capture(),
        }
    }

    /// Creates a general command failure ([`ErrorClass::Command`]).
    ///
    /// Use this for the common case: the server was reached and understood the
    /// request, but the command could not be completed.
    ///
    /// # Examples
    /// ```
    /// use astraval_core::Error;
    /// let err = Error::command("file not found in depot");
    /// assert!(err.is_command());
    /// ```
    pub fn command(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Command, message)
    }

    /// Creates a connection failure ([`ErrorClass::Connection`]).
    ///
    /// Use this when the client cannot reach the server at all: refused
    /// connection, timeout, DNS failure, or a dropped socket.
    ///
    /// # Examples
    /// ```
    /// use astraval_core::Error;
    /// let err = Error::connection("connection refused");
    /// assert!(err.is_connection());
    /// ```
    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Connection, message)
    }

    /// Creates an authentication failure ([`ErrorClass::Authentication`]).
    ///
    /// Use this when the server was reached but rejected the request for lack of
    /// valid credentials: no ticket, an expired ticket, or a wrong password.
    ///
    /// # Examples
    /// ```
    /// use astraval_core::Error;
    /// let err = Error::authentication("your session has expired, please login again");
    /// assert!(err.is_authentication());
    /// ```
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Authentication, message)
    }

    /// Attaches an upstream `source` as the cause of this error.
    ///
    /// The cause is surfaced through [`StdError::source`] for error-chain
    /// inspection and appended when the error is rendered, so the original
    /// failure is not lost behind Astraval's summary message.
    ///
    /// # Examples
    /// ```
    /// use std::error::Error as _;
    /// use std::io;
    /// use astraval_core::Error;
    ///
    /// let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
    /// let err = Error::connection("cannot reach server").with_source(io_err);
    /// assert!(err.source().is_some());
    /// ```
    #[must_use]
    pub fn with_source(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Returns the [`ErrorClass`] of this error.
    ///
    /// The CLI uses this to select a Perforce-compatible exit code; library
    /// callers use it to branch on the kind of failure.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        self.class
    }

    /// Returns the human-readable message describing what went wrong.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns `true` if this is a general command failure.
    #[must_use]
    pub const fn is_command(&self) -> bool {
        matches!(self.class, ErrorClass::Command)
    }

    /// Returns `true` if this is a connection failure.
    #[must_use]
    pub const fn is_connection(&self) -> bool {
        matches!(self.class, ErrorClass::Connection)
    }

    /// Returns `true` if this is an authentication failure.
    #[must_use]
    pub const fn is_authentication(&self) -> bool {
        matches!(self.class, ErrorClass::Authentication)
    }
}

/// Renders the error as a single human-readable line, cause appended.
///
/// The format is the summary message, then `: <cause>` for each wrapped source,
/// matching how callers expect a chained error to read. The backtrace is not
/// rendered here; it is a debugging aid reached through [`Error::backtrace`] via
/// the standard error machinery, not part of the user-facing line.
impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;

        // Append the upstream cause chain so the original failure is visible,
        // e.g. "cannot reach server: connection refused".
        let mut cause = self.source.as_deref().map(|s| s as &dyn StdError);
        while let Some(err) = cause {
            write!(f, ": {err}")?;
            cause = err.source();
        }
        Ok(())
    }
}

/// A `Debug` view that shows the class, message, cause, and backtrace.
///
/// Hand-written rather than derived so the backtrace renders as readable frames
/// instead of the opaque struct dump `derive(Debug)` would produce.
impl Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("class", &self.class)
            .field("message", &self.message)
            .field("source", &self.source)
            .field("backtrace", &self.backtrace)
            .finish()
    }
}

impl StdError for Error {
    /// Returns the wrapped upstream cause, if any, for error-chain inspection.
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_round_trips_through_constructors() {
        assert_eq!(Error::command("x").class(), ErrorClass::Command);
        assert_eq!(Error::connection("x").class(), ErrorClass::Connection);
        assert_eq!(
            Error::authentication("x").class(),
            ErrorClass::Authentication
        );
    }

    #[test]
    fn is_helpers_match_their_class() {
        assert!(Error::command("x").is_command());
        assert!(Error::connection("x").is_connection());
        assert!(Error::authentication("x").is_authentication());

        // A helper must not report a class it is not.
        assert!(!Error::command("x").is_connection());
    }

    #[test]
    fn display_renders_message_alone_without_a_cause() {
        let err = Error::command("file not found");
        assert_eq!(err.to_string(), "file not found");
    }

    #[test]
    fn display_appends_the_cause_chain() {
        let inner =
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        let err = Error::connection("cannot reach server").with_source(inner);

        assert_eq!(err.to_string(), "cannot reach server: connection refused");
        assert!(err.source().is_some());
    }

    #[test]
    fn every_class_renders_the_error_severity_prefix() {
        for class in [
            ErrorClass::Command,
            ErrorClass::Connection,
            ErrorClass::Authentication,
        ] {
            assert_eq!(class.severity_prefix(), "error");
        }
    }
}
