// SPDX-License-Identifier: Apache-2.0

//! The resolved configuration and the resolver that produces it.
//!
//! [`ResolvedConfig`] is the single connection-and-identity snapshot the rest of
//! the CLI reads. It is produced by [`resolve`], which consults each
//! configuration source in Perforce's documented precedence order and records,
//! per value, where the winning value came from.
//!
//! # Precedence (highest wins)
//!
//! 1. Command-line global flags (`-p`, `-u`, ...).
//! 2. The nearest `P4CONFIG` file found by walking up from the current directory.
//! 3. Process environment variables (`P4PORT`, `P4USER`, ...).
//! 4. Compiled-in defaults (only `P4PORT`, which defaults to `perforce:1666`).
//!
//! This order is taken from Perforce's published "How `p4` reads configuration"
//! documentation and was confirmed against the real client as a behavioral
//! oracle: `p4 set` prints each variable with the source it resolved from, and on
//! a machine with both an environment variable and a `.p4config` entry for the
//! same setting, `p4 set` reports the **config file** value, proving the file
//! outranks the environment. (`P4ENVIRO` and the Windows registry sit between the
//! environment and the defaults in `p4`; Astraval models those origins in
//! [`Source`] but does not read them yet, a seam left for a later issue.)
//!
//! The resolver takes all of its inputs explicitly, an environment map and a
//! starting directory rather than reaching for `std::env` itself, so it is fully
//! deterministic and unit-testable without touching the host machine (rust.md,
//! M-MOCKABLE-SYSCALLS). The binary supplies the real process environment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::Error;
use crate::config::config_file::{ConfigFile, discover};
use crate::config::setting::Setting;
use crate::config::source::Source;

/// The Perforce variable naming the `P4CONFIG` filename to search for.
///
/// `P4CONFIG` is itself read from the environment (or a flag, in `p4`'s case): it
/// does not name a value but a *filename*, e.g. `.p4config`, that the resolver
/// then walks up the tree to find. Kept as a constant so the one place that reads
/// it is named and greppable.
const P4CONFIG_VARIABLE: &str = "P4CONFIG";

/// A configuration value paired with the source it was resolved from.
///
/// The `value` is what the CLI uses; the `source` is why it has that value, which
/// the CLI surfaces to the user (the way `p4 set` annotates each line) and which
/// tests assert against to pin precedence. Sensitive values such as `P4PASSWD`
/// are still strings here; redaction, if any, happens at the render site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValue {
    /// The resolved value, e.g. `tcp:localhost:1666` for `P4PORT`.
    value: String,

    /// Where this value came from in the precedence chain.
    source: Source,
}

impl ConfigValue {
    /// Returns the resolved value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the source this value was resolved from.
    #[must_use]
    pub fn source(&self) -> &Source {
        &self.source
    }
}

/// The command-line global options that feed configuration resolution.
///
/// This mirrors the subset of the CLI's global flags that participate in
/// resolution, decoupled from the binary's clap types so the core crate does not
/// depend on the CLI. The binary translates its parsed flags into this struct.
/// Every field is optional: `None` means "the flag was not given, fall back to a
/// lower-precedence source".
///
/// `dir` is the `-d` current-directory override. It is not a resolved *setting*
/// but an input to resolution: it changes the directory the `P4CONFIG` search
/// starts from. When `None`, the resolver uses the `start_dir` it is given (the
/// process's real current directory).
#[derive(Debug, Clone, Default)]
pub struct GlobalSettings {
    /// `-p`: the server address (`P4PORT`).
    pub port: Option<String>,
    /// `-u`: the user name (`P4USER`).
    pub user: Option<String>,
    /// `-c`: the client/workspace name (`P4CLIENT`).
    pub client: Option<String>,
    /// `-P`: the password or ticket (`P4PASSWD`).
    pub password: Option<String>,
    /// `-C`: the client character set (`P4CHARSET`).
    pub charset: Option<String>,
    /// `-H`: the host name override (`P4HOST`).
    pub host: Option<String>,
    /// `-d`: override the directory the `P4CONFIG` search starts from.
    pub dir: Option<PathBuf>,
}

