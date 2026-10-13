//! Secondary text must stay readable on dark and light themes. Gray, white,
//! black and bright black vanish on some themes, so text uses the default
//! foreground plus the DIM modifier, or a named ANSI color. Borders may keep
//! DarkGray.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "DarkGray",
    "Color::Gray",
    ".gray()",
    ".dark_gray()",
    "Color::Indexed(7)",
    "Color::Indexed(8)",
    "Color::White",
    ".white()",
    "Color::Black",
    ".black()",
];
/// A line is exempt only when it is a border style call or the palette `border:` field.
const BORDER_MARKERS: &[&str] = &["border_style", "Borders::", ".border", "border:"];

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

fn scanned_files() -> Vec<PathBuf> {
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");
    let mut files = Vec::new();
    rs_files(&cli.join("tui"), &mut files);
    rs_files(&cli.join("monitor"), &mut files);
    files.sort();
    files
}

/// `file:line: text` for each non-border line that uses a forbidden color.
fn violations(file: &str, src: &str) -> Vec<String> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| FORBIDDEN.iter().any(|t| l.contains(t)))
        .filter(|(_, l)| !BORDER_MARKERS.iter().any(|m| l.contains(m)))
        .map(|(i, l)| format!("{file}:{}: {}", i + 1, l.trim()))
        .collect()
}

#[test]
fn text_never_uses_gray_white_black_or_bright_black() {
    let mut found = Vec::new();
    for path in scanned_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        found.extend(violations(&path.to_string_lossy(), &src));
    }
    assert!(
        found.is_empty(),
        "use default foreground + Modifier::DIM for secondary text; DarkGray only on border styles:\n{}",
        found.join("\n")
    );
}

#[test]
fn scanner_flags_text_and_spares_only_border_styles() {
    let src = [
        "a.fg(Color::DarkGray)",                                  // 1 flag
        "b.border_style(Style::new().fg(Color::DarkGray))",       // 2 spare
        "c.gray()",                                               // 3 flag
        "d.dark_gray()",                                          // 4 flag
        "e.fg(Color::Gray)",                                      // 5 flag
        "f.fg(Color::LightGray)",                                 // 6 spare
        "g.fg(Color::Indexed(7))",                                // 7 flag
        "h.fg(Color::Indexed(8))",                                // 8 flag
        "i.fg(Color::White)",                                     // 9 flag
        "j.white()",                                              // 10 flag
        "k.fg(Color::Black)",                                     // 11 flag
        "l.black()",                                              // 12 flag
        "m.fg(Color::Indexed(9))",                                // 13 spare
        "let border = Style::new().gray(); // not a border call", // 14 flag
        "let x = Borders::ALL; y.fg(Color::DarkGray)",            // 15 spare
        "let bordered_text = Style::new().fg(Color::White);",     // 16 flag
        "border: Color::DarkGray,",                               // 17 spare
    ]
    .join("\n");
    let lines: Vec<usize> = violations("x.rs", &src)
        .iter()
        .map(|v| v.split(':').nth(1).unwrap().parse().unwrap())
        .collect();
    assert_eq!(lines, vec![1, 3, 4, 5, 7, 8, 9, 10, 11, 12, 14, 16]);
}
