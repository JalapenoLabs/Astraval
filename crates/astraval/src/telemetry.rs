// SPDX-License-Identifier: Apache-2.0

//! Tracing setup: turning verbosity into a level and installing the subscriber.
//!
//! Astraval observes its own RPC traffic and internal state through the
//! `tracing` ecosystem. Library crates (`astraval-proto`, `astraval-core`) emit
//! spans and events through the lightweight `tracing` facade; this module is the
//! binary-side counterpart that decides *where* those events go and *how loud*
//! the stream is. It owns the single process-wide subscriber, installed once,
//! early in `main`.
//!
//! # Verbosity model
//!
//! Perforce exposes debug output through `P4DEBUG` (a per-subsystem level map)
//! and a `-v` flag. Astraval keeps that mental model but renders it idiomatically
//! with two layered controls:
//!
//! - A repeatable `-v` command-line flag ([`crate::cli::GlobalOptions::verbose`])
//!   that raises a coarse, global level. Each `-v` is one step louder.
//! - An [`EnvFilter`] honoring the standard `RUST_LOG` and a documented,
//!   `P4DEBUG`-style [`ASTRAVAL_LOG_ENV`] variable for fine-grained, per-target
//!   control (e.g. `ASTRAVAL_LOG=astraval_proto=trace`).
//!
//! The environment variable wins when present, because a contributor reaching for
//! a directive string wants exactly what they typed; the `-v` flag supplies the
//! default filter only when no variable is set. Either way, output goes to
//! **stderr**, never stdout, so the machine-readable stdout of `-G` and `-ztag`
//! stays uncontaminated.
//!
//! # Level mapping
//!
//! | `-v` count | Level   | What it shows                                  |
//! |------------|---------|------------------------------------------------|
//! | 0          | `WARN`  | warnings and errors only (the quiet default)   |
//! | 1          | `INFO`  | high-level progress                            |
//! | 2          | `DEBUG` | internal state transitions                     |
//! | 3 or more  | `TRACE` | full detail, including per-RPC tracing later   |
//!
//! The protocol layer (next issues) emits its detailed RPC spans through this
//! same subscriber; nothing here has to change for that to light up.

use tracing::Level;
use tracing_subscriber::EnvFilter;

/// Environment variable for a `P4DEBUG`-style tracing filter directive.
///
/// Set this to any [`EnvFilter`] directive string (for example
/// `astraval_proto=trace,astraval_core=debug`) to control tracing per target.
/// It mirrors the role of Perforce's `P4DEBUG` while using `tracing`'s standard
/// directive syntax. When set, it takes precedence over the `-v` flag.
pub const ASTRAVAL_LOG_ENV: &str = "ASTRAVAL_LOG";

/// Translates a repeated-`-v` count into the matching tracing [`Level`].
///
/// Zero (no `-v`) is the quiet default of `WARN`; each additional `-v` steps the
/// floor down one level, saturating at `TRACE`. The mapping is documented in the
/// module table and pinned by unit tests.
#[must_use]
pub fn level_for_verbosity(verbose: u8) -> Level {
    match verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        // Three or more `-v` flags: there is no level quieter to escalate to, so
        // we saturate at the most verbose level rather than wrapping or erroring.
        _ => Level::TRACE,
    }
}

/// Builds the active [`EnvFilter`] from the environment and the `-v` count.
///
/// Resolution order, most specific first:
///
/// 1. [`ASTRAVAL_LOG_ENV`], the documented `P4DEBUG`-style variable.
/// 2. `RUST_LOG`, the ecosystem-standard variable.
/// 3. The `-v` count, mapped through [`level_for_verbosity`].
///
/// A variable wins only when it is present *and* parses; a malformed directive
/// falls through to the next source rather than aborting startup, since a typo in
/// a debug variable should not stop the command the user actually asked for.
fn filter_for(verbose: u8) -> EnvFilter {
    // Try the documented variable first, then the ecosystem-standard one. Either
    // may carry a full per-target directive; we honor it verbatim.
    for variable in [ASTRAVAL_LOG_ENV, EnvFilter::DEFAULT_ENV] {
        if let Ok(directive) = std::env::var(variable) {
            if let Ok(filter) = EnvFilter::try_new(&directive) {
                return filter;
            }
        }
    }

    // No usable variable: derive a single global level from the `-v` count.
    EnvFilter::new(level_for_verbosity(verbose).to_string())
}

/// Installs the process-wide tracing subscriber from the resolved verbosity.
///
/// Called once, early in `main`, before any command runs. Events are written to
/// **stderr** so they never mingle with a command's machine-readable stdout
/// (`-G`, `-ztag`). The effective filter comes from [`filter_for`]: a
/// `RUST_LOG`/`ASTRAVAL_LOG` directive if set, otherwise the level implied by the
/// `-v` count.
///
/// # Panics
///
/// Panics if a global subscriber is already installed. That can only happen if
/// this is called more than once, which is a programming error: per
/// [M-PANIC-ON-BUG], a broken call sequence is a panic, not a recoverable error.
///
/// [M-PANIC-ON-BUG]: a detected programming bug terminates rather than returning
/// an unactionable error.
pub fn init(verbose: u8) {
    tracing_subscriber::fmt()
        .with_env_filter(filter_for(verbose))
        // Trace output is a diagnostic stream, not program output. Sending it to
        // stderr keeps stdout clean for `-G`/`-ztag` consumers that parse it.
        .with_writer(std::io::stderr)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No `-v` is the quiet default: only warnings and errors surface.
    #[test]
    fn zero_verbosity_is_warn() {
        assert_eq!(level_for_verbosity(0), Level::WARN);
    }

    /// Each `-v` steps the level down one notch through info and debug.
    #[test]
    fn verbosity_steps_through_info_and_debug() {
        assert_eq!(level_for_verbosity(1), Level::INFO);
        assert_eq!(level_for_verbosity(2), Level::DEBUG);
    }

    /// The third `-v` reaches `TRACE`, and further flags saturate there.
    #[test]
    fn verbosity_saturates_at_trace() {
        assert_eq!(level_for_verbosity(3), Level::TRACE);
        assert_eq!(level_for_verbosity(10), Level::TRACE);
        assert_eq!(level_for_verbosity(u8::MAX), Level::TRACE);
    }

    /// The mapping is monotonic: more `-v` is never quieter than less.
    ///
    /// `tracing::Level` orders so that *more verbose is "greater than"* (`TRACE`
    /// is the maximum, `ERROR` the minimum). Guarding the property directly keeps
    /// the table honest if the arms are ever reshuffled.
    #[test]
    fn verbosity_is_monotonically_louder() {
        let levels: Vec<Level> = (0..=4).map(level_for_verbosity).collect();
        for pair in levels.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "raising verbosity must not lower the level: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }
}
