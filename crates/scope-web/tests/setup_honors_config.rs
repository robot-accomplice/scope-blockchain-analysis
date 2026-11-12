//! `scope setup` reads and writes the config file the user chose:
//! `--config`, else `SCOPE_CONFIG`, else `<home>/config.yaml`.
//!
//! Every run uses a temp dir as `SCOPE_BCA_HOME`, so no test touches the
//! real `~/.scope-bca`. The key in the fixtures is not a real key.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const KEY: &str = "fixture-key-not-real-0001";
const HOME_FILE: &str = "chains:\n  api_keys:\n    etherscan: \"home-file-marker-0000\"\n";

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

    fn chosen(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Writes the home file and returns its bytes.
    fn seed_home(&self) -> Vec<u8> {
        let p = self.home_file();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, HOME_FILE).unwrap();
        std::fs::read(&p).unwrap()
    }

    /// Runs `scope <args>`, with `scope_config` as `SCOPE_CONFIG` when given.
    fn run(&self, args: &[&str], scope_config: Option<&Path>, input: &str) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_scope"));
        cmd.args(args)
            .env("SCOPE_BCA_HOME", self.home())
            .env("SCOPE_TEST_HOME", self.dir.path().join("sandbox"))
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env_remove("SCOPE_CONFIG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(p) = scope_config {
            cmd.env("SCOPE_CONFIG", p);
        }
        let mut child = cmd.spawn().unwrap();
        let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
        child.wait_with_output().unwrap()
    }
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

fn has_key(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|s| s.contains(KEY))
}

#[test]
fn test_config_flag_file_is_saved_and_home_file_is_untouched() {
    // Before the fix, `--config X setup` wrote the key to the home file and
    // left X unchanged: the user's chosen file never got the key.
    let env = Env::new();
    let home_before = env.seed_home();
    let x = env.chosen("chosen.yaml");
    ok(&env.run(
        &[
            "--config",
            x.to_str().unwrap(),
            "setup",
            "--key",
            "etherscan",
        ],
        None,
        &format!("{KEY}\n"),
    ));
    assert!(has_key(&x), "key not saved to the --config file");
    assert_eq!(std::fs::read(env.home_file()).unwrap(), home_before);
}

#[test]
fn test_config_flag_file_is_absent_from_home() {
    // With no home file, a save for `--config X` must not create one.
    let env = Env::new();
    let x = env.chosen("chosen.yaml");
    ok(&env.run(
        &[
            "--config",
            x.to_str().unwrap(),
            "setup",
            "--key",
            "etherscan",
        ],
        None,
        &format!("{KEY}\n"),
    ));
    assert!(has_key(&x));
    assert!(!env.home_file().exists(), "home file was created");
}

#[test]
fn test_scope_config_variable_file_is_saved() {
    // `SCOPE_CONFIG` selects the file for every other command; setup must
    // use it too, or the key lands where the other commands do not look.
    let env = Env::new();
    let home_before = env.seed_home();
    let x = env.chosen("from-env.yaml");
    ok(&env.run(
        &["setup", "--key", "etherscan"],
        Some(&x),
        &format!("{KEY}\n"),
    ));
    assert!(has_key(&x));
    assert_eq!(std::fs::read(env.home_file()).unwrap(), home_before);
}

#[test]
fn test_config_flag_beats_scope_config_variable() {
    // The rule is flag, then variable, then home. If the variable won, a
    // user who passes `--config` would still write to the variable's file.
    let env = Env::new();
    let flag = env.chosen("flag.yaml");
    let var = env.chosen("var.yaml");
    ok(&env.run(
        &[
            "--config",
            flag.to_str().unwrap(),
            "setup",
            "--key",
            "etherscan",
        ],
        Some(&var),
        &format!("{KEY}\n"),
    ));
    assert!(has_key(&flag));
    assert!(!var.exists(), "SCOPE_CONFIG file was written");
}

#[test]
fn test_without_flag_or_variable_the_home_file_is_saved() {
    // The default must stay `<home>/config.yaml`.
    let env = Env::new();
    ok(&env.run(&["setup", "--key", "etherscan"], None, &format!("{KEY}\n")));
    assert!(has_key(&env.home_file()));
}

#[test]
fn test_reset_removes_the_chosen_file_not_the_home_file() {
    // Reset deleted the home file even when the user chose another one:
    // that destroys the wrong file.
    let env = Env::new();
    let home_before = env.seed_home();
    let x = env.chosen("chosen.yaml");
    std::fs::write(&x, HOME_FILE).unwrap();
    ok(&env.run(
        &["--config", x.to_str().unwrap(), "setup", "--reset"],
        None,
        "y\n",
    ));
    assert!(!x.exists(), "chosen file still there");
    assert_eq!(std::fs::read(env.home_file()).unwrap(), home_before);
}

#[test]
fn test_status_names_the_chosen_file() {
    // `setup --status` shows the file in use; showing the home path while
    // reading another file misleads the user.
    let env = Env::new();
    let x = env.chosen("chosen.yaml");
    let out = env.run(
        &["--config", x.to_str().unwrap(), "setup", "--status"],
        None,
        "",
    );
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("chosen.yaml"));
}

#[test]
fn test_changes_start_from_the_chosen_file_not_defaults() {
    // Setup reloads from the file. A key already in X must survive a save
    // of another key; if setup started from defaults it would be lost.
    let env = Env::new();
    let x = env.chosen("chosen.yaml");
    std::fs::write(&x, HOME_FILE).unwrap();
    ok(&env.run(
        &["--config", x.to_str().unwrap(), "setup", "--key", "bscscan"],
        None,
        &format!("{KEY}\n"),
    ));
    let saved = std::fs::read_to_string(&x).unwrap();
    assert!(saved.contains("home-file-marker-0000"), "{saved}");
    assert!(saved.contains(KEY), "{saved}");
}
