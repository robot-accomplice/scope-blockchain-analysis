//! The text adapter of the theme: it renders a [`Tone`] as ANSI SGR escape
//! codes around a string, for CLI output.
//!
//! Each part of a tone wraps the text on its own, the color outermost and
//! the effects inside it. A color ends with "default foreground"; an
//! effect ends with "reset all". This keeps the bytes that the `colored`
//! style crates wrote before the theme existed.

use super::decision::{color_enabled, color_enabled_stderr};
use super::{AnsiColor, Role, Tone, tone};

const CSI: &str = "\x1b[";
const SGR_END: &str = "m";
const RESET_ALL: u8 = 0;
const BOLD: u8 = 1;
const DIM: u8 = 2;
const UNDERLINE: u8 = 4;
const REVERSE: u8 = 7;
const DEFAULT_FOREGROUND: u8 = 39;

/// Paints text for one output stream: with escape codes when color is on
/// for it, as the text unchanged when color is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Painter {
    enabled: bool,
}

impl Painter {
    /// A painter that paints when `enabled` is true. Use it when the
    /// caller already holds the color decision.
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// A painter for stdout, following [`color_enabled`].
    pub fn stdout() -> Self {
        Self::new(color_enabled())
    }

    /// A painter for stderr, following [`color_enabled_stderr`].
    pub fn stderr() -> Self {
        Self::new(color_enabled_stderr())
    }

    /// `text` in the tone of `role`.
    pub fn paint(self, role: Role, text: &str) -> String {
        self.paint_tone(tone(role), text)
    }

    /// `text` in the strong (bold) form of `role`.
    pub fn paint_strong(self, role: Role, text: &str) -> String {
        self.paint_tone(tone(role).strong(), text)
    }

    /// `text` in `tone`, for tones that do not come from a role (the
    /// monitor schemes).
    pub fn paint_tone(self, tone: Tone, text: &str) -> String {
        if self.enabled {
            render(tone, text)
        } else {
            text.to_string()
        }
    }
}

/// `text` in the tone of `role`, for stdout.
pub fn paint(role: Role, text: &str) -> String {
    Painter::stdout().paint(role, text)
}

/// Wraps `text` in the escape codes of `tone`.
fn render(tone: Tone, text: &str) -> String {
    let effects = [
        (tone.reverse, REVERSE),
        (tone.underline, UNDERLINE),
        (tone.dim, DIM),
        (tone.bold, BOLD),
    ];
    let mut out = text.to_string();
    for (_, code) in effects.into_iter().filter(|(on, _)| *on) {
        out = wrap(code, &out, RESET_ALL);
    }
    match tone.color {
        Some(color) => wrap(foreground_code(color), &out, DEFAULT_FOREGROUND),
        None => out,
    }
}

fn wrap(start: u8, text: &str, end: u8) -> String {
    format!("{CSI}{start}{SGR_END}{text}{CSI}{end}{SGR_END}")
}

/// The SGR foreground code of a named color.
const fn foreground_code(color: AnsiColor) -> u8 {
    match color {
        AnsiColor::Black => 30,
        AnsiColor::Red => 31,
        AnsiColor::Green => 32,
        AnsiColor::Yellow => 33,
        AnsiColor::Blue => 34,
        AnsiColor::Magenta => 35,
        AnsiColor::Cyan => 36,
        AnsiColor::White => 37,
        AnsiColor::BrightBlack => 90,
        AnsiColor::BrightRed => 91,
        AnsiColor::BrightGreen => 92,
        AnsiColor::BrightYellow => 93,
        AnsiColor::BrightBlue => 94,
        AnsiColor::BrightMagenta => 95,
        AnsiColor::BrightCyan => 96,
        AnsiColor::BrightWhite => 97,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Painter = Painter::new(true);
    const OFF: Painter = Painter::new(false);

    #[test]
    fn color_off_leaves_every_role_as_plain_text() {
        // The plain output (NO_COLOR, pipes) must never carry an escape.
        for role in Role::ALL {
            assert_eq!(OFF.paint(role, "x"), "x", "{role:?}");
            assert_eq!(OFF.paint_strong(role, "x"), "x", "{role:?}");
        }
    }

    #[test]
    fn color_on_marks_every_role_but_neutral() {
        for role in Role::ALL {
            let painted = ON.paint(role, "x");
            assert_eq!(painted == "x", role == Role::Neutral, "{role:?}");
        }
    }

    #[test]
    fn a_color_closes_with_default_foreground_and_effects_reset_inside_it() {
        assert_eq!(ON.paint(Role::Accent, "x"), "\x1b[36mx\x1b[39m");
        assert_eq!(ON.paint(Role::Secondary, "x"), "\x1b[2mx\x1b[0m");
        assert_eq!(
            ON.paint_strong(Role::Err, "x"),
            "\x1b[31m\x1b[1mx\x1b[0m\x1b[39m"
        );
        assert_eq!(
            ON.paint(Role::Selected, "x"),
            "\x1b[36m\x1b[7mx\x1b[0m\x1b[39m"
        );
    }

    #[test]
    fn bright_colors_use_the_high_intensity_codes() {
        let tone = Tone {
            color: Some(AnsiColor::BrightBlack),
            ..Tone::PLAIN
        };
        assert_eq!(ON.paint_tone(tone, "x"), "\x1b[90mx\x1b[39m");
    }
}
