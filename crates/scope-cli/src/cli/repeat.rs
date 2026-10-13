//! The one repeat runner for commands that take `--every` and `--duration`
//! (#58).
//!
//! It owns the interval, the duration, the run count, the progress record,
//! the final-run rule and the rule that an error ends the repeat. A command
//! supplies one run (`step`). Each run's key values go to the CSV file and
//! to the screen's series (TUI); in the CLI the run's report prints as
//! before.

use crate::cli::output::{Output, RunRecord};
use crate::{note, out};
use scope::error::{Result, ScopeError};
use scope::market::series::{self, Metric, Sample};
use std::future::Future;
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};
// The schedule runs on tokio's clock, the one its sleep uses.
use tokio::time::Instant as Clock;

/// The timestamp format of a CSV row.
const CSV_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// When a repeating command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    /// The time from one run's start to the next.
    pub every: Duration,
    /// How long the repeat lasts. A last run starts when it ends.
    pub duration: Duration,
}

impl Schedule {
    /// A schedule; the interval must be positive.
    pub fn new(every: Duration, duration: Duration) -> Result<Self> {
        if every.is_zero() {
            return Err(ScopeError::InvalidInput(
                "Interval must be positive".to_string(),
            ));
        }
        Ok(Self { every, duration })
    }

    /// How many runs it makes: one every `every` over `duration`, and a last
    /// one at the end when the span is not a whole number of intervals (the
    /// final sleep is cut to the time left).
    pub fn total(&self) -> u64 {
        let every = self.every.as_millis().max(1);
        let total = self.duration.as_millis().div_ceil(every).saturating_add(1);
        u64::try_from(total).unwrap_or(u64::MAX)
    }
}

/// What one run gives the runner.
#[derive(Debug, Clone)]
pub struct RunOutput<T> {
    /// The run's key values, in the order of the runner's metric table.
    pub sample: Sample,
    /// The run's report, as the CLI prints it.
    pub snapshot: String,
    /// The command's own result of the run (for example for `--report`).
    pub value: T,
}

/// Runs `step` on `schedule` and returns the last run's value.
///
/// Before each run in the CLI, it notes `--- Run #N at HH:MM:SS ---`; after
/// it, the CLI prints the snapshot. With a screen series (TUI), the record
/// goes to the series instead, so the pane does not stack one report per
/// run. With `csv`, the file gets the header, then one row per run. An
/// error from `step` ends the repeat and is returned.
pub async fn run<T, F, Fut>(
    schedule: &Schedule,
    metrics: &[Metric],
    csv: Option<&Path>,
    out: &Output,
    mut step: F,
) -> Result<Option<T>>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = Result<RunOutput<T>>>,
{
    let columns = series::csv_columns(metrics).map_err(|e| ScopeError::Other(e.to_string()))?;
    if let Some(path) = csv {
        std::fs::write(path, series::csv_header(&columns))
            .map_err(|e| ScopeError::io_context(e, &format!("cannot write {}", path.display())))?;
    }
    out.update_progress(|p| {
        p.run = 0;
        p.total = Some(schedule.total());
        p.next_at = None;
    });

    let tui = out.series_hook().is_some();
    let start = Clock::now();
    let mut run: u64 = 1;
    loop {
        out.update_progress(|p| {
            p.run = run;
            p.next_at = None;
        });
        if !tui {
            let ts = chrono::Utc::now().format("%H:%M:%S");
            note!(out, "--- Run #{} at {} ---\n", run, ts);
        }
        // An error ends the repeat: `?` returns it and no later run starts.
        let output = step(run).await?;
        let at = chrono::Utc::now();
        if let Some(path) = csv {
            let time = at.format(CSV_TIME_FORMAT).to_string();
            let row = series::csv_row(&columns, metrics, &time, run, &output.sample);
            append(path, &row)?;
        }
        let record = RunRecord {
            run,
            at,
            sample: output.sample,
            snapshot: output.snapshot.clone(),
        };
        if !out.record_run(metrics, record) {
            out!(out, "{}", output.snapshot)?;
        }
        let last = output.value;

        if start.elapsed() >= schedule.duration {
            note!(
                out,
                "\nCompleted {} run(s) over {}s.",
                run,
                schedule.duration.as_secs()
            );
            return Ok(Some(last));
        }
        run += 1;
        let remaining = schedule.duration.saturating_sub(start.elapsed());
        let pause = remaining.min(schedule.every);
        out.update_progress(|p| p.next_at = Some(Instant::now() + pause));
        tokio::time::sleep(pause).await;
    }
}

