//! # Config clean: remove the dead keys of the old monitor dashboard
//!
//! Pure planning for `scope setup --clean`. [`plan`](crate::config_clean::plan) takes the text of a
//! config file and returns the text without the keys in
//! [`DROPPED_MONITOR_KEYS`](crate::config::DROPPED_MONITOR_KEYS), or says
//! why it cannot. It reads and writes no file.
//!
//! The edit is line based, so comments, key order, unknown keys, line
//! endings and a BOM stay as they are. It handles one shape: the block form
//! of the top-level `monitor:` mapping, which every `scope setup` wrote.
//! For any other shape it refuses ([`CleanError::Unsupported`](crate::config_clean::CleanError::Unsupported)). Before it
//! returns a plan, [`check`](crate::config_clean::check) proves with the YAML parser that only the dead
//! keys left (design: `docs/design/2026-11-12-config-clean-design.md`).

use crate::config::{Config, DROPPED_MONITOR_KEYS, dropped_monitor_keys};

/// One line the plan removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedLine {
    /// Line number in the original file, from 1.
    pub line: usize,
    /// The line without its line ending (and without a BOM).
    pub text: String,
}

/// The result of [`plan`]: what to remove and the text that remains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The cleaned file text.
    pub text: String,
    /// The removed lines, in file order.
    pub removed: Vec<RemovedLine>,
    /// The dead keys that were removed, in file order.
    pub keys: Vec<&'static str>,
    /// The `monitor:` line was removed too, because no live key was left.
    pub header_removed: bool,
}

/// Why [`plan`] gives no plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanError {
    /// The file is not valid config YAML.
    Parse(String),
    /// A dead key is in a shape the scanner does not handle.
    Unsupported {
        /// The key that cannot be removed automatically.
        key: String,
        /// Where to look, from 1, when known.
        line: Option<usize>,
    },
    /// A check of the result failed, so nothing may be written.
    Check(String),
}

impl std::fmt::Display for CleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "the config is not valid YAML: {e}. Fix the YAML first."),
            Self::Unsupported { key, line } => {
                write!(
                    f,
                    "cannot remove `monitor.{key}` automatically (the layout is not the block form)."
                )?;
                match line {
                    Some(n) => write!(f, " Delete it by hand: line {n}."),
                    None => write!(f, " Delete it by hand."),
                }
            }
            Self::Check(why) => write!(f, "the check of the cleaned text failed: {why}"),
        }
    }
}

impl std::error::Error for CleanError {}

/// Plans the removal of the dead monitor keys from `old`.
///
/// Returns `Ok(None)` when there is nothing to remove.
///
/// # Errors
///
/// [`CleanError::Parse`] when `old` is not valid config YAML,
/// [`CleanError::Unsupported`] when a dead key has a shape the scanner does
/// not know, [`CleanError::Check`] when the cleaned text fails [`check`].
pub fn plan(old: &str) -> Result<Option<Plan>, CleanError> {
    if let Err(e) = serde_saphyr::from_str::<Config>(old) {
        return Err(CleanError::Parse(e.to_string()));
    }
    let found = dropped_monitor_keys(old);
    if found.is_empty() {
        return Ok(None);
    }
    let plan = scan(old, found[0])?;
    if let Some(missed) = found.iter().find(|k| !plan.keys.contains(k)) {
        let line = monitor_line(old).map(|n| n + 1);
        tracing::warn!(
            key = *missed,
            ?line,
            "dead config key in an unsupported shape"
        );
        return Err(CleanError::Unsupported {
            key: (*missed).to_string(),
            line,
        });
    }
    check(old, &plan.text, &plan.keys, plan.header_removed).map_err(CleanError::Check)?;
    Ok(Some(plan))
}

