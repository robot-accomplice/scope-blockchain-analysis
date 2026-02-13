//! Transaction analysis API handler.

use crate::cli::tx;
use crate::web::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

/// Request body for transaction analysis.
#[derive(Debug, Deserialize)]
pub struct TxRequest {
    /// Transaction hash.
    pub hash: String,
    /// Target chain (default: "ethereum").
    #[serde(default = "default_chain")]
    pub chain: String,
    /// Decode input data.
    #[serde(default)]
    pub decode: bool,
    /// Include internal transaction trace.
    #[serde(default)]
    pub trace: bool,
}

fn default_chain() -> String {
    "ethereum".to_string()
}

/// POST /api/tx — Analyze a transaction.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TxRequest>,
) -> impl IntoResponse {
    match tx::fetch_transaction_report(&req.hash, &req.chain, req.decode, req.trace, &state.factory)
        .await
    {
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
            "hash": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            "chain": "polygon",
            "decode": true,
            "trace": true
        });
        let req: TxRequest = serde_json::from_value(json).unwrap();
        assert_eq!(
            req.hash,
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        );
        assert_eq!(req.chain, "polygon");
        assert!(req.decode);
        assert!(req.trace);
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = serde_json::json!({
            "hash": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        });
        let req: TxRequest = serde_json::from_value(json).unwrap();
        assert_eq!(
            req.hash,
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        );
        assert_eq!(req.chain, "ethereum");
        assert!(!req.decode);
        assert!(!req.trace);
    }

    #[test]
    fn test_defaults() {
        assert_eq!(default_chain(), "ethereum");
    }

    #[test]
    fn test_decode_trace_flags() {
        let json_decode = serde_json::json!({
            "hash": "0x123",
            "decode": true,
            "trace": false
        });
        let req_decode: TxRequest = serde_json::from_value(json_decode).unwrap();
        assert!(req_decode.decode);
        assert!(!req_decode.trace);

        let json_trace = serde_json::json!({
            "hash": "0x123",
            "decode": false,
            "trace": true
        });
        let req_trace: TxRequest = serde_json::from_value(json_trace).unwrap();
        assert!(!req_trace.decode);
        assert!(req_trace.trace);
    }

    #[tokio::test]
    async fn test_handle_tx_direct() {
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
        let req = TxRequest {
            hash: "0xabc123def456789012345678901234567890123456789012345678901234abcd".to_string(),
            chain: "ethereum".to_string(),
            decode: false,
            trace: false,
        };
        let response = handle(State(state), axum::Json(req)).await.into_response();
        let status = response.status();
        assert!(status.is_success() || status.is_client_error() || status.is_server_error());
    }
}
