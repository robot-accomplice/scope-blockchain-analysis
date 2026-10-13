//! Fits text to a terminal width. Shared by the TUI and the monitor.

use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// Marks text that was cut: the one ellipsis, from core.
pub use scope::display::text::ELLIPSIS;

/// The display width of `text`, in columns.
pub fn cols(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cuts `text` to at most `max` display columns, ending in `…` when it cuts.
pub fn cut_with_ellipsis(text: &str, max: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if cols(text) <= max {
        return text.to_string();
    }
    let room = max.saturating_sub(cols(ELLIPSIS));
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        let w = cols(g);
        if used + w > room {
            break;
        }
        out.push_str(g);
        used += w;
    }
    if max >= cols(ELLIPSIS) {
        out.push_str(ELLIPSIS);
    }
    out
}

/// Wraps `text` on word boundaries to `width` columns. A word wider than
/// `width` is cut. At most `height` lines come back.
pub fn wrap_words(text: &str, width: usize, height: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let word = cut_with_ellipsis(word, width);
        match lines.last_mut() {
            Some(last) if cols(last) + 1 + cols(&word) <= width => {
                last.push(' ');
                last.push_str(&word);
            }
            _ => lines.push(word),
        }
    }
    lines.truncate(height);
    lines
}

/// Draws `note` alone in the frame, wrapped on words and centered. Used
/// when the terminal is too small for the real screen.
pub fn draw_note_alone(f: &mut Frame, note: &str) {
    let area = f.area();
    let lines = wrap_words(note, area.width as usize, area.height as usize);
    let top = area.y + (area.height as usize - lines.len()) as u16 / 2;
    let rows = lines.len() as u16;
    let text: Vec<Line> = lines.into_iter().map(Line::from).collect();
    let rect = ratatui::layout::Rect {
        y: top,
        height: rows,
        ..area
    };
    f.render_widget(Paragraph::new(text).centered(), rect);
}
