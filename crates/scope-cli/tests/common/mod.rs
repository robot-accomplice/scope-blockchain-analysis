//! Source scanning for the guard tests: which files, which of their lines
//! are code, and which function each line is in.

use std::path::{Path, PathBuf};

/// The `src/cli` directory of this crate.
pub fn cli_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli")
}

/// The `src` directory of the `scope` binary's crate (scope-web).
pub fn web_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scope-web/src")
}

/// Every product source the guards check: `src/cli` of this crate (names
/// relative to it) and the `scope` binary's crate (names under `web/`).
/// The TUI test harness crate is not a product: it is a developer tool
/// that does not link the CLI and prints its own reports.
pub fn product_sources() -> Vec<(String, String)> {
    let mut all = rs_sources(&cli_dir());
    all.extend(
        rs_sources(&web_dir())
            .into_iter()
            .map(|(name, src)| (format!("web/{name}"), src)),
    );
    all
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

/// Where the lexer is.
#[derive(Clone, Copy, PartialEq)]
enum Lex {
    Code,
    /// In `"…"`.
    Str,
    /// In `r#…"…"#…` with this many `#`.
    RawStr(usize),
    /// In `/* … */`, nested this deep.
    Block(usize),
    /// In `// …`, to the end of the line.
    Line,
}

/// True when `line` opens a test module: `mod`, `pub mod` or
/// `pub(crate) mod` after a `#[cfg(test)]` line.
fn opens_module(line: &str) -> bool {
    let line = line.trim_start();
    ["mod ", "pub mod ", "pub(crate) mod "]
        .iter()
        .any(|p| line.starts_with(p))
}

/// One line of code, twice: `code` keeps string and character literals;
/// `masked` has their contents blanked (one space per character, the
/// quotes kept), so a name inside a literal is not code.
pub struct Line {
    pub n: usize,
    pub code: String,
    pub masked: String,
}

/// The code lines of `src`, up to the test module (a `#[cfg(test)]` line
/// followed by a `mod` line). Comments (`//` and `/* */`) are removed. A
/// `//` inside a string literal is not a comment.
pub fn lex(src: &str) -> Vec<Line> {
    let lines: Vec<&str> = src.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && opens_module(w[1]))
        .unwrap_or(lines.len());
    let chars: Vec<char> = lines[..end].join("\n").chars().collect();
    let mut out = vec![(String::new(), String::new())];
    let mut state = Lex::Code;
    let mut i = 0;
    let at = |j: usize| chars.get(j).copied();
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            out.push((String::new(), String::new()));
            if state == Lex::Line {
                state = Lex::Code;
            }
            i += 1;
            continue;
        }
        let (code, masked) = out.last_mut().unwrap();
        let mut both = |text: &[char], hide: bool| {
            code.extend(text);
            masked.extend(text.iter().map(|&t| if hide { ' ' } else { t }));
        };
        let mut take = 1;
        match state {
            Lex::Line => {}
            Lex::Block(depth) => match (c, at(i + 1)) {
                ('/', Some('*')) => (state, take) = (Lex::Block(depth + 1), 2),
                ('*', Some('/')) if depth == 1 => (state, take) = (Lex::Code, 2),
                ('*', Some('/')) => (state, take) = (Lex::Block(depth - 1), 2),
                _ => {}
            },
            Lex::Str => match (c, at(i + 1)) {
                ('\\', Some(n)) if n != '\n' => {
                    both(&[c, n], true);
                    take = 2;
                }
                ('"', _) => {
                    both(&[c], false);
                    state = Lex::Code;
                }
                _ => both(&[c], true),
            },
            Lex::RawStr(hashes) => {
                if c == '"' && (1..=hashes).all(|k| at(i + k) == Some('#')) {
                    both(&chars[i..=i + hashes], false);
                    (state, take) = (Lex::Code, 1 + hashes);
                } else {
                    both(&[c], true);
                }
            }
            Lex::Code => match (c, at(i + 1)) {
                ('/', Some('/')) => state = Lex::Line,
                ('/', Some('*')) => (state, take) = (Lex::Block(1), 2),
                ('"', _) => {
                    both(&[c], false);
                    state = Lex::Str;
                }
                ('r', _) if raw_start(&chars, i).is_some() => {
                    let hashes = raw_start(&chars, i).unwrap();
                    take = hashes + 2;
                    both(&chars[i..i + take], false);
                    state = Lex::RawStr(hashes);
                }
                ('\'', _) => {
                    take = char_literal_len(&chars, i);
                    both(&chars[i..=i], false);
                    if take > 1 {
                        both(&chars[i + 1..i + take - 1], true);
                        both(&chars[i + take - 1..i + take], false);
                    }
                }
                _ => both(&[c], false),
            },
        }
        i += take;
    }
    out.into_iter()
        .enumerate()
        .map(|(n, (code, masked))| Line {
            n: n + 1,
            code,
            masked,
        })
        .collect()
}

/// The code lines of `src` as `(line number, code)`, literals kept.
pub fn code_lines(src: &str) -> Vec<(usize, String)> {
    lex(src).into_iter().map(|l| (l.n, l.code)).collect()
}

