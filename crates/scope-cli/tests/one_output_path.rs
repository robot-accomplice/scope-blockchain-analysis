//! Command output has one path: [`Output`](scope_cli::cli::output::Output).
//! A TUI run copies what goes through it into the scrollback; text printed
//! with `println!` or written to `io::stdout()` never gets there. Status
//! lines are notes (`note!`), not data (`outln!`).
//!
//! The scan covers the non-test code of `src/cli`. Each direct use that
//! stays is listed with its file, function, exact count and reason.

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
            "x.rs fn f: println! 1 times, allowed 0 [x.rs:2]",
            "x.rs fn f: print! 1 times, allowed 0 [x.rs:3]",
            "x.rs fn f: eprintln! 1 times, allowed 0 [x.rs:4]",
            "x.rs fn f: eprint! 1 times, allowed 0 [x.rs:5]",
            "x.rs fn f: stdout() 1 times, allowed 0 [x.rs:6]",
            "x.rs fn f: stderr() 1 times, allowed 0 [x.rs:7]",
        ],
        "a call named inside a string literal is not a call: {found:?}"
    );
}

/// Words that start a status line. A status line is a note: it goes to
/// the diagnostic channel with `note!`, so data stays clean.
const STATUS_WORDS: &[&str] = &[
    "Analyzing",
    "Assessing",
    "Checking",
    "Completed",
    "Exporting",
    "Fetching",
    "Flagging",
    "Found",
    "Generating",
    "Loading",
    "Monitoring",
    "Note",
    "Press",
    "Running",
    "Saved",
    "Saving",
    "Searching",
    "Selected",
    "Starting",
    "Tracing",
    "Using",
    "Warning",
    "⚠",
];

/// Phrases that mark a status line wherever they are in the text.
const STATUS_ANYWHERE: &[&str] = &["saved to", "exported to"];

/// The first argument after the output in the `outln!(` call whose `(` is
/// just before `from` (char index into `masked`): its start, when it is a
/// string literal. The scan follows brackets on the masked text, so a
/// comma inside the output expression or inside a literal is not the one.
fn literal_argument(masked: &[char], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &c) in masked.iter().enumerate().skip(from) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return None,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let start = i
                    + 1
                    + masked[i + 1..]
                        .iter()
                        .take_while(|c| c.is_whitespace())
                        .count();
                return (masked.get(start) == Some(&'"')).then_some(start + 1);
            }
            _ => {}
        }
    }
    None
}

/// `file:line` for each `outln!` whose text is a status line: it starts
/// with a status word (after blanks and `\n` escapes) or says "saved to".
fn status_on_data(file: &str, src: &str) -> Vec<String> {
    let lines = common::lex(src);
    let join = |f: fn(&common::Line) -> &str| -> Vec<char> {
        lines
            .iter()
            .map(f)
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .collect()
    };
    let code = join(|l| &l.code);
    let masked = join(|l| &l.masked);
    let masked_text: String = masked.iter().collect();
    let mut found = Vec::new();
    for (byte_at, _) in masked_text.match_indices("outln!(") {
        let call = masked_text[..byte_at].chars().count();
        if call > 0 && (masked[call - 1].is_alphanumeric() || masked[call - 1] == '_') {
            continue;
        }
        let Some(start) = literal_argument(&masked, call + "outln!(".len()) else {
            continue;
        };
        let end = start + masked[start..].iter().position(|&c| c == '"').unwrap_or(0);
        let literal: String = code[start..end].iter().collect();
        let words = skip_blanks(&literal);
        if STATUS_WORDS.iter().any(|w| words.starts_with(w))
            || STATUS_ANYWHERE
                .iter()
                .any(|p| literal.to_lowercase().contains(p))
        {
            let line = masked[..call].iter().filter(|&&c| c == '\n').count() + 1;
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
        "a status line is a note: use note!(out, …) or out.note_saved, not outln!:\n{}",
        found.join("\n")
    );
}

#[test]
fn status_scanner_flags_status_lines_on_the_data_channel() {
    let src = [
        r#"outln!(out, "Analyzing {}", x)?;"#, // 1 flag
        r#"outln!(out, "\n  Note: {}", x)?;"#, // 2 flag
        "outln!(",                             // 3 flag (spans lines)
        "    out,",
        r#"    "Monitoring {} on {}","#,
        "    a, b)?;",
        r#"outln!(out, "{}", json)?;"#,                 // 7 spare
        r#"outln!(out, "\nReport saved to: {}", p)?;"#, // 8 flag: saved to
        r#"note!(out, "Fetching {}", x);"#,             // 9 spare
        r#"outln!(out, "Nothing found")?;"#,            // 10 spare
        r#"outln!(out)?; let s = "a, \"Using\"";"#,     // 11 spare: no literal argument
        r#"outln!(pick(a, b), "Starting {}", x)?;"#,    // 12 flag: comma in the output
        r#"outln!(out, "{}", "Using")?;"#,              // 13 spare: the text is "{}"
        r#"let s = "outln!(out, \"Saved\")";"#,         // 14 spare: inside a string
        r#"youtln!(out, "Saved")?;"#,                   // 15 spare: another name
    ]
    .join("\n");
    assert_eq!(
        status_on_data("x.rs", &src),
        vec!["x.rs:1", "x.rs:2", "x.rs:3", "x.rs:8", "x.rs:12"]
    );
}

/// The files that may write to the diagnostic channel past `note!`, and
/// why. Everything else there is a note.
const REPORTERS: &[(&str, &str, &str)] = &[
    (
        "errors.rs",
        "errln!(",
        "the error report of a failed command",
    ),
    (
        "tui/exec.rs",
        "errln!(",
        "the TUI's answer to a mistyped session command",
    ),
    ("prompt.rs", "err!(", "a prompt's question"),
];

/// `file:line: call` for each `errln!(`/`err!(` outside [`REPORTERS`].
fn unreported(file: &str, src: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in common::masked_lines(src) {
        for call in ["errln!(", "err!("] {
            let allowed = REPORTERS.iter().any(|(f, c, _)| *f == file && *c == call);
            if !allowed && common::calls(&line, call) > 0 {
                found.push(format!("{file}:{n}: {call}"));
            }
        }
    }
    found
}

#[test]
fn only_error_reports_and_prompts_bypass_note() {
    let mut found = Vec::new();
    for (file, src) in rs_sources(&cli_dir()) {
        found.extend(unreported(&file, &src));
    }
    assert!(
        found.is_empty(),
        "status, notices and warnings use note! (see cli/output.rs):\n{}",
        found.join("\n")
    );
    for (file, call, why) in REPORTERS {
        let src = std::fs::read_to_string(cli_dir().join(file)).unwrap();
        let used = common::masked_lines(&src)
            .iter()
            .any(|(_, l)| common::calls(l, call) > 0);
        assert!(used, "stale entry: {file} has no {call} ({why})");
    }
}

#[test]
fn reporter_scan_flags_other_files_only() {
    let src = r#"errln!(out, "x")?; crate::err!(out, "q")?; note!(out, "ok");"#;
    assert_eq!(
        unreported("market.rs", src),
        vec!["market.rs:1: errln!(", "market.rs:1: err!("]
    );
    assert!(unreported("errors.rs", r#"errln!(out, "x")?;"#).is_empty());
}
