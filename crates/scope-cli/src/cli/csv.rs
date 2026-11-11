//! The one CSV quoting function, shared by the writers of the CLI: the
//! watch trail and `token-health --format csv`. No second quoting function
//! exists (design 2026-11-11, section 7).

/// The characters that make a spreadsheet read a cell as a formula.
const FORMULA_TRIGGERS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// A CSV field, quoted when it holds a comma, a quote or a line break.
pub(crate) fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Text from a third party (a DEX symbol, a venue error) for a CSV cell.
/// A value that starts with a formula character gets a leading `'`, so a
/// spreadsheet shows it as text. Only text columns call this: a negative
/// number is correct data and stays as it is.
pub(crate) fn defuse_formula(s: &str) -> String {
    if s.starts_with(FORMULA_TRIGGERS) {
        format!("'{s}")
    } else {
        s.to_string()
    }
}

/// One CSV line from its cells: each cell quoted by [`csv_field`], the line
/// end an LF.
pub(crate) fn csv_line(cells: &[String]) -> String {
    let quoted: Vec<String> = cells.iter().map(|c| csv_field(c)).collect();
    format!("{}\n", quoted.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_is_quoted_only_when_a_reader_would_split_it() {
        // RFC 4180. A comma, a quote or a line break must be quoted, or the
        // row gets the wrong number of cells. A plain field must stay plain,
        // or the watch trail changes for every reader of it.
        assert_eq!(csv_field("plain text"), "plain text");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("a\nb"), "\"a\nb\"");
        assert_eq!(csv_field("a\rb"), "\"a\rb\"");
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn text_that_starts_like_a_formula_gets_a_leading_quote() {
        // A spreadsheet runs such a cell. Every trigger must be covered.
        for trigger in ["=1+1", "+1", "-1", "@SUM(A1)", "\tx", "\rx"] {
            assert_eq!(defuse_formula(trigger), format!("'{trigger}"));
        }
        // Text that only holds the character is not at risk.
        assert_eq!(defuse_formula("a=b"), "a=b");
        assert_eq!(defuse_formula("USDC"), "USDC");
        assert_eq!(defuse_formula(""), "");
    }

    #[test]
    fn a_line_has_one_cell_for_each_input_and_ends_with_lf() {
        let cells = ["a".to_string(), "b,c".to_string(), String::new()];
        assert_eq!(csv_line(&cells), "a,\"b,c\",\n");
    }
}
