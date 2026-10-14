//! # Contract Analysis API Handler
//!
//! POST /api/contract - Analyze a smart contract address.

use crate::AppState;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use scope::contract;
use serde::Deserialize;
use std::sync::Arc;

use super::{error_response, status_for};

/// Request body for contract analysis.
#[derive(Debug, Deserialize)]
pub struct ContractRequest {
    /// Contract address to analyze.
    pub address: String,
    /// Chain (default: ethereum).
    #[serde(default = "super::default_chain")]
    pub chain: String,
}

/// Handle contract analysis request.
///
/// Supports address book shortcuts: pass `@label` as the address to
/// resolve it from the address book.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ContractRequest>,
) -> impl IntoResponse {
    // Resolve address book shortcuts (@label or direct address match)
    let resolved = match super::resolve_address_book(&req.address, &state.config) {
        Ok(r) => r,
        Err(e) => return error_response(status_for(&e), e.to_string()),
    };
    let address = resolved.value;
    let chain = resolved.chain.unwrap_or(req.chain);

    let client: Box<dyn scope::chains::ChainClient> =
        match state.factory.create_chain_client(&chain) {
            Ok(c) => c,
            Err(e) => return error_response(status_for(&e), e.to_string()),
        };

    let http_client = scope::http::transport();

    match contract::analyze_contract(&address, &chain, client.as_ref(), http_client.as_ref()).await
    {
        Ok(analysis) => Json(serde_json::json!(analysis)).into_response(),
        Err(e) => error_response(status_for(&e), e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_deserialize_full() {
        let json = serde_json::json!({
            "address": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "polygon"
        });
        let req: ContractRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(req.chain, "polygon");
    }

    #[test]
    fn test_contract_deserialize_minimal() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890"
        });
        let req: ContractRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "ethereum");
    }

    #[test]
    fn test_contract_default_chain() {
        assert_eq!(super::super::default_chain(), "ethereum");
    }

    #[test]
    fn test_contract_request_debug() {
        let req = ContractRequest {
            address: "0xabc".to_string(),
            chain: "ethereum".to_string(),
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("ContractRequest"));
    }

    #[tokio::test]
    async fn test_handle_contract_direct() {
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
        let req = ContractRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(
            status.is_success()
                || status == axum::http::StatusCode::BAD_REQUEST
                || status.is_server_error()
        );
    }

    #[tokio::test]
    async fn test_handle_contract_solana_chain() {
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
        let req = ContractRequest {
            address: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            chain: "solana".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(
            status.is_success()
                || status == axum::http::StatusCode::BAD_REQUEST
                || status.is_server_error()
        );
    }

    #[tokio::test]
    async fn test_handle_contract_label_not_found() {
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
        let req = ContractRequest {
            address: "@missing-label".to_string(),
            chain: "ethereum".to_string(),
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("@missing-label"));
    }
}
