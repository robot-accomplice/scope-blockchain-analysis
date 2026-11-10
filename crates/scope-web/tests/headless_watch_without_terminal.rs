//! `scope watch` runs with no terminal: no stdin, piped stdout and stderr,
//! as in cron or CI (#123). It never prompts, it prints data on stdout and
//! notes on stderr, and it ends with the exit code that a script reads.
//!
//! Every run uses its own `SCOPE_BCA_HOME` in a temp folder, never the
//! real `~/.scope-bca`. The only venue is a local one whose port refuses
//! connections: no test reaches the network.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// A venue descriptor for the `dead` venue: the weex shape, a port that
/// refuses connections.
const DEAD_VENUE: &str = "id: dead
name: Dead venue
base_url: http://127.0.0.1:1
timeout_secs: 2
symbol:
  template: \"{base}{quote}\"
  default_quote: USDT
capabilities:
  order_book:
    path: /depth
    params:
      symbol: \"{pair}\"
    response:
      asks_key: asks
      bids_key: bids
      level_format: positional
  ticker:
    path: /ticker
    params:
      symbol: \"{pair}\"
    response:
      last_price: lastPrice
      quote_volume_24h: quoteVolume
";

const LIST: &str = "name: pegs
targets:
  - kind: pair
    id: USDC
    venue: dead
    preset: market health
";

/// A scope home with the dead venue, a config that sends the venue catalog
/// to the refusing port, and the watchlist `pegs`.
fn home(list: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".scope-bca");
    std::fs::create_dir_all(home.join("venues")).unwrap();
    std::fs::create_dir_all(home.join("watchlists")).unwrap();
    std::fs::write(home.join("venues/dead.yaml"), DEAD_VENUE).unwrap();
    std::fs::write(home.join("watchlists/pegs.yaml"), list).unwrap();
    std::fs::write(
        home.join("config.yaml"),
        "market:\n  venue_catalog_url: http://127.0.0.1:1\n",
    )
    .unwrap();
    dir
}

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_scope"));
    cmd.args(args)
        // A release binary reads SCOPE_BCA_HOME; a binary built with the
        // `test-support` feature (as in a workspace test run) ignores it and
        // reads SCOPE_TEST_HOME. Both name the same folder.
        .env("SCOPE_BCA_HOME", dir.join(".scope-bca"))
        .env("SCOPE_TEST_HOME", dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        // No terminal on any stream.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for proxy in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
    ] {
        cmd.env_remove(proxy);
    }
    cmd
}

fn run(dir: &Path, args: &[&str]) -> Output {
    command(dir, args)
        .spawn()
        .unwrap()
        .wait_with_output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_venue_error_exits_2_with_data_on_stdout_and_notes_on_stderr_and_no_prompt() {
    let dir = home(LIST);
    let csv = dir.path().join("trail.csv");
    let report = dir.path().join("report.md");
    let out = run(
        dir.path(),
        &[
            "watch",
            "pegs",
            "--once",
            "--csv",
            csv.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ],
    );
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(2), "{stdout}{stderr}");
    // The reason for code 2 is on stdout, with the exact venue.
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("ERROR") && l.contains("USDC@dead")),
        "{stdout}"
    );
    // Notes are on stderr only.
    assert!(stderr.contains("Scope v"), "{stderr}");
    assert!(stderr.contains("Run ended: once"), "{stderr}");
    assert!(
        !stdout.contains("Scope v") && !stdout.contains("Run ended"),
        "{stdout}"
    );
    // No question reached a user who is not there.
    let all = format!("{stdout}{stderr}");
    assert!(
        !all.contains("[Y/n]") && !all.contains("Run setup now"),
        "{all}"
    );
    // The files are whole.
    let rows = std::fs::read_to_string(&csv).unwrap();
    assert!(rows.starts_with("ts,target,metric"), "{rows}");
    assert!(
        rows.contains("USDC@dead") && rows.contains(",error,"),
        "{rows}"
    );
    assert!(
        std::fs::read_to_string(&report)
            .unwrap()
            .contains("# Watchlist Report: pegs")
    );
}

#[test]
fn json_lines_have_start_tick_and_end_records() {
    let dir = home(LIST);
    let out = run(dir.path(), &["watch", "pegs", "--once", "--format", "json"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    let records: Vec<serde_json::Value> = text(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    let kinds: Vec<&str> = records
        .iter()
        .map(|r| r["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["start", "tick", "end"]);
    assert_eq!(records[2]["exit_code"], 2);
    assert_eq!(records[2]["reason"], "once");
}

// Exit 3 is the code of a run that could not start. The home has no
// watchlist of this name, and nothing was requested.
#[test]
fn a_missing_list_exits_3_and_lists_the_saved_names() {
    let dir = home(LIST);
    let out = run(dir.path(), &["watch", "nope", "--once"]);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("Saved watchlists: pegs"), "{stderr}");
    assert_eq!(stdout, "");
}

#[test]
fn a_csv_folder_that_is_missing_exits_4() {
    let dir = home(LIST);
    let csv = dir.path().join("no-such-folder/t.csv");
    let out = run(
        dir.path(),
        &["watch", "pegs", "--once", "--csv", csv.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(4), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "");
}

#[test]
fn help_lists_the_exit_codes_and_a_flag_conflict_is_a_usage_error() {
    let dir = home(LIST);
    let out = run(dir.path(), &["watch", "--help"]);
    let help = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for needle in ["Exit codes", "scope watch pegs --once", "--append"] {
        assert!(help.contains(needle), "{needle}: {help}");
    }
    // clap's own usage error is 2, the code of "unknown": the docs say so.
    let out = run(dir.path(), &["watch", "pegs", "--once", "--every", "1s"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("cannot be used with"),
        "{}",
        text(&out.stderr)
    );
}

#[cfg(unix)]
fn wait_for(child: &mut std::process::Child, path: &Path) {
    for _ in 0..200 {
        if path.exists()
            && std::fs::metadata(path)
                .map(|m| m.len() > 80)
                .unwrap_or(false)
        {
            return;
        }
        if child.try_wait().unwrap().is_some() {
            panic!("the run ended before its first tick");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("no tick after 10 seconds");
}

// SIGTERM stops a run with no end: the report is written, and the code is
// the code of the last tick. Unix only: Windows has no SIGTERM.
#[cfg(unix)]
#[test]
fn sigterm_stops_a_run_with_no_duration_and_writes_the_report() {
    let dir = home(LIST);
    let csv = dir.path().join("trail.csv");
    let report = dir.path().join("report.md");
    let mut child = command(
        dir.path(),
        &[
            "watch",
            "pegs",
            "--every",
            "1h",
            "--duration",
            "forever",
            "--csv",
            csv.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ],
    )
    .spawn()
    .unwrap();
    wait_for(&mut child, &csv);
    let sent = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let out = child.wait_with_output().unwrap();
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("Run ended: signal after 1 run(s)"),
        "{stderr}"
    );
    assert!(
        std::fs::read_to_string(&report)
            .unwrap()
            .contains("## Last values")
    );
}
