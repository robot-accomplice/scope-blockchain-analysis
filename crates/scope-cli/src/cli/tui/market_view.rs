//! The compact market summary of the TUI (design 2026-10-21, section 2.2).
//!
//! The CLI prints its text report; the TUI draws the same `MarketSummary`
//! here, as lines sized to their content: the verdict first, the key
//! values in two columns, the book as one ladder (asks above the mid, bids
//! below, one size per level and a bar), and the checks in a grid of two
//! columns. Pure: no I/O, no state.

use crate::cli::output::SummaryView;
use crate::cli::theme::{ACCENT, ERR, OK, SECONDARY, WARN};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use scope::display::format::format_large_number;
use scope::market::{HealthCheck, MarketSummary, OrderBookLevel};

/// The width of the full snapshot, in columns.
pub(crate) const FULL_WIDTH: usize = 50;

/// Book levels per side in the full snapshot (Q2).
const LADDER_LEVELS: usize = 5;

/// Columns of a level's size, and the widest bar of the ladder.
const SIZE_COLS: usize = 9;
const BAR_COLS: usize = 24;

/// One cell of the checks grid: a glyph, a space and the text.
const CHECK_COLS: usize = FULL_WIDTH / 2;

/// How much of the snapshot to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Size {
    /// The ladder with bars and the checks grid (`FULL_WIDTH` columns).
    Full,
    /// For a narrow pane: a one-line book and a one-line check count.
    Compact,
}

impl Size {
    /// The size that fits `width` columns.
    pub(crate) fn for_width(width: usize) -> Size {
        if width >= FULL_WIDTH {
            Size::Full
        } else {
            Size::Compact
        }
    }
}

/// The widest line of `lines`, in columns.
pub(crate) fn width(lines: &[Line<'_>]) -> usize {
    lines.iter().map(Line::width).max().unwrap_or(0)
}

/// What the snapshot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Opts {
    pub size: Size,
    /// Book levels per side in the ladder (Full).
    pub levels: usize,
    /// The checks that did not pass in one line each, not in full.
    pub short_problems: bool,
    /// The trends chart the key values beside it (the repeating view):
    /// leave them out here, so no value shows twice.
    pub with_trends: bool,
}

impl Opts {
    /// Everything, at `size`.
    pub(crate) fn of(size: Size) -> Self {
        Self {
            size,
            levels: LADDER_LEVELS,
            short_problems: size == Size::Compact,
            with_trends: false,
        }
    }
}

/// The snapshot of `view`, at `size`.
pub(crate) fn lines(view: &SummaryView, size: Size) -> Vec<Line<'static>> {
    lines_with(view, Opts::of(size))
}

/// The snapshot of `view` in at most `rows` lines, when it can: it leaves
/// out the least needed lines first (fewer ladder levels, then the checks
/// that did not pass in one line each, then fewer levels again).
pub(crate) fn fit(view: &SummaryView, base: Opts, rows: usize) -> Vec<Line<'static>> {
    let mut opts = base;
    let mut lines = lines_with(view, opts);
    let steps = [
        Opts { levels: 3, ..opts },
        Opts {
            levels: 3,
            short_problems: true,
            ..opts
        },
        Opts {
            levels: 1,
            short_problems: true,
            ..opts
        },
    ];
    for step in steps {
        if lines.len() <= rows {
            break;
        }
        opts = step;
        lines = lines_with(view, opts);
    }
    lines
}

/// The snapshot of `view` with `opts`.
pub(crate) fn lines_with(view: &SummaryView, opts: Opts) -> Vec<Line<'static>> {
    let s = &view.summary;
    let quote = quote_of(&s.pair);
    let mut out = vec![header(s, &view.venue, opts.size)];
    out.extend(key_values(s, opts));
    match opts.size {
        Size::Full => {
            out.push(rule(&format!("Book · {quote}"), FULL_WIDTH));
            out.extend(ladder(s, opts.levels));
            out.push(rule(&checks_title(s), FULL_WIDTH));
            out.extend(checks_grid(s));
        }
        Size::Compact => {
            out.push(one_line_book(s));
            out.push(Line::styled(
                format!(" Checks · {}", checks_count(s)),
                ACCENT,
            ));
        }
    }
    out.extend(problems(s, opts));
    out
}

