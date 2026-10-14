//! The one decision whether scope emits color. Every adapter asks here.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicU8, Ordering};

/// Override set by `--no-color`: 0 = none, 1 = off, 2 = on.
static COLOR_OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// Forces color off (`Some(false)`) or on (`Some(true)`) for the whole
/// process, or removes the override (`None`). `--no-color` uses it.
pub fn set_color_override(value: Option<bool>) {
    let code = match value {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    COLOR_OVERRIDE.store(code, Ordering::Relaxed);
}

/// Decides whether to emit color for one stream. Pure: the caller passes
/// the environment and whether the stream is a terminal.
///
/// Color is off when `NO_COLOR` is set and not empty (see no-color.org),
/// when `TERM` is `dumb`, or when the stream is not a terminal.
pub fn color_enabled_with(env: &dyn Fn(&str) -> Option<String>, stream_is_tty: bool) -> bool {
    if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if env("TERM").as_deref() == Some("dumb") {
        return false;
    }
    stream_is_tty
}

/// Applies the process override, then the environment rules.
fn color_enabled_for(stream_is_tty: bool) -> bool {
    match COLOR_OVERRIDE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => color_enabled_with(&|k| std::env::var(k).ok(), stream_is_tty),
    }
}

/// Returns `true` when output to stdout may use color.
pub fn color_enabled() -> bool {
    color_enabled_for(std::io::stdout().is_terminal())
}

/// Returns `true` when output to stderr may use color.
pub fn color_enabled_stderr() -> bool {
    color_enabled_for(std::io::stderr().is_terminal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_enabled_rules() {
        let env = |m: &'static [(&'static str, &'static str)]| {
            move |k: &str| m.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert!(color_enabled_with(&env(&[]), true));
        assert!(!color_enabled_with(&env(&[("NO_COLOR", "1")]), true));
        // no-color.org: only a non-empty NO_COLOR disables color.
        assert!(color_enabled_with(&env(&[("NO_COLOR", "")]), true));
        assert!(!color_enabled_with(&env(&[("TERM", "dumb")]), true));
        assert!(!color_enabled_with(&env(&[]), false));
    }

    #[test]
    fn test_color_override_beats_a_tty_and_none_restores() {
        // The override is process-global. Never set Some(true): parallel
        // tests run the plain wrappers, which expect uncolored output.
        set_color_override(Some(false));
        assert!(!color_enabled_for(true));
        set_color_override(None);
        let plain = color_enabled_with(&|k| std::env::var(k).ok(), true);
        assert_eq!(color_enabled_for(true), plain);
    }
}
