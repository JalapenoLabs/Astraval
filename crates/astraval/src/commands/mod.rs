// SPDX-License-Identifier: Apache-2.0

//! Subcommand implementations and the dispatch glue that selects one.
//!
//! Each command lives in its own submodule and exposes a `run` function with a
//! uniform signature: it takes the resolved [`ResolvedConfig`] and returns an
//! [`astraval_core::Result`]. The [`dispatch`] function maps a parsed
//! [`Commands`] variant to the matching `run`, then funnels the outcome through
//! [`crate::exit::report`] so success and every failure class resolve to an
//! exit code in exactly one place. Adding a command is a new module plus a
//! single `match` arm; it never has to think about exit codes or stderr.

use std::process::ExitCode;

use astraval_core::Result;
use astraval_core::config::ResolvedConfig;
use astraval_core::output::OutputMode;

use crate::cli::Commands;
use crate::exit;

pub mod info;

/// Routes a parsed subcommand to its implementation and returns its exit code.
///
/// Each command's [`Result`] flows through [`exit::report`], the single error
/// path: `Ok` becomes [`ExitCode::SUCCESS`], and any error is rendered to
/// stderr and mapped to its Perforce-compatible exit code there. Later command
/// issues add their variant to [`Commands`] and a matching arm here; the resolved
/// `config` is forwarded by reference so every command sees the same already
/// resolved configuration without consuming it.
///
/// The selected [`OutputMode`] is forwarded too, so every command renders its
/// records through the shared output layer (human, `-ztag`, or `-G`) instead of
/// each one re-deriving the mode from the flags.
pub fn dispatch(command: &Commands, config: &ResolvedConfig, mode: OutputMode) -> ExitCode {
    let outcome: Result<()> = match command {
        Commands::Info => info::run(config, mode),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => exit::report(&error),
    }
}