/// The quote asset of a pair (`USDC/USDT` gives `USDT`).
fn quote_of(pair: &str) -> String {
    scope::market::Pair::parse(pair)
        .quote
        .unwrap_or_else(|| "quote".to_string())
}

/// `USDC/USDT · binance` and the verdict at the right edge.
fn header(s: &MarketSummary, venue: &str, size: Size) -> Line<'static> {
    let failed = s
        .checks
        .iter()
        .filter(|c| matches!(c, HealthCheck::Fail(_)))
        .count();
    let (verdict, style) = if s.healthy {
        ("✓ HEALTHY".to_string(), OK)
    } else {
        (format!("✗ UNHEALTHY · {failed} failed"), ERR)
    };
    let mut left = format!(" {} · {venue}", s.pair);
    if size == Size::Full {
        left = cut(&left, FULL_WIDTH - 2 - verdict.chars().count());
    }
    let total = match size {
        Size::Full => FULL_WIDTH,
        Size::Compact => left.chars().count() + 2 + verdict.chars().count(),
    };
    let gap = total
        .saturating_sub(left.chars().count() + verdict.chars().count())
        .max(2);
    Line::from(vec![
        Span::styled(left, ACCENT.add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(gap)),
        Span::styled(verdict, style.add_modifier(Modifier::BOLD)),
    ])
}

/// An amount in the one large-number format; `-0` reads `0.00`.
fn amount(v: f64) -> String {
    format_large_number(if v == 0.0 { 0.0 } else { v })
}

fn price(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| format!("{v:.6}"))
}

fn pct(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| format!("{v:.3}%"))
}

fn signed_pct(v: Option<f64>) -> String {
    v.map_or_else(String::new, |v| format!("{v:+.3}%"))
}

/// Columns of a key-value label and of the first value.
const LABEL_COLS: usize = 8;
const VALUE_COLS: usize = 17;

/// ` Label   value            Label   value`: two pairs in fixed columns.
fn two(left: (&str, String), right: (&str, String)) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<w$} ", left.0, w = LABEL_COLS), SECONDARY),
        Span::raw(format!(
            "{:<w$}  ",
            cut(&left.1, VALUE_COLS),
            w = VALUE_COLS
        )),
        Span::styled(format!("{:<w$} ", right.0, w = LABEL_COLS), SECONDARY),
        Span::raw(right.1),
    ])
}

/// Columns of a side in the bid/ask table.
const SIDE_COLS: usize = 12;

/// One row of the bid/ask table: a label, the bid value, the ask value.
fn side_row(label: &str, bid: String, ask: String, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<w$} ", label, w = LABEL_COLS), SECONDARY),
        Span::styled(format!("{bid:>w$}", w = SIDE_COLS), style),
        Span::styled(format!("{ask:>w$}", w = SIDE_COLS), style),
    ])
}

