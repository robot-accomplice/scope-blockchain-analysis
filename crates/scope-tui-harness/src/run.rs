//! Runs one scenario (design 4.2, 5.2).

use crate::scenario::{Scenario, Step, Tier, key_bytes};
use crate::screen::{SnapshotResult, compare_snapshot, mask};
use crate::session::{Session, StartOpts, data_dir};
use anyhow::Result;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct RunCtx {
    pub binary: PathBuf,
    pub fixtures_root: PathBuf,
    pub snapshots_root: PathBuf,
    pub scenario_id: String,
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
    std::fs::read_to_string(data_dir(home).join("tui-events.jsonl"))
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

pub fn run_scenario(
    s: &Scenario,
    tier: Tier,
    ctx: &RunCtx,
) -> Result<(Outcome, Session, tempfile::TempDir)> {
    let home = tempfile::tempdir()?;
    let cfg_dir = home.path().join(".config/scope");
    std::fs::create_dir_all(&cfg_dir)?;
    std::fs::write(
        cfg_dir.join("config.yaml"),
        s.config.clone().unwrap_or_else(|| "{}".into()),
    )?;
    let mut env = vec![];
    if tier == Tier::Offline {
        env.push(("SCOPE_HTTP_MODE".to_string(), "replay".to_string()));
        env.push((
            "SCOPE_HTTP_FIXTURES".to_string(),
            ctx.fixtures_root
                .join(&ctx.scenario_id)
                .display()
                .to_string(),
        ));
    }
    let mut sess = Session::start(&StartOpts {
        binary: ctx.binary.clone(),
        home: home.path().into(),
        cols: s.terminal.cols,
        rows: s.terminal.rows,
        env,
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
            out.argv_seen = journal(home.path()).iter().map(argv_of).collect();
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

fn exited(sess: &mut Session) -> Result<()> {
    match sess.exit_status() {
        Some(code) => anyhow::bail!("process exited with status {code}"),
        None => Ok(()),
    }
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
            sess.send(text.as_bytes())?;
            sess.pump(SETTLE)?;
        }
        Step::Key(k) => {
            exited(sess)?;
            sess.send(&key_bytes(k)?)?;
            sess.pump(SETTLE)?;
        }
        Step::Resize { cols, rows } => {
            exited(sess)?;
            sess.resize(*cols, *rows)?;
            sess.pump(SETTLE * 2)?;
        }
        Step::WaitFor { text, timeout } => {
            let re = Regex::new(text)?;
            let until = Instant::now() + timeout.unwrap_or(DEFAULT_STEP_TIMEOUT);
            loop {
                sess.pump(POLL)?;
                if re.is_match(&sess.screen()) {
                    break;
                }
                exited(sess)?;
                if Instant::now() >= until {
                    anyhow::bail!("{text:?} did not appear");
                }
            }
        }
        Step::ExpectScreen {
            contains,
            not_contains,
        } => {
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
        Step::ExpectJournal {
            argv_prefix,
            outcome,
            error,
        } => {
            let recs = journal(home);
            let new = &recs[(*journal_seen).min(recs.len())..];
            let err_re = error.as_deref().map(Regex::new).transpose()?;
            let hit = new.iter().any(|r| {
                argv_of(r).starts_with(argv_prefix)
                    && outcome.as_deref().is_none_or(|o| r["outcome"] == o)
                    && err_re
                        .as_ref()
                        .is_none_or(|re| r["error"].as_str().is_some_and(|e| re.is_match(e)))
            });
            *journal_seen = recs.len();
            if !hit {
                anyhow::bail!("no journal record matches; new records: {new:?}");
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
            sess.pump(SETTLE)?;
            let masked = mask(&sess.screen(), &s.masks)?;
            match compare_snapshot(&ctx.snapshots_root.join(&ctx.scenario_id), name, &masked)? {
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
            let until = Instant::now() + DEFAULT_STEP_TIMEOUT;
            loop {
                sess.pump(POLL)?;
                if let Some(c) = sess.exit_status() {
                    if c != *code {
                        anyhow::bail!("exit status {c}, expected {code}");
                    }
                    break;
                }
                if Instant::now() >= until {
                    anyhow::bail!("process did not exit");
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

    fn ctx() -> Option<RunCtx> {
        let bin = std::env::var("SCOPE_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/scope")
            });
        if !bin.exists() {
            eprintln!("skip: build scope first (cargo build --bin scope)");
            return None;
        }
        let d = tempfile::tempdir().unwrap().keep();
        Some(RunCtx {
            binary: bin,
            fixtures_root: d.join("fx"),
            snapshots_root: d.join("snap"),
            scenario_id: "t".into(),
        })
    }

    #[test]
    fn test_session_command_runs_and_is_journaled() {
        let Some(c) = ctx() else { return };
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
    fn test_wait_for_times_out_with_the_screen_in_the_failure() {
        let Some(c) = ctx() else { return };
        let s = parse("name: never\nsteps:\n  - wait_for: { text: \"NEVER-APPEARS\", timeout: 1s }\n  - expect_exit: 0\n").unwrap();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        let f = out.failure.unwrap();
        assert!(f.contains("NEVER-APPEARS") && f.contains("screen:"), "{f}");
    }

    #[test]
    fn test_early_exit_fails_the_next_step_without_hanging() {
        let Some(c) = ctx() else { return };
        let s = parse("name: early\ntimeout: 20s\nsteps:\n  - key: Ctrl-D\n  - wait_for: { text: \"x\", timeout: 10s }\n  - expect_exit: 0\n").unwrap();
        let start = std::time::Instant::now();
        let (out, _, _) = run_scenario(&s, Tier::Offline, &c).unwrap();
        assert!(!out.passed);
        assert!(out.failure.unwrap().contains("process exited"));
        assert!(start.elapsed() < std::time::Duration::from_secs(15));
    }
}
