// SPDX-License-Identifier: Apache-2.0

//! Command-line surface for the `astraval` binary.
//!
//! This module defines the parser only: the global options every command
//! shares, and the [`Commands`] enum of subcommands. It deliberately holds no
//! behavior. Parsing here, dispatch in [`crate::commands`], and per-command
//! logic in that module's submodules keeps each concern in one place.
//!
//! # Perforce-compatible global flags
//!
//! Astraval speaks the Perforce wire protocol, so its CLI mirrors `p4`'s global
//! options too. Studios already have scripts and muscle memory built around
//! single-dash flags like `-p tcp:host:1666` and `-ztag`; matching them exactly
//! means those transfer unchanged. Each flag below maps to one `p4` global
//! option and, where Perforce reads an environment variable (P4PORT, P4USER,
//! and friends), to that variable's role.
//!
//! Two flags, `-V` (version) and `-h` (help), are provided by clap's built-ins
//! rather than declared here, so their behavior matches what users expect from
//! any clap program while still using the short letters `p4` uses.

use std::path::PathBuf;

use astraval_core::config::GlobalSettings;
use clap::{Parser, Subcommand};

/// The `astraval` command-line client.
///
/// Parsed from the process arguments via [`clap::Parser`]. The global options
/// are flattened in so they may appear before or after the subcommand, matching
/// how `p4` accepts its global flags in either position.
#[derive(Debug, Parser)]
#[command(
    name = "astraval",
    bin_name = "astraval",
    version,
    about = "Astraval, the open-source alternative to Perforce.",
    // Pin the long help text explicitly. Without this, clap would lift the
    // struct's rustdoc (intended for developers, and full of doc-link syntax)
    // into `--help`, which is meant for end users.
    long_about = "Astraval, the open-source alternative to Perforce.\n\nStudio-grade version control with exclusive file locking and terabyte-scale binary assets. Its command-line surface mirrors `p4`, so existing scripts and muscle memory transfer.",
    // Print help when invoked with no subcommand instead of a terse error, so a
    // bare `astraval` greets the user with the command list.
    arg_required_else_help = true
)]
pub struct Cli {
    /// Global options shared by every subcommand.
    #[command(flatten)]
    pub global: GlobalOptions,

    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Global options that mirror Perforce's `p4` global flags.
///
/// Every field is optional: an unset flag means "fall back to the environment
/// or a built-in default," which the configuration subsystem (issue #4) will
/// resolve later. For now these values are parsed and carried, nothing more.
///
/// Marking the group `global = true` lets each flag appear either before or
/// after the subcommand, so both `astraval -u alice info` and
/// `astraval info -u alice` work, just as they do with `p4`.
#[derive(Debug, Default, Parser)]
pub struct GlobalOptions {
    /// Server address, e.g. `tcp:localhost:1666` (Perforce P4PORT).
    #[arg(short = 'p', value_name = "port", global = true)]
    pub port: Option<String>,

    /// User name (Perforce P4USER).
    #[arg(short = 'u', value_name = "user", global = true)]
    pub user: Option<String>,

    /// Client/workspace name (Perforce P4CLIENT).
    #[arg(short = 'c', value_name = "client", global = true)]
    pub client: Option<String>,

    /// Override the current working directory (Perforce `-d`).
    #[arg(short = 'd', value_name = "dir", global = true)]
    pub dir: Option<String>,

    /// Override the host name (Perforce P4HOST).
    #[arg(short = 'H', value_name = "host", global = true)]
    pub host: Option<String>,

    /// Password or ticket (Perforce P4PASSWD).
    #[arg(short = 'P', value_name = "pass", global = true)]
    pub password: Option<String>,

    /// Client character set (Perforce P4CHARSET).
    #[arg(short = 'C', value_name = "charset", global = true)]
    pub charset: Option<String>,

    /// Tagged output mode, e.g. `-ztag` (Perforce `-z`).
    #[arg(short = 'z', value_name = "tag", global = true)]
    pub tag: Option<String>,

    /// Marshalled output for scripting (Perforce `-G`).
    #[arg(short = 'G', global = true)]
    pub marshalled: bool,
}

impl GlobalOptions {
    /// Translates the parsed flags into the core resolver's input struct.
    ///
    /// The configuration subsystem lives in `astraval-core` and is intentionally
    /// decoupled from clap, so the binary maps its flags onto
    /// [`GlobalSettings`] here. Only the flags that participate in configuration
    /// resolution carry over: `-z` (tagged output) and `-G` (marshalled output)
    /// are output *modes*, not connection or identity settings, so they are not
    /// part of resolution. The `-d` override is turned into a [`PathBuf`] because
    /// it names a directory the `P4CONFIG` search starts from.
    #[must_use]
    pub fn to_settings(&self) -> GlobalSettings {
        GlobalSettings {
            port: self.port.clone(),
            user: self.user.clone(),
            client: self.client.clone(),
            password: self.password.clone(),
            charset: self.charset.clone(),
            host: self.host.clone(),
            dir: self.dir.as_ref().map(PathBuf::from),
        }
    }
}

/// The set of subcommands `astraval` understands.
///
/// Each variant corresponds to a module under [`crate::commands`]. Later command
/// issues add their own variant here and a matching module; the dispatcher in
/// [`crate::commands::dispatch`] keeps the wiring to a single `match` arm.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Report connection and client details (Perforce `p4 info`).
    Info,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// clap's own consistency checks must pass, or the parser is malformed.
    ///
    /// This catches structural mistakes (duplicate short flags, bad value
    /// configuration) at test time instead of at first run.
    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    /// Global flags parse into the shared options and select the subcommand.
    ///
    /// Exercises the Perforce-style flags before the subcommand, which is the
    /// position scripts most often use.
    #[test]
    fn parses_global_flags_with_info_subcommand() {
        let cli = Cli::try_parse_from([
            "astraval",
            "-p",
            "tcp:localhost:1666",
            "-u",
            "alice",
            "-c",
            "workspace-main",
            "-ztag",
            "-G",
            "info",
        ])
        .expect("valid global flags and subcommand should parse");

        assert_eq!(cli.global.port.as_deref(), Some("tcp:localhost:1666"));
        assert_eq!(cli.global.user.as_deref(), Some("alice"));
        assert_eq!(cli.global.client.as_deref(), Some("workspace-main"));
        assert_eq!(cli.global.tag.as_deref(), Some("tag"));
        assert!(cli.global.marshalled);
        assert!(matches!(cli.command, Commands::Info));
    }

    /// Global flags may also follow the subcommand, just as they do in `p4`.
    #[test]
    fn parses_global_flags_after_subcommand() {
        let cli = Cli::try_parse_from(["astraval", "info", "-u", "bob"])
            .expect("global flags after the subcommand should parse");

        assert_eq!(cli.global.user.as_deref(), Some("bob"));
        assert!(matches!(cli.command, Commands::Info));
    }

    /// An unknown subcommand is a usage error, not a silent no-op.
    #[test]
    fn rejects_unknown_subcommand() {
        let error = Cli::try_parse_from(["astraval", "nonsense"])
            .expect_err("an unknown subcommand must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}
