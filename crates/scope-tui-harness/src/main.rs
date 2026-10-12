use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use scope_tui_harness::{
    evidence,
    report::{self, Line, Result3},
    run::{RunCtx, run_scenario},
    scenario::{self, Tier},
};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(about = "Runs YAML scenarios against the real scope TUI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum TierArg {
    Offline,
    Live,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run scenarios.
    Run {
        #[arg(long, value_enum, default_value = "offline")]
        tier: TierArg,
        #[arg(long, default_value_t = std::thread::available_parallelism().map_or(2, |n| n.get()))]
        jobs: usize,
        #[arg(long)]
        keep: bool,
        #[arg(long, default_value = "tests/tui/scenarios")]
        scenarios: PathBuf,
        #[arg(long, default_value = "target/debug/scope")]
        binary: PathBuf,
        /// Scenario names (file stems). All when empty.
        names: Vec<String>,
    },
}

fn main() -> Result<()> {
    let Cmd::Run {
        tier,
        jobs,
        keep,
        scenarios,
        binary,
        names,
    } = Cli::parse().cmd;
    let tier = match tier {
        TierArg::Offline => Tier::Offline,
        TierArg::Live => Tier::Live,
    };
    let binary = std::fs::canonicalize(&binary)
        .map_err(|e| anyhow::anyhow!("scope binary {}: {e}", binary.display()))?;
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .to_string();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&scenarios)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .filter(|p| {
            names.is_empty()
                || names
                    .iter()
                    .any(|n| p.file_stem().is_some_and(|s| s == n.as_str()))
        })
        .collect();
    files.sort();
    let lines = std::sync::Mutex::new(Vec::new());
    let queue = std::sync::Mutex::new(files);
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let Some(path) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let id = path.file_stem().unwrap().to_string_lossy().to_string();
                    let line = match scenario::load(&path) {
                        Err(e) => {
                            eprintln!("{id}: {e:#}");
                            Line {
                                name: id,
                                tier,
                                result: Result3::Fail,
                                ms: 0,
                            }
                        }
                        Ok(s) if !s.tiers.contains(&tier) => Line {
                            name: id,
                            tier,
                            result: Result3::Skip,
                            ms: 0,
                        },
                        Ok(s) => {
                            let ctx = RunCtx {
                                binary: binary.clone(),
                                fixtures_root: "tests/tui/fixtures".into(),
                                snapshots_root: "tests/tui/snapshots".into(),
                                scenario_id: id.clone(),
                            };
                            let t = Instant::now();
                            match run_scenario(&s, tier, &ctx) {
                                Ok((out, sess, home)) => {
                                    if !out.passed {
                                        let dir = PathBuf::from("target/tui-harness")
                                            .join(&run_id)
                                            .join(&id);
                                        if let Err(e) =
                                            evidence::save(&dir, &out, &sess, home.path())
                                        {
                                            eprintln!("{id}: evidence save failed: {e:#}");
                                        }
                                        eprintln!(
                                            "{id}: {}\n  evidence: {}",
                                            out.failure.clone().unwrap_or_default(),
                                            dir.display()
                                        );
                                    }
                                    if keep {
                                        let _ = home.keep();
                                    }
                                    Line {
                                        name: id,
                                        tier,
                                        result: if out.passed {
                                            Result3::Pass
                                        } else {
                                            Result3::Fail
                                        },
                                        ms: t.elapsed().as_millis(),
                                    }
                                }
                                Err(e) => {
                                    eprintln!("{id}: {e:#}");
                                    Line {
                                        name: id,
                                        tier,
                                        result: Result3::Fail,
                                        ms: t.elapsed().as_millis(),
                                    }
                                }
                            }
                        }
                    };
                    lines.lock().unwrap().push(line);
                }
            });
        }
    });
    let mut lines = lines.into_inner().unwrap();
    lines.sort_by(|a, b| a.name.cmp(&b.name));
    std::process::exit(if report::print(&lines) { 0 } else { 1 });
}
