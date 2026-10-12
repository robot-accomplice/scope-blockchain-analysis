//! # Copy and Save
//!
//! The last command's plain output, as an OSC 52 clipboard sequence (Ctrl-Y)
//! or as a file (`save`).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// What `save` says when a relative path leaves the current directory.
pub const SAVE_ESCAPE: &str = "save writes inside the current directory or to an absolute path";

/// The usage line for `save`.
pub const SAVE_USAGE: &str = "Usage: save <file> [--force]";

/// The OSC 52 sequence that puts `text` on the system clipboard.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// `text` without ANSI escape sequences (CSI and OSC).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then one final byte in 0x40..=0x7e.
            Some('[') => {
                for n in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: ends at BEL or at ESC \.
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\x07' {
                        break;
                    }
                    if n == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other two-byte escape.
            _ => {}
        }
    }
    out
}

/// Where `save` writes `arg`: an absolute path as given, or a relative path
/// inside `cwd`. A relative path with `..`, or one whose parent resolves
/// outside `cwd` (a symlink), is refused.
fn resolve(arg: &str, cwd: &Path) -> Result<PathBuf, String> {
    let path = Path::new(arg);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(SAVE_ESCAPE.to_string());
    }
    let target = cwd.join(path);
    let (Some(parent), Some(_)) = (target.parent(), target.file_name()) else {
        return Err(format!("{}: not a file name", arg));
    };
    let root = cwd
        .canonicalize()
        .map_err(|e| format!("Could not read {}: {}", cwd.display(), e))?;
    let real = parent
        .canonicalize()
        .map_err(|e| format!("Could not write {}: {}", target.display(), e))?;
    if !real.starts_with(&root) {
        return Err(SAVE_ESCAPE.to_string());
    }
    Ok(target)
}

/// Writes `text` to `target`. Without `force` it creates the file and fails
/// when it exists. With `force` it writes a temp file beside the target and
/// renames it over the target, so a symlink at the target is replaced and
/// never followed.
fn write_file(target: &Path, text: &str, force: bool) -> std::io::Result<()> {
    if !force {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        return f.write_all(text.as_bytes());
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
/// show: the result or the reason it did not write.
pub fn save(args: &[&str], last: Option<&str>, cwd: &Path) -> String {
    let force = args.contains(&"--force");
    let files: Vec<&str> = args.iter().copied().filter(|a| *a != "--force").collect();
    let [file] = files.as_slice() else {
        return SAVE_USAGE.to_string();
    };
    let Some(text) = last else {
        return "nothing to save yet".to_string();
    };
    let target = match resolve(file, cwd) {
        Ok(t) => t,
        Err(reason) => return reason,
    };
    if !force && std::fs::symlink_metadata(&target).is_ok() {
        return format!("{} exists; add --force to replace it", target.display());
    }
    match write_file(&target, text, force) {
        Ok(()) => format!(
            "saved {} lines to {}",
            text.lines().count(),
            target.display()
        ),
        Err(e) => format!("Could not write {}: {}", target.display(), e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc52_encodes_base64_payload() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn test_strip_ansi_removes_sgr() {
        assert_eq!(strip_ansi("\x1b[1;36mA\x1b[0m b"), "A b");
    }

    #[test]
    fn test_strip_ansi_removes_osc_and_keeps_text() {
        assert_eq!(
            strip_ansi("\x1b]8;;http://x\x1b\\link\x1b]8;;\x07!"),
            "link!"
        );
        assert_eq!(strip_ansi("a\x1bXb"), "ab");
    }

    #[test]
    fn test_save_writes_inside_cwd_and_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let note = save(&["out.json"], Some("{\"a\":1}\n"), dir.path());
        assert!(note.starts_with("saved 1 lines to "), "{note}");
        let path = dir.path().join("out.json");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");

        let again = save(&["out.json"], Some("new"), dir.path());
        assert!(again.contains("exists; add --force"), "{again}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");

        assert!(save(&["--force", "out.json"], Some("new"), dir.path()).starts_with("saved"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn test_save_refuses_a_path_that_climbs_out() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir(&cwd).unwrap();
        for arg in ["../x.json", "sub/../../x.json"] {
            assert_eq!(save(&[arg], Some("t"), &cwd), SAVE_ESCAPE, "{arg}");
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
        assert_eq!(save(&["link/x.json"], Some("t"), &cwd), SAVE_ESCAPE);
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
        assert!(save(&["out.txt"], Some("new"), dir.path()).contains("exists"));
        assert!(save(&["--force", "out.txt"], Some("new"), dir.path()).starts_with("saved"));
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        let meta = std::fs::symlink_metadata(dir.path().join("out.txt")).unwrap();
        assert!(meta.is_file(), "the link is replaced by a plain file");
    }

    #[test]
    fn test_save_allows_an_absolute_path_and_reports_os_errors() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let abs = elsewhere.path().join("a.txt");
        let note = save(&[abs.to_str().unwrap()], Some("t"), dir.path());
        assert!(note.starts_with("saved"), "{note}");

        let missing = elsewhere.path().join("no-dir").join("a.txt");
        let note = save(&[missing.to_str().unwrap()], Some("t"), dir.path());
        assert!(note.starts_with("Could not write "), "{note}");
        assert!(note.contains(missing.to_str().unwrap()), "{note}");
        let note = save(&["no-dir/a.txt"], Some("t"), dir.path());
        assert!(note.starts_with("Could not write "), "{note}");
    }

    #[test]
    fn test_save_needs_one_file_and_some_output() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(save(&[], Some("t"), dir.path()), SAVE_USAGE);
        assert_eq!(save(&["a", "b"], Some("t"), dir.path()), SAVE_USAGE);
        assert_eq!(save(&["a"], None, dir.path()), "nothing to save yet");
    }
}