/// The code lines of `src` as `(line number, code)`, literal contents
/// blanked.
pub fn masked_lines(src: &str) -> Vec<(usize, String)> {
    lex(src).into_iter().map(|l| (l.n, l.masked)).collect()
}

/// The number of `#` when a raw string (`r"`, `r#"`, `br"` …) starts at
/// the `r` at `i`.
fn raw_start(chars: &[char], i: usize) -> Option<usize> {
    let before = i.checked_sub(1).map(|j| chars[j]);
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let prefix_ok = match before {
        Some('b') => i < 2 || !ident(chars[i - 2]),
        Some(c) => !ident(c),
        None => true,
    };
    if !prefix_ok {
        return None;
    }
    let hashes = chars[i + 1..].iter().take_while(|&&c| c == '#').count();
    (chars.get(i + 1 + hashes) == Some(&'"')).then_some(hashes)
}

/// The length of the character literal at `i` (`'x'`, `'\n'`, `'\''`,
/// `'\u{1b}'`), or 1 for a lifetime's quote.
fn char_literal_len(chars: &[char], i: usize) -> usize {
    match chars.get(i + 1) {
        // An escape: the character after the backslash is part of it, so
        // the closing quote comes after that.
        Some('\\') => chars
            .get(i + 3..)
            .and_then(|rest| rest.iter().position(|&c| c == '\''))
            .map_or(1, |p| p + 4),
        Some(_) if chars.get(i + 2) == Some(&'\'') => 3,
        _ => 1,
    }
}

/// The function each code line is in: the nearest open `fn` header above
/// it (or on it) that is indented less than the line. A `}` line closes
/// the headers at its indent or deeper. `None` at module level.
pub fn enclosing_fns(code: &[(usize, String)]) -> Vec<Option<String>> {
    let mut headers: Vec<(usize, String)> = Vec::new();
    code.iter()
        .map(|(_, line)| {
            let indent = line.len() - line.trim_start().len();
            let header = fn_name(line).map(|name| (indent, name));
            let found = header
                .clone()
                .or_else(|| headers.iter().rev().find(|(i, _)| *i < indent).cloned())
                .filter(|_| !line.trim().is_empty());
            if let Some(h) = header {
                headers.retain(|(i, _)| *i < h.0);
                headers.push(h);
            } else if line.trim_start().starts_with('}') {
                // A closing brace ends the functions opened at its indent
                // or deeper.
                headers.retain(|(i, _)| *i < indent);
            }
            found.map(|(_, name)| name)
        })
        .collect()
}

