//! Scenario files (TUI test harness design, section 5).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
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
    ExpectExit(i32),
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
        )
    }
}

fn default_tiers() -> Vec<Tier> {
    vec![Tier::Offline, Tier::Live]
}

fn default_timeout() -> Duration {
    Duration::from_secs(120)
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

/// Parses and validates a scenario.
pub fn parse(yaml: &str) -> Result<Scenario> {
    let s: Scenario = serde_yaml::from_str(yaml)?;
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
    fn test_key_bytes() {
        assert_eq!(key_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_bytes("Ctrl-K").unwrap(), vec![0x0b]);
        assert_eq!(key_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("Alt-x").unwrap(), b"\x1bx");
        assert_eq!(key_bytes("F5").unwrap(), b"\x1b[15~");
    }
}
