//! # Fit Output to the Pane
//!
//! Breaks output lines that are wider than the output pane. A broken line
//! continues on the next row after the gutter of its first row (leading
//! spaces and `│` bars), so text inside a section stays inside it. A rule
//! (a line that ends in a long `─` run) is cut at the pane edge instead.
//!
//! Widths are measured per grapheme cluster, as ratatui draws them: `⚠️`
//! (with its variation selector) is two columns, and an emoji ZWJ
//! sequence is one cluster. A cluster is never split across rows. Tabs
//! become spaces to the next tab stop first, because ratatui does not
//! draw them. Styles of the spans are kept.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A `─` run at least this wide at the end of a line makes it a rule.
const RULE_MIN_RUN: usize = 8;

/// Tab stops are every this many columns.
const TAB_STOP: usize = 4;

/// One grapheme cluster with its style and display width.
#[derive(Clone, Copy)]
struct Cell<'a> {
    sym: &'a str,
    style: Style,
    width: usize,
}

impl Cell<'_> {
    fn is(&self, sym: &str) -> bool {
        self.sym == sym
    }
}

/// Fits `lines` to `width` columns. Lines that fit are kept as they are
/// (with tabs expanded).
pub fn fit(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let line = expand_tabs(line);
        if graphemes(&line).map(|g| g.width).sum::<usize>() <= width {
            out.push(line);
            continue;
        }
        let cells: Vec<Cell> = graphemes(&line).collect();
        let rows = if is_rule(&cells) {
            vec![cut(&cells, width)]
        } else {
            wrap(&cells, width)
        };
        out.extend(rows.into_iter().map(|row| Line {
            spans: spans(&row),
            style: line.style,
            alignment: line.alignment,
        }));
    }
    out
}

/// The grapheme clusters ratatui draws for `line`: clusters that hold a
/// control char are skipped, as ratatui skips them.
fn graphemes<'a>(line: &'a Line<'_>) -> impl Iterator<Item = Cell<'a>> {
    line.spans.iter().flat_map(|span| {
        span.styled_graphemes(Style::default()).map(|g| Cell {
            sym: g.symbol,
            style: g.style,
            width: g.symbol.width(),
        })
    })
}

/// Replaces each tab with spaces up to the next tab stop.
fn expand_tabs(line: Line<'static>) -> Line<'static> {
    if !line.spans.iter().any(|s| s.content.contains('\t')) {
        return line;
    }
    let Line {
        style,
        alignment,
        spans,
    } = line;
    let mut col = 0;
    let spans = spans
        .into_iter()
        .map(|span| {
            let mut text = String::new();
            for g in span.content.graphemes(true) {
                if g == "\t" {
                    let n = TAB_STOP - col % TAB_STOP;
                    text.extend(std::iter::repeat_n(' ', n));
                    col += n;
                } else {
                    text.push_str(g);
                    if !g.contains(char::is_control) {
                        col += g.width();
                    }
                }
            }
            Span::styled(text, span.style)
        })
        .collect();
    Line {
        style,
        alignment,
        spans,
    }
}

/// True when the line ends (before trailing spaces) in a `─` run of at
/// least `RULE_MIN_RUN` columns.
fn is_rule(cells: &[Cell]) -> bool {
    let run: usize = cells
        .iter()
        .rev()
        .skip_while(|c| c.is(" "))
        .take_while(|c| c.is("─"))
        .map(|c| c.width)
        .sum();
    run >= RULE_MIN_RUN
}

/// The leading cells of `cells` that fit in `width` columns.
fn cut<'a>(cells: &[Cell<'a>], width: usize) -> Vec<Cell<'a>> {
    let mut used = 0;
    cells
        .iter()
        .take_while(|c| {
            used += c.width;
            used <= width
        })
        .copied()
        .collect()
}

/// The gutter of a row: its leading spaces and `│` bars.
fn gutter<'a>(cells: &[Cell<'a>]) -> Vec<Cell<'a>> {
    cells
        .iter()
        .take_while(|c| c.is(" ") || c.is("│"))
        .copied()
        .collect()
}

