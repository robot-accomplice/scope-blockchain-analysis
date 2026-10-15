//! `scope tui` without a terminal exits 2 with a clear message, before the
//! setup prompt can read stdin.

use std::process::{Command, Output, Stdio};

const MESSAGE: &str = "scope tui needs an interactive terminal";

/// Runs `scope tui` in an empty HOME (so no config exists and the setup
/// prompt would show if the check ran too late).
fn run(stdin: Stdio, stdout: Stdio, term: Option<&str>) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_scope"));
    cmd.arg("tui")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .stdin(stdin)
        .stdout(stdout)
        .stderr(Stdio::piped());
    if let Some(t) = term {
        cmd.env("TERM", t);
    }
    cmd.spawn().unwrap().wait_with_output().unwrap()
}

fn check(out: &Output) {
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(MESSAGE), "stderr: {err}");
    let all = format!("{}{}", err, String::from_utf8_lossy(&out.stdout));
    assert!(!all.contains("Run setup now"), "setup prompt shown: {all}");
}

#[test]
fn test_stdin_not_a_terminal_exits_2() {
    check(&run(Stdio::null(), Stdio::piped(), Some("xterm")));
}

#[test]
fn test_stdout_not_a_terminal_exits_2() {
    check(&run(Stdio::piped(), Stdio::piped(), Some("xterm")));
}

#[test]
fn test_term_dumb_exits_2() {
    // Piped streams also fail the check; the point is that TERM=dumb
    // alone is reported the same way.
    check(&run(Stdio::null(), Stdio::piped(), Some("dumb")));
}
