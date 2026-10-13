//! Secondary text must stay readable on dark and light themes. Gray, white,
//! black and bright black vanish on some themes, so text uses the default
//! foreground plus the DIM modifier, or a named ANSI color. Borders may keep
//! DarkGray.
//!
//! The TUI names no color outside `cli/theme.rs`. Its test modules are exempt:
//! they read colors back from a rendered buffer.

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

/// Any named color or ratatui color shorthand.
const COLOR_TOKENS: &[&str] = &[
    "Color::",
    ".red()",
    ".cyan()",
    ".yellow()",
    ".green()",
    ".blue()",
    ".magenta()",
];

/// True when the `use` statement `stmt` imports ratatui's `Color`, alone
/// (`style::Color`) or in a group (`style::{Color, …}`).
fn imports_color(stmt: &str) -> bool {
    let Some((_, path)) = stmt.split_once("style::") else {
        return false;
    };
    let names = path.trim_start_matches('{');
    names
        .split([',', '}', ';'])
        .map(str::trim)
        .any(|name| name == "Color" || name.starts_with("Color as "))
}

/// `file:line: text` for each line of the non-test code that names a color,
/// and for each `use` that imports `Color`. The scan stops at the test
/// module: a `#[cfg(test)]` followed by `mod tests`. Another `#[cfg(test)]`
/// item does not stop it.
fn color_uses(file: &str, src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim_start().starts_with("mod tests"))
        .unwrap_or(lines.len());
    let mut found = Vec::new();
    let mut i = 0;
    while i < end {
        let line = lines[i];
        let flag = |found: &mut Vec<String>| {
            found.push(format!("{file}:{}: {}", i + 1, line.trim()));
        };
        if line.trim_start().starts_with("use ") {
            // A use statement can span lines; it ends at the `;`.
            let last = (i..end)
                .find(|&j| lines[j].contains(';'))
                .unwrap_or(end - 1);
            let stmt = lines[i..=last].join(" ");
            if imports_color(&stmt) || COLOR_TOKENS.iter().any(|t| stmt.contains(t)) {
                flag(&mut found);
            }
            i = last + 1;
            continue;
        }
        if COLOR_TOKENS.iter().any(|t| line.contains(t)) {
            flag(&mut found);
        }
        i += 1;
    }
    found
}

#[test]
fn tui_files_name_no_color_outside_theme_rs() {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/tui"),
        &mut files,
    );
    files.sort();
    let mut found = Vec::new();
    for path in files {
        let src = std::fs::read_to_string(&path).unwrap();
        found.extend(color_uses(&path.to_string_lossy(), &src));
    }
    assert!(
        found.is_empty(),
        "name colors only in cli/theme.rs (ACCENT, KEY, OK, WARN, ERR, INFO, SECONDARY, SELECTED, MATCH):\n{}",
        found.join("\n")
    );
}

#[test]
fn color_scanner_flags_named_colors_and_shorthands_in_non_test_code() {
    let src = [
        "a.fg(Color::Red)",                       // 1 flag
        "b.red()",                                // 2 flag
        "c.cyan().bold()",                        // 3 flag
        "d.yellow()",                             // 4 flag
        "e.green()",                              // 5 flag
        "f.blue()",                               // 6 flag
        "g.magenta()",                            // 7 flag
        "h.bold()",                               // 8 spare
        "i.fg(ACCENT.fg)",                        // 9 spare
        "#[cfg(test)]",                           // 10 not a test module: the scan goes on
        "static LOCK: () = ();",                  // 11 spare
        "k.fg(Color::Red)",                       // 12 flag
        "use ratatui::style::Color;",             // 13 flag
        "use ratatui::style::{Color, Modifier};", // 14 flag
        "use ratatui::style::{Modifier, Style};", // 15 spare
        "use ratatui::style::{",                  // 16 flag (the use)
        "    Color as C,",                        // 17
        "};",                                     // 18
        "use crate::cli::theme::{ACCENT, KEY};",  // 19 spare
        "#[cfg(test)]",                           // 20 stop
        "mod tests {",                            // 21
        "j.fg(Color::Red)",                       // 22 spare: test code
        "use ratatui::style::Color;",             // 23 spare: test code
    ]
    .join("\n");
    let lines: Vec<usize> = color_uses("x.rs", &src)
        .iter()
        .map(|v| v.split(':').nth(1).unwrap().parse().unwrap())
        .collect();
    assert_eq!(lines, vec![1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 16]);
}
