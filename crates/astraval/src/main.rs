// SPDX-License-Identifier: Apache-2.0

//! The `astraval` command-line client.
//!
//! This binary is the studio-facing entry point to Astraval, an open-source
//! alternative to Perforce. Its command-line surface mirrors `p4`: the same
//! single-dash global flags (`-p`, `-u`, `-c`, and friends) and a set of
//! subcommands that grow over the milestones.
//!
//! `main` is deliberately thin. It parses arguments into [`cli::Cli`], then
//! hands the chosen subcommand and the shared global options to
//! [`commands::dispatch`], which returns the process exit code. Parsing lives in
//! [`cli`], per-command behavior in [`commands`]; this file only connects them.

mod cli;
mod commands;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;

fn main() -> ExitCode {
    // `parse` handles `-h`/`--help`, `-V`/`--version`, usage errors, and the
    // no-subcommand case (which prints help) by exiting before returning. Past
    // this point we always have a valid command to dispatch.
    let cli = Cli::parse();
    commands::dispatch(&cli.command, &cli.global)
}
