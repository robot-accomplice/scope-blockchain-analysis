//! Address analysis API handler.

use crate::chains::ChainClientFactory;
use crate::cli::address::{self, AddressArgs};
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
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
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddressRequest>,
) -> impl IntoResponse {
    let args = AddressArgs {
        address: req.address,
        chain: req.chain,
        format: None,
        include_txs: req.include_txs,
        include_tokens: req.include_tokens,
        limit: req.limit,
        report: None,
        dossier: req.dossier,
    };

    let client: Box<dyn crate::chains::ChainClient> = match state.factory.create_chain_client(&args.chain) {
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
