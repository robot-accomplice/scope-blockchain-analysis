//! Secondary text must stay readable on dark and light themes. Bright black
//! and gray vanish on some themes, so text uses the default foreground plus
//! the DIM modifier. Borders may keep DarkGray.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["DarkGray", "Color::Gray", ".gray()", ".dark_gray()"];
/// A line that names one of these is a border style, which is exempt.
const BORDER_MARKERS: &[&str] = &["border", "Borders"];

fn scanned_files() -> Vec<PathBuf> {
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");
    let mut files: Vec<PathBuf> = std::fs::read_dir(cli.join("tui"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.push(cli.join("monitor/widgets.rs"));
    files.push(cli.join("monitor/config.rs"));
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
fn text_never_uses_gray_or_dark_gray() {
    let mut found = Vec::new();
    for path in scanned_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        found.extend(violations(&path.to_string_lossy(), &src));
    }
    assert!(
        found.is_empty(),
        "use default foreground + Modifier::DIM for secondary text; DarkGray/Gray only on borders:\n{}",
        found.join("\n")
    );
}

#[test]
fn scanner_flags_text_and_spares_borders() {
    let src = "a.fg(Color::DarkGray)\nb.border_style(Style::new().fg(Color::DarkGray))\nc.gray()\nd.dark_gray()\ne.fg(Color::Gray)\nf.fg(Color::LightGray)\n";
    let lines: Vec<usize> = violations("x.rs", src)
        .iter()
        .map(|v| v.split(':').nth(1).unwrap().parse().unwrap())
        .collect();
    assert_eq!(lines, vec![1, 3, 4, 5]);
}
