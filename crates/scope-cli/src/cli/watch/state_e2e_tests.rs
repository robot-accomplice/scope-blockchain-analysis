//! End to end: two `scope watch --once` runs of one list, as cron makes
//! them (design 2026-11-11, section 3.3, tests 13 to 17, 10 and 16).
//!
//! The venue is a test transport on a paused clock (see
//! `fetch::tests::LatencyHttp`). Each run has its own state folder inside a
//! temporary folder, never the real scope home.

use super::fetch::tests::{Answer, LATENCY, LatencyFactory};
use super::headless::{Context, EXIT_BREACH, EXIT_PASS, run};
use crate::cli::output::Output;
use crate::cli::{Cli, Commands};
use clap::Parser;
use scope::config::Config;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

const MIN: Duration = Duration::from_secs(60);

/// A list with one rule on the volume change.
const VOLUME_LIST: &str = "targets:\n  - kind: pair\n    id: USDC\n    venue: lat-a\n    rules: [\"volume_change_pct <= 0.5\"]\n";
/// A list with one rule on the largest trade.
const TRADE_LIST: &str = "targets:\n  - kind: pair\n    id: USDC\n    venue: lat-a\n    rules: [\"largest_trade < 1000000\"]\n";

struct Dir {
    dir: tempfile::TempDir,
}

impl Dir {
    fn new(list: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pegs.yaml"), list).unwrap();
        Self { dir }
    }

    fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// The one state file the runs wrote, if any.
    fn state_file(&self) -> Option<PathBuf> {
        std::fs::read_dir(self.state_dir())
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "json"))
    }
}

struct Run {
    code: i32,
    records: Vec<Value>,
    err: String,
}

impl Run {
    fn of(&self, kind: &str) -> &Value {
        self.records.iter().find(|r| r["type"] == kind).unwrap()
    }

    /// The value of the first cell of the first row of the tick.
    fn value(&self) -> Option<f64> {
        self.of("tick")["rows"][0]["cells"][0]["value"].as_f64()
    }

    fn status(&self) -> &str {
        self.of("tick")["rows"][0]["cells"][0]["status"]
            .as_str()
            .unwrap()
    }
}

/// `scope watch <dir>/pegs.yaml --once --format json <flags>`.
async fn once(d: &Dir, factory: &LatencyFactory, flags: &[&str]) -> Run {
    let list = d.dir.path().join("pegs.yaml").display().to_string();
    let mut argv = vec!["scope", "watch", &list, "--once", "--format", "json"];
    argv.extend(flags);
    let Commands::Watch(args) = Cli::try_parse_from(&argv).unwrap().command else {
        panic!("not watch")
    };
    let (out, cap) = Output::capture();
    let config = Config::default();
    let ctx = Context {
        config: &config,
        clients: factory,
        out: &out,
        dir: d.dir.path().to_path_buf(),
        state_dir: d.state_dir(),
        venues: vec!["lat-a".into()],
    };
    let code = run(&args, &ctx, std::future::pending()).await;
    let records = cap
        .out()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    Run {
        code,
        records,
        err: cap.err(),
    }
}

fn factory() -> LatencyFactory {
    LatencyFactory::new(&[("lat-a", Answer::After(LATENCY))])
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap()
}

/// Rewrites the times of the one entry in the state file: when it was last
/// written, and the time of the newest trade it saw (both before now).
fn age_state(d: &Dir, updated_ago: Duration, last_trade_ago: Duration) {
    let path = d.state_file().expect("a state file");
    let mut file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let now = now_ms();
    file["markets"][0]["updated_ms"] = (now - ms(updated_ago)).into();
    file["markets"][0]["last_trade_ms"] = (now - ms(last_trade_ago)).into();
    std::fs::write(path, file.to_string()).unwrap();
}

