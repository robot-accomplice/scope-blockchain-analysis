//! # Copy and Save
//!
//! The last command's plain output, as an OSC 52 clipboard sequence (Ctrl-Y)
//! or as a file (`save`).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use scope::error::ScopeError;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// What `save` says when a relative path leaves the current directory.
pub const SAVE_ESCAPE: &str = "save writes inside the current directory or to an absolute path";

/// The usage line for `save`.
pub const SAVE_USAGE: &str = "Usage: save <file> [--force]";

/// The most base64 bytes Ctrl-Y sends. Terminals drop larger OSC 52 writes.
pub const OSC52_MAX_PAYLOAD: usize = 100_000;

/// Why `save` wrote nothing.
#[derive(Debug)]
pub enum SaveError {
    /// The request was refused: usage, path rules, nothing to save, or an
    /// existing file.
    Refused(String),
    /// The OS failed the write. The error keeps its kind and its source
    /// ([`ScopeError::io_context`]).
    Failed(ScopeError),
}

impl SaveError {
    /// An OS failure while `doing` (for example `Could not write x.txt`).
    pub fn failed(e: std::io::Error, doing: impl std::fmt::Display) -> Self {
        Self::Failed(ScopeError::io_context(e, doing))
    }

    /// The text to show and to journal.
    pub fn message(&self) -> String {
        match self {
            SaveError::Refused(m) => m.clone(),
            SaveError::Failed(ScopeError::IoError(e)) => e.to_string(),
            SaveError::Failed(other) => other.to_string(),
        }
    }
}

/// The OSC 52 sequence that puts `text` on the system clipboard.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// True when the OSC 52 payload for `text` stays within the cap.
pub fn fits_osc52(text: &str) -> bool {
    text.len().div_ceil(3) * 4 <= OSC52_MAX_PAYLOAD
}

