//! A run that takes venue changes between its ticks (design 2026-11-11,
//! item 2, tests 6, 10, 11 and 9 in a whole run).
//!
//! The venues are test transports on a paused clock (see
//! `fetch::tests::LatencyHttp`). The CSV and the report go to a temporary
//! folder, never the real scope home.

use super::fetch::Tick;
use super::fetch::tests::{Answer, LATENCY, LatencyFactory};
use super::run::{Edits, VenueChange, VenueQueue, run_editable, run_ticks_edited};
use super::{Ready, Sources};
use crate::cli::output::Output;
use scope::watch::{History, Rule, RuleStatus, Target, TargetKind, Watchlist};
use std::time::Duration;

/// The time from one tick to the next. More than one read at the test
/// latency, so a tick ends before the next starts.
const INTERVAL: Duration = Duration::from_secs(10);

/// The rule that makes the memory visible: its cell is `n/a` on a market's
/// first reading and a number after it.
const RULE: &str = "volume_change_pct <= 100";

fn factory() -> LatencyFactory {
    LatencyFactory::new(&[
        ("lat-a", Answer::After(LATENCY)),
        ("lat-b", Answer::After(LATENCY)),
        ("lat-c", Answer::After(LATENCY)),
    ])
}

/// One pair, USDC, on `venues`, with [`RULE`]. `ticks` ticks.
fn ready(venues: &[&str], ticks: u32) -> Ready {
    let mut t = Target::new(TargetKind::Pair, "USDC");
    t.venues = venues.iter().map(|v| (*v).to_string()).collect();
    t.rules = vec![Rule::parse(RULE).unwrap()];
    let mut list = Watchlist::new("pegs");
    list.interval = INTERVAL;
    list.duration = Some(INTERVAL * (ticks - 1));
    list.targets = vec![t];
    Ready {
        list,
        contracts: vec![None],
    }
}

fn change(seq: u64, venues: &[&str]) -> VenueChange {
    VenueChange {
        seq,
        target: 0,
        venues: venues.iter().map(|v| (*v).to_string()).collect(),
    }
}

/// The row labels of a tick.
fn labels(tick: &Tick) -> Vec<String> {
    tick.snapshot
        .targets
        .iter()
        .map(|r| r.target.clone())
        .collect()
}

/// The status of the cell of the row `label`.
fn status<'a>(tick: &'a Tick, label: &str) -> &'a RuleStatus {
    let row = tick
        .snapshot
        .targets
        .iter()
        .find(|r| r.target == label)
        .unwrap_or_else(|| panic!("no row {label}: {:?}", labels(tick)));
    &row.cells[0].status
}

fn is_first_reading(s: &RuleStatus) -> bool {
    matches!(s, RuleStatus::NotApplicable(m) if m.contains("second reading"))
}

