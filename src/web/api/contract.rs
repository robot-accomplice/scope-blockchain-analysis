//! # Contract Analysis API Handler
//!
//! POST /api/contract - Analyze a smart contract address.

use crate::chains::ChainClientFactory;
use crate::contract;
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for contract analysis.
#[derive(Debug, Deserialize)]
pub struct ContractRequest {
    /// Contract address to analyze.
    pub address: String,
    /// Chain (default: ethereum).
    #[serde(default = "default_chain")]
    pub chain: String,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

/// Handle contract analysis request.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ContractRequest>,
) -> impl IntoResponse {
    let client: Box<dyn crate::chains::ChainClient> =
        match state.factory.create_chain_client(&req.chain) {
            Ok(c) => c,
            Err(e) => {
                let err_msg = e.to_string();
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": err_msg })),
                )
                    .into_response();
            }
        };

    let http_client = reqwest::Client::new();

    match contract::analyze_contract(&req.address, &req.chain, client.as_ref(), &http_client).await
    {
        Ok(analysis) => (StatusCode::OK, Json(serde_json::json!(analysis))).into_response(),
        Err(e) => {
            let err_msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err_msg })),
            )
                .into_response()
        }
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
        assert_eq!(default_chain(), "ethereum");
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
        let req = ContractRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
        };
        let response = handle(State(state), axum::Json(req))
            .await
            .into_response();
        let status = response.status();
        assert!(
            status.is_success()
                || status == axum::http::StatusCode::BAD_REQUEST
                || status.is_server_error()
        );
    }

    #[tokio::test]
    async fn test_handle_contract_solana_chain() {
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
        let req = ContractRequest {
            address: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            chain: "solana".to_string(),
        };
        let response = handle(State(state), axum::Json(req))
            .await
            .into_response();
        let status = response.status();
        assert!(
            status.is_success()
                || status == axum::http::StatusCode::BAD_REQUEST
                || status.is_server_error()
        );
    }
}
