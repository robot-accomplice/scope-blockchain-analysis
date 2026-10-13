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
//!
//! ## Prompts
//!
//! A bar that draws on the terminal is registered here. [`paused`] stops
//! every registered bar while it runs a closure, so an interactive prompt
//! ([`crate::cli::prompt::ask`]) is never drawn over. Lines a command
//! prints next to a bar ([`Spinner::println`]) go through its [`Output`],
//! so the TUI keeps a copy; the bar's own finish line is progress and is
//! not copied.

use crate::cli::output::Output;
use crate::note;
use indicatif::{ProgressBar, ProgressStyle, WeakProgressBar};
use scope::display::terminal::color_enabled_stderr;
use std::sync::Mutex;
use std::time::Duration;

/// The bars that draw on the terminal now. Weak: a dropped bar leaves.
static DRAWING: Mutex<Vec<WeakProgressBar>> = Mutex::new(Vec::new());

/// Adds a drawing bar to [`DRAWING`] and drops the ones that ended.
fn register(bar: &ProgressBar) {
    let mut bars = DRAWING.lock().unwrap_or_else(|e| e.into_inner());
    bars.retain(|weak| weak.upgrade().is_some_and(|b| !b.is_finished()));
    bars.push(bar.downgrade());
}

/// Runs `f` with every drawing bar stopped and its line cleared. The bars
/// draw again when `f` returns. No bar draws while `f` runs: the bar's
/// tick waits for its lock.
pub(crate) fn paused<R>(f: impl FnOnce() -> R) -> R {
    // Copy the bars out: the registry lock is not held while `f` runs.
    let bars: Vec<ProgressBar> = DRAWING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(WeakProgressBar::upgrade)
        .filter(|b| !b.is_finished())
        .collect();
    paused_all(&bars, f)
}