/// The proof that `new` differs from `old` only by the dead keys `keys`
/// (and the `monitor:` line when `header_removed`). All five must hold:
///
/// 1. `new` parses as a [`Config`].
/// 2. The [`Config`] of `new` equals the [`Config`] of `old`.
/// 3. The generic YAML tree of `old`, minus `keys`, equals the tree of
///    `new`. This sees the keys that [`Config`] ignores.
/// 4. `new` has no dead key.
/// 5. `keys` are exactly the dead keys of `old`.
///
/// # Errors
///
/// A message that names the check that failed.
pub fn check(old: &str, new: &str, keys: &[&str], header_removed: bool) -> Result<(), String> {
    let new_config = serde_saphyr::from_str::<Config>(new)
        .map_err(|e| format!("check 1: the cleaned text does not parse: {e}"))?;
    let old_config = serde_saphyr::from_str::<Config>(old)
        .map_err(|e| format!("check 2: the original does not parse: {e}"))?;
    if old_config != new_config {
        return Err("check 2: the cleaned text loads to a different config".into());
    }

    let tree = |text: &str| serde_saphyr::from_str::<serde_json::Value>(text);
    let mut expected = tree(old).map_err(|e| format!("check 3: cannot read the original: {e}"))?;
    let actual = tree(new).map_err(|e| format!("check 3: cannot read the cleaned text: {e}"))?;
    if let Some(monitor) = expected.get_mut("monitor").and_then(|m| m.as_object_mut()) {
        for key in keys {
            monitor.remove(*key);
        }
        if header_removed && monitor.is_empty() {
            expected.as_object_mut().map(|top| top.remove("monitor"));
        }
    }
    // A file with no keys reads as null, an empty mapping as `{}`: the same.
    let norm = |v: serde_json::Value| match v {
        serde_json::Value::Object(o) if o.is_empty() => serde_json::Value::Null,
        v => v,
    };
    if norm(expected) != norm(actual) {
        return Err(
            "check 3: the cleaned text differs from the original by more than the dead keys".into(),
        );
    }

    if !dropped_monitor_keys(new).is_empty() {
        return Err("check 4: the cleaned text still has a dead key".into());
    }

    let mut removed: Vec<&str> = keys.to_vec();
    removed.sort_unstable();
    let mut dead: Vec<&str> = dropped_monitor_keys(old);
    dead.sort_unstable();
    if removed != dead {
        return Err(format!(
            "check 5: removed keys [{}] are not the dead keys [{}]",
            removed.join(", "),
            dead.join(", ")
        ));
    }
    Ok(())
}

/// The removed lines as hunks of a unified diff, deletions only. There are
/// no context lines, so no neighbour of a dead key (an API key, for
/// example) can show. A hunk is `@@ -start,count +before,0 @@`, where
/// `before` is the line before the deletion in the new file, as `diff -u`
/// writes it.
pub fn diff_body(removed: &[RemovedLine]) -> String {
    let mut out = String::new();
    let mut gone = 0usize;
    let mut i = 0;
    while i < removed.len() {
        let mut j = i;
        while j + 1 < removed.len() && removed[j + 1].line == removed[j].line + 1 {
            j += 1;
        }
        let start = removed[i].line;
        let count = j - i + 1;
        out.push_str(&format!("@@ -{start},{count} +{},0 @@\n", start - 1 - gone));
        for r in &removed[i..=j] {
            out.push_str(&format!("-{}\n", r.text));
        }
        gone += count;
        i = j + 1;
    }
    out
}

// ---------------------------------------------------------------------
// The scanner
// ---------------------------------------------------------------------

/// A line without its ending, and without a BOM on line 1.
fn body(line: &str, index: usize) -> &str {
    let line = if index == 0 {
        line.strip_prefix('\u{feff}').unwrap_or(line)
    } else {
        line
    };
    line.trim_end_matches(['\r', '\n'])
}

fn is_live(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && !t.starts_with('#')
}

/// Leading spaces. A tab in the leading whitespace is `None`.
fn indent(text: &str) -> Option<usize> {
    let lead = text.len() - text.trim_start_matches([' ', '\t']).len();
    let lead = &text[..lead];
    if lead.contains('\t') {
        None
    } else {
        Some(lead.len())
    }
}

/// What follows a key's colon is nothing, or a comment.
fn value_is_empty(after_colon: &str) -> bool {
    let t = after_colon.trim();
    t.is_empty() || (after_colon.starts_with([' ', '\t']) && t.starts_with('#'))
}

