//! A watchlist run: ticks on the watchlist's schedule, through the one
//! repeat runner, with the CSV trail and the final report.

use super::fetch::Tick;
use super::{Ready, Sources, tick};
use crate::cli::output::Output;
use crate::cli::repeat::{self, RunOutput, Schedule};
use scope::error::{Result, ScopeError};
use scope::metrics::Sample;
use scope::watch::{Event, Snapshot, Watchlist, diff};
use std::io::Write as _;
use std::path::Path;

/// The CSV header of a watchlist run (one row per cell per tick).
pub fn csv_header() -> &'static str {
    "ts,target,metric,value,op,threshold,status,source,error,detail\n"
}

/// The CSV rows of one tick: every cell of every target. `source` is
/// `preset` for a preset check and the metric's group for a rule.
pub fn csv_rows(at: &str, snapshot: &Snapshot) -> String {
    let mut rows = String::new();
    for t in &snapshot.targets {
        for c in &t.cells {
            let (metric, op, threshold, source) = match &c.rule {
                Some(r) => (
                    r.metric.name().to_string(),
                    r.op.symbol().to_string(),
                    r.value.to_string(),
                    format!("{:?}", r.metric.group()).to_lowercase(),
                ),
                None => (
                    c.label.clone(),
                    String::new(),
                    String::new(),
                    "preset".into(),
                ),
            };
            let value = c.value.map(|v| v.to_string()).unwrap_or_default();
            let error = match &c.status {
                scope::watch::RuleStatus::Error(e) => e.as_str(),
                _ => "",
            };
            let fields = [
                at,
                &t.target,
                &metric,
                &value,
                &op,
                &threshold,
                c.status.name(),
                &source,
                error,
                // A preset check's text: it keeps the numbers of a check
                // that passed, too.
                c.detail.as_deref().unwrap_or(""),
            ];
            let cells: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
            rows.push_str(&cells.join(","));
            rows.push('\n');
        }
    }
    rows
}

/// A CSV field, quoted when it holds a comma, a quote or a line break.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The report of a run: one `market summary --report` section per target
/// with a summary, then every cell's last value.
pub fn report_text(w: &Watchlist, last: &Tick) -> String {
    let mut md = format!("# Watchlist Report: {}\n\n", w.name);
    for (t, summary) in w.targets.iter().zip(&last.summaries) {
        if let (Some(summary), Some(venue)) = (summary, &t.venue) {
            md.push_str(&crate::cli::market::market_summary_to_markdown(
                summary, venue, &t.id,
            ));
            md.push('\n');
        }
    }
    md.push_str(
        "## Last values\n\n| Target | Check | Value | Status | Detail |\n|---|---|---|---|---|\n",
    );
    for t in &last.snapshot.targets {
        for c in &t.cells {
            let value = c.value.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
            let detail = c.status.message().unwrap_or("").replace('|', "\\|");
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                t.target,
                c.label.replace('|', "\\|"),
                value,
                c.status.name(),
                detail
            ));
        }
    }
    md
}

/// Writes the report of `last` to `path`.
pub fn write_report(path: &Path, w: &Watchlist, last: &Tick) -> Result<()> {
    std::fs::write(path, report_text(w, last))
        .map_err(|e| ScopeError::io_context(e, format!("cannot write {}", path.display())))
}

/// The run's schedule; a run with no duration goes until cancelled.
pub fn schedule(w: &Watchlist) -> Result<Schedule> {
    match w.duration {
        Some(d) => Schedule::new(w.interval, d),
        None => Schedule::until_cancelled(w.interval),
    }
}

