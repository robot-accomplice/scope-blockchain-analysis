//! Screen text, masks and snapshots (design 5.2, 5.4).

use anyhow::Result;
use regex::Regex;
use std::path::{Path, PathBuf};

/// The screen as text: one line per row, trailing spaces removed.
pub fn text(parser: &vt100::Parser) -> String {
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let mut out = String::new();
    for line in screen.rows(0, cols).take(rows as usize) {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Built-in patterns. The RFC 3339 pattern runs first so the bare
/// `HH:MM:SS` and date patterns do not split it.
const BUILT_IN: &[(&str, &str)] = &[
    (
        r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?",
        "<TIME>",
    ),
    (r"\b\d{2}:\d{2}:\d{2}\b", "<TIME>"),
    (r"\b\d{4}-\d{2}-\d{2}\b", "<DATE>"),
    (r"\b\d+(\.\d+)?(ms|s)\b", "<DUR>"),
];

/// Replaces text that changes from run to run.
pub fn mask(text: &str, extra: &[String]) -> Result<String> {
    let mut out = text.to_string();
    for (pat, rep) in BUILT_IN {
        out = Regex::new(pat)?.replace_all(&out, *rep).into_owned();
    }
    for pat in extra {
        out = Regex::new(pat)?.replace_all(&out, "<MASK>").into_owned();
    }
    Ok(out)
}

#[derive(Debug)]
pub enum SnapshotResult {
    Match,
    Written(PathBuf),
    Differs { expected: String, actual: String },
}

/// Compares `masked` with `<dir>/<name>.txt`, writing it when missing.
pub fn compare_snapshot(dir: &Path, name: &str, masked: &str) -> Result<SnapshotResult> {
    let path = dir.join(format!("{name}.txt"));
    match std::fs::read_to_string(&path) {
        Ok(expected) if expected == masked => Ok(SnapshotResult::Match),
        Ok(expected) => Ok(SnapshotResult::Differs {
            expected,
            actual: masked.to_string(),
        }),
        Err(_) => {
            std::fs::create_dir_all(dir)?;
            std::fs::write(&path, masked)?;
            Ok(SnapshotResult::Written(path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_reads_the_screen_and_trims() {
        let mut p = vt100::Parser::new(3, 10, 0);
        p.process(b"hi   \r\nthere");
        assert_eq!(text(&p), "hi\nthere\n");
    }

    #[test]
    fn test_mask_hides_times_dates_and_durations() {
        let m = mask(
            "at 2026-10-12T09:30:00Z, 14:05:09, took 123ms and 1.2s",
            &[],
        )
        .unwrap();
        assert_eq!(m, "at <TIME>, <TIME>, took <DUR> and <DUR>");
    }

    #[test]
    fn test_extra_mask() {
        assert_eq!(
            mask("age 3 days", &["age \\d+ days".into()]).unwrap(),
            "<MASK>"
        );
    }

    #[test]
    fn test_missing_snapshot_is_written_and_not_a_match() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(
            compare_snapshot(d.path(), "a", "x\n").unwrap(),
            SnapshotResult::Written(_)
        ));
        assert!(matches!(
            compare_snapshot(d.path(), "a", "x\n").unwrap(),
            SnapshotResult::Match
        ));
        assert!(matches!(
            compare_snapshot(d.path(), "a", "y\n").unwrap(),
            SnapshotResult::Differs { .. }
        ));
    }
}
