//! Source scanning for the guard tests: which files, and which of their
//! lines are code.

use std::path::{Path, PathBuf};

/// The `src/cli` directory of this crate.
pub fn cli_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli")
}

/// Every `.rs` file under `dir`, sorted, as `(path relative to dir with
/// '/' separators, source)`.
pub fn rs_sources(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let src = std::fs::read_to_string(&path).unwrap();
            (name, src)
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The code lines of `src`, as `(1-based line number, code)`: the text
/// before a `//` comment, up to the test module (a `#[cfg(test)]` line
/// followed by a `mod` line). Another `#[cfg(test)]` item does not stop
/// it. A `//` inside a string literal also ends the code on that line.
pub fn code_lines(src: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = src.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim_start().starts_with("mod "))
        .unwrap_or(lines.len());
    lines[..end]
        .iter()
        .enumerate()
        .map(|(i, line)| (i + 1, line.split("//").next().unwrap_or("")))
        .collect()
}

/// How often `call` starts a call in `code`: the character before it is
/// not part of a name, so `atty_stderr()` does not count as `stderr()`.
pub fn calls(code: &str, call: &str) -> usize {
    code.match_indices(call)
        .filter(|&(at, _)| {
            code[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
        .count()
}

/// One allowed use: `count` calls of `call` in `file`, for `reason`.
pub struct Allowed {
    pub file: &'static str,
    pub call: &'static str,
    pub count: usize,
    pub reason: &'static str,
}

/// Checks each of `sources` (`(name, source)`) for each of `watched`
/// calls. A file may make exactly the calls an [`Allowed`] entry names for
/// it, and no others. Returns one line per mismatch, with the `file:line`
/// of each call found.
pub fn audit(sources: &[(String, String)], watched: &[&str], allowed: &[Allowed]) -> Vec<String> {
    let mut found = Vec::new();
    for (file, src) in sources {
        let code = code_lines(src);
        for call in watched {
            let at: Vec<usize> = code
                .iter()
                .flat_map(|(n, line)| std::iter::repeat_n(*n, calls(line, call)))
                .collect();
            let entry = allowed.iter().find(|a| a.file == file && a.call == *call);
            let (want, why) = entry.map_or((0, String::new()), |a| {
                (a.count, format!(" ({})", a.reason))
            });
            if at.len() != want {
                let lines: Vec<String> = at.iter().map(|n| format!("{file}:{n}")).collect();
                found.push(format!(
                    "{file}: {call} {} times, allowed {want}{why} [{}]",
                    at.len(),
                    lines.join(", ")
                ));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_lines_skip_comments_and_stop_at_the_test_module() {
        let src = [
            "a(); // b()",
            "#[cfg(test)]",
            "static X: () = ();",
            "#[cfg(test)]",
            "mod tests {",
            "c();",
        ]
        .join("\n");
        assert_eq!(
            code_lines(&src),
            vec![(1, "a(); "), (2, "#[cfg(test)]"), (3, "static X: () = ();")]
        );
    }

    #[test]
    fn audit_names_extra_and_missing_calls_by_line() {
        let sources = vec![
            ("a.rs".to_string(), "x();\nx(); // x()\n".to_string()),
            ("b.rs".to_string(), "y();\n".to_string()),
        ];
        let allowed = [Allowed {
            file: "b.rs",
            call: "x()",
            count: 1,
            reason: "test",
        }];
        assert_eq!(
            audit(&sources, &["x()"], &allowed),
            vec![
                "a.rs: x() 2 times, allowed 0 [a.rs:1, a.rs:2]",
                "b.rs: x() 0 times, allowed 1 (test) []"
            ]
        );
    }

    #[test]
    fn calls_count_whole_names_only() {
        assert_eq!(calls("stderr(); x.stderr(); atty_stderr()", "stderr()"), 2);
        assert_eq!(calls("eprint!(); print!()", "print!"), 1);
    }
}
