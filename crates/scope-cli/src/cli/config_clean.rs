//! # Config clean
//!
//! `scope setup --clean [--write]` (also `scope config --clean`): removes
//! the config keys of the removed monitor dashboard from the config file.
//! The planning is pure and lives in [`scope::config_clean`]. This module
//! does the file work: the dry run, the backup, the temporary file, the
//! rename and the log. See `docs/design/2026-11-12-config-clean-design.md`.
//!
//! A dry run (the default) prints the diff and changes nothing. `--write`
//! copies the file to `<name>.backup-<stamp>` beside it, writes
//! `<name>.cleaning`, and renames that over the config. Any failure leaves
//! the original file as it was. Every `--write` run appends one JSON line
//! to `<home>/state/config-clean.log`, also when it fails.
//!
//! Exit codes: 0 for a dry run, a done write and "nothing to clean"; 1 for
//! every error (the binary maps an `Err` to 1).

use crate::cli::output::Output;
use crate::{note, out, outln};
use scope::config::{Config, clean_command};
use scope::config_clean::{Plan, diff_body, plan};
use scope::error::{Result, ScopeError};
use sha2::{Digest, Sha256};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The most backups with one time stamp. The 100th is an error.
pub const MAX_BACKUPS_PER_STAMP: u32 = 99;

/// The log file name, in `<home>/state`.
pub const LOG_FILE: &str = "config-clean.log";

/// Suffix of the temporary file, beside the config.
const TEMP_SUFFIX: &str = ".cleaning";

/// The parts of a run that a test replaces.
pub struct Hooks<'a> {
    /// The time stamp in the backup name.
    pub stamp: String,
    /// Renames the temporary file over the config.
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    /// Runs after the backup, before the file is read again.
    pub after_backup: &'a dyn Fn(&Path),
    /// The log file.
    pub log: PathBuf,
}

/// Runs the command on `file`.
///
/// # Errors
///
/// Returns an error (exit code 1) when the file is missing, not UTF-8, not
/// valid YAML, in a shape the scanner refuses, when a check fails, or when
/// the backup, the write or the rename fails.
pub fn run(file: &Path, write: bool, out: &Output) -> Result<()> {
    let hooks = Hooks {
        stamp: chrono::Local::now().format("%Y-%m-%d-%H%M%S").to_string(),
        rename: &|from, to| std::fs::rename(from, to),
        after_backup: &|_| {},
        log: scope::paths::state_dir().join(LOG_FILE),
    };
    run_with(file, write, &hooks, out)
}

/// What a `--write` run records in the log.
struct Trail<'a> {
    config: &'a Path,
    backup: Option<PathBuf>,
    plan: &'a Plan,
    bytes_before: &'a [u8],
}

impl Trail<'_> {
    /// Appends one JSON line. A failure is a note and changes nothing else.
    fn log(&self, log: &Path, result: &str, bytes_after: Option<&[u8]>, out: &Output) {
        let removed: Vec<_> = self
            .plan
            .removed
            .iter()
            .map(|r| serde_json::json!({"line": r.line, "text": r.text}))
            .collect();
        let entry = serde_json::json!({
            "at": chrono::Local::now().to_rfc3339(),
            "config": self.config.display().to_string(),
            "backup": self.backup.as_ref().map(|b| b.display().to_string()),
            "keys": self.plan.keys,
            "removed": removed,
            "bytes_before": self.bytes_before.len(),
            "bytes_after": bytes_after.map(<[u8]>::len),
            "sha256_before": hex::encode(Sha256::digest(self.bytes_before)),
            "sha256_after": bytes_after.map(|b| hex::encode(Sha256::digest(b))),
            "result": result,
        });
        tracing::info!(target: "config_clean", result, config = %self.config.display(), "config clean finished");
        let appended = (|| -> io::Result<()> {
            if let Some(dir) = log.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)?;
            writeln!(f, "{entry}")
        })();
        if let Err(e) = appended {
            note!(out, "Could not write the log {}: {e}", log.display());
        }
    }
}

