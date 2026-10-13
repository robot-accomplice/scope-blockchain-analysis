//! Command output has one path: [`Output`](scope_cli::cli::output::Output).
//! A TUI run copies what goes through it into the scrollback; text printed
//! with `println!` or written to `io::stdout()` never gets there. Status
//! lines are notes (`note!`), not data (`outln!`).
//!
//! The scan covers the non-test code of `src/cli`. Each direct use that
//! stays is listed with its file, function, exact count and reason.

mod common;

use common::{Allowed, audit, cli_dir, code_lines, rs_sources};

/// Calls that write to the process streams past `Output`.
const DIRECT: &[&str] = &[
    "println!",
    "print!",
    "eprintln!",
    "eprint!",
    "stdout()",
    "stderr()",
];

const fn allow(
    file: &'static str,
    func: &'static str,
    call: &'static str,
    count: usize,
    reason: &'static str,
) -> Allowed {
    Allowed {
        file,
        func,
        call,
        count,
        reason,
    }
}

const ALLOWED: &[Allowed] = &[
    allow(
        "output.rs",
        "stdio",
        "stdout()",
        1,
        "binds the data channel to the process stdout",
    ),
    allow(
        "output.rs",
        "stdio",
        "stderr()",
        1,
        "binds the diagnostic channel to the process stderr",
    ),
    allow(
        "screen.rs",
        "enter",
        "stdout()",
        2,
        "writes the take sequences; gives ratatui its backend",
    ),
    allow(
        "screen.rs",
        "restore",
        "stdout()",
        1,
        "writes the give-back sequences",
    ),
    allow(
        "progress.rs",
        "atty_stderr",
        "stderr()",
        1,
        "asks whether stderr is a terminal; writes nothing",
    ),
    allow(
        "tui/mod.rs",
        "write_raw",
        "stdout()",
        1,
        "sends the OSC 52 copy sequence; it is not output",
    ),
    allow(
        "tui/crash.rs",
        "install_panic_hook",
        "stderr()",
        1,
        "the crash notice; no Output exists in a panic hook",
    ),
];

#[test]
fn command_code_writes_only_through_output() {
    let found = audit(&rs_sources(&cli_dir()), DIRECT, ALLOWED);
    assert!(
        found.is_empty(),
        "write command output with outln!/errln!/note! (prompts: prompt::ask), or change \
         ALLOWED with a reason:\n{}",
        found.join("\n")
    );
}

#[test]
fn scanner_flags_direct_writes_and_spares_comments_and_tests() {
    let src = [
        "fn f() {",
        r#"    println!("x");"#,
        r#"    print!("x");"#,
        r#"    eprintln!("x");"#,
        r#"    eprint!("x");"#,
        "    let w = io::stdout();",
        "    let w = std::io::stderr().lock();",
        "    let ok = atty_stderr();",
        r#"    outln!(out, "x // println!")?;"#,
        "    // not eprintln!: a comment",
        "}",
        "#[cfg(test)]",
        "mod tests {",
        r#"println!("in a test");"#,
    ]
    .join("\n");
    let found = audit(&[("x.rs".into(), src)], DIRECT, &[]);
    assert_eq!(
        found,
        vec![
            "x.rs fn f: println! 2 times, allowed 0 [x.rs:2, x.rs:9]",
            "x.rs fn f: print! 1 times, allowed 0 [x.rs:3]",
            "x.rs fn f: eprintln! 1 times, allowed 0 [x.rs:4]",
            "x.rs fn f: eprint! 1 times, allowed 0 [x.rs:5]",
            "x.rs fn f: stdout() 1 times, allowed 0 [x.rs:6]",
            "x.rs fn f: stderr() 1 times, allowed 0 [x.rs:7]",
        ],
        "a call named inside a string literal counts too: {found:?}"
    );
}

/// Words that start a status line. A status line is a note: it goes to
/// the diagnostic channel with `note!`, so data stays clean.
const STATUS_WORDS: &[&str] = &[
    "Analyzing",
    "Using",
    "Note",
    "Fetching",
    "Searching",
    "Found",
    "Selected",
    "Saved",
    "Running",
    "Monitoring",
    "Press",
];

/// `file:line` for each `outln!` whose text starts with a status word.
/// The literal may follow the output on the next lines, and may start
/// with blanks or `\n`.
fn status_on_data(file: &str, src: &str) -> Vec<String> {
    let code = code_lines(src);
    let text: Vec<&str> = code.iter().map(|(_, l)| l.as_str()).collect();
    let joined = text.join("\n");
    let mut found = Vec::new();
    for (at, _) in joined.match_indices("outln!(") {
        let args = &joined[at + "outln!(".len()..];
        let Some(comma) = args.find(',') else {
            continue;
        };
        let rest = args[comma + 1..].trim_start();
        let Some(literal) = rest.strip_prefix('"') else {
            continue;
        };
        let words = skip_blanks(literal);
        if STATUS_WORDS.iter().any(|w| words.starts_with(w)) {
            let line = joined[..at].matches('\n').count() + 1;
            found.push(format!("{file}:{line}"));
        }
    }
    found
}

/// `text` after its leading blanks and `\n` escapes.
fn skip_blanks(mut text: &str) -> &str {
    loop {
        let rest = text.trim_start_matches(' ');
        match rest.strip_prefix("\\n") {
            Some(after) => text = after,
            None => return rest,
        }
    }
}

#[test]
fn status_lines_are_notes_not_data() {
    let mut found = Vec::new();
    for (file, src) in rs_sources(&cli_dir()) {
        found.extend(status_on_data(&file, &src));
    }
    assert!(
        found.is_empty(),
        "a status line is a note: use note!(out, …), not outln!:\n{}",
        found.join("\n")
    );
}

#[test]
fn status_scanner_flags_status_words_on_the_data_channel() {
    let src = [
        r#"outln!(out, "Analyzing {}", x)?;"#, // 1 flag
        r#"outln!(out, "\n  Note: {}", x)?;"#, // 2 flag
        "outln!(",                             // 3 flag (spans lines)
        "    out,",
        r#"    "Monitoring {} on {}","#,
        "    a, b)?;",
        r#"outln!(out, "{}", json)?;"#,               // 7 spare
        r#"outln!(out, "Report saved to: {}", p)?;"#, // 8 spare
        r#"note!(out, "Fetching {}", x);"#,           // 9 spare
        r#"outln!(out, "Nothing found")?;"#,          // 10 spare
    ]
    .join("\n");
    assert_eq!(
        status_on_data("x.rs", &src),
        vec!["x.rs:1", "x.rs:2", "x.rs:3"]
    );
}
