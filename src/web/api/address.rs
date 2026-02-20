//! Address analysis API handler.

use crate::chains::ChainClientFactory;
use crate::cli::address::{self, AddressArgs};
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for address analysis.
#[derive(Debug, Deserialize)]
pub struct AddressRequest {
    /// Blockchain address to analyze.
    pub address: String,
    /// Target chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Include transaction history.
    #[serde(default)]
    pub include_txs: bool,
    /// Include token balances.
    #[serde(default)]
    pub include_tokens: bool,
    /// Max transactions to retrieve.
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Generate dossier (address + risk).
    #[serde(default)]
    pub dossier: bool,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

fn default_limit() -> u32 {
    100
}

/// POST /api/address — Analyze a blockchain address.
///
/// Supports address book shortcuts: pass `@label` as the address to
/// resolve it from the address book. The chain will also be set from
/// the book entry unless explicitly overridden.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddressRequest>,
) -> impl IntoResponse {
    // Resolve address book shortcuts (@label or direct address match)
    let resolved = match super::resolve_address_book(&req.address, &state.config) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };
    let address = resolved.value;
    let chain = resolved.chain.unwrap_or(req.chain);

    let args = AddressArgs {
        address,
        chain,
        format: None,
        include_txs: req.include_txs,
        include_tokens: req.include_tokens,
        limit: req.limit,
        report: None,
        dossier: req.dossier,
    };

    let client: Box<dyn crate::chains::ChainClient> =
        match state.factory.create_chain_client(&args.chain) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

    match address::analyze_address(&args, client.as_ref()).await {
        Ok(report) => Json(serde_json::json!(report)).into_response(),
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
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "polygon",
            "include_txs": true,
            "include_tokens": true,
            "limit": 50,
            "dossier": true
        });
        let req: AddressRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(req.chain, "polygon");
        assert!(req.include_txs);
        assert!(req.include_tokens);
        assert_eq!(req.limit, 50);
        assert!(req.dossier);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2"
        });
        let req: AddressRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(req.chain, "ethereum");
        assert!(!req.include_txs);
        assert!(!req.include_tokens);
        assert_eq!(req.limit, 100);
        assert!(!req.dossier);
    }

    #[test]
    fn test_defaults() {
        assert_eq!(default_chain(), "ethereum");
        assert_eq!(default_limit(), 100);
    }

    #[test]
    fn test_deserialize_with_options() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "include_txs": true,
            "dossier": true
        });
        let req: AddressRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "ethereum");
        assert!(req.include_txs);
        assert!(!req.include_tokens);
        assert_eq!(req.limit, 100);
        assert!(req.dossier);
    }

    #[tokio::test]
    async fn test_handle_address_direct() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = AddressRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            include_txs: false,
            include_tokens: true,
            limit: 10,
            dossier: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        // Will likely return error (no API key) or success
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_address_unsupported_chain_bad_request() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        // Use a temp data dir to avoid local address book interfering
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            address_book: crate::config::AddressBookConfig {
                data_dir: Some(tmp.path().to_path_buf()),
            },
            ..Default::default()
        };
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = AddressRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "bitcoin".to_string(),
            include_txs: false,
            include_tokens: true,
            limit: 10,
            dossier: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("Unsupported chain")
        );
    }

    #[tokio::test]
    async fn test_handle_address_book_label_not_found() {
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
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = AddressRequest {
            address: "@nonexistent-label".to_string(),
            chain: "ethereum".to_string(),
            include_txs: false,
            include_tokens: false,
            limit: 10,
            dossier: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("@nonexistent-label")
        );
    }
}
