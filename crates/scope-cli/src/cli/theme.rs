//! The TUI color theme: the one module that names a color.
//!
//! Only the 16 named ANSI colors are used, so the terminal's own palette
//! decides the exact shade on dark and light themes. No code here checks
//! whether color is on: `plain::finish_frame` strips every fg and bg color
//! from a frame when it is off. Modifiers (bold, dim, reverse) stay, and
//! each state also shows a word or glyph, so no meaning depends on color.
//! The contrast guard (`tests/tui_contrast.rs`) keeps other TUI files from
//! naming a color. The monitor keeps its own palettes.

use ratatui::style::{Color, Modifier, Style};

/// Title, idle pane rule, prompt `scope`, status values, palette names.
pub const ACCENT: Style = Style::new().fg(Color::Cyan);

/// Key names in hints and help, and the prompt `:<chain>`.
pub const KEY: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);

/// `✓ Done in N.Ns.`
pub const OK: Style = Style::new().fg(Color::Green);

/// Running pane rule and title, `running Ns`, spinner, `Cancelled after`.
pub const WARN: Style = Style::new().fg(Color::Yellow);

/// Failed pane rule and `✗ Failed in N.Ns.` (the footer adds bold).
pub const ERR: Style = Style::new().fg(Color::Red);

/// The `ℹ` glyph on notes.
pub const INFO: Style = Style::new().fg(Color::Blue);

/// Secondary text: the default foreground, dimmed. DarkGray and Gray vanish
/// on some dark and light themes. DIM is a modifier, so it also holds under
/// `NO_COLOR`.
pub const SECONDARY: Style = Style::new().add_modifier(Modifier::DIM);

/// The selected palette or menu row.
pub const SELECTED: Style = Style::new()
    .fg(Color::Cyan)
    .add_modifier(Modifier::REVERSED);

/// A search match.
pub const MATCH: Style = Style::new()
    .fg(Color::Yellow)
    .add_modifier(Modifier::REVERSED);

/// ANSI start of [`KEY`], for text that goes through a command's output.
pub const KEY_ANSI: &str = "\x1b[1;35m";
/// ANSI start of [`SECONDARY`], for the same.
pub const SECONDARY_ANSI: &str = "\x1b[2m";
/// ANSI reset.
pub const RESET_ANSI: &str = "\x1b[0m";

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

    /// The style of `text` after the pane parses its ANSI, as
    /// `TuiState::push_ansi` does.
    fn parsed(start: &str) -> Style {
        use ansi_to_tui::IntoText;
        let text = format!("{start}x{RESET_ANSI}").into_text().unwrap();
        text.lines[0].spans[0].style
    }

    #[test]
    fn test_the_ansi_starts_parse_back_to_their_styles() {
        // Help text reaches the pane as ANSI. A constant that drifts from
        // its style would show keys in another color than the hint line.
        assert_eq!(parsed(KEY_ANSI), KEY);
        assert_eq!(parsed(SECONDARY_ANSI), SECONDARY);
    }

    #[test]
    fn test_key_is_bold_magenta() {
        assert_eq!(KEY.fg, Some(Color::Magenta));
        assert!(KEY.add_modifier.contains(Modifier::BOLD));
    }
}
