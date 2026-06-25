// SPDX-License-Identifier: Apache-2.0

//! Resolving configuration from the real process environment.
//!
//! The resolution *logic* lives in [`astraval_core::config`], which is written
//! sans-io: it takes an explicit environment map and starting directory so it can
//! be unit-tested without touching the host. This module is the thin binary-side
//! seam that supplies those real inputs, reading the process environment and
//! current directory, then hands them to the core resolver. Keeping the syscalls
//! here means the core stays deterministic and the binary owns the one place that
//! observes the machine.

use std::collections::HashMap;

use astraval_core::config::{ResolvedConfig, resolve};

use crate::cli::GlobalOptions;

/// Resolves the full configuration from the parsed flags and the live process.
///
/// Gathers the process environment and current directory, translates the global
/// flags into the core resolver's input, and returns the [`ResolvedConfig`] every
/// command reads. This is the only place in the binary that observes the real
/// environment, so the rest of the CLI works against an already-resolved value.
///
/// # Errors
/// Propagates a core error if a located `P4CONFIG` file cannot be read. The
/// current directory failing to resolve is treated the same way: it surfaces as a
/// command-class error rather than a panic, since a process can legitimately have
/// no accessible working directory.
pub fn resolve_from_process(global: &GlobalOptions) -> astraval_core::Result<ResolvedConfig> {
    // Snapshot the environment once. The resolver only reads it, so a plain map
    // is the simplest faithful representation of the process's variables.
    let environment: HashMap<String, String> = std::env::vars().collect();

    // The current directory is where the `P4CONFIG` search begins unless `-d`
    // overrides it. A missing or inaccessible cwd is a real, reportable failure.
    let current_dir = std::env::current_dir().map_err(|error| {
        astraval_core::Error::command("could not determine the current directory")
            .with_source(error)
    })?;

    resolve(&global.to_settings(), &environment, &current_dir)
}