/// Why: a change in the middle of a tick would mix two venue sets in one
/// snapshot; two edits before the tick must give one change (the last).
/// The CSV is started once and goes on with the same header, rows start and
/// stop with the labels, and no row is lost (a restart would truncate it).
/// The report reads the list as it is at the end.
#[tokio::test(start_paused = true)]
async fn a_change_applies_at_the_next_tick_and_the_csv_and_the_report_follow_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut ready = ready(&["lat-a", "lat-b"], 4);
    let csv = dir.path().join("pegs.csv");
    let report = dir.path().join("pegs.md");
    ready.list.outputs.csv = Some(csv.clone());
    ready.list.outputs.report = Some(report.clone());
    let factory = factory();
    let config = scope::config::Config::default();
    let src = Sources {
        config: &config,
        clients: &factory,
    };
    let queue = VenueQueue::default();
    let mut applied: Vec<(u64, Vec<VenueChange>, Vec<String>)> = Vec::new();
    let mut on_applied = |run: u64, r: &Ready, changes: &[VenueChange]| {
        applied.push((run, changes.to_vec(), r.list.targets[0].venues.clone()));
    };
    let mut ticks: Vec<Tick> = Vec::new();
    run_editable(
        &ready,
        src,
        &Output::capture().0,
        Some(Edits {
            queue: &queue,
            on_applied: &mut on_applied,
        }),
        |run, tick, _| {
            if run == 2 {
                // Two edits before the tick: the second replaces the first.
                queue.submit(change(1, &["lat-a"]));
                queue.submit(change(2, &["lat-a", "lat-c"]));
            }
            ticks.push(tick.clone());
        },
    )
    .await
    .unwrap();

    let seen: Vec<Vec<String>> = ticks.iter().map(labels).collect();
    assert_eq!(
        seen,
        [
            ["USDC@lat-a", "USDC@lat-b"],
            ["USDC@lat-a", "USDC@lat-b"],
            ["USDC@lat-a", "USDC@lat-c"],
            ["USDC@lat-a", "USDC@lat-c"],
        ],
        "each tick read one venue set, the new one from the third tick"
    );
    assert_eq!(applied.len(), 1, "two edits gave one change");
    let (run, changes, venues) = &applied[0];
    assert_eq!(*run, 3, "the run that took it");
    assert_eq!(changes, &[change(2, &["lat-a", "lat-c"])]);
    assert_eq!(venues, &["lat-a", "lat-c"]);

    // The kept venue read against its own last tick; the new one had a
    // first tick; the next tick of the new one has a number.
    assert!(matches!(status(&ticks[2], "USDC@lat-a"), RuleStatus::Pass));
    assert!(is_first_reading(status(&ticks[2], "USDC@lat-c")));
    assert!(matches!(status(&ticks[3], "USDC@lat-c"), RuleStatus::Pass));

    let text = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.starts_with("ts,")).count(),
        1,
        "one header"
    );
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| l.split(',').nth(2) == Some("volume_change_pct"))
        .map(|l| l.split(',').nth(1).unwrap())
        .collect();
    assert_eq!(
        rows,
        [
            "USDC@lat-a",
            "USDC@lat-b",
            "USDC@lat-a",
            "USDC@lat-b",
            "USDC@lat-a",
            "USDC@lat-c",
            "USDC@lat-a",
            "USDC@lat-c",
        ],
        "the rows of lat-b stop and the rows of lat-c start at tick 3"
    );
    let deviation: Vec<&str> = text
        .lines()
        .filter(|l| l.split(',').nth(2) == Some("peg_deviation_bps"))
        .map(|l| l.split(',').nth(1).unwrap())
        .collect();
    assert_eq!(deviation.len(), 8, "the chart rows follow the list too");
    assert_eq!(deviation.iter().filter(|l| l.ends_with("lat-c")).count(), 2);

    let md = std::fs::read_to_string(&report).unwrap();
    assert!(md.contains("| USDC@lat-c |"), "{md}");
    assert!(
        !md.contains("USDC@lat-b"),
        "the final list is lat-a, lat-c: {md}"
    );
}

/// Why: without `History::forget`, a venue that comes back after a gap
/// compares its volume with the one from before the gap and shows a false
/// jump. Here the volume is 1% higher than the old one, so the number would
/// show. The run removes `lat-b` and adds it again; its first tick after
/// that is a first tick.
#[tokio::test(start_paused = true)]
async fn a_venue_removed_and_added_again_has_a_first_tick_in_a_run() {
    let ready = ready(&["lat-a", "lat-b"], 3);
    let factory = factory();
    let config = scope::config::Config::default();
    let src = Sources {
        config: &config,
        clients: &factory,
    };
    let queue = VenueQueue::default();
    let mut on_applied = |_: u64, _: &Ready, _: &[VenueChange]| {};
    let mut ticks: Vec<Tick> = Vec::new();
    run_ticks_edited(
        &ready,
        src,
        &Output::capture().0,
        History::default(),
        Some(Edits {
            queue: &queue,
            on_applied: &mut on_applied,
        }),
        |run, _, tick, _, _| {
            match run {
                1 => queue.submit(change(1, &["lat-a"])),
                2 => queue.submit(change(2, &["lat-a", "lat-b"])),
                _ => {}
            }
            ticks.push(tick.clone());
        },
    )
    .await
    .unwrap();

    assert_eq!(labels(&ticks[1]), ["USDC@lat-a"]);
    assert_eq!(labels(&ticks[2]), ["USDC@lat-a", "USDC@lat-b"]);
    assert!(
        is_first_reading(status(&ticks[2], "USDC@lat-b")),
        "{:?}",
        status(&ticks[2], "USDC@lat-b")
    );
    assert!(
        matches!(status(&ticks[2], "USDC@lat-a"), RuleStatus::Pass),
        "the venue that stayed keeps reading against its last tick"
    );
}

/// Why: a run that is never edited must read exactly as before. The
/// headless run calls `run_ticks`, which has no edits.
#[tokio::test(start_paused = true)]
async fn a_run_with_no_edits_keeps_its_list() {
    let ready = ready(&["lat-a", "lat-b"], 2);
    let factory = factory();
    let config = scope::config::Config::default();
    let src = Sources {
        config: &config,
        clients: &factory,
    };
    let (last, end) = run_ticks_edited(
        &ready,
        src,
        &Output::capture().0,
        History::default(),
        None,
        |_, _, _, _, _| {},
    )
    .await
    .unwrap();
    assert_eq!(end, ready);
    assert_eq!(labels(&last), ["USDC@lat-a", "USDC@lat-b"]);
}
