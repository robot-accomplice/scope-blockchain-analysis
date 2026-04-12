//! Error display and remediation hints for CLI output.
//!
//! Colors errors red and hints dimmed when stderr is a TTY.
//! Falls back to plain text when piped.

use owo_colors::OwoColorize;
use scope::error::ScopeError;
use std::io::IsTerminal;

/// Returns `true` when stderr is an interactive terminal.
fn is_tty_stderr() -> bool {
    std::io::stderr().is_terminal()
}

/// Displays an error with a remediation hint when available.
///
/// Uses color when stderr is a TTY, plain text otherwise.
pub fn display_error(e: &ScopeError) {
    display_error_styled(e, is_tty_stderr())
}

/// Internal styled implementation, testable with an explicit `tty` flag.
fn display_error_styled(e: &ScopeError, tty: bool) {
    let msg = match e {
        ScopeError::NotFound(inner) => inner.clone(),
        other => format!("{}", other),
    };

    if tty {
        eprintln!("\n  {} {}", "✗".red().bold(), msg.red());
    } else {
        eprintln!("\n  ✗ {}", msg);
    }

    if let Some(hint) = error_suggestion(e) {
        if tty {
            eprintln!("\n  {}", hint.dimmed());
        } else {
            eprintln!("\n  {}", hint);
        }
    }
    eprintln!();
}

/// Returns a user-facing suggestion for common error types.
pub fn error_suggestion(e: &ScopeError) -> Option<&'static str> {
    match e {
        ScopeError::InvalidAddress(_) => Some(
            "Ensure the address format matches the target chain.\n      \
             EVM: 0x followed by 40 hex characters\n      \
             Solana: base58 encoded public key\n      \
             Tron: T followed by base58 characters",
        ),
        ScopeError::InvalidHash(_) => Some(
            "Ensure the transaction hash matches the target chain.\n      \
             EVM: 0x followed by 64 hex characters\n      \
             Solana: base58 encoded signature",
        ),
        ScopeError::Config(_) => Some("Run `scope setup` to create or repair your configuration."),
        ScopeError::Request(_) | ScopeError::Network(_) => Some(
            "Check your network connection and try again.\n      \
             Use -v for more details on the failing request.",
        ),
        ScopeError::Api(msg)
            if msg.contains("401") || msg.contains("403") || msg.contains("key") =>
        {
            Some(
                "Your API key may be missing or invalid.\n      Run `scope setup --key <provider>` to configure it.",
            )
        }
        ScopeError::NotFound(_) => Some(
            "The resource was not found. Verify the address, hash, or token exists on the specified chain.",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ================================================================
    // display_error (delegates to non-TTY in CI)
    // ================================================================

    #[test]
    fn test_display_error_not_found() {
        let err = ScopeError::NotFound("test resource".into());
        display_error(&err);
    }

    #[test]
    fn test_display_error_invalid_address() {
        let err = ScopeError::InvalidAddress("0xbad".into());
        display_error(&err);
    }

    #[test]
    fn test_display_error_other() {
        let err = ScopeError::Other("something went wrong".into());
        display_error(&err);
    }

    #[test]
    fn test_display_error_chain() {
        let err = ScopeError::Chain("chain error".into());
        display_error(&err);
    }

    #[test]
    fn test_display_error_api() {
        let err = ScopeError::Api("500 Internal Server Error".into());
        display_error(&err);
    }

    // ================================================================
    // display_error_styled — TTY branch (colored output)
    // ================================================================

    #[test]
    fn test_display_error_styled_tty_not_found() {
        let err = ScopeError::NotFound("test resource".into());
        display_error_styled(&err, true);
    }

    #[test]
    fn test_display_error_styled_tty_invalid_address() {
        let err = ScopeError::InvalidAddress("0xbad".into());
        display_error_styled(&err, true);
    }

    #[test]
    fn test_display_error_styled_tty_config() {
        use scope::error::ConfigError;
        let err = ScopeError::Config(ConfigError::NotFound {
            path: std::path::PathBuf::from("/missing"),
        });
        display_error_styled(&err, true);
    }

    #[test]
    fn test_display_error_styled_tty_network() {
        let err = ScopeError::Network("timeout".into());
        display_error_styled(&err, true);
    }

    #[test]
    fn test_display_error_styled_tty_api_auth() {
        let err = ScopeError::Api("401 Unauthorized".into());
        display_error_styled(&err, true);
    }

    #[test]
    fn test_display_error_styled_tty_other_no_hint() {
        let err = ScopeError::Other("random".into());
        display_error_styled(&err, true);
    }

    #[test]
    fn test_display_error_styled_non_tty() {
        let err = ScopeError::NotFound("test".into());
        display_error_styled(&err, false);
    }

    // ================================================================
    // error_suggestion
    // ================================================================

    #[test]
    fn test_error_suggestion_invalid_address() {
        let err = ScopeError::InvalidAddress("bad".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("EVM"));
    }

    #[test]
    fn test_error_suggestion_invalid_hash() {
        let err = ScopeError::InvalidHash("bad".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("64 hex"));
    }

    #[test]
    fn test_error_suggestion_config() {
        use scope::error::ConfigError;
        let err = ScopeError::Config(ConfigError::NotFound {
            path: std::path::PathBuf::from("/missing"),
        });
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("scope setup"));
    }

    #[test]
    fn test_error_suggestion_network() {
        let err = ScopeError::Network("timeout".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("network"));
    }

    #[test]
    fn test_error_suggestion_api_auth() {
        let err = ScopeError::Api("401 Unauthorized".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("API key"));
    }

    #[test]
    fn test_error_suggestion_api_key_keyword() {
        let err = ScopeError::Api("invalid api key".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
    }

    #[test]
    fn test_error_suggestion_api_no_auth() {
        let err = ScopeError::Api("500 Internal Server Error".into());
        assert!(error_suggestion(&err).is_none());
    }

    #[test]
    fn test_error_suggestion_not_found() {
        let err = ScopeError::NotFound("address".into());
        let hint = error_suggestion(&err);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("not found"));
    }

    #[test]
    fn test_error_suggestion_other_returns_none() {
        let err = ScopeError::Other("random".into());
        assert!(error_suggestion(&err).is_none());
    }
}