/// The name in a `fn` header on `line`, if there is one.
fn fn_name(line: &str) -> Option<String> {
    let at = line.find("fn ")?;
    if at > 0
        && line[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    let name: String = line[at + 3..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
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

/// One allowed use: `count` calls of `call` in function `func` of `file`,
/// for `reason`.
pub struct Allowed {
    pub file: &'static str,
    pub func: &'static str,
    pub call: &'static str,
    pub count: usize,
    pub reason: &'static str,
}

/// Checks each of `sources` (`(name, source)`) for each of `watched`
/// calls. A function may make exactly the calls an [`Allowed`] entry names
/// for it, and no others. An entry whose file or function is gone is an
/// error too. Returns one line per mismatch, with the `file:line` of each
/// call found.
pub fn audit(sources: &[(String, String)], watched: &[&str], allowed: &[Allowed]) -> Vec<String> {
    let mut found = Vec::new();
    for (file, src) in sources {
        let code = masked_lines(src);
        let fns = enclosing_fns(&code);
        for call in watched {
            let mut by_fn: Vec<(Option<String>, Vec<usize>)> = Vec::new();
            for ((n, line), func) in code.iter().zip(&fns) {
                for _ in 0..calls(line, call) {
                    match by_fn.iter_mut().find(|(f, _)| f == func) {
                        Some((_, lines)) => lines.push(*n),
                        None => by_fn.push((func.clone(), vec![*n])),
                    }
                }
            }
            for a in allowed.iter().filter(|a| a.file == file && a.call == *call) {
                if !by_fn.iter().any(|(f, _)| f.as_deref() == Some(a.func)) {
                    by_fn.push((Some(a.func.to_string()), Vec::new()));
                }
            }
            for (func, lines) in by_fn {
                let entry = allowed
                    .iter()
                    .find(|a| a.file == file && a.call == *call && Some(a.func) == func.as_deref());
                let (want, why) = entry.map_or((0, String::new()), |a| {
                    (a.count, format!(" ({})", a.reason))
                });
                if lines.len() != want {
                    let at: Vec<String> = lines.iter().map(|n| format!("{file}:{n}")).collect();
                    found.push(format!(
                        "{file} fn {}: {call} {} times, allowed {want}{why} [{}]",
                        func.as_deref().unwrap_or("<module>"),
                        lines.len(),
                        at.join(", ")
                    ));
                }
            }
        }
    }
    for a in allowed {
        let Some((_, src)) = sources.iter().find(|(f, _)| f == a.file) else {
            found.push(format!("stale entry: {} does not exist", a.file));
            continue;
        };
        let code = masked_lines(src);
        if !code
            .iter()
            .any(|(_, l)| fn_name(l).as_deref() == Some(a.func))
        {
            found.push(format!("stale entry: {} has no fn {}", a.file, a.func));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<String> {
        code_lines(src).into_iter().map(|(_, l)| l).collect()
    }

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
            texts(&src),
            vec!["a(); ", "#[cfg(test)]", "static X: () = ();"]
        );
    }

    #[test]
    fn code_lines_keep_strings_and_drop_only_real_comments() {
        let src = [
            r#"let u = "https://x"; f(); // g()"#,
            r##"let r = r#"a // "b" "#; h();"##,
            r#"let s = "esc \" // q"; i();"#,
            "let c = '\"'; j(); /* k() */ l();",
            "let lt: &'a str = m(); // n()",
            "/* open",
            "o() */ p();",
            r#"let b = br"//"; q();"#,
            r#"let m = "multi"#,
            r#"line // kept"; s();"#,
        ]
        .join("\n");
        assert_eq!(
            texts(&src),
            vec![
                r#"let u = "https://x"; f(); "#,
                r##"let r = r#"a // "b" "#; h();"##,
                r#"let s = "esc \" // q"; i();"#,
                "let c = '\"'; j();  l();",
                "let lt: &'a str = m(); ",
                "",
                " p();",
                r#"let b = br"//"; q();"#,
                r#"let m = "multi"#,
                r#"line // kept"; s();"#,
            ]
        );
    }

    #[test]
    fn masked_lines_blank_literal_contents() {
        let src = [
            r#"let s = "fn fake() { println!() }"; real();"#,
            r"let q = '\''; let b = '\\'; let e = '\u{1b}'; x();",
            "let lt: &'a str = y();",
        ]
        .join("\n");
        let masked: Vec<String> = masked_lines(&src).into_iter().map(|(_, l)| l).collect();
        assert_eq!(calls(&masked[0], "println!"), 0, "{:?}", masked[0]);
        assert!(masked[0].ends_with("\"; real();"), "{:?}", masked[0]);
        assert_eq!(
            fn_name(&masked[0]),
            None,
            "a fn inside a string is no header"
        );
        assert!(masked[1].ends_with("'; x();"), "{:?}", masked[1]);
        assert_eq!(masked[1].matches('\'').count(), 6, "{:?}", masked[1]);
        assert_eq!(masked[2], "let lt: &'a str = y();");
    }

    #[test]
    fn a_pub_crate_test_module_ends_the_code() {
        let src = "a();\n#[cfg(test)]\npub(crate) mod tests {\nb();";
        assert_eq!(code_lines(src).len(), 1);
    }

    #[test]
    fn enclosing_fns_follow_indentation() {
        let src = [
            "fn a() {",
            "    x();",
            "}",
            "impl T {",
            "    pub(crate) fn b(&self) {",
            "        let f = || y();",
            "    }",
            "}",
            "static S: () = z();",
            "fn c(",
            "    a: u8,",
            ") -> u8 {",
            "    w();",
            "}",
        ]
        .join("\n");
        let want: Vec<Option<String>> = [
            Some("a"),
            Some("a"),
            None,
            None,
            Some("b"),
            Some("b"),
            None,
            None,
            None,
            Some("c"),
            Some("c"),
            None,
            Some("c"),
            None,
        ]
        .iter()
        .map(|f| f.map(String::from))
        .collect();
        assert_eq!(enclosing_fns(&code_lines(&src)), want);
    }

    #[test]
    fn audit_counts_per_function_and_flags_stale_entries() {
        let sources = vec![
            (
                "a.rs".to_string(),
                "fn one() {\n    x();\n    x(); // x()\n}\nfn two() {\n    x();\n}\n".to_string(),
            ),
            ("b.rs".to_string(), "fn three() {}\n".to_string()),
        ];
        let allowed = [
            Allowed {
                file: "a.rs",
                func: "two",
                call: "x()",
                count: 1,
                reason: "ok",
            },
            Allowed {
                file: "b.rs",
                func: "gone",
                call: "x()",
                count: 1,
                reason: "old",
            },
            Allowed {
                file: "c.rs",
                func: "f",
                call: "x()",
                count: 1,
                reason: "old",
            },
        ];
        assert_eq!(
            audit(&sources, &["x()"], &allowed),
            vec![
                "a.rs fn one: x() 2 times, allowed 0 [a.rs:2, a.rs:3]",
                "b.rs fn gone: x() 0 times, allowed 1 (old) []",
                "stale entry: b.rs has no fn gone",
                "stale entry: c.rs does not exist",
            ]
        );
    }

    #[test]
    fn calls_count_whole_names_only() {
        assert_eq!(calls("stderr(); x.stderr(); atty_stderr()", "stderr()"), 2);
        assert_eq!(calls("eprint!(); print!()", "print!"), 1);
    }
}
