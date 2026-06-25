// SPDX-License-Identifier: Apache-2.0

//! The `astraval` command-line client.
//!
//! This binary is the studio-facing entry point to Astraval. Real commands
//! (`info`, `sync`, `lock`, `submit`, and the rest) arrive in later milestones;
//! for now it prints a short stub so the workspace builds and runs end to end.

use astraval_core::target_protocol_level;

fn main() {
    // Reference the core crate so the dependency wiring is exercised by a real
    // call, not just a declaration. Replaced by the `info` command later.
    println!(
        "astraval {} (scaffold). Targeting Perforce protocol level {}.",
        env!("CARGO_PKG_VERSION"),
        target_protocol_level(),
    );
}
