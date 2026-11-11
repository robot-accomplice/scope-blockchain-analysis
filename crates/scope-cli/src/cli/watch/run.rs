//! A watchlist run: ticks on the watchlist's schedule, through the one
//! repeat runner, with the CSV trail and the final report.

use super::deviation;
use super::fetch::Tick;
use super::{Ready, Sources, tick};
use crate::cli::csv::csv_field;
use crate::cli::output::Output;
use crate::cli::repeat::{self, RunOutput, Schedule};
use scope::error::{Result, ScopeError};
use scope::metrics::Sample;
use scope::watch::{Cell, Event, History, RuleStatus, Snapshot, Watchlist, diff};
use std::io::Write as _;
use std::path::Path;

/// The CSV header of a watchlist run (one row per cell per tick).
pub fn csv_header() -> &'static str {
    "ts,target,metric,value,op,threshold,status,source,error,detail\n"
}

/// One cell of a row as a record: the one source of the CSV fields and of
/// the JSON cell of `scope watch --format json`, so the two outputs agree.
/// `source` is `preset` for a preset check and the metric's group for a
/// rule.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CellRecord {
    pub metric: String,
    pub op: Option<String>,
    pub threshold: Option<f64>,
    pub value: Option<f64>,
    pub status: &'static str,
    pub source: String,
    pub error: Option<String>,
    pub detail: Option<String>,
}

/// The record of `cell`.
pub fn cell_record(cell: &Cell) -> CellRecord {
    let (metric, op, threshold, source) = match &cell.rule {
        Some(r) => (
            r.metric.name().to_string(),
            Some(r.op.symbol().to_string()),
            Some(r.value),
            format!("{:?}", r.metric.group()).to_lowercase(),
        ),
        None => (cell.label.clone(), None, None, "preset".into()),
    };
    let error = match &cell.status {
        RuleStatus::Error(e) => Some(e.clone()),
        _ => None,
    };
    CellRecord {
        metric,
        op,
        threshold,
        value: cell.value,
        status: cell.status.name(),
        source,
        error,
        // A preset check's text: it keeps the numbers of a check that
        // passed, too.
        detail: cell.detail.clone(),
    }
}

/// The CSV rows of one tick: every cell of every target.
pub fn csv_rows(at: &str, snapshot: &Snapshot) -> String {
    let mut rows = String::new();
    for t in &snapshot.targets {
        for c in &t.cells {
            let r = cell_record(c);
            let number = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_default();
            let fields = [
                at,
                &t.target,
                &r.metric,
                &number(r.value),
                r.op.as_deref().unwrap_or(""),
                &number(r.threshold),
                r.status,
                &r.source,
                r.error.as_deref().unwrap_or(""),
                r.detail.as_deref().unwrap_or(""),
            ];
            let cells: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
            rows.push_str(&cells.join(","));
            rows.push('\n');
        }
    }
    rows
}

