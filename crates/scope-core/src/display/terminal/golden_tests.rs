//! Golden tests: they pin the exact bytes of the display helpers, plain
//! and colored, so a refactor of how the helpers paint text cannot change
//! the output by accident.

use super::*;

/// The terminal width the goldens render at. Narrow enough that the long
/// messages wrap, so continuation lines are pinned too.
const GOLDEN_WIDTH: usize = 40;

const LONG_MSG: &str = "Bid depth is below the minimum on both sides of the book";

/// Renders every pinned helper once, in the given color mode.
fn render_all(tty: bool) -> Vec<(&'static str, String)> {
    let cols = [
        Col {
            label: "Rank",
            width: 4,
            align: '>',
        },
        Col {
            label: "Address",
            width: 10,
            align: '<',
        },
    ];
    vec![
        ("section_header", section_header_styled("Token Health", tty)),
        ("subsection_header", subsection_header_styled("DEX", tty)),
        ("kv_row", kv_row_styled("Price", "$1.00", tty)),
        ("kv_row_wrapped", kv_row_styled("Note", LONG_MSG, tty)),
        (
            "kv_row_delta_up",
            kv_row_delta_styled("Change", 5.0, "+5%", tty),
        ),
        (
            "kv_row_delta_down",
            kv_row_delta_styled("Change", -3.0, "-3%", tty),
        ),
        (
            "kv_row_delta_flat",
            kv_row_delta_styled("Change", 0.0, "0%", tty),
        ),
        ("check_pass", check_pass_styled(LONG_MSG, tty)),
        ("check_fail", check_fail_styled(LONG_MSG, tty)),
        ("check_warn", check_warn_styled(LONG_MSG, tty)),
        ("check_na", check_na_styled(LONG_MSG, tty)),
        ("status_healthy", status_line_styled(true, tty)),
        ("status_unhealthy", status_line_styled(false, tty)),
        ("section_footer", section_footer_styled(tty)),
        ("separator", separator_styled(tty)),
        ("blank_row", blank_row_styled(tty)),
        ("price_peg_near", format_price_peg_styled(1.0001, 1.0, tty)),
        ("price_peg_mid", format_price_peg_styled(1.003, 1.0, tty)),
        ("price_peg_far", format_price_peg_styled(0.95, 1.0, tty)),
        (
            "orderbook_level",
            orderbook_level_styled(1.0001, 500.0, "DAI", 500.05, 1.0, tty),
        ),
        ("score_bar_high", score_bar_styled("Score", 85, 100, tty)),
        ("score_bar_mid", score_bar_styled("Score", 55, 100, tty)),
        ("score_bar_low", score_bar_styled("Score", 20, 100, tty)),
        ("severity_critical", severity_label_styled("Critical", tty)),
        ("severity_high", severity_label_styled("High", tty)),
        ("severity_medium", severity_label_styled("Medium", tty)),
        ("severity_low", severity_label_styled("Low", tty)),
        ("severity_other", severity_label_styled("Info", tty)),
        ("warning_row", warning_row_styled(LONG_MSG, tty)),
        ("info_row", info_row_styled(LONG_MSG, tty)),
        (
            "link_row",
            link_row_styled("Explorer", "https://example.com", tty),
        ),
        ("detail_row", detail_row_styled(LONG_MSG, tty)),
        ("bullet_row", bullet_row_styled(LONG_MSG, tty)),
        ("table_header", table_header_styled(&cols, tty)),
        ("table_row", table_row_styled(&cols, &["1", "0xdAC1"], tty)),
        ("numbered_row", numbered_row_styled(3, LONG_MSG, tty)),
    ]
}

/// Renders at [`GOLDEN_WIDTH`] under the width lock.
fn render_at_golden_width(tty: bool) -> Vec<(&'static str, String)> {
    let _lock = width_lock();
    set_width_override(Some(GOLDEN_WIDTH));
    let out = render_all(tty);
    set_width_override(None);
    out
}