/// Mid and spread, volume and execution cost, then a bid/ask table: best
/// price, depth, top-3 and top-10 depth. With the trends beside it, only
/// what they do not chart.
fn key_values(s: &MarketSummary, opts: Opts) -> Vec<Line<'static>> {
    let full = opts.size == Size::Full;
    let vol = s
        .volume_24h
        .map_or_else(|| "n/a".to_string(), format_large_number);
    let mut lines = Vec::new();
    if !opts.with_trends {
        let dev = s.mid_price.and_then(|m| s.peg_deviation_pct(m));
        let mid = format!("{} {}", price(s.mid_price), signed_pct(dev));
        lines.push(two(("Mid", mid), ("Spread", pct(s.spread_pct))));
    }
    let label = |text: &str| Span::styled(format!(" {:<w$} ", text, w = LABEL_COLS), SECONDARY);
    lines.push(Line::from(vec![label("Vol 24h"), Span::raw(vol)]));
    if !full && !opts.with_trends {
        let depth = format!("{} / {}", amount(s.bid_depth), amount(s.ask_depth));
        lines.push(Line::from(vec![label("Depth"), Span::raw(depth)]));
        return lines;
    }
    if !full {
        return lines;
    }
    lines.push(side_row("", "bid".into(), "ask".into(), SECONDARY));
    lines.push(side_row(
        "Price",
        price(s.best_bid),
        price(s.best_ask),
        Style::new(),
    ));
    // A 10K sell walks the bids; a 10K buy walks the asks.
    if let (Some(sell), Some(buy)) = (&s.execution_10k_sell, &s.execution_10k_buy) {
        let slip = |e: &scope::market::ExecutionEstimate| {
            if e.fillable {
                format!("{:.2} bps", e.slippage_bps)
            } else {
                "not fillable".to_string()
            }
        };
        lines.push(side_row("Slip 10K", slip(sell), slip(buy), Style::new()));
    }
    if !opts.with_trends {
        let t = &s.top_depth;
        lines.push(side_row(
            "Depth",
            amount(s.bid_depth),
            amount(s.ask_depth),
            Style::new(),
        ));
        lines.push(side_row(
            "Top-3",
            amount(t.bid_top3),
            amount(t.ask_top3),
            Style::new(),
        ));
        lines.push(side_row(
            "Top-10",
            amount(t.bid_top10),
            amount(t.ask_top10),
            Style::new(),
        ));
    }
    lines
}

/// ` Title ─────…` to `width` columns.
fn rule(title: &str, width: usize) -> Line<'static> {
    let head = format!(" {title} ");
    let fill = width.saturating_sub(head.chars().count());
    Line::from(vec![
        Span::styled(head, ACCENT),
        Span::styled("─".repeat(fill), SECONDARY),
    ])
}

/// One ladder row: price, size in the quote asset, and a bar. `price_cols`
/// is the widest price of the ladder.
fn level(l: &OrderBookLevel, max: f64, style: Style, price_cols: usize) -> Line<'static> {
    let size = l.value();
    let cells = if max > 0.0 {
        ((size / max) * BAR_COLS as f64)
            .ceil()
            .clamp(1.0, BAR_COLS as f64) as usize
    } else {
        1
    };
    Line::from(vec![
        Span::raw(format!("  {:>w$.6}", l.price, w = price_cols)),
        Span::raw(format!(" {:>w$}", amount(size), w = SIZE_COLS)),
        Span::raw(" "),
        Span::styled("█".repeat(cells), style),
    ])
}

/// Asks above (highest first), the mid line, bids below (best first), and
/// a count of the levels and outliers left out.
fn ladder(s: &MarketSummary, levels: usize) -> Vec<Line<'static>> {
    let asks: Vec<&OrderBookLevel> = s.asks.iter().take(levels).collect();
    let bids: Vec<&OrderBookLevel> = s.bids.iter().take(levels).collect();
    // One scale for both sides, so a wall on either side reads the same.
    let max = asks
        .iter()
        .chain(&bids)
        .map(|l| l.value())
        .fold(0.0_f64, f64::max);
    let price_cols = asks
        .iter()
        .chain(&bids)
        .map(|l| format!("{:.6}", l.price).len())
        .max()
        .unwrap_or(0);
    let mut out: Vec<Line> = asks
        .iter()
        .rev()
        .map(|l| level(l, max, ERR, price_cols))
        .collect();
    out.push(rule(
        &format!("mid {} · spread {}", price(s.mid_price), pct(s.spread_pct)),
        FULL_WIDTH,
    ));
    out.extend(bids.iter().map(|l| level(l, max, OK, price_cols)));
    let mut rest = Vec::new();
    let more_asks = s.asks.len().saturating_sub(levels);
    let more_bids = s.bids.len().saturating_sub(levels);
    if more_asks > 0 {
        rest.push(format!("+{more_asks} asks"));
    }
    if more_bids > 0 {
        rest.push(format!("+{more_bids} bids"));
    }
    let outliers = s.ask_outliers + s.bid_outliers;
    if outliers > 0 {
        rest.push(format!("{outliers} outliers left out"));
    }
    if asks.is_empty() && bids.is_empty() {
        rest.push("no levels".to_string());
    }
    if !rest.is_empty() {
        out.push(Line::styled(format!("  {}", rest.join(" · ")), SECONDARY));
    }
    out
}

