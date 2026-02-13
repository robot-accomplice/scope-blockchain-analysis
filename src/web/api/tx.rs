//! Transaction analysis API handler.

use crate::cli::tx;
use crate::web::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
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
