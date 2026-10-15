//! # Short text
//!
//! The one way to shorten an identifier (address, hash, mint) for a
//! narrow column, and the one ellipsis every surface uses.

/// Marks text that was shortened or cut.
pub const ELLIPSIS: &str = "…";

/// How many characters of an identifier stay before the ellipsis.
const HEAD_CHARS: usize = 6;

/// How many characters of an identifier stay after the ellipsis.
const TAIL_CHARS: usize = 4;

/// `id` shortened to its start, [`ELLIPSIS`] and its end
/// (`0x742d…3c2b`), or `id` itself when that would not be shorter. It
/// counts characters, so any text is safe.
pub fn abbreviate(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= HEAD_CHARS + TAIL_CHARS + 1 {
        return id.to_string();
    }
    let head: String = chars[..HEAD_CHARS].iter().collect();
    let tail: String = chars[chars.len() - TAIL_CHARS..].iter().collect();
    format!("{head}{ELLIPSIS}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abbreviate_keeps_head_and_tail() {
        assert_eq!(
            abbreviate("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            "0xA0b8…eB48"
        );
    }

    #[test]
    fn test_short_ids_stay_whole() {
        assert_eq!(abbreviate(""), "");
        assert_eq!(abbreviate("0x12345678a"), "0x12345678a");
        assert_eq!(abbreviate("0x12345678ab"), "0x1234…78ab");
    }

    #[test]
    fn test_non_ascii_ids_do_not_panic() {
        // Byte slicing cut inside a character here.
        assert_eq!(abbreviate("ünïcødé-ädrèss-ñame"), "ünïcød…ñame");
        assert_eq!(
            abbreviate("日本語のアドレス文字列です"),
            "日本語のアド…字列です"
        );
    }
}
