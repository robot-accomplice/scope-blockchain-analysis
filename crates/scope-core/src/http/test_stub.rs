//! Test helpers for the stub programs the Ghola tests run: a `sh` script
//! that stands in for `ghola` or `go`.
//!
//! Linux refuses to run a file that any process still holds open for
//! writing (`ETXTBSY`, "Text file busy"). That can happen even after the
//! writer closed the file. Rust opens files with `O_CLOEXEC`, but a fork
//! copies the descriptor, and the copy lives in the child until the child
//! calls `exec`. If another test thread forks while this thread has the
//! stub open, the child holds a write descriptor for a short time. Then
//! this thread's `exec` of the stub fails. Moving the file (`rename`, a
//! hard link) does not help: the open descriptor points to the same
//! inode. No order of calls in the writing thread can close the window.
//!
//! So the one safe fix is to run the stub again when the error is `ETXTBSY`.
//! The child's `exec` ends the window, so a short bounded retry always
//! succeeds. See rust-lang/rust#114554.

use crate::error::ScopeError;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The text of the OS error for `ETXTBSY`.
const BUSY_MESSAGE: &str = "Text file busy";
/// How many times to retry. With the pause below this allows one second.
const BUSY_RETRIES: u32 = 50;
/// The pause between two tries.
const BUSY_PAUSE: Duration = Duration::from_millis(20);
/// The permissions of a stub: readable and executable by everyone.
const STUB_MODE: u32 = 0o755;

/// An executable `sh` script named `name` in `dir`.
///
/// Run it through [`retry_while_busy`].
pub(super) fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(STUB_MODE)).unwrap();
    path
}

/// Call `run` again while it fails because a stub is busy (`ETXTBSY`).
/// Any other result, and the last result after the retries, is returned.
pub(super) fn retry_while_busy<T>(
    mut run: impl FnMut() -> Result<T, ScopeError>,
) -> Result<T, ScopeError> {
    for _ in 0..BUSY_RETRIES {
        match run() {
            Err(e) if e.to_string().contains(BUSY_MESSAGE) => std::thread::sleep(BUSY_PAUSE),
            other => return other,
        }
    }
    run()
}
