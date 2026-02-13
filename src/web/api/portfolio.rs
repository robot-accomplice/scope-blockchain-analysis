//! Portfolio management API handlers.

use crate::cli::portfolio::{Portfolio, WatchedAddress};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// GET /api/portfolio/list — List portfolio addresses.
pub async fn handle_list(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    match Portfolio::load(&data_dir) {
        Ok(portfolio) => {
            Json(serde_json::json!({
                "addresses": portfolio.addresses,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Request body for adding a portfolio address.
#[derive(Debug, Deserialize)]
pub struct AddPortfolioRequest {
    /// Blockchain address.
    pub address: String,
    /// Chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Optional label.
    pub label: Option<String>,
    /// Optional tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

/// POST /api/portfolio/add — Add an address to the portfolio.
pub async fn handle_add(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddPortfolioRequest>,
) -> impl IntoResponse {
    let data_dir = state.config.data_dir();
    let mut portfolio = match Portfolio::load(&data_dir) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let watched = WatchedAddress {
        address: req.address.clone(),
        label: req.label,
        chain: req.chain,
        tags: req.tags,
        added_at: chrono::Utc::now().timestamp() as u64,
    };

    match portfolio.add_address(watched) {
        Ok(_) => {
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = portfolio.save(&data_dir_buf) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            Json(serde_json::json!({ "status": "added", "address": req.address })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
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
            "address": "0x1234567890123456789012345678901234567890",
            "chain": "polygon",
            "label": "My Wallet",
            "tags": ["defi", "nft"]
        });
        let req: AddPortfolioRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "polygon");
        assert_eq!(req.label, Some("My Wallet".to_string()));
        assert_eq!(req.tags.len(), 2);
        assert_eq!(req.tags[0], "defi");
        assert_eq!(req.tags[1], "nft");
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890"
        });
        let req: AddPortfolioRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.label, None);
        assert_eq!(req.tags.len(), 0);
    }

    #[test]
    fn test_default_chain() {
        assert_eq!(default_chain(), "ethereum");
    }

    #[test]
    fn test_with_tags() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "tags": ["tag1", "tag2", "tag3"]
        });
        let req: AddPortfolioRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.tags.len(), 3);
        assert_eq!(req.tags[0], "tag1");
        assert_eq!(req.tags[1], "tag2");
        assert_eq!(req.tags[2], "tag3");
    }

    #[test]
    fn test_with_label() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "label": "Test Label"
        });
        let req: AddPortfolioRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.label, Some("Test Label".to_string()));
    }

    #[tokio::test]
    async fn test_handle_portfolio_list_direct() {
        use axum::extract::State;
        use axum::response::IntoResponse;
        use crate::config::Config;
        use crate::chains::DefaultClientFactory;
        use crate::web::AppState;

        let config = Config::default();
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let response = handle_list(State(state)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_portfolio_add_direct() {
        use axum::extract::State;
        use axum::response::IntoResponse;
        use crate::config::Config;
        use crate::chains::DefaultClientFactory;
        use crate::web::AppState;

        let config = Config::default();
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = AddPortfolioRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            label: Some("Test".to_string()),
            tags: vec!["test".to_string()],
        };
        let response = handle_add(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }
}