/// Plain bytes (`NO_COLOR`, pipes). The CLI-visible plain output: it must
/// never change.
const PLAIN: &[(&str, &str)] = &[
    (
        "section_header",
        "\n┌─ Token Health ────────────────────────",
    ),
    ("subsection_header", "│\n├── DEX"),
    ("kv_row", "│  Price             $1.00"),
    (
        "kv_row_wrapped",
        "│  Note              Bid depth is below\n│                    the minimum on both\n│                    sides of the book",
    ),
    ("kv_row_delta_up", "│  Change            +5%"),
    ("kv_row_delta_down", "│  Change            -3%"),
    ("kv_row_delta_flat", "│  Change            0%"),
    (
        "check_pass",
        "│  ✓ Bid depth is below the minimum on\n│    both sides of the book",
    ),
    (
        "check_fail",
        "│  ✗ Bid depth is below the minimum on\n│    both sides of the book",
    ),
    (
        "check_warn",
        "│  ! Bid depth is below the minimum on\n│    both sides of the book",
    ),
    (
        "check_na",
        "│  – Bid depth is below the minimum on\n│    both sides of the book",
    ),
    ("status_healthy", "│  HEALTHY"),
    ("status_unhealthy", "│  UNHEALTHY"),
    ("section_footer", "└───────────────────────────────────────"),
    ("separator", "├───────────────────────────────────────"),
    ("blank_row", "│"),
    ("price_peg_near", "1.0001"),
    ("price_peg_mid", "1.0030"),
    ("price_peg_far", "0.9500"),
    (
        "orderbook_level",
        "│    1.0001      500.00 DAI      500.05 USDT",
    ),
    (
        "score_bar_high",
        "│  Score             [█████████████████───] 85/100",
    ),
    (
        "score_bar_mid",
        "│  Score             [███████████─────────] 55/100",
    ),
    (
        "score_bar_low",
        "│  Score             [████────────────────] 20/100",
    ),
    ("severity_critical", "Critical"),
    ("severity_high", "High"),
    ("severity_medium", "Medium"),
    ("severity_low", "Low"),
    ("severity_other", "Info"),
    (
        "warning_row",
        "│  ⚠ Bid depth is below the minimum on\n│    both sides of the book",
    ),
    (
        "info_row",
        "│  ℹ Bid depth is below the minimum on\n│    both sides of the book",
    ),
    ("link_row", "│  Explorer          https://example.com"),
    (
        "detail_row",
        "│      Bid depth is below the minimum on\n│      both sides of the book",
    ),
    (
        "bullet_row",
        "│    • Bid depth is below the minimum on\n│      both sides of the book",
    ),
    ("table_header", "│    Rank  Address\n│  ──────────────────"),
    ("table_row", "│       1  0xdAC1"),
    (
        "numbered_row",
        "│  3. Bid depth is below the minimum on\n│     both sides of the book",
    ),
];