/// [`run`] with the replaceable parts given.
pub fn run_with(file: &Path, write: bool, hooks: &Hooks<'_>, out: &Output) -> Result<()> {
    // A symbolic link stays a link: edit the file it points to.
    let is_link = std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink());
    let path = if is_link {
        std::fs::canonicalize(file)
            .map_err(|e| ScopeError::io_context(e, format!("cannot follow {}", file.display())))?
    } else {
        file.to_path_buf()
    };
    let mode = if write { "" } else { " (dry run)" };
    note!(out, "config clean{mode}: {}", path.display());

    if !path.exists() {
        return Err(ScopeError::Other(format!(
            "no config file at {}",
            path.display()
        )));
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| ScopeError::io_context(e, format!("cannot read {}", path.display())))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        ScopeError::Other(format!(
            "{} is not valid UTF-8; nothing was changed",
            path.display()
        ))
    })?;

    let plan = match plan(text) {
        Ok(Some(plan)) => plan,
        Ok(None) => {
            note!(out, "no dead keys in {}; nothing to do", path.display());
            return Ok(());
        }
        Err(e) => return Err(ScopeError::Other(format!("{e} Nothing was changed."))),
    };
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let keys = plan.keys.join(", ");
    let counts = format!(
        "{} {} ({keys}) in {} {}",
        plan.keys.len(),
        plural(plan.keys.len(), "key", "keys"),
        plan.removed.len(),
        plural(plan.removed.len(), "line", "lines"),
    );
    let show = |out: &Output| -> io::Result<()> {
        outln!(out, "--- {name}")?;
        outln!(out, "+++ {name} (cleaned)")?;
        out!(out, "{}", diff_body(&plan.removed))
    };

    if !write {
        show(out)?;
        note!(out, "would remove {counts}. Nothing was changed.");
        note!(out, "run `{} --write` to apply.", command_for(&path));
        return Ok(());
    }

    let mut trail = Trail {
        config: &path,
        backup: None,
        plan: &plan,
        bytes_before: &bytes,
    };
    let fail = |trail: &Trail<'_>, step: &str, msg: String| -> ScopeError {
        trail.log(&hooks.log, step, None, out);
        let backup = trail
            .backup
            .as_ref()
            .map_or_else(String::new, |b| format!(" The backup is {}.", b.display()));
        ScopeError::Other(format!("{msg}{backup} The config was not changed."))
    };

    let meta = std::fs::metadata(&path)
        .map_err(|e| ScopeError::io_context(e, format!("cannot read {}", path.display())))?;

    // 1. The backup: the exact bytes that were read.
    let backup = match create_backup(&path, &hooks.stamp, &bytes, &meta) {
        Ok(b) => b,
        Err(e) => {
            return Err(fail(
                &trail,
                "backup",
                format!("cannot make the backup: {e}."),
            ));
        }
    };
    trail.backup = Some(backup.clone());

    // 2. The temporary file.
    let temp = sibling(&path, TEMP_SUFFIX);
    let written = (|| -> io::Result<()> {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(plan.text.as_bytes())?;
        f.sync_all()?;
        std::fs::set_permissions(&temp, meta.permissions())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(fail(
            &trail,
            "write",
            format!("cannot write {}: {e}.", temp.display()),
        ));
    }

    // 3. The file must be as it was when it was read.
    (hooks.after_backup)(&path);
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        let _ = std::fs::remove_file(&temp);
        return Err(fail(
            &trail,
            "file_changed",
            "the file changed while scope was working; nothing was written.".into(),
        ));
    }

    // 4. The rename.
    if let Err(e) = (hooks.rename)(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(fail(
            &trail,
            "rename",
            format!("cannot replace {}: {e}.", path.display()),
        ));
    }

    // 5. Read it back.
    let after = std::fs::read(&path).unwrap_or_default();
    if after != plan.text.as_bytes() {
        trail.log(&hooks.log, "verify_after", Some(&after), out);
        let restore = if cfg!(windows) {
            format!("copy /Y \"{}\" \"{}\"", backup.display(), path.display())
        } else {
            format!("cp \"{}\" \"{}\"", backup.display(), path.display())
        };
        return Err(ScopeError::Other(format!(
            "the file on disk is not the planned text. Restore it with: {restore}"
        )));
    }

    // 6. Report.
    show(out)?;
    note!(out, "backup: {}", backup.display());
    note!(
        out,
        "removed {counts}. File: {} bytes to {} bytes. Delete the backup when the result is good.",
        bytes.len(),
        after.len()
    );
    trail.log(&hooks.log, "ok", Some(&after), out);
    Ok(())
}