/// Breaks `cells` into rows of at most `width` columns. Each row after the
/// first starts with the gutter of the first, and drops the spaces that
/// would follow it. A row breaks after its last space when it has one past
/// the gutter's text, else at the edge.
fn wrap<'a>(cells: &[Cell<'a>], width: usize) -> Vec<Vec<Cell<'a>>> {
    let mut gutter = gutter(cells);
    // A gutter that leaves no room for text is dropped.
    if gutter.iter().map(|c| c.width).sum::<usize>() >= width / 2 {
        gutter.clear();
    }
    let head = gutter.len();

    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut used = 0;
    // Index in `row` of the last space after text past the gutter.
    let mut space: Option<usize> = None;
    for &cell in cells {
        // A row that holds text past the gutter breaks before a cluster
        // that does not fit.
        if cell.width > 0 && used + cell.width > width && row.len() > head {
            // A space that does not fit is itself the break.
            let rest = match space.filter(|_| !cell.is(" ")) {
                Some(i) => {
                    let rest = row.split_off(i + 1);
                    row.truncate(i);
                    rest
                }
                None => Vec::new(),
            };
            while row.len() > head && row.last().is_some_and(|c| c.is(" ")) {
                row.pop();
            }
            rows.push(std::mem::replace(&mut row, gutter.clone()));
            row.extend(rest.into_iter().skip_while(|c| c.is(" ")));
            used = row.iter().map(|c| c.width).sum();
            space = None;
        }
        if cell.is(" ") {
            // A continuation row has no spaces between gutter and text.
            if !rows.is_empty() && row.len() == head {
                continue;
            }
            // A space is a break point only after text past the gutter, so
            // no row is left empty.
            if row.iter().skip(head).any(|c| !c.is(" ")) {
                space = Some(row.len());
            }
        }
        row.push(cell);
        used += cell.width;
    }
    rows.push(row);
    rows
}

/// Joins runs of cells with one style into spans.
fn spans(row: &[Cell]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for cell in row {
        match spans.last_mut() {
            Some(last) if last.style == cell.style => last.content.to_mut().push_str(cell.sym),
            _ => spans.push(Span::styled(cell.sym.to_string(), cell.style)),
        }
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
    fn test_continuation_rows_drop_padding_after_the_gutter() {
        // Wide padding between a key and a long value must not leave a
        // row that is only gutter and spaces.
        let line = Line::raw(format!("│  key{}value", " ".repeat(20)));
        let rows = plain(&fit(vec![line], 12));
        assert_eq!(rows, vec!["│  key", "│  value"]);
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
    fn test_text_with_dashes_inside_is_not_a_rule() {
        // Only a `─` run that ends the line makes a rule; text after a run
        // must wrap, not be cut away.
        let line = Line::raw(format!("a {} tail text", "─".repeat(10)));
        let rows = plain(&fit(vec![line], 14));
        assert_eq!(
            rows.concat().replace(' ', ""),
            format!("a{}tailtext", "─".repeat(10))
        );
        assert!(rows.len() > 1);
    }

    #[test]
    fn test_wide_chars_are_not_split_and_rows_fit() {
        // Five CJK chars are ten columns: a row of nine holds four.
        let line = Line::raw("日本語の文");
        let rows = plain(&fit(vec![line], 9));
        assert_eq!(rows, vec!["日本語の", "文"]);
    }

    #[test]
    fn test_emoji_presentation_at_the_edge_moves_to_the_next_row() {
        // `⚠️` is U+26A0 + U+FE0F and draws two columns. Summing chars
        // (1 + 0) keeps it on a full row, where its second column clips.
        let line = Line::raw("abcd\u{26a0}\u{fe0f}");
        let rows = plain(&fit(vec![line], 5));
        assert_eq!(rows, vec!["abcd", "\u{26a0}\u{fe0f}"]);
    }

    #[test]
    fn test_a_zwj_emoji_is_one_cluster() {
        // A family emoji is one two-column cluster; summing its chars gives
        // six columns and splits it.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let line = Line::raw(family.repeat(3));
        let rows = plain(&fit(vec![line], 4));
        assert_eq!(rows, vec![family.repeat(2), family.to_string()]);
    }

    #[test]
    fn test_zero_width_marks_stay_with_their_char() {
        // "e" + combining acute is one column; it must not start a row.
        let line = Line::raw("abcde\u{301}f");
        let rows = plain(&fit(vec![line], 5));
        assert_eq!(rows, vec!["abcde\u{301}", "f"]);
    }

    #[test]
    fn test_tabs_expand_to_the_next_tab_stop() {
        // ratatui draws nothing for a tab, so the width math would be off.
        let rows = plain(&fit(vec![Line::raw("ab\tcd\tx")], 40));
        assert_eq!(rows, vec!["ab  cd  x"]);
        let line = Line::from(vec![Span::raw("a"), Span::raw("\tb")]);
        assert_eq!(plain(&fit(vec![line], 40)), vec!["a   b"]);
        // Expanded tabs count when the line is fitted.
        let rows = plain(&fit(vec![Line::raw("x\t\t\ty")], 8));
        assert_eq!(rows, vec!["x", "y"]);
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
