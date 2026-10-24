//! With no config file and no terminal, scope runs the command with the
//! defaults instead of starting the setup wizard: an empty line from a pipe
//! would have answered its question with yes.

use std::io::Write;
use std::process::{Command, Output, Stdio};

/// Runs `scope <args>` in an empty home, writing `input` to its stdin.
fn run(args: &[&str], input: &str) -> (Output, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(args)
        .env("SCOPE_BCA_HOME", home.path().join("scope-home"))
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
    // Nothing was configured on the user's behalf.
    assert!(!home.path().join("scope-home/config.yaml").exists());
}

#[test]
fn test_setup_status_without_a_terminal_has_no_first_run_note() {
    let (out, _home) = run(&["setup", "--status"], "");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(!err.contains("Run 'scope setup' to configure"), "{err}");
}
