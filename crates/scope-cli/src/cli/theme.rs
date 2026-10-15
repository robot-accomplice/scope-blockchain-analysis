//! The ratatui adapter of the theme (`scope::display::theme`).
//!
//! It only translates: each role's tone becomes a ratatui `Style`, and this
//! file is the one TUI file that names a ratatui color. Only the 16 named
//! ANSI colors are used, so the terminal's own palette decides the exact
//! shade on dark and light themes. No code here checks whether color is
//! on: `plain::finish_frame` strips every fg and bg color from a frame when
//! it is off. Modifiers (bold, dim, reverse) stay, and each state also shows
//! a word or glyph, so no meaning depends on color. The contrast guard
//! (`tests/tui_contrast.rs`) keeps other files from naming a color.

use ratatui::style::{Color, Modifier, Style};
use scope::display::theme::{
    AnsiColor, Direction, Role, Scheme, Tone, scheme_role_tone, scheme_tone, tone,
};

/// The ratatui color of a named ANSI color.
const fn color(c: AnsiColor) -> Color {
    match c {
        AnsiColor::Black => Color::Black,
        AnsiColor::Red => Color::Red,
        AnsiColor::Green => Color::Green,
        AnsiColor::Yellow => Color::Yellow,
        AnsiColor::Blue => Color::Blue,
        AnsiColor::Magenta => Color::Magenta,
        AnsiColor::Cyan => Color::Cyan,
        AnsiColor::White => Color::Gray,
        AnsiColor::BrightBlack => Color::DarkGray,
        AnsiColor::BrightRed => Color::LightRed,
        AnsiColor::BrightGreen => Color::LightGreen,
        AnsiColor::BrightYellow => Color::LightYellow,
        AnsiColor::BrightBlue => Color::LightBlue,
        AnsiColor::BrightMagenta => Color::LightMagenta,
        AnsiColor::BrightCyan => Color::LightCyan,
        AnsiColor::BrightWhite => Color::White,
    }
}

/// The ratatui foreground color of a tone: its color, or the terminal
/// default (`Reset`) when it has none. Effects are not part of a color.
pub const fn tone_color(t: Tone) -> Color {
    match t.color {
        Some(c) => color(c),
        None => Color::Reset,
    }
}

/// The terminal's own default color: what a cell shows with color off.
pub const DEFAULT_COLOR: Color = tone_color(Tone::PLAIN);

/// The ratatui style of a tone.
pub const fn tone_style(t: Tone) -> Style {
    let mut s = Style::new();
    if let Some(c) = t.color {
        s = s.fg(color(c));
    }
    if t.bold {
        s = s.add_modifier(Modifier::BOLD);
    }
    if t.dim {
        s = s.add_modifier(Modifier::DIM);
    }
    if t.underline {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    if t.reverse {
        s = s.add_modifier(Modifier::REVERSED);
    }
    s
}

/// The ratatui style of a role.
pub const fn style(role: Role) -> Style {
    tone_style(tone(role))
}

/// The ratatui style of a value that moved in `direction` under `scheme`.
pub const fn scheme_style(scheme: Scheme, direction: Direction) -> Style {
    tone_style(scheme_tone(scheme, direction))
}

/// Title, idle pane rule, prompt `scope`, status values, palette names.
pub const ACCENT: Style = style(Role::Accent);

/// Key names in hints and help, and the prompt `:<chain>`.
pub const KEY: Style = style(Role::Key);

/// `✓ Done in N.Ns.`
pub const OK: Style = style(Role::Ok);

/// Running pane rule and title, `running Ns`, spinner, `Cancelled after`.
pub const WARN: Style = style(Role::Warn);

/// Failed pane rule and `✗ Failed in N.Ns.` (the footer adds bold).
pub const ERR: Style = style(Role::Err);

/// The `ℹ` glyph on notes.
pub const INFO: Style = style(Role::Info);

/// Secondary text: the default foreground, dimmed. DIM is a modifier, so it
/// also holds under `NO_COLOR`.
pub const SECONDARY: Style = style(Role::Secondary);

/// The selected palette or menu row.
pub const SELECTED: Style = style(Role::Selected);

/// A search match.
pub const MATCH: Style = style(Role::Match);

/// A URL in a link row.
pub const LINK: Style = style(Role::Link);

/// The strong (bold) form of a role: verdicts, states and headings.
pub const fn strong(role: Role) -> Style {
    tone_style(tone(role).strong())
}

// =============================================================================
// Monitor palette
// =============================================================================

/// Extension trait for `ColorScheme`: its ratatui palette.
pub trait ColorSchemeExt {
    fn palette(&self) -> ColorPalette;
}

impl ColorSchemeExt for Scheme {
    /// The palette for this scheme, from the theme (`scope::display::theme`):
    /// up and down per scheme, the rest from their roles. Monochrome keeps
    /// no color.
    fn palette(&self) -> ColorPalette {
        let scheme = *self;
        let role = |r| tone_color(scheme_role_tone(scheme, r));
        ColorPalette {
            up: tone_color(scheme_tone(scheme, Direction::Up)),
            down: tone_color(scheme_tone(scheme, Direction::Down)),
            neutral: tone_color(Tone::PLAIN),
            header_fg: tone_color(Tone::PLAIN),
            highlight: role(Role::Warn),
            volume_bar: role(Role::Info),
            sparkline: role(Role::Accent),
        }
    }
}

/// Named color palette derived from a ColorScheme.
#[derive(Debug, Clone, Copy)]
pub struct ColorPalette {
    /// Color for price-up / bullish.
    pub up: Color,
    /// Color for price-down / bearish.
    pub down: Color,
    /// Neutral/secondary text color.
    pub neutral: Color,
    /// Header foreground.
    pub header_fg: Color,
    /// Highlight/accent color.
    pub highlight: Color,
    /// Volume bar color.
    pub volume_bar: Color,
    /// Sparkline color.
    pub sparkline: Color,
}

/// What the output pane is doing. It sets the rule color and the state word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneState {
    /// No command runs and the last one did not fail.
    Idle,
    /// A command runs.
    Running,
    /// The last command failed. Lasts until the next command starts.
    Failed,
}