/// Where `save` writes `arg`: the checked canonical parent joined with the
/// file name, so the path that was checked is the path that is written. A
/// `~` or a leading `~/` means `home`. A relative path with `..`, or one whose
/// parent resolves outside `cwd` (a symlink), is refused.
fn resolve(arg: &str, cwd: &Path, home: Option<&Path>) -> Result<PathBuf, SaveError> {
    let expanded = match (arg, arg.strip_prefix("~/"), home) {
        ("~", _, Some(home)) => home.to_path_buf(),
        (_, Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(arg),
    };
    let absolute = expanded.is_absolute();
    if !absolute && expanded.components().any(|c| c == Component::ParentDir) {
        return Err(SaveError::Refused(SAVE_ESCAPE.to_string()));
    }
    let Some(name) = expanded.file_name() else {
        return Err(SaveError::Refused(SAVE_USAGE.to_string()));
    };
    let joined = if absolute {
        expanded.clone()
    } else {
        cwd.join(&expanded)
    };
    let parent = joined.parent().unwrap_or(&joined);
    let real = parent
        .canonicalize()
        .map_err(|e| SaveError::failed(e, format!("Could not write {}", joined.display())))?;
    if !absolute {
        let root = cwd
            .canonicalize()
            .map_err(|e| SaveError::failed(e, format!("Could not read {}", cwd.display())))?;
        if !real.starts_with(&root) {
            return Err(SaveError::Refused(SAVE_ESCAPE.to_string()));
        }
    }
    Ok(real.join(name))
}

/// Writes `text` to `target`. Without `force` it creates the file and fails
/// when it exists; a failed write removes the partial file. With `force` it
/// writes a temp file beside the target and renames it over the target, so
/// a symlink at the target is replaced and never followed.
fn write_file(target: &Path, text: &str, force: bool) -> std::io::Result<()> {
    if !force {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        let result = f.write_all(text.as_bytes()).and_then(|()| f.sync_all());
        if result.is_err() {
            // This call created the file, so it is not anyone's data.
            let _ = std::fs::remove_file(target);
        }
        return result;
    }
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = target.with_file_name(format!(".{}.scope-save-{}", name, std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Runs `save <file> [--force]` over the last output. Returns the note to
/// show on success, or the reason nothing was written.
pub fn save(
    args: &[&str],
    last: Option<&str>,
    cwd: &Path,
    home: Option<&Path>,
) -> Result<String, SaveError> {
    let force = args.contains(&"--force");
    let files: Vec<&str> = args.iter().copied().filter(|a| *a != "--force").collect();
    let [file] = files.as_slice() else {
        return Err(SaveError::Refused(SAVE_USAGE.to_string()));
    };
    let target = resolve(file, cwd, home)?;
    let Some(text) = last else {
        return Err(SaveError::Refused("nothing to save yet".to_string()));
    };
    match std::fs::symlink_metadata(&target) {
        Ok(m) if m.is_dir() => {
            return Err(SaveError::Refused(format!(
                "{} is a directory",
                target.display()
            )));
        }
        Ok(_) if !force => {
            return Err(SaveError::Refused(format!(
                "{} exists; add --force to replace it",
                target.display()
            )));
        }
        _ => {}
    }
    write_file(&target, text, force)
        .map_err(|e| SaveError::failed(e, format!("Could not write {}", target.display())))?;
    let n = text.lines().count();
    Ok(format!(
        "saved {} line{} to {}",
        n,
        if n == 1 { "" } else { "s" },
        target.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(args: &[&str], text: Option<&str>, cwd: &Path) -> Result<String, SaveError> {
        save(args, text, cwd, None)
    }

    fn refused(r: Result<String, SaveError>) -> String {
        match r {
            Err(SaveError::Refused(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn test_osc52_encodes_base64_payload() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn test_osc52_cap_is_on_the_base64_payload() {
        let at_cap = "a".repeat(OSC52_MAX_PAYLOAD / 4 * 3);
        assert!(fits_osc52(&at_cap));
        assert!(!fits_osc52(&format!("{at_cap}a")));
    }

    #[test]
    fn test_save_writes_inside_cwd_and_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let note = saved(&["out.json"], Some("{\"a\":1}\n"), dir.path()).unwrap();
        assert!(note.starts_with("saved 1 line to "), "{note}");
        let path = dir.path().join("out.json");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");

        let again = refused(saved(&["out.json"], Some("new"), dir.path()));
        assert!(again.contains("exists; add --force"), "{again}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");

        let note = saved(&["--force", "out.json"], Some("a\nb\n"), dir.path()).unwrap();
        assert!(note.starts_with("saved 2 lines to "), "{note}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\n");
    }

    #[test]
    fn test_save_refuses_a_path_that_climbs_out() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir(&cwd).unwrap();
        for arg in ["../x.json", "sub/../../x.json", ".."] {
            assert_eq!(
                refused(saved(&[arg], Some("t"), &cwd)),
                SAVE_ESCAPE,
                "{arg}"
            );
        }
        assert!(!dir.path().join("x.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_save_refuses_a_symlinked_parent_outside_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, cwd.join("link")).unwrap();
        assert_eq!(
            refused(saved(&["link/x.json"], Some("t"), &cwd)),
            SAVE_ESCAPE
        );
        assert!(!outside.join("x.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_save_force_replaces_a_symlink_and_never_writes_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("out.txt")).unwrap();
        // Without --force the link counts as an existing file.
        assert!(refused(saved(&["out.txt"], Some("new"), dir.path())).contains("exists"));
        saved(&["--force", "out.txt"], Some("new"), dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        let meta = std::fs::symlink_metadata(dir.path().join("out.txt")).unwrap();
        assert!(meta.is_file(), "the link is replaced by a plain file");
    }

    #[test]
    fn test_save_allows_an_absolute_path_and_reports_os_errors() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let abs = elsewhere.path().join("a.txt");
        saved(&[abs.to_str().unwrap()], Some("t"), dir.path()).unwrap();

        let missing = elsewhere.path().join("no-dir").join("a.txt");
        let Err(failed @ SaveError::Failed(_)) =
            saved(&[missing.to_str().unwrap()], Some("t"), dir.path())
        else {
            panic!("expected an OS failure");
        };
        let SaveError::Failed(ScopeError::IoError(e)) = &failed else {
            panic!("an I/O error");
        };
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "the kind stays");
        assert!(std::error::Error::source(e).is_some(), "the source stays");
        let note = failed.message();
        assert!(note.starts_with("Could not write "), "{note}");
        assert!(note.contains("no-dir"), "{note}");
        assert!(matches!(
            saved(&["no-dir/a.txt"], Some("t"), dir.path()),
            Err(SaveError::Failed(_))
        ));
    }

    #[test]
    fn test_save_needs_one_file_and_some_output() {
        let dir = tempfile::tempdir().unwrap();
        for args in [&[][..], &["a", "b"], &[""], &["."]] {
            assert_eq!(
                refused(saved(args, Some("t"), dir.path())),
                SAVE_USAGE,
                "{args:?}"
            );
        }
        assert_eq!(
            refused(saved(&["a"], None, dir.path())),
            "nothing to save yet"
        );
    }

    #[test]
    fn test_save_to_a_directory_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        for force in [false, true] {
            let args: Vec<&str> = if force {
                vec!["--force", "d"]
            } else {
                vec!["d"]
            };
            let m = refused(saved(&args, Some("t"), dir.path()));
            assert!(m.ends_with("d is a directory"), "{m}");
        }
    }

    #[test]
    fn test_save_expands_a_leading_tilde_to_home() {
        let cwd = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        save(&["~/h.txt"], Some("t"), cwd.path(), Some(home.path())).unwrap();
        assert!(home.path().join("h.txt").exists());
        assert!(!cwd.path().join("~").exists());
    }

    #[test]
    fn test_save_to_bare_tilde_means_the_home_directory() {
        let cwd = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let err = save(&["~"], Some("t"), cwd.path(), Some(home.path())).unwrap_err();
        assert!(format!("{err:?}").contains("is a directory"), "{err:?}");
        assert!(!cwd.path().join("~").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_a_refused_create_never_removes_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("q.txt");
        std::fs::write(&big, "").unwrap();
        assert!(
            write_file(&big, "x", false).is_err(),
            "exists: nothing is touched"
        );
        assert!(big.exists(), "an existing file is never removed");
    }
}