fn paused_all<R>(bars: &[ProgressBar], f: impl FnOnce() -> R) -> R {
    match bars.split_first() {
        None => f(),
        Some((bar, rest)) => bar.suspend(|| paused_all(rest, f)),
    }
}

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
    /// Creates and starts a spinner with the given message. A hidden
    /// spinner writes the message as a note.
    pub fn new(message: &str, output: &Output) -> Self {
        let bar = if output.progress_enabled() && atty_stderr() {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template(if color_enabled_stderr() {
                    "{spinner:.cyan} {msg}"
                } else {
                    "{spinner} {msg}"
                })
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message(message.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            register(&pb);
            pb
        } else {
            // No animation: write a simple status line instead
            note!(output, "{}", message);
            ProgressBar::hidden()
        };
        Self {
            bar,
            output: output.clone(),
        }
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

    /// Writes a note above the spinner. The spinner's line is cleared
    /// while the note is written and drawn again below it. The note goes
    /// through the [`Output`], so a TUI run keeps a copy.
    pub fn println(&self, message: &str) {
        self.bar.suspend(|| note!(self.output, "{}", message));
    }
}

/// A status that passes: the spinner's message when a spinner draws,
/// otherwise a note.
pub(crate) fn status(spinner: Option<&Spinner>, out: &Output, message: &str) {
    match spinner {
        Some(sp) => sp.set_message(message.to_string()),
        None => note!(out, "{}", message),
    }
}

/// A status line that stays: above the spinner when one draws, otherwise
/// a note.
pub(crate) fn notice(spinner: Option<&Spinner>, out: &Output, message: &str) {
    match spinner {
        Some(sp) => sp.println(message),
        None => note!(out, "{}", message),
    }
}

/// A counted progress bar for multi-step operations (X of Y).
pub struct StepProgress {
    bar: ProgressBar,
}

impl StepProgress {
    /// Creates a progress bar for `total` steps with the given prefix. A
    /// hidden bar writes the prefix as a note.
    pub fn new(total: u64, prefix: &str, output: &Output) -> Self {
        let bar = if output.progress_enabled() && atty_stderr() {
            let pb = ProgressBar::new(total);
            pb.set_style(
                ProgressStyle::with_template(if color_enabled_stderr() {
                    "{prefix} [{bar:30.cyan/dim}] {pos}/{len} {msg}"
                } else {
                    "{prefix} [{bar:30}] {pos}/{len} {msg}"
                })
                .unwrap()
                .progress_chars("━━╸"),
            );
            pb.set_prefix(prefix.to_string());
            register(&pb);
            pb
        } else {
            note!(output, "{} (0/{})", prefix, total);
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

/// Decides whether a progress bar may draw. Pure: the caller passes the
/// environment and whether stderr is a terminal.
///
/// A bar needs a terminal that can redraw a line, so `TERM=dumb` turns it
/// off. `NO_COLOR` does not: it governs color only.
fn progress_bar_ok(env: &dyn Fn(&str) -> Option<String>, stderr_is_tty: bool) -> bool {
    stderr_is_tty && env("TERM").as_deref() != Some("dumb")
}

/// Checks if stderr is a terminal that can show a progress bar.
fn atty_stderr() -> bool {
    use std::io::IsTerminal;
    progress_bar_ok(&|k| std::env::var(k).ok(), std::io::stderr().is_terminal())
}

// ============================================================================
// Unit Tests
// ============================================================================

/// Seams for tests that need a bar that draws, here and in the prompt
/// tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A terminal that records what a bar draws.
    #[derive(Clone, Default, Debug)]
    pub(crate) struct Screen(std::sync::Arc<Mutex<Vec<String>>>);

    impl Screen {
        pub(crate) fn ops(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
        fn push(&self, op: String) -> std::io::Result<()> {
            self.0.lock().unwrap().push(op);
            Ok(())
        }
    }

    impl indicatif::TermLike for Screen {
        fn width(&self) -> u16 {
            80
        }
        fn move_cursor_up(&self, n: usize) -> std::io::Result<()> {
            self.push(format!("up {n}"))
        }
        fn move_cursor_down(&self, n: usize) -> std::io::Result<()> {
            self.push(format!("down {n}"))
        }
        fn move_cursor_right(&self, n: usize) -> std::io::Result<()> {
            self.push(format!("right {n}"))
        }
        fn move_cursor_left(&self, n: usize) -> std::io::Result<()> {
            self.push(format!("left {n}"))
        }
        fn write_line(&self, s: &str) -> std::io::Result<()> {
            self.push(format!("line {s}"))
        }
        fn write_str(&self, s: &str) -> std::io::Result<()> {
            self.push(format!("str {s}"))
        }
        fn clear_line(&self) -> std::io::Result<()> {
            self.push("clear".into())
        }
        fn flush(&self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A spinner that draws on `screen`, as one on a real terminal does.
    pub(crate) fn drawing_spinner(message: &str, output: &Output, screen: &Screen) -> Spinner {
        let bar = ProgressBar::with_draw_target(
            None,
            indicatif::ProgressDrawTarget::term_like(Box::new(screen.clone())),
        );
        bar.set_style(ProgressStyle::with_template("{spinner} {msg}").unwrap());
        bar.set_message(message.to_string());
        bar.enable_steady_tick(Duration::from_millis(5));
        register(&bar);
        Spinner {
            bar,
            output: output.clone(),
        }
    }

    /// Serializes the tests that use the process-wide bar registry.
    pub(crate) static REGISTRY_LOCK: Mutex<()> = Mutex::new(());
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn test_progress_bar_rules() {
        let env = |m: &'static [(&'static str, &'static str)]| {
            move |k: &str| m.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert!(progress_bar_ok(&env(&[]), true));
        // NO_COLOR governs color, not the bar.
        assert!(progress_bar_ok(&env(&[("NO_COLOR", "1")]), true));
        assert!(!progress_bar_ok(&env(&[("TERM", "dumb")]), true));
        assert!(!progress_bar_ok(&env(&[]), false));
    }

    fn quiet() -> Output {
        Output::capture().0
    }

    #[test]
    fn test_hidden_spinner_writes_status_to_err_channel() {
        // Inside the TUI the spinner must not draw; its message must still
        // reach the user through the error channel, never through data.
        let (o, cap) = Output::capture();
        let sp = Spinner::new("Fetching...", &o);
        sp.println("note");
        sp.finish("done");
        assert_eq!(cap.err(), "Fetching...\nnote\n");
        assert_eq!(cap.out(), "");
    }

    #[test]
    fn test_spinner_create_and_finish() {
        // In test context, stderr is not a TTY, so spinner is hidden
        let sp = Spinner::new("Testing...", &quiet());
        sp.set_message("Updated");
        sp.finish("Done");
    }

    #[test]
    fn test_spinner_finish_and_clear() {
        let sp = Spinner::new("Testing...", &quiet());
        sp.finish_and_clear();
    }

    #[test]
    fn test_spinner_finish_warn() {
        let sp = Spinner::new("Testing...", &quiet());
        sp.finish_warn("Warning");
    }

    #[test]
    fn test_step_progress_create_and_finish() {
        let prog = StepProgress::new(3, "Processing", &quiet());
        prog.inc("Step 1");
        prog.inc("Step 2");
        prog.inc("Step 3");
        prog.finish("Done");
    }

    #[test]
    fn test_spinner_multiple_set_message() {
        let sp = Spinner::new("Initial", &quiet());
        sp.set_message("First update");
        sp.set_message("Second update");
        sp.set_message("Third update");
        sp.finish("Complete");
    }

    #[test]
    fn test_step_progress_multiple_inc() {
        let prog = StepProgress::new(5, "Processing", &quiet());
        prog.inc("Step 1");
        prog.inc("Step 2");
        prog.inc("Step 3");
        prog.inc("Step 4");
        prog.inc("Step 5");
        prog.finish("Done");
    }

    #[test]
    fn test_step_progress_single_step() {
        let prog = StepProgress::new(1, "Single", &quiet());
        prog.inc("Only step");
        prog.finish("Complete");
    }

    #[test]
    fn test_step_progress_large_total() {
        let prog = StepProgress::new(100, "Large", &quiet());
        for i in 1..=100 {
            prog.inc(&format!("Step {}", i));
        }
        prog.finish("Complete");
    }

    #[test]
    fn test_step_progress_zero_total() {
        let prog = StepProgress::new(0, "Empty", &quiet());
        prog.finish("Complete");
    }

    #[test]
    fn test_spinner_finish_warn_with_message() {
        let sp = Spinner::new("Warning test", &quiet());
        sp.finish_warn("Something went wrong");
    }

    #[test]
    fn test_spinner_finish_warn_multiple_calls() {
        let sp = Spinner::new("Test", &quiet());
        sp.finish_warn("First warning");
        // finish_warn can be called multiple times (though unusual)
        sp.finish_warn("Second warning");
    }

    #[test]
    fn test_spinner_println() {
        let sp = Spinner::new("Working...", &quiet());
        sp.println("A message above the spinner");
        sp.finish("Done");
    }

    #[test]
    fn test_paused_stops_every_drawing_bar_while_it_runs() {
        // A prompt shown while a spinner ticks is drawn over: the spinner
        // must clear its line and stay away until the prompt is answered.
        let _lock = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let screen = Screen::default();
        let sp = drawing_spinner("Searching...", &quiet(), &screen);
        std::thread::sleep(Duration::from_millis(30));
        let drawn_during = paused(|| {
            let at_start = screen.ops();
            let last = |op: &str| at_start.iter().rposition(|o| o.starts_with(op));
            assert!(last("clear") > last("str"), "the line is cleared");
            std::thread::sleep(Duration::from_millis(40));
            screen.ops().len() - at_start.len()
        });
        assert_eq!(drawn_during, 0, "the spinner drew while paused");
        std::thread::sleep(Duration::from_millis(30));
        let after = screen.ops();
        assert!(
            after[after.len() - 4..]
                .iter()
                .any(|op| op.contains("Searching...")),
            "it draws again after: {after:?}"
        );
        sp.finish_and_clear();
    }

    #[test]
    fn test_paused_skips_ended_bars_and_runs_without_any() {
        let _lock = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let screen = Screen::default();
        let sp = drawing_spinner("x", &quiet(), &screen);
        sp.finish("done");
        let before = screen.ops().len();
        assert_eq!(paused(|| 7), 7);
        assert_eq!(screen.ops().len(), before, "an ended bar keeps its line");
        drop(sp);
        assert_eq!(paused(|| 8), 8);
    }

    #[test]
    fn test_println_beside_a_drawing_spinner_goes_through_the_output() {
        // In a TUI run outside the TUI, the copy for the scrollback comes
        // from the Output. A line the bar printed itself never reached it.
        let _lock = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let screen = Screen::default();
        let (o, cap) = Output::capture();
        let sp = drawing_spinner("Searching...", &o, &screen);
        sp.println("Not found on DexScreener; found X on Y (CEX)");
        assert_eq!(cap.err(), "Not found on DexScreener; found X on Y (CEX)\n");
        assert!(
            !screen.ops().iter().any(|op| op.contains("DexScreener")),
            "not drawn by the bar"
        );
        sp.finish_and_clear();
    }
}