/// The command to show in the hint: with `--config` when the file is not
/// the home file.
fn command_for(path: &Path) -> String {
    let flag = (path != Config::default_path()).then_some(path);
    clean_command(flag)
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// `<file name><suffix>` in the folder of `path`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Creates `<name>.backup-<stamp>` (then `-2`, `-3`, ...) with `create_new`,
/// so an older backup is never touched. Writes `bytes`, gives it the mode of
/// the original, and reads it back.
fn create_backup(
    path: &Path,
    stamp: &str,
    bytes: &[u8],
    meta: &std::fs::Metadata,
) -> io::Result<PathBuf> {
    for n in 1..=MAX_BACKUPS_PER_STAMP {
        let suffix = if n == 1 {
            format!(".backup-{stamp}")
        } else {
            format!(".backup-{stamp}-{n}")
        };
        let target = sibling(path, &suffix);
        let mut f = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let done = (|| -> io::Result<()> {
            f.write_all(bytes)?;
            f.sync_all()?;
            std::fs::set_permissions(&target, meta.permissions())?;
            if std::fs::read(&target)? != bytes {
                return Err(io::Error::other("the backup differs from the file"));
            }
            Ok(())
        })();
        if let Err(e) = done {
            let _ = std::fs::remove_file(&target);
            return Err(e);
        }
        return Ok(target);
    }
    Err(io::Error::other(format!(
        "{MAX_BACKUPS_PER_STAMP} backups with the stamp {stamp} exist"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a key. The tests look for it in every output.
    const SECRET: &str = "fixture-secret-token-000";

    const DIRTY: &str = "chains:
  api_keys:
    etherscan: \"fixture-secret-token-000\"
monitor:
  layout: default
  widgets:
    - price
  refresh_seconds: 7
  scale: 2
web:
  note: \"fixture-secret-token-000\"
";

    struct Case {
        dir: tempfile::TempDir,
        config: PathBuf,
        log: PathBuf,
    }

    impl Case {
        fn new(text: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("config.yaml");
            std::fs::write(&config, text).unwrap();
            let log = dir.path().join("state").join(LOG_FILE);
            Self { dir, config, log }
        }

        fn run(&self, write: bool) -> (Result<()>, String, String) {
            self.run_hooked(
                write,
                "2026-11-12-143005",
                &|from, to| std::fs::rename(from, to),
                &|_| {},
            )
        }

        fn run_hooked(
            &self,
            write: bool,
            stamp: &str,
            rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
            after_backup: &dyn Fn(&Path),
        ) -> (Result<()>, String, String) {
            let (out, cap) = Output::capture();
            let hooks = Hooks {
                stamp: stamp.to_string(),
                rename,
                after_backup,
                log: self.log.clone(),
            };
            let res = run_with(&self.config, write, &hooks, &out);
            (res, cap.out(), cap.err())
        }

        /// File names in the config folder.
        fn names(&self) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(self.dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }

        fn log_lines(&self) -> Vec<serde_json::Value> {
            std::fs::read_to_string(&self.log)
                .map(|s| {
                    s.lines()
                        .map(|l| serde_json::from_str(l).unwrap())
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    #[test]
    fn test_a_dry_run_changes_nothing() {
        // The default must be safe: same bytes, same time, no backup, no
        // temporary file, no log.
        let case = Case::new(DIRTY);
        let before = std::fs::metadata(&case.config).unwrap().modified().unwrap();
        let (res, stdout, stderr) = case.run(false);
        res.unwrap();
        assert_eq!(std::fs::read_to_string(&case.config).unwrap(), DIRTY);
        assert_eq!(
            std::fs::metadata(&case.config).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(case.names(), ["config.yaml"]);
        assert!(!case.log.exists());
        assert!(stdout.contains("@@ -5,"), "{stdout}");
        assert!(stderr.contains("dry run"), "{stderr}");
        assert!(stderr.contains("Nothing was changed"), "{stderr}");
        assert!(stderr.contains("--write"), "{stderr}");
    }

    #[test]
    fn test_write_makes_a_byte_exact_backup_and_the_planned_file() {
        // This is the contract: the backup is the file that was there, and
        // the new file is what the dry run showed.
        let case = Case::new(DIRTY);
        let (res, stdout, stderr) = case.run(true);
        res.unwrap();
        let backup = case.dir.path().join("config.yaml.backup-2026-11-12-143005");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), DIRTY);
        let planned = plan(DIRTY).unwrap().unwrap().text;
        assert_eq!(std::fs::read_to_string(&case.config).unwrap(), planned);
        assert!(stdout.contains("-  layout: default"), "{stdout}");
        assert!(stdout.contains("@@ -5,"), "{stdout}");
        assert!(
            stderr.contains("config.yaml.backup-2026-11-12-143005"),
            "{stderr}"
        );
        assert_eq!(
            case.names(),
            [
                "config.yaml",
                "config.yaml.backup-2026-11-12-143005",
                "state"
            ]
        );
    }

    #[test]
    fn test_a_second_run_changes_nothing_and_makes_no_backup() {
        // Idempotent: cleaning a clean file must not pile up backups.
        let case = Case::new(DIRTY);
        case.run(true).0.unwrap();
        let after_first = std::fs::read(&case.config).unwrap();
        let (res, _, stderr) = case.run_hooked(
            true,
            "2026-11-12-150000",
            &|a, b| std::fs::rename(a, b),
            &|_| {},
        );
        res.unwrap();
        assert_eq!(std::fs::read(&case.config).unwrap(), after_first);
        assert_eq!(
            case.names(),
            [
                "config.yaml",
                "config.yaml.backup-2026-11-12-143005",
                "state"
            ]
        );
        assert!(stderr.contains("nothing to do"), "{stderr}");
        assert_eq!(case.log_lines().len(), 1);
    }

    #[test]
    fn test_a_backup_never_overwrites_an_older_one() {
        // Two runs in the same second must keep both backups.
        let case = Case::new(DIRTY);
        let old = case.dir.path().join("config.yaml.backup-2026-11-12-143005");
        std::fs::write(&old, "older backup").unwrap();
        case.run(true).0.unwrap();
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "older backup");
        let second = case
            .dir
            .path()
            .join("config.yaml.backup-2026-11-12-143005-2");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), DIRTY);
    }

    #[test]
    fn test_a_failed_rename_costs_nothing() {
        // A failed write must never cost the config: original intact,
        // temporary file gone, backup kept, message names it, exit 1.
        let case = Case::new(DIRTY);
        let fail = |_: &Path, _: &Path| Err(io::Error::other("locked by another program"));
        let (res, _, _) = case.run_hooked(true, "2026-11-12-143005", &fail, &|_| {});
        let err = res.unwrap_err().to_string();
        assert!(err.contains("locked by another program"), "{err}");
        assert!(
            err.contains("config.yaml.backup-2026-11-12-143005"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&case.config).unwrap(), DIRTY);
        assert_eq!(
            case.names(),
            [
                "config.yaml",
                "config.yaml.backup-2026-11-12-143005",
                "state"
            ]
        );
        assert_eq!(case.log_lines()[0]["result"], "rename");
    }

    #[test]
    fn test_a_file_changed_during_the_run_is_not_overwritten() {
        // The user's edit made in an editor must win.
        let case = Case::new(DIRTY);
        let edited = "chains: {}\n# edited while scope ran\nmonitor:\n  layout: x\n";
        let edit = |p: &Path| std::fs::write(p, edited).unwrap();
        let (res, _, _) = case.run_hooked(
            true,
            "2026-11-12-143005",
            &|a, b| std::fs::rename(a, b),
            &edit,
        );
        let err = res.unwrap_err().to_string();
        assert!(err.contains("changed while scope was working"), "{err}");
        assert_eq!(std::fs::read_to_string(&case.config).unwrap(), edited);
        assert!(!case.names().iter().any(|n| n.ends_with(".cleaning")));
        assert_eq!(case.log_lines()[0]["result"], "file_changed");
    }

    #[test]
    fn test_missing_file_bad_yaml_and_nothing_to_clean() {
        // Each case has its own outcome and must not leave files.
        let missing = Case::new("");
        std::fs::remove_file(&missing.config).unwrap();
        let (res, _, _) = missing.run(true);
        assert!(res.unwrap_err().to_string().contains("no config file at"));
        assert!(missing.names().is_empty(), "{:?}", missing.names());

        let bad = Case::new("monitor: [unclosed\n");
        let (res, _, _) = bad.run(true);
        assert!(res.unwrap_err().to_string().contains("Nothing was changed"));
        assert_eq!(
            std::fs::read_to_string(&bad.config).unwrap(),
            "monitor: [unclosed\n"
        );
        assert_eq!(bad.names(), ["config.yaml"]);

        let clean = Case::new("monitor:\n  venue: b\n");
        let (res, _, stderr) = clean.run(true);
        res.unwrap();
        assert!(stderr.contains("nothing to do"), "{stderr}");
        assert_eq!(clean.names(), ["config.yaml"]);

        let not_utf8 = Case::new("");
        std::fs::write(&not_utf8.config, [0xff, 0xfe, 0x00]).unwrap();
        assert!(not_utf8.run(true).0.is_err());
        assert_eq!(not_utf8.names(), ["config.yaml"]);
    }

    #[test]
    fn test_an_unsupported_shape_writes_nothing() {
        // The scanner must refuse, not guess; the file stays as it was.
        let text = "monitor: {layout: x, venue: b}\n";
        let case = Case::new(text);
        let (res, _, _) = case.run(true);
        assert!(res.unwrap_err().to_string().contains("Delete it by hand"));
        assert_eq!(std::fs::read_to_string(&case.config).unwrap(), text);
        assert_eq!(case.names(), ["config.yaml"]);
    }

    #[test]
    fn test_no_secret_in_the_output_or_the_log() {
        // The rule is "never print an API key". A diff with context lines
        // would print the lines next to `monitor:`, which hold keys.
        let case = Case::new(DIRTY);
        let (_, dry_out, dry_err) = case.run(false);
        assert!(!dry_out.contains(SECRET) && !dry_err.contains(SECRET));
        let (_, out, err) = case.run(true);
        assert!(!out.contains(SECRET) && !err.contains(SECRET));
        let log = std::fs::read_to_string(&case.log).unwrap();
        assert!(!log.contains(SECRET), "{log}");
        // The new file keeps the key lines: the test is not vacuous.
        assert!(
            std::fs::read_to_string(&case.config)
                .unwrap()
                .contains(SECRET)
        );
    }

    #[test]
    fn test_the_log_line_records_the_decision() {
        // The trail is the RCA record: keys, removed lines with numbers,
        // hashes of both files, the backup path and the result.
        let case = Case::new(DIRTY);
        case.run(true).0.unwrap();
        let lines = case.log_lines();
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l["result"], "ok");
        assert_eq!(l["keys"], serde_json::json!(["layout", "widgets", "scale"]));
        assert_eq!(
            l["removed"][0],
            serde_json::json!({"line": 5, "text": "  layout: default"})
        );
        assert_eq!(l["bytes_before"], DIRTY.len());
        assert_eq!(
            l["sha256_before"],
            hex::encode(Sha256::digest(DIRTY.as_bytes()))
        );
        assert!(
            l["backup"]
                .as_str()
                .unwrap()
                .ends_with("config.yaml.backup-2026-11-12-143005")
        );
        assert_ne!(l["sha256_before"], l["sha256_after"]);
    }

    #[test]
    fn test_a_log_that_cannot_be_written_does_not_change_the_result() {
        // A log failure is a note. The config was cleaned; exit stays 0.
        let mut case = Case::new(DIRTY);
        let blocker = case.dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        case.log = blocker.join("sub").join(LOG_FILE);
        let (res, _, stderr) = case.run(true);
        res.unwrap();
        assert!(stderr.contains("Could not write the log"), "{stderr}");
        assert!(
            !std::fs::read_to_string(&case.config)
                .unwrap()
                .contains("layout")
        );
    }

    #[test]
    fn test_the_hint_names_the_file_when_it_is_not_the_home_file() {
        // A bare `scope setup --clean --write` would clean another file.
        let case = Case::new(DIRTY);
        let (_, _, stderr) = case.run(false);
        assert!(stderr.contains("scope --config \""), "{stderr}");
        assert!(stderr.contains("setup --clean --write"), "{stderr}");
    }

    #[test]
    fn test_crlf_files_stay_crlf_through_the_command() {
        // A Windows config must not become a mixed file on disk.
        let crlf = DIRTY.replace('\n', "\r\n");
        let case = Case::new(&crlf);
        case.run(true).0.unwrap();
        let now = std::fs::read(&case.config).unwrap();
        let text = String::from_utf8(now).unwrap();
        assert!(
            !text.replace("\r\n", "").contains('\n'),
            "bare LF in {text:?}"
        );
        assert!(text.contains("refresh_seconds: 7\r\n"));
    }

    #[test]
    fn test_a_failed_step_is_logged_with_the_step_name() {
        // The log is the record of a failed run, not only of a good one.
        let case = Case::new(DIRTY);
        // Make the backup fail: the name is taken 99 times.
        for n in 1..=MAX_BACKUPS_PER_STAMP {
            let suffix = if n == 1 {
                String::new()
            } else {
                format!("-{n}")
            };
            std::fs::write(
                case.dir
                    .path()
                    .join(format!("config.yaml.backup-s{suffix}")),
                "x",
            )
            .unwrap();
        }
        let (res, _, _) = case.run_hooked(true, "s", &|a, b| std::fs::rename(a, b), &|_| {});
        assert!(res.unwrap_err().to_string().contains("99 backups"));
        assert_eq!(std::fs::read_to_string(&case.config).unwrap(), DIRTY);
        assert_eq!(case.log_lines()[0]["result"], "backup");
    }

    #[cfg(unix)]
    #[test]
    fn test_mode_and_symbolic_link_are_kept() {
        // The file holds an API key: a 0600 config needs a 0600 backup and
        // result, and a link must stay a link.
        use std::os::unix::fs::PermissionsExt;
        let case = Case::new(DIRTY);
        std::fs::set_permissions(&case.config, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = case.dir.path().join("link.yaml");
        std::os::unix::fs::symlink(&case.config, &link).unwrap();
        let (out, _) = Output::capture();
        let hooks = Hooks {
            stamp: "s".into(),
            rename: &|a, b| std::fs::rename(a, b),
            after_backup: &|_| {},
            log: case.log.clone(),
        };
        run_with(&link, true, &hooks, &out).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            !std::fs::read_to_string(&case.config)
                .unwrap()
                .contains("layout")
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&case.config), 0o600);
        assert_eq!(mode(&case.dir.path().join("config.yaml.backup-s")), 0o600);
    }
}