/// The reason for the feature (test 13). Run 1 reads a volume. Run 2, a
/// cron period later, reads 1% more. With the state, run 2 sees the change
/// and the rule `<= 0.5` breaches (exit 1). Without it, run 2 is a first
/// tick: `n/a`, and the same job passes (exit 0). The first design left
/// this case out.
#[tokio::test(start_paused = true)]
async fn a_second_once_run_sees_the_volume_change_of_the_first() {
    let factory = factory();
    let d = Dir::new(VOLUME_LIST);
    let first = once(&d, &factory, &["--every", "5m"]).await;
    assert_eq!(first.code, EXIT_PASS);
    assert_eq!(first.status(), "n/a", "a first run has no second reading");
    assert_eq!(first.of("start")["state"]["file"], "none");
    assert_eq!(first.of("end")["state_written"], true);

    let second = once(&d, &factory, &["--every", "5m"]).await;
    assert_eq!(second.code, EXIT_BREACH, "{:?}", second.records);
    let pct = second.value().unwrap();
    assert!((pct - 1.0).abs() < 1e-9, "{pct}");
    let start = &second.of("start")["state"];
    assert_eq!(start["file"], "loaded");
    assert_eq!(start["markets"][0]["basis"], "loaded");
    assert_eq!(start["markets"][0]["row"], "USDC@lat-a");
    assert_eq!(start["markets"][0]["stale_limit_ms"], ms(15 * MIN));
    assert!(start["markets"][0]["volume_24h"].as_f64().is_some());

    // Section 3.3, test 16: --no-state reads and writes nothing, so the
    // same job is a first tick and passes. The file is as it was.
    let path = d.state_file().unwrap();
    let before = std::fs::read(&path).unwrap();
    let off = once(&d, &factory, &["--every", "5m", "--no-state"]).await;
    assert_eq!(off.code, EXIT_PASS);
    assert_eq!(off.status(), "n/a");
    assert_eq!(off.of("start")["state"]["mode"], "off");
    assert_eq!(off.of("end")["state_written"], false);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

/// Test 14, in the form that the code can show. The window of a first tick
/// is one interval, and the state removes the trades that the run before
/// has already counted. A trade 4 minutes old that the last run saw is not
/// counted again with the state, and is counted without it.
#[tokio::test(start_paused = true)]
async fn the_state_keeps_a_trade_that_an_earlier_run_saw_from_counting_twice() {
    let factory = factory();
    let d = Dir::new(TRADE_LIST);
    once(&d, &factory, &["--every", "5m"]).await;
    // The last run saw a trade 3 minutes ago, 5 minutes ago it wrote.
    age_state(&d, 5 * MIN, 3 * MIN);
    factory
        .http
        .set_tape(&[(4 * MIN, 9_000.0), (2 * MIN, 2_500.0)]);

    let with = once(&d, &factory, &["--every", "5m"]).await;
    assert_eq!(with.value(), Some(2_500.0), "{:?}", with.records);
    let without = once(&d, &factory, &["--every", "5m", "--no-state"]).await;
    assert_eq!(without.value(), Some(9_000.0));
}

/// Test 15 (Q6): an entry older than 3 intervals acts as a first tick, and
/// the `start` record says `stale` with the age.
#[tokio::test(start_paused = true)]
async fn an_entry_older_than_three_intervals_is_a_first_tick_and_says_stale() {
    let factory = factory();
    let d = Dir::new(TRADE_LIST);
    once(&d, &factory, &["--every", "5m"]).await;
    age_state(&d, 16 * MIN, 3 * MIN);
    factory
        .http
        .set_tape(&[(4 * MIN, 9_000.0), (2 * MIN, 2_500.0)]);

    let run = once(&d, &factory, &["--every", "5m"]).await;
    let market = &run.of("start")["state"]["markets"][0];
    assert_eq!(market["basis"], "stale");
    let age = market["age_ms"].as_u64().unwrap();
    assert!(age >= ms(16 * MIN), "{age}");
    assert!(age > market["stale_limit_ms"].as_u64().unwrap());
    // A first tick looks back one interval (5 minutes): both trades.
    assert_eq!(run.value(), Some(9_000.0));
}

/// Test 17 (Q7): `--once --every 5m` sets the look-back window and the stale
/// limit. Without `--every` the file's interval (60 s) applies, and a trade
/// 4 minutes old is outside the window of a first tick.
#[tokio::test(start_paused = true)]
async fn once_with_every_sets_the_window_and_the_stale_limit() {
    let factory = factory();
    factory.http.set_tape(&[(4 * MIN, 9_000.0)]);
    let narrow = once(&Dir::new(TRADE_LIST), &factory, &[]).await;
    assert_eq!(narrow.value(), Some(0.0));
    assert_eq!(narrow.of("start")["state"]["stale_limit_ms"], ms(3 * MIN));

    let wide = once(&Dir::new(TRADE_LIST), &factory, &["--every", "5m"]).await;
    assert_eq!(wide.value(), Some(9_000.0));
    assert_eq!(wide.of("start")["state"]["stale_limit_ms"], ms(15 * MIN));
    assert_eq!(wide.of("start")["interval_secs"], 300);
}

/// Test 10 (Q9): the state is a cache. A folder that cannot be written
/// leaves the exit code of the checks, a note, and the exact error in the
/// `end` record. Exit 4 means the CSV or the report.
#[tokio::test(start_paused = true)]
async fn an_unwritable_state_folder_is_a_note_and_a_field_and_not_an_exit_code() {
    let factory = factory();
    let d = Dir::new(VOLUME_LIST);
    // A file where the state folder should be.
    std::fs::write(d.state_dir(), b"in the way").unwrap();
    let run = once(&d, &factory, &["--every", "5m"]).await;
    assert_eq!(run.code, EXIT_PASS);
    let end = run.of("end");
    assert_eq!(end["state_written"], false);
    let error = end["state_error"].as_str().unwrap();
    assert!(error.contains("cannot create"), "{error}");
    assert!(run.err.contains("State not saved"), "{}", run.err);
}

/// Test 9, end to end (Q12): a bad file moves aside as `.bad` with its
/// bytes, the run is a first tick, and the `start` record says why.
#[tokio::test(start_paused = true)]
async fn a_bad_state_file_moves_aside_and_the_run_goes_on() {
    let factory = factory();
    let d = Dir::new(VOLUME_LIST);
    once(&d, &factory, &["--every", "5m"]).await;
    let path = d.state_file().unwrap();
    std::fs::write(&path, b"{ this is not json").unwrap();

    let run = once(&d, &factory, &["--every", "5m"]).await;
    assert_eq!(
        run.code, EXIT_PASS,
        "a first tick passes: {:?}",
        run.records
    );
    let state = &run.of("start")["state"];
    assert_eq!(state["file"], "discarded");
    assert!(
        state["discarded"]["reason"]
            .as_str()
            .unwrap()
            .contains("not a valid state file")
    );
    let bad = PathBuf::from(state["discarded"]["bad_path"].as_str().unwrap());
    assert_eq!(std::fs::read(bad).unwrap(), b"{ this is not json");
    assert!(run.err.contains("is not usable"), "{}", run.err);
    // The run wrote a new, valid file.
    assert_eq!(run.of("end")["state_written"], true);
}

/// Why: a list with no rule on a since-last-tick metric has no memory to
/// keep. It must not create a state file or a folder.
#[tokio::test(start_paused = true)]
async fn a_list_without_a_since_last_tick_rule_reads_and_writes_no_state() {
    let factory = factory();
    let d = Dir::new(
        "targets:\n  - kind: pair\n    id: USDC\n    venue: lat-a\n    rules: [\"mid_price > 0\"]\n",
    );
    let run = once(&d, &factory, &[]).await;
    assert_eq!(run.code, EXIT_PASS);
    assert_eq!(run.of("end")["state_written"], false);
    assert!(d.state_file().is_none());
    assert!(!d.state_dir().exists());
}
