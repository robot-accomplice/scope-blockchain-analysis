//! The watchlist file: `<scope home>/watchlists/<name>.yaml`.
//!
//! ```yaml
//! name: stablecoins
//! interval: 30s
//! duration: 1h          # optional; absent or `forever` = until stopped
//! outputs:
//!   csv: ./watch.csv
//!   report: ./watch.md
//! targets:
//!   - kind: pair
//!     id: USDC
//!     venue: binance
//!     preset: market health
//!     health:
//!       max_spread_pct: 0.5
//!   - kind: token
//!     id: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
//!     chain: ethereum
//!     rules:
//!       - holders >= 1000
//!       - liquidity_usd > 1000000
//! ```
//!
//! [`parse`] and [`to_text`] are the only functions that touch the YAML
//! crate (#47, N7). [`parse`] reports every error with its line, not only
//! the first.

use super::time::{format_duration, is_until_cancelled, parse_duration};
use super::{Outputs, Preset, Rule, Target, TargetKind, Watchlist};
use crate::market::HealthOverrides;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The file extension of a watchlist.
pub const EXTENSION: &str = "yaml";

/// One problem in a watchlist file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    /// 1-based line, when the problem has one.
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Every problem in a watchlist file, one per line of the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileErrors(pub Vec<FileError>);

impl fmt::Display for FileErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&lines.join("\n"))
    }
}

impl std::error::Error for FileErrors {}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct RawWatchlist {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration: Option<String>,
    #[serde(default, skip_serializing_if = "RawOutputs::is_empty")]
    outputs: RawOutputs,
    #[serde(default)]
    targets: Vec<RawTarget>,
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct RawOutputs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    csv: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report: Option<PathBuf>,
}

impl RawOutputs {
    fn is_empty(&self) -> bool {
        self.csv.is_none() && self.report.is_none()
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    kind: String,
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    health: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rules: Vec<String>,
}

/// Parses a watchlist file's text. `default_name` names a file with no
/// `name:`. With `venues`, a venue must be one of them.
pub fn parse(
    text: &str,
    default_name: &str,
    venues: Option<&[String]>,
) -> Result<Watchlist, FileErrors> {
    let raw: RawWatchlist = serde_saphyr::from_str(text).map_err(|e| {
        FileErrors(vec![FileError {
            line: None,
            message: format!("not a valid watchlist: {e}"),
        }])
    })?;
    let mut errors = Vec::new();
    let mut error = |needle: Option<&str>, message: String| {
        errors.push(FileError {
            line: needle.and_then(|n| line_of(text, n)),
            message,
        })
    };

    let interval = match raw.interval.as_deref() {
        None => Duration::from_secs(super::time::DEFAULT_INTERVAL_SECS),
        Some(s) => match parse_duration(s) {
            Ok(secs) => Duration::from_secs(secs),
            Err(e) => {
                error(Some("interval:"), format!("interval `{s}`: {e}"));
                Duration::from_secs(super::time::DEFAULT_INTERVAL_SECS)
            }
        },
    };
    let duration = match raw.duration.as_deref() {
        None => None,
        Some(s) if is_until_cancelled(s) => None,
        Some(s) => match parse_duration(s) {
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(e) => {
                error(Some("duration:"), format!("duration `{s}`: {e}"));
                None
            }
        },
    };

    let mut targets = Vec::new();
    for (i, t) in raw.targets.into_iter().enumerate() {
        let n = i + 1;
        let id_line = format!("id: {}", t.id);
        let at = Some(id_line.as_str());
        let kind = match t.kind.trim().to_ascii_lowercase().as_str() {
            "token" => TargetKind::Token,
            "pair" => TargetKind::Pair,
            "address" => TargetKind::Address,
            other => {
                error(
                    Some(&format!("kind: {}", t.kind)),
                    format!("target {n}: unknown kind `{other}`; use token, pair or address"),
                );
                continue;
            }
        };
        if t.id.trim().is_empty() {
            error(None, format!("target {n}: the id is empty"));
        }
        let mut target = Target::new(kind, t.id.trim());
        target.chain = t.chain;
        target.venue = t.venue;
        if let (Some(venue), Some(known)) = (&target.venue, venues)
            && !known.iter().any(|v| v.eq_ignore_ascii_case(venue))
        {
            error(
                Some(&format!("venue: {venue}")),
                format!(
                    "target {n} ({}): unknown venue `{venue}`; the venues are {}",
                    target.id,
                    known.join(", ")
                ),
            );
        }
        if let Some(p) = &t.preset {
            match Preset::from_name(p) {
                Some(preset) if kind.accepts(crate::metrics::MetricGroup::Market) => {
                    target.preset = Some(preset)
                }
                Some(_) => error(
                    Some(&format!("preset: {p}")),
                    format!(
                        "target {n} ({}): the preset `{p}` needs a token or a pair, not an {}",
                        target.id,
                        kind.name()
                    ),
                ),
                None => error(
                    Some(&format!("preset: {p}")),
                    format!("target {n} ({}): unknown preset `{p}`", target.id),
                ),
            }
        }
        if let Some(map) = t.health {
            match serde_json::from_value::<HealthOverrides>(serde_json::Value::Object(map)) {
                Ok(h) => target.health = h,
                Err(e) => error(
                    Some("health:"),
                    format!("target {n} ({}): health: {e}", target.id),
                ),
            }
        }
        for text_rule in &t.rules {
            match Rule::parse(text_rule) {
                Ok(rule) if kind.accepts(rule.metric.group()) => target.rules.push(rule),
                Ok(rule) => {
                    let valid: Vec<&str> = kind.metrics().iter().map(|m| m.name()).collect();
                    error(
                        Some(text_rule),
                        format!(
                            "target {n} ({}): `{}` does not apply to a {}; its metrics are {}",
                            target.id,
                            rule.metric.name(),
                            kind.name(),
                            valid.join(", ")
                        ),
                    );
                }
                Err(e) => error(Some(text_rule), format!("target {n} ({}): {e}", target.id)),
            }
        }
        if target.needs_market() && target.venue.is_none() {
            error(
                at,
                format!(
                    "target {n} ({}): market metrics need a venue (`venue: binance`)",
                    target.id
                ),
            );
        }
        if kind == TargetKind::Pair && target.venue.is_none() && !target.needs_market() {
            error(
                at,
                format!("target {n} ({}): a pair needs a venue", target.id),
            );
        }
        targets.push(target);
    }

    if !errors.is_empty() {
        return Err(FileErrors(errors));
    }
    Ok(Watchlist {
        name: raw
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| default_name.to_string()),
        interval,
        duration,
        outputs: Outputs {
            csv: raw.outputs.csv,
            report: raw.outputs.report,
        },
        targets,
    })
}

