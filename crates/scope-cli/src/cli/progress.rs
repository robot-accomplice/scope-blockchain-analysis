//! # Progress Indicators
//!
//! Shared progress display utilities for long-running CLI operations.
//! Uses `indicatif` spinners and progress bars, respecting `--no-color`
//! and non-TTY contexts (e.g. pipes).
//!
//! ## Usage
//!
//! ```rust,ignore
//! use scope_cli::cli::{output::Output, progress::Spinner};
//!
//! let out = Output::stdio();
//! let sp = Spinner::new("Fetching address data...", &out);
//! // ... do work ...
//! sp.finish("Address data loaded.");
//! ```

use crate::cli::output::Output;
use crate::errln;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// A simple spinner for single-step or short sequential operations.
///
/// Automatically disables itself when stderr is not a TTY (e.g. piped output)
/// or when the output does not allow progress (e.g. inside the TUI). A hidden
/// spinner writes its messages as plain lines to the output's error channel.
pub struct Spinner {
    bar: ProgressBar,
    output: Output,
}

impl Spinner {
    /// Creates and starts a spinner with the given message.
    ///
    /// # Errors
    ///
    /// Returns the output's error when the status line cannot be written.
    pub fn new(message: &str, output: &Output) -> std::io::Result<Self> {
        let bar = if output.progress_enabled() && atty_stderr() {
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
            // No animation: write a simple status line instead
            errln!(output, "{}", message)?;
            ProgressBar::hidden()
        };
        Ok(Self {
            bar,
            output: output.clone(),
        })
    }

    /// Updates the spinner message in-place.
    pub fn set_message(&self, message: impl Into<String>) {
        self.bar.set_message(message.into());
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

    /// Prints a line above the spinner without garbling the animation.
    ///
    /// # Errors
    ///
    /// Returns the output's error when the spinner is hidden and the line
    /// cannot be written.
    ///
    /// Uses `indicatif`'s built-in `println` which clears the spinner line,
    /// writes the message, then redraws the spinner below it.
    pub fn println(&self, message: &str) -> std::io::Result<()> {
        if self.bar.is_hidden() {
            errln!(self.output, "{}", message)?;
        } else {
            self.bar.println(message);
        }
        Ok(())
    }

    /// Temporarily suspends the spinner while running a closure, returning its result.
    ///
    /// The spinner is paused (line cleared) before `f` runs and resumed after.
    /// Useful when other code needs to print to the terminal.
    pub fn suspend<R, F: FnOnce() -> R>(&self, f: F) -> R {
        if self.bar.is_hidden() {
            f()
        } else {
            self.bar.suspend(f)
        }
    }
}

/// A counted progress bar for multi-step operations (X of Y).
pub struct StepProgress {
    bar: ProgressBar,
}

impl StepProgress {
    /// Creates a progress bar for `total` steps with the given prefix.
    ///
    /// # Errors
    ///
    /// Returns the output's error when the status line cannot be written.
    pub fn new(total: u64, prefix: &str, output: &Output) -> std::io::Result<Self> {
        let bar = if output.progress_enabled() && atty_stderr() {
            let pb = ProgressBar::new(total);
            pb.set_style(
                ProgressStyle::with_template("{prefix} [{bar:30.cyan/dim}] {pos}/{len} {msg}")
                    .unwrap()
                    .progress_chars("━━╸"),
            );
            pb.set_prefix(prefix.to_string());
            pb
        } else {
            errln!(output, "{} (0/{})", prefix, total)?;
            ProgressBar::hidden()
        };
        Ok(Self { bar })
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

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn test_hidden_spinner_writes_status_to_err_channel() {
        // Inside the TUI the spinner must not draw; its message must still
        // reach the user through the error channel, never through data.
        let (o, cap) = Output::capture();
        let sp = Spinner::new("Fetching...", &o).unwrap();
        sp.println("note").unwrap();
        sp.finish("done");
        assert_eq!(cap.err(), "Fetching...\nnote\n");
        assert_eq!(cap.out(), "");
    }

    #[test]
    fn test_spinner_create_and_finish() {
        // In test context, stderr is not a TTY, so spinner is hidden
        let sp = Spinner::new("Testing...", &quiet()).unwrap();
        sp.set_message("Updated");
        sp.finish("Done");
    }

    #[test]
    fn test_spinner_finish_and_clear() {
        let sp = Spinner::new("Testing...", &quiet()).unwrap();
        sp.finish_and_clear();
    }

    #[test]
    fn test_spinner_finish_warn() {
        let sp = Spinner::new("Testing...", &quiet()).unwrap();
        sp.finish_warn("Warning");
    }

    #[test]
    fn test_step_progress_create_and_finish() {
        let prog = StepProgress::new(3, "Processing", &quiet()).unwrap();
        prog.inc("Step 1");
        prog.inc("Step 2");
        prog.inc("Step 3");
        prog.finish("Done");
    }

    #[test]
    fn test_spinner_multiple_set_message() {
        let sp = Spinner::new("Initial", &quiet()).unwrap();
        sp.set_message("First update");
        sp.set_message("Second update");
        sp.set_message("Third update");
        sp.finish("Complete");
    }

    #[test]
    fn test_step_progress_multiple_inc() {
        let prog = StepProgress::new(5, "Processing", &quiet()).unwrap();
        prog.inc("Step 1");
        prog.inc("Step 2");
        prog.inc("Step 3");
        prog.inc("Step 4");
        prog.inc("Step 5");
        prog.finish("Done");
    }

    #[test]
    fn test_step_progress_single_step() {
        let prog = StepProgress::new(1, "Single", &quiet()).unwrap();
        prog.inc("Only step");
        prog.finish("Complete");
    }

    #[test]
    fn test_step_progress_large_total() {
        let prog = StepProgress::new(100, "Large", &quiet()).unwrap();
        for i in 1..=100 {
            prog.inc(&format!("Step {}", i));
        }
        prog.finish("Complete");
    }

    #[test]
    fn test_step_progress_zero_total() {
        let prog = StepProgress::new(0, "Empty", &quiet()).unwrap();
        prog.finish("Complete");
    }

    #[test]
    fn test_spinner_finish_warn_with_message() {
        let sp = Spinner::new("Warning test", &quiet()).unwrap();
        sp.finish_warn("Something went wrong");
    }

    #[test]
    fn test_spinner_finish_warn_multiple_calls() {
        let sp = Spinner::new("Test", &quiet()).unwrap();
        sp.finish_warn("First warning");
        // finish_warn can be called multiple times (though unusual)
        sp.finish_warn("Second warning");
    }

    #[test]
    fn test_spinner_println() {
        let sp = Spinner::new("Working...", &quiet()).unwrap();
        sp.println("A message above the spinner").unwrap();
        sp.finish("Done");
    }

    #[test]
    fn test_spinner_suspend() {
        let sp = Spinner::new("Working...", &quiet()).unwrap();
        sp.suspend(|| {
            // Code that prints directly
            eprintln!("Suspended output");
        });
        sp.finish("Done");
    }
}