/// `Some(rest)` when `text` (after `ind` spaces) is `name:` and then a
/// space or the end of the line.
fn key_rest<'a>(text: &'a str, ind: usize, name: &str) -> Option<&'a str> {
    let after = text.get(ind..)?.strip_prefix(name)?.strip_prefix(':')?;
    (after.is_empty() || after.starts_with([' ', '\t'])).then_some(after)
}

/// The 0-based index of the one top-level `monitor:` line in block form.
/// `None` when there is none, or the key appears in another form or twice.
fn monitor_line(text: &str) -> Option<usize> {
    let mut found = None;
    let mut count = 0;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let b = body(raw, i);
        let like =
            b.starts_with("monitor") || b.starts_with("\"monitor\"") || b.starts_with("'monitor'");
        if !like {
            continue;
        }
        count += 1;
        if let Some(rest) = key_rest(b, 0, "monitor")
            && value_is_empty(rest)
        {
            found = Some(i);
        }
    }
    if count == 1 { found } else { None }
}

/// Scans the `monitor:` block and builds the plan. `first` names a dead key
/// for the error when there is no usable `monitor:` line.
fn scan(old: &str, first: &str) -> Result<Plan, CleanError> {
    let unsupported = |key: &str, line: Option<usize>| CleanError::Unsupported {
        key: key.to_string(),
        line: line.map(|n| n + 1),
    };
    let raw: Vec<&str> = old.split_inclusive('\n').collect();
    let text = |i: usize| body(raw[i], i);
    let m = monitor_line(old).ok_or_else(|| unsupported(first, None))?;

    // The block ends at the next live line with no indent.
    let mut end = raw.len();
    for i in m + 1..raw.len() {
        let t = text(i);
        if is_live(t) {
            match indent(t) {
                None => return Err(unsupported(first, Some(i))),
                Some(0) => {
                    end = i;
                    break;
                }
                Some(_) => {}
            }
        }
    }

    let Some(first_live) = (m + 1..end).find(|&i| is_live(text(i))) else {
        return Err(unsupported(first, Some(m)));
    };
    let child = indent(text(first_live)).unwrap_or(0);
    if text(first_live).trim_start().starts_with('-') {
        return Err(unsupported(first, Some(first_live)));
    }

    let mut gone = vec![false; raw.len()];
    let mut keys: Vec<&'static str> = Vec::new();
    let mut i = m + 1;
    while i < end {
        let t = text(i);
        let hit = if is_live(t) && indent(t) == Some(child) {
            DROPPED_MONITOR_KEYS
                .iter()
                .find_map(|name| key_rest(t, child, name).map(|rest| (*name, rest)))
        } else {
            None
        };
        let Some((name, rest)) = hit else {
            i += 1;
            continue;
        };
        // The extent: deeper lines, plus (for an empty value) list items
        // at the key's own indent. Blank and comment lines count only
        // between two lines of the value.
        let own_list = value_is_empty(rest);
        let mut last = i;
        for k in i + 1..end {
            let tk = text(k);
            if !is_live(tk) {
                continue;
            }
            let deeper = indent(tk).is_some_and(|n| n > child);
            let item = own_list
                && indent(tk) == Some(child)
                && tk
                    .trim_start()
                    .strip_prefix('-')
                    .is_some_and(|r| r.is_empty() || r.starts_with(' '));
            if deeper || item {
                last = k;
            } else {
                break;
            }
        }
        for flag in &mut gone[i..=last] {
            *flag = true;
        }
        keys.push(name);
        i = last + 1;
    }

    let live_left = (m + 1..end).any(|i| !gone[i] && is_live(text(i)));
    let header_removed = !live_left;
    if header_removed {
        gone[m] = true;
    }

    let mut new_text = String::with_capacity(old.len());
    let mut removed = Vec::new();
    for (i, line) in raw.iter().enumerate() {
        if gone[i] {
            removed.push(RemovedLine {
                line: i + 1,
                text: text(i).to_string(),
            });
        } else {
            new_text.push_str(line);
        }
    }
    Ok(Plan {
        text: new_text,
        removed,
        keys,
        header_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All six dead keys, a mix of shapes, and live neighbours.
    const FULL: &str = "# top comment
chains:
  api_keys:
    etherscan: \"fixture-not-a-key\"
layout: top-level-layout-stays
output:
  layout: output-layout-stays
monitor:
  # about the refresh
  refresh_seconds: 7
  layout: default
  widgets:
    price_chart: false
    volume: true
  scale: log
  color_scheme: monochrome
  export:
    path: out
  auto_pause_on_input: true
  refresh_secs: 3
  venue: binance
web:
  port: 1
";

    fn planned(text: &str) -> Plan {
        plan(text).unwrap().expect("expected dead keys")
    }

    fn line_texts(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn test_only_the_six_keys_under_monitor_go() {
        // The owner said "never unknown keys". A `layout` elsewhere, and a
        // typo such as `refresh_secs`, are the user's data.
        let p = planned(FULL);
        let left = line_texts(&p.text);
        for kept in [
            "layout: top-level-layout-stays",
            "  layout: output-layout-stays",
            "  refresh_secs: 3",
            "  refresh_seconds: 7",
            "  venue: binance",
        ] {
            assert!(left.iter().any(|l| l == kept), "lost: {kept}");
        }
        assert!(dropped_monitor_keys(&p.text).is_empty());
        let mut keys = p.keys.clone();
        keys.sort_unstable();
        let mut all = DROPPED_MONITOR_KEYS.to_vec();
        all.sort_unstable();
        assert_eq!(keys, all);
    }

    #[test]
    fn test_whole_values_go_with_no_orphan_line() {
        // A half-removed value is invalid YAML, or a changed one. Four
        // shapes: block map, list, block scalar, flow list over lines.
        let text = "monitor:
  venue: binance
  export:
    path: out
    nested:
      deep: 1
  widgets:
    - a
    - b
  layout: |
    line one
    line two

    line four
  scale: [1,
    2,
    3]
  refresh_seconds: 9
";
        let p = planned(text);
        assert_eq!(p.text, "monitor:\n  venue: binance\n  refresh_seconds: 9\n");
        assert_eq!(p.keys, ["export", "widgets", "layout", "scale"]);
    }

    #[test]
    fn test_a_list_at_the_keys_own_indent_goes_whole() {
        // `scope setup` could write a list level with its key. Leaving the
        // `- ` lines would make the file invalid.
        let text = "monitor:\n  widgets:\n  - a\n  - b\n  venue: binance\n";
        assert_eq!(planned(text).text, "monitor:\n  venue: binance\n");
    }

    #[test]
    fn test_everything_else_is_byte_identical_in_order_with_comments() {
        // The reason for the line-based method: the user's comments and
        // layout survive.
        let p = planned(FULL);
        let removed: Vec<usize> = p.removed.iter().map(|r| r.line).collect();
        let kept: Vec<&str> = FULL
            .split_inclusive('\n')
            .enumerate()
            .filter(|(i, _)| !removed.contains(&(i + 1)))
            .map(|(_, l)| l)
            .collect();
        assert_eq!(p.text, kept.concat());
        assert!(p.text.starts_with("# top comment\n"));
        assert!(p.text.contains("  # about the refresh\n"));
    }

    #[test]
    fn test_line_endings_and_bom_are_kept() {
        // A Windows config must not become a mixed file, and a BOM must
        // not be eaten.
        let lf = "monitor:\n  layout: x\n  venue: binance\n";
        let crlf = lf.replace('\n', "\r\n");
        let bom = format!("\u{feff}{lf}");
        let bom_crlf = format!("\u{feff}{crlf}");
        let mixed = "monitor:\r\n  layout: x\n  venue: binance\r\n";
        let no_final = "monitor:\n  venue: binance\n  layout: x";
        assert_eq!(planned(lf).text, "monitor:\n  venue: binance\n");
        assert_eq!(planned(&crlf).text, "monitor:\r\n  venue: binance\r\n");
        assert_eq!(planned(&bom).text, "\u{feff}monitor:\n  venue: binance\n");
        assert_eq!(
            planned(&bom_crlf).text,
            "\u{feff}monitor:\r\n  venue: binance\r\n"
        );
        assert_eq!(planned(mixed).text, "monitor:\r\n  venue: binance\r\n");
        assert_eq!(planned(no_final).text, "monitor:\n  venue: binance\n");
    }

    #[test]
    fn test_a_bom_before_the_monitor_line_still_matches() {
        // The BOM is not matched, but it must not hide `monitor:` on line 1.
        let text = "\u{feff}monitor:\n  layout: x\n  venue: b\n";
        assert_eq!(planned(text).text, "\u{feff}monitor:\n  venue: b\n");
        assert_eq!(planned(text).removed[0].text, "  layout: x");
    }

    #[test]
    fn test_an_empty_block_loses_its_header() {
        // A bare `monitor:` is a null. The check must prove the parser
        // accepts the result and the config is equal.
        let text =
            "chains:\n  api_keys: {}\nmonitor:\n  layout: x\n  scale: 2\noutput:\n  color: true\n";
        let p = planned(text);
        assert!(p.header_removed);
        assert_eq!(p.text, "chains:\n  api_keys: {}\noutput:\n  color: true\n");
        assert_eq!(
            serde_saphyr::from_str::<Config>(&p.text).unwrap(),
            serde_saphyr::from_str::<Config>(text).unwrap()
        );
    }

    #[test]
    fn test_unsupported_shapes_refuse_with_key_and_line() {
        // The scanner must not guess at a shape it does not know.
        for (text, line) in [
            ("monitor: {layout: x, venue: b}\n", None),
            ("\"monitor\":\n  layout: x\n", None),
            // A tab in the indent of a block scalar line parses, but the
            // scanner cannot measure it.
            (
                "monitor:\n  layout: |\n    a\n    \tb\n  venue: v\n",
                Some(4),
            ),
            ("monitor:\n  \"layout\": x\n  venue: b\n", Some(1)),
        ] {
            let err = plan(text).unwrap_err();
            let CleanError::Unsupported { key, line: at } = &err else {
                panic!("{text:?} gave {err:?}");
            };
            assert!(!key.is_empty());
            assert_eq!(*at, line, "{text:?}");
            assert!(err.to_string().contains("Delete it by hand"), "{err}");
        }
    }

    #[test]
    fn test_a_second_monitor_key_stops_at_the_parser() {
        // The YAML parser refuses a duplicate key, so the scanner never
        // sees it: a parse error, with nothing planned.
        let text = "monitor:\n  layout: x\nmonitor:\n  scale: 1\n";
        assert!(matches!(plan(text), Err(CleanError::Parse(_))));
    }

    #[test]
    fn test_cleaning_twice_finds_nothing() {
        // A second run must not change a file or make a backup.
        let p = planned(FULL);
        assert_eq!(plan(&p.text).unwrap(), None);
    }

    #[test]
    fn test_no_dead_keys_means_no_plan() {
        assert_eq!(plan("monitor:\n  venue: b\n").unwrap(), None);
        assert_eq!(plan("").unwrap(), None);
    }

    #[test]
    fn test_bad_yaml_is_a_parse_error() {
        // Nothing may be guessed from text that does not parse.
        assert!(matches!(
            plan("monitor: [unclosed\n"),
            Err(CleanError::Parse(_))
        ));
    }

    #[test]
    fn test_the_check_can_fail() {
        // A check that never failed is not proven. Each bad plan must trip
        // the check that is meant to catch it.
        let old = "chains:\n  ethereum_rpc: http://a\nmonitor:\n  layout: x\n  venue: b\n  refresh_secs: 3\n";
        let good = "chains:\n  ethereum_rpc: http://a\nmonitor:\n  venue: b\n  refresh_secs: 3\n";
        assert_eq!(check(old, good, &["layout"], false), Ok(()));

        // Check 1: not YAML.
        let err = check(old, "monitor: [", &["layout"], false).unwrap_err();
        assert!(err.starts_with("check 1"), "{err}");
        // Check 2: a live line is gone (the RPC), so the Config differs.
        let live = "monitor:\n  venue: b\n  refresh_secs: 3\n";
        let err = check(old, live, &["layout"], false).unwrap_err();
        assert!(err.starts_with("check 2"), "{err}");
        // Check 3: an unknown key is gone. Config ignores it, the tree sees it.
        let unknown = "chains:\n  ethereum_rpc: http://a\nmonitor:\n  venue: b\n";
        let err = check(old, unknown, &["layout"], false).unwrap_err();
        assert!(err.starts_with("check 3"), "{err}");
        // Check 4: the dead key is still there.
        let err = check(old, old, &[], false).unwrap_err();
        assert!(err.starts_with("check 4"), "{err}");
        // A plan that names both keys but left one behind trips check 3.
        let old2 = "monitor:\n  layout: x\n  scale: 1\n";
        let still = "monitor:\n  scale: 1\n";
        let err = check(old2, still, &["layout", "scale"], false).unwrap_err();
        assert!(err.starts_with("check 3"), "{err}");
        // A plan that removed only one of two dead keys trips check 4.
        let err = check(old2, still, &["layout"], false).unwrap_err();
        assert!(err.starts_with("check 4"), "{err}");
        // A plan that removed a live line trips check 3.
        let err = check(old2, "{}\n", &["layout"], false).unwrap_err();
        assert!(err.starts_with("check 3"), "{err}");
    }

    #[test]
    fn test_check_5_names_a_missed_dead_key() {
        // The text is right, but `keys` names `venue`, which is not a dead
        // key: only check 5 can see it.
        let old = "a: 1\nmonitor:\n  layout: x\n";
        let err = check(old, "a: 1\n", &["layout", "venue"], true).unwrap_err();
        assert!(err.starts_with("check 5"), "{err}");
    }

    #[test]
    fn test_one_list_feeds_detection_notice_and_cleaner() {
        // A key added to the constant must reach all three readers. A
        // second list would drift.
        let mut text = String::from("monitor:\n");
        for key in DROPPED_MONITOR_KEYS {
            text.push_str(&format!("  {key}: 1\n"));
        }
        text.push_str("  venue: b\n");
        let found = dropped_monitor_keys(&text);
        assert_eq!(found.len(), DROPPED_MONITOR_KEYS.len());
        let notice = crate::config::dropped_keys_notice(&found, None);
        let p = planned(&text);
        for key in DROPPED_MONITOR_KEYS {
            assert!(notice.contains(key), "notice lacks {key}");
            assert!(p.keys.contains(&key), "cleaner lacks {key}");
        }
    }

    #[test]
    fn test_detector_and_scanner_agree_on_a_table_of_files() {
        // They are two readers of one fact. If they differ, a file is
        // either half-cleaned or refused.
        for text in [
            FULL,
            "monitor:\n  layout: x\n",
            "monitor: # c\n  export:\n    a: 1\n  venue: b\n",
            "a: 1\nmonitor:\n  scale: 1\n\n# tail\nb: 2\n",
            "monitor:\n  widgets:\n  - a\n  color_scheme: x\n",
        ] {
            let p = planned(text);
            let mut a = p.keys.clone();
            a.sort_unstable();
            let mut b = dropped_monitor_keys(text);
            b.sort_unstable();
            assert_eq!(a, b, "{text}");
        }
    }

    #[test]
    fn test_a_comment_above_a_removed_key_stays() {
        // Q4: a comment may not be about the key; an orphan costs nothing.
        let text = "monitor:\n  # about layout\n  layout: x\n  venue: b\n";
        assert_eq!(
            planned(text).text,
            "monitor:\n  # about layout\n  venue: b\n"
        );
    }

    #[test]
    fn test_diff_has_only_deletions_and_no_context() {
        // Context lines would print the neighbours of `monitor:`, and the
        // config holds API keys in `chains:`.
        let p = planned(FULL);
        let diff = diff_body(&p.removed);
        assert!(!diff.contains("fixture-not-a-key"), "{diff}");
        assert!(
            diff.lines()
                .all(|l| l.starts_with('-') || l.starts_with("@@"))
        );
        // Original lines 10 (layout), 11-13 (widgets), 14, 15, 16-17, 18.
        assert!(diff.starts_with("@@ -11,9 +10,0 @@\n"), "{diff}");
        // The second hunk's new-file start is the line before the cut.
        let text = "monitor:\n  layout: a\n  venue: b\n  scale: 2\n  refresh_seconds: 1\n";
        let d = diff_body(&planned(text).removed);
        assert_eq!(
            d,
            "@@ -2,1 +1,0 @@\n-  layout: a\n@@ -4,1 +2,0 @@\n-  scale: 2\n"
        );
    }
}