/// The 1-based line of the first line that contains `needle`.
fn line_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

/// The file text of `w`. [`parse`] reads it back equal.
pub fn to_text(w: &Watchlist) -> String {
    let raw = RawWatchlist {
        name: Some(w.name.clone()),
        interval: Some(format_duration(w.interval.as_secs())),
        duration: Some(
            w.duration
                .map_or_else(|| "forever".to_string(), |d| format_duration(d.as_secs())),
        ),
        outputs: RawOutputs {
            csv: w.outputs.csv.clone(),
            report: w.outputs.report.clone(),
        },
        targets: w
            .targets
            .iter()
            .map(|t| RawTarget {
                kind: t.kind.name().to_string(),
                id: t.id.clone(),
                chain: t.chain.clone(),
                venue: t.venue.clone(),
                preset: t.preset.map(|p| p.name().to_string()),
                health: health_map(&t.health),
                rules: t.rules.iter().map(ToString::to_string).collect(),
            })
            .collect(),
    };
    serde_saphyr::to_string(&raw).unwrap_or_default()
}

/// The overrides that are set, or `None` when none is.
fn health_map(h: &HealthOverrides) -> Option<serde_json::Map<String, serde_json::Value>> {
    let serde_json::Value::Object(map) = serde_json::to_value(h).ok()? else {
        return None;
    };
    let set: serde_json::Map<_, _> = map.into_iter().filter(|(_, v)| !v.is_null()).collect();
    (!set.is_empty()).then_some(set)
}

/// The file of the watchlist `name` in `dir`.
pub fn path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXTENSION}"))
}

/// Reads the watchlist at `path`. A file with no `name:` takes its stem.
pub fn load(path: &Path, venues: Option<&[String]>) -> Result<Watchlist, FileErrors> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        FileErrors(vec![FileError {
            line: None,
            message: format!("cannot read {}: {e}", path.display()),
        }])
    })?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("watchlist");
    parse(&text, stem, venues)
}

/// Writes `w` to `path`, creating its folder.
pub fn save(path: &Path, w: &Watchlist) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, to_text(w))
}

/// The watchlist names in `dir`, sorted. A missing folder has none.
pub fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXTENSION))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricId;
    use crate::watch::Op;

    const EXAMPLE: &str = "name: stablecoins
interval: 30s
duration: 1h
outputs:
  csv: ./watch.csv
  report: ./watch.md