/// ` Book · ask 1.000970 225.28K · bid 1.000960 2.63M` (Compact).
fn one_line_book(s: &MarketSummary) -> Line<'static> {
    let side = |name: &str, l: Option<&OrderBookLevel>| {
        l.map_or_else(
            || format!("{name} n/a"),
            |l| format!("{name} {:.6} {}", l.price, amount(l.value())),
        )
    };
    Line::from(vec![
        Span::styled(" Book ", SECONDARY),
        Span::raw(format!(
            "{} · {}",
            side("ask", s.asks.first()),
            side("bid", s.bids.first())
        )),
    ])
}

/// `11 pass · 1 n/a`, or with the failures first.
fn checks_count(s: &MarketSummary) -> String {
    let count = |f: fn(&HealthCheck) -> bool| s.checks.iter().filter(|c| f(c)).count();
    let parts = [
        (count(|c| matches!(c, HealthCheck::Fail(_))), "failed"),
        (count(|c| matches!(c, HealthCheck::Warn(_))), "warn"),
        (count(|c| matches!(c, HealthCheck::Pass(_))), "pass"),
        (count(|c| matches!(c, HealthCheck::NotApplicable(_))), "n/a"),
    ];
    let text: Vec<String> = parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect();
    if text.is_empty() {
        "none".to_string()
    } else {
        text.join(" · ")
    }
}

fn checks_title(s: &MarketSummary) -> String {
    format!("Checks · {}", checks_count(s))
}

fn glyph(c: &HealthCheck) -> (&'static str, Style) {
    match c {
        HealthCheck::Pass(_) => ("✓", OK),
        HealthCheck::Warn(_) => ("!", WARN),
        HealthCheck::Fail(_) => ("✗", ERR),
        HealthCheck::NotApplicable(_) => ("–", SECONDARY),
    }
}

/// A check in the grid: its name and its limit (`Spread ≤3%`,
/// `Bid depth ≥3K`), as the value is in the key values above; a check with
/// no limit shows its value (`Bid/Ask ratio 2.21x`).
fn grid_text(c: &HealthCheck) -> String {
    let message = c.message();
    let Some((name, rest)) = message.split_once(':') else {
        return message.to_string();
    };
    let name = name
        .trim()
        .replace("top-3 depth", "top-3")
        .replace("top-10 depth", "top-10");
    match limit_of(rest) {
        Some(limit) => format!("{name} {limit}"),
        None => short(c),
    }
}

/// `≤3%` for `(max 3.0%)`, `≥3K` for `(min 3000)`, if the text has one.
fn limit_of(text: &str) -> Option<String> {
    let (sign, rest) = match text.find("(max ") {
        Some(i) => ("≤", &text[i + 5..]),
        None => ("≥", &text[text.find("(min ")? + 5..]),
    };
    let word = rest.split([')', ' ']).next()?;
    let (num, unit) = word.split_at(
        word.find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(word.len()),
    );
    let v: f64 = num.parse().ok()?;
    Some(format!("{sign}{}{unit}", compact_number(v)))
}

/// A number as short as it reads: `3K`, `2.5K`, `300`, `3`, `0.5`.
pub(crate) fn compact_number(v: f64) -> String {
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    let (scaled, suffix) = match v.abs() {
        a if a >= 1e9 => (v / 1e9, "B"),
        a if a >= 1e6 => (v / 1e6, "M"),
        a if a >= 1e3 => (v / 1e3, "K"),
        _ => (v, ""),
    };
    format!("{}{suffix}", trim(format!("{scaled:.2}")))
}

