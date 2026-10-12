//! Evidence of a failed scenario (design 8.1).

use crate::run::Outcome;
use crate::session::{Session, data_dir};
use anyhow::Result;
use std::path::Path;

pub fn save(dir: &Path, out: &Outcome, sess: &Session, home: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("screen.txt"), sess.screen())?;
    std::fs::write(dir.join("pty.raw"), sess.raw())?;
    let events = data_dir(home).join("tui-events.jsonl");
    if events.exists() {
        std::fs::copy(&events, dir.join("tui-events.jsonl"))?;
    }
    let steps: Vec<String> = out
        .steps
        .iter()
        .map(|s| serde_json::to_string(s).unwrap_or_default())
        .collect();
    std::fs::write(dir.join("steps.jsonl"), steps.join("\n"))?;
    std::fs::write(
        dir.join("failure.txt"),
        out.failure.clone().unwrap_or_default(),
    )?;
    Ok(())
}
