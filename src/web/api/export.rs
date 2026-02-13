//! Export API handler.

use crate::chains::ChainClientFactory;
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
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
/// Returns the export data directly as JSON (regardless of requested format)
/// since the web API always returns JSON. The `format` field is preserved
/// in the response metadata for client-side handling.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExportRequest>,
) -> impl IntoResponse {
    let client: Box<dyn crate::chains::ChainClient> = match state.factory.create_chain_client(&req.chain) {
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
    let mut balance = match client.get_balance(&req.address).await {
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
    let txs = client.get_transactions(&req.address, 100).await.ok();

    // Fetch token balances
    let tokens = client.get_token_balances(&req.address).await.ok();

    Json(serde_json::json!({
        "address": req.address,
        "chain": req.chain,
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
