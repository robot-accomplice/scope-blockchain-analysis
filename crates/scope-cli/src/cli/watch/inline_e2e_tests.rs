//! End to end: `scope watch <target flags>` (design 2026-11-11, section 2.3,
//! tests 4, 5, 7 and 8, and the inline state identity of section 3.2.2).
//!
//! The venue is a test transport on a paused clock (see
//! `fetch::tests::LatencyHttp`) that counts its requests. Each test has its
//! own temporary folder for saved lists and state, never the real scope
//! home.

use super::fetch::tests::{Answer, LATENCY, LatencyFactory};
use super::headless::{
    Context, EXIT_BREACH, EXIT_NOT_STARTED, EXIT_PASS, EXIT_UNKNOWN, EXIT_WRITE_ERROR, run,
};
use crate::cli::output::Output;
use crate::cli::{Cli, Commands};
use clap::Parser;
use scope::config::Config;
use serde_json::Value;

struct Run {
    code: i32,
    records: Vec<Value>,
    err: String,
}

impl Run {
    fn of(&self, kind: &str) -> &Value {
        self.records.iter().find(|r| r["type"] == kind).unwrap()
    }
}

/// `scope watch <args>` with a test folder, the test venue `lat-a` and the
/// `config`. `args` is split on spaces.
async fn watch(
    dir: &tempfile::TempDir,
    factory: &LatencyFactory,
    config: &Config,
    args: &str,
) -> Run {
    let words: Vec<String> = std::iter::once("scope".to_string())
        .chain(["watch".to_string()])
        .chain(args.split_whitespace().map(str::to_string))
        .collect();
    let Commands::Watch(parsed) = Cli::try_parse_from(&words).unwrap().command else {
        panic!("not watch")
    };
    let (out, cap) = Output::capture();
    let ctx = Context {
        config,
        clients: factory,
        out: &out,
        dir: dir.path().join("lists"),
        state_dir: dir.path().join("state"),
        venues: vec!["lat-a".into()],
        argv: words,
    };
    let code = run(&parsed, &ctx, std::future::pending()).await;
    let records = cap
        .out()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Run {
        code,
        records,
        err: cap.err(),
    }
}

fn setup(venue: Answer) -> (tempfile::TempDir, LatencyFactory) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("lists")).unwrap();
    (dir, LatencyFactory::new(&[("lat-a", venue)]))
}

fn up() -> Answer {
    Answer::After(LATENCY)
}

const ONCE: &str = "--once --format json --preset none";

/// Why: the codes are the interface of the command. Each of 0, 1, 2 and 4
/// has a test with an inline target.
#[tokio::test(start_paused = true)]
async fn an_inline_target_exits_0_1_2_and_4() {
    let config = Config::default();

    let (dir, factory) = setup(up());
    let pass = watch(
        &dir,
        &factory,
        &config,
        &format!("USDC --venue lat-a {ONCE} --rule mid_price>0"),
    )
    .await;
    assert_eq!(pass.code, EXIT_PASS, "{}", pass.err);
    assert_eq!(pass.of("tick")["counts"]["passed"], 1);

    let breach = watch(
        &dir,
        &factory,
        &config,
        &format!("USDC --venue lat-a {ONCE} --no-state --rule mid_price<0.5"),
    )
    .await;
    assert_eq!(breach.code, EXIT_BREACH);

    let (dir, factory) = setup(Answer::Fails);
    let unknown = watch(
        &dir,
        &factory,
        &config,
        &format!("USDC --venue lat-a {ONCE} --rule mid_price>0"),
    )
    .await;
    assert_eq!(unknown.code, EXIT_UNKNOWN, "{}", unknown.err);

    let (dir, factory) = setup(up());
    let csv = dir.path().join("no-such-folder").join("t.csv");
    let write = watch(
        &dir,
        &factory,
        &config,
        &format!(
            "USDC --venue lat-a {ONCE} --rule mid_price>0 --csv {}",
            csv.display()
        ),
    )
    .await;
    assert_eq!(write.code, EXIT_WRITE_ERROR);
    assert_eq!(
        factory.http.started(),
        0,
        "the CSV is checked before a read"
    );
}

/// Why: a cron job fails on its first line and not on its tenth. Each input
/// error is exit code 3 and the transport counts zero requests.
#[tokio::test(start_paused = true)]
async fn each_input_error_exits_3_and_makes_no_request() {
    let config = Config::default();
    let cases = [
        ("USDC --venue nowhere --once", "nowhere"),
        ("USDC --venue lat-a --preset peg --once", "unknown preset"),
        ("USDC --preset market-health --once", "need a venue"),
        (
            "USDC --pair USDC_USDT --chain base --once",
            "--pair needs a venue",
        ),
        ("USDC --venue lat-a --rule nonsense --once", "--rule"),
        ("USDC --venue lat-a --rule holders>1 --once", "holders"),
        ("0xW --kind address --once", "needs a rule"),
        ("USDC --chain nochain --once", "unknown chain"),
        ("USDC --venue lat-a --every soon --once", "--every"),
        ("pegs.yaml --venue lat-a --once", "is a path"),
        ("./pegs --chain base --once", "is a path"),
        ("USDC --once", "To watch a token, give a venue or a chain"),
    ];
    for (args, expect) in cases {
        let (dir, factory) = setup(up());
        let run = watch(&dir, &factory, &config, args).await;
        assert_eq!(run.code, EXIT_NOT_STARTED, "{args}: {}", run.err);
        assert!(run.err.contains(expect), "{args}: {}", run.err);
        assert_eq!(factory.http.started(), 0, "{args}: made a request");
    }
}

