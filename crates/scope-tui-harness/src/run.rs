//! Runs one scenario (design 4.2, 5.2).

use crate::scenario::{Scenario, Step, Tier, key_bytes};
use crate::screen::{SnapshotResult, compare_snapshot, mask};
use crate::session::{Session, StartOpts, state_dir};
use anyhow::Result;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct RunCtx {
    pub binary: PathBuf,
    pub fixtures_root: PathBuf,
    pub snapshots_root: PathBuf,
    pub scenario_id: String,
    /// Record the live replies as this scenario's fixtures (`just tui-record`).
    pub record: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StepLog {
    pub index: usize,
    pub step: String,
    pub ms: u128,
    pub ok: bool,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub passed: bool,
    pub failure: Option<String>,
    pub steps: Vec<StepLog>,
    pub argv_seen: Vec<Vec<String>>,
}

const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_millis(150);
const POLL: Duration = Duration::from_millis(100);

fn journal(home: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(state_dir(home).join("tui-events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn argv_of(rec: &serde_json::Value) -> Vec<String> {
    rec["argv"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Makes a path absolute against the current directory. The path need not exist.
fn absolutize(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(p))
    }
}

/// The env the child sees. The child's cwd is its temp HOME, so paths must be absolute.
fn child_env(tier: Tier, ctx: &RunCtx) -> Result<Vec<(String, String)>> {
    let mut env = vec![];
    if ctx.record {
        // The live APIs answer; each reply is written as a fixture.
        env.push(("SCOPE_HTTP_MODE".to_string(), "record".to_string()));
        env.push((
            "SCOPE_HTTP_FIXTURES".to_string(),
            absolutize(&ctx.fixtures_root)?
                .join(&ctx.scenario_id)
                .display()
                .to_string(),
        ));
    } else if tier == Tier::Offline {
        env.push(("SCOPE_HTTP_MODE".to_string(), "replay".to_string()));
        env.push((
            "SCOPE_HTTP_FIXTURES".to_string(),
            absolutize(&ctx.fixtures_root)?
                .join(&ctx.scenario_id)
                .display()
                .to_string(),
        ));
    }
    Ok(env)
}

/// Installs the repo's venue catalog in a scenario's home, as
/// `scope venues update` would: scope carries no venues, and a scenario
/// must not depend on fetching them.
fn seed_catalog(dest: &Path) -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scope-core/venues");
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(&repo)? {
        let path = entry?.path();
        if path.is_file() {
            std::fs::copy(&path, dest.join(path.file_name().unwrap()))?;
        }
    }
    Ok(())
}

pub fn run_scenario(
    s: &Scenario,
    tier: Tier,
    ctx: &RunCtx,
) -> Result<(Outcome, Session, tempfile::TempDir)> {
    let home = tempfile::tempdir()?;
    let cfg_dir = home.path().join(".scope-bca");
    std::fs::create_dir_all(&cfg_dir)?;
    std::fs::write(
        cfg_dir.join("config.yaml"),
        s.config.clone().unwrap_or_else(|| "{}".into()),
    )?;
    seed_catalog(&cfg_dir.join("venues").join("catalog"))?;
    let mut env = child_env(tier, ctx)?;
    env.extend(s.env.iter().map(|(k, v)| (k.clone(), v.clone())));
    let mut sess = Session::start(&StartOpts {
        binary: ctx.binary.clone(),
        home: home.path().into(),
        cols: s.terminal.cols,
        rows: s.terminal.rows,
        env,
        args: s.args.clone(),
        wait_for_paint: s.wait_for_paint,
    })?;
    let deadline = Instant::now() + s.timeout;
    let mut out = Outcome::default();
    let mut journal_seen = 0usize;

    for (i, step) in s.steps.iter().enumerate() {
        let started = Instant::now();
        let result = run_step(
            step,
            s,
            ctx,
            &mut sess,
            home.path(),
            &mut journal_seen,
            deadline,
        );
        out.steps.push(StepLog {
            index: i,
            step: format!("{step:?}"),
            ms: started.elapsed().as_millis(),
            ok: result.is_ok(),
        });
        if let Err(e) = result {
            out.failure = Some(format!(
                "step {} {:?}: {e}\nscreen:\n{}",
                i + 1,
                step,
                sess.screen()
            ));
            sess.kill();
            return Ok((out, sess, home));
        }
    }
    out.passed = true;
    out.argv_seen = journal(home.path())
        .iter()
        .filter(|r| r["outcome"] == "ok" || r["outcome"] == "error")
        .map(argv_of)
        .collect();
    Ok((out, sess, home))
}

/// The end of a step's wait, and whether the scenario deadline is what ends it.
fn wait_end(timeout: Duration, deadline: Instant) -> (Instant, bool) {
    let step_until = Instant::now() + timeout;
    if deadline < step_until {
        (deadline, true)
    } else {
        (step_until, false)
    }
}

fn timed_out(clamped: bool, scenario: Duration, what: impl FnOnce() -> String) -> anyhow::Error {
    if clamped {
        anyhow::anyhow!("scenario timeout {scenario:?} reached")
    } else {
        anyhow::anyhow!(what())
    }
}

fn exited(sess: &mut Session) -> Result<()> {
    match sess.exit_status() {
        Some(code) => anyhow::bail!("process exited with status {code}"),
        None => Ok(()),
    }
}

/// The bytes a `type` step sends. A line feed ends a line at a cooked
/// prompt: the unix pty has no ICRNL, so the scenarios type `\n`. ConPTY
/// turns an input LF into Ctrl+J, which does not end a console read; a
/// console line ends with Enter, a CR.
fn typed_bytes(text: &str) -> std::borrow::Cow<'_, str> {
    if cfg!(windows) {
        text.replace('\n', "\r").into()
    } else {
        text.into()
    }
}

/// Sends a named signal to the scope process.
#[cfg(unix)]
fn send_signal(sess: &Session, name: &str) -> Result<()> {
    let pid = sess
        .pid()
        .ok_or_else(|| anyhow::anyhow!("signal {name}: the process id is unknown"))?;
    let sig = crate::scenario::signal_number(name)?;
    // SAFETY: kill takes plain integers and has no memory effects.
    let rc = unsafe { libc::kill(pid as i32, sig) };
    anyhow::ensure!(
        rc == 0,
        "signal {name}: kill failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

#[cfg(not(unix))]
fn send_signal(_sess: &Session, name: &str) -> Result<()> {
    anyhow::bail!("signal {name}: signals need a unix host")
}

fn run_step(
    step: &Step,
    s: &Scenario,
    ctx: &RunCtx,
    sess: &mut Session,
    home: &Path,
    journal_seen: &mut usize,
    deadline: Instant,
) -> Result<()> {
    if Instant::now() >= deadline {
        anyhow::bail!("scenario timeout {:?} reached", s.timeout);
    }
    match step {
        Step::Type(text) => {
            exited(sess)?;
            sess.send(typed_bytes(text).as_bytes())?;
            sess.pump(SETTLE)?;
        }
        Step::Key(k) => {
            exited(sess)?;
            sess.send(&key_bytes(k)?)?;
            sess.pump(SETTLE)?;
        }
        Step::Signal(name) => {
            exited(sess)?;
            send_signal(sess, name)?;
            sess.pump(SETTLE)?;
        }
        Step::Resize { cols, rows } => {
            exited(sess)?;
            sess.resize(*cols, *rows)?;
            sess.pump(SETTLE * 2)?;
        }
        Step::WaitFor { text, timeout } => {
            let re = Regex::new(text)?;
            let (until, clamped) = wait_end(timeout.unwrap_or(DEFAULT_STEP_TIMEOUT), deadline);
            loop {
                sess.pump(POLL)?;
                if re.is_match(&sess.screen()) {
                    break;
                }
                exited(sess)?;
                if Instant::now() >= until {
                    return Err(timed_out(clamped, s.timeout, || {
                        format!("{text:?} did not appear")
                    }));
                }
            }
        }
        Step::ExpectScreen {
            contains,
            not_contains,
        } => {
            exited(sess)?;
            sess.pump(SETTLE)?;
            let screen = sess.screen();
            for p in contains {
                if !Regex::new(p)?.is_match(&screen) {
                    anyhow::bail!("screen does not match {p:?}");
                }
            }
            for p in not_contains {
                if Regex::new(p)?.is_match(&screen) {
                    anyhow::bail!("screen matches forbidden {p:?}");
                }
            }
        }
        Step::ExpectPasteMode(want) => {
            exited(sess)?;
            sess.pump(SETTLE)?;
            if sess.bracketed_paste() != *want {
                anyhow::bail!("bracketed paste mode is not {want}");
            }
        }
        Step::ExpectJournal {
            argv_prefix,
            outcome,
            error,
        } => {
            let err_re = error.as_deref().map(Regex::new).transpose()?;
            let (until, clamped) = wait_end(DEFAULT_STEP_TIMEOUT, deadline);
            loop {
                // Read the exit state first, so a record written before exit is still seen.
                let exit = sess.exit_status();
                let recs = journal(home);
                let new = &recs[(*journal_seen).min(recs.len())..];
                let hit = new.iter().any(|r| {
                    argv_of(r).starts_with(argv_prefix)
                        && outcome.as_deref().is_none_or(|o| r["outcome"] == o)
                        && err_re
                            .as_ref()
                            .is_none_or(|re| r["error"].as_str().is_some_and(|e| re.is_match(e)))
                });
                if hit {
                    *journal_seen = recs.len();
                    break;
                }
                if let Some(code) = exit {
                    *journal_seen = recs.len();
                    anyhow::bail!(
                        "no journal record matches (process exited with status {code}); new records: {new:?}"
                    );
                }
                if Instant::now() >= until {
                    *journal_seen = recs.len();
                    return Err(timed_out(clamped, s.timeout, || {
                        format!("no journal record matches; new records: {new:?}")
                    }));
                }
                sess.pump(POLL)?;
            }
        }
        Step::ExpectFile { path, contains } => {
            let p = home.join(path);
            let text =
                std::fs::read_to_string(&p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
            for pat in contains {
                if !Regex::new(pat)?.is_match(&text) {
                    anyhow::bail!("{} does not match {pat:?}", p.display());
                }
            }
        }
        Step::Snapshot(name) => {
            exited(sess)?;
            sess.pump(SETTLE)?;
            let masked = mask(&sess.screen(), &s.masks)?;
            match compare_snapshot(
                &absolutize(&ctx.snapshots_root)?.join(&ctx.scenario_id),
                name,
                &masked,
            )? {
                SnapshotResult::Match => {}
                SnapshotResult::Written(p) => {
                    anyhow::bail!(
                        "new snapshot written to {}; review it and run again",
                        p.display()
                    )
                }
                SnapshotResult::Differs { expected, actual } => {
                    anyhow::bail!(
                        "snapshot {name} differs\nexpected:\n{expected}\nactual:\n{actual}"
                    )
                }
            }
        }
        Step::ExpectExit(code) => {
            let (until, clamped) = wait_end(DEFAULT_STEP_TIMEOUT, deadline);
            loop {
                sess.pump(POLL)?;
                if let Some(c) = sess.exit_status() {
                    if c != *code {
                        anyhow::bail!("exit status {c}, expected {code}");
                    }
                    break;
                }
                if Instant::now() >= until {
                    return Err(timed_out(clamped, s.timeout, || {
                        "process did not exit".to_string()
                    }));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::parse;
    use std::path::PathBuf;

    #[test]
    fn test_absolutize_makes_relative_paths_absolute_without_requiring_them_to_exist() {
        let p = absolutize(Path::new("tests/tui/fixtures-that-do-not-exist")).unwrap();
        assert!(p.is_absolute());
        assert!(p.starts_with(std::env::current_dir().unwrap()));
        // An absolute path on this platform (`/x` has no drive on Windows).
        let abs = std::env::temp_dir().join("x").join("y");
        assert_eq!(absolutize(&abs).unwrap(), abs);
    }

    #[test]
    fn test_record_mode_writes_the_scenario_fixtures_from_the_live_apis() {
        let c = RunCtx {
            binary: "scope".into(),
            fixtures_root: "tests/tui/fixtures".into(),
            snapshots_root: "tests/tui/snapshots".into(),
            scenario_id: "demo".into(),
            record: true,
        };
        let env = child_env(Tier::Offline, &c).unwrap();
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("SCOPE_HTTP_MODE").as_deref(), Some("record"));
        // By components, so the separator does not matter.
        let fx = get("SCOPE_HTTP_FIXTURES").unwrap();
        assert!(Path::new(&fx).ends_with("tests/tui/fixtures/demo"), "{fx}");
    }

    #[test]
    fn test_child_fixtures_env_is_absolute_for_a_relative_root() {
        let c = RunCtx {
            binary: "scope".into(),
            fixtures_root: "tests/tui/fixtures".into(),
            snapshots_root: "tests/tui/snapshots".into(),
            scenario_id: "demo".into(),
            record: false,
        };
        let env = child_env(Tier::Offline, &c).unwrap();
        let fx = &env
            .iter()
            .find(|(k, _)| k == "SCOPE_HTTP_FIXTURES")
            .unwrap()
            .1;
        assert!(Path::new(fx).is_absolute(), "{fx}");
        assert!(Path::new(fx).ends_with("tests/tui/fixtures/demo"), "{fx}");
        assert!(child_env(Tier::Live, &c).unwrap().is_empty());
    }

    fn ctx() -> (RunCtx, tempfile::TempDir) {
        let bin = std::env::var("SCOPE_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/debug/scope")
                    .with_extension(std::env::consts::EXE_EXTENSION)
            });
        assert!(
            bin.exists(),
            "scope binary missing at {}: run `cargo build --bin scope` or set SCOPE_BIN",
            bin.display()
        );
        let d = tempfile::tempdir().unwrap();
        let c = RunCtx {
            binary: bin,
            fixtures_root: d.path().join("fx"),
            snapshots_root: d.path().join("snap"),
            scenario_id: "t".into(),
            record: false,
        };
        (c, d)
    }

    #[test]
    #[ignore = "needs the scope binary: cargo build --bin scope && cargo test -p scope-tui-harness -- --ignored"]
    fn test_session_command_runs_and_is_journaled() {
        let (c, _keep) = ctx();
        let s = parse(
            r#"
name: ctx shows the session
steps:
  - type: "ctx"
  - key: Enter
  - wait_for: { text: "Current Context", timeout: 10s }
  - expect_journal: { argv_prefix: [], outcome: session }
  - key: Ctrl-D
  - expect_exit: 0
"#,
        )
        .unwrap();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(out.passed, "{:?}", out.failure);
    }

    #[test]
    #[ignore = "needs the scope binary: cargo build --bin scope && cargo test -p scope-tui-harness -- --ignored"]
    fn test_wait_for_times_out_with_the_screen_in_the_failure() {
        let (c, _keep) = ctx();
        let s = parse("name: never\nsteps:\n  - wait_for: { text: \"NEVER-APPEARS\", timeout: 1s }\n  - expect_exit: 0\n").unwrap();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        let f = out.failure.unwrap();
        assert!(f.contains("NEVER-APPEARS") && f.contains("screen:"), "{f}");
    }

    #[test]
    #[ignore = "needs the scope binary: cargo build --bin scope && cargo test -p scope-tui-harness -- --ignored"]
    fn test_early_exit_fails_the_next_step_without_hanging() {
        let (c, _keep) = ctx();
        let s = parse("name: early\ntimeout: 20s\nsteps:\n  - key: Ctrl-D\n  - wait_for: { text: \"NEVER-APPEARS\", timeout: 10s }\n  - expect_exit: 0\n").unwrap();
        let start = std::time::Instant::now();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        assert!(out.failure.unwrap().contains("process exited"));
        assert!(start.elapsed() < std::time::Duration::from_secs(15));
    }

    #[test]
    #[ignore = "needs the scope binary: cargo build --bin scope && cargo test -p scope-tui-harness -- --ignored"]
    fn test_scenario_deadline_clamps_a_longer_step_wait() {
        let (c, _keep) = ctx();
        let s = parse("name: clamp\ntimeout: 1s\nsteps:\n  - wait_for: { text: \"NEVER-APPEARS\", timeout: 10s }\n  - expect_exit: 0\n").unwrap();
        let start = std::time::Instant::now();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        assert!(out.failure.unwrap().contains("scenario timeout"));
        assert!(out.argv_seen.is_empty());
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    #[ignore = "needs the scope binary: cargo build --bin scope && cargo test -p scope-tui-harness -- --ignored"]
    fn test_expect_journal_after_exit_fails_at_once_without_spinning() {
        let (c, _keep) = ctx();
        let s = parse("name: exitj\nsteps:\n  - key: Ctrl-D\n  - expect_journal: { argv_prefix: [\"market\"], outcome: ok }\n  - expect_exit: 0\n").unwrap();
        let start = std::time::Instant::now();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        assert!(out.failure.unwrap().contains("process exited"));
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "needs the scope binary: cargo build --bin scope && cargo test -p scope-tui-harness -- --ignored"]
    fn test_signal_step_ends_process() {
        let (c, _keep) = ctx();
        // The signal reaches the process, and scope exits with 128+15 (R1).
        let s = parse(
            "name: sig\nargs: [tui, --no-banner]\nunix_only: true\nsteps:\n  - signal: TERM\n  - expect_exit: 143\n",
        )
        .unwrap();
        let start = std::time::Instant::now();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        let f = out.failure.unwrap_or_default();
        assert!(out.passed, "{f}");
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{f}");
    }
}
