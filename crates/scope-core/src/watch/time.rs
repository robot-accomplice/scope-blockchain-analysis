//! Intervals and durations, shared by `market summary --every` and
//! watchlists, so both read `30s` the same way and have the same defaults
//! (#47, Q7).

/// Default interval between runs (60 seconds).
pub const DEFAULT_INTERVAL_SECS: u64 = 60;

/// Default total duration of a repeat (1 hour).
pub const DEFAULT_DURATION_SECS: u64 = 3600;

/// Duration words that mean "until the user stops it" (#96).
pub const UNTIL_CANCELLED: [&str; 2] = ["forever", "inf"];

/// `forever` or `inf`, in any case.
pub fn is_until_cancelled(s: &str) -> bool {
    UNTIL_CANCELLED.contains(&s.trim().to_ascii_lowercase().as_str())
}

/// Parses `30s`, `5m`, `1h`, `2d` or a bare number of seconds into whole
/// seconds. Zero and negative values are errors.
pub fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty duration".to_string());
    }
    let (num_str, unit) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| (&s[..i], s[i..].trim()))
        .unwrap_or((s, "s"));

    let num: f64 = num_str
        .parse()
        .map_err(|_| format!("Invalid duration number: {}", num_str))?;

    if num <= 0.0 {
        return Err("Duration must be positive".to_string());
    }

    let secs = match unit.to_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => num,
        "m" | "min" | "mins" | "minute" | "minutes" => num * 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => num * 3600.0,
        "d" | "day" | "days" => num * 86400.0,
        _ => return Err(format!("Unknown duration unit: {}", unit)),
    };

    Ok(secs as u64)
}

/// The shortest text that [`parse_duration`] reads back as `secs`.
pub fn format_duration(secs: u64) -> String {
    match secs {
        s if s > 0 && s % 86400 == 0 => format!("{}d", s / 86400),
        s if s > 0 && s % 3600 == 0 => format!("{}h", s / 3600),
        s if s > 0 && s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_parse() {
        assert_eq!(parse_duration("30s"), Ok(30));
        assert_eq!(parse_duration("5m"), Ok(300));
        assert_eq!(parse_duration("1h"), Ok(3600));
        assert_eq!(parse_duration("2d"), Ok(172_800));
        assert_eq!(parse_duration("45"), Ok(45));
        assert_eq!(parse_duration("1.5m"), Ok(90));
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("").is_err());
        assert!(
            parse_duration("3w")
                .unwrap_err()
                .contains("Unknown duration unit")
        );
    }

    #[test]
    fn format_reads_back() {
        for secs in [1, 30, 60, 90, 3600, 5400, 86400] {
            assert_eq!(parse_duration(&format_duration(secs)), Ok(secs));
        }
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(90), "90s");
    }

    #[test]
    fn until_cancelled_words() {
        assert!(is_until_cancelled("forever"));
        assert!(is_until_cancelled(" INF "));
        assert!(!is_until_cancelled("1h"));
    }
}
