//! Every HTTP request must use the shared transport (TUI test harness
//! design, section 2.2). This test fails when production code builds its
//! own client.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "reqwest::Client",
    "reqwest::get(",
    "NativeHttpClient::new",
    "GholaHttpClient::new",
];
/// Start of a grouped `reqwest` import, which can bring `Client` in unnamed.
const GROUPED_IMPORT: &str = "use reqwest::{";
const CFG_TEST: &str = "#[cfg(test)]";

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            let name = p.file_name().unwrap().to_string_lossy();
            if name != "target" && name != "tests" {
                rs_files(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Production lines of a file, as `(1-based line number, line)`.
///
/// Skips each `#[cfg(test)]` item. A `#[cfg(test)] mod` skips to the end of
/// the file. Any other `#[cfg(test)]` item is skipped through its closing
/// brace, or through its `;` when it has no body.
fn production_lines(src: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() != CFG_TEST {
            out.push((i + 1, lines[i]));
            i += 1;
            continue;
        }
        let item = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("#["))
            .unwrap_or("");
        if item.starts_with("mod ") {
            break;
        }
        i = skip_item(&lines, i + 1);
    }
    out
}

/// The code part of a line, before any `//`.
fn code_of(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

/// True when the code part of a line builds a client.
fn has_forbidden(line: &str) -> bool {
    let code = code_of(line);
    FORBIDDEN.iter().any(|pat| code.contains(pat))
}

/// Production lines that build a client or import `reqwest::Client` in a
/// group. A grouped import is reported at its first line, and it is read
/// through its closing `}`, so a multi-line group is covered.
fn forbidden_lines<'a>(lines: &[(usize, &'a str)]) -> Vec<(usize, &'a str)> {
    let mut hits = Vec::new();
    for (i, &(n, line)) in lines.iter().enumerate() {
        if has_forbidden(line) {
            hits.push((n, line));
            continue;
        }
        let code = code_of(line);
        let Some(start) = code.find(GROUPED_IMPORT) else {
            continue;
        };
        let mut group = code[start..].to_string();
        for &(_, next) in &lines[i + 1..] {
            if group.contains('}') {
                break;
            }
            group.push_str(code_of(next));
        }
        if group.contains("Client") {
            hits.push((n, line));
        }
    }
    hits
}

/// Index of the first line after the item that starts at or after `start`.
fn skip_item(lines: &[&str], start: usize) -> usize {
    let mut depth = 0usize;
    let mut opened = false;
    for (j, line) in lines.iter().enumerate().skip(start) {
        let code = line.split("//").next().unwrap_or("");
        if line.trim().starts_with("#[") {
            continue;
        }
        for c in code.chars() {
            match c {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if opened && depth == 0 {
            return j + 1;
        }
        if !opened && code.trim_end().ends_with(';') {
            return j + 1;
        }
    }
    lines.len()
}

#[test]
fn test_no_http_client_is_built_outside_the_http_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates");
    let http_dir = root.join("scope-core/src/http");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let mut hits = Vec::new();
    for f in files.iter().filter(|f| !f.starts_with(&http_dir)) {
        let src = std::fs::read_to_string(f).unwrap();
        for (n, line) in forbidden_lines(&production_lines(&src)) {
            hits.push(format!("{}:{}: {}", f.display(), n, line.trim()));
        }
    }
    assert!(
        hits.is_empty(),
        "HTTP client built outside scope::http:\n{}",
        hits.join("\n")
    );
}

fn text(src: &str) -> Vec<&str> {
    production_lines(src).into_iter().map(|(_, l)| l).collect()
}

#[test]
fn test_classifier_skips_a_cfg_test_module_tail() {
    let src = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { reqwest::Client::new(); }\n}\n";
    assert_eq!(text(src), vec!["fn a() {}"]);
}

#[test]
fn test_classifier_skips_a_mid_file_cfg_test_fn_but_scans_after_it() {
    let src = "fn a() {}\n#[cfg(test)]\nfn helper() {\n    if x {\n        y();\n    }\n}\nfn prod() { z(); }\n";
    let kept = production_lines(src);
    assert_eq!(
        kept,
        vec![(1, "fn a() {}"), (8, "fn prod() { z(); }")],
        "line numbers stay tied to the source"
    );
}

#[test]
fn test_classifier_skips_a_bodyless_cfg_test_item() {
    let src = "#[cfg(test)]\nuse foo::Bar;\nfn prod() {}\n";
    assert_eq!(text(src), vec!["fn prod() {}"]);
}

#[test]
fn test_classifier_keeps_cfg_any_test_support_items() {
    let src = "#[cfg(any(test, feature = \"test-support\"))]\nfn h() {}\n";
    assert_eq!(text(src).len(), 2);
}

#[test]
fn test_forbidden_pattern_in_a_comment_is_ignored() {
    assert!(!has_forbidden("let x = 1; // NativeHttpClient::new"));
    assert!(has_forbidden("let c = NativeHttpClient::new(); // ok"));
}

#[test]
fn test_new_forbidden_patterns_are_caught() {
    assert!(has_forbidden("let r = reqwest::get(url).await?;"));
    assert!(has_forbidden("let g = GholaHttpClient::new(true, 1);"));
}

fn grouped_hits(src: &str) -> Vec<usize> {
    forbidden_lines(&production_lines(src))
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

#[test]
fn test_grouped_reqwest_import_of_client_is_caught() {
    assert_eq!(grouped_hits("use reqwest::{Client, Url};\n"), vec![1]);
    let multi = "fn a() {}\nuse reqwest::{\n    header,\n    Client,\n};\n";
    assert_eq!(grouped_hits(multi), vec![2], "multi-line group");
}

#[test]
fn test_grouped_reqwest_import_without_client_is_allowed() {
    assert!(grouped_hits("use reqwest::{header, Url};\n").is_empty());
    assert!(grouped_hits("use reqwest::{\n    Url,\n};\nfn Client() {}\n").is_empty());
}
