//! Command output has one path: [`Output`](scope_cli::cli::output::Output).
//! A TUI run copies what goes through it into the scrollback; text printed
//! with `println!` or written to `io::stdout()` never gets there.
//!
//! The scan covers the non-test code of `src/cli`. Each direct use that
//! stays is listed with its file, its exact count and its reason.

mod common;

use common::{Allowed, audit, cli_dir, rs_sources};

/// Calls that write to the process streams past `Output`.
const DIRECT: &[&str] = &[
    "println!",
    "print!",
    "eprintln!",
    "eprint!",
    "stdout()",
    "stderr()",
];

const ALLOWED: &[Allowed] = &[
    Allowed {
        file: "output.rs",
        call: "stdout()",
        count: 1,
        reason: "Output::stdio binds the data channel to the process stdout",
    },
    Allowed {
        file: "output.rs",
        call: "stderr()",
        count: 1,
        reason: "Output::stdio binds the diagnostic channel to the process stderr",
    },
    Allowed {
        file: "screen.rs",
        call: "stdout()",
        count: 3,
        reason: "enter and restore write terminal control sequences; enter gives ratatui its backend",
    },
    Allowed {
        file: "progress.rs",
        call: "stderr()",
        count: 1,
        reason: "asks whether stderr is a terminal a bar can draw on; writes nothing",
    },
    Allowed {
        file: "tui/mod.rs",
        call: "stdout()",
        count: 1,
        reason: "RealHost::write_raw sends the OSC 52 copy sequence; it is not output",
    },
    Allowed {
        file: "tui/crash.rs",
        call: "stderr()",
        count: 1,
        reason: "the panic hook writes the crash notice; no Output exists inside a panic hook",
    },
];

#[test]
fn command_code_writes_only_through_output() {
    let found = audit(&rs_sources(&cli_dir()), DIRECT, ALLOWED);
    assert!(
        found.is_empty(),
        "write command output with outln!/errln! (prompts: prompt::ask), or change \
         ALLOWED with a reason:\n{}",
        found.join("\n")
    );
}

#[test]
fn scanner_flags_direct_writes_and_spares_comments_and_tests() {
    let src = [
        r#"println!("x");"#,                 // 1 flag
        r#"print!("x");"#,                   // 2 flag
        r#"eprintln!("x");"#,                // 3 flag
        r#"eprint!("x");"#,                  // 4 flag
        "let w = io::stdout();",             // 5 flag
        "let w = std::io::stderr().lock();", // 6 flag
        "let ok = atty_stderr();",           // 7 spare: another name
        r#"outln!(out, "x")?;"#,             // 8 spare
        r#"// not eprintln!: a comment"#,    // 9 spare
        "#[cfg(test)]",                      // 10 stop
        "mod tests {",                       // 11
        r#"println!("in a test");"#,         // 12 spare
    ]
    .join("\n");
    let found = audit(&[("x.rs".into(), src)], DIRECT, &[]);
    assert_eq!(
        found,
        vec![
            "x.rs: println! 1 times, allowed 0 [x.rs:1]",
            "x.rs: print! 1 times, allowed 0 [x.rs:2]",
            "x.rs: eprintln! 1 times, allowed 0 [x.rs:3]",
            "x.rs: eprint! 1 times, allowed 0 [x.rs:4]",
            "x.rs: stdout() 1 times, allowed 0 [x.rs:5]",
            "x.rs: stderr() 1 times, allowed 0 [x.rs:6]",
        ]
    );
}