/// Runs `ready` on its schedule. After each tick, `on_tick` gets the run
/// number, the tick and the events since the tick before. The CSV gets one
/// row per cell per tick; at the end of the duration, the report is
/// written. A CSV or report write error ends the run and is returned.
///
/// Returns the last tick.
pub async fn run(
    ready: &Ready,
    src: Sources<'_>,
    out: &Output,
    mut on_tick: impl FnMut(u64, &Tick, &[Event]),
) -> Result<Tick> {
    let w = &ready.list;
    if let Some(path) = &w.outputs.csv {
        std::fs::write(path, csv_header())
            .map_err(|e| ScopeError::io_context(e, format!("cannot write {}", path.display())))?;
    }
    // The step closure is called once per tick and its future outlives
    // the call, so the state it changes is shared through cells.
    let prev = std::cell::RefCell::new(Snapshot::default());
    let on_tick = std::cell::RefCell::new(&mut on_tick);
    let (prev, on_tick) = (&prev, &on_tick);
    let last = repeat::run(&schedule(w)?, &[], None, out, |n| async move {
        let t = tick(ready, src, out).await;
        let events = diff(&prev.borrow(), &t.snapshot);
        if let Some(path) = &w.outputs.csv {
            let at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            append(path, &csv_rows(&at, &t.snapshot))?;
        }
        (on_tick.borrow_mut())(n, &t, &events);
        *prev.borrow_mut() = t.snapshot.clone();
        let failed: Vec<String> = events
            .iter()
            .filter(|e| matches!(e, Event::BreachStarted { .. }))
            .map(Event::banner)
            .collect();
        Ok(RunOutput {
            sample: Sample {
                values: Vec::new(),
                healthy: t.snapshot.breaches() == 0,
                failed_checks: failed,
            },
            snapshot: String::new(),
            view: None,
            value: t,
        })
    })
    .await?
    .unwrap_or_default();
    if let Some(path) = &w.outputs.report {
        write_report(path, w, &last)?;
        out.note_saved("Report", path.display());
    }
    Ok(last)
}