impl PaneState {
    /// The word at the right end of the top rule.
    pub fn word(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Failed => "failed",
        }
    }

    /// The color of the pane rule and title.
    pub fn style(self) -> Style {
        match self {
            Self::Idle => ACCENT,
            Self::Running => WARN,
            Self::Failed => ERR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secondary_is_dim_on_the_default_foreground() {
        assert_eq!(SECONDARY.fg, None);
        assert!(SECONDARY.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn test_only_named_ansi_colors_are_used() {
        // Named colors let the terminal palette pick the shade.
        for style in [ACCENT, KEY, OK, WARN, ERR, INFO, SELECTED, MATCH] {
            assert!(matches!(
                style.fg,
                Some(
                    Color::Cyan
                        | Color::Magenta
                        | Color::Green
                        | Color::Yellow
                        | Color::Red
                        | Color::Blue
                )
            ));
        }
    }

    #[test]
    fn test_pane_state_maps_to_a_rule_color_and_a_word() {
        assert_eq!(PaneState::Idle.style(), ACCENT);
        assert_eq!(PaneState::Running.style(), WARN);
        assert_eq!(PaneState::Failed.style(), ERR);
        assert_eq!(PaneState::Idle.word(), "idle");
        assert_eq!(PaneState::Running.word(), "running");
        assert_eq!(PaneState::Failed.word(), "failed");
    }

    #[test]
    fn test_selected_and_match_are_reversed_cyan_and_yellow() {
        assert_eq!(SELECTED.fg, Some(Color::Cyan));
        assert!(SELECTED.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(MATCH.fg, Some(Color::Yellow));
        assert!(MATCH.add_modifier.contains(Modifier::REVERSED));
    }

    /// The style of text painted with `role`, after the pane parses its
    /// ANSI, as `TuiState::push_ansi` does.
    fn parsed(role: Role) -> Style {
        use ansi_to_tui::IntoText;
        let painted = scope::display::theme::Painter::new(true).paint(role, "x");
        let text = painted.into_text().unwrap();
        text.lines[0].spans[0].style
    }

    #[test]
    fn test_painted_text_parses_back_to_the_same_style() {
        // Help text reaches the pane as ANSI from the text painter. Both
        // adapters must give a role the same look in the pane.
        for role in [
            Role::Key,
            Role::Secondary,
            Role::Accent,
            Role::Ok,
            Role::Err,
        ] {
            assert_eq!(parsed(role), style(role), "{role:?}");
        }
    }

    #[test]
    fn test_scheme_styles_follow_the_core_scheme_tones() {
        assert_eq!(
            scheme_style(Scheme::GreenRed, Direction::Up).fg,
            Some(Color::Green)
        );
        assert_eq!(
            scheme_style(Scheme::BlueOrange, Direction::Down).fg,
            Some(Color::Yellow)
        );
        assert_eq!(
            scheme_style(Scheme::Monochrome, Direction::Up),
            Style::new()
        );
    }

    #[test]
    fn test_key_is_bold_magenta() {
        assert_eq!(KEY.fg, Some(Color::Magenta));
        assert!(KEY.add_modifier.contains(Modifier::BOLD));
    }
}
