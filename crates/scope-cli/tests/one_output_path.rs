//! Command output has one path: [`Output`](scope_cli::cli::output::Output).
//! A TUI run copies what goes through it into the scrollback; text printed
//! with `println!` or written to `io::stdout()` never gets there.
//!
//! The scan covers the non-test code of `src/cli`. A few direct uses are
//! allowed, each for its reason below.

use std::path::{Path, PathBuf};

/// Calls that write to the process streams past `Output`.
const DIRECT: &[&str] = &[
    "println!",
    "print!",
    "eprintln!",
    "eprint!",
    "stdout()",
    "stderr()",
];

/// `(file under src/cli, call, reason)`: the direct uses that stay.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "output.rs",
        "stdout()",
        "Output::stdio binds the data channel to the process stdout",
    ),
    (
        "output.rs",
        "stderr()",
        "Output::stdio binds the diagnostic channel to the process stderr",
    ),
    (
        "screen.rs",
        "stdout()",
        "the screen owner writes terminal control sequences and gives ratatui its backend",
    ),
    (
        "progress.rs",
        "stderr()",
        "asks whether stderr is a terminal a bar can draw on; writes nothing",
    ),
    (
        "tui/mod.rs",
        "stdout()",
        "RealHost::write_raw sends the OSC 52 copy sequence to the terminal; it is not output",
    ),
    (
        "tui/crash.rs",
        "stderr()",
        "the panic hook writes the crash notice; no Output exists inside a panic hook",
    ),
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// True when `call` starts a call at `at` in `line`: the character before
/// it is not part of a name, so `atty_stderr()` is not `stderr()`.
fn starts_call(line: &str, at: usize) -> bool {
    line[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// `file:line: call` for each direct write in the non-test code of `src`
/// that `ALLOWED` does not name for `file`. Comments are skipped. The scan
/// stops at the test module (`#[cfg(test)]` then `mod tests`).
fn direct_writes(file: &str, src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim_start().starts_with("mod "))
        .unwrap_or(lines.len());
    let mut found = Vec::new();
    for (i, line) in lines[..end].iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        for call in DIRECT {
            let hit = code
                .match_indices(call)
                .any(|(at, _)| starts_call(code, at));
            let allowed = ALLOWED.iter().any(|(f, c, _)| *f == file && c == call);
            if hit && !allowed {
                found.push(format!("{file}:{}: {call}", i + 1));
            }
        }
    }
    found
}

#[test]
fn command_code_writes_only_through_output() {
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");
    let mut files = Vec::new();
    rs_files(&cli, &mut files);
    files.sort();
    let mut found = Vec::new();
    for path in files {
        let name = path
            .strip_prefix(&cli)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(&path).unwrap();
        found.extend(direct_writes(&name, &src));
    }
    assert!(
        found.is_empty(),
        "write command output with outln!/errln! (or Output::data_writer), or add an \
         ALLOWED entry with its reason:\n{}",
        found.join("\n")
    );
}

#[test]
fn scanner_flags_direct_writes_and_spares_allowed_comments_and_tests() {
    let src = [
        r#"println!("x");"#,                 // 1 flag
        r#"print!("x");"#,                   // 2 flag
        r#"eprintln!("x");"#,                // 3 flag
        r#"eprint!("x");"#,                  // 4 flag
        "let w = io::stdout();",             // 5 flag
        "let w = std::io::stderr().lock();", // 6 flag
        "let ok = atty_stderr();",           // 7 spare: another name
        "let c = color_enabled_stderr();",   // 8 spare
        r#"outln!(out, "x")?;"#,             // 9 spare
        r#"// not eprintln!: a comment"#,    // 10 spare
        r#"writeln!(writer, "x")?;"#,        // 11 spare
        "#[cfg(test)]",                      // 12 stop
        "mod tests {",                       // 13
        r#"println!("in a test");"#,         // 14 spare
    ]
    .join("\n");
    let lines: Vec<usize> = direct_writes("x.rs", &src)
        .iter()
        .map(|v| v.split(':').nth(1).unwrap().parse().unwrap())
        .collect();
    assert_eq!(lines, vec![1, 2, 3, 4, 5, 6]);
    assert!(direct_writes("output.rs", "let w = io::stdout();").is_empty());
    assert_eq!(
        direct_writes("setup.rs", "let w = io::stdout();"),
        vec!["setup.rs:1: stdout()"],
        "an allowance names its file"
    );
}