impl GlobalSettings {
    /// Returns the flag value for `setting`, if one was supplied on the command line.
    ///
    /// Maps each [`Setting`] to its corresponding `-x` flag field. This is the
    /// highest-precedence source the resolver consults.
    fn flag_for(&self, setting: Setting) -> Option<&str> {
        match setting {
            Setting::Port => self.port.as_deref(),
            Setting::User => self.user.as_deref(),
            Setting::Client => self.client.as_deref(),
            Setting::Password => self.password.as_deref(),
            Setting::Charset => self.charset.as_deref(),
            Setting::Host => self.host.as_deref(),
        }
    }
}

/// A fully resolved connection-and-identity configuration.
///
/// Holds one optional [`ConfigValue`] per [`Setting`]. A setting is `None` when no
/// source supplied it and it has no default (only `P4PORT` has a default, so it is
/// the one setting guaranteed to be present). Construct one with [`resolve`]; read
/// values with [`ResolvedConfig::get`] or the named accessors.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// One slot per setting, in [`Setting::ALL`] order. `None` means unresolved.
    values: HashMap<Setting, ConfigValue>,
}

impl ResolvedConfig {
    /// Returns the resolved value and source for `setting`, if it has one.
    ///
    /// Returns `None` for a setting that no source supplied and that has no
    /// default. Callers that only need the string can chain
    /// [`ConfigValue::value`].
    #[must_use]
    pub fn get(&self, setting: Setting) -> Option<&ConfigValue> {
        self.values.get(&setting)
    }

    /// Returns the resolved `P4PORT` value.
    ///
    /// Always present, because `P4PORT` has the compiled-in default
    /// `perforce:1666` that resolution falls back to when nothing else sets it.
    ///
    /// # Panics
    /// Never in practice. The `expect` guards an internal invariant: `P4PORT` is
    /// the one setting with a default, so [`resolve`] always produces a value for
    /// it. A panic here would mean that default was removed, a programming error.
    #[must_use]
    pub fn port(&self) -> &str {
        // Safe to unwrap: `P4PORT` is the one setting with a default, so
        // resolution always produces a value for it. If this ever panics it
        // means the default was removed, a programming error, not a runtime one.
        self.get(Setting::Port)
            .expect("P4PORT always resolves via its compiled-in default")
            .value()
    }

    /// Returns the resolved `P4USER` value, if any source supplied one.
    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.get(Setting::User).map(ConfigValue::value)
    }

    /// Returns the resolved `P4CLIENT` value, if any source supplied one.
    #[must_use]
    pub fn client(&self) -> Option<&str> {
        self.get(Setting::Client).map(ConfigValue::value)
    }

    /// Returns the resolved `P4PASSWD` value, if any source supplied one.
    #[must_use]
    pub fn password(&self) -> Option<&str> {
        self.get(Setting::Password).map(ConfigValue::value)
    }

    /// Returns the resolved `P4CHARSET` value, if any source supplied one.
    #[must_use]
    pub fn charset(&self) -> Option<&str> {
        self.get(Setting::Charset).map(ConfigValue::value)
    }

    /// Returns the resolved `P4HOST` value, if any source supplied one.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.get(Setting::Host).map(ConfigValue::value)
    }
}

