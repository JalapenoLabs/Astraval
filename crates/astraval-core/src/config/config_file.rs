// SPDX-License-Identifier: Apache-2.0

//! Discovery and parsing of `P4CONFIG` files.
//!
//! When `P4CONFIG` names a filename (for example `.p4config`), Perforce walks
//! *up* the directory tree from the current directory looking for a file of that
//! name, and the nearest one wins. This lets a studio drop a `.p4config` at the
//! root of a workspace and have every command run from anywhere inside it pick up
//! the same server and client settings. This module finds that file and parses
//! its `VAR=value` lines into a simple map.
//!
//! The parser is deliberately forgiving in the same ways `p4` is: blank lines and
//! `#` comments are skipped, surrounding whitespace is trimmed, and a line
//! without `=` is ignored rather than treated as an error. Reading is fallible
//! only for genuine I/O problems on a file that was found; a missing file is not
//! an error, it simply means "no config here, keep walking".

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::Error;

/// The parsed contents of a single `P4CONFIG` file: its path and its variables.
///
/// Produced by [`discover`]. The `path` is retained so each resolved value can
/// record exactly which file it came from, the way `p4 set` prints
/// `(config '<path>')`.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    /// The absolute or as-discovered path of the file these variables came from.
    path: PathBuf,

    /// The `VAR=value` pairs parsed from the file, keyed by variable name.
    variables: HashMap<String, String>,
}

impl ConfigFile {
    /// Returns the path of the file these variables were read from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the value for `variable`, if the file defined it.
    ///
    /// The lookup is exact and case-sensitive, matching the `P4*` spelling used
    /// throughout Perforce configuration.
    #[must_use]
    pub fn get(&self, variable: &str) -> Option<&str> {
        self.variables.get(variable).map(String::as_str)
    }
}

/// Walks up from `start_dir` to find the nearest `filename`, returning it parsed.
///
/// This is the `P4CONFIG` discovery rule: starting at `start_dir` and ascending
/// through each parent, the first directory that contains a file named
/// `filename` wins, and its contents are parsed and returned. Returns `Ok(None)`
/// when no such file exists anywhere up the tree, which is the common, expected
/// case and not an error.
///
/// # Errors
/// Returns a command-class [`Error`] if a matching file is found but cannot be
/// read (for example a permissions failure). A file that simply does not exist is
/// never an error; the walk continues past it.
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// use astraval_core::config::discover;
///
/// // From deep inside a workspace, find the `.p4config` at its root.
/// let found = discover(Path::new("/work/game/content"), ".p4config")?;
/// if let Some(file) = found {
///     println!("using config at {}", file.path().display());
/// }
/// # Ok::<(), astraval_core::Error>(())
/// ```
pub fn discover(start_dir: &Path, filename: &str) -> Result<Option<ConfigFile>, Error> {
    // `Path::ancestors` yields `start_dir`, then each parent up to the root,
    // which is exactly the nearest-first order Perforce searches in.
    for directory in start_dir.ancestors() {
        let candidate = directory.join(filename);

        match std::fs::read_to_string(&candidate) {
            Ok(contents) => {
                return Ok(Some(ConfigFile {
                    variables: parse(&contents),
                    path: candidate,
                }));
            }
            // Not found here is the normal case: keep ascending.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // Any other I/O failure on a file we did locate is a real error the
            // user should hear about, rather than being silently skipped.
            Err(error) => {
                return Err(Error::command(format!(
                    "failed to read P4CONFIG file '{}'",
                    candidate.display()
                ))
                .with_source(error));
            }
        }
    }

    Ok(None)
}

