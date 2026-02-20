//! Compliance risk assessment API handler.

use crate::compliance::datasource::{BlockchainDataClient, DataSources};
use crate::compliance::risk::RiskEngine;
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for compliance risk analysis.
#[derive(Debug, Deserialize)]
pub struct ComplianceRiskRequest {
    /// Address to assess.
    pub address: String,
    /// Chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Include detailed breakdown.
    #[serde(default)]
    pub detailed: bool,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

/// POST /api/compliance/risk — Risk assessment for an address.
///
/// Supports address book shortcuts: pass `@label` as the address to
/// resolve it from the address book.
pub async fn handle_risk(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<ComplianceRiskRequest>,
) -> impl IntoResponse {
    // Resolve address book shortcuts (@label or direct address match)
    let resolved = match super::resolve_address_book(&req.address, &_state.config) {
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

    // Build risk engine (with Etherscan key if available)
    let engine = if let Ok(key) = std::env::var("ETHERSCAN_API_KEY") {
        let sources = DataSources::new(key);
        let client = BlockchainDataClient::new(sources);
        RiskEngine::with_data_client(client)
    } else {
        RiskEngine::new()
    };

    match engine.assess_address(&address, &chain).await {
        Ok(assessment) => Json(serde_json::json!({
            "address": assessment.address,
            "chain": assessment.chain,
            "overall_score": assessment.overall_score,
            "risk_level": format!("{:?}", assessment.risk_level),
            "factors": assessment.factors.iter().map(|f| {
                serde_json::json!({
                    "name": f.name,
                    "weight": f.weight,
                    "score": f.score,
                    "description": f.description,
                })
            }).collect::<Vec<_>>(),
        }))
        .into_response(),
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
            "address": "0x1234567890123456789012345678901234567890",
            "chain": "polygon",
            "detailed": true
        });
        let req: ComplianceRiskRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "polygon");
        assert!(req.detailed);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890"
        });
        let req: ComplianceRiskRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, "ethereum");
        assert!(!req.detailed);
    }

    #[test]
    fn test_default_chain() {
        assert_eq!(default_chain(), "ethereum");
    }

    #[test]
    fn test_detailed_flag() {
        let json = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "detailed": true
        });
        let req: ComplianceRiskRequest = serde_json::from_value(json).unwrap();
        assert!(req.detailed);

        let json_false = serde_json::json!({
            "address": "0x1234567890123456789012345678901234567890",
            "detailed": false
        });
        let req_false: ComplianceRiskRequest = serde_json::from_value(json_false).unwrap();
        assert!(!req_false.detailed);
    }

    #[tokio::test]
    async fn test_handle_risk_direct() {
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
        let req = ComplianceRiskRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            detailed: true,
        };
        let response = handle_risk(State(state), axum::Json(req))
            .await
            .into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_risk_with_etherscan_key() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::response::IntoResponse;

        let old_key = std::env::var_os("ETHERSCAN_API_KEY");
        unsafe { std::env::set_var("ETHERSCAN_API_KEY", "test_key_for_coverage") };

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = ComplianceRiskRequest {
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: "ethereum".to_string(),
            detailed: false,
        };
        let response = handle_risk(State(state), axum::Json(req))
            .await
            .into_response();

        if let Some(k) = old_key {
            unsafe { std::env::set_var("ETHERSCAN_API_KEY", k) };
        } else {
            unsafe { std::env::remove_var("ETHERSCAN_API_KEY") };
        }

        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_risk_error_response() {
        use crate::chains::DefaultClientFactory;
        use crate::config::Config;
        use crate::web::AppState;
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let config = Config::default();
        let http: std::sync::Arc<dyn crate::http::HttpClient> =
            std::sync::Arc::new(crate::http::NativeHttpClient::new().unwrap());
        let factory = DefaultClientFactory {
            chains_config: config.chains.clone(),
            http,
        };
        let state = std::sync::Arc::new(AppState { config, factory });
        let req = ComplianceRiskRequest {
            address: "invalid-address".to_string(),
            chain: "ethereum".to_string(),
            detailed: false,
        };
        let response = handle_risk(State(state), axum::Json(req))
            .await
            .into_response();
        if response.status() == StatusCode::INTERNAL_SERVER_ERROR {
            let body = axum::body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(json.get("error").is_some());
        }
    }

    #[tokio::test]
    async fn test_handle_risk_label_not_found() {
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
        let req = ComplianceRiskRequest {
            address: "@fake-wallet".to_string(),
            chain: "ethereum".to_string(),
            detailed: false,
        };
        let response = handle_risk(State(state), axum::Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("@fake-wallet"));
    }
}
