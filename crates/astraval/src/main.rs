// SPDX-License-Identifier: Apache-2.0

//! The `astraval` command-line client.
//!
//! This binary is the studio-facing entry point to Astraval, an open-source
//! alternative to Perforce. Its command-line surface mirrors `p4`: the same
//! single-dash global flags (`-p`, `-u`, `-c`, and friends) and a set of
//! subcommands that grow over the milestones.
//!
//! `main` is deliberately thin. It parses arguments into [`cli::Cli`], resolves
//! the Perforce-compatible configuration from the flags and the process
//! environment, then hands the chosen subcommand and that resolved configuration
//! to [`commands::dispatch`], which returns the process exit code. Parsing lives
//! in [`cli`], configuration resolution in [`config`], per-command behavior in
//! [`commands`]; this file only connects them.

mod cli;
mod commands;
mod config;
mod exit;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;

fn main() -> ExitCode {
    // `parse` handles `-h`/`--help`, `-V`/`--version`, usage errors, and the
    // no-subcommand case (which prints help) by exiting before returning. Past
    // this point we always have a valid command to dispatch.
    let cli = Cli::parse();

    // Resolve configuration once, up front, so every command receives an
    // already-resolved view. A resolution failure (for example an unreadable
    // P4CONFIG file) flows through the same single error path as any command
    // failure, keeping the exit-code contract uniform.
    let config = match config::resolve_from_process(&cli.global) {
        Ok(config) => config,
        Err(error) => return exit::report(&error),
    };

    // The output mode is a presentation concern derived purely from the global
    // flags (`-G`, `-ztag`), separate from the connection/identity configuration
    // resolved above. Compute it here and hand it to every command so the
    // rendering choice lives in one place.
    let mode = cli.global.output_mode();

    commands::dispatch(&cli.command, &config, mode)
}
