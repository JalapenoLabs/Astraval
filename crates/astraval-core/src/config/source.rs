// SPDX-License-Identifier: Apache-2.0

//! Where a resolved configuration value came from.
//!
//! Every setting Astraval resolves can be traced back to exactly one origin in
//! Perforce's precedence chain. Recording that origin alongside the value is not
//! decoration: a future `astraval set` command (issue #15) must tell users *why*
//! a setting has the value it does ("it comes from this `.p4config` file" versus
//! "it is the compiled-in default"), exactly as `p4 set` does when it appends
//! `(config '...')` to a line. Keeping the provenance on the value means that
//! explanation is always available without re-running resolution.

use std::path::{Path, PathBuf};

/// The origin of a resolved configuration value, in Perforce precedence order.
///
/// The variants are ordered highest-precedence first, matching the order the
/// resolver consults them. A value's `Source` answers "where did this come
/// from", which the CLI surfaces to users and which tests assert against to pin
/// the precedence behavior.
///
/// This enum is `#[non_exhaustive]`: the Windows registry source that `p4 set`
/// can report is modeled by [`Source::Registry`] today but not yet *read* by the
/// resolver (that is a platform-specific seam left for later), and other origins
/// such as a `P4ENVIRO` file may join the chain, so callers must keep a
/// catch-all arm.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Source {
    /// A command-line global flag, e.g. `-p tcp:host:1666`. Highest precedence.
    CommandLine,

    /// A `P4CONFIG` file discovered by walking up from the current directory.
    ///
    /// Carries the path of the file the value was read from, so the CLI can show
    /// it the way `p4 set` shows `(config '<path>')`.
    Config(PathBuf),

    /// A process environment variable, e.g. `P4PORT`.
    Environment,

    /// A value set by `p4 set` and stored in the Windows registry.
    ///
    /// Modeled for completeness and for `astraval set` parity, but the resolver
    /// does not read the registry yet; no value is produced with this source
    /// today. Reading it is a Windows-only seam left for a later issue.
    Registry,

    /// A compiled-in default, such as `P4PORT` defaulting to `perforce:1666`.
    /// Lowest precedence: used only when no higher source supplied a value.
    Default,
}

impl Source {
    /// Returns the `P4CONFIG` file path if this value came from one.
    ///
    /// Returns `None` for every other source. Useful for the CLI to render the
    /// `(config '<path>')` annotation only when it applies.
    ///
    /// # Examples
    /// ```
    /// use std::path::Path;
    /// use astraval_core::config::Source;
    ///
    /// let from_env = Source::Environment;
    /// assert!(from_env.config_path().is_none());
    ///
    /// let from_file = Source::Config(Path::new(".p4config").to_path_buf());
    /// assert_eq!(from_file.config_path(), Some(Path::new(".p4config")));
    /// ```
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> {
        match self {
            Self::Config(path) => Some(path),
            _ => None,
        }
    }
}