/// Resolves every setting from `flags`, `environment`, and `P4CONFIG` files.
///
/// Walks Perforce's precedence chain (flags, then the nearest `P4CONFIG` file,
/// then environment variables, then compiled-in defaults; see the module docs)
/// and returns the winning value and source for each [`Setting`]. The
/// `P4CONFIG` search starts at `flags.dir` when the `-d` override is given,
/// otherwise at `start_dir`, which the binary supplies as the process's current
/// directory.
///
/// All inputs are explicit so resolution is deterministic and testable without
/// touching the host environment. `environment` is a plain map of variable names
/// to values, exactly as the binary builds from `std::env::vars`.
///
/// # Errors
/// Returns a command-class [`Error`] only if a located `P4CONFIG` file cannot be
/// read. Missing files and absent variables are not errors; they fall through to
/// the next source.
///
/// # Examples
/// ```
/// use std::collections::HashMap;
/// use std::path::Path;
/// use astraval_core::config::{GlobalSettings, Setting, Source, resolve};
///
/// // No flags, no config file, no env: only P4PORT resolves, via its default.
/// let config = resolve(&GlobalSettings::default(), &HashMap::new(), Path::new("."))?;
/// assert_eq!(config.port(), "perforce:1666");
/// assert_eq!(config.get(Setting::Port).unwrap().source(), &Source::Default);
/// assert!(config.user().is_none());
/// # Ok::<(), astraval_core::Error>(())
/// ```
pub fn resolve<S: std::hash::BuildHasher>(
    flags: &GlobalSettings,
    environment: &HashMap<String, String, S>,
    start_dir: &Path,
) -> Result<ResolvedConfig, Error> {
    // The `-d` flag relocates where the P4CONFIG search begins; otherwise we use
    // the process's current directory as supplied by the caller.
    let search_root = flags.dir.as_deref().unwrap_or(start_dir);
    let config_file = load_config_file(environment, search_root)?;

    let mut values = HashMap::with_capacity(Setting::ALL.len());
    for setting in Setting::ALL {
        if let Some(resolved) = resolve_one(setting, flags, config_file.as_ref(), environment) {
            values.insert(setting, resolved);
        }
    }

    Ok(ResolvedConfig { values })
}

/// Resolves a single setting by consulting each source in precedence order.
///
/// Returns the first source that supplies a value, tagged with where it came
/// from, or `None` if no source has it and it has no default. Keeping this as one
/// short-circuiting chain makes the precedence order literally readable top to
/// bottom, which is the property the tests pin.
fn resolve_one<S: std::hash::BuildHasher>(
    setting: Setting,
    flags: &GlobalSettings,
    config_file: Option<&ConfigFile>,
    environment: &HashMap<String, String, S>,
) -> Option<ConfigValue> {
    let variable = setting.variable_name();

    // 1. Command-line flag: highest precedence.
    if let Some(value) = flags.flag_for(setting) {
        return Some(ConfigValue {
            value: value.to_owned(),
            source: Source::CommandLine,
        });
    }

    // 2. Nearest P4CONFIG file: outranks the environment, as the real client
    //    confirms (a config entry beats an env var of the same name).
    if let Some(file) = config_file
        && let Some(value) = file.get(variable)
    {
        return Some(ConfigValue {
            value: value.to_owned(),
            source: Source::Config(file.path().to_path_buf()),
        });
    }

    // 3. Process environment variable.
    if let Some(value) = environment.get(variable) {
        return Some(ConfigValue {
            value: value.clone(),
            source: Source::Environment,
        });
    }

    // 4. Compiled-in default, if this setting has one (only P4PORT does).
    if let Some(value) = setting.default_value() {
        return Some(ConfigValue {
            value: value.to_owned(),
            source: Source::Default,
        });
    }

    None
}