/// The report of a run: one `market summary --report` section per row
/// with a summary (one per venue), then every cell's last value.
pub fn report_text(w: &Watchlist, last: &Tick) -> String {
    let mut md = format!("# Watchlist Report: {}\n\n", w.name);
    // The tick's rows, and so its summaries, follow each target's rows.
    let rows = w
        .targets
        .iter()
        .flat_map(|t| t.rows().into_iter().map(move |venue| (t, venue)));
    for ((t, venue), summary) in rows.zip(&last.summaries) {
        if let (Some(summary), Some(venue)) = (summary, venue) {
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

/// Starts the CSV file of a run: a new file with the header. With
/// `append`, a file that has rows keeps them and gets no second header.
///
/// # Errors
///
/// The file cannot be written.
pub fn start_csv(path: &Path, append: bool) -> Result<()> {
    let has_rows = append && std::fs::metadata(path).is_ok_and(|m| m.len() > 0);
    let started = if has_rows {
        // Fails now, not at the first tick, when the file cannot be written.
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map(drop)
    } else {
        std::fs::write(path, csv_header())
    };
    started.map_err(|e| ScopeError::io_context(e, format!("cannot write {}", path.display())))
}

/// One change of the venues of one target in a running list: the venue
/// editor of the grid asks for it, the run applies it between two ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VenueChange {
    /// The grid's number of the edit. The run gives it back with the change
    /// that it applied, so the grid knows which edit took effect.
    pub seq: u64,
    /// The index of the target in the list.
    pub target: usize,
    /// The venues the target has from the next tick on.
    pub venues: Vec<String>,
}

/// The changes the grid has asked for and the run has not taken yet. The
/// grid and the run share it (the same pattern as the progress hook). A
/// second change of one target replaces the first.
#[derive(Debug, Clone, Default)]
pub struct VenueQueue(std::sync::Arc<std::sync::Mutex<Vec<VenueChange>>>);

impl VenueQueue {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<VenueChange>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Asks for `change`. It replaces a change of the same target that
    /// waits.
    pub fn submit(&self, change: VenueChange) {
        let mut queue = self.lock();
        queue.retain(|c| c.target != change.target);
        queue.push(change);
    }

    /// Takes back the change `seq` if the run has not taken it. Returns
    /// whether it did.
    pub fn withdraw(&self, seq: u64) -> bool {
        let mut queue = self.lock();
        let before = queue.len();
        queue.retain(|c| c.seq != seq);
        queue.len() != before
    }

    /// Takes all the changes that wait.
    pub fn take(&self) -> Vec<VenueChange> {
        std::mem::take(&mut *self.lock())
    }
}

/// The list `ready` has after `changes`, and `history` after them: a venue
/// that stays keeps its memory, a venue that goes is forgotten, a venue
/// that comes has none ([`History::retarget`]). The contracts stay, as the
/// targets stay. A change of a target the list does not have is dropped;
/// the rest are returned as applied.
pub fn apply_venue_changes(
    ready: &Ready,
    history: &mut History,
    changes: &[VenueChange],
) -> (Ready, Vec<VenueChange>) {
    let mut next = ready.clone();
    let mut applied = Vec::new();
    for change in changes {
        if let Some(target) = next.list.targets.get_mut(change.target) {
            target.venues.clone_from(&change.venues);
            applied.push(change.clone());
        }
    }
    history.retarget(
        &scope::watch::market_keys(&ready.list.targets),
        &scope::watch::market_keys(&next.list.targets),
    );
    (next, applied)
}

/// How a run takes venue changes: the queue to read at the start of each
/// tick, and what to tell when some applied.
pub struct Edits<'a> {
    pub queue: &'a VenueQueue,
    /// Called at the start of the tick that applied changes: the run number
    /// of that tick, the list from then on, and the changes.
    pub on_applied: &'a mut dyn FnMut(u64, &Ready, &[VenueChange]),
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
    on_tick: impl FnMut(u64, &Tick, &[Event]),
) -> Result<Tick> {
    run_editable(ready, src, out, None, on_tick).await
}

/// [`run`], with venue changes from `edits` applied between ticks. The CSV
/// is started once and never rewritten by a change; the report is made from
/// the list as it is at the end.
pub async fn run_editable(
    ready: &Ready,
    src: Sources<'_>,
    out: &Output,
    edits: Option<Edits<'_>>,
    mut on_tick: impl FnMut(u64, &Tick, &[Event]),
) -> Result<Tick> {
    if let Some(path) = &ready.list.outputs.csv {
        start_csv(path, false)?;
    }
    let (last, end) = run_ticks_edited(
        ready,
        src,
        out,
        History::default(),
        edits,
        |n, _, t, events, _| on_tick(n, t, events),
    )
    .await?;
    if let Some(path) = &end.list.outputs.report {
        write_report(path, &end.list, &last)?;
        out.note_saved("Report", path.display());
    }
    Ok(last)
}

/// The loop of [`run`]: the ticks on the schedule, with a CSV row set per
/// tick when the watchlist has a CSV file (the caller has started it with
/// [`start_csv`]). `on_tick` also gets the time written in the CSV rows,
/// and runs after the append, so the file and the callback agree. No
/// report is written.
///
/// Returns the last tick.
pub async fn run_ticks(
    ready: &Ready,
    src: Sources<'_>,
    out: &Output,
    start: History,
    on_tick: impl FnMut(u64, &str, &Tick, &[Event], &History),
) -> Result<Tick> {
    Ok(run_ticks_edited(ready, src, out, start, None, on_tick)
        .await?
        .0)
}

/// [`run_ticks`], with the venue changes of `edits` applied at the start of
/// a tick, never in the middle of one: a tick reads one list. The changes
/// are taken, applied to the run's own copy of `ready` and to the memory
/// ([`apply_venue_changes`]), and told to `on_applied`, before the tick
/// reads.
///
/// Returns the last tick and the list as it is at the end.
pub async fn run_ticks_edited(
    ready: &Ready,
    src: Sources<'_>,
    out: &Output,
    start: History,
    edits: Option<Edits<'_>>,
    mut on_tick: impl FnMut(u64, &str, &Tick, &[Event], &History),
) -> Result<(Tick, Ready)> {
    // The step closure is called once per tick and its future outlives
    // the call, so the state it changes is shared through cells.
    let prev = std::cell::RefCell::new(Snapshot::default());
    let history = std::cell::RefCell::new(start);
    let live = std::cell::RefCell::new(std::rc::Rc::new(ready.clone()));
    let edits = edits.map(|e| (e.queue, std::cell::RefCell::new(e.on_applied)));
    let on_tick = std::cell::RefCell::new(&mut on_tick);
    let (prev, history, live, edits, on_tick) = (&prev, &history, &live, &edits, &on_tick);
    let last = repeat::run(&schedule(&ready.list)?, &[], None, out, |n| async move {
        if let Some((queue, on_applied)) = edits {
            let changes = queue.take();
            if !changes.is_empty() {
                let mut memory = history.take();
                let (next, applied) = apply_venue_changes(&live.borrow(), &mut memory, &changes);
                history.replace(memory);
                *live.borrow_mut() = std::rc::Rc::new(next);
                (on_applied.borrow_mut())(n, &live.borrow(), &applied);
            }
        }
        let ready = live.borrow().clone();
        let w = &ready.list;
        // Taken out for the tick, so no borrow is held across its awaits.
        let mut memory = history.take();
        let t = tick(&ready, src, out, &mut memory).await;
        history.replace(memory);
        let events = diff(&prev.borrow(), &t.snapshot);
        let at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        if let Some(path) = &w.outputs.csv {
            append(path, &tick_csv_rows(&at, w, &t))?;
        }
        (on_tick.borrow_mut())(n, &at, &t, &events, &history.borrow());
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
            venues: None,
            value: t,
        })
    })
    .await?
    .unwrap_or_default();
    let end = live.borrow().as_ref().clone();
    Ok((last, end))
}