/// Parses `VAR=value` lines from `P4CONFIG` file `contents` into a map.
///
/// Mirrors how `p4` reads these files: each non-empty, non-comment line is split
/// on its first `=`; the name and value are trimmed of surrounding whitespace; a
/// later definition of the same variable overrides an earlier one. Lines that are
/// blank, start with `#`, or contain no `=` are skipped. Splitting on the *first*
/// `=` keeps values that themselves contain `=` (rare, but possible in a
/// password or ticket) intact.
fn parse(contents: &str) -> HashMap<String, String> {
    let mut variables = HashMap::new();

    for line in contents.lines() {
        let line = line.trim();

        // Skip blank lines and `#` comments, as `p4` does.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split on the first `=` only, so values containing `=` survive.
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };

        let name = name.trim();
        if name.is_empty() {
            continue;
        }

        variables.insert(name.to_owned(), value.trim().to_owned());
    }

    variables
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_simple_var_value_pairs() {
        let parsed = parse("P4PORT=tcp:host:1666\nP4USER=alice\n");
        assert_eq!(
            parsed.get("P4PORT").map(String::as_str),
            Some("tcp:host:1666")
        );
        assert_eq!(parsed.get("P4USER").map(String::as_str), Some("alice"));
    }

    #[test]
    fn parse_trims_whitespace_around_name_and_value() {
        let parsed = parse("  P4USER  =   bob  \n");
        assert_eq!(parsed.get("P4USER").map(String::as_str), Some("bob"));
    }

    #[test]
    fn parse_skips_blank_lines_and_comments() {
        let parsed = parse("\n# a comment\nP4CLIENT=ws\n   \n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("P4CLIENT").map(String::as_str), Some("ws"));
    }

    #[test]
    fn parse_ignores_lines_without_an_equals_sign() {
        let parsed = parse("this is not a setting\nP4PORT=1666\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("P4PORT").map(String::as_str), Some("1666"));
    }

    #[test]
    fn parse_keeps_equals_signs_inside_the_value() {
        let parsed = parse("P4PASSWD=a=b=c\n");
        assert_eq!(parsed.get("P4PASSWD").map(String::as_str), Some("a=b=c"));
    }

    #[test]
    fn parse_lets_a_later_definition_win() {
        let parsed = parse("P4USER=first\nP4USER=second\n");
        assert_eq!(parsed.get("P4USER").map(String::as_str), Some("second"));
    }

    /// A unique temporary directory that deletes itself when dropped.
    ///
    /// Discovery walks the real filesystem, so these tests need real directories.
    /// We avoid a `tempfile` dev-dependency (keeping the crate dependency-free per
    /// rust.md M-OOBE) with this tiny RAII helper: it creates a uniquely named
    /// directory under the system temp dir and removes it on `Drop`, so tests
    /// never depend on or leak machine state.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};

            // A process-unique counter plus the PID keeps concurrent tests from
            // colliding on the same directory name.
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "astraval-config-test-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("temp dir should be creatable");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // Best-effort cleanup: a failure here must not mask a test result.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn discover_returns_none_when_no_file_exists_up_the_tree() {
        let temp = TempDir::new();
        let found = discover(temp.path(), ".p4config").expect("absence is not an error");
        assert!(found.is_none());
    }

    #[test]
    fn discover_finds_a_file_in_a_parent_directory() {
        let temp = TempDir::new();
        std::fs::write(temp.path().join(".p4config"), "P4PORT=tcp:root:1666\n")
            .expect("writing the config file should succeed");

        let deep = temp.path().join("a").join("b");
        std::fs::create_dir_all(&deep).expect("nested dirs should be creatable");

        let found = discover(&deep, ".p4config")
            .expect("discovery should succeed")
            .expect("the parent's file should be found by walking up");
        assert_eq!(found.get("P4PORT"), Some("tcp:root:1666"));
    }

    #[test]
    fn discover_prefers_the_nearest_file_when_several_exist() {
        let temp = TempDir::new();
        // A file at the root and a nearer one in a subdirectory: the nearer wins,
        // matching how `p4` lets a deeper `.p4config` override a shallower one.
        std::fs::write(temp.path().join(".p4config"), "P4PORT=tcp:root:1666\n")
            .expect("writing the root config should succeed");

        let sub = temp.path().join("sub");
        std::fs::create_dir_all(&sub).expect("subdir should be creatable");
        std::fs::write(sub.join(".p4config"), "P4PORT=tcp:near:1666\n")
            .expect("writing the nearer config should succeed");

        let found = discover(&sub, ".p4config")
            .expect("discovery should succeed")
            .expect("a file exists");
        assert_eq!(found.get("P4PORT"), Some("tcp:near:1666"));
        assert_eq!(found.path(), sub.join(".p4config"));
    }
}
