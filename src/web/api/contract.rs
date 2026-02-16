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
