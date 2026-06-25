// SPDX-License-Identifier: Apache-2.0

//! Subcommand implementations and the dispatch glue that selects one.
//!
//! Each command lives in its own submodule and exposes a `run` function with a
//! uniform signature: it takes the resolved [`GlobalOptions`] and returns an
//! [`ExitCode`]. The [`dispatch`] function maps a parsed [`Commands`] variant to
//! the matching `run`, so adding a command is a two-line change here plus a new
//! module. Keeping the shape uniform is what lets later command issues plug in
//! without reshaping the frame.

use std::process::ExitCode;

use crate::cli::{Commands, GlobalOptions};

pub mod info;

/// Routes a parsed subcommand to its implementation and returns its exit code.
///
/// Later command issues add their variant to [`Commands`] and a matching arm
/// here; the `global` options are forwarded by reference so every command sees
/// the same resolved configuration without consuming it.
pub fn dispatch(command: &Commands, global: &GlobalOptions) -> ExitCode {
    match command {
        Commands::Info => info::run(global),
    }
}