/// The CSV rows of one tick: every cell of every target ([`csv_rows`]),
/// then one `peg_deviation_bps` row for each market row of a target on two
/// or more venues ([`chart_csv_rows`]). The file and the headless CSV
/// format both write this, so they stay the same bytes.
pub fn tick_csv_rows(at: &str, w: &Watchlist, tick: &Tick) -> String {
    let mut rows = csv_rows(at, &tick.snapshot);
    rows.push_str(&chart_csv_rows(at, w, tick));
    rows
}

/// One row for each market row of a target on two or more venues, in the
/// columns of the cell rows: `metric` is `peg_deviation_bps`, `source` is
/// `chart`, `status` is the legend glyph in words, and `detail` holds the
/// band and the verdict, or why there is no value. The row exists when the
/// value does not, so a gap in the chart can be told from a missing row.
/// The deviation is the one `MarketSummary::mid_peg_deviation_bps` the chart
/// draws, so a grid chart can be rebuilt from this record.
///
/// The rows of the tick line up with the rows of the watchlist's targets
/// by index, as the report reads them.
pub fn chart_csv_rows(at: &str, w: &Watchlist, tick: &Tick) -> String {
    let rows = w
        .targets
        .iter()
        .flat_map(|t| t.rows().into_iter().map(move |venue| (t, venue)));
    let readings = tick.snapshot.targets.iter().zip(&tick.summaries);
    let mut text = String::new();
    for ((t, venue), (row, summary)) in rows.zip(readings) {
        if venue.is_none() || t.venues.len() < 2 {
            continue;
        }
        let r = deviation::reading(summary.as_ref());
        let value = r.bps.map(|v| v.to_string()).unwrap_or_default();
        let fields = [
            at,
            &row.target,
            deviation::METRIC,
            &value,
            "",
            "",
            r.status.name(),
            deviation::SOURCE,
            "",
            &r.detail,
        ];
        let cells: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        text.push_str(&cells.join(","));
        text.push('\n');
    }
    text
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
        on_venue.venues = vec!["binance".into()];
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
            ..Tick::default()
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

    // Design 2026-11-04, section 2.6: a target on two venues gives a
    // market section and a set of CSV rows per venue.
    #[test]
    fn two_venues_give_two_sections_and_two_labelled_row_sets() {
        use scope::market::{BookSource, HealthThresholds, MarketSummary, OrderBook};
        let level = |price, quantity| scope::market::OrderBookLevel { price, quantity };
        let book = OrderBook {
            pair: "USDN/USDT".into(),
            source: BookSource::Exchange,
            bids: vec![level(0.999, 5_000.0)],
            asks: vec![level(1.001, 5_000.0)],
        };
        let summary = MarketSummary::from_order_book(&book, &HealthThresholds::default(), None);
        let mut t = Target::new(TargetKind::Pair, "USDN");
        t.venues = vec!["weex".into(), "binance".into()];
        t.rules = vec![scope::watch::Rule::parse("spread_pct < 1").unwrap()];
        let mut w = Watchlist::new("pegs");
        w.targets = vec![t];
        let row = |venue: &str| {
            TargetSnapshot::new(
                format!("USDN@{venue}"),
                vec![Cell {
                    label: "spread_pct < 1".into(),
                    rule: scope::watch::Rule::parse("spread_pct < 1").ok(),
                    value: Some(0.2),
                    status: RuleStatus::Pass,
                    detail: None,
                }],
            )
            .on_venue(Some(venue))
        };
        let last = Tick {
            snapshot: Snapshot {
                targets: vec![row("weex"), row("binance")],
            },
            summaries: vec![Some(summary.clone()), Some(summary)],
            ..Tick::default()
        };
        let md = report_text(&w, &last);
        assert_eq!(
            md.matches("# Market Health Report: USDN").count(),
            2,
            "{md}"
        );
        assert!(
            md.contains("| USDN@weex |") && md.contains("| USDN@binance |"),
            "{md}"
        );
        let csv = csv_rows("t0", &last.snapshot);
        let targets: Vec<&str> = csv.lines().map(|l| l.split(',').nth(1).unwrap()).collect();
        assert_eq!(targets, ["USDN@weex", "USDN@binance"]);
    }

    // Q-D7 b. Why: the owner chose a record from which a grid chart can be
    // rebuilt. The row must carry the number the chart draws, exist when the
    // number does not, and skip a target on one venue (it has no chart).
    #[test]
    fn two_venues_give_a_deviation_row_each_and_one_venue_gives_none() {
        use scope::market::{BookSource, HealthThresholds, MarketSummary, OrderBook};
        let level = |price, quantity| scope::market::OrderBookLevel { price, quantity };
        let summary_at = |bid, ask| {
            let book = OrderBook {
                pair: "USDN/USDT".into(),
                source: BookSource::Exchange,
                bids: vec![level(bid, 5_000.0)],
                asks: vec![level(ask, 5_000.0)],
            };
            MarketSummary::from_order_book(&book, &HealthThresholds::default(), None)
        };
        let weex = summary_at(1.0011, 1.0013); // mid 1.0012: +12 bps, past the 10 bps band, inside the 50 bps one
        let mut many = Target::new(TargetKind::Pair, "USDN");
        many.venues = vec!["weex".into(), "binance".into()];
        many.rules = vec![scope::watch::Rule::parse("spread_pct < 1").unwrap()];
        let mut one = Target::new(TargetKind::Pair, "USDC");
        one.venues = vec!["okx".into()];
        one.rules = many.rules.clone();
        let mut w = Watchlist::new("pegs");
        w.targets = vec![many, one];
        let row =
            |label: &str, venue: &str| TargetSnapshot::new(label, Vec::new()).on_venue(Some(venue));
        let tick = Tick {
            snapshot: Snapshot {
                targets: vec![
                    row("USDN@weex", "weex"),
                    row("USDN@binance", "binance"),
                    row("USDC@okx", "okx"),
                ],
            },
            // The binance read failed: no summary.
            summaries: vec![Some(weex.clone()), None, Some(weex.clone())],
            ..Default::default()
        };
        let rows = chart_csv_rows("t0", &w, &tick);
        let fields: Vec<Vec<&str>> = rows
            .lines()
            .map(|l| l.splitn(10, ',').collect::<Vec<_>>())
            .collect();
        assert_eq!(
            fields.len(),
            2,
            "one row per market row of the 2-venue target: {rows}"
        );
        let bps = fields[0][3].parse::<f64>().unwrap();
        assert_eq!(
            Some(bps),
            weex.mid_peg_deviation_bps(),
            "the chart's number"
        );
        assert!((bps - 12.0).abs() < 1e-6, "{bps}");
        assert_eq!(
            fields[0][..3],
            ["t0", "USDN@weex", "peg_deviation_bps"],
            "{rows}"
        );
        assert_eq!(fields[0][6..8], ["warn", "chart"], "slight: {rows}");
        assert!(fields[0][9].contains("band ±10.0 bps"), "{rows}");
        // The failed venue keeps its row: a gap is not a missing row.
        assert_eq!(fields[1][1..4], ["USDN@binance", "peg_deviation_bps", ""]);
        assert_eq!(fields[1][6], "n/a");
        assert_eq!(fields[1][9], "no mid price");
        // The tick's rows are the cell rows, then the chart rows.
        let all = tick_csv_rows("t0", &w, &tick);
        assert!(all.starts_with(&csv_rows("t0", &tick.snapshot)));
        assert!(all.ends_with(&rows));
    }

    // Why: the legend glyph and the CSV status are one rule. If the CSV
    // classified on its own, the chart and the record would disagree.
    #[test]
    fn the_csv_status_of_a_deviation_is_the_legend_glyph() {
        use scope::market::health::PegBands;
        let bands = PegBands::new(1.0, 0.001);
        for (bps, name) in [(3.0, "pass"), (-30.0, "warn"), (80.0, "fail")] {
            let status = deviation::status(Some(&bands), Some(bps), None);
            assert_eq!(status.name(), name, "{bps} bps");
        }
        let none = deviation::status(Some(&bands), None, None);
        assert_eq!(none.name(), "n/a");
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
