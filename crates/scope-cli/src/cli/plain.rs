//! Color-free frames for the ratatui screens.
//!
//! The TUI and the monitor set many foreground colors. When
//! `color_enabled()` is false (`NO_COLOR`, `TERM=dumb`, `--no-color`), one
//! pass over the finished frame removes them. Bold, reverse and the other
//! modifiers stay, so glyphs and words still carry the meaning.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use scope::display::terminal::color_enabled;

/// Removes every foreground, background and underline color from `buf`.
pub(crate) fn strip_color(buf: &mut Buffer) {
    for cell in &mut buf.content {
        cell.fg = Color::Reset;
        cell.bg = Color::Reset;
        cell.underline_color = Color::Reset;
    }
}

/// Call at the end of a draw closure. Strips color when color is off.
pub(crate) fn finish_frame(f: &mut Frame) {
    if !color_enabled() {
        strip_color(f.buffer_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};

    #[test]
    fn test_strip_color_keeps_modifiers_and_text() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf.set_string(
            0,
            0,
            "ab",
            Style::new().fg(Color::Red).bg(Color::Blue).bold(),
        );
        strip_color(&mut buf);
        let cell = &buf[(0, 0)];
        assert_eq!(cell.symbol(), "a");
        assert_eq!(cell.fg, Color::Reset);
        assert_eq!(cell.bg, Color::Reset);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }
}
