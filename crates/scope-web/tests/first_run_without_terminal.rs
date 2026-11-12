//! With no config file and no terminal, scope runs the command with the
//! defaults instead of starting the setup wizard: an empty line from a pipe
//! would have answered its question with yes.

use std::io::Write;
use std::process::{Command, Output, Stdio};

/// The scope home that `scope` uses in `dir`. A test build ignores
/// `SCOPE_BCA_HOME` and reads `SCOPE_TEST_HOME`, so the test sets the
/// second one and names the folder it makes: `<dir>/sandbox/.scope-bca`.
fn scope_home(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("sandbox").join(".scope-bca")
}

/// Runs `scope <args>` in a home with no config file (but with the venue
/// catalog, as a user's install has), writing `input` to its stdin.
fn run(args: &[&str], input: &str) -> (Output, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let catalog = scope_home(home.path()).join("venues").join("catalog");
    std::fs::create_dir_all(&catalog).unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scope-core/venues");
    for entry in std::fs::read_dir(repo).unwrap().flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), catalog.join(entry.file_name())).unwrap();
        }
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(args)
        .env("SCOPE_BCA_HOME", scope_home(home.path()))
        .env("SCOPE_TEST_HOME", home.path().join("sandbox"))
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The process may exit before it reads; a closed pipe is fine.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    (child.wait_with_output().unwrap(), home)
}

#[test]
fn test_piped_first_run_skips_the_wizard_and_runs_the_command() {
    let (out, home) = run(&["venues", "list"], "\n\n\n");
    let err = String::from_utf8_lossy(&out.stderr);
    let all = format!("{err}{}", String::from_utf8_lossy(&out.stdout));
    assert!(out.status.success(), "{all}");
    assert!(!all.contains("Run setup now"), "setup prompt shown: {all}");
    assert!(err.contains("Run 'scope setup' to configure"), "{err}");
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .to_lowercase()
            .contains("binance"),
        "{all}"
    );
    // Nothing was configured on the user's behalf. The folder checked is the
    // one this binary really uses (it holds the catalog), so the check can
    // fail: a binary that wrote its config there would leave the file.
    let used = scope_home(home.path());
    assert!(used.join("venues/catalog").is_dir());
    assert!(!used.join("config.yaml").exists());
}

#[test]
fn test_setup_status_without_a_terminal_has_no_first_run_note() {
    let (out, _home) = run(&["setup", "--status"], "");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(!err.contains("Run 'scope setup' to configure"), "{err}");
}
