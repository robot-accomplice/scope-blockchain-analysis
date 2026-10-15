//! Scenario files (TUI test harness design, section 5).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Offline,
    Live,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Terminal {
    pub cols: u16,
    pub rows: u16,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            cols: 120,
            rows: 36,
        }
    }
}

fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let split = s
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| anyhow!("duration {s:?} has no unit"))?;
    let (n, unit) = s.split_at(split);
    let n: u64 = n.parse().with_context(|| format!("duration {s:?}"))?;
    Ok(match unit {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        _ => bail!("duration {s:?}: unit must be ms, s or m"),
    })
}

fn de_duration<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Duration, D::Error> {
    let s = String::deserialize(d)?;
    parse_duration(&s).map_err(serde::de::Error::custom)
}

fn de_opt_duration<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Duration>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|s| parse_duration(&s))
        .transpose()
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Type(String),
    Key(String),
    WaitFor {
        text: String,
        #[serde(default, deserialize_with = "de_opt_duration")]
        timeout: Option<Duration>,
    },
    ExpectScreen {
        #[serde(default)]
        contains: Vec<String>,
        #[serde(default)]
        not_contains: Vec<String>,
    },
    ExpectJournal {
        argv_prefix: Vec<String>,
        outcome: Option<String>,
        error: Option<String>,
    },
    ExpectFile {
        path: String,
        #[serde(default)]
        contains: Vec<String>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Snapshot(String),
    /// Sends a signal to the scope process: TERM, HUP, INT, QUIT, TSTP, CONT or STOP.
    Signal(String),
    ExpectExit(i32),
    /// Whether the terminal is in bracketed paste mode (`?2004h` seen, not reset).
    ExpectPasteMode(bool),
}

impl Step {
    fn is_expectation(&self) -> bool {
        matches!(
            self,
            Step::ExpectScreen { .. }
                | Step::ExpectJournal { .. }
                | Step::ExpectFile { .. }
                | Step::Snapshot(_)
                | Step::ExpectExit(_)
                | Step::ExpectPasteMode(_)
        )
    }
}

fn default_tiers() -> Vec<Tier> {
    vec![Tier::Offline, Tier::Live]
}

fn default_timeout() -> Duration {
    Duration::from_secs(120)
}

fn default_args() -> Vec<String> {
    vec!["tui".into(), "--no-banner".into()]
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default = "default_tiers")]
    pub tiers: Vec<Tier>,
    #[serde(default)]
    pub terminal: Terminal,
    #[serde(default = "default_timeout", deserialize_with = "de_duration")]
    pub timeout: Duration,
    pub config: Option<String>,
    #[serde(default)]
    pub masks: Vec<String>,
    /// Extra child env, applied after the defaults.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Arguments passed to the scope binary.
    #[serde(default = "default_args")]
    pub args: Vec<String>,
    /// Wait for the alternate screen before running steps.
    #[serde(default = "yes")]
    pub wait_for_paint: bool,
    #[serde(with = "serde_yaml::with::singleton_map_recursive")]
    pub steps: Vec<Step>,
}

/// The bytes a terminal sends for a named key.
pub fn key_bytes(name: &str) -> Result<Vec<u8>> {
    let fixed: Option<&[u8]> = match name {
        "Enter" => Some(b"\r"),
        "Tab" => Some(b"\t"),
        "Esc" => Some(b"\x1b"),
        "Backspace" => Some(b"\x7f"),
        "Up" => Some(b"\x1b[A"),
        "Down" => Some(b"\x1b[B"),
        "Right" => Some(b"\x1b[C"),
        "Left" => Some(b"\x1b[D"),
        "PageUp" => Some(b"\x1b[5~"),
        "PageDown" => Some(b"\x1b[6~"),
        "F1" => Some(b"\x1bOP"),
        "F2" => Some(b"\x1bOQ"),
        "F3" => Some(b"\x1bOR"),
        "F4" => Some(b"\x1bOS"),
        "F5" => Some(b"\x1b[15~"),
        "F6" => Some(b"\x1b[17~"),
        "F7" => Some(b"\x1b[18~"),
        "F8" => Some(b"\x1b[19~"),
        "F9" => Some(b"\x1b[20~"),
        "F10" => Some(b"\x1b[21~"),
        "F11" => Some(b"\x1b[23~"),
        "Home" => Some(b"\x1b[H"),
        "End" => Some(b"\x1b[F"),
        "Delete" => Some(b"\x1b[3~"),
        "Ctrl-Home" => Some(b"\x1b[1;5H"),
        "Ctrl-End" => Some(b"\x1b[1;5F"),
        "F12" => Some(b"\x1b[24~"),
        _ => None,
    };
    if let Some(b) = fixed {
        return Ok(b.to_vec());
    }
    let one_letter = |rest: &str| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic());
    if let Some(rest) = name.strip_prefix("Ctrl-").filter(|r| one_letter(r)) {
        return Ok(vec![rest.as_bytes()[0].to_ascii_uppercase() & 0x1f]);
    }
    if let Some(rest) = name.strip_prefix("Alt-").filter(|r| one_letter(r)) {
        return Ok(vec![0x1b, rest.as_bytes()[0]]);
    }
    bail!("unknown key name {name:?}")
}

