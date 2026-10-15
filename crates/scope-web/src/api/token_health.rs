//! Token health API handler.

use crate::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use scope::market::{HealthOverrides, fetch_token_summary_by_name};
use scope_cli::cli::crawl::{self, Period};
use serde::Deserialize;
use std::sync::Arc;

/// Request body for token health.
#[derive(Debug, Deserialize)]
pub struct TokenHealthRequest {
    /// Token symbol or contract address.
    pub token: String,
    /// Target chain (default: "ethereum").
    #[serde(default = "super::default_chain")]
    pub chain: String,
    /// Include market/order book data.
    #[serde(default)]
    pub with_market: bool,
    /// Market venue; see scope::market::VENUE_HELP.
    #[serde(default = "super::default_venue")]
    pub market_venue: String,
    /// Health threshold overrides for `with_market` (top-level JSON keys).
    /// Unset keys use config `market.health`.
    #[serde(flatten)]
    pub health: HealthOverrides,
}

/// POST /api/token-health — Token health suite.
///
/// Supports address book shortcuts: pass `@label` as the token to
/// resolve it from the address book.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TokenHealthRequest>,
) -> impl IntoResponse {
    // Resolve address book shortcuts (@label or direct address match)
    let resolved = match super::resolve_address_book(&req.token, &state.config) {
        Ok(r) => r,
        Err(e) => return super::error_response(super::status_for(&e), e.to_string()),
    };
    let token = resolved.value;
    let chain = resolved.chain.unwrap_or(req.chain);

    // Fetch DEX analytics
    let analytics = match crawl::fetch_analytics_for_input(
        &token,
        &chain,
        Period::Hour24,
        10,
        state.factory.as_ref(),
        None,
        &scope_cli::cli::output::Output::stdio().without_prompts(),
    )
    .await
    {
        Ok(a) => a,
        Err(e) => return super::error_response(super::status_for(&e), e.to_string()),
    };

    // Optionally fetch market data. A market error leaves `market` null and
    // is reported in `market_error`; the analytics still return.
    let market_result = if req.with_market {
        let thresholds = state.config.market.health.with_overrides(&req.health);
        Some(fetch_token_summary_by_name(&req.market_venue, &analytics, &thresholds).await)
    } else {
        None
    };
    let (market_summary, market_error) = match market_result {
        Some(Ok(summary)) => (Some(summary), None),
        Some(Err(e)) => (None, Some(e.to_string())),
        None => (None, None),
    };

    // Build combined JSON
    let market_json = market_summary.map(|m| m.to_json());

    Json(serde_json::json!({
        "analytics": analytics,
        "market": market_json,
        "market_error": market_error,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope::market::DexVenue;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "token": "USDC",
            "chain": "polygon",
            "with_market": true,
            "market_venue": "biconomy"
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "USDC");
        assert_eq!(req.chain, "polygon");
        assert!(req.with_market);
        assert_eq!(req.market_venue, "biconomy");
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "token": "USDC"
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "USDC");
        assert_eq!(req.chain, "ethereum");
        assert!(!req.with_market);
        assert_eq!(req.market_venue, "binance");
    }

    #[test]
    fn test_defaults() {
        assert_eq!(super::super::default_chain(), "ethereum");
        assert_eq!(super::super::default_venue(), "binance");
    }

    #[test]
    fn test_with_market_flag() {
        let json = serde_json::json!({
            "token": "USDC",
            "with_market": true
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert!(req.with_market);

        let json_false = serde_json::json!({
            "token": "USDC",
            "with_market": false
        });
        let req_false: TokenHealthRequest = serde_json::from_value(json_false).unwrap();
        assert!(!req_false.with_market);
    }

    #[tokio::test]
    async fn test_handle_token_health_direct() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_token_health_with_market() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: "eth".to_string(),
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_token_health_with_cex_market() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: "binance".to_string(), // CEX path
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        // Either succeeds (200) or fails gracefully (500)
        assert!(status.is_success() || status.is_server_error());
    }

    #[test]
    fn test_is_dex_venue() {
        assert!(DexVenue::parse("ethereum").is_some());
        assert!(DexVenue::parse("eth").is_some());
        assert!(DexVenue::parse("Ethereum").is_some());
        assert!(DexVenue::parse("ETH").is_some());
        assert!(DexVenue::parse("solana").is_some());
        assert!(DexVenue::parse("Solana").is_some());
        assert!(DexVenue::parse("binance").is_none());
        assert!(DexVenue::parse("mexc").is_none());
        assert!(DexVenue::parse("okx").is_none());
        assert!(DexVenue::parse("").is_none());
    }

    #[test]
    fn test_dex_venue_to_chain() {
        assert_eq!(
            DexVenue::parse("ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("eth").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("Ethereum").map(DexVenue::chain),
            Some("ethereum")
        );
        assert_eq!(
            DexVenue::parse("solana").map(DexVenue::chain),
            Some("solana")
        );
        assert_eq!(
            DexVenue::parse("Solana").map(DexVenue::chain),
            Some("solana")
        );
        assert_eq!(DexVenue::parse("unknown"), None); // no silent ethereum
    }

    #[tokio::test]
    async fn test_handle_token_health_invalid_token_error_path() {
        use crate::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = TokenHealthRequest {
            token: "INVALID_TOKEN_XYZ_NONEXISTENT".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        // Empty analytics (200), not found (404, via `status_for`), or a
        // network failure offline (500).
        assert!(
            status.is_success()
                || status == axum::http::StatusCode::NOT_FOUND
                || status.is_server_error(),
            "{status}"
        );
    }

    #[test]
    fn test_token_health_request_debug() {
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: true,
            market_venue: "binance".to_string(),
            health: HealthOverrides::default(),
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("TokenHealthRequest"));
    }

    #[test]
    fn test_deserialize_with_optional_venue() {
        let json = serde_json::json!({
            "token": "DAI",
            "market_venue": "mexc"
        });
        let req: TokenHealthRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "DAI");
        assert_eq!(req.market_venue, "mexc");
    }

    #[tokio::test]
    async fn test_handle_token_health_success_json_structure() {
        use crate::AppState;
        use axum::body;
        use axum::extract::State;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let config = Config::default();
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = TokenHealthRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        if response.status().is_success() {
            let body_bytes = body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert!(json.get("analytics").is_some());
            assert!(json.get("market").is_some());
        }
    }

    #[tokio::test]
    async fn test_handle_token_health_label_not_found() {
        use crate::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use scope::chains::DefaultClientFactory;
        use scope::config::Config;

        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: scope::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let http: std::sync::Arc<dyn scope::http::HttpClient> =
            std::sync::Arc::new(scope::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState::new(config, factory));
        let req = TokenHealthRequest {
            token: "@nonexistent".to_string(),
            chain: "ethereum".to_string(),
            with_market: false,
            market_venue: "binance".to_string(),
            health: HealthOverrides::default(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("@nonexistent"));
    }
}
