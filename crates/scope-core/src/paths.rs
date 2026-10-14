//! The user's directories, resolved in one place.
//!
//! Every file scope keeps for the user (config, address book, token
//! aliases, session, journal, venues) is found through these functions.
//! Code calls them instead of `dirs::*` (clippy's `disallowed-methods`
//! enforces this).
//!
//! In test builds (`cfg(test)` or the `test-support` feature, which every
//! test crate turns on) they resolve under a sandbox directory for each
//! process, never the real home. Tests that wrote the real directories
//! replaced a user's API keys, address book and token aliases with test
//! data.

use std::path::PathBuf;

/// In test builds, the environment variable that moves the sandbox, for a
/// test that needs a home it controls (for example one that cannot be
/// written).
pub const TEST_HOME_VAR: &str = "SCOPE_TEST_HOME";

/// The home directory.
#[allow(clippy::disallowed_methods)]
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox());
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::home_dir()
}

/// The platform's config directory (for example
/// `~/Library/Application Support` on macOS).
#[allow(clippy::disallowed_methods)]
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox().join("config"));
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::config_dir()
}

/// The platform's data directory (for example
/// `~/Library/Application Support` on macOS).
#[allow(clippy::disallowed_methods)]
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return Some(sandbox().join("data"));
    #[cfg(not(any(test, feature = "test-support")))]
    dirs::data_dir()
}

/// The test sandbox: `SCOPE_TEST_HOME` when set, else a directory for this
/// process under the system temp directory.
#[cfg(any(test, feature = "test-support"))]
fn sandbox() -> PathBuf {
    std::env::var_os(TEST_HOME_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("scope-test-home-{}", std::process::id()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_builds_never_resolve_the_real_home() {
        // The guard against tests writing the user's files: every
        // directory is in the sandbox under the temp directory.
        let temp = std::env::temp_dir();
        for dir in [home_dir(), config_dir(), data_dir()] {
            let dir = dir.unwrap();
            assert!(
                dir.starts_with(&temp),
                "{} is outside the sandbox",
                dir.display()
            );
        }
    }
}