/// A check in a few words: its name and the first word of its value, with
/// a large number shortened (`Bid depth: 38544484 USDT …` is
/// `Bid depth 38.54M`).
fn short(c: &HealthCheck) -> String {
    let message = c.message();
    let Some((name, rest)) = message.split_once(':') else {
        return message.to_string();
    };
    // The value is the text up to its limit in brackets; its numbers are
    // shortened (`38544484 USDT (min 3000)` is `38.54M USDT`).
    let value: Vec<String> = rest
        .split(['(', ';'])
        .next()
        .unwrap_or("")
        .split_whitespace()
        // The quote asset is in the book's title.
        .filter(|w| !is_asset(w))
        .map(shorten)
        .collect();
    let name = name
        .trim()
        .replace("top-3 depth", "top-3")
        .replace("top-10 depth", "top-10");
    format!("{name} {}", value.join(" "))
}

/// An asset symbol such as `USDT`: all capitals, 2 to 6 letters.
fn is_asset(w: &str) -> bool {
    (2..=6).contains(&w.len()) && w.chars().all(|c| c.is_ascii_uppercase())
}

/// A word with a large number in the one large-number format, keeping a
/// sign in front (`≥1600000` is `≥1.60M`).
fn shorten(w: &str) -> String {
    let start = w
        .find(|c: char| c.is_ascii_digit() || c == '-')
        .unwrap_or(w.len());
    let (lead, rest) = w.split_at(start);
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    let (num, tail) = rest.split_at(end);
    match num.parse::<f64>() {
        Ok(v) if v.abs() >= 1000.0 => format!("{lead}{}{tail}", format_large_number(v)),
        _ => w.to_string(),
    }
}

/// The checks that passed, two per row. The others come after, in full
/// ([`problems`]), so none shows twice.
fn checks_grid(s: &MarketSummary) -> Vec<Line<'static>> {
    let checks: Vec<&HealthCheck> = s
        .checks
        .iter()
        .filter(|c| matches!(c, HealthCheck::Pass(_)))
        .collect();
    let text_cols = CHECK_COLS - 4;
    checks
        .chunks(2)
        .map(|pair| {
            let mut spans = vec![Span::raw(" ")];
            for c in pair {
                let (g, st) = glyph(c);
                let text = cut(&grid_text(c), text_cols);
                let pad = text_cols.saturating_sub(text.chars().count());
                spans.push(Span::styled(g, st));
                spans.push(Span::raw(format!(" {text}{} ", " ".repeat(pad))));
            }
            // No trailing blanks: the width is the content's.
            if let Some(last) = spans.last_mut() {
                *last = Span::raw(last.content.trim_end().to_string());
            }
            Line::from(spans)
        })
        .collect()
}

