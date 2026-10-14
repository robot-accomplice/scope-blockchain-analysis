//! # Web API Handlers
//!
//! REST API endpoints mirroring CLI commands. Each handler accepts JSON
//! request bodies matching CLI argument structures and returns JSON responses.

pub mod address;
pub mod address_book;
pub mod compliance;
pub mod config_status;
pub mod contract;
pub mod crawl;
pub mod discover;
pub mod exchange;
pub mod export;
pub mod insights;
pub mod market;
pub mod token_health;
pub mod tx;
pub mod venues;

use crate::AppState;
use axum::Router;
use std::sync::Arc;

/// The one map from a core error kind to an HTTP status: a missing resource
/// is 404, a request fault (unknown venue, chain mismatch, bad address) is
/// 400, and anything else is 500.
pub(crate) fn status_for(err: &scope::error::ScopeError) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    use scope::error::ScopeError;
    match err {
        ScopeError::NotFound(_) => StatusCode::NOT_FOUND,
        ScopeError::InvalidInput(_)
        | ScopeError::InvalidAddress(_)
        | ScopeError::InvalidHash(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// A JSON error body, `{ "error": message }`, with `status`.
pub(crate) fn error_response(
    status: axum::http::StatusCode,
    message: impl Into<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = serde_json::json!({ "error": message.into() });
    (status, axum::Json(body)).into_response()
}

/// Serde default for a request's chain field.
pub(crate) fn default_chain() -> String {
    scope::chains::DEFAULT_CHAIN.to_string()
}

/// Serde default for a request's market venue field.
pub(crate) fn default_venue() -> String {
    scope::market::DEFAULT_VENUE.to_string()
}

/// Serde default for a market request's pair field.
pub(crate) fn default_pair() -> String {
    scope::market::DEFAULT_PAIR.to_string()
}

/// Result of resolving a user input through the address book.
///
/// When the input matches an address book entry (either via `@label`
/// prefix or direct address match), both the resolved address and chain
/// are returned. Handlers should use the resolved chain when the user
/// hasn't explicitly overridden it.
#[derive(Debug)]
pub struct ResolvedInput {
    /// The resolved address/token string.
    pub value: String,
    /// The chain from the address book entry (if resolved).
    pub chain: Option<String>,
}

/// Resolves a user-supplied input string against the address book.
///
/// Supports `@label` shortcuts (e.g. `@main-wallet`) and direct address
/// matching. Returns the original input unchanged if no match is found.
///
/// # Errors
///
/// [`scope::error::ScopeError::InvalidInput`] when `@label` names no address
/// book entry — a request fault, so handlers answer
/// `error_response(status_for(&e), ...)` with 400.
pub fn resolve_address_book(
    input: &str,
    config: &scope::config::Config,
) -> scope::error::Result<ResolvedInput> {
    match scope_cli::cli::address_book::resolve_address_book_input(input, config) {
        Ok(Some((address, chain))) => Ok(ResolvedInput {
            value: address,
            chain: Some(chain),
        }),
        Ok(None) => Ok(ResolvedInput {
            value: input.to_string(),
            chain: None,
        }),
        // A misspelled @label is a bad request, not a missing resource.
        Err(scope::error::ScopeError::NotFound(msg)) => {
            Err(scope::error::ScopeError::InvalidInput(msg))
        }
        Err(e) => Err(scope::error::ScopeError::InvalidInput(e.to_string())),
    }
}

/// Registers all API routes under the `/api` prefix.
pub fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/address", axum::routing::post(address::handle))
        .route("/tx", axum::routing::post(tx::handle))
        .route("/insights", axum::routing::post(insights::handle))
        .route("/crawl", axum::routing::post(crawl::handle))
        .route("/discover", axum::routing::get(discover::handle))
        .route("/token-health", axum::routing::post(token_health::handle))
        .route("/market/summary", axum::routing::post(market::handle))
        .route(
            "/address-book/list",
            axum::routing::get(address_book::handle_list),
        )
        .route(
            "/address-book/add",
            axum::routing::post(address_book::handle_add),
        )
        .route(
            "/address-book/remove",
            axum::routing::post(address_book::handle_remove),
        )
        .route("/contract", axum::routing::post(contract::handle))
        .route("/export", axum::routing::post(export::handle))
        .route(
            "/compliance/risk",
            axum::routing::post(compliance::handle_risk),
        )
        .route("/config/status", axum::routing::get(config_status::handle))
        .route("/config", axum::routing::post(config_status::handle_save))
        .route("/venues", axum::routing::get(venues::handle))
        .route("/exchange/snapshot", axum::routing::post(exchange::handle))
        .route(
            "/exchange/trades",
            axum::routing::post(exchange::handle_trades),
        )
        .route("/exchange/ohlc", axum::routing::post(exchange::handle_ohlc))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::chains::DefaultClientFactory;
    use scope::config::Config;
    use scope_cli::cli::address_book::{AddressBook, WatchedAddress};

    #[test]
    fn test_routes_construction() {
        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = Arc::new(AppState::new(config, factory));
        let _router = routes(state);
        // If this doesn't panic, routes are properly constructed
    }

    #[test]
    fn test_resolve_address_book_label_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ab = AddressBook::default();
        ab.add_address(WatchedAddress {
            address: "0xABCDEF1234567890abcdef1234567890ABCDEF12".to_string(),
            label: Some("hot-wallet".to_string()),
            chain: "polygon".to_string(),
            tags: vec![],
            added_at: 0,
        })
        .unwrap();
        ab.save(&tmp.path().to_path_buf()).unwrap();

        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let result = resolve_address_book("@hot-wallet", &config);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.value, "0xABCDEF1234567890abcdef1234567890ABCDEF12");
        assert_eq!(r.chain, Some("polygon".to_string()));
    }

    #[test]
    fn test_resolve_address_book_label_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let result = resolve_address_book("@nonexistent", &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, scope::error::ScopeError::InvalidInput(_)));
        assert!(err.to_string().contains("@nonexistent"));
    }

    #[test]
    fn test_resolve_address_book_raw_address_passthrough() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let result = resolve_address_book("0xSomeRawAddress", &config);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.value, "0xSomeRawAddress");
        assert_eq!(r.chain, None);
    }

    #[test]
    fn test_resolve_address_book_address_match_returns_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ab = AddressBook::default();
        ab.add_address(WatchedAddress {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            label: Some("main".to_string()),
            chain: "arbitrum".to_string(),
            tags: vec![],
            added_at: 0,
        })
        .unwrap();
        ab.save(&tmp.path().to_path_buf()).unwrap();

        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        // Pass raw address (no @ prefix) — should still match and return chain
        let result = resolve_address_book("0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2", &config);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.value, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(r.chain, Some("arbitrum".to_string()));
    }

    #[test]
    fn test_resolve_address_book_token_symbol_passthrough() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        // Token symbols like "USDC" should pass through unchanged
        let result = resolve_address_book("USDC", &config);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.value, "USDC");
        assert_eq!(r.chain, None);
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use axum::http::StatusCode;
    use scope::error::ScopeError;

    /// One map from core error kinds to HTTP status: a missing resource
    /// (e.g. no DEX pairs for the token) is 404, bad input is 400, and
    /// anything else is 500.
    #[test]
    fn core_error_kinds_map_to_one_status_each() {
        let no_pairs = ScopeError::NotFound("No DEX pairs found for USDC on solana".into());
        assert_eq!(status_for(&no_pairs), StatusCode::NOT_FOUND);
        assert_eq!(
            status_for(&ScopeError::InvalidAddress("0x1".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_for(&ScopeError::Network("timeout".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
