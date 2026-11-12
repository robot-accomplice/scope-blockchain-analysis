//! `scope setup --clean [--write]` through the real binary: the file it
//! cleans (`--config`, `SCOPE_CONFIG`, home), its exit codes, its log and
//! the notice that names it.
//!
//! Every run uses a temp dir as the scope home, so no test touches the real
//! `~/.scope-bca`. The key in the fixtures is not a real key.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SECRET: &str = "fixture-secret-token-000";
const DIRTY: &str = "chains:
  api_keys:
    etherscan: \"fixture-secret-token-000\"
monitor:
  layout: default
  refresh_seconds: 7
  scale: 2
";
const CLEAN: &str = "chains:
  api_keys:
    etherscan: \"fixture-secret-token-000\"
monitor:
  refresh_seconds: 7
";

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The scope home. Test builds put it under `SCOPE_TEST_HOME` and
    /// ignore `SCOPE_BCA_HOME`; both are set so the paths agree.
    fn home(&self) -> PathBuf {
        self.dir.path().join("sandbox").join(".scope-bca")
    }

    fn home_file(&self) -> PathBuf {
        self.home().join("config.yaml")
    }

    fn chosen(&self, name: &str, text: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn seed_home(&self, text: &str) {
        std::fs::create_dir_all(self.home()).unwrap();
        std::fs::write(self.home_file(), text).unwrap();
    }

    fn run(&self, args: &[&str], scope_config: Option<&Path>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_scope"));
        cmd.args(args)
            .env("SCOPE_BCA_HOME", self.home())
            .env("SCOPE_TEST_HOME", self.dir.path().join("sandbox"))
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env_remove("SCOPE_CONFIG")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(p) = scope_config {
            cmd.env("SCOPE_CONFIG", p);
        }
        cmd.output().unwrap()
    }

    fn backups(&self) -> Vec<String> {
        std::fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".backup-"))
            .collect()
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn test_config_flag_file_is_cleaned_and_the_home_file_is_not() {
    // Without the shared path rule, `--config X ... --clean` would clean
    // (or report on) the home file and leave X with its dead keys.
    let env = Env::new();
    env.seed_home(DIRTY);
    let x = env.chosen("x.yaml", DIRTY);
    let out = env.run(
        &[
            "--config",
            x.to_str().unwrap(),
            "setup",
            "--clean",
            "--write",
        ],
        None,
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(&x).unwrap(), CLEAN);
    assert_eq!(std::fs::read_to_string(env.home_file()).unwrap(), DIRTY);
    assert_eq!(env.backups().len(), 1);
    // The log is in the scope home and holds one line.
    let log = std::fs::read_to_string(env.home().join("state").join("config-clean.log")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
    assert!(log.contains("\"result\":\"ok\""), "{log}");
    assert!(!log.contains(SECRET) && !text(&out.stdout).contains(SECRET));
}

#[test]
fn test_scope_config_variable_file_is_cleaned() {
    let env = Env::new();
    env.seed_home(DIRTY);
    let x = env.chosen("from-env.yaml", DIRTY);
    let out = env.run(&["setup", "--clean", "--write"], Some(&x));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(&x).unwrap(), CLEAN);
    assert_eq!(std::fs::read_to_string(env.home_file()).unwrap(), DIRTY);
}

#[test]
fn test_config_flag_beats_scope_config_variable() {
    // The rule: flag, then variable, then home.
    let env = Env::new();
    let flag = env.chosen("flag.yaml", DIRTY);
    let var = env.chosen("var.yaml", DIRTY);
    let out = env.run(
        &[
            "--config",
            flag.to_str().unwrap(),
            "config",
            "--clean",
            "--write",
        ],
        Some(&var),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(&flag).unwrap(), CLEAN);
    assert_eq!(std::fs::read_to_string(&var).unwrap(), DIRTY);
}

#[test]
fn test_the_home_file_is_cleaned_with_no_flag_or_variable() {
    let env = Env::new();
    env.seed_home(DIRTY);
    let out = env.run(&["setup", "--clean", "--write"], None);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(env.home_file()).unwrap(), CLEAN);
}

#[test]
fn test_a_dry_run_exits_0_shows_the_diff_and_changes_nothing() {
    // Q6: a dry run that finds dead keys is still a success.
    let env = Env::new();
    let x = env.chosen("x.yaml", DIRTY);
    let out = env.run(&["--config", x.to_str().unwrap(), "setup", "--clean"], None);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("-  layout: default"), "{stdout}");
    assert!(!stdout.contains(SECRET) && !text(&out.stderr).contains(SECRET));
    assert_eq!(std::fs::read_to_string(&x).unwrap(), DIRTY);
    assert!(env.backups().is_empty());
    assert!(!env.home().join("state").join("config-clean.log").exists());
}

#[test]
fn test_exit_codes_for_errors() {
    // Missing file and bad YAML are exit 1 and leave no file; a flag
    // mistake is a usage error, exit 2.
    let env = Env::new();
    let missing = env.dir.path().join("missing.yaml");
    let out = env.run(
        &["--config", missing.to_str().unwrap(), "setup", "--clean"],
        None,
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(!missing.exists());

    let bad = env.chosen("bad.yaml", "monitor: [unclosed\n");
    let out = env.run(
        &[
            "--config",
            bad.to_str().unwrap(),
            "setup",
            "--clean",
            "--write",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(&bad).unwrap(),
        "monitor: [unclosed\n"
    );
    assert!(env.backups().is_empty());

    let out = env.run(&["setup", "--write"], None);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn test_the_load_notice_names_the_command_once() {
    // The notice is the only way users find the command. With --config it
    // must carry the path, or the hint would clean another file.
    let env = Env::new();
    let x = env.chosen("x.yaml", DIRTY);
    let out = env.run(
        &["--config", x.to_str().unwrap(), "setup", "--status"],
        None,
    );
    let err = text(&out.stderr);
    assert_eq!(err.matches("ignoring config keys").count(), 1, "{err}");
    assert!(err.contains("setup --clean` to remove them"), "{err}");
    assert!(err.contains("scope --config \""), "{err}");
}
