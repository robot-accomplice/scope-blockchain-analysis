//! # Progress Indicators
//!
//! Shared progress display utilities for long-running CLI operations.
//! Uses `indicatif` spinners and progress bars, respecting `--no-color`
//! and non-TTY contexts (e.g. pipes).
//!
//! ## Usage
//!
//! ```rust,ignore
//! use scope::cli::progress::Spinner;
//!
//! let sp = Spinner::new("Fetching address data...");
//! // ... do work ...
//! sp.finish("Address data loaded.");
//! ```

use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// A simple spinner for single-step or short sequential operations.
///
/// Automatically disables itself when stdout is not a TTY (e.g. piped output).
pub struct Spinner {
    bar: ProgressBar,
}

impl Spinner {
    /// Creates and starts a spinner with the given message.
    pub fn new(message: &str) -> Self {
        let bar = if atty_stderr() {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("{spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message(message.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        } else {
            // Non-TTY: print a simple status line to stderr instead
            eprintln!("{}", message);
            ProgressBar::hidden()
        };
        Self { bar }
    }

    /// Updates the spinner message in-place.
    pub fn set_message(&self, message: &str) {
        self.bar.set_message(message.to_string());
    }

    /// Finishes the spinner with a success message (checkmark).
    pub fn finish(&self, message: &str) {
        if !self.bar.is_hidden() {
            self.bar.finish_with_message(format!("✓ {}", message));
        }
    }

    /// Finishes the spinner with a warning message.
    pub fn finish_warn(&self, message: &str) {
        if !self.bar.is_hidden() {
            self.bar.finish_with_message(format!("⚠ {}", message));
        }
    }

    /// Finishes and clears the spinner line (no residual output).
    pub fn finish_and_clear(&self) {
        self.bar.finish_and_clear();
    }
}

/// A counted progress bar for multi-step operations (X of Y).
pub struct StepProgress {
    bar: ProgressBar,
}

impl StepProgress {
    /// Creates a progress bar for `total` steps with the given prefix.
    pub fn new(total: u64, prefix: &str) -> Self {
        let bar = if atty_stderr() {
            let pb = ProgressBar::new(total);
            pb.set_style(
                ProgressStyle::with_template("{prefix} [{bar:30.cyan/dim}] {pos}/{len} {msg}")
                    .unwrap()
                    .progress_chars("━━╸"),
            );
            pb.set_prefix(prefix.to_string());
            pb
        } else {
            eprintln!("{} (0/{})", prefix, total);
            ProgressBar::hidden()
        };
        Self { bar }
    }

    /// Increments progress by one and updates the message.
    pub fn inc(&self, message: &str) {
        self.bar.set_message(message.to_string());
        self.bar.inc(1);
    }

    /// Finishes the progress bar with a success message.
    pub fn finish(&self, message: &str) {
        if !self.bar.is_hidden() {
            self.bar.finish_with_message(format!("✓ {}", message));
        }
    }
}

/// Checks if stderr is a TTY (interactive terminal).
fn atty_stderr() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_create_and_finish() {
        // In test context, stderr is not a TTY, so spinner is hidden
        let sp = Spinner::new("Testing...");
        sp.set_message("Updated");
        sp.finish("Done");
    }

    #[test]
    fn test_spinner_finish_and_clear() {
        let sp = Spinner::new("Testing...");
        sp.finish_and_clear();
    }

    #[test]
    fn test_spinner_finish_warn() {
        let sp = Spinner::new("Testing...");
        sp.finish_warn("Warning");
    }

    #[test]
    fn test_step_progress_create_and_finish() {
        let prog = StepProgress::new(3, "Processing");
        prog.inc("Step 1");
        prog.inc("Step 2");
        prog.inc("Step 3");
        prog.finish("Done");
    }
}
