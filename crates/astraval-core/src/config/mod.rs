// SPDX-License-Identifier: Apache-2.0

//! Perforce-compatible configuration resolution for the Astraval client.
//!
//! Astraval resolves the same connection and identity settings as `p4`, from the
//! same sources, in the same precedence order, so the `P4PORT`, `P4USER`, and
//! related values studios already export keep working unchanged. This module
//! turns those scattered sources into one [`ResolvedConfig`] the rest of the CLI
//! reads, recording per value where it came from for an eventual `astraval set`.
//!
//! # What gets resolved
//!
//! The settings are named by [`Setting`]: `P4PORT`, `P4USER`, `P4CLIENT`,
//! `P4PASSWD`, `P4CHARSET`, and `P4HOST`. The `-d` current-directory override is
//! an *input* to resolution (it moves where the `P4CONFIG` search starts), not a
//! resolved setting.
//!
//! # Precedence
//!
//! Highest source wins: command-line flags, then the nearest `P4CONFIG` file
//! (found by walking up from the current directory), then environment variables,
//! then compiled-in defaults. The full rationale, including how this was
//! confirmed against the real `p4 set` client as a behavioral oracle, lives in
//! the [`resolved`] module docs.
//!
//! # Usage
//!
//! Call [`resolve`] with the parsed global flags, an environment map, and the
//! starting directory. It returns a [`ResolvedConfig`] whose accessors yield each
//! value and its [`Source`].
//!
//! ```
//! use std::collections::HashMap;
//! use std::path::Path;
//! use astraval_core::config::{GlobalSettings, resolve};
//!
//! let flags = GlobalSettings {
//!     user: Some("alice".to_owned()),
//!     ..GlobalSettings::default()
//! };
//! let env = HashMap::from([("P4PORT".to_owned(), "tcp:host:1666".to_owned())]);
//!
//! let config = resolve(&flags, &env, Path::new("."))?;
//! assert_eq!(config.user(), Some("alice"));
//! assert_eq!(config.port(), "tcp:host:1666");
//! # Ok::<(), astraval_core::Error>(())
//! ```

mod config_file;
mod resolved;
mod setting;
mod source;

#[doc(inline)]
pub use config_file::{ConfigFile, discover};
#[doc(inline)]
pub use resolved::{ConfigValue, GlobalSettings, ResolvedConfig, resolve};
#[doc(inline)]
pub use setting::Setting;
#[doc(inline)]
pub use source::Source;