fn append(path: &Path, row: &str) -> Result<()> {
    let context = || format!("cannot append to {}", path.display());
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| ScopeError::io_context(e, &context()))?;
    file.write_all(row.as_bytes())
        .map_err(|e| ScopeError::io_context(e, &context()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::market::HealthThresholds;
    use scope::market::series::summary_metrics;

    fn metrics() -> Vec<Metric> {
        summary_metrics(&HealthThresholds::default())
    }

    fn output(run: u64, metrics: &[Metric]) -> RunOutput<u64> {
        RunOutput {
            sample: Sample {
                values: vec![Some(run as f64); metrics.len()],
                healthy: true,
                failed_checks: vec![],
            },
            snapshot: format!("report {run}\n"),
            value: run,
        }
    }

    #[test]
    fn test_total_counts_the_final_run_at_the_boundary() {
        // 5s every 2s runs at 0, 2, 4 and then at 5 (the sleep is cut to
        // the time left): four runs, not 5/2+1.
        let s = |every, duration| {
            Schedule::new(Duration::from_secs(every), Duration::from_secs(duration)).unwrap()
        };
        assert_eq!(s(2, 5).total(), 4);
        assert_eq!(s(1, 2).total(), 3);
        assert_eq!(s(2, 1).total(), 2);
    }

    #[test]
    fn test_a_zero_interval_is_the_users_error() {
        let err = Schedule::new(Duration::ZERO, Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, ScopeError::InvalidInput(_)), "{err:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn test_cli_prints_each_report_and_returns_the_last_value() {
        let m = metrics();
        let (out, cap) = Output::capture();
        let schedule = Schedule::new(Duration::from_secs(1), Duration::from_secs(2)).unwrap();
        let last = run(&schedule, &m, None, &out, |n| {
            let o = output(n, &m);
            async move { Ok(o) }
        })
        .await
        .unwrap();
        assert_eq!(last, Some(3));
        assert_eq!(cap.out(), "report 1\nreport 2\nreport 3\n");
        assert!(cap.err().contains("--- Run #3 at"), "{}", cap.err());
        assert!(
            cap.err().contains("Completed 3 run(s) over 2s."),
            "{}",
            cap.err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_tui_gets_records_and_no_repeated_reports() {
        let m = metrics();
        let (out, cap) = Output::capture_merged();
        let schedule = Schedule::new(Duration::from_secs(1), Duration::from_secs(1)).unwrap();
        run(&schedule, &m, None, &out, |n| {
            let o = output(n, &m);
            async move { Ok(o) }
        })
        .await
        .unwrap();
        assert!(!cap.out().contains("report 1"), "{}", cap.out());
        assert!(!cap.out().contains("--- Run #"), "{}", cap.out());
        let series = out.series_hook().unwrap();
        let series = series.lock().unwrap();
        assert_eq!(series.metrics, m);
        let runs: Vec<u64> = series.runs.iter().map(|r| r.run).collect();
        assert_eq!(runs, vec![1, 2]);
        assert_eq!(series.runs[1].snapshot, "report 2\n");
    }

    #[tokio::test(start_paused = true)]
    async fn test_an_error_ends_the_repeat() {
        let m = metrics();
        let (out, _cap) = Output::capture_merged();
        let schedule = Schedule::new(Duration::from_secs(1), Duration::from_secs(30)).unwrap();
        let mut calls = 0;
        let result: Result<Option<u64>> = run(&schedule, &m, None, &out, |n| {
            calls += 1;
            let o = output(n, &m);
            async move {
                if n == 2 {
                    Err(ScopeError::Network("venue down".into()))
                } else {
                    Ok(o)
                }
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 2, "no run after the error");
        let progress = out.progress_hook().unwrap().lock().unwrap().clone();
        assert_eq!(progress.run, 2);
        // The run before the error stays in the series.
        assert_eq!(out.series_hook().unwrap().lock().unwrap().runs.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn test_csv_gets_the_header_then_one_row_per_run() {
        let m = metrics();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.csv");
        let (out, _cap) = Output::capture();
        let schedule = Schedule::new(Duration::from_secs(1), Duration::from_secs(1)).unwrap();
        run(&schedule, &m, Some(&path), &out, |n| {
            let o = output(n, &m);
            async move { Ok(o) }
        })
        .await
        .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        let columns = series::csv_columns(&m).unwrap();
        assert_eq!(format!("{}\n", lines[0]), series::csv_header(&columns));
        assert_eq!(lines.len(), 3, "{body}");
        assert_eq!(lines[2].split(',').nth(1), Some("2"));
    }

    #[test]
    fn test_the_series_ring_keeps_the_newest_runs() {
        let m = metrics();
        let (out, _cap) = Output::capture_merged();
        let extra = 3;
        for n in 1..=(crate::cli::output::MAX_SERIES_RUNS as u64 + extra) {
            let o = output(n, &m);
            out.record_run(
                &m,
                RunRecord {
                    run: n,
                    at: chrono::Utc::now(),
                    sample: o.sample,
                    snapshot: o.snapshot,
                },
            );
        }
        let series = out.series_hook().unwrap();
        let series = series.lock().unwrap();
        assert_eq!(series.runs.len(), crate::cli::output::MAX_SERIES_RUNS);
        assert_eq!(series.dropped, extra);
        assert_eq!(series.runs.front().unwrap().run, extra + 1);
    }
}
