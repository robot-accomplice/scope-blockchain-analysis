//! Where scope keeps the user's files: one folder, `$SCOPE_BCA_HOME`, else
//! `~/.scope-bca`.
//!
//! ```text
//! <home>/config.yaml   venues/   data/ (address book, tokens)
//!        state/ (session, history, tui-events, migration log, daemon files)
//! ```
//!
//! This module is the only place that knows the layout. Code calls these
//! functions instead of `dirs::*` (clippy's `disallowed-methods` enforces
//! this).
//!
//! In test builds (`cfg(test)` or the `test-support` feature, which every
//! test crate turns on) every path is under a sandbox for each process,
//! never the real home, and `SCOPE_BCA_HOME` is ignored. Tests that wrote
//! the real folders replaced a user's API keys, address book and token
//! aliases with test data (#98).

pub mod migration;

use std::ffi::OsString;
use std::path::PathBuf;

/// In test builds, the environment variable that moves the sandbox, for a
/// test that needs a home it controls (for example one that cannot be
/// written).
pub const TEST_HOME_VAR: &str = "SCOPE_TEST_HOME";

/// The environment variable that moves the whole scope folder.
pub const HOME_VAR: &str = "SCOPE_BCA_HOME";

/// The scope folder's name in the user's home.
pub const HOME_DIR_NAME: &str = ".scope-bca";

/// The scope folder: `$SCOPE_BCA_HOME`, else `~/.scope-bca`.
pub fn home() -> PathBuf {
    #[cfg(any(test, feature = "test-support"))]
    return sandbox().join(HOME_DIR_NAME);
    #[cfg(not(any(test, feature = "test-support")))]
    home_from(std::env::var_os(HOME_VAR), user_home())
}

/// The root for an override and a user home. An empty override is no
/// override; with no user home the folder is relative.
pub fn home_from(override_root: Option<OsString>, user_home: Option<PathBuf>) -> PathBuf {
    match override_root.filter(|r| !r.is_empty()) {
        Some(root) => PathBuf::from(root),
        None => user_home
            .map(|h| h.join(HOME_DIR_NAME))
            .unwrap_or_else(|| PathBuf::from(HOME_DIR_NAME)),
    }
}

/// `<home>/config.yaml`.
pub fn config_file() -> PathBuf {
    home().join("config.yaml")
}

/// `<home>/venues`: the user's venue descriptors.
pub fn venues_dir() -> PathBuf {
    home().join("venues")
}

/// `<home>/data`: what scope keeps for the user.
pub fn data_dir() -> PathBuf {
    home().join("data")
}

/// `<home>/state`: session, history and logs; safe to delete.
pub fn state_dir() -> PathBuf {
    home().join("state")
}

/// The user's home folder (for `~` in paths the user types).
#[allow(clippy::disallowed_methods)]
pub fn user_home() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox());
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::home_dir()
}

/// The platform's config folder, where old versions could keep the config.
/// Only the migration uses it.
#[allow(clippy::disallowed_methods)]
pub(crate) fn legacy_config_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox().join("legacy-config"));
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::config_dir()
}

/// The platform's data folder, where old versions kept tokens and TUI
/// state. Only the migration uses it.
#[allow(clippy::disallowed_methods)]
pub(crate) fn legacy_data_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox().join("legacy-data"));
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::data_dir()
}

/// The test sandbox: `SCOPE_TEST_HOME` when set, else a directory for this
/// process under the system temp directory, emptied on first use.
#[cfg(any(test, feature = "test-support"))]
fn sandbox() -> PathBuf {
    static FRESH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(dir) = std::env::var_os(TEST_HOME_VAR) {
        return PathBuf::from(dir);
    }
    FRESH
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("scope-test-home-{}", std::process::id()));
            // Process ids are reused: a finished test process with this id
            // may have left its files here, such as a saved token choice
            // that made a later run's ambiguous token resolve.
            let _ = std::fs::remove_dir_all(&dir);
            dir
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_test_builds_never_resolve_the_real_home() {
        // The guard against tests writing the user's files: every
        // directory is in the sandbox under the temp directory.
        let temp = std::env::temp_dir();
        let mut all = vec![home(), config_file(), venues_dir(), data_dir(), state_dir()];
        all.extend(user_home());
        all.extend(legacy_config_dir());
        all.extend(legacy_data_dir());
        for dir in all {
            assert!(
                dir.starts_with(&temp),
                "{} is outside the sandbox",
                dir.display()
            );
        }
    }

    #[test]
    fn test_the_layout_is_under_one_home() {
        let root = home();
        assert_eq!(config_file(), root.join("config.yaml"));
        assert_eq!(venues_dir(), root.join("venues"));
        assert_eq!(data_dir(), root.join("data"));
        assert_eq!(state_dir(), root.join("state"));
    }

    #[test]
    fn test_home_from_uses_the_override_else_the_user_home() {
        let user = Some(PathBuf::from("/u/jon"));
        assert_eq!(
            home_from(None, user.clone()),
            PathBuf::from("/u/jon/.scope-bca")
        );
        assert_eq!(
            home_from(Some(OsString::from("/profiles/two")), user),
            PathBuf::from("/profiles/two")
        );
    }

    #[test]
    fn test_home_from_ignores_an_empty_override() {
        // Review focus 4: `SCOPE_BCA_HOME=` must not make the root ".".
        let user = Some(PathBuf::from("/u/jon"));
        assert_eq!(
            home_from(Some(OsString::new()), user),
            PathBuf::from("/u/jon/.scope-bca")
        );
    }

    #[test]
    fn test_home_from_without_a_user_home_is_relative() {
        assert_eq!(home_from(None, None), PathBuf::from(HOME_DIR_NAME));
    }
}
