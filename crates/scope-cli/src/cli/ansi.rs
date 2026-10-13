//! # ANSI escape sequences
//!
//! The one grammar for the escape sequences in command output. Both the
//! copy for Ctrl-Y and `save` (which strips them) and the bounded TUI copy
//! (which must not cut one in half) read it from here.
//!
//! Recognized, as bytes of UTF-8 text:
//! - CSI: `ESC [` or U+009B, parameters, one final byte (0x40..=0x7E).
//! - Strings: OSC (`ESC ]`), DCS (`ESC P`), SOS (`ESC X`), PM (`ESC ^`),
//!   APC (`ESC _`), up to BEL, ST (`ESC \` or U+009C), or a lone ESC.
//! - `ESC`, intermediate bytes (0x20..=0x2F), one final character.
//! - Any other two-character escape: `ESC` and the next character.
//!
//! An unterminated sequence runs to the end of the text.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// The UTF-8 lead byte of U+0080..=U+00BF, which holds the C1 controls.
const C1_LEAD: u8 = 0xc2;
/// U+009B (CSI) and U+009C (ST), second UTF-8 byte.
const C1_CSI: u8 = 0x9b;
const C1_ST: u8 = 0x9c;

/// When an escape sequence starts at `at` in `bytes`, the index just after
/// it; otherwise `None`. The result is always a UTF-8 character boundary
/// of valid text.
pub(crate) fn escape_end(bytes: &[u8], at: usize) -> Option<usize> {
    match (bytes.get(at)?, bytes.get(at + 1)) {
        (&C1_LEAD, Some(&C1_CSI)) => Some(csi_end(bytes, at + 2)),
        (&ESC, None) => Some(at + 1),
        (&ESC, Some(b'[')) => Some(csi_end(bytes, at + 2)),
        (&ESC, Some(b']' | b'P' | b'X' | b'^' | b'_')) => Some(string_end(bytes, at + 2)),
        (&ESC, Some(0x20..=0x2f)) => Some(intermediates_end(bytes, at + 1)),
        (&ESC, Some(&next)) => Some(at + 1 + char_len(next).min(bytes.len() - at - 1)),
        _ => None,
    }
}

/// `text` without its escape sequences.
pub(crate) fn strip(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut kept) = (0, 0);
    while i < bytes.len() {
        match escape_end(bytes, i) {
            Some(end) => {
                out.push_str(&text[kept..i]);
                (i, kept) = (end, end);
            }
            None => i += 1,
        }
    }
    out.push_str(&text[kept..]);
    out
}

/// After the CSI parameters that start at `from`: just past the final
/// byte, or the end.
fn csi_end(bytes: &[u8], from: usize) -> usize {
    bytes[from.min(bytes.len())..]
        .iter()
        .position(|b| (0x40..=0x7e).contains(b))
        .map_or(bytes.len(), |i| from + i + 1)
}

/// After a string body that starts at `from`: past BEL, ST or a lone ESC,
/// or the end.
fn string_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1)) {
            (BEL, _) => return i + 1,
            (C1_LEAD, Some(&C1_ST)) => return i + 2,
            (ESC, Some(b'\\')) => return i + 2,
            (ESC, _) => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// After the intermediate bytes from `from` and the character that ends
/// them, or the end.
fn intermediates_end(bytes: &[u8], from: usize) -> usize {
    match bytes[from..]
        .iter()
        .position(|b| !(0x20..=0x2f).contains(b))
    {
        Some(i) => {
            let last = from + i;
            last + char_len(bytes[last]).min(bytes.len() - last)
        }
        None => bytes.len(),
    }
}

/// The length of the UTF-8 character whose first byte is `lead`.
fn char_len(lead: u8) -> usize {
    match lead {
        0xf0..=0xff => 4,
        0xe0..=0xef => 3,
        0xc0..=0xdf => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_removes_sgr() {
        assert_eq!(strip("\x1b[1;36mA\x1b[0m b"), "A b");
    }

    #[test]
    fn test_strip_ansi_removes_osc_with_either_terminator() {
        assert_eq!(strip("\x1b]8;;http://x\x1b\\link\x1b]8;;\x07!"), "link!");
    }

    #[test]
    fn test_strip_ansi_removes_dcs_sos_pm_apc_strings() {
        for intro in ['P', 'X', '^', '_'] {
            let text = format!("a\x1b{intro}payload\x1b\\b");
            assert_eq!(strip(&text), "ab", "ESC {intro}");
            let bel = format!("a\x1b{intro}payload\x07b");
            assert_eq!(strip(&bel), "ab", "ESC {intro} with BEL");
        }
    }

    #[test]
    fn test_strip_ansi_consumes_intermediates_through_the_final_byte() {
        assert_eq!(strip("a\x1b(Bb\x1b)0c"), "abc");
        assert_eq!(strip("a\x1bXb"), "a", "ESC X opens a string");
        assert_eq!(strip("a\x1b=b"), "ab", "plain two-byte escape");
    }

    #[test]
    fn test_strip_ansi_removes_the_8_bit_csi() {
        assert_eq!(strip("a\u{9b}1;31mb\u{9b}0mc"), "abc");
    }

    #[test]
    fn test_strip_ansi_unterminated_sequence_drops_only_itself() {
        assert_eq!(strip("keep\x1b[1;3"), "keep");
        assert_eq!(strip("keep\x1b]0;title"), "keep");
        assert_eq!(strip("keep\x1b"), "keep");
        assert_eq!(strip("keep\x1b("), "keep");
    }

    #[test]
    fn test_strip_handles_c1_csi_and_st() {
        assert_eq!(strip("a\u{9b}31mb"), "ab");
        assert_eq!(strip("a\x1b]0;title\u{9c}b"), "ab");
    }

    #[test]
    fn test_two_character_escape_takes_a_whole_character() {
        // ESC then a multi-byte character: the end is a character
        // boundary, so the rest stays valid text.
        assert_eq!(strip("a\x1béb"), "ab");
        assert_eq!(strip("a\x1bMb"), "ab");
    }

    #[test]
    fn test_escape_end_is_none_outside_a_sequence() {
        assert_eq!(escape_end(b"abc", 1), None);
        assert_eq!(escape_end(b"abc", 9), None);
        assert_eq!(escape_end("é".as_bytes(), 0), None);
        assert_eq!(escape_end(b"x\x1b[1mz", 1), Some(5));
    }
}
