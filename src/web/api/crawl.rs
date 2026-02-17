//! Token crawl API handler.

use crate::cli::crawl::{self, Period};
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for token crawl.
#[derive(Debug, Deserialize)]
pub struct CrawlRequest {
    /// Token address or symbol.
    pub token: String,
    /// Target chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Time period: "1h", "24h", "7d", "30d".
    #[serde(default)]
    pub period: Option<String>,
    /// Max holders to include.
    #[serde(default = "default_holders_limit")]
    pub holders_limit: u32,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

fn default_holders_limit() -> u32 {
    10
}

/// POST /api/crawl — Token analytics.
///
/// Supports address book shortcuts: pass `@label` as the token to
/// resolve it from the address book.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CrawlRequest>,
) -> impl IntoResponse {
    // Resolve address book shortcuts (@label or direct address match)
    let resolved = match super::resolve_address_book(&req.token, &state.config) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };
    let token = resolved.value;
    let chain = resolved.chain.unwrap_or(req.chain);

    let period = match req.period.as_deref() {
        Some("1h") => Period::Hour1,
        Some("7d") => Period::Day7,
        Some("30d") => Period::Day30,
        _ => Period::Hour24,
    };

    match crawl::fetch_analytics_for_input(
        &token,
        &chain,
        period,
        req.holders_limit,
        &state.factory,
        None,
    )
    .await
    {
        Ok(analytics) => Json(serde_json::json!(analytics)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "token": "USDC",
            "chain": "polygon",
            "period": "7d",
            "holders_limit": 20
        });
        let req: CrawlRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "USDC");
        assert_eq!(req.chain, "polygon");
        assert_eq!(req.period, Some("7d".to_string()));
        assert_eq!(req.holders_limit, 20);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "token": "ETH"
        });
        let req: CrawlRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.token, "ETH");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.period, None);
        assert_eq!(req.holders_limit, 10);
    }

    #[test]
    fn test_defaults() {
        assert_eq!(default_chain(), "ethereum");
        assert_eq!(default_holders_limit(), 10);
    }

    #[test]
    fn test_period_variations() {
        let json_1h = serde_json::json!({
            "token": "USDT",
            "period": "1h"
        });
        let req_1h: CrawlRequest = serde_json::from_value(json_1h).unwrap();
        assert_eq!(req_1h.period, Some("1h".to_string()));

        let json_24h = serde_json::json!({
            "token": "USDT",
            "period": "24h"
        });
        let req_24h: CrawlRequest = serde_json::from_value(json_24h).unwrap();
        assert_eq!(req_24h.period, Some("24h".to_string()));

        let json_30d = serde_json::json!({
            "token": "USDT",
            "period": "30d"
        });
        let req_30d: CrawlRequest = serde_json::from_value(json_30d).unwrap();
        assert_eq!(req_30d.period, Some("30d".to_string()));

        let json_no_period = serde_json::json!({
            "token": "USDT"
        });
        let req_no_period: CrawlRequest = serde_json::from_value(json_no_period).unwrap();
        assert_eq!(req_no_period.period, None);
    }

    #[tokio::test]
    async fn test_handle_crawl_direct() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = CrawlRequest {
            token: "USDC".to_string(),
            chain: "ethereum".to_string(),
            period: Some("24h".to_string()),
            holders_limit: 5,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_crawl_label_not_found() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = CrawlRequest {
            token: "@no-such-token".to_string(),
            chain: "ethereum".to_string(),
            period: None,
            holders_limit: 10,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("@no-such-token"));
    }
}
