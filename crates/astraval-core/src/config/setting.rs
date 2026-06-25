// SPDX-License-Identifier: Apache-2.0

//! The connection and identity settings Astraval resolves, and their names.
//!
//! Perforce exposes its configuration as a fixed set of `P4*` variables shared
//! across the environment, `P4CONFIG` files, and the registry. Astraval resolves
//! the same names so the values studios already export keep working unchanged.
//! This module names that set once, as the [`Setting`] enum, so the resolver,
//! the config-file parser, and the CLI all agree on which variables exist and
//! what each is called on the wire.

/// A single connection or identity setting Astraval resolves.
///
/// Each variant maps to one Perforce `P4*` variable. The set is the minimum the
/// CLI needs to open a connection and identify the user; later milestones may
/// extend it, so the enum is `#[non_exhaustive]` and callers should keep a
/// catch-all arm when matching.
///
/// The current-directory override (`p4`'s `-d`) is intentionally *not* a
/// `Setting`: it is an input to resolution (it changes where `P4CONFIG` files
/// are discovered) rather than a resolved output, so it lives on the resolver's
/// inputs instead of here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Setting {
    /// The server address (`P4PORT`), e.g. `tcp:localhost:1666`.
    Port,
    /// The user name (`P4USER`).
    User,
    /// The client/workspace name (`P4CLIENT`).
    Client,
    /// The password or login ticket (`P4PASSWD`).
    Password,
    /// The client character set (`P4CHARSET`).
    Charset,
    /// The host name override (`P4HOST`).
    Host,
}

impl Setting {
    /// Every setting the resolver knows about, in a stable order.
    ///
    /// Used to iterate the full set when resolving or rendering all values. The
    /// order is the natural connection-then-identity grouping and is relied upon
    /// only for presentation, not correctness.
    pub const ALL: [Self; 6] = [
        Self::Port,
        Self::User,
        Self::Client,
        Self::Password,
        Self::Charset,
        Self::Host,
    ];

    /// Returns the Perforce variable name for this setting, e.g. `"P4PORT"`.
    ///
    /// This is the exact spelling used as an environment variable, as the key in
    /// a `P4CONFIG` file, and in `p4 set` output, so the resolver matches on it
    /// directly.
    ///
    /// # Examples
    /// ```
    /// use astraval_core::config::Setting;
    /// assert_eq!(Setting::Port.variable_name(), "P4PORT");
    /// ```
    #[must_use]
    pub const fn variable_name(self) -> &'static str {
        match self {
            Self::Port => "P4PORT",
            Self::User => "P4USER",
            Self::Client => "P4CLIENT",
            Self::Password => "P4PASSWD",
            Self::Charset => "P4CHARSET",
            Self::Host => "P4HOST",
        }
    }

    /// Returns the compiled-in default for this setting, if one exists.
    ///
    /// Only `P4PORT` has a Perforce-defined default, `perforce:1666`: the
    /// historical well-known host and port a bare `p4` falls back to. Every other
    /// setting has no sensible universal default (a user or client name cannot be
    /// guessed), so resolution simply leaves them unset.
    ///
    /// # Examples
    /// ```
    /// use astraval_core::config::Setting;
    /// assert_eq!(Setting::Port.default_value(), Some("perforce:1666"));
    /// assert_eq!(Setting::User.default_value(), None);
    /// ```
    #[must_use]
    pub const fn default_value(self) -> Option<&'static str> {
        match self {
            // The well-known Perforce fallback: host `perforce`, port `1666`.
            // This is the value a `p4` client with nothing configured uses, so
            // matching it keeps Astraval's out-of-the-box behavior identical.
            Self::Port => Some("perforce:1666"),
            _ => None,
        }
    }
}