targets:
  - kind: pair
    id: USDC
    venue: binance
    preset: market health
    health:
      max_spread_pct: 0.5
  - kind: token
    id: \"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48\"
    chain: ethereum
    venue: binance
    rules:
      - holders >= 1000
      - liquidity_usd > 1000000
      - spread_pct < 1
  - kind: address
    id: \"0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2\"
    rules:
      - risk_score <= 5
";

    fn venues() -> Vec<String> {
        vec!["binance".into(), "weex".into()]
    }

    fn errors(text: &str) -> Vec<FileError> {
        parse(text, "w", Some(&venues())).unwrap_err().0
    }

    #[test]
    fn the_example_loads() {
        let w = parse(EXAMPLE, "file", Some(&venues())).unwrap();
        assert_eq!(w.name, "stablecoins");
        assert_eq!(w.interval, Duration::from_secs(30));
        assert_eq!(w.duration, Some(Duration::from_secs(3600)));
        assert_eq!(w.outputs.csv.as_deref(), Some(Path::new("./watch.csv")));
        assert_eq!(w.targets.len(), 3);
        let pair = &w.targets[0];
        assert_eq!(pair.preset, Some(Preset::MarketHealth));
        assert_eq!(pair.health.max_spread_pct, Some(0.5));
        let token = &w.targets[1];
        assert_eq!(token.chain.as_deref(), Some("ethereum"));
        assert_eq!(
            token.rules[0],
            Rule {
                metric: MetricId::Holders,
                op: Op::Ge,
                value: 1000.0
            }
        );
        assert_eq!(w.targets[2].kind, TargetKind::Address);
    }

    #[test]
    fn save_then_load_is_equal() {
        let w = parse(EXAMPLE, "file", None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir.path().join("watchlists"), "stablecoins");
        save(&path, &w).unwrap();
        assert_eq!(load(&path, None).unwrap(), w);
        assert_eq!(list(&dir.path().join("watchlists")), vec!["stablecoins"]);
    }

    #[test]
    fn until_stopped_reads_back() {
        let w = parse("duration: forever\n", "w", None).unwrap();
        assert_eq!(w.duration, None);
        assert_eq!(parse(&to_text(&w), "w", None).unwrap(), w);
        let w = parse("{}\n", "plain", None).unwrap();
        assert_eq!((w.name.as_str(), w.duration), ("plain", None));
        assert_eq!(w.interval, Duration::from_secs(60));
    }

    #[test]
    fn bad_yaml_is_one_error() {
        let e = errors("targets: [\n");
        assert_eq!(e.len(), 1);
        assert!(e[0].message.starts_with("not a valid watchlist"), "{e:?}");
        let e = errors("intervall: 30s\n");
        assert!(
            e[0].message.contains("intervall"),
            "unknown key named: {e:?}"
        );
    }

    #[test]
    fn every_problem_is_reported_with_its_line() {
        let text = "interval: 0s
targets:
  - kind: pair
    id: USDT
  - kind: token
    id: PEPE
    rules:
      - holdrs > 1
      - risk_score < 3
  - kind: pair
    id: DAI
    venue: nowhere
    preset: market health
  - kind: wallet
    id: x
";
        let e = errors(text);
        let find = |s: &str| {
            e.iter()
                .find(|x| x.message.contains(s))
                .unwrap_or_else(|| panic!("no error with {s:?} in {e:#?}"))
        };
        assert_eq!(find("interval `0s`").line, Some(1));
        assert_eq!(find("a pair needs a venue").line, Some(4));
        let unknown = find("unknown metric `holdrs`");
        assert_eq!(unknown.line, Some(8));
        let kind = find("`risk_score` does not apply to a token");
        assert_eq!(kind.line, Some(9));
        assert!(kind.message.contains("holders"), "names the valid metrics");
        let venue = find("unknown venue `nowhere`");
        assert_eq!(venue.line, Some(12));
        assert!(venue.message.contains("binance, weex"));
        assert_eq!(find("unknown kind `wallet`").line, Some(14));
        assert_eq!(e.len(), 6, "{e:#?}");
        let all = FileErrors(e).to_string();
        assert!(all.starts_with("line 1: interval"), "{all}");
    }

    #[test]
    fn market_metrics_need_a_venue_and_the_preset_needs_a_market_kind() {
        let e =
            errors("targets:\n  - kind: token\n    id: PEPE\n    rules: [\"spread_pct < 1\"]\n");
        assert!(
            e[0].message.contains("market metrics need a venue"),
            "{e:?}"
        );
        let e = errors("targets:\n  - kind: address\n    id: \"0x1\"\n    preset: market health\n");
        assert!(e[0].message.contains("needs a token or a pair"), "{e:?}");
    }

    #[test]
    fn a_missing_folder_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("none")).is_empty());
    }
}