fn append(path: &Path, rows: &str) -> Result<()> {
    let context = || format!("cannot append to {}", path.display());
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| ScopeError::io_context(e, context()))?;
    file.write_all(rows.as_bytes())
        .map_err(|e| ScopeError::io_context(e, context()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::chains::mocks::MockClientFactory;
    use scope::watch::{Cell, Rule, RuleStatus, Target, TargetKind, TargetSnapshot};
    use std::time::Duration;

    #[test]
    fn csv_rows_hold_every_cell_and_quote_commas() {
        let snapshot = Snapshot {
            targets: vec![TargetSnapshot::new(
                "USDC@binance",
                vec![
                    Cell {
                        label: "Spread".into(),
                        rule: None,
                        value: Some(3.4),
                        status: RuleStatus::Fail("Spread: 3.4%, max 3%".into()),
                        detail: Some("Spread: 3.4%, max 3%".into()),
                    },
                    // A passing check keeps its value and text (the 0.8.0
                    // smoke test found them empty).
                    Cell {
                        label: "Bid depth".into(),
                        rule: None,
                        value: Some(4200.0),
                        status: RuleStatus::Pass,
                        detail: Some("Bid depth: 4200 USDT (min 3000)".into()),
                    },
                    Cell {
                        label: "holders >= 10".into(),
                        rule: Some(Rule::parse("holders >= 10").unwrap()),
                        value: None,
                        status: RuleStatus::Error("timeout, retry".into()),
                        detail: None,
                    },
                ],
            )],
        };
        let rows = csv_rows("2026-10-16T12:00:00Z", &snapshot);
        let lines: Vec<&str> = rows.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "2026-10-16T12:00:00Z,USDC@binance,Spread,3.4,,,fail,preset,,\"Spread: 3.4%, max 3%\""
        );
        assert_eq!(
            lines[1],
            "2026-10-16T12:00:00Z,USDC@binance,Bid depth,4200,,,pass,preset,,Bid depth: 4200 USDT (min 3000)"
        );
        assert_eq!(
            lines[2],
            "2026-10-16T12:00:00Z,USDC@binance,holders,,>=,10,error,token,\"timeout, retry\","
        );
        assert_eq!(csv_header().split(',').count(), 10);
    }

    #[tokio::test]
    async fn a_run_ticks_writes_the_csv_and_the_report() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("w.csv");
        let report = dir.path().join("w.md");
        let mut t = Target::new(
            TargetKind::Token,
            "0x1234567890123456789012345678901234567890",
        );
        t.rules = vec![Rule::parse("price_usd > 0").unwrap()];
        let mut w = Watchlist::new("test");
        // Room for several ticks even when a tick is slow (a coverage build
        // runs instrumented): 15 ms left room for one, and the test needs
        // two.
        w.interval = Duration::from_millis(50);
        w.duration = Some(Duration::from_millis(500));
        w.outputs.csv = Some(csv.clone());
        w.outputs.report = Some(report.clone());
        w.targets = vec![t];
        let ready = Ready {
            list: w,
            contracts: vec![None],
        };
        let factory = MockClientFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let mut ticks = Vec::new();
        let (out, _cap) = Output::capture();
        let last = run(&ready, src, &out, |n, _, events| {
            ticks.push((n, events.len()))
        })
        .await
        .unwrap();
        // The tick count depends on timing; the runs are numbered from 1.
        let runs: Vec<u64> = ticks.iter().map(|t| t.0).collect();
        assert!(runs.len() >= 2, "{runs:?}");
        assert_eq!(runs, (1..=runs.len() as u64).collect::<Vec<_>>());
        assert!(ticks.iter().all(|t| t.1 == 0), "a pass starts no event");
        assert_eq!(last.snapshot.targets.len(), 1);
        let text = std::fs::read_to_string(&csv).unwrap();
        assert_eq!(
            text.lines().count(),
            1 + runs.len(),
            "header and one row per tick"
        );
        let md = std::fs::read_to_string(&report).unwrap();
        assert!(md.contains("# Watchlist Report: test"), "{md}");
        assert!(md.contains("| price_usd > 0 |"), "{md}");
    }

    #[test]
    fn the_report_has_a_market_section_per_target_with_a_summary_and_a_venue() {
        use scope::market::{BookSource, HealthThresholds, MarketSummary, OrderBook};
        let level = |price, quantity| scope::market::OrderBookLevel { price, quantity };
        let book = OrderBook {
            pair: "USDC/USDT".into(),
            source: BookSource::Exchange,
            bids: vec![level(0.999, 5_000.0)],
            asks: vec![level(1.001, 5_000.0)],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let mut on_venue = Target::new(TargetKind::Pair, "USDC");
        on_venue.venue = Some("binance".into());
        // A summary with no venue to name has no section.
        let no_venue = Target::new(TargetKind::Pair, "DAI");
        let mut w = Watchlist::new("pegs");
        w.targets = vec![on_venue, no_venue];
        let last = Tick {
            snapshot: Snapshot {
                targets: vec![TargetSnapshot::new(
                    "USDC@binance",
                    vec![Cell {
                        label: "a|b".into(),
                        rule: None,
                        value: None,
                        status: RuleStatus::Error("x|y".into()),
                        detail: None,
                    }],
                )],
            },
            summaries: vec![Some(summary.clone()), Some(summary)],
        };
        let md = report_text(&w, &last);
        assert!(md.starts_with("# Watchlist Report: pegs\n\n"), "{md}");
        assert_eq!(md.matches("# Market Health Report:").count(), 1, "{md}");
        let section = md.find("# Market Health Report: USDC").unwrap();
        assert!(section < md.find("## Last values").unwrap(), "{md}");
        assert!(!md.contains("Market Health Report: DAI"), "{md}");
        // A pipe in a label or an error would split the table cell.
        assert!(
            md.contains("| USDC@binance | a\\|b | - | error | x\\|y |"),
            "{md}"
        );
    }

    #[tokio::test]
    async fn a_report_that_cannot_be_written_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Watchlist::new("test");
        w.interval = Duration::from_millis(5);
        w.duration = Some(Duration::from_millis(1));
        // A folder cannot be written as a file.
        w.outputs.report = Some(dir.path().to_path_buf());
        let ready = Ready {
            list: w,
            contracts: vec![],
        };
        let factory = MockClientFactory::new();
        let config = scope::config::Config::default();
        let src = Sources {
            config: &config,
            clients: &factory,
        };
        let (out, _cap) = Output::capture();
        let err = run(&ready, src, &out, |_, _, _| {}).await.unwrap_err();
        assert!(err.to_string().contains("cannot write"), "{err}");
    }
}