/// Finds and parses the `P4CONFIG` file, if `P4CONFIG` names one to look for.
///
/// `P4CONFIG` is read from the environment; when set, its value is a filename the
/// resolver searches for by walking up from `search_root`. When `P4CONFIG` is
/// unset (or empty), there is no file to find and this returns `Ok(None)` without
/// touching the filesystem.
fn load_config_file<S: std::hash::BuildHasher>(
    environment: &HashMap<String, String, S>,
    search_root: &Path,
) -> Result<Option<ConfigFile>, Error> {
    match environment.get(P4CONFIG_VARIABLE) {
        Some(filename) if !filename.is_empty() => discover(search_root, filename),
        // No P4CONFIG set means no config-file source participates at all.
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an environment map from `(name, value)` pairs for terse test setup.
    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn port_falls_back_to_its_compiled_in_default() {
        let config = resolve(&GlobalSettings::default(), &HashMap::new(), Path::new("."))
            .expect("resolution with no sources should succeed");

        assert_eq!(config.port(), "perforce:1666");
        assert_eq!(
            config.get(Setting::Port).map(ConfigValue::source),
            Some(&Source::Default)
        );
    }

    #[test]
    fn settings_without_a_default_resolve_to_none() {
        let config = resolve(&GlobalSettings::default(), &HashMap::new(), Path::new("."))
            .expect("resolution should succeed");

        assert!(config.user().is_none());
        assert!(config.client().is_none());
        assert!(config.password().is_none());
    }

    #[test]
    fn environment_beats_the_default() {
        let environment = env(&[("P4PORT", "tcp:envhost:1666")]);

        let config = resolve(&GlobalSettings::default(), &environment, Path::new("."))
            .expect("resolution should succeed");

        assert_eq!(config.port(), "tcp:envhost:1666");
        assert_eq!(
            config.get(Setting::Port).map(ConfigValue::source),
            Some(&Source::Environment)
        );
    }

    #[test]
    fn flag_beats_environment() {
        let flags = GlobalSettings {
            user: Some("flaguser".to_owned()),
            ..GlobalSettings::default()
        };
        let environment = env(&[("P4USER", "envuser")]);

        let config =
            resolve(&flags, &environment, Path::new(".")).expect("resolution should succeed");

        assert_eq!(config.user(), Some("flaguser"));
        assert_eq!(
            config.get(Setting::User).map(ConfigValue::source),
            Some(&Source::CommandLine)
        );
    }

    /// A unique, self-deleting temporary directory for filesystem-backed tests.
    ///
    /// Mirrors the helper in `config_file`'s tests so the resolver can exercise
    /// real `P4CONFIG` discovery without a `tempfile` dev-dependency (rust.md
    /// M-OOBE) and without leaking machine state.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};

            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "astraval-resolve-test-{}-{unique}",
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
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn config_file_beats_environment() {
        // The load-bearing precedence rule, confirmed against the real `p4 set`
        // oracle: when both a `P4CONFIG` file and an environment variable set the
        // same setting, the file wins.
        let temp = TempDir::new();
        std::fs::write(temp.path().join(".p4config"), "P4USER=configuser\n")
            .expect("writing the config file should succeed");

        let environment = env(&[("P4CONFIG", ".p4config"), ("P4USER", "envuser")]);

        let config = resolve(&GlobalSettings::default(), &environment, temp.path())
            .expect("resolution should succeed");

        assert_eq!(config.user(), Some("configuser"));
        match config.get(Setting::User).map(ConfigValue::source) {
            Some(Source::Config(path)) => assert_eq!(path, &temp.path().join(".p4config")),
            other => panic!("expected a config-file source, got {other:?}"),
        }
    }

    #[test]
    fn flag_beats_config_file() {
        // The full chain: a flag must outrank even a `P4CONFIG` file.
        let temp = TempDir::new();
        std::fs::write(
            temp.path().join(".p4config"),
            "P4PORT=tcp:confighost:1666\n",
        )
        .expect("writing the config file should succeed");

        let flags = GlobalSettings {
            port: Some("tcp:flaghost:1666".to_owned()),
            ..GlobalSettings::default()
        };
        let environment = env(&[("P4CONFIG", ".p4config")]);

        let config = resolve(&flags, &environment, temp.path()).expect("resolution should succeed");

        assert_eq!(config.port(), "tcp:flaghost:1666");
        assert_eq!(
            config.get(Setting::Port).map(ConfigValue::source),
            Some(&Source::CommandLine)
        );
    }

    #[test]
    fn dir_override_relocates_the_config_search() {
        // The `-d` override must change where the P4CONFIG walk begins. We point
        // `start_dir` at one place but `flags.dir` at the directory holding the
        // file, and expect the file to be found.
        let temp = TempDir::new();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace dir should be creatable");
        std::fs::write(workspace.join(".p4config"), "P4CLIENT=ws-client\n")
            .expect("writing the config file should succeed");

        let flags = GlobalSettings {
            dir: Some(workspace.clone()),
            ..GlobalSettings::default()
        };
        let environment = env(&[("P4CONFIG", ".p4config")]);

        // `start_dir` is the bare temp root, which has no `.p4config`; only the
        // `-d` override points resolution at the workspace that does.
        let config = resolve(&flags, &environment, temp.path()).expect("resolution should succeed");

        assert_eq!(config.client(), Some("ws-client"));
    }
}
