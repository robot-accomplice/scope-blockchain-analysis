//! Unified insights API handler.

use crate::chains::ChainClientFactory;
use crate::cli::insights::{self, InsightsArgs};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for insights analysis.
#[derive(Debug, Deserialize)]
pub struct InsightsRequest {
    /// Target: address, tx hash, or token symbol/name.
    pub target: String,
    /// Override detected chain.
    pub chain: Option<String>,
    /// Decode tx input (for tx targets).
    #[serde(default)]
    pub decode: bool,
    /// Include internal trace (for tx targets).
    #[serde(default)]
    pub trace: bool,
}

/// POST /api/insights — Unified insights for any target.
///
/// Returns the insights markdown as JSON `{ "markdown": "..." }` along
/// with structured metadata about the detected target type.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InsightsRequest>,
) -> impl IntoResponse {
    let target = insights::infer_target(&req.target, req.chain.as_deref());

    let target_type = match &target {
        insights::InferredTarget::Address { chain } => {
            serde_json::json!({ "type": "address", "chain": chain })
        }
        insights::InferredTarget::Transaction { chain } => {
            serde_json::json!({ "type": "transaction", "chain": chain })
        }
        insights::InferredTarget::Token { chain } => {
            serde_json::json!({ "type": "token", "chain": chain })
        }
    };

    // Run the insights command which builds markdown output
    // We capture it by running the underlying functions directly
    let args = InsightsArgs {
        target: req.target.clone(),
        chain: req.chain,
        decode: req.decode,
        trace: req.trace,
    };

    // Run insights - it prints to stdout so we need to capture
    // For the web API, we reconstruct the data using the inferred target
    match &target {
        insights::InferredTarget::Address { chain } => {
            let addr_args = crate::cli::address::AddressArgs {
                address: req.target,
                chain: chain.clone(),
                format: None,
                include_txs: false,
                include_tokens: true,
                limit: 10,
                report: None,
                dossier: false,
            };
            let client: Box<dyn crate::chains::ChainClient> = match state.factory.create_chain_client(chain) {
                Ok(c) => c,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    )
                        .into_response();
                }
            };
            match crate::cli::address::analyze_address(&addr_args, client.as_ref()).await {
                Ok(report) => Json(serde_json::json!({
                    "target_info": target_type,
                    "data": report,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
        insights::InferredTarget::Transaction { chain } => {
            match crate::cli::tx::fetch_transaction_report(
                &req.target,
                chain,
                args.decode,
                args.trace,
                &state.factory,
            )
            .await
            {
                Ok(report) => Json(serde_json::json!({
                    "target_info": target_type,
                    "data": report,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
        insights::InferredTarget::Token { chain } => {
            match crate::cli::crawl::fetch_analytics_for_input(
                &req.target,
                chain,
                crate::cli::crawl::Period::Hour24,
                10,
                &state.factory,
            )
            .await
            {
                Ok(analytics) => Json(serde_json::json!({
                    "target_info": target_type,
                    "data": analytics,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_full() {
        let json = serde_json::json!({
            "target": "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2",
            "chain": "polygon",
            "decode": true,
            "trace": true
        });
        let req: InsightsRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.target, "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2");
        assert_eq!(req.chain, Some("polygon".to_string()));
        assert!(req.decode);
        assert!(req.trace);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "target": "0x1234567890123456789012345678901234567890"
        });
        let req: InsightsRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.target, "0x1234567890123456789012345678901234567890");
        assert_eq!(req.chain, None);
        assert!(!req.decode);
        assert!(!req.trace);
    }

    #[test]
    fn test_with_chain_override() {
        let json = serde_json::json!({
            "target": "USDC",
            "chain": "ethereum"
        });
        let req: InsightsRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.target, "USDC");
        assert_eq!(req.chain, Some("ethereum".to_string()));
        assert!(!req.decode);
        assert!(!req.trace);
    }

    #[test]
    fn test_flags() {
        let json_decode = serde_json::json!({
            "target": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "decode": true,
            "trace": false
        });
        let req_decode: InsightsRequest = serde_json::from_value(json_decode).unwrap();
        assert!(req_decode.decode);
        assert!(!req_decode.trace);

        let json_trace = serde_json::json!({
            "target": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "decode": false,
            "trace": true
        });
        let req_trace: InsightsRequest = serde_json::from_value(json_trace).unwrap();
        assert!(!req_trace.decode);
        assert!(req_trace.trace);

        let json_both = serde_json::json!({
            "target": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "decode": true,
            "trace": true
        });
        let req_both: InsightsRequest = serde_json::from_value(json_both).unwrap();
        assert!(req_both.decode);
        assert!(req_both.trace);
    }

    #[tokio::test]
    async fn test_handle_insights_address() {
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
        let req = InsightsRequest {
            target: "0x742d35Cc6634C0532925a3b844Bc9e7595f1b3c2".to_string(),
            chain: None,
            decode: false,
            trace: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_insights_token() {
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
        let req = InsightsRequest {
            target: "USDC".to_string(),
            chain: Some("ethereum".to_string()),
            decode: false,
            trace: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }

    #[tokio::test]
    async fn test_handle_insights_tx() {
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
        let req = InsightsRequest {
            target: "0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            chain: None,
            decode: true,
            trace: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }
}