/// The signal number for a scenario signal name.
#[cfg(unix)]
pub fn signal_number(name: &str) -> Result<i32> {
    Ok(match name {
        "TERM" => libc::SIGTERM,
        "HUP" => libc::SIGHUP,
        "INT" => libc::SIGINT,
        "QUIT" => libc::SIGQUIT,
        "TSTP" => libc::SIGTSTP,
        "CONT" => libc::SIGCONT,
        "STOP" => libc::SIGSTOP,
        _ => bail!("unknown signal name {name:?}; use TERM, HUP, INT, QUIT, TSTP, CONT or STOP"),
    })
}

/// Parses and validates a scenario.
pub fn parse(yaml: &str) -> Result<Scenario> {
    let s: Scenario = serde_yaml::from_str(yaml)?;
    if s.tiers.is_empty() {
        bail!(
            "scenario {:?} has an empty tiers list; it never runs",
            s.name
        );
    }
    for step in &s.steps {
        if let Step::ExpectScreen {
            contains,
            not_contains,
        } = step
            && contains.is_empty()
            && not_contains.is_empty()
        {
            bail!(
                "scenario {:?} has an expect_screen with no contains or not_contains; it cannot fail",
                s.name
            );
        }
    }
    if !s.steps.iter().any(Step::is_expectation) {
        bail!(
            "scenario {:?} has no expect_* or snapshot step; it cannot fail",
            s.name
        );
    }
    for step in &s.steps {
        if let Step::Key(k) = step {
            key_bytes(k)?;
        }
        #[cfg(unix)]
        if let Step::Signal(n) = step {
            signal_number(n)?;
        }
    }
    Ok(s)
}

/// Loads a scenario file.
pub fn load(path: &Path) -> Result<Scenario> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text).with_context(|| format!("in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
name: palette opens
terminal: { cols: 100, rows: 30 }
steps:
  - key: Ctrl-K
  - wait_for: { text: "Commands", timeout: 5s }
  - expect_screen: { contains: ["venues list"] }
  - key: Esc
  - expect_exit: 0
"#;

    #[test]
    fn test_parse_good_scenario() {
        let s = parse(GOOD).unwrap();
        assert_eq!(s.terminal.cols, 100);
        assert_eq!(s.tiers, vec![Tier::Offline, Tier::Live]);
        assert_eq!(s.timeout, Duration::from_secs(120));
        assert!(
            matches!(&s.steps[1], Step::WaitFor { timeout: Some(t), .. } if *t == Duration::from_secs(5))
        );
    }

    #[test]
    fn test_scenario_without_an_expectation_is_rejected() {
        let err = parse("name: x\nsteps:\n  - type: \"help\"\n  - key: Enter\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no expect_"), "{err}");
    }

    #[test]
    fn test_empty_expect_screen_is_rejected() {
        // An expect_screen with no lists checks nothing, so it must not
        // count as the expectation that lets a scenario fail.
        let err = parse("name: x\nsteps:\n  - expect_screen: {}\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("expect_screen"), "{err}");
    }

    #[test]
    fn test_empty_tiers_is_rejected() {
        // No tier means the scenario is skipped in every run: a silent pass.
        let err = parse("name: x\ntiers: []\nsteps:\n  - expect_exit: 0\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("tiers"), "{err}");
    }

    #[test]
    fn test_expect_file_without_contains_is_an_existence_check() {
        let s = parse("name: x\nsteps:\n  - expect_file: { path: out.csv }\n").unwrap();
        assert!(matches!(&s.steps[0], Step::ExpectFile { contains, .. } if contains.is_empty()));
    }

    #[test]
    fn test_unknown_key_name_is_rejected() {
        let err = parse("name: x\nsteps:\n  - key: Hyper-Q\n  - expect_exit: 0\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Hyper-Q"), "{err}");
    }

    #[test]
    fn test_unknown_step_field_is_rejected() {
        assert!(parse("name: x\nsteps:\n  - typo: 1\n  - expect_exit: 0\n").is_err());
    }

    #[test]
    fn test_new_fields_parse_with_defaults() {
        let s = parse("name: x\nsteps:\n  - expect_exit: 0\n").unwrap();
        assert!(s.env.is_empty());
        assert_eq!(s.args, vec!["tui".to_string(), "--no-banner".to_string()]);
        assert!(s.wait_for_paint);
    }

    #[test]
    fn test_signal_step_and_new_keys() {
        let s = parse("name: x\nenv: {TERM: dumb}\nargs: [tui]\nwait_for_paint: false\nsteps:\n  - signal: TERM\n  - expect_exit: 143\n").unwrap();
        assert_eq!(s.env.get("TERM").map(String::as_str), Some("dumb"));
        assert!(matches!(&s.steps[0], Step::Signal(n) if n == "TERM"));
        assert_eq!(key_bytes("Ctrl-End").unwrap(), b"\x1b[1;5F");
        assert_eq!(key_bytes("Delete").unwrap(), b"\x1b[3~");
        assert!(parse("name: x\nsteps:\n  - signal: BOGUS\n  - expect_exit: 0\n").is_err());
    }

    #[test]
    fn test_key_bytes() {
        assert_eq!(key_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_bytes("Ctrl-K").unwrap(), vec![0x0b]);
        assert_eq!(key_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("Alt-x").unwrap(), b"\x1bx");
        assert_eq!(key_bytes("F5").unwrap(), b"\x1b[15~");
    }
}
