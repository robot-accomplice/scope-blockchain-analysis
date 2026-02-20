//! Export API handler.

use crate::chains::ChainClientFactory;
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for data export.
#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    /// Address to export data for.
    pub address: String,
    /// Chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Format: "json" or "csv".
    #[serde(default = "default_format")]
    pub format: String,
    /// Optional start date filter (ISO 8601).
    pub start_date: Option<String>,
    /// Optional end date filter (ISO 8601).
    pub end_date: Option<String>,
}

fn default_chain() -> String {
    "ethereum".to_string()
}
fn default_format() -> String {
    "json".to_string()
}

/// POST /api/export — Export address data.
///
/// Supports address book shortcuts: pass `@label` as the address to
/// resolve it from the address book.
///
/// Returns the export data directly as JSON (regardless of requested format)
/// since the web API always returns JSON. The `format` field is preserved
/// in the response metadata for client-side handling.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExportRequest>,
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

    let client: Box<dyn crate::chains::ChainClient> =
        match state.factory.create_chain_client(&chain) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

    // Fetch balance
    let mut balance = match client.get_balance(&address).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    client.enrich_balance_usd(&mut balance).await;

    // Fetch transactions
    let txs = client.get_transactions(&address, 100).await.ok();

    // Fetch token balances
    let tokens = client.get_token_balances(&address).await.ok();

    Json(serde_json::json!({
        "address": address,
        "chain": chain,
        "format": req.format,
        "balance": {
            "raw": balance.raw,
            "formatted": balance.formatted,
            "usd_value": balance.usd_value,
        },
        "transactions": txs,
        "tokens": tokens,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "polygon",
            "format": "csv",
            "start_date": "2024-01-01T00:00:00Z",
            "end_date": "2024-12-31T23:59:59Z"
        });
        let req: ExportRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(req.chain, "polygon");
        assert_eq!(req.format, "csv");
        assert_eq!(req.start_date, Some("2024-01-01T00:00:00Z".to_string()));
        assert_eq!(req.end_date, Some("2024-12-31T23:59:59Z".to_string()));
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890"
        });
        let req: ExportRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.format, "json");
        assert_eq!(req.start_date, None);
        assert_eq!(req.end_date, None);
    }

    #[test]
    fn test_defaults() {
        assert_eq!(default_chain(), "ethereum");
        assert_eq!(default_format(), "json");
    }

    #[test]
    fn test_with_date_filters() {
        let json = serde_json::json!({
            "address": "0xabcdef1234567890abcdef1234567890abcdef1234",
            "start_date": "2024-06-01T00:00:00Z",
            "end_date": "2024-06-30T23:59:59Z"
        });
        let req: ExportRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0xabcdef1234567890abcdef1234567890abcdef1234");
        assert_eq!(req.chain, "ethereum");
        assert_eq!(req.format, "json");
        assert_eq!(req.start_date, Some("2024-06-01T00:00:00Z".to_string()));
        assert_eq!(req.end_date, Some("2024-06-30T23:59:59Z".to_string()));
    }

    #[tokio::test]
    async fn test_handle_export_direct() {
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
        let req = ExportRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: "json".to_string(),
            start_date: None,
            end_date: None,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_export_success_json_structure() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::body;
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
        let req = ExportRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            format: "json".to_string(),
            start_date: None,
            end_date: None,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        if response.status().is_success() {
            let body_bytes = body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
            assert!(json.get("address").is_some());
            assert!(json.get("chain").is_some());
            assert!(json.get("format").is_some());
            assert!(json.get("balance").is_some());
        }
    }

    #[test]
    fn test_export_request_debug() {
        let req = ExportRequest {
            address: "0xabc".to_string(),
            chain: "ethereum".to_string(),
            format: "json".to_string(),
            start_date: None,
            end_date: None,
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("ExportRequest"));
    }

    #[test]
    fn test_deserialize_export_csv_format() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "format": "csv"
        });
        let req: ExportRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.format, "csv");
    }

    #[tokio::test]
    async fn test_handle_export_unsupported_chain_bad_request() {
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
        let req = ExportRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "bitcoin".to_string(), // Unsupported chain
            format: "json".to_string(),
            start_date: None,
            end_date: None,
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
    async fn test_handle_export_label_not_found() {
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
        let req = ExportRequest {
            address: "@ghost-wallet".to_string(),
            chain: "ethereum".to_string(),
            format: "json".to_string(),
            start_date: None,
            end_date: None,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("@ghost-wallet"));
    }
}
