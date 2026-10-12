//! # Fit Output to the Pane
//!
//! Breaks output lines that are wider than the output pane. A broken line
//! continues on the next row after the gutter of its first row (leading
//! spaces and `│` bars), so text inside a section stays inside it. A rule
//! (a line that is mostly `─`) is cut at the pane edge instead.
//!
//! Widths are display widths: a wide char (CJK, emoji) takes two columns
//! and is never split across rows. Styles of the spans are kept.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// The fewest `─` chars that make a line a rule.
const RULE_MIN_DASHES: usize = 3;

/// One char with its style.
type Cell = (char, Style);

/// Fits `lines` to `width` columns. Lines that fit are kept as they are.
pub fn fit(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if line.width() <= width {
            out.push(line);
            continue;
        }
        let cells = cells(&line);
        let rows = if is_rule(&cells) {
            vec![cut(&cells, width)]
        } else {
            wrap(&cells, width)
        };
        out.extend(rows.into_iter().map(|row| Line {
            spans: spans(row),
            style: line.style,
            alignment: line.alignment,
        }));
    }
    out
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

fn cells(line: &Line<'_>) -> Vec<Cell> {
    line.spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect()
}

/// True when `─` makes up at least half of the visible chars.
fn is_rule(cells: &[Cell]) -> bool {
    let dashes = cells.iter().filter(|(c, _)| *c == '─').count();
    let visible = cells.iter().filter(|(c, _)| !c.is_whitespace()).count();
    dashes >= RULE_MIN_DASHES && dashes * 2 >= visible
}

/// The leading cells of `cells` that fit in `width` columns.
fn cut(cells: &[Cell], width: usize) -> Vec<Cell> {
    let mut used = 0;
    cells
        .iter()
        .take_while(|(c, _)| {
            used += char_width(*c);
            used <= width
        })
        .copied()
        .collect()
}

/// The gutter of a row: its leading spaces and `│` bars.
fn gutter(cells: &[Cell]) -> &[Cell] {
    let n = cells
        .iter()
        .take_while(|(c, _)| *c == ' ' || *c == '│')
        .count();
    &cells[..n]
}

/// Breaks `cells` into rows of at most `width` columns. Each row after the
/// first starts with the gutter of the first. A row breaks after its last
/// space when it has one past the gutter, else at the edge.
fn wrap(cells: &[Cell], width: usize) -> Vec<Vec<Cell>> {
    let mut gutter = gutter(cells).to_vec();
    let gutter_width: usize = gutter.iter().map(|(c, _)| char_width(*c)).sum();
    // A gutter that leaves no room for text is dropped.
    if gutter_width >= width / 2 {
        gutter.clear();
    }
    let head = gutter.len();

    let mut rows = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut used = 0;
    // Index in `row` of the last space past the gutter.
    let mut space: Option<usize> = None;
    for &(c, style) in cells {
        let w = char_width(c);
        // A row that holds text past the gutter breaks before a char that
        // does not fit. Zero-width chars stay with the char before them.
        if w > 0 && used + w > width && row.len() > head {
            // A space that does not fit is itself the break.
            let rest = match space.filter(|_| c != ' ') {
                Some(i) => {
                    let rest = row.split_off(i + 1);
                    row.truncate(i);
                    rest
                }
                None => Vec::new(),
            };
            rows.push(std::mem::replace(&mut row, gutter.clone()));
            row.extend(rest.into_iter().skip_while(|(c, _)| *c == ' '));
            used = row.iter().map(|(c, _)| char_width(*c)).sum();
            space = None;
            if c == ' ' {
                continue;
            }
        }
        // A space is a break point only after text past the gutter, so no
        // row is left empty.
        if c == ' ' && row.iter().skip(head).any(|(c, _)| *c != ' ') {
            space = Some(row.len());
        }
        row.push((c, style));
        used += w;
    }
    rows.push(row);
    rows
}

/// Joins runs of cells with one style into spans.
fn spans(row: Vec<Cell>) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut text = String::new();
    let mut style = None;
    for (c, s) in row {
        if style.is_some_and(|cur| cur != s) {
            spans.push(Span::styled(
                std::mem::take(&mut text),
                style.unwrap_or_default(),
            ));
        }
        style = Some(s);
        text.push(c);
    }
    if let Some(s) = style {
        spans.push(Span::styled(text, s));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn test_a_line_that_fits_is_kept_as_is() {
        let line = Line::styled("short", Style::new().fg(Color::Red));
        assert_eq!(fit(vec![line.clone()], 10), vec![line]);
    }

    #[test]
    fn test_wrapped_rows_keep_the_gutter() {
        // Text inside a section box stays inside it when it wraps.
        let line = Line::raw("│  alpha beta gamma delta");
        let rows = plain(&fit(vec![line], 13));
        assert_eq!(rows, vec!["│  alpha beta", "│  gamma", "│  delta"]);
    }

    #[test]
    fn test_a_long_word_breaks_at_the_edge() {
        let line = Line::raw("│  /very/long/path/name");
        let rows = plain(&fit(vec![line], 10));
        assert_eq!(rows, vec!["│  /very/l", "│  ong/pat", "│  h/name"]);
    }

    #[test]
    fn test_a_rule_is_cut_not_wrapped() {
        let line = Line::raw(format!("└{}", "─".repeat(50)));
        let rows = plain(&fit(vec![line], 40));
        assert_eq!(rows, vec![format!("└{}", "─".repeat(39))]);
        let header = Line::raw(format!("┌─ Venues {}", "─".repeat(40)));
        assert_eq!(fit(vec![header], 20).len(), 1);
    }

    #[test]
    fn test_wide_chars_are_not_split_and_rows_fit() {
        // Five CJK chars are ten columns: a row of nine holds four.
        let line = Line::raw("日本語の文");
        let rows = plain(&fit(vec![line], 9));
        assert_eq!(rows, vec!["日本語の", "文"]);
    }

    #[test]
    fn test_zero_width_marks_stay_with_their_char() {
        // "e" + combining acute is one column; it must not start a row.
        let line = Line::raw("abcde\u{301}f");
        let rows = plain(&fit(vec![line], 5));
        assert_eq!(rows, vec!["abcde\u{301}", "f"]);
    }

    #[test]
    fn test_span_and_line_styles_survive_a_wrap() {
        // The red `✗ Failed in` footer stays red on every row it wraps to.
        let red = Style::new().fg(Color::Red);
        let footer = Line::styled("✗ Failed in 12.3s.", red);
        let rows = fit(vec![footer], 10);
        assert!(rows.len() > 1);
        assert!(rows.iter().all(|r| r.style == red), "{rows:?}");

        let bar = Style::new().fg(Color::Cyan);
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let line = Line::from(vec![
            Span::styled("│", bar),
            Span::raw("  "),
            Span::styled("bold words that wrap", bold),
        ]);
        let rows = fit(vec![line], 12);
        assert_eq!(rows.len(), 3, "{rows:?}");
        for row in &rows {
            assert_eq!(row.spans[0].style, bar, "the gutter keeps its color");
            assert_eq!(row.spans.last().unwrap().style, bold);
        }
    }

    #[test]
    fn test_a_deep_gutter_is_dropped() {
        // A gutter of half the width or more would leave no room for text.
        let line = Line::raw(format!("{}abc def", " ".repeat(6)));
        let rows = plain(&fit(vec![line], 10));
        assert_eq!(rows, vec!["      abc", "def"]);
    }

    #[test]
    fn test_zero_width_keeps_the_lines() {
        let lines = vec![Line::raw("anything")];
        assert_eq!(fit(lines.clone(), 0), lines);
    }
}