/// Colored bytes. Seven entries changed in the theme refactor: six only in
/// escape order (proved by `reordered_escapes_look_the_same`), and the section
/// title color (`section_title_moves_from_bright_white_to_accent`).
const COLORED: &[(&str, &str)] = &[
    (
        "section_header",
        "\n\u{1b}[36m┌─\u{1b}[39m \u{1b}[36m\u{1b}[1mToken Health\u{1b}[0m\u{1b}[39m \u{1b}[36m────────────────────────\u{1b}[39m",
    ),
    (
        "subsection_header",
        "\u{1b}[36m│\u{1b}[39m\n\u{1b}[36m├── \u{1b}[39m\u{1b}[36m\u{1b}[1mDEX\u{1b}[0m\u{1b}[39m",
    ),
    (
        "kv_row",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mPrice             \u{1b}[0m$1.00",
    ),
    (
        "kv_row_wrapped",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mNote              \u{1b}[0mBid depth is below\n\u{1b}[36m│\u{1b}[39m                    the minimum on both\n\u{1b}[36m│\u{1b}[39m                    sides of the book",
    ),
    (
        "kv_row_delta_up",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mChange            \u{1b}[0m\u{1b}[32m+5%\u{1b}[39m",
    ),
    (
        "kv_row_delta_down",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mChange            \u{1b}[0m\u{1b}[31m-3%\u{1b}[39m",
    ),
    (
        "kv_row_delta_flat",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mChange            \u{1b}[0m\u{1b}[2m0%\u{1b}[0m",
    ),
    (
        "check_pass",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[32m✓\u{1b}[39m Bid depth is below the minimum on\n\u{1b}[36m│\u{1b}[39m    both sides of the book",
    ),
    (
        "check_fail",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[31m✗\u{1b}[39m Bid depth is below the minimum on\n\u{1b}[36m│\u{1b}[39m    both sides of the book",
    ),
    (
        "check_warn",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[33m\u{1b}[1m!\u{1b}[0m\u{1b}[39m \u{1b}[33mBid depth is below the minimum on\u{1b}[39m\n\u{1b}[36m│\u{1b}[39m    \u{1b}[33mboth sides of the book\u{1b}[39m",
    ),
    (
        "check_na",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2m–\u{1b}[0m \u{1b}[2mBid depth is below the minimum on\u{1b}[0m\n\u{1b}[36m│\u{1b}[39m    \u{1b}[2mboth sides of the book\u{1b}[0m",
    ),
    (
        "status_healthy",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[32m\u{1b}[1mHEALTHY\u{1b}[0m\u{1b}[39m",
    ),
    (
        "status_unhealthy",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[31m\u{1b}[1mUNHEALTHY\u{1b}[0m\u{1b}[39m",
    ),
    (
        "section_footer",
        "\u{1b}[36m└───────────────────────────────────────\u{1b}[39m",
    ),
    (
        "separator",
        "\u{1b}[36m├───────────────────────────────────────\u{1b}[39m",
    ),
    ("blank_row", "\u{1b}[36m│\u{1b}[39m"),
    ("price_peg_near", "\u{1b}[32m1.0001\u{1b}[39m"),
    ("price_peg_mid", "\u{1b}[33m1.0030\u{1b}[39m"),
    ("price_peg_far", "\u{1b}[31m0.9500\u{1b}[39m"),
    (
        "orderbook_level",
        "\u{1b}[36m│\u{1b}[39m    \u{1b}[32m1.0001\u{1b}[39m      500.00 \u{1b}[2mDAI\u{1b}[0m      500.05 USDT",
    ),
    (
        "score_bar_high",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mScore             \u{1b}[0m\u{1b}[32m[█████████████████───] 85/100\u{1b}[39m",
    ),
    (
        "score_bar_mid",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mScore             \u{1b}[0m\u{1b}[33m[███████████─────────] 55/100\u{1b}[39m",
    ),
    (
        "score_bar_low",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mScore             \u{1b}[0m\u{1b}[31m[████────────────────] 20/100\u{1b}[39m",
    ),
    (
        "severity_critical",
        "\u{1b}[31m\u{1b}[1mCritical\u{1b}[0m\u{1b}[39m",
    ),
    ("severity_high", "\u{1b}[31mHigh\u{1b}[39m"),
    ("severity_medium", "\u{1b}[33mMedium\u{1b}[39m"),
    ("severity_low", "\u{1b}[36mLow\u{1b}[39m"),
    ("severity_other", "\u{1b}[2mInfo\u{1b}[0m"),
    (
        "warning_row",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[33m\u{1b}[1m⚠\u{1b}[0m\u{1b}[39m \u{1b}[33mBid depth is below the minimum on\u{1b}[39m\n\u{1b}[36m│\u{1b}[39m    \u{1b}[33mboth sides of the book\u{1b}[39m",
    ),
    (
        "info_row",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[34mℹ\u{1b}[39m \u{1b}[2mBid depth is below the minimum on\u{1b}[0m\n\u{1b}[36m│\u{1b}[39m    \u{1b}[2mboth sides of the book\u{1b}[0m",
    ),
    (
        "link_row",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2mExplorer          \u{1b}[0m\u{1b}[4mhttps://example.com\u{1b}[0m",
    ),
    (
        "detail_row",
        "\u{1b}[36m│\u{1b}[39m      \u{1b}[2mBid depth is below the minimum on\u{1b}[0m\n\u{1b}[36m│\u{1b}[39m      \u{1b}[2mboth sides of the book\u{1b}[0m",
    ),
    (
        "bullet_row",
        "\u{1b}[36m│\u{1b}[39m    \u{1b}[2m•\u{1b}[0m Bid depth is below the minimum on\n\u{1b}[36m│\u{1b}[39m      both sides of the book",
    ),
    (
        "table_header",
        "\u{1b}[36m│\u{1b}[39m    \u{1b}[36m\u{1b}[1mRank  Address\u{1b}[0m\u{1b}[39m\n\u{1b}[36m│\u{1b}[39m  \u{1b}[36m──────────────────\u{1b}[39m",
    ),
    ("table_row", "\u{1b}[36m│\u{1b}[39m       1  0xdAC1"),
    (
        "numbered_row",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[2m3.\u{1b}[0m Bid depth is below the minimum on\n\u{1b}[36m│\u{1b}[39m     both sides of the book",
    ),
];

/// Asserts that each helper renders exactly its pinned bytes, and that
/// the goldens name every helper the render covers.
fn assert_goldens(tty: bool, goldens: &[(&str, &str)]) {
    let rendered = render_at_golden_width(tty);
    assert_eq!(rendered.len(), goldens.len(), "golden list out of date");
    let changed: Vec<String> = rendered
        .iter()
        .zip(goldens)
        .filter(|((name, got), (golden_name, want))| {
            assert_eq!(name, golden_name, "golden order out of date");
            got != want
        })
        .map(|((name, got), _)| format!("{name} (tty={tty}) is now {got:?}"))
        .collect();
    assert!(changed.is_empty(), "changed:\n{}", changed.join("\n"));
}

#[test]
fn plain_output_matches_goldens() {
    assert_goldens(false, PLAIN);
}

#[test]
fn colored_output_matches_goldens() {
    assert_goldens(true, COLORED);
}

/// Colored bytes of the helpers whose escape order changed in the theme
/// refactor, as they were before it. Kept to prove the new bytes look the
/// same: see `reordered_escapes_look_the_same`.
const COLORED_BEFORE_THEME: &[(&str, &str)] = &[
    (
        "section_header",
        "\n\u{1b}[36m┌─\u{1b}[39m \u{1b}[97m\u{1b}[1mToken Health\u{1b}[0m\u{1b}[39m \u{1b}[36m────────────────────────\u{1b}[39m",
    ),
    (
        "subsection_header",
        "\u{1b}[36m│\u{1b}[39m\n\u{1b}[36m├── \u{1b}[1mDEX\u{1b}[0m\u{1b}[39m",
    ),
    (
        "check_warn",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[1m\u{1b}[33m!\u{1b}[39m\u{1b}[0m \u{1b}[33mBid depth is below the minimum on\u{1b}[39m\n\u{1b}[36m│\u{1b}[39m    \u{1b}[33mboth sides of the book\u{1b}[39m",
    ),
    (
        "status_healthy",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[1m\u{1b}[32mHEALTHY\u{1b}[39m\u{1b}[0m",
    ),
    (
        "status_unhealthy",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[1m\u{1b}[31mUNHEALTHY\u{1b}[39m\u{1b}[0m",
    ),
    (
        "severity_critical",
        "\u{1b}[1m\u{1b}[31mCritical\u{1b}[39m\u{1b}[0m",
    ),
    (
        "warning_row",
        "\u{1b}[36m│\u{1b}[39m  \u{1b}[1m\u{1b}[33m⚠\u{1b}[39m\u{1b}[0m \u{1b}[33mBid depth is below the minimum on\u{1b}[39m\n\u{1b}[36m│\u{1b}[39m    \u{1b}[33mboth sides of the book\u{1b}[39m",
    ),
];

/// The look of one visible character: what a terminal shows for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CellStyle {
    foreground: Option<u8>,
    bold: bool,
    dim: bool,
    underline: bool,
    reverse: bool,
}

/// Interprets the SGR escapes in `s` the way a terminal does and returns
/// each visible character with its look. Two strings with equal cells
/// look the same, whatever order their escapes come in.
fn cells(s: &str) -> Vec<(char, CellStyle)> {
    let mut style = CellStyle::default();
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push((ch, style));
            continue;
        }
        assert_eq!(chars.next(), Some('['), "not an SGR escape in {s:?}");
        let code: String = chars.by_ref().take_while(|c| *c != 'm').collect();
        match code.parse::<u8>().expect("one SGR code per escape") {
            0 => style = CellStyle::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            4 => style.underline = true,
            7 => style.reverse = true,
            39 => style.foreground = None,
            fg @ (30..=37 | 90..=97) => style.foreground = Some(fg),
            other => panic!("unexpected SGR code {other} in {s:?}"),
        }
    }
    out
}

fn colored_golden(name: &str) -> &'static str {
    COLORED
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .expect("a colored golden")
}

/// The theme painter always puts the color outside the effects; some
/// helpers had the other order before. The order must not change what a
/// terminal shows.
#[test]
fn reordered_escapes_look_the_same() {
    for (name, before) in COLORED_BEFORE_THEME {
        if *name == "section_header" {
            continue;
        }
        assert_eq!(cells(before), cells(colored_golden(name)), "{name}");
    }
}

/// The one intended change to colored output: the section title moves
/// from bright white (unreadable on a light terminal) to bold accent,
/// like every other heading. The rest of the header looks the same.
#[test]
fn section_title_moves_from_bright_white_to_accent() {
    const BRIGHT_WHITE: u8 = 97;
    const CYAN: u8 = 36;
    let (_, before) = COLORED_BEFORE_THEME
        .iter()
        .find(|(n, _)| *n == "section_header")
        .expect("the section header before the theme");
    let recolored: Vec<_> = cells(before)
        .into_iter()
        .map(|(ch, mut style)| {
            if style.foreground == Some(BRIGHT_WHITE) {
                style.foreground = Some(CYAN);
            }
            (ch, style)
        })
        .collect();
    assert_ne!(cells(before), recolored, "the title was bright white");
    assert_eq!(recolored, cells(colored_golden("section_header")));
}
