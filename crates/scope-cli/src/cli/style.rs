//! Shared text styles for the TUI and the monitor.

use ratatui::style::{Modifier, Style};

/// Secondary text: the default foreground, dimmed. DarkGray and Gray vanish
/// on some dark and light themes. DIM is a modifier, so it also holds under
/// `NO_COLOR`.
pub const SECONDARY: Style = Style::new().add_modifier(Modifier::DIM);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secondary_is_dim_on_the_default_foreground() {
        assert_eq!(SECONDARY.fg, None);
        assert!(SECONDARY.add_modifier.contains(Modifier::DIM));
    }
}
