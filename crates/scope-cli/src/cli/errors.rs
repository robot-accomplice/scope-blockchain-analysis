//! Error display and remediation hints for CLI output.
//!
//! Colors errors red and hints dimmed when stderr is a TTY.
//! Falls back to plain text when piped.

use crate::cli::output::Output;
use crate::errln;
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
pub fn display_error(e: &ScopeError, out: &Output) -> std::io::Result<()> {
    display_error_styled(e, is_tty_stderr(), out)
}

/// Internal styled implementation, testable with an explicit `tty` flag.
fn display_error_styled(e: &ScopeError, tty: bool, out: &Output) -> std::io::Result<()> {
    let msg = match e {
        ScopeError::NotFound(inner) => inner.clone(),
        other => format!("{}", other),
    };

    if tty {
        errln!(out, "\n  {} {}", "✗".red().bold(), msg.red())?;
    } else {
        errln!(out, "\n  ✗ {}", msg)?;
    }

    // The source chain names the real cause (for example a TLS failure
    // under "error sending request"). Show each link once.
    let causes = cause_chain(e, &msg);
    for cause in &causes {
        if tty {
            errln!(out, "    {} {}", "caused by:".dimmed(), cause)?;
        } else {
            errln!(out, "    caused by: {}", cause)?;
        }
    }

    let hint = if causes.iter().any(|c| is_non_tls_reply(c)) {
        Some(INTERCEPTION_HINT)
    } else {
        error_suggestion(e)
    };
    if let Some(hint) = hint {
        if tty {
            errln!(out, "\n  {}", hint.dimmed())?;
        } else {
            errln!(out, "\n  {}", hint)?;
        }
    }
    errln!(out)?;
    Ok(())
}

/// Shown when a TLS handshake got a non-TLS reply.
const INTERCEPTION_HINT: &str = "The server's reply was not TLS. A router, firewall, proxy or VPN on your\n      \
     network may be blocking this host and answering with its own block page.\n      \
     Allow the host in that device, or try another network.";

/// The messages of `e`'s source chain, without links whose text is already
/// in `shown` (the top-level message usually repeats the first source).
fn cause_chain(e: &ScopeError, shown: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        let text = s.to_string();
        if !shown.contains(&text) && !out.contains(&text) {
            out.push(text);
        }
        src = s.source();
    }
    out
}

/// True when a cause says the peer answered a TLS handshake with non-TLS
/// data (rustls: InvalidContentType; OpenSSL: wrong version number).
fn is_non_tls_reply(cause: &str) -> bool {
    cause.contains("InvalidContentType") || cause.contains("wrong version number")
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

    /// Reproduces a router block page: a TCP server on 127.0.0.1 that answers
    /// the TLS ClientHello with plaintext HTTP (what ASUS AiProtection did to
    /// api.dexscreener.com). Returns the error reqwest gives for it.
    async fn plaintext_reply_error() -> ScopeError {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
        });
        let err = reqwest::Client::new()
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap_err();
        ScopeError::Request(err)
    }

    #[tokio::test]
    async fn test_request_error_shows_its_cause_chain() {
        // "error sending request" alone hid the real cause for an hour.
        let e = plaintext_reply_error().await;
        let (o, cap) = crate::cli::output::Output::capture();
        display_error_styled(&e, false, &o).unwrap();
        let text = cap.err();
        assert!(text.contains("caused by:"), "{}", text);
        assert!(
            text.contains("InvalidContentType") || text.contains("corrupt message"),
            "{}",
            text
        );
    }

    #[tokio::test]
    async fn test_non_tls_reply_gets_an_interception_hint() {
        // A plaintext reply to a TLS handshake means something on the
        // network path answered instead of the server.
        let e = plaintext_reply_error().await;
        let (o, cap) = crate::cli::output::Output::capture();
        display_error_styled(&e, false, &o).unwrap();
        let text = cap.err();
        assert!(text.contains("router, firewall, proxy or VPN"), "{}", text);
        assert!(!text.contains("Check your network connection"), "{}", text);
    }

    #[test]
    fn test_error_without_source_prints_no_cause_lines() {
        let (o, cap) = crate::cli::output::Output::capture();
        display_error_styled(&ScopeError::Other("plain".into()), false, &o).unwrap();
        assert!(!cap.err().contains("caused by"));
    }

    fn quiet() -> crate::cli::output::Output {
        crate::cli::output::Output::capture().0
    }

    // ================================================================
    // display_error (delegates to non-TTY in CI)
    // ================================================================

    #[test]
    fn test_display_error_goes_to_err_channel_with_hint() {
        // Piped stdout must stay clean on failure, and the user still needs
        // the message and the remediation hint.
        let (o, cap) = crate::cli::output::Output::capture();
        display_error_styled(&ScopeError::InvalidAddress("0xbad".into()), false, &o).unwrap();
        assert_eq!(cap.out(), "");
        assert!(
            cap.err().contains("✗ Invalid address format: 0xbad"),
            "{}",
            cap.err()
        );
        assert!(cap.err().contains("EVM: 0x followed by 40 hex characters"));
    }

    #[test]
    fn test_display_error_not_found() {
        let err = ScopeError::NotFound("test resource".into());
        display_error(&err, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_invalid_address() {
        let err = ScopeError::InvalidAddress("0xbad".into());
        display_error(&err, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_other() {
        let err = ScopeError::Other("something went wrong".into());
        display_error(&err, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_chain() {
        let err = ScopeError::Chain("chain error".into());
        display_error(&err, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_api() {
        let err = ScopeError::Api("500 Internal Server Error".into());
        display_error(&err, &quiet()).unwrap();
    }

    // ================================================================
    // display_error_styled — TTY branch (colored output)
    // ================================================================

    #[test]
    fn test_display_error_styled_tty_not_found() {
        let err = ScopeError::NotFound("test resource".into());
        display_error_styled(&err, true, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_styled_tty_invalid_address() {
        let err = ScopeError::InvalidAddress("0xbad".into());
        display_error_styled(&err, true, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_styled_tty_config() {
        use scope::error::ConfigError;
        let err = ScopeError::Config(ConfigError::NotFound {
            path: std::path::PathBuf::from("/missing"),
        });
        display_error_styled(&err, true, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_styled_tty_network() {
        let err = ScopeError::Network("timeout".into());
        display_error_styled(&err, true, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_styled_tty_api_auth() {
        let err = ScopeError::Api("401 Unauthorized".into());
        display_error_styled(&err, true, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_styled_tty_other_no_hint() {
        let err = ScopeError::Other("random".into());
        display_error_styled(&err, true, &quiet()).unwrap();
    }

    #[test]
    fn test_display_error_styled_non_tty() {
        let err = ScopeError::NotFound("test".into());
        display_error_styled(&err, false, &quiet()).unwrap();
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
