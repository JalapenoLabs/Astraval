// SPDX-License-Identifier: Apache-2.0

//! The `info` command: report connection and client details.
//!
//! In Perforce, `p4 info` prints server address, user, client, and similar
//! facts about the current connection. Astraval mirrors that command, but the
//! real implementation (which needs a live server connection) lands in a later
//! milestone. For now this is a deliberate stub: rather than print and pick its
//! own exit code, it returns a command-class [`Error`] so its failure flows
//! through the same single error path (`crate::exit::report`) every command
//! uses. The real implementation arrives in issue #14.

use astraval_core::config::ResolvedConfig;
use astraval_core::{Error, Result};

/// Runs the `info` stub, reporting that it is not implemented yet.
///
/// Returns a command-class [`Error`] so the dispatcher renders it through the
/// shared error path; there is no bespoke exit code here. The resolved `_config`
/// is accepted now so the signature matches every other command and so the real
/// implementation (issue #14) can read the connection details straight from it.
/// It is borrowed, not consumed.
pub fn run(_config: &ResolvedConfig) -> Result<()> {
    Err(Error::command(
        "info: not implemented yet. Connecting to a server and reporting its \
         details arrives in a later milestone.",
    ))
}
