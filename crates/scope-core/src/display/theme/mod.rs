//! # Display theme
//!
//! The one source of every color and text style that scope shows in a
//! terminal: CLI output, the TUI and the monitor. Callers name a semantic
//! [`Role`]; [`tone`] is the single mapping from a role to its look.
//!
//! This module is presentation policy. It holds no framework type: the
//! adapters translate a [`Tone`] to their medium (the text painter in
//! `text` for CLI output; ratatui styles in the CLI crate).
//!
//! Only the 16 named ANSI colors are used, so the terminal's own palette
//! decides the exact shade on dark and light themes.

mod decision;
mod text;

pub use decision::{color_enabled, color_enabled_stderr, color_enabled_with, set_color_override};
pub use text::{Painter, paint};

/// The monitor's color schemes. The configuration type is the scheme, so
/// one enum names the schemes everywhere.
pub use crate::config::ColorScheme as Scheme;

/// What a piece of text means. Callers pick a role; never a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Structure and titles: box rules, section and table headers.
    Accent,
    /// Key names in hints and help.
    Key,
    /// A passed check, a gain, a healthy state.
    Ok,
    /// A warning: outside its target, but not a failure.
    Warn,
    /// A failure, a loss, an unhealthy state.
    Err,
    /// The glyph of an informational note.
    Info,
    /// Labels, descriptions and notes that sit behind the main text.
    Secondary,
    /// Plain text in the terminal's default style.
    Neutral,
    /// The selected row of a list.
    Selected,
    /// A search match.
    Match,
    /// A URL.
    Link,
}

impl Role {
    /// Every role, for adapters and tests that walk the whole theme.
    pub const ALL: [Role; 11] = [
        Role::Accent,
        Role::Key,
        Role::Ok,
        Role::Warn,
        Role::Err,
        Role::Info,
        Role::Secondary,
        Role::Neutral,
        Role::Selected,
        Role::Match,
        Role::Link,
    ];
}

/// The 16 named ANSI foreground colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

/// How a role looks: a foreground color (`None` keeps the terminal's
/// default) and text effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tone {
    /// The foreground color; `None` keeps the terminal default.
    pub color: Option<AnsiColor>,
    /// Bold (heavier) text.
    pub bold: bool,
    /// Dim (fainter) text.
    pub dim: bool,
    /// Underlined text.
    pub underline: bool,
    /// Foreground and background swapped.
    pub reverse: bool,
}

impl Tone {
    /// The terminal's default style: no color, no effect.
    pub const PLAIN: Tone = Tone {
        color: None,
        bold: false,
        dim: false,
        underline: false,
        reverse: false,
    };

    /// A tone with only a foreground color.
    const fn colored(color: AnsiColor) -> Tone {
        Tone {
            color: Some(color),
            ..Tone::PLAIN
        }
    }

    /// The same tone in bold: the strong form of a role, for verdicts
    /// (`HEALTHY`, `Critical`) and headings.
    pub const fn strong(self) -> Tone {
        Tone { bold: true, ..self }
    }
}

/// The single mapping from a role to its tone. The role table in
/// `docs/design/2026-10-13-tui-color-theme.md` is its specification.
pub const fn tone(role: Role) -> Tone {
    match role {
        Role::Accent => Tone::colored(AnsiColor::Cyan),
        Role::Key => Tone::colored(AnsiColor::Magenta).strong(),
        Role::Ok => Tone::colored(AnsiColor::Green),
        Role::Warn => Tone::colored(AnsiColor::Yellow),
        Role::Err => Tone::colored(AnsiColor::Red),
        Role::Info => Tone::colored(AnsiColor::Blue),
        Role::Secondary => Tone {
            dim: true,
            ..Tone::PLAIN
        },
        Role::Neutral => Tone::PLAIN,
        Role::Selected => Tone {
            reverse: true,
            ..Tone::colored(AnsiColor::Cyan)
        },
        Role::Match => Tone {
            reverse: true,
            ..Tone::colored(AnsiColor::Yellow)
        },
        Role::Link => Tone {
            underline: true,
            ..Tone::PLAIN
        },
    }
}

/// Which way a monitored value moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Flat,
}

/// The tone of a value that moved in `direction` under a monitor scheme.
/// A flat value is always plain; the monochrome scheme is always plain.
pub const fn scheme_tone(scheme: Scheme, direction: Direction) -> Tone {
    match (scheme, direction) {
        (_, Direction::Flat) | (Scheme::Monochrome, _) => Tone::PLAIN,
        (Scheme::GreenRed, Direction::Up) => Tone::colored(AnsiColor::Green),
        (Scheme::GreenRed, Direction::Down) => Tone::colored(AnsiColor::Red),
        (Scheme::BlueOrange, Direction::Up) => Tone::colored(AnsiColor::Blue),
        // Orange is not one of the 16 named colors; yellow is the nearest.
        (Scheme::BlueOrange, Direction::Down) => Tone::colored(AnsiColor::Yellow),
    }
}

#[cfg(test)]
mod tests;