/// Why: the cron job must mean one thing, whatever files exist. A saved
/// list named USDC does not change `USDC --venue lat-a`, and the run says
/// so. Without a target flag the same word is the saved list.
#[tokio::test(start_paused = true)]
async fn a_target_flag_wins_over_a_saved_list_of_the_same_name_and_says_so() {
    let config = Config::default();
    let (dir, factory) = setup(up());
    std::fs::write(
        dir.path().join("lists").join("USDC.yaml"),
        "targets:\n  - kind: pair\n    id: DAI\n    venue: lat-a\n    rules: [\"mid_price<0.5\"]\n",
    )
    .unwrap();
    let inline = watch(
        &dir,
        &factory,
        &config,
        &format!("USDC --venue lat-a {ONCE} --rule mid_price>0"),
    )
    .await;
    assert_eq!(inline.code, EXIT_PASS, "{}", inline.err);
    assert!(
        inline
            .err
            .contains("A saved watchlist named USDC exists. This run ignores it."),
        "{}",
        inline.err
    );
    assert_eq!(inline.of("start")["shadowed_saved_list"], "USDC");
    let rows = inline.of("tick")["rows"].as_array().unwrap().clone();
    assert_eq!(rows[0]["target"], "USDC@lat-a");

    let saved = watch(&dir, &factory, &config, "USDC --once --format json").await;
    assert_eq!(
        saved.code, EXIT_BREACH,
        "the saved list breaches: {}",
        saved.err
    );
    assert_eq!(saved.of("start")["source"], "file");
    assert_eq!(
        saved.of("tick")["rows"][0]["target"],
        "DAI@lat-a",
        "the saved list ran"
    );
}

/// Why: when a job breaches at 03:00, the record must say what it watched
/// (section 2.4), including the rule from the config that nobody typed.
#[tokio::test(start_paused = true)]
async fn the_start_record_says_what_the_inline_run_watched() {
    let mut config = Config::default();
    config.monitor.alerts.price_min = Some(0.5);
    let (dir, factory) = setup(up());
    let run = watch(
        &dir,
        &factory,
        &config,
        &format!("USDC --venue lat-a {ONCE} --rule mid_price>0 --every 5m"),
    )
    .await;
    assert_eq!(run.code, EXIT_PASS, "{}", run.err);
    let start = run.of("start");
    assert_eq!(start["source"], "cli");
    assert_eq!(start["name"], "cli-USDC");
    assert_eq!(start["path"], Value::Null);
    assert_eq!(start["argv"][1], "watch");
    assert_eq!(start["argv"][2], "USDC");
    assert_eq!(start["config_alerts_applied"][0], "mid_price >= 0.5");
    assert_eq!(start["shadowed_saved_list"], Value::Null);
    let definition = start["definition"].as_str().unwrap();
    assert!(definition.contains("mid_price >= 0.5"), "{definition}");
    // A person can paste the definition into a file and get the same run.
    let venues = vec!["lat-a".to_string()];
    let list = scope::watch::file::parse(definition, "x", Some(&venues)).unwrap();
    assert_eq!(list.targets[0].id, "USDC");
    assert_eq!(list.interval, std::time::Duration::from_secs(300));
    assert!(
        run.err.contains("Rules from config `monitor.alerts`"),
        "{}",
        run.err
    );
}

/// Why (Q5): the same words give the same length in both commands. With no
/// `--once` and no `--duration`, an inline run lasts 1 hour, and says so.
#[tokio::test(start_paused = true)]
async fn an_inline_run_with_no_duration_lasts_one_hour() {
    let config = Config::default();
    let (dir, factory) = setup(up());
    let run = watch(
        &dir,
        &factory,
        &config,
        "USDC --venue lat-a --format json --preset none --rule mid_price>0 --every 30m",
    )
    .await;
    assert_eq!(run.of("start")["duration_secs"], 3600);
    assert_eq!(run.of("end")["reason"], "duration");
    assert_eq!(run.code, EXIT_PASS, "{}", run.err);
    assert!(run.err.contains("for 1h"), "{}", run.err);
}

/// Why (section 3.2.2): the memory of an inline job belongs to its
/// definition. The same words share it; a different rule is a different
/// job with its own first tick.
#[tokio::test(start_paused = true)]
async fn the_same_inline_words_share_a_memory_and_other_words_do_not() {
    let config = Config::default();
    let (dir, factory) = setup(up());
    let words = format!("USDC --venue lat-a {ONCE} --every 5m --rule volume_change_pct<=0.5");
    let first = watch(&dir, &factory, &config, &words).await;
    assert_eq!(first.code, EXIT_PASS, "{}", first.err);
    assert_eq!(first.of("start")["state"]["file"], "none");

    // The test venue reports 1% more volume at each read.
    let second = watch(&dir, &factory, &config, &words).await;
    assert_eq!(second.of("start")["state"]["file"], "loaded");
    assert_eq!(second.code, EXIT_BREACH, "{:?}", second.records);

    let other = format!("USDC --venue lat-a {ONCE} --every 5m --rule volume_change_pct<=0.6");
    let third = watch(&dir, &factory, &config, &other).await;
    assert_eq!(third.of("start")["state"]["file"], "none");
    assert_eq!(third.code, EXIT_PASS, "a first tick for a new job");

    let files = std::fs::read_dir(dir.path().join("state"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count();
    assert_eq!(files, 2, "one state file for each definition");
}