/// `text` cut to `cols` characters, with `…` when cut.
fn cut(text: &str, cols: usize) -> String {
    if text.chars().count() <= cols {
        text.to_string()
    } else {
        let mut t: String = text.chars().take(cols.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// The whole message of every check that did not pass, wrapped to the
/// snapshot width under its glyph.
fn problems(s: &MarketSummary, opts: Opts) -> Vec<Line<'static>> {
    let width = match opts.size {
        Size::Full => FULL_WIDTH,
        Size::Compact => FULL_WIDTH.min(60),
    };
    let mut out = Vec::new();
    let mut checks: Vec<&HealthCheck> = s
        .checks
        .iter()
        .filter(|c| !matches!(c, HealthCheck::Pass(_)))
        .collect();
    checks.sort_by_key(|c| match c {
        HealthCheck::Fail(_) => 0,
        HealthCheck::Warn(_) => 1,
        _ => 2,
    });
    for c in checks {
        let (g, st) = glyph(c);
        if opts.short_problems {
            // One line each in the compact view: the pane is short.
            out.push(Line::from(vec![
                Span::styled(format!(" {g} "), st),
                Span::styled(cut(&short(c), width - 3), SECONDARY),
            ]));
            continue;
        }
        let message: Vec<String> = c.message().split_whitespace().map(shorten).collect();
        for (i, part) in wrap(&message.join(" "), width - 3).into_iter().enumerate() {
            let lead = if i == 0 {
                format!(" {g} ")
            } else {
                "   ".to_string()
            };
            out.push(Line::from(vec![
                Span::styled(lead, st),
                Span::styled(part, SECONDARY),
            ]));
        }
    }
    out
}

/// Word wrap to `cols` columns; a word longer than a line is cut.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word = cut(word, cols);
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > cols {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::market::{BookSource, HealthThresholds, OrderBook};

    fn side(start: f64, step: f64, qtys: &[f64]) -> Vec<OrderBookLevel> {
        qtys.iter()
            .enumerate()
            .map(|(i, &quantity)| OrderBookLevel {
                price: start + step * i as f64,
                quantity,
            })
            .collect()
    }

    fn view(bids: usize, asks: usize, qty: f64) -> SummaryView {
        let book = OrderBook {
            pair: "USDC/USDT".to_string(),
            source: BookSource::Exchange,
            bids: side(0.9999, -0.0001, &vec![qty; bids]),
            asks: side(1.0001, 0.0001, &vec![qty; asks]),
        };
        SummaryView {
            summary: MarketSummary::from_order_book(
                &book,
                &HealthThresholds::default(),
                Some(3_714_594_299.0),
            ),
            venue: "binance".to_string(),
        }
    }

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_verdict_is_on_the_first_line() {
        let healthy = lines(&view(20, 20, 40_000.0), Size::Full);
        assert!(text(&healthy[..1]).contains("USDC/USDT · binance"));
        assert!(
            text(&healthy[..1]).ends_with("✓ HEALTHY"),
            "{}",
            text(&healthy[..1])
        );
        let thin = lines(&view(2, 2, 1.0), Size::Full);
        assert!(
            text(&thin[..1]).contains("✗ UNHEALTHY ·"),
            "{}",
            text(&thin[..1])
        );
        assert!(text(&thin[..1]).contains("failed"));
    }

    #[test]
    fn the_ladder_has_five_levels_per_side_one_size_and_a_mid_line() {
        let t = text(&lines(&view(20, 20, 40_000.0), Size::Full));
        assert!(t.contains("Book · USDT"), "{t}");
        assert!(t.contains("mid 1.000000"), "{t}");
        assert!(t.contains("+15 asks · +15 bids"), "{t}");
        // Asks highest first above the mid, bids best first below it.
        let book = t.find("Book · USDT").unwrap();
        let t = &t[book..];
        let a5 = t.find("1.000500").unwrap();
        let a1 = t.find("1.000100").unwrap();
        let mid = t.find("mid 1.000000").unwrap();
        let b1 = t.find("0.999900").unwrap();
        assert!(a5 < a1 && a1 < mid && mid < b1, "{t}");
        // One size per level, in the quote asset: no base amount.
        let row = t.lines().find(|l| l.contains("0.999900")).unwrap();
        assert_eq!(row.matches("40.00K").count(), 1, "{row}");
        assert!(!row.contains("USDC"), "{row}");
    }

    #[test]
    fn one_number_format_for_depths_and_sizes() {
        let t = text(&lines(&view(20, 20, 4_000_000.0), Size::Full));
        assert!(t.contains("Depth "), "{t}");
        assert!(
            t.contains("80.00M / 80.00M USDT") || t.contains("M / "),
            "{t}"
        );
        assert!(!t.contains("3999600"), "no raw large numbers: {t}");
        assert!(t.contains("Vol 24h  3.71B"), "{t}");
    }

    #[test]
    fn checks_are_counted_and_sorted_and_problems_spelled_out() {
        let thin = lines(&view(2, 2, 1.0), Size::Full);
        let t = text(&thin);
        let title = t.lines().find(|l| l.contains("Checks · ")).unwrap();
        assert!(title.contains("failed"), "{title}");
        // The grid holds the passes; each failure is spelled out once.
        let after = &t[t.find("Checks · ").unwrap()..];
        assert_eq!(after.matches("Bid levels").count(), 1, "{t}");
        // Each failure's whole message is under the grid.
        let after = &t[t.find("Checks · ").unwrap()..];
        assert!(
            after.contains(
                "
 ✗ Bid levels: 2"
            ),
            "{t}"
        );
    }

    #[test]
    fn full_never_passes_its_width_and_compact_is_short() {
        for (b, a, q) in [
            (0, 0, 1.0),
            (2, 2, 1.0),
            (20, 20, 40_000.0),
            (200, 150, 1e9),
        ] {
            let v = view(b, a, q);
            let full = lines(&v, Size::Full);
            assert!(
                width(&full) <= FULL_WIDTH,
                "{b}/{a}: {}\n{}",
                width(&full),
                text(&full)
            );
            let compact = lines(&v, Size::Compact);
            assert!(compact.len() < full.len());
            assert!(width(&compact) <= 80, "{}", text(&compact));
        }
    }

    #[test]
    fn compact_has_a_one_line_book_and_a_check_count() {
        let t = text(&lines(&view(20, 20, 40_000.0), Size::Compact));
        assert!(
            t.contains("Book ask 1.000100 40.00K · bid 0.999900 40.00K"),
            "{t}"
        );
        assert!(t.contains("Checks · "), "{t}");
        assert!(!t.contains("█"), "{t}");
    }

    #[test]
    fn with_trends_leaves_out_what_they_chart() {
        let v = view(20, 20, 40_000.0);
        let t = text(&lines_with(
            &v,
            Opts {
                with_trends: true,
                ..Opts::of(Size::Full)
            },
        ));
        assert!(!t.contains("Mid "), "{t}");
        assert!(!t.contains("Depth "), "{t}");
        assert!(!t.contains("Top-3 "), "{t}");
        assert!(t.contains("Price ") && t.contains("Vol 24h"), "{t}");
        // One-shot: the key values, the top depths too.
        let t = text(&lines(&v, Size::Full));
        assert!(
            t.contains("Mid ") && t.contains("Top-3 ") && t.contains("Top-10 "),
            "{t}"
        );
    }

    #[test]
    fn the_grid_shows_limits_not_values_shown_above() {
        let t = text(&lines(&view(20, 20, 40_000.0), Size::Full));
        assert!(t.contains("✓ Spread ≤3%"), "{t}");
        assert!(t.contains("✓ Bid depth ≥3K"), "{t}");
        assert!(t.contains("✓ Bid top-10 ≥2K"), "{t}");
        assert!(
            t.contains("✓ Bid/Ask ratio 1.00x"),
            "a check with no limit shows its value: {t}"
        );
        assert_eq!(compact_number(300.0), "300");
        assert_eq!(compact_number(2500.0), "2.5K");
        assert_eq!(compact_number(0.5), "0.5");
    }

    #[test]
    fn fit_leaves_lines_out_until_it_fits() {
        let v = view(20, 20, 40_000.0);
        let all = lines(&v, Size::Full);
        let fitted = fit(&v, Opts::of(Size::Full), all.len() - 4);
        assert!(fitted.len() <= all.len() - 4, "{}", text(&fitted));
        assert!(text(&fitted).contains("✓ HEALTHY"), "the verdict stays");
        assert_eq!(fit(&v, Opts::of(Size::Full), 100), all, "room: no change");
    }

    #[test]
    fn short_check_texts() {
        assert_eq!(
            short(&HealthCheck::Pass(
                "Bid top-10 depth: 25012831 USDT (min 2000)".into()
            )),
            "Bid top-10 25.01M"
        );
        assert_eq!(shorten("≥1600000"), "≥1.60M");
        assert_eq!(shorten("5–10%);"), "5–10%);");
        assert_eq!(
            short(&HealthCheck::Pass(
                "Bid depth: 38544484 USDT (min 3000)".into()
            )),
            "Bid depth 38.54M"
        );
        assert_eq!(
            short(&HealthCheck::Pass("Spread: 0.001% (max 3.0%)".into())),
            "Spread 0.001%"
        );
        assert_eq!(
            short(&HealthCheck::Pass("No sells below peg".into())),
            "No sells below peg"
        );
    }
}
